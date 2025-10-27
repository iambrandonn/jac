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
