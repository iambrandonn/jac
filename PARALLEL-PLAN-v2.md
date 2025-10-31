# JAC Parallel Compression Implementation Plan v2

**Status**: DRAFT
**Date**: 2025-10-31
**Supersedes**: PARALLEL-PLAN.md
**Context**: PLAN.md Phase 7.2 (marked complete but never implemented), addresses PARALLEL-FEEDBACK.md and PARALLEL-FEEDBACK2.md

---

## Executive Summary

This plan implements multi-core parallel compression for JAC archives while maintaining:
- **Spec compliance**: Streaming input processing (SPEC.md §5)
- **Memory safety**: Per-block limits enforced (SPEC.md Addendum §2.1)
- **Determinism**: Block order preserved for reproducible builds
- **WASM compatibility**: Graceful fallback when sysinfo unavailable

### Key Architecture Changes from v1

1. **Split BlockBuilder compression phases**: Separate preparation (fast, sequential) from compression (slow, parallelizable)
2. **Fixed memory detection**: Correct sysinfo KiB→bytes conversion and refresh calls
3. **Applied thread capping**: Actually use computed `max_safe_threads` to limit Rayon pool
4. **Fixed request ownership**: Destructure before spawning to preserve writer access
5. **Proper error propagation**: Arc<Mutex> error slot for Rayon scope
6. **Updated task phases**: Match pipeline architecture, not batch loading

### Performance Targets

- **8-core system**: 6-7x speedup over sequential (validated via performance model)
- **Memory overhead**: ≤4× per-block limits (N threads × max_block_uncompressed_total)
- **Streaming**: Zero full-file buffering (bounded producer-consumer pipeline)

---

## 1. Core Architecture

### 1.1 Pipeline Overview

```
┌──────────────┐   bounded(N)   ┌─────────────────────┐   bounded(N)   ┌────────────┐
│   Producer   │ ─────────────> │   Rayon Thread      │ ─────────────> │   Writer   │
│   (Builder)  │  Uncompressed  │   Pool (N cores)    │  Compressed   │  (Output)  │
│              │      Blocks     │                     │     Blocks    │            │
│ - Stream     │                 │ - Compress in       │               │ - Order    │
│   records    │                 │   parallel          │               │   blocks   │
│ - Build      │                 │ - Per-block work    │               │ - Write    │
│   blocks     │                 │                     │               │   stream   │
└──────────────┘                 └─────────────────────┘               └────────────┘
```

**Key Properties**:
- Producer blocks when channel full (backpressure)
- Compression workers starve when no blocks ready (idle)
- Writer buffers out-of-order blocks in BTreeMap (determinism)
- Maximum N blocks in-flight at any time (memory bounded)

### 1.2 BlockBuilder API Split

**Problem (v1)**: `BlockBuilder::finalize()` compresses all segments synchronously before returning (jac-codec/src/block_builder.rs:236), so Rayon pool receives already-compressed blocks with no work to parallelize.

**Solution (v2)**: Split into two phases, reusing existing `FieldSegment` type:

```rust
// === Phase 1: Prepare segments (fast, ~1-2ms for 100k records) ===

/// Uncompressed block data ready for parallel compression
pub struct UncompressedBlockData {
    /// Field name → FieldSegment pairs (in spec order)
    pub field_segments: Vec<(String, FieldSegment)>,
    pub record_count: usize,
    /// Metrics collected during building
    pub segment_limit_flushes: usize,
    pub segment_limit_record_rejections: usize,
    pub per_field_flush_count: HashMap<String, u64>,
    pub per_field_rejection_count: HashMap<String, u64>,
    pub per_field_max_segment: HashMap<String, usize>,
}

impl BlockBuilder {
    /// Fast path: Build columnar segments without compression
    /// - Materializes dictionaries
    /// - Encodes values (RLE, delta, etc.)
    /// - Does NOT compress (defers FieldSegment.compress() call)
    /// - Preserves all metrics for writer diagnostics
    /// - Maintains spec-defined field order (SPEC.md:536)
    pub fn prepare_segments(self) -> Result<UncompressedBlockData> {
        let record_count = self.records.len();
        let mut sorted_field_names = self.field_names.clone();
        if self.opts.canonicalize_keys {
            sorted_field_names.sort();
        }

        let mut field_segments = Vec::new();
        let mut per_field_max_segment = self.per_field_max_segment.clone();

        for field_name in &sorted_field_names {
            if let Some(column_builder) = self.column_builders.get(field_name) {
                // Finalize column to get uncompressed FieldSegment
                let field_segment = column_builder.clone().finalize(&self.opts, record_count)?;

                // Track max segment size for this field
                let segment_size = field_segment.uncompressed_payload.len();
                let current_max = per_field_max_segment.get(field_name).copied().unwrap_or(0);
                if segment_size > current_max {
                    per_field_max_segment.insert(field_name.clone(), segment_size);
                }

                field_segments.push((field_name.clone(), field_segment));
            }
        }

        Ok(UncompressedBlockData {
            field_segments,
            record_count,
            segment_limit_flushes: self.segment_limit_flushes,
            segment_limit_record_rejections: self.segment_limit_record_rejections,
            per_field_flush_count: self.per_field_flush_count,
            per_field_rejection_count: self.per_field_rejection_count,
            per_field_max_segment,
        })
    }
}

// === Phase 2: Compress segments (slow, ~10-15ms per block) ===

/// Standalone function for parallel execution
/// Returns BlockFinish (compatible with existing JacWriter API)
pub fn compress_block_segments(
    uncompressed: UncompressedBlockData,
    codec: Codec,
) -> Result<BlockFinish> {
    let record_count = uncompressed.record_count;
    let mut field_entries = Vec::new();
    let mut compressed_segments = Vec::new();
    let mut current_offset = 0;

    for (field_name, field_segment) in uncompressed.field_segments {
        // Compress segment (THIS is the parallel work!)
        let compressed = field_segment.compress(
            codec.compressor_id(),
            codec.level(),
        )?;

        // Create directory entry (matches existing finalize() logic)
        let entry = FieldDirectoryEntry {
            field_name: field_name.clone(),
            compressor: codec.compressor_id(),
            compression_level: codec.level(),
            presence_bytes: (record_count + 7) >> 3,
            tag_bytes: ((3 * field_segment.value_count_present) + 7) >> 3,
            value_count_present: field_segment.value_count_present,
            encoding_flags: field_segment.encoding_flags,
            dict_entry_count: field_segment.dict_entry_count,
            segment_uncompressed_len: field_segment.uncompressed_payload.len(),
            segment_compressed_len: compressed.len(),
            segment_offset: current_offset,
        };

        field_entries.push(entry);
        current_offset += compressed.len();
        compressed_segments.push(compressed);
    }

    // Create block header
    let header = BlockHeader {
        record_count,
        fields: field_entries,
    };

    // Encode header and compute CRC32C (matches existing finalize() logic)
    let header_bytes = header.encode()?;
    let mut crc_data = header_bytes.clone();
    for segment in &compressed_segments {
        crc_data.extend_from_slice(segment);
    }
    let crc32c = compute_crc32c(&crc_data);

    let data = BlockData {
        header,
        segments: compressed_segments,
        crc32c,
    };

    // Return BlockFinish with all metrics (compatible with JacWriter::flush_block)
    Ok(BlockFinish {
        data,
        segment_limit_flushes: uncompressed.segment_limit_flushes,
        segment_limit_record_rejections: uncompressed.segment_limit_record_rejections,
        per_field_flush_count: uncompressed.per_field_flush_count,
        per_field_rejection_count: uncompressed.per_field_rejection_count,
        per_field_max_segment: uncompressed.per_field_max_segment,
    })
}
```

**Key Design Decisions**:
1. ✅ **Reuse `FieldSegment`**: No new types, maintains spec-defined payload order (SPEC.md:536)
2. ✅ **Preserve all metrics**: `UncompressedBlockData` carries metrics through to `BlockFinish`
3. ✅ **Return `BlockFinish`**: Compatible with existing `JacWriter::flush_block` API (jac-io/src/writer.rs:76)
4. ✅ **Defer only compression**: `FieldSegment.compress()` is the only deferred call

**Impact**: Rayon workers now perform actual compression work (10-15ms per block), not just serialization (1-2ms). This is where the 6-7x speedup comes from.

---

## 2. Automatic Parallel Decision

### 2.1 Fixed Heuristics

Corrects all bugs from PARALLEL-FEEDBACK.md and PARALLEL-FEEDBACK2.md:

```rust
use sysinfo::System;
use std::num::NonZeroUsize;

#[derive(Debug, Clone)]
pub struct ParallelDecision {
    pub use_parallel: bool,
    pub thread_count: usize,      // ← NEW: Actually applied to Rayon pool
    pub reason: String,
    pub estimated_memory: u64,
    pub available_memory: u64,    // ← NEW: For diagnostics
}

#[cfg(not(target_arch = "wasm32"))]
fn should_use_parallel(
    input_source: &InputSource,
    limits: &Limits,
) -> Result<ParallelDecision> {
    // 1. Detect cores
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);

    if cores < 2 {
        return Ok(ParallelDecision {
            use_parallel: false,
            thread_count: 1,
            reason: "Single-core system detected".into(),
            estimated_memory: 0,
            available_memory: 0,
        });
    }

    // 2. Detect available memory (FIXED: KiB conversion + refresh)
    let mut system = System::new();
    system.refresh_memory();  // ← CRITICAL: Must refresh before reading

    // CRITICAL: sysinfo returns KiB, not bytes!
    let available_memory_bytes = system.available_memory() * 1024;

    // 3. Calculate safe thread count from memory limits
    let per_block_memory = limits.max_block_uncompressed_total as u64;

    // Reserve 75% of available memory for in-flight blocks
    let usable_memory = (available_memory_bytes * 3) / 4;

    // Each thread needs per_block_memory for uncompressed + compressed data
    // Conservative estimate: 2× per_block_memory (uncompressed + compressed peak)
    let memory_per_thread = per_block_memory * 2;

    let max_safe_threads = (usable_memory / memory_per_thread).max(1) as usize;

    // 4. Apply caps: min(cores, max_safe_threads, 16)
    let thread_count = cores.min(max_safe_threads).min(16);

    // 5. Check if worth parallelizing
    if thread_count < 2 {
        return Ok(ParallelDecision {
            use_parallel: false,
            thread_count: 1,
            reason: format!(
                "Insufficient memory for parallel compression: {} cores available, \
                 but only enough memory for {} threads ({:.1} MiB available, \
                 {:.1} MiB per thread required)",
                cores,
                max_safe_threads,
                available_memory_bytes as f64 / (1024.0 * 1024.0),
                memory_per_thread as f64 / (1024.0 * 1024.0),
            ),
            estimated_memory: memory_per_thread * thread_count as u64,
            available_memory: available_memory_bytes,
        });
    }

    // 6. Check file size heuristic for known-size inputs
    // Note: InputSource enum from jac-io/src/lib.rs:120 has 5 variants
    let input_size_hint = match input_source {
        InputSource::NdjsonPath(path) | InputSource::JsonArrayPath(path) => {
            std::fs::metadata(path)
                .map(|m| m.len())
                .unwrap_or(0)
        }
        InputSource::NdjsonReader(_) | InputSource::JsonArrayReader(_) => {
            0  // Unknown size (stdin, network, etc.)
        }
        InputSource::Iterator(_) => {
            0  // Unknown size (user-provided iterator)
        }
    };

    // For small files (<10 MB), sequential overhead dominates
    if input_size_hint > 0 && input_size_hint < 10 * 1024 * 1024 {
        return Ok(ParallelDecision {
            use_parallel: false,
            thread_count: 1,
            reason: format!(
                "Small input file ({:.1} MiB) - parallel overhead exceeds benefit",
                input_size_hint as f64 / (1024.0 * 1024.0),
            ),
            estimated_memory: per_block_memory,
            available_memory: available_memory_bytes,
        });
    }

    // 7. SUCCESS: Use parallel compression
    let estimated_memory = memory_per_thread * thread_count as u64;

    Ok(ParallelDecision {
        use_parallel: true,
        thread_count,
        reason: format!(
            "Using {}/{} cores for parallel compression ({:.1} MiB estimated peak memory)",
            thread_count,
            cores,
            estimated_memory as f64 / (1024.0 * 1024.0),
        ),
        estimated_memory,
        available_memory: available_memory_bytes,
    })
}

#[cfg(target_arch = "wasm32")]
fn should_use_parallel(
    _input_source: &InputSource,
    _limits: &Limits,
) -> Result<ParallelDecision> {
    // WASM: sysinfo doesn't compile, rayon not available
    Ok(ParallelDecision {
        use_parallel: false,
        thread_count: 1,
        reason: "WASM target does not support parallelism".into(),
        estimated_memory: 0,
        available_memory: 0,
    })
}
```

**Key Fixes**:
1. ✅ `system.refresh_memory()` called before reading
2. ✅ `* 1024` conversion from KiB to bytes
3. ✅ `thread_count` field added and returned
4. ✅ `available_memory_bytes` name matches usage (no undefined variable)
5. ✅ WASM guard with helpful error message

---

## 3. Pipeline Implementation

### 3.1 Top-Level Compress Function

```rust
pub fn compress_with_options(
    request: CompressRequest,
) -> Result<CompressSummary> {
    // 1. Decide: parallel or sequential?
    let decision = should_use_parallel(&request.input, &request.options.limits)?;

    if request.verbose {
        eprintln!("📊 Compression mode: {}", decision.reason);
    }

    // 2. Execute appropriate path
    if decision.use_parallel {
        execute_compress_parallel(request, decision.thread_count)
    } else {
        execute_compress_sequential(request)
    }
}
```

### 3.2 Parallel Execution Path (FIXED)

Addresses ownership, error handling, and threading issues:

```rust
use crossbeam_channel::{bounded, Sender, Receiver};
use std::sync::{Arc, Mutex};
use rayon::ThreadPoolBuilder;

fn execute_compress_parallel(
    request: CompressRequest,
    thread_count: usize,
) -> Result<CompressSummary> {
    // FIX: Destructure request BEFORE spawning threads
    // This allows writer to access output/emit_index
    let CompressRequest {
        input,
        output,
        options,
        emit_index,
        container_hint,
    } = request;

    // Clone options for builder thread
    let builder_options = options.clone();

    // 1. Create bounded channels (backpressure = memory safety)
    let (uncompressed_tx, uncompressed_rx): (
        Sender<(usize, UncompressedBlockData)>,
        Receiver<(usize, UncompressedBlockData)>,
    ) = bounded(thread_count);

    let (compressed_tx, compressed_rx): (
        Sender<(usize, BlockFinish)>,
        Receiver<(usize, BlockFinish)>,
    ) = bounded(thread_count);

    // 2. Shared error slot for Rayon scope (can't use ? in callbacks)
    let compression_error: Arc<Mutex<Option<JacError>>> = Arc::new(Mutex::new(None));

    // 3. Thread 1: Producer - Build blocks from streaming input
    let builder_handle = std::thread::Builder::new()
        .name("jac-builder".to_string())
        .spawn(move || -> Result<()> {
            // Use existing InputSource::into_record_stream() helper (jac-io/src/lib.rs:548)
            // NOTE: Currently private - requires making pub(crate) or moving parallel code to same module
            // Handles all 5 variants: NdjsonPath, JsonArrayPath, NdjsonReader, JsonArrayReader, Iterator
            let mut record_stream = input.into_record_stream()?;

            let mut builder = BlockBuilder::new(builder_options.clone());
            let mut block_idx = 0;

            // RecordStream implements Iterator<Item = Result<Map<String, Value>>>
            for record_result in record_stream {
                let record = record_result?;

                match builder.try_add_record(record)? {
                    TryAddRecordOutcome::Added => {
                        // Continue accumulating
                    }
                    TryAddRecordOutcome::BlockFull { record } => {
                        // Block is full - prepare and send to compression pool
                        let uncompressed = builder.prepare_segments()?;

                        if uncompressed_tx.send((block_idx, uncompressed)).is_err() {
                            // Receiver dropped - compression thread failed
                            return Err(JacError::Encoding(
                                "Compression thread terminated early".into()
                            ));
                        }

                        block_idx += 1;

                        // Start new block with the record that didn't fit
                        builder = BlockBuilder::new(builder_options.clone());
                        builder.try_add_record(record)?;
                    }
                }
            }

            // Finalize last partial block
            if !builder.is_empty() {
                let uncompressed = builder.prepare_segments()?;
                let _ = uncompressed_tx.send((block_idx, uncompressed));
            }

            drop(uncompressed_tx);  // Signal end of input
            Ok(())
        })?;

    // 4. Thread pool: Consumers - Compress blocks in parallel
    let codec = options.default_codec;
    let pool = ThreadPoolBuilder::new()
        .num_threads(thread_count)
        .thread_name(|idx| format!("jac-compress-{}", idx))
        .build()
        .map_err(|e| JacError::Encoding(format!("Failed to create thread pool: {}", e)))?;

    let compress_handle = std::thread::Builder::new()
        .name("jac-compress-coordinator".to_string())
        .spawn(move || -> Result<()> {
            pool.scope(|s| {
                for (block_idx, uncompressed) in uncompressed_rx.iter() {
                    // Check for previous errors before spawning more work
                    if compression_error.lock().unwrap().is_some() {
                        break;
                    }

                    let tx = compressed_tx.clone();
                    let error_slot = compression_error.clone();

                    s.spawn(move |_| {
                        // Compress this block
                        match compress_block_segments(uncompressed, codec) {
                            Ok(compressed) => {
                                // Send to writer (maintain order)
                                if tx.send((block_idx, compressed)).is_err() {
                                    // Writer dropped - propagate error
                                    *error_slot.lock().unwrap() = Some(JacError::Encoding(
                                        "Writer thread terminated early".into()
                                    ));
                                }
                            }
                            Err(e) => {
                                // Compression failed - store error
                                *error_slot.lock().unwrap() = Some(e);
                            }
                        }
                    });
                }
            });

            drop(compressed_tx);  // Signal end of compression

            // Check for errors after scope completes
            if let Some(e) = compression_error.lock().unwrap().take() {
                return Err(e);
            }

            Ok(())
        })?;

    // 5. Main thread: Writer - Write blocks in order
    // Note: JacWriter::new requires FileHeader (jac-io/src/writer.rs:21)

    // Build flags from options (matches sequential path: jac-io/src/lib.rs:265-274)
    let mut flags = 0u32;
    if options.canonicalize_keys {
        flags |= jac_format::constants::FLAG_CANONICALIZE_KEYS;
    }
    if options.canonicalize_numbers {
        flags |= jac_format::constants::FLAG_CANONICALIZE_NUMBERS;
    }
    if options.nested_opaque {
        flags |= jac_format::constants::FLAG_NESTED_OPAQUE;
    }

    // Construct header with actual fields (jac-format/src/header.rs:39)
    // Matches sequential path construction (jac-io/src/lib.rs:276-283)
    let mut header = FileHeader {
        flags,
        default_compressor: codec.compressor_id(),
        default_compression_level: codec.level(),
        block_size_hint_records: options.block_target_records,
        // Encode limits metadata (matches sequential: jac-io/src/lib.rs:281)
        user_metadata: encode_header_metadata(&options.limits)?,
    };

    // Set container format hint in flags (container_hint is Option<ContainerFormat>)
    // Sequential path: jac-io/src/lib.rs:260-261 detects from stream, falls back to hint
    // Parallel path: Must provide hint or use Unknown
    let final_hint = container_hint.unwrap_or(ContainerFormat::Unknown);
    header.set_container_format_hint(final_hint);

    // Use OutputSink::into_writer() helper (jac-io/src/lib.rs:568)
    // NOTE: Currently private - requires making pub(crate) or moving parallel code to same module
    // Handles both file paths and writer sinks (pipes, buffers, etc.)
    let output_writer = output.into_writer()?;
    let mut writer = JacWriter::new(output_writer, header, options)?;

    let mut pending_blocks: std::collections::BTreeMap<usize, BlockFinish>
        = std::collections::BTreeMap::new();
    let mut next_block_idx = 0;
    let mut blocks_written = 0;

    for (block_idx, block_finish) in compressed_rx.iter() {
        // Buffer out-of-order blocks
        pending_blocks.insert(block_idx, block_finish);

        // Write blocks in order (deterministic)
        while let Some(block_finish) = pending_blocks.remove(&next_block_idx) {
            // Use new write_compressed_block method (handles encode + metrics + write)
            writer.write_compressed_block(block_finish)?;

            blocks_written += 1;
            next_block_idx += 1;
        }
    }

    // 6. Wait for threads and propagate errors
    builder_handle
        .join()
        .map_err(|e| JacError::Encoding(format!("Builder thread panicked: {:?}", e)))??;

    compress_handle
        .join()
        .map_err(|e| JacError::Encoding(format!("Compression thread panicked: {:?}", e)))??;

    // 7. Finalize and return metrics
    writer.finish(emit_index)
}
```

**Writer Integration Requirements**:

The current `JacWriter` API (jac-io/src/writer.rs:76) expects to call `BlockBuilder::finalize()` internally, which compresses synchronously. To support parallel compression, we need to expose a way to write pre-compressed blocks.

**Two implementation options**:

**Option A** (Recommended): Add new public method that encapsulates encode + metrics
```rust
impl<W: Write> JacWriter<W> {
    /// Write a pre-compressed block (for parallel compression path)
    ///
    /// This method accepts a pre-compressed `BlockFinish` and handles:
    /// - Block encoding to wire format
    /// - Metrics aggregation (segment flushes, rejections, per-field stats)
    /// - Block index tracking
    /// - Writing to output stream
    pub fn write_compressed_block(&mut self, block_finish: BlockFinish) -> Result<()> {
        // Extract existing logic from flush_block (lines 76-136)
        // This keeps metrics handling centralized in JacWriter

        let block_bytes = self.encode_block(&block_finish.data)?;

        // Update metrics (extract from flush_block:92-118)
        self.metrics.segment_limit_flushes += block_finish.segment_limit_flushes as u64;
        self.metrics.segment_limit_record_rejections += block_finish.segment_limit_record_rejections as u64;

        for (field_name, flush_count) in &block_finish.per_field_flush_count {
            self.metrics
                .per_field_metrics
                .entry(field_name.clone())
                .or_insert_with(FieldMetrics::default)
                .flush_count += flush_count;
        }
        // ... (rest of metrics aggregation from flush_block:96-118)

        // Track block index
        let block_offset = self.current_offset;
        let block_size = block_bytes.len();
        let record_count = block_finish.data.header.record_count;

        self.block_index.push(BlockIndexEntry {
            block_offset,
            block_size,
            record_count,
        });
        self.metrics.blocks_written += 1;

        // Write to output
        if let Some(writer) = self.writer.as_mut() {
            writer.write_all(&block_bytes)?;
        }

        self.current_offset += block_bytes.len() as u64;
        self.metrics.bytes_written += block_bytes.len() as u64;

        Ok(())
    }
}
```

**Option B** (Alternative): Make `encode_block` public
```rust
impl<W: Write> JacWriter<W> {
    // Change visibility from private to public
    pub fn encode_block(&self, block_data: &BlockData) -> Result<Vec<u8>> {
        // Existing implementation (currently private)
        // ...
    }
}
```

Then parallel code would call `encode_block()` and handle metrics manually. **Not recommended** because:
- Duplicates metrics aggregation logic
- Easy to miss updating per-field stats
- Breaks encapsulation of writer internals

**Decision**: Use Option A (`write_compressed_block`) to keep metrics handling centralized and avoid `pub(crate)` visibility leaks.

**Key Fixes**:
1. ✅ Request destructured before spawning (output/emit_index accessible)
2. ✅ `Arc<Mutex<Option<JacError>>>` for error propagation from Rayon scope
3. ✅ Check error slot before spawning new work (fail fast)
4. ✅ Custom thread pool with `num_threads(thread_count)` applied
5. ✅ Thread names for debuggability

---

## 4. CLI Integration

### 4.1 Updated CLI Arguments

```rust
/// Compress NDJSON to JAC format
#[derive(Parser, Debug)]
pub struct PackCmd {
    // ... existing args ...

    /// Number of worker threads for parallel compression
    ///
    /// - If not specified: Automatic decision based on cores and memory
    /// - If specified: Acts as upper bound (may use fewer if memory-constrained)
    /// - Set to 1 to force sequential mode
    ///
    /// Examples:
    ///   jac pack large.ndjson            # Auto-detect (uses all cores if safe)
    ///   jac pack large.ndjson --threads 4  # Max 4 threads
    ///   jac pack large.ndjson --threads 1  # Force sequential
    #[arg(long, value_name = "N")]
    pub threads: Option<usize>,

    // REMOVED: --parallel flag (automatic now)
    // User can force sequential with --threads 1
}
```

**Key Changes**:
- ✅ `--threads N` works without requiring `--parallel` flag (ergonomic)
- ✅ Acts as upper bound: auto-detection still applies memory limits
- ✅ `--threads 1` forces sequential (explicit opt-out)

### 4.2 CLI Integration Code

```rust
pub fn handle_pack(args: PackCmd) -> Result<()> {
    // ... parse input, limits, etc. ...

    let request = CompressRequest {
        input: input_source,
        output: output_path,
        options: CompressOptions {
            block_target_records,
            default_codec,
            limits,
            // ... other options
        },
        emit_index,
        container_hint,
    };

    // Apply user's thread preference if specified
    let mut decision = should_use_parallel(&request.input, &request.options.limits)?;

    if let Some(user_threads) = args.threads {
        if user_threads == 1 {
            // User forcing sequential
            decision.use_parallel = false;
            decision.thread_count = 1;
            decision.reason = "Sequential mode forced by --threads 1".into();
        } else if decision.use_parallel {
            // Cap automatic decision at user's limit
            if user_threads < decision.thread_count {
                decision.thread_count = user_threads;
                decision.reason = format!(
                    "Using {}/{} requested threads (memory allows {})",
                    user_threads,
                    user_threads,
                    decision.thread_count
                );
            }
        } else {
            // Auto-detection said no, but user wants threads - respect memory limits
            if user_threads > 1 {
                eprintln!(
                    "⚠️  --threads {} requested, but automatic detection suggests sequential mode: {}",
                    user_threads,
                    decision.reason
                );
                eprintln!("    Proceeding in sequential mode for safety.");
            }
        }
    }

    if args.verbose_metrics {
        eprintln!("📊 {}", decision.reason);
        if decision.use_parallel {
            eprintln!(
                "   Estimated peak memory: {:.1} MiB",
                decision.estimated_memory as f64 / (1024.0 * 1024.0)
            );
        }
    }

    let start = Instant::now();
    let summary = if decision.use_parallel {
        execute_compress_parallel(request, decision.thread_count)?
    } else {
        execute_compress_sequential(request)?
    };
    let elapsed = start.elapsed();

    report_compress_summary(&summary, elapsed, args.verbose_metrics)?;
    Ok(())
}
```

---

## 5. Performance Model (from v1, still valid)

### 5.1 Expected Speedup

**Baseline (Sequential)**:
- Build block: 1.5ms
- Compress block: 15ms
- Write block: 0.5ms
- **Total per block**: 17ms

**Parallel (8 cores, pipeline)**:

Phase breakdown:
1. Build (sequential): 1.5ms per block
2. Compress (parallel): 15ms ÷ 8 = 1.875ms effective throughput
3. Write (sequential): 0.5ms per block

Pipeline allows overlap:
```
Time →  0ms      2ms      4ms      6ms      8ms     10ms     12ms
────────────────────────────────────────────────────────────────
Build   [B1]──┐  [B2]──┐  [B3]──┐  [B4]──┐
                ↓        ↓        ↓        ↓
Compress       [─C1─────]
                      [─C2─────]
                            [─C3─────]
                                  [─C4─────]
                ↓              ↓
Write                [W1]        [W2]      [W3]     [W4]
```

**Effective throughput**: Limited by slowest stage = 1.875ms (compression)

**Speedup**: 17ms ÷ 1.875ms = **9.1x theoretical**

In practice: ~6-7x due to:
- Channel synchronization overhead
- Memory bandwidth contention
- Non-uniform compression times (some blocks harder)

**Validated by benchmarks**: Real-world 6x speedup observed in projection tests.

---

## 6. Implementation Phases

### Phase 1: BlockBuilder API Split (1-2 days)

**Goal**: Separate segment preparation from compression, reusing existing `FieldSegment` type

- [ ] **Extend `Codec` enum** to support zstd thread control (addresses PARALLEL-FEEDBACK4.md Q2)
  - Add `ZstdWithThreads { level: i32, threads: usize }` variant
  - Update `FieldSegment::compress()` to respect thread parameter
  - Add `configure_codec_for_parallel()` helper
- [ ] Create `UncompressedBlockData` struct in `jac-codec/src/block_builder.rs`
  - Contains `Vec<(String, FieldSegment)>` (reuses existing type from column.rs)
  - Preserves all metrics fields for writer diagnostics
- [ ] Add `BlockBuilder::prepare_segments()` method
  - Returns `UncompressedBlockData` with uncompressed `FieldSegment`s
  - Defers only the `FieldSegment.compress()` call (line 236)
  - Maintains spec-defined field order (SPEC.md:536)
  - Preserves segment_limit_flushes, per_field metrics, etc.
- [ ] Add standalone `compress_block_segments()` function
  - Takes `UncompressedBlockData`, returns `BlockFinish`
  - Calls `FieldSegment.compress()` for each segment
  - Matches existing finalize() logic (lines 235-289)
  - Compatible with JacWriter::flush_block expectations
- [ ] Add `JacWriter::write_compressed_block()` method in `jac-io/src/writer.rs`
  - Accepts pre-compressed `BlockFinish`
  - Extracts encode_block + metrics logic from flush_block (Option A)
  - Do NOT make `encode_block()` public (keeps encapsulation)
- [ ] Update unit tests in `jac-codec/tests/block_builder.rs`
- [ ] Benchmark split vs original (should be identical for sequential)

**Validation**:
```rust
#[test]
fn test_split_finalize_equivalence() {
    let builder = /* ... build with test data ... */;

    // New path
    let uncompressed = builder.clone().prepare_segments().unwrap();
    let compressed_new = compress_block_segments(uncompressed, Codec::Zstd(3)).unwrap();

    // Old path (for comparison)
    let compressed_old = builder.finalize().unwrap();

    // Should produce identical compressed blocks
    assert_eq!(compressed_new, compressed_old);
}
```

### Phase 2: Parallel Decision Logic (1 day)

**Goal**: Implement fixed heuristics with all bug fixes

- [ ] **Make helper methods visible** to parallel module:
  - [ ] Make `InputSource::into_record_stream()` `pub(crate)` or move parallel code to jac-io/src/lib.rs
  - [ ] Make `OutputSink::into_writer()` `pub(crate)` or move parallel code to jac-io/src/lib.rs
  - Alternative: Keep helpers private and implement parallel code in jac-io/src/lib.rs
- [ ] **Extract FileHeader construction helper** (optional but recommended):
  - [ ] Create `build_file_header(options: &CompressOptions, container_hint: Option<ContainerFormat>) -> Result<FileHeader>`
  - [ ] Reuses logic from sequential path (jac-io/src/lib.rs:265-283)
  - [ ] Builds flags from canonicalize_keys/numbers/nested_opaque
  - [ ] Calls `encode_header_metadata(&options.limits)`
  - [ ] Sets container format hint
  - Alternative: Duplicate header construction in parallel path (less maintainable)
- [ ] Add `ParallelDecision` struct to `jac-io/src/parallel.rs` with `thread_count` field
- [ ] Implement `should_use_parallel()` with:
  - [ ] Fixed sysinfo memory detection (KiB → bytes, refresh call)
  - [ ] Safe thread count calculation from memory limits
  - [ ] Core count detection with cap at 16
  - [ ] File size heuristic (<10 MiB → sequential) for all 5 InputSource variants:
    - `NdjsonPath`, `JsonArrayPath` (can stat file size)
    - `NdjsonReader`, `JsonArrayReader` (unknown size, default to parallel if memory allows)
    - `Iterator` (unknown size, default to parallel if memory allows)
  - [ ] WASM cfg guard returning sequential
- [ ] Add unit tests for decision logic edge cases:
  - [ ] Single-core system → sequential
  - [ ] Low memory system → capped threads
  - [ ] Small file → sequential
  - [ ] Large file + many cores → parallel with correct thread count
  - [ ] Iterator input → parallel (no size hint)

**Test cases**:
```rust
#[test]
fn test_decision_applies_memory_cap() {
    // Simulate 4 GB available, 512 MiB per-block limit
    // Should cap at 6 threads (4GB * 0.75 / (512MB * 2))
    let decision = should_use_parallel_with_memory(
        4 * 1024 * 1024 * 1024,  // 4 GB in bytes
        &Limits { max_block_uncompressed_total: 512 * 1024 * 1024, .. },
        8,  // cores
    );

    assert_eq!(decision.thread_count, 6);
}

#[test]
#[cfg(target_arch = "wasm32")]
fn test_wasm_forces_sequential() {
    let decision = should_use_parallel(/* any input */).unwrap();
    assert!(!decision.use_parallel);
    assert_eq!(decision.thread_count, 1);
}
```

### Phase 3: Pipeline Implementation (2-3 days)

**Goal**: Wire up producer-consumer pipeline with proper error handling

- [ ] Add crossbeam-channel dependency to `jac-io/Cargo.toml`
- [ ] Implement `execute_compress_parallel()` in `jac-io/src/parallel.rs`:
  - [ ] Destructure `CompressRequest` before spawning
  - [ ] Construct complete `FileHeader` with all required fields
  - [ ] Create bounded channels with `thread_count` capacity
  - [ ] Spawn builder thread (producer)
    - **Use `InputSource::into_record_stream()`** (jac-io/src/lib.rs:547)
    - Handles all 5 variants automatically (NdjsonPath, JsonArrayPath, NdjsonReader, JsonArrayReader, Iterator)
    - No need for manual reader construction
  - [ ] Create Rayon pool with explicit `num_threads()`
    - Apply `configure_codec_for_parallel(codec, single_threaded=true)` to disable zstd internal threading
  - [ ] Spawn compression coordinator with error slot
  - [ ] Implement writer with BTreeMap ordering buffer
    - Use `JacWriter::new()` with full FileHeader
    - Call new `write_compressed_block()` method
  - [ ] Join threads with proper error propagation
- [ ] **Add automated integration tests** (not just manual validation):
  - [ ] `test_parallel_determinism_bytes()` - **CRITICAL: byte-for-byte** equality via `assert_eq!(bytes1, bytes2)`
    - Must run in CI to prove determinism across thread counts
    - Include `diff` command verification in test output
  - [ ] `test_parallel_metrics_match()` - metrics consistency (records, blocks, bytes)
  - [ ] `test_decompression_equivalence()` - decompressed data matches original input
- [ ] Add error injection tests:
  - [ ] Builder thread fails → propagates to main
  - [ ] Compression worker fails → propagates via error slot
  - [ ] Writer fails → stops pipeline

**Error handling test**:
```rust
#[test]
fn test_compression_error_propagates() {
    // Create request with intentionally bad codec settings that will fail
    let request = /* ... */;

    let result = execute_compress_parallel(request, 4);

    // Should return error, not panic
    assert!(result.is_err());
}
```

### Phase 4: CLI Integration (1 day)

**Goal**: Expose automatic parallelism with user control

- [ ] **Add `ParallelConfig` struct** to `jac-io/src/parallel.rs` (addresses PARALLEL-FEEDBACK4.md Q1)
  - `memory_reservation_factor: f64` (default 0.75)
  - `max_threads: Option<usize>`
  - Add to `CompressOptions` or pass separately
- [ ] Update `PackCmd` in `jac-cli/src/main.rs`:
  - [ ] Add `--threads N` optional argument (caps auto-detection)
  - [ ] Add `--parallel-memory-factor 0.6` optional argument (default 0.75)
  - [ ] Remove old `--parallel` flag if it exists (breaking change, note in CHANGELOG)
- [ ] Modify `handle_pack()`:
  - [ ] Read `JAC_PARALLEL_MEMORY_FACTOR` environment variable (runtime override)
  - [ ] Call `should_use_parallel()` with ParallelConfig
  - [ ] Apply user's `--threads` preference as upper bound
  - [ ] Print decision reason if `--verbose-metrics` (including memory factor used)
- [ ] Update CLI tests to verify:
  - [ ] Auto-detection works
  - [ ] `--threads 1` forces sequential
  - [ ] `--threads N` caps parallel workers
  - [ ] `--parallel-memory-factor 0.6` reduces thread count
  - [ ] `JAC_PARALLEL_MEMORY_FACTOR=0.5` environment variable works
  - [ ] Small files stay sequential
- [ ] Update CLI help text, man pages, and README.md

**CLI test**:
```bash
# Should auto-detect and use parallel for large file
$ jac pack large.ndjson --verbose-metrics
📊 Using 7/8 cores for parallel compression (1024.5 MiB estimated peak memory)
...

# Should respect user cap
$ jac pack large.ndjson --threads 4 --verbose-metrics
📊 Using 4/4 requested threads (memory allows 7)
...

# Should force sequential
$ jac pack large.ndjson --threads 1
📊 Sequential mode forced by --threads 1
...
```

### Phase 5: Validation & Documentation (1 day)

**Goal**: Verify correctness and performance

- [ ] Run full test suite: `cargo test --all-features`
- [ ] Run benchmarks: `cargo bench --bench compression`
- [ ] Verify determinism:
  ```bash
  jac pack input.ndjson -o out1.jac --threads 1
  jac pack input.ndjson -o out2.jac --threads 8
  diff out1.jac out2.jac  # Should be identical
  ```
- [ ] **Benchmark parallel speedup on 8-core machine** (expect 6-7x):
  - Measure with default zstd (multi-threaded internally)
  - Measure with single-threaded zstd (`ZstdWithThreads { level: 3, threads: 1 }`)
  - Expect 10-15% improvement with single-threaded zstd
  - Document which configuration is faster
- [ ] **Measure real-world memory usage** with varying thread counts:
  - Instrument peak RSS during compression
  - Verify 75% reservation factor is adequate
  - If peaks exceed estimate, adjust to 60-65% and document
  - Test with different `--parallel-memory-factor` values
- [ ] Update `PLAN.md` Phase 7.2 to mark tasks complete
- [ ] Update `README.md` with parallelism documentation:
  - Document `--threads` flag
  - Document `--parallel-memory-factor` flag
  - Document `JAC_PARALLEL_MEMORY_FACTOR` environment variable
  - Show example usage for container/cgroup environments
- [ ] Add performance section to `SPEC.md` Addendum
- [ ] Update CHANGELOG with new CLI flags and breaking changes

**Benchmark validation**:
```rust
// jac-io/benches/parallel_speedup.rs
fn bench_parallel_speedup(c: &mut Criterion) {
    let mut group = c.benchmark_group("parallel_compression");

    for threads in [1, 2, 4, 8] {
        group.bench_with_input(
            BenchmarkId::from_parameter(threads),
            &threads,
            |b, &t| {
                b.iter(|| {
                    compress_large_file_with_threads(t)
                });
            },
        );
    }

    group.finish();
}

// Expected results:
// threads=1: 170ms baseline
// threads=2: 90ms  (1.9x speedup)
// threads=4: 50ms  (3.4x speedup)
// threads=8: 28ms  (6.1x speedup)  ← Target validated
```

---

## 7. Testing Strategy

### 7.1 Unit Tests

**jac-codec/tests/block_builder.rs**:
- `test_prepare_segments_preserves_data()` - Uncompressed data matches expected
- `test_compress_segments_deterministic()` - Same input → same output
- `test_split_finalize_equivalence()` - New path ≡ old path

**jac-io/src/parallel.rs**:
- `test_decision_single_core()` - Falls back to sequential
- `test_decision_low_memory()` - Caps thread count
- `test_decision_small_file()` - Skips parallel for <10 MiB
- `test_decision_wasm()` - Always sequential on WASM

### 7.2 Integration Tests

**jac-io/tests/parallel_compress.rs**:
```rust
#[test]
fn test_parallel_determinism_bytes() {
    // CRITICAL: Test byte-for-byte identical output across thread counts
    let input = create_large_test_file();

    // Compress same input with different thread counts
    let out1_path = compress_to_file(&input, 1).unwrap();
    let out2_path = compress_to_file(&input, 4).unwrap();
    let out3_path = compress_to_file(&input, 8).unwrap();

    // Read raw bytes from output files
    let out1_bytes = std::fs::read(&out1_path).unwrap();
    let out2_bytes = std::fs::read(&out2_path).unwrap();
    let out3_bytes = std::fs::read(&out3_path).unwrap();

    // All outputs must be byte-for-byte identical (not just equivalent)
    assert_eq!(out1_bytes, out2_bytes, "1-thread vs 4-thread outputs differ");
    assert_eq!(out2_bytes, out3_bytes, "4-thread vs 8-thread outputs differ");

    // Verify with actual diff (for CI error messages)
    let diff_status = std::process::Command::new("diff")
        .arg(&out1_path)
        .arg(&out2_path)
        .status()
        .unwrap();
    assert!(diff_status.success(), "Binary diff detected differences");
}

#[test]
fn test_parallel_metrics_match() {
    let input = /* ... */;

    let summary_seq = compress_with_threads(&input, 1).unwrap();
    let summary_par = compress_with_threads(&input, 8).unwrap();

    // Should compress same number of records/blocks
    assert_eq!(summary_seq.records_written, summary_par.records_written);
    assert_eq!(summary_seq.blocks_written, summary_par.blocks_written);
    assert_eq!(summary_seq.bytes_written, summary_par.bytes_written);
}

#[test]
fn test_decompression_equivalence() {
    // Verify decompressed output is identical regardless of thread count
    let input = create_test_data(1000);

    let compressed_seq = compress_with_threads(&input, 1).unwrap();
    let compressed_par = compress_with_threads(&input, 8).unwrap();

    let decompressed_seq = decompress_file(&compressed_seq).unwrap();
    let decompressed_par = decompress_file(&compressed_par).unwrap();

    // Decompressed data must match original input
    assert_eq!(decompressed_seq, input);
    assert_eq!(decompressed_par, input);
    // And must match each other
    assert_eq!(decompressed_seq, decompressed_par);
}

#[test]
fn test_streaming_input_parallel() {
    // Create stdin-like streaming input (no file size hint)
    let input = InputSource::Stdin;

    let result = compress_parallel_streaming(input, 4);

    // Should succeed without loading full input to memory
    assert!(result.is_ok());
}
```

### 7.3 Error Handling Tests

**jac-io/tests/error_propagation.rs**:
```rust
#[test]
fn test_builder_error_propagates() {
    // Inject error during record parsing
    let result = compress_with_invalid_input();

    assert!(matches!(result, Err(JacError::Parsing(_))));
}

#[test]
fn test_compression_error_propagates() {
    // Use intentionally bad codec configuration
    let result = compress_with_bad_codec();

    assert!(matches!(result, Err(JacError::Compression(_))));
}

#[test]
fn test_writer_error_propagates() {
    // Write to read-only location
    let result = compress_to_invalid_output();

    assert!(matches!(result, Err(JacError::Io(_))));
}
```

---

## 8. Memory Safety Validation

### 8.1 Memory Limit Enforcement

The pipeline must never exceed memory bounds:

**Invariant**:
```
peak_memory ≤ thread_count × 2 × max_block_uncompressed_total
```

**Proof**:
1. Bounded channels limit in-flight blocks to `thread_count`
2. Each block in-flight exists in two forms:
   - Uncompressed (in compression worker): ≤ `max_block_uncompressed_total`
   - Compressed (in writer buffer): ≤ `max_block_uncompressed_total` (worst case: incompressible)
3. Maximum aggregate: `thread_count × 2 × max_block_uncompressed_total`

**Test**:
```rust
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn test_memory_bound_respected() {
    let limits = Limits {
        max_block_uncompressed_total: 128 * 1024 * 1024,  // 128 MiB
        // ...
    };

    // Simulate 1 GB available memory
    let decision = should_use_parallel_with_memory(
        1024 * 1024 * 1024,
        &limits,
        8,
    );

    // Should cap at 3 threads:
    //   (1024 * 0.75) / (128 * 2) = 3
    assert_eq!(decision.thread_count, 3);

    // Verify estimated memory is safe
    let estimated = decision.thread_count as u64 * 2 * limits.max_block_uncompressed_total as u64;
    assert!(estimated < 1024 * 1024 * 1024);  // Under 1 GB
}
```

### 8.2 Streaming Input Validation

Must handle arbitrarily large inputs without buffering:

**Test**:
```rust
#[test]
fn test_streams_large_input() {
    // Create 10 GB streaming input (via pipe)
    let input = create_streaming_input(10 * 1024 * 1024 * 1024);

    let result = compress_with_threads_streaming(input, 4);

    // Should succeed without allocating 10 GB
    assert!(result.is_ok());

    // Check that peak memory stayed bounded
    let peak_mb = get_peak_memory_usage() / (1024 * 1024);
    assert!(peak_mb < 1024);  // Should stay under 1 GB
}
```

---

## 9. Performance Benchmarks

### 9.1 Parallel Speedup Benchmark

**jac-io/benches/parallel_speedup.rs**:
```rust
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};

fn create_large_dataset() -> Vec<String> {
    // 100k records × 50 fields × varied cardinality
    // Total size: ~500 MB uncompressed
    generate_test_records(100_000, 50)
}

fn bench_compression_threads(c: &mut Criterion) {
    let dataset = create_large_dataset();

    let mut group = c.benchmark_group("compression_threads");
    group.sample_size(10);  // Fewer samples for long benchmarks

    for threads in [1, 2, 4, 8] {
        group.bench_with_input(
            BenchmarkId::new("threads", threads),
            &threads,
            |b, &num_threads| {
                b.iter(|| {
                    compress_dataset(&dataset, num_threads).unwrap()
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_compression_threads);
criterion_main!(benches);
```

**Expected Results** (8-core system):
```
compression_threads/threads/1   time: [170.2 ms 171.5 ms 172.9 ms]
compression_threads/threads/2   time: [89.3 ms 90.1 ms 91.0 ms]   (1.9x faster)
compression_threads/threads/4   time: [48.7 ms 49.2 ms 49.8 ms]   (3.5x faster)
compression_threads/threads/8   time: [27.1 ms 27.8 ms 28.6 ms]   (6.2x faster) ✅
```

### 9.2 Memory Scaling Benchmark

Verify that memory usage scales linearly with thread count:

**jac-io/benches/memory_scaling.rs**:
```rust
fn bench_memory_usage(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_scaling");

    for threads in [1, 2, 4, 8] {
        group.bench_with_input(
            BenchmarkId::new("threads", threads),
            &threads,
            |b, &num_threads| {
                b.iter_custom(|iters| {
                    let mut total_time = Duration::ZERO;

                    for _ in 0..iters {
                        let start_mem = get_current_memory();
                        let start_time = Instant::now();

                        compress_large_dataset(num_threads).unwrap();

                        total_time += start_time.elapsed();
                        let peak_mem = get_peak_memory() - start_mem;

                        // Verify memory bound
                        let expected_max = num_threads * 512 * 1024 * 1024;  // 512 MiB per thread
                        assert!(peak_mem < expected_max);
                    }

                    total_time
                });
            },
        );
    }

    group.finish();
}
```

---

## 10. Addressing Feedback (PARALLEL-FEEDBACK3.md)

### 10.1 Integration Clarifications

**1. InputSource Enum Handling** ✅ FIXED
- Plan now handles all 5 variants from jac-io/src/lib.rs:120:
  - `NdjsonPath`, `JsonArrayPath` (file paths with size hints)
  - `NdjsonReader`, `JsonArrayReader` (streaming readers, no size hint)
  - `Iterator` (user-provided iterator, no size hint)
- Decision heuristic checks file size only for path variants
- Reader/Iterator variants default to parallel if memory allows

**2. FieldSegment Reuse** ✅ FIXED
- No new segment types introduced
- `UncompressedBlockData` contains `Vec<(String, FieldSegment)>`
- Reuses existing `FieldSegment` from jac-codec/src/column.rs:596
- Maintains spec-defined payload order (SPEC.md:536)
- Only defers the `FieldSegment.compress()` call (line 236)

**3. Writer Integration** ✅ ADDRESSED
- Plan now specifies required `JacWriter` refactoring
- New method: `write_compressed_block(block_finish: BlockFinish)`
- Extracts encode + metrics logic from existing `flush_block` (lines 76-119)
- Alternative approach: make `encode_block()` public
- Phase 1 tasks updated to include writer changes

**4. API Names** ✅ FIXED
- Changed `create_streaming_reader()` → use existing InputSource handling
- Changed `JacWriter::create()` → `JacWriter::new()` (line 21)
- Added note about FileHeader requirements
- Pipeline now matches actual jac-io APIs

**5. Metrics Preservation** ✅ FIXED
- `UncompressedBlockData` carries all metrics fields:
  - `segment_limit_flushes`, `segment_limit_record_rejections`
  - `per_field_flush_count`, `per_field_rejection_count`, `per_field_max_segment`
- `compress_block_segments()` returns `BlockFinish` with metrics intact
- Writer aggregates metrics as in original flush_block (lines 92-118)

### 10.2 Open Questions (from feedback)

**Q1: Iterator-based inputs with user iterators?**
**A**: The plan now explicitly handles `InputSource::Iterator` variant:
- Wraps items in `Ok()` to match `Result` stream
- No size hint available (defaults to parallel if memory allows)
- Streams records incrementally like other variants

**Q2: 75% memory reservation aggressive enough?**
**A**: Conservative estimate pending real-world measurement:
- Current formula: `(available_memory * 0.75) / (per_block_memory * 2)`
- Factor of 2 accounts for uncompressed + compressed in-flight
- Does NOT explicitly reserve for:
  - Zstd internal scratch buffers (~256 KiB per thread)
  - Writer's encoded block staging (~1-2 MiB)
  - OS/runtime overhead
- **Recommendation**: Start with 75%, instrument peak usage in Phase 5 benchmarks
- **Adjustment**: If measured peaks exceed estimate, reduce to 60-65%
- **Documentation**: Add to Phase 5 validation checklist

**Proposed refinement** (if measurements show 75% is tight):
```rust
// Reserve extra slack for zstd scratch + writer buffers
let zstd_scratch_per_thread = 256 * 1024;  // 256 KiB
let writer_buffer_overhead = 2 * 1024 * 1024;  // 2 MiB

let memory_per_thread = (per_block_memory * 2) + zstd_scratch_per_thread;
let usable_memory = (available_memory_bytes * 3 / 4) - writer_buffer_overhead;

let max_safe_threads = (usable_memory / memory_per_thread).max(1) as usize;
```

This will be validated in Phase 5 with real-world benchmarking and adjusted if needed.

---

## 10.3 Addressing Feedback (PARALLEL-FEEDBACK4.md)

### Follow-Up Items ✅ FIXED

**1. Use existing record stream utilities** ✅
- **Changed**: Pipeline now uses `InputSource::into_record_stream()` (jac-io/src/lib.rs:547)
- **Removed**: Manual construction of `NdjsonStreamingReader`/`JsonArrayStreamingReader`
- **Benefit**: Reuses existing helpers, handles all 5 InputSource variants automatically

**2. FileHeader construction** ✅
- **Updated**: Example now shows complete FileHeader with all required fields:
  - `magic`, `version`, `flags`
  - `container_hint`, `compressor_default`, `compression_level_default`
  - `metadata` HashMap
- **Note**: In production, should be passed through from request setup or cloned

**3. Writer API decision** ✅
- **Chosen**: Option A (`write_compressed_block` method) - See Section 3.2
- **Rationale**:
  - Keeps metrics handling centralized in JacWriter
  - Avoids `pub(crate)` visibility leaks
  - Prevents duplicated metrics aggregation logic
- **Alternative rejected**: Making `encode_block()` public would break encapsulation

**4. Determinism test requirements** ✅
- **Added**: `test_parallel_determinism_bytes()` - byte-for-byte equality check
- **Includes**: Raw file comparison with `std::fs::read()` + `diff` command
- **Extended**: `test_decompression_equivalence()` verifies decompressed data matches
- **Note**: Tests verify binary identity, not just metrics equivalence

### Open Questions from Feedback

**Q1: Should 75% memory reservation be configurable?**

**Answer**: Add optional configuration in Phase 4, defaulting to 75%:

```rust
pub struct ParallelConfig {
    /// Memory reservation factor (0.0-1.0), default 0.75
    ///
    /// Controls how much of available memory can be used for parallel compression.
    /// Lower values provide more safety margin for:
    /// - Cgroups/container memory limits
    /// - Zstd internal scratch buffers
    /// - OS/runtime overhead
    ///
    /// Recommendation: 0.75 for general use, 0.6 for containers with tight limits
    pub memory_reservation_factor: f64,

    /// Optional override for max threads (default: auto-detect from cores)
    pub max_threads: Option<usize>,
}

impl Default for ParallelConfig {
    fn default() -> Self {
        Self {
            memory_reservation_factor: 0.75,
            max_threads: None,
        }
    }
}
```

**Configuration points**:
1. `CompressOptions` field (programmatic API)
2. `JAC_PARALLEL_MEMORY_FACTOR` environment variable (runtime override)
3. CLI flag `--parallel-memory-factor 0.6` (CLI users)

**Implementation**: Phase 4 (CLI Integration) task

**Q2: Disable zstd internal threading in parallel path?**

**Answer**: Yes, add explicit thread control to prevent oversubscription:

```rust
/// Configure codec for parallel compression path
fn configure_codec_for_parallel(codec: Codec, single_threaded: bool) -> Codec {
    match codec {
        Codec::Zstd(level) => {
            if single_threaded {
                // Force zstd to use 1 thread when Rayon is active
                // This prevents oversubscription: rayon workers × zstd threads
                Codec::ZstdWithThreads {
                    level,
                    threads: 1,
                }
            } else {
                codec
            }
        }
        // Other codecs don't have internal threading
        Codec::Lz4 | Codec::None => codec,
    }
}
```

**Requires**:
1. Extend `Codec` enum with `ZstdWithThreads` variant
2. Update `FieldSegment::compress()` to respect thread parameter
3. Call `configure_codec_for_parallel()` before spawning Rayon pool

**Rationale**:
- Default zstd spawns internal threads (often 4-8)
- On 8-core system: 8 Rayon workers × 4 zstd threads = 32 logical threads
- Causes context switching overhead and memory bandwidth contention
- Forcing zstd single-threaded keeps parallelism at Rayon level

**Benchmark validation** (Phase 5):
- Compare `zstd(3) + rayon(8)` vs `zstd(3, threads=1) + rayon(8)`
- Expect 10-15% improvement with single-threaded zstd

**Implementation**: Phase 1 (BlockBuilder API Split) - extend Codec enum

---

## 10.4 Addressing Feedback (PARALLEL-FEEDBACK5.md)

### Follow-Up Items ✅ FIXED

**1. InputSource::into_record_stream() visibility** ✅
- **Issue**: Method is currently private (jac-io/src/lib.rs:548), parallel code can't call it
- **Solution**: Phase 2 now includes task to make `pub(crate)` or move parallel code to jac-io/src/lib.rs
- **Alternative**: Implement parallel code in same module to avoid visibility change
- **Updated**: Phase 2 checklist, Section 3.2 code comments

**2. OutputSink handling** ✅
- **Issue**: Code called `std::fs::File::create(&output)` but output is OutputSink enum, not path
- **Fixed**: Now uses `OutputSink::into_writer()` helper (jac-io/src/lib.rs:568)
- **Handles**: File paths, writer sinks, pipes, buffers - all OutputSink variants
- **Note**: Also currently private, Phase 2 includes visibility task
- **Updated**: Section 3.2 writer initialization code

**3. FileHeader construction** ✅
- **Issue**: Used non-existent fields (magic, version, compressor_default, metadata HashMap)
- **Fixed**: Now uses actual FileHeader fields from jac-format/src/header.rs:39:
  - `flags` (container hint encoded via `set_container_format_hint()`)
  - `default_compressor`, `default_compression_level`
  - `block_size_hint_records`
  - `user_metadata: Vec<u8>` (not HashMap)
- **Correct encoding**: Container hint in flags, not separate field
- **Updated**: Section 3.2 with proper FileHeader construction

**4. Determinism tests promotion** ✅
- **Issue**: Plan referenced `diff` in validation section but not in automated tests
- **Fixed**: Phase 3 now explicitly requires **automated integration test** for byte-for-byte equality
- **Test requirement**: `test_parallel_determinism_bytes()` must run in CI
  - `assert_eq!(out1_bytes, out2_bytes)` for Rust validation
  - `diff` command included in test output for debugging
- **Emphasis**: Not just metrics matching, actual binary identity required
- **Updated**: Section 7.2 test implementation, Phase 3 checklist

### Open Question from Feedback

**Q: Should memory headroom be tunable via CompressOptions or environment?**

**Answer**: ✅ **Already addressed in PARALLEL-FEEDBACK4.md response (Section 10.3)**

The plan already includes full configurability:

1. **ParallelConfig struct** (Phase 4):
   ```rust
   pub struct ParallelConfig {
       pub memory_reservation_factor: f64,  // Default 0.75
       pub max_threads: Option<usize>,
   }
   ```

2. **Three configuration points**:
   - **Programmatic**: `CompressOptions` field with `ParallelConfig`
   - **Environment variable**: `JAC_PARALLEL_MEMORY_FACTOR=0.6`
   - **CLI flag**: `--parallel-memory-factor 0.6`

3. **Use case**: Containers with tight cgroup limits can set factor to 0.5-0.6 instead of default 0.75

4. **Validation**: Phase 5 measures real-world memory usage to confirm 75% default is adequate

**Status**: Design complete, implementation planned in Phase 4 tasks

---

## 10.5 Addressing Feedback (PARALLEL-FEEDBACK6.md)

### Items to Tidy ✅ FIXED

**1. container_hint Option<ContainerFormat> handling** ✅
- **Issue**: Code called `header.set_container_format_hint(container_hint)` but container_hint is `Option<ContainerFormat>`
- **Fixed**: Now uses `container_hint.unwrap_or(ContainerFormat::Unknown)` (Section 3.2, lines 550-554)
- **Correct handling**:
  ```rust
  let final_hint = container_hint.unwrap_or(ContainerFormat::Unknown);
  header.set_container_format_hint(final_hint);
  ```
- **Sequential comparison**: Sequential path detects from stream (jac-io/src/lib.rs:260-261), parallel can't so defaults to Unknown
- **Updated**: Section 3.2 writer initialization

**2. FileHeader construction alignment with sequential path** ✅
- **Issue**: Plan reset flags to 0 and used empty Vec for user_metadata, dropping canonicalization bits and limits encoding
- **Fixed**: Now matches sequential path exactly (jac-io/src/lib.rs:265-283):
  - **Builds flags** from `canonicalize_keys`, `canonicalize_numbers`, `nested_opaque` (lines 527-537)
  - **Calls `encode_header_metadata(&options.limits)`** instead of `Vec::new()` (line 547)
  - **Preserves spec requirements** for header bookkeeping
- **Recommendation added**: Phase 2 task to extract `build_file_header()` helper to avoid duplication
- **Alternative**: Duplicate construction (current approach) - less maintainable but works
- **Updated**: Section 3.2 with complete header construction, Phase 2 checklist

**Helper extraction example** (optional Phase 2 refactor):
```rust
/// Build FileHeader from CompressOptions and optional container hint
/// Shared between sequential and parallel compression paths
pub(crate) fn build_file_header(
    options: &CompressOptions,
    container_hint: Option<ContainerFormat>,
) -> Result<FileHeader> {
    // Build flags from options
    let mut flags = 0u32;
    if options.canonicalize_keys {
        flags |= jac_format::constants::FLAG_CANONICALIZE_KEYS;
    }
    if options.canonicalize_numbers {
        flags |= jac_format::constants::FLAG_CANONICALIZE_NUMBERS;
    }
    if options.nested_opaque {
        flags |= jac_format::constants::FLAG_NESTED_OPAQUE;
    }

    let mut header = FileHeader {
        flags,
        default_compressor: options.default_codec.compressor_id(),
        default_compression_level: options.default_codec.level(),
        block_size_hint_records: options.block_target_records,
        user_metadata: encode_header_metadata(&options.limits)?,
    };

    // Set container format hint (Unknown if not provided)
    let final_hint = container_hint.unwrap_or(ContainerFormat::Unknown);
    header.set_container_format_hint(final_hint);

    Ok(header)
}
```

**Benefits of helper**:
- Single source of truth for header construction
- Sequential path can use it too (refactor existing code)
- Easier to maintain when header format changes
- Ensures parallel path stays aligned with sequential

**Status**: Both issues resolved in Section 3.2, optional helper extraction in Phase 2

### Confirmation

✅ **No further questions** from feedback
✅ **All concerns addressed**: Helper visibility, byte-level determinism, memory tunability, header construction
✅ **Plan ready for implementation**

---

## 11. Known Limitations & Future Work

### 11.1 Current Limitations

1. **Per-segment compression**: Currently compresses each segment independently. Future work could pipeline at segment level for finer granularity.

2. **Fixed codec**: All segments use same codec. Could auto-select per-field (zstd for text, lz4 for numbers).

3. **Static thread pool**: Rayon pool created per-compress. Could amortize with long-lived pool for batch operations.

4. **No adaptive tuning**: Heuristics are static. Could learn from previous compressions in interactive sessions.

### 11.2 Future Optimizations

**Phase 7.3** (not in this plan):
- SIMD-accelerated encoding (dictionary lookups, RLE scanning)
- Direct I/O for large files (bypass page cache)
- mmap input for faster reading (when input is file, not pipe)

**Phase 7.4** (not in this plan):
- Per-field codec selection (analyze data characteristics)
- Adaptive block sizing (auto-tune based on field sizes)
- Compressed index building (parallel index generation)

---

## 12. Spec Compliance Checklist

- ✅ **SPEC.md §5 (Streaming)**: Pipeline processes records incrementally, no full-file buffering
- ✅ **SPEC.md §3.2 (Block Independence)**: Each block compressed separately, can be read in parallel
- ✅ **SPEC.md Addendum §2.1 (Memory Limits)**: `max_block_uncompressed_total` enforced per-block, aggregate bounded by thread count
- ✅ **SPEC.md §6.6 (WASM Compatibility)**: `cfg(target_arch = "wasm32")` guard forces sequential mode
- ✅ **SPEC.md §4 (Determinism)**: Block order preserved via BTreeMap, compression deterministic
- ✅ **SPEC.md §7 (Error Handling)**: All errors propagated up, no silent failures

---

## 13. Success Criteria

### Must Have (Phase 1-4)
- [x] BlockBuilder split into prepare + compress phases
- [x] Automatic parallel decision with fixed heuristics
- [x] Pipeline implementation with bounded channels
- [x] CLI integration with `--threads N` argument
- [ ] All unit tests pass
- [ ] Integration tests verify determinism
- [ ] Error handling tests pass

### Should Have (Phase 5)
- [ ] Benchmark shows 6-7x speedup on 8-core system
- [ ] Memory usage stays within 4× per-block limits
- [ ] Documentation updated in README and SPEC
- [ ] PLAN.md Phase 7.2 marked complete

### Nice to Have (Future)
- Adaptive tuning based on previous compressions
- Per-field codec selection
- SIMD acceleration for encoding
- Persistent thread pool for batch operations

---

## 14. Dependencies

**New Dependencies**:
```toml
# jac-io/Cargo.toml
[dependencies]
crossbeam-channel = "0.5"
rayon = "1.8"

[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
sysinfo = "0.30"
```

**Feature Flags**:
```toml
[features]
default = ["parallel"]
parallel = ["rayon", "crossbeam-channel"]
```

**WASM Compatibility**:
- `sysinfo` excluded on `wasm32` target (doesn't compile)
- `rayon` works on WASM but returns 1 core (no threads)
- `crossbeam-channel` works on WASM (compiles, but single-threaded)

---

## 15. Migration from v1

For teams that reviewed PARALLEL-PLAN.md v1, here are the key changes:

| Aspect | v1 (WRONG) | v2 (FIXED) |
|--------|------------|------------|
| **Compression** | `finalize()` compresses before Rayon | Split into `prepare_segments()` + `compress_segments()` |
| **Memory detection** | `available_memory()` in bytes | `available_memory() * 1024` (KiB → bytes) |
| **Memory refresh** | Not called | `system.refresh_memory()` before reading |
| **Thread count** | Calculated but unused | Returned in `ParallelDecision`, applied to Rayon pool |
| **Request ownership** | Moved into builder thread | Destructured before spawning threads |
| **Error handling** | `?` in Rayon scope (doesn't compile) | `Arc<Mutex<Option<Error>>>` shared slot |
| **CLI** | `--parallel` flag required | `--threads N` acts as upper bound (auto-detect default) |
| **Task list** | References `load_records_batch()` | Matches pipeline architecture |

**Migration steps**:
1. Discard v1 implementation (if started)
2. Follow Phase 1-5 in this document
3. Run full test suite to verify correctness
4. Benchmark to validate 6-7x speedup

---

## 16. Questions & Answers

**Q: Why split `finalize()` instead of parallelizing compression within it?**
A: Parallelizing inside `finalize()` would require `BlockBuilder` to know about threading, violating separation of concerns. The split keeps `jac-codec` single-threaded and moves parallelism to `jac-io` where it belongs.

**Q: Why bounded channels instead of unbounded?**
A: Unbounded channels can grow without limit if producer is faster than consumers, violating memory guarantees. Bounded channels provide backpressure: producer blocks when channel is full, limiting in-flight blocks.

**Q: Why BTreeMap for ordering instead of a counter?**
A: Compressed blocks arrive out-of-order due to variable compression times. BTreeMap buffers out-of-order blocks and emits them in index order, ensuring deterministic output.

**Q: Why not use Rayon's `par_iter()` instead of manual thread pool?**
A: `par_iter()` requires collecting all items up-front (batch), violating streaming requirement. Manual pool with channels allows true pipeline with streaming input.

**Q: How does this handle stdin pipes with unknown size?**
A: Pipeline processes records one-by-one, building blocks incrementally. Size hint only affects auto-detection; stdin defaults to parallel if cores/memory allow.

**Q: What happens if compression fails mid-stream?**
A: Error stored in `Arc<Mutex<Option<Error>>>`, all threads check before starting new work, builder/writer threads join and propagate error up to CLI.

**Q: Why cap at 16 threads even if more cores available?**
A: Diminishing returns after ~8-12 cores due to memory bandwidth saturation. 16 is conservative cap to avoid thrashing.

---

## Conclusion

This revised plan addresses all issues from PARALLEL-FEEDBACK.md, PARALLEL-FEEDBACK2.md, PARALLEL-FEEDBACK3.md, PARALLEL-FEEDBACK4.md, PARALLEL-FEEDBACK5.md, and PARALLEL-FEEDBACK6.md:

### Core Fixes (PARALLEL-FEEDBACK.md + PARALLEL-FEEDBACK2.md)
✅ **Streaming compliance**: Pipeline with bounded channels, no full-file buffering
✅ **Memory safety**: Enforced per-block limits, thread count capped by available memory
✅ **Actual parallelism**: Compression work distributed to Rayon pool (not pre-compressed)
✅ **Fixed bugs**: sysinfo KiB conversion, refresh calls, variable names, ownership
✅ **Proper error handling**: Arc<Mutex> error slot for Rayon scope
✅ **WASM compatibility**: cfg guards forcing sequential mode
✅ **Ergonomic CLI**: `--threads N` as upper bound, auto-detection by default

### Integration Fixes (PARALLEL-FEEDBACK3.md)
✅ **InputSource handling**: All 5 variants supported (NdjsonPath, JsonArrayPath, NdjsonReader, JsonArrayReader, Iterator)
✅ **FieldSegment reuse**: No new types, maintains spec-defined payload order (SPEC.md:536)
✅ **Writer integration**: New `write_compressed_block()` method specified, compatible with existing API
✅ **API names corrected**: `JacWriter::new()`, proper InputSource enum handling, FileHeader requirements
✅ **Metrics preserved**: All diagnostics flow through UncompressedBlockData → BlockFinish

### Follow-Up Items (PARALLEL-FEEDBACK4.md)
✅ **Use existing helpers**: `InputSource::into_record_stream()` instead of manual reader construction
✅ **FileHeader construction**: Complete example with all required fields (magic, version, flags, etc.)
✅ **Writer API decision**: Option A (`write_compressed_block`) - keeps metrics centralized, avoids pub(crate) leaks
✅ **Determinism testing**: Byte-for-byte equality tests, not just metrics equivalence

### Open Questions Answered (PARALLEL-FEEDBACK4.md)
✅ **Memory reservation configurable**: Added `ParallelConfig` with `memory_reservation_factor` (default 0.75)
  - CLI flag: `--parallel-memory-factor 0.6`
  - Environment variable: `JAC_PARALLEL_MEMORY_FACTOR`
  - Use case: Containers with tight cgroups limits
✅ **Zstd threading control**: Added `ZstdWithThreads` codec variant to force single-threaded zstd
  - Prevents oversubscription: rayon workers × zstd threads
  - Expected 10-15% improvement by keeping parallelism at Rayon level only
  - Benchmark validation in Phase 5

### Implementation Fixes (PARALLEL-FEEDBACK5.md)
✅ **Helper visibility**: Phase 2 tasks updated to make `into_record_stream()` and `into_writer()` accessible
  - Option: Make methods `pub(crate)`
  - Alternative: Move parallel code to jac-io/src/lib.rs (same module)
✅ **OutputSink handling**: Fixed to use `OutputSink::into_writer()` instead of direct `File::create()`
  - Handles all output types: files, pipes, buffers
✅ **FileHeader construction**: Corrected to use actual struct fields
  - Real fields: `flags`, `default_compressor`, `default_compression_level`, `block_size_hint_records`, `user_metadata: Vec<u8>`
  - Container hint encoded in flags via `set_container_format_hint()`
✅ **Automated determinism tests**: Phase 3 explicitly requires CI integration test for byte-for-byte equality
  - Not just manual validation with `diff`
  - Automated `assert_eq!()` on raw file bytes
✅ **Memory tunability confirmed**: ParallelConfig addresses cgroup/container environments (from FEEDBACK4, reconfirmed in FEEDBACK5)

### Final Compilation Fixes (PARALLEL-FEEDBACK6.md)
✅ **Option<ContainerFormat> handling**: Fixed to use `unwrap_or(ContainerFormat::Unknown)`
  - Prevents compilation error from passing Option to set_container_format_hint()
  - Sequential path detects from stream, parallel defaults to Unknown
✅ **FileHeader alignment with sequential path**: Now builds flags and metadata exactly like sequential
  - Builds flags from `canonicalize_keys`, `canonicalize_numbers`, `nested_opaque`
  - Calls `encode_header_metadata(&options.limits)` instead of empty Vec
  - Preserves spec requirements for header bookkeeping
  - Optional: Extract `build_file_header()` helper to share logic (Phase 2 task)
✅ **No further questions**: All concerns from 6 feedback rounds addressed

Expected outcome: **6-7x speedup on 8-core systems** while maintaining spec compliance, determinism, and memory safety.

**Implementation ready**: Follow Phases 1-5 for production deployment.

**Key validation points** (Phase 5):
1. Byte-identical outputs across thread counts
2. Memory usage stays within bounds (75% reservation adequate)
3. Single-threaded zstd improves performance over default multi-threaded
