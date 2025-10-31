//! Parallel processing support utilities.
//!
//! Phase 2 introduces automatic decision logic for selecting between the
//! sequential and parallel compression paths. Later phases will add the
//! actual parallel pipeline that consumes this decision.

use crate::InputSource;
use jac_format::{Limits, Result};

#[cfg(not(target_arch = "wasm32"))]
use sysinfo::System;

#[cfg(not(target_arch = "wasm32"))]
use crate::{build_file_header, writer::JacWriter, CompressOpts, CompressRequest, CompressSummary};
#[cfg(not(target_arch = "wasm32"))]
use jac_codec::{
    compress_block_segments, configure_codec_for_parallel, BlockBuilder, BlockFinish,
    TryAddRecordOutcome,
};
#[cfg(not(target_arch = "wasm32"))]
use jac_format::JacError;
#[cfg(not(target_arch = "wasm32"))]
use rayon::ThreadPoolBuilder;
#[cfg(not(target_arch = "wasm32"))]
use std::{
    collections::BTreeMap,
    io::{BufWriter, Write},
    mem,
    sync::{mpsc::sync_channel, Arc, Mutex},
    thread,
};

const SMALL_FILE_THRESHOLD_BYTES: u64 = 10 * 1024 * 1024;
const MEMORY_PER_THREAD_MULTIPLIER: u64 = 2;
const MAX_PARALLEL_THREADS: usize = 16;
const DEFAULT_MEMORY_RESERVATION_FACTOR: f64 = 0.75;

/// Configuration controlling how the parallel compression heuristic behaves.
#[derive(Debug, Clone, Copy)]
pub struct ParallelConfig {
    /// Fraction of available memory considered usable for in-flight blocks.
    pub memory_reservation_factor: f64,
    /// Optional cap on worker thread count after applying heuristics.
    pub max_threads: Option<usize>,
}

impl Default for ParallelConfig {
    fn default() -> Self {
        Self {
            memory_reservation_factor: DEFAULT_MEMORY_RESERVATION_FACTOR,
            max_threads: None,
        }
    }
}

/// Decision returned by the parallel heuristic describing whether parallel
/// compression should be enabled along with diagnostic metadata.
#[derive(Debug, Clone)]
pub struct ParallelDecision {
    /// Whether the parallel pipeline should be used.
    pub use_parallel: bool,
    /// Number of worker threads to use when parallelism is enabled.
    pub thread_count: usize,
    /// Human-readable explanation of the chosen mode.
    pub reason: String,
    /// Estimated peak memory consumption in bytes.
    pub estimated_memory: u64,
    /// Available memory reported by the system in bytes.
    pub available_memory: u64,
    /// Normalized reservation factor that was applied.
    pub memory_reservation_factor: f64,
    /// Maximum threads allowed after memory limits (before user caps).
    pub memory_limited_thread_count: usize,
}

/// Determine whether parallel compression should be used for the provided
/// input. On non-WASM targets this consults CPU and memory availability;
/// WASM targets always fall back to sequential execution.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn should_use_parallel(
    input_source: &InputSource,
    limits: &Limits,
    config: &ParallelConfig,
) -> Result<ParallelDecision> {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);

    let mut system = System::new();
    system.refresh_memory();
    let available_memory_bytes = system.available_memory().saturating_mul(1024);

    let input_size_hint = match input_source {
        InputSource::NdjsonPath(path) | InputSource::JsonArrayPath(path) => {
            std::fs::metadata(path).map(|meta| meta.len()).ok()
        }
        InputSource::NdjsonReader(_)
        | InputSource::JsonArrayReader(_)
        | InputSource::Iterator(_) => None,
    };

    Ok(evaluate_parallel_decision(
        cores,
        available_memory_bytes,
        limits.max_block_uncompressed_total,
        input_size_hint,
        config,
    ))
}

/// WASM targets default to sequential compression to avoid depending on
/// unavailable system APIs and threading primitives.
#[cfg(target_arch = "wasm32")]
pub(crate) fn should_use_parallel(
    _input_source: &InputSource,
    _limits: &Limits,
    _config: &ParallelConfig,
) -> Result<ParallelDecision> {
    Ok(ParallelDecision {
        use_parallel: false,
        thread_count: 1,
        reason: "WASM target does not support parallelism".into(),
        estimated_memory: 0,
        available_memory: 0,
        memory_reservation_factor: DEFAULT_MEMORY_RESERVATION_FACTOR,
        memory_limited_thread_count: 1,
    })
}

fn evaluate_parallel_decision(
    cores: usize,
    available_memory_bytes: u64,
    max_block_uncompressed_total: usize,
    input_size_hint: Option<u64>,
    config: &ParallelConfig,
) -> ParallelDecision {
    if cores < 2 {
        return ParallelDecision {
            use_parallel: false,
            thread_count: 1,
            reason: "Single-core system detected".into(),
            estimated_memory: 0,
            available_memory: available_memory_bytes,
            memory_reservation_factor: normalized_memory_factor(config.memory_reservation_factor),
            memory_limited_thread_count: 1,
        };
    }

    let memory_factor = normalized_memory_factor(config.memory_reservation_factor);
    let per_block_memory = std::cmp::max(max_block_uncompressed_total as u64, 1u64);
    let memory_per_thread = std::cmp::max(
        per_block_memory.saturating_mul(MEMORY_PER_THREAD_MULTIPLIER),
        1u64,
    );

    let usable_memory = (available_memory_bytes as f64 * memory_factor).floor() as u64;

    let raw_safe_threads = if memory_per_thread == 0 {
        0
    } else {
        (usable_memory / memory_per_thread) as usize
    };
    let max_safe_threads = std::cmp::max(raw_safe_threads, 1);

    let memory_limited_threads =
        std::cmp::min(cores, std::cmp::min(max_safe_threads, MAX_PARALLEL_THREADS));
    let user_cap = config
        .max_threads
        .map(|cap| cap.max(1))
        .unwrap_or(MAX_PARALLEL_THREADS)
        .min(MAX_PARALLEL_THREADS);

    let thread_count = std::cmp::max(1, std::cmp::min(memory_limited_threads, user_cap));

    if thread_count < 2 {
        let estimated_memory = memory_per_thread.saturating_mul(thread_count as u64);
        let reason = if config.max_threads.is_some() && user_cap <= 1 {
            format!(
                "Sequential mode forced by thread cap (--threads 1 or equivalent, reservation factor {:.2})",
                memory_factor,
            )
        } else {
            format!(
                "Insufficient memory for parallel compression: {} cores available, but only enough memory for {} threads ({:.1} MiB available, {:.1} MiB per thread required, reservation factor {:.2})",
                cores,
                max_safe_threads,
                bytes_to_mib(available_memory_bytes),
                bytes_to_mib(memory_per_thread),
                memory_factor,
            )
        };
        return ParallelDecision {
            use_parallel: false,
            thread_count: 1,
            reason,
            estimated_memory,
            available_memory: available_memory_bytes,
            memory_reservation_factor: memory_factor,
            memory_limited_thread_count: memory_limited_threads,
        };
    }

    if let Some(size) = input_size_hint {
        if size > 0 && size < SMALL_FILE_THRESHOLD_BYTES {
            return ParallelDecision {
                use_parallel: false,
                thread_count: 1,
                reason: format!(
                    "Small input file ({:.1} MiB) - parallel overhead exceeds benefit (reservation factor {:.2})",
                    bytes_to_mib(size),
                    memory_factor,
                ),
                estimated_memory: per_block_memory,
                available_memory: available_memory_bytes,
                memory_reservation_factor: memory_factor,
                memory_limited_thread_count: memory_limited_threads,
            };
        }
    }

    let estimated_memory = memory_per_thread.saturating_mul(thread_count as u64);
    let reason = if config.max_threads.is_some() && thread_count < memory_limited_threads {
        format!(
            "Using {}/{} requested threads (memory allows {}, reservation factor {:.2}, estimated peak {:.1} MiB)",
            thread_count,
            user_cap,
            memory_limited_threads,
            memory_factor,
            bytes_to_mib(estimated_memory),
        )
    } else {
        format!(
            "Using {}/{} cores for parallel compression (reservation factor {:.2}, {:.1} MiB estimated peak memory)",
            thread_count,
            cores,
            memory_factor,
            bytes_to_mib(estimated_memory),
        )
    };

    ParallelDecision {
        use_parallel: true,
        thread_count,
        reason,
        estimated_memory,
        available_memory: available_memory_bytes,
        memory_reservation_factor: memory_factor,
        memory_limited_thread_count: memory_limited_threads,
    }
}

fn bytes_to_mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn normalized_memory_factor(factor: f64) -> f64 {
    if !factor.is_finite() || factor <= 0.0 {
        DEFAULT_MEMORY_RESERVATION_FACTOR
    } else if factor > 1.0 {
        1.0
    } else {
        factor
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn execute_compress_parallel(
    request: CompressRequest,
    thread_count: usize,
) -> Result<CompressSummary> {
    let CompressRequest {
        input,
        output,
        options,
        container_hint,
        emit_index,
    } = request;

    let record_stream = input.into_record_stream()?;
    let detected_hint = record_stream.container_format();
    let final_hint = container_hint.unwrap_or(detected_hint);

    let writer_target = output.into_writer()?;
    let buf_writer = BufWriter::new(writer_target);
    let header = build_file_header(&options, Some(final_hint))?;

    let codec_opts = CompressOpts {
        block_target_records: options.block_target_records,
        default_codec: options.default_codec,
        canonicalize_keys: options.canonicalize_keys,
        canonicalize_numbers: options.canonicalize_numbers,
        nested_opaque: options.nested_opaque,
        max_dict_entries: options.max_dict_entries,
        limits: options.limits.clone(),
    };

    let builder_opts = codec_opts.clone();
    let mut writer = JacWriter::new(buf_writer, header, codec_opts)?;

    let worker_codec = configure_codec_for_parallel(options.default_codec, true);

    let (uncompressed_tx, uncompressed_rx) = sync_channel(thread_count);
    let (compressed_tx, compressed_rx) = sync_channel(thread_count);

    let compression_error: Arc<Mutex<Option<JacError>>> = Arc::new(Mutex::new(None));

    let builder_handle = {
        let builder_opts = builder_opts.clone();
        thread::Builder::new()
            .name("jac-builder".to_string())
            .spawn(move || -> Result<()> {
                let mut block_idx = 0usize;
                let mut builder = BlockBuilder::new(builder_opts.clone());
                let mut stream = record_stream;

                while let Some(record_result) = stream.next() {
                    let record = record_result?;
                    match builder.try_add_record(record)? {
                        TryAddRecordOutcome::Added => {}
                        TryAddRecordOutcome::BlockFull { record } => {
                            let full_builder =
                                mem::replace(&mut builder, BlockBuilder::new(builder_opts.clone()));
                            let uncompressed = full_builder.prepare_segments()?;

                            if uncompressed_tx.send((block_idx, uncompressed)).is_err() {
                                return Err(JacError::Internal(
                                    "Compression workers terminated early".into(),
                                ));
                            }
                            block_idx += 1;

                            match builder.try_add_record(record)? {
                                TryAddRecordOutcome::Added => {}
                                TryAddRecordOutcome::BlockFull { .. } => {
                                    return Err(JacError::Internal(
                                        "New block reported full immediately".into(),
                                    ));
                                }
                            }
                        }
                    }
                }

                if !builder.is_empty() {
                    let uncompressed = builder.prepare_segments()?;
                    if uncompressed_tx.send((block_idx, uncompressed)).is_err() {
                        return Err(JacError::Internal(
                            "Compression workers terminated before receiving final block".into(),
                        ));
                    }
                }

                drop(uncompressed_tx);
                Ok(())
            })?
    };

    let pool = ThreadPoolBuilder::new()
        .num_threads(thread_count)
        .thread_name(|idx| format!("jac-compress-{}", idx))
        .build()
        .map_err(|e| JacError::Internal(format!("Failed to create thread pool: {}", e)))?;

    let compression_error_for_workers = Arc::clone(&compression_error);
    let compress_handle = thread::Builder::new()
        .name("jac-compress-coordinator".to_string())
        .spawn(move || -> Result<()> {
            pool.scope(|scope| {
                for (block_idx, uncompressed) in uncompressed_rx {
                    if compression_error_for_workers.lock().unwrap().is_some() {
                        break;
                    }

                    let tx = compressed_tx.clone();
                    let error_slot = Arc::clone(&compression_error_for_workers);

                    scope.spawn(move |_| {
                        match compress_block_segments(uncompressed, worker_codec) {
                            Ok(block_finish) => {
                                if tx.send((block_idx, block_finish)).is_err() {
                                    let mut guard = error_slot.lock().unwrap();
                                    if guard.is_none() {
                                        *guard = Some(JacError::Internal(
                                            "Writer thread terminated early".into(),
                                        ));
                                    }
                                }
                            }
                            Err(err) => {
                                let mut guard = error_slot.lock().unwrap();
                                if guard.is_none() {
                                    *guard = Some(err);
                                }
                            }
                        }
                    });
                }
            });
            drop(compressed_tx);
            Ok(())
        })?;

    let mut pending_blocks: BTreeMap<usize, BlockFinish> = BTreeMap::new();
    let mut next_block_idx = 0usize;
    let mut records_written = 0u64;
    let mut encountered_error = false;

    for (block_idx, block_finish) in compressed_rx {
        if encountered_error {
            break;
        }

        if block_idx == next_block_idx {
            let record_count = block_finish.data.header.record_count as u64;
            if let Err(err) = writer.write_compressed_block(block_finish) {
                let mut slot = compression_error.lock().unwrap();
                if slot.is_none() {
                    *slot = Some(err);
                }
                encountered_error = true;
                break;
            }
            records_written += record_count;
            next_block_idx += 1;

            while let Some(pending) = pending_blocks.remove(&next_block_idx) {
                let record_count = pending.data.header.record_count as u64;
                if let Err(err) = writer.write_compressed_block(pending) {
                    let mut slot = compression_error.lock().unwrap();
                    if slot.is_none() {
                        *slot = Some(err);
                    }
                    encountered_error = true;
                    break;
                }
                records_written += record_count;
                next_block_idx += 1;
            }
            if encountered_error {
                break;
            }
        } else {
            pending_blocks.insert(block_idx, block_finish);
        }
    }

    let builder_result = builder_handle
        .join()
        .map_err(|e| JacError::Internal(format!("Builder thread panicked: {:?}", e)))?;
    if let Err(err) = builder_result {
        return Err(err);
    }

    let compress_result = compress_handle
        .join()
        .map_err(|e| JacError::Internal(format!("Compression thread panicked: {:?}", e)))?;
    if let Err(err) = compress_result {
        return Err(err);
    }

    if let Some(err) = compression_error.lock().unwrap().take() {
        return Err(err);
    }

    if !encountered_error && !pending_blocks.is_empty() {
        return Err(JacError::Internal(
            "Parallel compression pipeline terminated early".into(),
        ));
    }

    let finish = if emit_index {
        writer.finish_with_index()?
    } else {
        writer.finish_without_index()?
    };

    let mut buf_writer = finish.writer;
    buf_writer.flush()?;

    let mut metrics = finish.metrics;
    metrics.records_written = records_written;

    Ok(CompressSummary {
        metrics,
        parallel_decision: None,
    })
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn execute_compress_parallel(
    request: CompressRequest,
    _thread_count: usize,
) -> Result<CompressSummary> {
    // This path should never be selected on WASM targets, but we fall back to sequential
    // execution for completeness.
    super::execute_compress_sequential(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jac_format::Limits;
    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn decision_respects_memory_cap() {
        let decision = evaluate_parallel_decision(
            8,
            4 * GIB,
            Limits::default().max_block_uncompressed_total,
            None,
            &ParallelConfig::default(),
        );

        assert!(decision.use_parallel);
        assert_eq!(decision.thread_count, 6);
    }

    #[test]
    fn single_core_forces_sequential() {
        let decision = evaluate_parallel_decision(
            1,
            16 * 1024 * 1024 * 1024,
            Limits::default().max_block_uncompressed_total,
            None,
            &ParallelConfig::default(),
        );

        assert!(!decision.use_parallel);
        assert_eq!(decision.thread_count, 1);
    }

    #[test]
    fn small_file_prefers_sequential() {
        let decision = evaluate_parallel_decision(
            8,
            16 * 1024 * 1024 * 1024,
            Limits::default().max_block_uncompressed_total,
            Some(5 * 1024 * 1024),
            &ParallelConfig::default(),
        );
        assert!(!decision.use_parallel);
        assert_eq!(decision.thread_count, 1);
    }

    #[test]
    fn iterator_default_parallel_when_resources_allow() {
        let decision = evaluate_parallel_decision(
            8,
            16 * GIB,
            Limits::default().max_block_uncompressed_total,
            None,
            &ParallelConfig::default(),
        );
        assert!(decision.use_parallel);
        assert!(decision.thread_count >= 2);
    }

    #[test]
    fn low_memory_caps_threads() {
        let decision = evaluate_parallel_decision(
            16,
            2 * GIB,
            256 * 1024 * 1024,
            None,
            &ParallelConfig::default(),
        );
        assert!(decision.use_parallel);
        assert_eq!(decision.thread_count, 3);
    }

    #[test]
    fn large_file_prefers_parallel() {
        let decision = evaluate_parallel_decision(
            12,
            32 * GIB,
            Limits::default().max_block_uncompressed_total,
            Some(50 * 1024 * 1024),
            &ParallelConfig::default(),
        );
        assert!(decision.use_parallel);
        assert_eq!(decision.thread_count, 12.min(MAX_PARALLEL_THREADS));
    }

    #[test]
    fn insufficient_memory_forces_sequential() {
        let decision = evaluate_parallel_decision(
            8,
            128 * 1024 * 1024, // 128 MiB total available
            256 * 1024 * 1024, // 256 MiB per block
            None,
            &ParallelConfig::default(),
        );
        assert!(!decision.use_parallel);
        assert_eq!(decision.thread_count, 1);
    }

    #[test]
    fn user_thread_cap_respected() {
        let mut config = ParallelConfig::default();
        config.max_threads = Some(4);
        let decision = evaluate_parallel_decision(
            16,
            32 * GIB,
            Limits::default().max_block_uncompressed_total,
            None,
            &config,
        );
        assert!(decision.use_parallel);
        assert_eq!(decision.thread_count, 4);
        assert!(decision.memory_limited_thread_count >= decision.thread_count);
    }

    #[test]
    fn memory_factor_adjusts_threads() {
        let default_decision = evaluate_parallel_decision(
            16,
            32 * GIB,
            Limits::default().max_block_uncompressed_total,
            None,
            &ParallelConfig::default(),
        );

        let mut tight_config = ParallelConfig::default();
        tight_config.memory_reservation_factor = 0.10;
        let reduced_decision = evaluate_parallel_decision(
            16,
            32 * GIB,
            Limits::default().max_block_uncompressed_total,
            None,
            &tight_config,
        );

        assert!(reduced_decision.use_parallel);
        assert!(
            reduced_decision.thread_count < default_decision.thread_count,
            "expected reduced thread count ({} < {})",
            reduced_decision.thread_count,
            default_decision.thread_count
        );
        assert!(
            (reduced_decision.memory_reservation_factor - 0.10).abs() < 1e-6,
            "expected memory factor ~= 0.10 but was {:.4}",
            reduced_decision.memory_reservation_factor
        );
    }
}
