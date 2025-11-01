# JAC Project Context for AI Agents

This document provides context and guidance for AI coding assistants working on the JAC (JSON-Aware Compression) project.

---

## Project Overview

**JAC** is a binary container and encoding format for JSON designed for archival workloads where compression ratio is the top priority. It uses columnar layout, dictionary encoding, bit-packing, and per-field compression (Zstandard) to achieve superior compression while enabling partial decoding (field projection).

**Key Files:**
- `SPEC.md` - Complete technical specification (Draft 0.9.1)
- `PLAN.md` - Implementation roadmap and phased plan
- This file - Context for AI agents

---

## Architecture

### Crate Topology

```
jac/
├─ jac-format/         # Core primitives (no I/O)
│  ├─ constants.rs     # Magic numbers, type tags
│  ├─ varint.rs        # ULEB128, ZigZag encoding
│  ├─ bitpack.rs       # Presence bitmaps, type-tag packing
│  ├─ checksum.rs      # CRC32C
│  ├─ error.rs         # Error types
│  ├─ limits.rs        # Security limits
│  ├─ header.rs        # File header
│  ├─ block.rs         # Block header & directory
│  ├─ footer.rs        # Optional index footer
│  ├─ decimal.rs       # Arbitrary-precision decimals
│  └─ types.rs         # TypeTag enum
│
├─ jac-codec/          # Encoder/decoder engines
│  ├─ column.rs        # ColumnBuilder (records → columnar)
│  ├─ segment.rs       # Field segment encoding
│  ├─ block_builder.rs # Aggregates columns into blocks
│  ├─ segment_decode.rs# Field segment decoder
│  └─ block_decode.rs  # Block decoder & projection
│
├─ jac-io/             # File I/O layer
│  ├─ writer.rs        # JacWriter (streaming encoder)
│  ├─ reader.rs        # JacReader (streaming decoder)
│  ├─ parallel.rs      # Rayon-based parallelism
│  ├─ wrapper/         # Input preprocessing (Phase 1: Pointer mode)
│  │  ├─ mod.rs        # Module documentation and exports
│  │  ├─ error.rs      # WrapperError with remediation helpers
│  │  ├─ pointer.rs    # RFC 6901 JSON Pointer envelope extraction
│  │  └─ utils.rs      # Shared pointer parsing and navigation
│  └─ lib.rs           # High-level APIs (compress, decompress, project)
│
└─ jac-cli/            # Command-line tool
   └─ main.rs          # pack, unpack, ls, cat commands
```

### Dependency Flow

```
jac-cli → jac-io → jac-codec → jac-format
```

Each crate only depends on layers below it. `jac-format` has zero I/O dependencies.

---

## Key Design Decisions

### 1. **Columnar Storage with Union Types**
- Each field is stored as a column across records in a block
- Type tags (3-bit) allow schema drift (a field can have different types across records)
- No up-front schema required

### 2. **Presence vs Null Distinction**
- **Absent**: Key not present in record (presence bit = 0)
- **Null**: Key present with value `null` (presence bit = 1, type tag = 0)
- This distinction is semantically important and MUST be preserved

### 3. **Per-Field Compression**
- Each field segment is independently compressed (default: Zstandard)
- Enables projection: decode only requested fields without scanning entire blocks
- Trade-off: slightly lower compression ratio vs whole-block compression, but enables selective decoding

### 4. **Exact Decimal Encoding**
- Non-integer numbers stored as (sign, digits, exponent) to preserve exact value
- Wire format: ASCII digits ('0'..'9'), not binary
- Ensures semantic equality on round-trip (not byte-identical formatting)

### 5. **Dictionary Encoding Heuristics**
- Dictionary encoding is used when: `distinct_count <= min(max_dict_entries, max(2, total_strings / 4))`
- This balances compression ratio with memory usage
- Dictionary entries are stored in first-occurrence order for better locality

### 6. **Delta Encoding for Integers**
- Delta encoding is used for monotonic integer sequences (timestamps, IDs)
- Applied when: sequence is strictly increasing and `delta_ratio < 0.5`
- Delta ratio = `(max_delta - min_delta) / (max_value - min_value)`
- Stores first value as-is, then deltas (varint-encoded)

### 7. **Block-Based Structure**
- Files divided into blocks (default: 100k records per block)
- Enables parallelism, seekability, and damage isolation
- Each block has its own header, directory, and CRC32C

---

## Critical Specification Details

### Encoding Order (§4.7, Addendum §1.1)

Field segment payload MUST be in this exact order:
1. Presence bitmap
2. Type-tag stream (3-bit packed)
3. String dictionary (if any)
4. Boolean substream (bit-packed)
5. Integer substream (varint/delta)
6. Decimal substream
7. String substream

**Rationale:** Small structures first for fast projection/skipping.

### Size Formulas

```rust
presence_bytes = (record_count + 7) >> 3
tag_bytes = ((3 * present_count) + 7) >> 3
```

### Type Tag Codes (3-bit)

| Code | Type    | Notes                          |
|------|---------|--------------------------------|
| 0    | null    | Present but null               |
| 1    | bool    | Bit-packed substream           |
| 2    | int     | i64 varint (zigzag)            |
| 3    | decimal | Exact decimal (arbitrary prec) |
| 4    | string  | Dictionary or raw              |
| 5    | object  | Minified JSON subdoc           |
| 6    | array   | Minified JSON subdoc           |
| 7    | reserved| MUST reject (UnsupportedFeature) |

### Security Limits (Addendum §2.1)

**Default / Hard Maximum:**
- `max_records_per_block`: 100,000 / 1,000,000
- `max_fields_per_block`: 4,096 / 65,535
- `max_segment_uncompressed_len`: 64 MiB (hard)
- `max_block_uncompressed_total`: 256 MiB (hard)
- `max_dict_entries_per_field`: 4,096 / 65,535
- `max_string_len_per_value`: 16 MiB (hard)
- `max_decimal_digits_per_value`: 65,536 (hard)

**All decoders MUST enforce hard limits to prevent decompression bombs.**

### Endianness & Alignment

- **Endianness:** All fixed-width integers are **little-endian**
- **Alignment:** No alignment required; segments can start at any byte offset
- **Lengths:** ULEB128 (unsigned varint) unless explicitly stated otherwise

---

## Common Tasks

### Adding a New Encoding

1. Update `encoding_flags` bitfield constants in `jac-format/src/constants.rs`
2. Implement encoder logic in `jac-codec/src/column.rs` (ColumnBuilder::finalize)
3. Implement decoder logic in `jac-codec/src/segment_decode.rs`
4. Add tests for round-trip
5. Update heuristics in PLAN.md Phase 4.1

### Adding a New Compressor

1. Add constant in `jac-format/src/constants.rs` (e.g., `COMPRESSOR_BROTLI = 2`)
2. Update `FieldSegment::compress()` in `jac-codec/src/segment.rs`
3. Update `FieldSegmentDecoder::new()` in `jac-codec/src/segment_decode.rs`
4. Update spec §6 and add to `DecompressOpts`

### Adding a New CLI Command

1. Add subcommand to `jac-cli/src/main.rs` (using clap)
2. Implement handler using high-level APIs from `jac-io`
3. Add integration test in `jac-cli/tests/`
4. Update README.md with usage example

### Working with Wrappers

**Overview:** Wrappers are input preprocessing transformations for enveloped JSON. They extract target arrays/objects before compression. The original envelope structure is NOT preserved in .jac files.

**Key Concepts:**

1. **Wrappers vs Core Format**
   - Wrappers are input-only preprocessing (not part of .jac encoding)
   - Output is always flattened records
   - Use `WrapperConfig` enum to specify mode (None, Pointer, Sections, KeyedMap)

2. **Limit Relationships**
   - `WrapperLimits`: Input preprocessing limits (depth, buffer, pointer length)
   - `Limits`: Output encoding limits (segment size, records per block)
   - Both must be enforced independently

3. **Phase Status**
   - Phase 1 (✅): Pointer mode - RFC 6901 JSON Pointer extraction
   - Phase 2 (⏳): Sections mode - Multi-array concatenation
   - Phase 3 (⏳): KeyedMap mode - Flatten object-of-objects

**Module Structure (`jac-io/src/wrapper/`):**
- `mod.rs` - Public exports and module docs
- `error.rs` - `WrapperError` with actionable remediation suggestions
- `pointer.rs` - `PointerArrayStream` implementing RFC 6901 extraction
- `utils.rs` - Shared utilities (pointer parsing, navigation, validation)

**Adding a New Wrapper Mode (Future):**

1. Add variant to `WrapperConfig` enum in `jac-io/src/lib.rs`
2. Create new file in `jac-io/src/wrapper/` (e.g., `sections.rs`)
3. Implement iterator that yields `serde_json::Map<String, Value>`
4. Update `InputSource::into_record_stream()` match to handle new mode
5. Add CLI flags with `requires`/`conflicts_with` attributes
6. Create integration test file in `jac-cli/tests/`
7. Add test fixtures in `jac-cli/tests/fixtures/wrapper/`
8. Update README.md and AGENTS.md with examples

**Testing Checklist for Wrappers:**
- [ ] Unit tests in wrapper module (parsing, validation, limits)
- [ ] Integration tests with real JSON fixtures (success + error cases)
- [ ] CLI flag validation (depth, buffer, pointer length limits)
- [ ] Security tests (exceed hard limits, malicious inputs)
- [ ] Regression tests (NDJSON/array without wrapper unchanged)
- [ ] Error message quality (actionable remediation hints)

**Wrapper Mode Examples:**

1. **Pointer Mode (Phase 1 - ✅ Complete)**
   ```bash
   # Input: {"data": [{"id": 1}, {"id": 2}]}
   jac pack api.json -o output.jac --wrapper-pointer /data

   # Decompression: jac unpack output.jac
   # Output: NDJSON stream with flattened records (envelope lost)
   # {"id": 1}
   # {"id": 2}
   ```

2. **Sections Mode (Phase 2 - ✅ Complete)**
   ```bash
   # Input: {"users": [{"id": 1, "name": "alice"}], "admins": [{"id": 2, "name": "bob"}]}
   jac pack input.json -o output.jac --wrapper-sections users admins

   # Output records include "_section" field:
   # {"id": 1, "name": "alice", "_section": "users"}
   # {"id": 2, "name": "bob", "_section": "admins"}

   # Decompression: jac unpack output.jac
   # Yields flattened NDJSON (envelope lost, section labels preserved)

   # Custom label field:
   jac pack input.json -o output.jac \
     --wrapper-sections users admins \
     --wrapper-section-label-field source
   # Records: {"id": 1, "name": "alice", "source": "users"}

   # Disable label injection:
   jac pack input.json -o output.jac \
     --wrapper-sections users admins \
     --wrapper-section-no-label
   # Records: {"id": 1, "name": "alice"} (no "_section" field)
   ```

3. **KeyedMap Mode (Phase 3 - ⏳ Not Yet Implemented)**
   ```bash
   # Input: {"alice": {"age": 30}, "bob": {"age": 25}}
   jac pack input.json -o output.jac --wrapper-map

   # Expected output:
   # {"_key": "alice", "age": 30}
   # {"_key": "bob", "age": 25}
   ```

---

## Testing Strategy

### Unit Tests
- **jac-format**: Every encoding/decoding function (varint, bitpack, decimal)
- **jac-codec**: Column building, segment encode/decode
- Test edge cases: 0 records, 1 record, boundary sizes

### Integration Tests
- **jac-io**: Full file round-trips (NDJSON → .jac → NDJSON)
- Projection tests (extract field → verify values)
- Parallel encoding/decoding

### Conformance Tests
- Implement spec §12.1 test vector (4 NDJSON records)
- Verify field encodings: ts (delta int), level (dict), user (dict), error (absent)
- Projection output: `user` field → `["alice","alice","bob","carol"]`

### Fuzz Tests
- Malformed varint streams
- Corrupted block headers
- Dictionary index out-of-bounds
- Use `cargo-fuzz` or `proptest`

### Error Tests
- Every error variant in `JacError` should have a test that triggers it
- CRC mismatch, truncated files, exceeded limits, unknown versions

---

## Code Conventions

### Error Handling
- Use `Result<T, JacError>` (aliased as `jac_format::Result<T>`)
- Never panic in production code; return errors
- Use `thiserror` for error enum

### Memory Safety
- Avoid `unsafe` except in audited hot paths (e.g., SIMD varint decoding)
- All limits must be checked before allocations
- Use `SmallVec` for small fixed-size buffers (e.g., varint encoding)

### Rust Style
- Follow StandardJS format (user preference, though this is Rust - interpret as: clean, idiomatic Rust)
- Run `cargo fmt` before commits
- Run `cargo clippy` and fix warnings
- Use `#![deny(unsafe_code)]` in crates where unsafe isn't needed

### Naming
- Use spec terminology: "record" not "row", "field" not "column name", "absent" not "missing"
- Function names: `encode_uleb128`, `decode_uleb128` (not `uleb128_encode`)
- Type names: `FileHeader`, `BlockHeader`, `TypeTag` (PascalCase)

---

## Development Environment

### Rust Toolchain Setup
- **Project uses Rust 1.80.0** (specified in `rust-toolchain.toml`)
- If `cargo` or `rustc` commands fail with "command not found":
  1. Check if Rust is installed: `which rustc` or `which cargo`
  2. If not installed, install via rustup: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
  3. Source the environment: `source ~/.cargo/env` or restart terminal
  4. Verify installation: `cargo --version` and `rustc --version`
- The project will automatically use the correct Rust version via `rust-toolchain.toml`
- If you see "rustc not found" errors, the Rust toolchain needs to be installed or sourced

### Common Commands
- `cargo test` - Run all tests
- `cargo test -p jac-format` - Run tests for specific crate
- `cargo build` - Build all crates
- `cargo check` - Check compilation without building
- `cargo fmt` - Format code
- `cargo clippy` - Run linter

---

## Performance Considerations

### Hot Paths (profile before optimizing)
1. Varint encoding/decoding (called per value)
2. Type-tag packing/unpacking
3. Dictionary lookups (use `ahash` for speed)
4. Zstd compression (use high levels for archival: 15-19)

### Parallelism
- Use `rayon` for block-level parallelism
- Blocks are independent → perfect for parallel compression/decompression
- Ensure deterministic output (block order preserved)

### Memory Budgeting
- Block builder should track memory usage
- Consider streaming large blocks to disk if near limits
- Reuse allocations where possible (object pools for records)

---

## Common Pitfalls

### ❌ Don't assume byte-identical JSON round-trip
- Keys may be reordered (lexicographic for compression)
- Whitespace is not preserved
- Number formatting may differ (`1e6` vs `1000000`)
- **Only semantic equality is guaranteed**

### ❌ Don't ignore limits
- Every decoder MUST enforce `Limits` to prevent OOM/decompression bombs
- Check lengths before allocating buffers

### ❌ Don't confuse absent and null
- `{"key": null}` → presence=1, tag=0 (null)
- `{}` (no "key") → presence=0 (absent)
- These are semantically different and MUST be preserved

### ❌ Don't use tag value 7
- Reserved for future use; MUST reject with `UnsupportedFeature`

### ❌ Don't pack segments in wrong order
- Segment order is normative (§4.7); decoders rely on it for efficient skipping

---

## Debugging Tips

### Inspecting .jac Files
```bash
# List blocks and fields
jac ls file.jac

# Extract specific field
jac cat file.jac --field userId

# Decompress to NDJSON for inspection
jac unpack file.jac -o debug.ndjson --ndjson
```

### Decoder Issues
1. Check file/block magic bytes first
2. Verify CRC32C (most corruption detected here)
3. Validate presence_bytes and tag_bytes sizes match formulas
4. Check encoding_flags for dictionary/delta mode

### Encoder Issues
1. Ensure field names are UTF-8 and not too long
2. Check dictionary threshold logic (distinct count)
3. Verify segment offsets are computed correctly (cumulative)
4. Profile to find bottlenecks (likely compression or dictionary building)

---

## Useful References

### Spec Sections
- **§3**: File & block structure
- **§4**: Field segments & encodings (critical)
- **§4.7 + Addendum §1.1**: Segment order (normative)
- **§8**: Error handling
- **§9**: Implementation blueprint
- **§12**: Test vectors
- **Addendum §2.1**: Limits (security)

### External Docs
- [ULEB128](https://en.wikipedia.org/wiki/LEB128) - Variable-length integer encoding
- [ZigZag](https://developers.google.com/protocol-buffers/docs/encoding#signed-ints) - Signed-to-unsigned mapping
- [Zstandard](https://github.com/facebook/zstd) - Compression library
- [CRC32C](https://en.wikipedia.org/wiki/Cyclic_redundancy_check) - Castagnoli CRC

---

## When in Doubt

1. **Check SPEC.md first** - It's the source of truth
2. **MUST/SHOULD/MAY** - Follow RFC 2119 keywords strictly
3. **Ask clarifying questions** - Don't make assumptions about ambiguous behavior
4. **Write tests** - If behavior is unclear, write a test to validate understanding
5. **Update this file** - If you discover important context, add it here

---

## Current Status

**Implementation Phase:** Phase 8 (CLI Tool Completion) – 🚧 In Progress

**Completed in Phase 0:**
- ✅ Rust workspace initialized with proper crate topology
- ✅ CI/CD pipeline configured (GitHub Actions)
- ✅ Rust toolchain pinned to 1.80.0
- ✅ Test data fixtures created (conformance, edge cases)
- ✅ Documentation skeleton (README, CHANGELOG, SPEC-COMPLIANCE)
- ✅ All crates compile successfully
- ✅ CLI tool structure in place

**Completed in Phase 1:**
- ✅ Constants and magic numbers (`constants.rs`)
- ✅ Variable-length integer encoding (`varint.rs`) - ULEB128 and ZigZag
- ✅ Bit packing utilities (`bitpack.rs`) - Presence bitmaps and 3-bit tag packing
- ✅ CRC32C checksums (`checksum.rs`)
- ✅ Error types (`error.rs`) - Complete error enum with thiserror
- ✅ Security limits (`limits.rs`) - All spec-defined limits
- ✅ Type tag enum (`types.rs`) - 3-bit type tags with validation
- ✅ Decimal encoding (`decimal.rs`) - Arbitrary-precision decimal support

**Completed in Phase 2:**
- ✅ File header encoding/decoding (`header.rs`) - With flag accessors and ULEB128
- ✅ Block header and directory (`block.rs`) - With comprehensive limit enforcement
- ✅ Index footer (`footer.rs`) - Optional footer with CRC32C verification
- ✅ All structures use little-endian encoding as required by spec
- ✅ Comprehensive test coverage (66 tests passing)
- ✅ Limit enforcement working correctly
- ✅ Error handling implemented and tested

**Completed in Phase 3:**
- ✅ Decimal type with exact representation (`decimal.rs`)
- ✅ Type tag enum with validation (`types.rs`)
- ✅ All decimal encoding/decoding functionality
- ✅ Type tag conversion and validation
- ✅ Comprehensive test coverage for decimal operations

**Completed in Phase 4:**
- ✅ ColumnBuilder for converting records to columnar format (`column.rs`)
- ✅ FieldSegment encoding with compression support (`segment.rs`)
- ✅ BlockBuilder for aggregating columns into blocks (`block_builder.rs`)
- ✅ Integer detection logic (i64 vs decimal per spec)
- ✅ Dictionary vs raw string heuristics
- ✅ Delta encoding for monotonic integers
- ✅ Segment payload follows spec order (§4.7)
- ✅ Zstandard compression support
- ✅ Comprehensive test coverage (77 total tests passing)
- ✅ Schema drift support (fields can change type across records)
- ✅ Memory budgeting and limit enforcement
- ✅ Canonical key ordering support

**Completed in Phase 5:**
- ✅ FieldSegmentDecoder for decoding individual field segments (`segment_decode.rs`)
- ✅ BlockDecoder for full record reconstruction and field projection (`block_decode.rs`)
- ✅ Pre-decompression security limit enforcement
- ✅ Zstandard decompression with proper error handling
- ✅ Dictionary decoding with bounds checking
- ✅ Delta encoding decoder for monotonic integer sequences
- ✅ Presence bitmap and type tag parsing
- ✅ All data type support (null, bool, int, decimal, string, object, array)
- ✅ Comprehensive error handling and validation
- ✅ Spec §12.1 conformance test implementation and passing
- ✅ Full round-trip testing (JSON → encode → decode → JSON)
- ✅ Field projection testing
- ✅ Security limit enforcement (decompression bomb prevention)
- ✅ Test coverage: 24 tests in jac-codec, 66 tests in jac-format (90 total)

**Completed in Phase 6:**
- ✅ `JacWriter` streamable encoder with partial-block flush, index footer emission, and drop guard (`jac-io/src/writer.rs`)
- ✅ `JacReader` streaming decoder supporting index-driven and sequential iteration, CRC verification, and strict vs. resync modes (`jac-io/src/reader.rs`)
- ✅ High-level `compress`, `decompress_full`, and `project` APIs with NDJSON/JSON-array projection output (`jac-io/src/lib.rs`)
- ✅ Parallel helpers refactored for direct use (`jac-io/src/parallel.rs`)
- ✅ Integration tests covering index pointer, manual flush, checksum failure, projection semantics, and resynchronization (`jac-io/tests/integration_tests.rs`)
- ✅ Workspace-wide `cargo clippy` clean (constants/docs/errors updated) and `cargo test` passing

**Completed in Phase 7:**
- ✅ Request-based API design with `CompressRequest`, `DecompressRequest`, `ProjectRequest` structs (`jac-io/src/lib.rs`)
- ✅ Input/Output source enums: `InputSource`, `OutputSink`, `JacInput`, `DecompressFormat`, `ProjectFormat` (`jac-io/src/lib.rs`)
- ✅ Writer enhancements: `WriterMetrics`, `finish_with_index()`, `finish_without_index()` helpers (`jac-io/src/writer.rs`)
- ✅ Reader & projection iterators: `FieldIterator`, `ProjectionStream`, `RecordStream` with lazy evaluation (`jac-io/src/reader.rs`)
- ✅ CLI baseline implementation: functional `pack`/`unpack` commands using new APIs (`jac-cli/src/main.rs`)
- ✅ Async facade (feature-gated): `async_io` module with `spawn_blocking` wrappers (`jac-io/src/lib.rs`)
- ✅ Backward compatibility: deprecated shims for old APIs with clear migration path
- ✅ Progress reporting: comprehensive metrics and summary structs for all operations
- ✅ Documentation: extensive rustdoc comments and usage examples
- ✅ Test coverage: 133 total tests passing (2 CLI + 34 codec + 84 format + 4 io + 9 integration)
- ✅ Feature flags: `async` feature properly configured with optional dependencies

**Completed in Phase 8 (Week 1):**
- ✅ `jac ls` command with table/JSON output, field statistics, and HashSet dedupe (`jac-cli/src/main.rs`)
- ✅ `jac cat` command with block range filtering, CSV/JSON support, and progress spinner hooks (`jac-cli/src/main.rs`)
- ✅ CLI unit and integration coverage for ls/cat flows (`jac-cli/tests/cli.rs`)
- ✅ Progress spinners for pack/unpack/cat/ls plus verbose stderr summaries
- ✅ `--stats` flag delivering per-field null/absent/type breakdowns (`jac-cli/src/main.rs`, `jac-cli/tests/cli.rs`)
- ✅ Container format hint recorded in header flags (bits 3–4) with CLI auto-selection of NDJSON vs JSON array on unpack.

**Completed in Phase 9 (Wrapper Support - Phase 1):**
- ✅ `WrapperConfig` and `WrapperLimits` structs with default and hard limits (`jac-io/src/lib.rs`)
- ✅ `RecordStreamInner::Wrapper` variant for wrapped stream integration
- ✅ `wrapper` module with clean separation: `mod.rs`, `error.rs`, `pointer.rs`, `utils.rs`
- ✅ RFC 6901 JSON Pointer parsing with escape handling (~0, ~1) and validation
- ✅ Depth/buffer/pointer-length enforcement (default: 3/16M/256, hard max: 10/128M/2048)
- ✅ CLI flags: `--wrapper-pointer`, `--wrapper-pointer-depth`, `--wrapper-pointer-buffer`
- ✅ `WrapperMetrics` captured and displayed in CLI verbose output
- ✅ Container hint records `JsonArray` when wrapper is used
- ✅ 28 unit tests covering parsing, validation, limits, and RFC 6901 compliance
- ✅ 13 integration tests covering success cases, error cases, and CLI flag validation
- ✅ Test fixtures for all wrapper scenarios (envelopes, escaped keys, error cases)
- ✅ Comprehensive documentation in README.md and AGENTS.md

**Completed in Phase 10 (Wrapper Support - Phase 2):**
- ✅ `SectionSpec` and `MissingSectionBehavior` types added to `jac-io/src/lib.rs`
- ✅ `WrapperConfig::Sections` variant with full configuration support
- ✅ `WrapperMetrics::section_counts` field for per-section record tracking
- ✅ Section-specific errors: `SectionNotFound`, `SectionLabelCollision` in `WrapperError`
- ✅ `SectionsStream` iterator in `jac-io/src/wrapper/sections.rs` with buffered parsing
- ✅ Section stream integrated into `InputSource::into_record_stream()`
- ✅ CLI flags: `--wrapper-sections`, `--wrapper-section-pointer`, `--wrapper-section-label-field`, `--wrapper-section-no-label`, `--wrapper-sections-missing-error`
- ✅ Section metrics displayed in verbose CLI output
- ✅ 9 unit tests covering concatenation, labels, missing sections, empty arrays, and error cases
- ✅ 9 integration tests covering CLI usage, flag validation, conflicts, and custom pointers
- ✅ Test fixtures for sections mode (`jac-cli/tests/fixtures/wrapper/sections_basic.json`)

**Completed in Phase 11 (Wrapper Support - Phase 3):**
- ✅ `KeyCollisionMode` enum added to `jac-io/src/lib.rs` (Error/Overwrite modes)
- ✅ `WrapperConfig::KeyedMap` variant with pointer, key_field, limits, and collision_mode
- ✅ `WrapperMetrics::map_entry_count` field for tracking map entries
- ✅ Map-specific errors: `MapKeyTooLong`, `KeyFieldCollision`, `MapValueNotObject` in `WrapperError`
- ✅ `KeyedMapStream` iterator in `jac-io/src/wrapper/map.rs` with key injection
- ✅ Map stream integrated into `InputSource::into_record_stream()`
- ✅ CLI flags: `--wrapper-map`, `--wrapper-map-pointer`, `--wrapper-map-key-field`, `--wrapper-map-overwrite-key`
- ✅ Map metrics displayed in verbose CLI output
- ✅ 11 unit tests covering basic map flattening, nested pointers, collisions, validation, and key length limits
- ✅ 8 integration tests covering CLI usage, flag validation, conflicts, custom key fields, and overwrite mode
- ✅ Test fixtures for map mode (`map_basic.json`, `map_nested_pointer.json`, `map_empty.json`, `map_collision.json`)

**Upcoming Focus:**
1. Performance optimization and benchmarking (if needed)
2. Additional documentation and examples
3. Consider additional wrapper modes based on user feedback

**Last Updated:** 2025-11-01 (Phase 11 – Wrapper Phase 3 complete: Keyed map object flattening)
