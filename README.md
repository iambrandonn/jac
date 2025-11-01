# JAC - JSON-Aware Compression

[![CI](https://github.com/jac-rs/jac/workflows/CI/badge.svg)](https://github.com/jac-rs/jac/actions)
[![Crates.io](https://img.shields.io/crates/v/jac.svg)](https://crates.io/crates/jac)
[![Documentation](https://docs.rs/jac/badge.svg)](https://docs.rs/jac)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](https://github.com/jac-rs/jac/blob/main/LICENSE)

**JAC** (JSON-Aware Compression) is a binary container and encoding format for JSON designed for **archival** workloads where **compression ratio** is the top priority, **semantic round-trip** is required (not byte-identical formatting), and **partial decoding** (field/column projection) is desirable.

## Features

- **Block + Columnar** layout for arrays/streams of objects
- **Dictionary** encoding for keys and string values
- **Bit-packing/RLE** for booleans
- **Varint** (LEB128) + **delta** for integers
- **Union-typed columns** with **type-tags** for schema drift tolerance
- **Per-field compressed segments** (default **Zstandard**)
- **Field projection** - extract only needed fields without scanning full blocks
- **Semantic JSON** round-trip (keys may be re-ordered, formatting may differ)

## Quick Start

### Installation

```bash
cargo install jac-cli
```

### Basic Usage

```bash
# Compress NDJSON to JAC format
jac pack input.ndjson -o output.jac --progress

# Decompress JAC to NDJSON
jac unpack output.jac -o decompressed.ndjson --progress

# Defaults to the original wrapper (NDJSON vs JSON array) unless you pass --ndjson/--json-array.

# Raise the per-segment ceiling (trusted data only)
jac pack input.ndjson -o output.jac --max-segment-bytes 134217728 --allow-large-segments

# List blocks and fields (table or JSON)
jac ls output.jac
jac ls output.jac --format json --verbose

# Extract specific field values (NDJSON/JSON-array/CSV)
jac cat output.jac --field userId
jac cat output.jac --field userId --format csv --blocks 2-5

# Compute detailed statistics
jac ls output.jac --format json --stats
jac ls output.jac --format json --stats --stats-sample 10000
```

### Library Usage

```rust
use jac_io::{compress, decompress_full, project};

// Compress JSON data
let input = std::fs::File::open("input.ndjson")?;
let output = std::fs::File::create("output.jac")?;
compress(input, output, Default::default())?;

// Decompress full data
let input = std::fs::File::open("output.jac")?;
let output = std::fs::File::create("decompressed.ndjson")?;
decompress_full(input, output, Default::default())?;

// Project specific fields
let input = std::fs::File::open("output.jac")?;
let output = std::fs::File::create("projected.ndjson")?;
project(input, output, &["userId", "timestamp"], true)?;
```

### CLI Overview

| Command | Purpose | Key Flags |
|---------|---------|-----------|
| `jac pack` | Compress NDJSON/JSON into `.jac` | `--block-records`, `--zstd-level`, `--ndjson`, `--json-array`, `--max-segment-bytes`, `--allow-large-segments`, `--progress` |
| `jac unpack` | Decompress `.jac` back to JSON (defaults follow stored wrapper) | `--ndjson`, `--json-array`, `--progress` |
| `jac ls` | Inspect blocks and field statistics | `--format {table,json}`, `--verbose`, `--fields-only`, `--blocks-only` |
| `jac ls --stats` | Opt-in deep field analysis (samples ≤50k values/field) | `--stats`, `--verbose`, `--stats-sample <N>` |
| `jac cat` | Stream values for a field | `--field <name>`, `--format {ndjson,json-array,csv}`, `--blocks <range>`, `--progress` |

`jac ls` surfaces per-block summaries including field presence counts and compression ratios, while `jac cat` streams projected values without loading entire blocks, optionally showing progress for long-running reads.

> **Sampling note:** `jac ls --stats` inspects up to 50k values per field by default (tunable via `--stats-sample <N>`) to avoid re-reading massive segments; verbose output and JSON/table stats indicate when sampling occurs.

## Wrapper Support

JAC can preprocess wrapped/enveloped JSON structures before compression using JSON Pointer notation (RFC 6901). This allows you to compress nested API responses and data exports without manual preprocessing.

### Basic Usage

Extract an array nested inside an envelope:

```bash
# Input: {"data": [{"id": 1}, {"id": 2}]}
jac pack input.json -o output.jac --wrapper-pointer /data
```

### Nested Pointers

Navigate multiple levels:

```bash
# Input: {"api": {"v1": {"results": [...]}}}
jac pack api-response.json -o output.jac --wrapper-pointer /api/v1/results
```

### Configuration Flags

- `--wrapper-pointer <PATH>` - RFC 6901 JSON Pointer to target array/object
- `--wrapper-pointer-depth <N>` - Max traversal depth (default: 3, max: 10)
- `--wrapper-pointer-buffer <SIZE>` - Buffer limit (default: 16M, max: 128M)

### ⚠️ Important Limitations

**Wrappers are preprocessing transformations.** The original envelope structure is **not preserved** in the `.jac` file. Running `jac unpack` will output flattened records only.

If you need to preserve the envelope:
1. Archive the original JSON file separately, or
2. Use external preprocessing: `jq '.data | .[]' input.json | jac pack --ndjson -o output.jac`

### When to Use Wrappers vs Preprocessing

| Scenario | Recommendation |
|----------|---------------|
| Envelope < 50 MiB, target array near start | ✅ Use `--wrapper-pointer` |
| Envelope > 50 MiB before target data | ⚠️ Preprocess with `jq` or `mlr` |
| Need to preserve envelope structure | ❌ Archive original separately |
| Multiple target arrays (e.g., users + admins) | ✅ Use `--wrapper-sections users admins` |
| Dictionary-style objects (`{id: {...}}`) | ⏳ Wait for Phase 3 (Map mode) |

### Performance

Wrapper traversal is serial and happens before compression:
- Buffer memory ≈ envelope size (up to configured limit)
- Overhead ≈ 100-500ms for typical API responses
- Use `--verbose-metrics` to see actual buffer usage and processing time

### Troubleshooting

**Error: Buffer limit exceeded**
```
Suggested fixes:
  1. Increase buffer: --wrapper-pointer-buffer 32M
  2. Preprocess with jq: jq '.data | .[]' input.json | jac pack --ndjson -o output.jac
```

**Error: Pointer not found**
- Check pointer syntax (must start with `/` unless empty for root)
- Verify path exists in input JSON
- Use escaped characters: `~0` for `~`, `~1` for `/`
- Example: `/field~1name` matches key `field/name`

**Error: Wrong type (null/scalar)**
- Wrappers can only extract arrays (streaming elements) or objects (single record)
- Null and scalar values are not supported as wrapper targets

See [RFC 6901](https://tools.ietf.org/html/rfc6901) for complete JSON Pointer syntax details.

### Multi-Section Arrays

Concatenate multiple named arrays from a single object:

```bash
# Input: {"users": [{"id": 1}, {"id": 2}], "guests": [{"id": 3}]}
jac pack data.json -o output.jac --wrapper-sections users guests

# With custom pointers
jac pack api.json -o output.jac \
  --wrapper-sections users admins \
  --wrapper-section-pointer users=/data/active \
  --wrapper-section-pointer admins=/data/privileged

# Disable automatic section labels
jac pack data.json -o output.jac \
  --wrapper-sections users guests \
  --wrapper-section-no-label

# Custom label field name
jac pack data.json -o output.jac \
  --wrapper-sections users guests \
  --wrapper-section-label-field source

# Error on missing sections
jac pack data.json -o output.jac \
  --wrapper-sections users guests admins \
  --wrapper-sections-missing-error
```

**Notes:**
- Section order determines record order in output
- Missing sections are skipped by default (use `--wrapper-sections-missing-error` to fail)
- Labels are injected into records by default (field: `_section`, customizable via `--wrapper-section-label-field`)
- Label injection can be disabled with `--wrapper-section-no-label`
- All sections must contain arrays of objects
- The entire top-level object is buffered in memory; for very large envelopes (>50 MiB), consider preprocessing with `jq`

## Architecture

JAC is implemented as a Rust workspace with four main crates:

- **`jac-format`** - Core primitives (no I/O dependencies)
- **`jac-codec`** - Encoder/decoder engines
- **`jac-io`** - File I/O layer and high-level APIs
- **`jac-cli`** - Command-line tool

## Specification

JAC v1 is currently in **Draft 0.9.1**. See [SPEC.md](SPEC.md) for the complete technical specification.

## Performance

JAC targets:
- **≥20-40%** size reduction vs minified JSON+zstd on highly repetitive logs
- **O(records)** projection time reading only targeted field segments
- **Parallel** compression/decompression using multiple cores

## Security

JAC enforces strict limits to prevent decompression bombs:
- Maximum records per block: 1,000,000 (hard limit)
- Maximum fields per block: 65,535 (hard limit)
- Maximum segment size: 64 MiB by default (hard guard unless the encoder opts in to a higher value)
- Maximum string length: 16 MiB (hard limit)

The encoder refuses to emit segments that exceed these ceilings. Advanced users may raise the segment ceiling during compression via `jac pack --max-segment-bytes <bytes> --allow-large-segments`; this warning-gated flag is intended for trusted data where larger segments are required. The effective limit is written into the file header metadata so that `jac` and library consumers enforce the same ceiling during decompression, while respecting any stricter limit explicitly supplied by the reader.

## Troubleshooting Segment Limits

### Auto-Flush Behavior

JAC automatically flushes blocks early when a field segment approaches the 64 MiB limit. This prevents encoding failures but may increase block count. You can monitor this behavior using the `--verbose-metrics` flag:

```bash
jac pack input.ndjson -o output.jac --verbose-metrics
```

This will display per-field metrics showing which fields triggered early flushes.

### Error: "Field 'X' single-record payload exceeds segment limit"

**Cause:** A single record contains a field that would exceed 64 MiB uncompressed.

**Solutions:**

1. **Increase segment limit (advanced, use with caution):**
   ```bash
   jac pack --max-segment-bytes 134217728 --allow-large-segments input.ndjson -o output.jac
   ```
   ⚠️ **Warning:** Raising limits above 64 MiB increases memory usage and DoS risk. Only use this for trusted data.

2. **Modify your data to reduce field sizes:**
   - Split large nested objects into multiple records
   - Store large blobs externally and reference by ID
   - Compress or truncate data before encoding

### Understanding Per-Field Metrics

When using `--verbose-metrics`, JAC displays:
- **flush_count**: Number of early block flushes caused by this field
- **rejection_count**: Number of records rejected due to this field exceeding limits
- **max_segment_size**: Largest uncompressed segment size seen for this field

High flush counts suggest you should reduce `--block-records` to avoid frequent early flushes:

```bash
# Default is 100,000 records per block
jac pack input.ndjson -o output.jac --block-records 50000
```

## Parallel Compression Controls

JAC automatically decides when to compress in parallel based on CPU count, available memory, and input size:

- Uses up to 16 worker threads by default, but only when multiple cores and sufficient RAM are available
- Reserves roughly 75% of reported free memory for in-flight blocks (each worker can hold one block’s uncompressed payload)
- Falls back to sequential mode for small files (<10 MiB) where parallel overhead would dominate

You can tune or override this behaviour:

| Control | Purpose | Notes |
|---------|---------|-------|
| `--threads N` | Cap worker threads (set `1` to force sequential mode) | Applies after memory-based cap |
| `--parallel-memory-factor F` | Adjust memory reservation factor (0 < F ≤ 1) | e.g. `0.6` reserves 60% of free RAM |
| `JAC_PARALLEL_MEMORY_FACTOR=F` | Environment override for reservation factor | CLI flag takes precedence |
| `--verbose-metrics` | Show the selected parallel decision and reasoning | Helpful for tuning |

Examples:

```bash
# Force sequential compression
jac pack data.ndjson -o data.jac --threads 1

# Cap to 4 threads and lower memory reservation to 60%
jac pack data.ndjson -o data.jac --threads 4 --parallel-memory-factor 0.6 --verbose-metrics

# Apply a lower factor from the environment (flag still wins)
JAC_PARALLEL_MEMORY_FACTOR=0.5 jac pack data.ndjson -o data.jac
```

When `--verbose-metrics` is enabled—or when you explicitly override threads or memory factor—the CLI prints the heuristic decision, including the effective reservation factor and estimated peak memory. This helps confirm that your tuning behaves as expected.

JAC now records and reports **actual** compression wall-clock time and peak RSS usage (measured at 50 ms intervals) so you can validate the heuristic against reality. The `--verbose-metrics` summary includes both the heuristic estimate and the observed peak, making it easy to spot under-provisioned hosts or cases where the reservation factor should be adjusted.

### Containers and cgroups

Inside containers the host-reported “available” memory often ignores cgroup limits. Set an explicit memory ceiling and/or thread cap so the heuristic matches your runtime budget:

```bash
limit_bytes=$(cat /sys/fs/cgroup/memory.max)
if [[ "$limit_bytes" == "max" ]]; then
  limit_bytes=$((8 * 1024 * 1024 * 1024)) # fallback for unconstrained hosts
fi
# Reserve 70% of the cgroup limit for compression buffers
factor=$(python -c "limit=$limit_bytes or 1; print(f'{0.70 if limit>0 else 0.70:.3f}')")
JAC_PARALLEL_MEMORY_FACTOR=$factor jac pack input.ndjson -o output.jac --threads 6 --verbose-metrics
```

If you already know the desired cap, skip the calculation and set `--parallel-memory-factor` (or the env var) directly. Combine the flag with `--threads` to keep concurrency within the container’s CPU quota.

## Testing

JAC includes a comprehensive Phase 9 validation suite with multiple testing categories and automated CI integration.

### Test Categories

| Category | Command | Purpose | Duration |
|----------|---------|---------|----------|
| **Unit Tests** | `cargo test -p jac-format` | Core format/unit coverage (varints, bitpacking, decimals) | ~5s |
| **Integration Tests** | `cargo test -p jac-codec` | Codec round-trips plus SPEC §12.1 conformance checks | ~10s |
| **IO Tests** | `cargo test -p jac-io` | Streaming encoder/decoder integration + negative/error harness | ~15s |
| **CLI Tests** | `cargo test -p jac-cli` | CLI smoke, pack/unpack round-trips, and SPEC fixture regression | ~5s |
| **Cross-Platform** | `cargo test --test cross_platform_compatibility` | Endianness, version compatibility, and platform validation | ~10s |
| **Security** | `cargo test --test security_property_tests` | Security-focused property tests and vulnerability prevention | ~30s |
| **Slow Tests** | `cargo test --ignored` | Million-record tests and stress scenarios (nightly/CI only) | ~5m |
| **Performance** | `cargo bench --workspace` | Benchmarking and performance regression detection | ~2m |

### Test Runner

Use the categorized test runner for efficient development:

```bash
# Run specific test categories
./scripts/run_tests.sh --unit --integration
./scripts/run_tests.sh --slow --stress  # For nightly/CI
./scripts/run_tests.sh --performance    # For performance testing

# Run all tests (CI mode)
./scripts/run_tests.sh --all
```

### Fuzzing and Property Testing

JAC includes comprehensive fuzzing and property testing for security and robustness:

```bash
# Install fuzzing tools
cargo install cargo-fuzz

# Run security fuzzing
./scripts/security_fuzz.sh

# Run specific fuzz targets
cargo fuzz run fuzz_decode_block
cargo fuzz run fuzz_varint
cargo fuzz run fuzz_compression
cargo fuzz run fuzz_projection
cargo fuzz run fuzz_security
```

### Debugging and Performance Tools

JAC includes advanced debugging and performance visualization tools:

```bash
# Run comprehensive test debugging
./scripts/test_debug_tool.sh

# Generate performance reports
./scripts/manage_ci.sh report

# Validate test data and provenance
./scripts/manage_fixture_provenance.sh validate
```

### CI Integration

JAC uses GitHub Actions with comprehensive CI workflows:

- **Basic Tests**: Unit, integration, and cross-platform tests on all platforms
- **Nightly Tests**: Comprehensive test suite with slow/stress tests
- **Security Tests**: Security scanning, fuzzing, and compliance checks
- **Performance Tests**: Benchmarking and performance monitoring
- **Fuzzing Tests**: Automated fuzzing and property testing
- **Quality Checks**: Clippy, rustfmt, and documentation validation

### Test Data Management

JAC includes sophisticated test data generation and management:

```bash
# Generate test data
./scripts/manage_test_data.sh generate --category all --size medium

# Validate test data
./scripts/manage_test_data.sh validate

# Manage fixture provenance
./scripts/manage_fixture_provenance.sh generate
./scripts/manage_fixture_provenance.sh validate
```

### Compliance and Security

JAC maintains comprehensive compliance and security documentation:

- **Security Reports**: `./scripts/generate_security_report.sh`
- **Threat Modeling**: `docs/security/threat_modeling.md`
- **Regression Scenarios**: `docs/security/regression_scenarios.md`
- **Compliance Matrix**: `cargo run -p xtask`

**Suggested pre-submit checklist:**
- `cargo test --all`
- `cargo run -p xtask`
- `./scripts/run_tests.sh --unit --integration`
- `./scripts/security_fuzz.sh`

## Development Status

**Current Phase:** Phase 9 (Testing & Validation) – ✅ **COMPLETED**

- ✅ Phases 0–8 delivered format/codec/IO/CLI foundations and baseline telemetry
- ✅ Phase 9 completed: Comprehensive testing and validation suite including:
  - Cross-platform/endianness/version compatibility tests
  - Test categorization and management system
  - Debugging and performance visualization tools
  - Security-focused fuzzing and property tests
  - Threat modeling and regression scenarios
  - Security compliance documentation and reporting
  - Large test data strategy and generation scripts
  - Fixture provenance documentation and tracking
  - Enhanced CI workflows with test/perf monitoring and caching
  - Updated documentation describing the validation suite

**Next Phase:** Phase 10 (Production Readiness) – Performance optimization, production hardening, and ecosystem integration

See [PLAN.md](PLAN.md) for the complete roadmap and phase breakdown.

## Contributing

We welcome contributions! Please see our [Contributing Guide](CONTRIBUTING.md) for details.

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for details.

## Related Projects

- [Zstandard](https://github.com/facebook/zstd) - Fast compression algorithm
- [Parquet](https://parquet.apache.org/) - Columnar storage format
- [MessagePack](https://msgpack.org/) - Binary serialization format
