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

### Advanced Configuration

#### Environment Variables

Override default wrapper settings via environment variables:

```bash
# Set default wrapper depth
export JAC_WRAPPER_DEPTH=5

# Set default buffer size
export JAC_WRAPPER_BUFFER=32M

# Enable debug logging for wrapper operations
export JAC_DEBUG_WRAPPER=1

# Custom config file location
export JAC_CONFIG_PATH=~/my-jac-config.toml
```

#### Configuration File

Create `~/.jac/config.toml` to set project-wide defaults:

```toml
[wrapper]
default_depth = 5
default_buffer = "32M"
debug = false

[compression]
default_block_records = 100000
default_zstd_level = 6
```

**Configuration Priority (highest to lowest):**
1. CLI flags (`--wrapper-pointer-depth 7`)
2. Environment variables (`JAC_WRAPPER_DEPTH=5`)
3. Config file (`~/.jac/config.toml`)
4. Built-in defaults

#### Debug Logging

Enable detailed wrapper preprocessing diagnostics:

```bash
JAC_DEBUG_WRAPPER=1 jac pack input.json -o output.jac --wrapper-pointer /data
```

Debug output includes:
- Wrapper mode and configuration
- Pointer depth and buffer limits
- Section/map entry details
- Actual buffer usage and processing time

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
| Dictionary-style objects (`{id: {...}}`) | ✅ Use `--wrapper-map` |

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

### Keyed Map Objects (Dictionary-Style)

Flatten object-of-objects structures where keys represent identifiers:

```bash
# Input: {"alice": {"age": 30, "role": "admin"}, "bob": {"age": 25, "role": "user"}}
jac pack users.json -o output.jac --wrapper-map

# Output records will include the map key in a "_key" field:
# {"_key": "alice", "age": 30, "role": "admin"}
# {"_key": "bob", "age": 25, "role": "user"}

# Custom key field name
jac pack users.json -o output.jac \
  --wrapper-map \
  --wrapper-map-key-field user_id

# With nested pointer to the map
jac pack api.json -o output.jac \
  --wrapper-map \
  --wrapper-map-pointer /data/users

# Allow overwriting if key field already exists
jac pack data.json -o output.jac \
  --wrapper-map \
  --wrapper-map-overwrite-key
```

**Configuration Flags:**
- `--wrapper-map` - Enable keyed map wrapper mode
- `--wrapper-map-pointer <PATH>` - JSON Pointer to map object (default: root)
- `--wrapper-map-key-field <FIELD>` - Field name for injected keys (default: `_key`)
- `--wrapper-map-overwrite-key` - Overwrite existing field if collision occurs (default: error)

**Notes:**
- The map object (or pointed-to object) must contain only object values
- Map keys are validated against JAC's string length limit (16 MiB)
- Keys are injected as string fields into each record
- By default, collisions with existing fields cause an error; use `--wrapper-map-overwrite-key` to replace
- The entire map is buffered in memory; for maps with >100K entries or large values, consider preprocessing
- Map key order is preserved from the JSON parser (typically insertion order, but not guaranteed)

**Memory Considerations:**

Map mode buffers all entries before streaming. Approximate memory usage:
- Small maps (<1K entries): negligible overhead
- Medium maps (1K-100K entries): ~few MB to tens of MB
- Large maps (>100K entries): may approach buffer limits

For very large maps, consider external preprocessing:
```bash
# Alternative for large maps
jq 'to_entries | map(.value + {_key: .key})' input.json | jac pack --ndjson -o output.jac
```

## Architecture

JAC is implemented as a Rust workspace with four main crates:

- **`jac-format`** - Core primitives (no I/O dependencies)
- **`jac-codec`** - Encoder/decoder engines
- **`jac-io`** - File I/O layer and high-level APIs
- **`jac-cli`** - Command-line tool

## Specification

JAC v1 is currently in **Draft 0.9.1**. See [SPEC.md](SPEC.md) for the complete technical specification.

## Wrapper FAQ

### Can I recover the original envelope structure from a .jac file?

**No.** Wrappers are preprocessing transformations that extract and flatten the target data before compression. The envelope structure is discarded during this process and cannot be reconstructed by `jac unpack`.

**Solutions:**
- Archive the original JSON file separately if you need to preserve the envelope
- Use external preprocessing with `jq` or `mlr` to extract data, then compress with standard JAC
- Document the envelope structure in your workflow/pipeline

### When should I use wrappers instead of preprocessing with jq/mlr?

**Use wrappers when:**
- The envelope is <50 MiB and the target data appears early
- You want a single-command workflow without external dependencies
- You're processing multiple files with the same structure in a pipeline
- The data source is consistent and trusted

**Use external preprocessing when:**
- The envelope is very large (>50 MiB) before reaching target data
- You need to preserve the envelope structure for other purposes
- You need complex transformations beyond array/object extraction
- You want deterministic key ordering in map mode

### What happens if I exceed the wrapper buffer limit?

JAC will return a `BufferLimitExceeded` error with actionable suggestions:
1. Increase the buffer limit: `--wrapper-pointer-buffer 64M`
2. Preprocess externally: `jq '.data | .[]' input.json | jac pack --ndjson -o output.jac`
3. Check that your pointer path is correct (buffering unnecessary parent data)

The error message includes the actual buffered size and suggested `jq` command for preprocessing.

### How do wrappers affect compression performance?

Wrapper preprocessing adds serial overhead before parallel compression begins:
- **Small envelopes** (<1 MiB): Negligible impact (10-100ms)
- **Medium envelopes** (1-10 MiB): Minor impact (100-500ms)
- **Large envelopes** (10-50 MiB): Noticeable but usually acceptable (0.5-2s)
- **Very large envelopes** (>50 MiB): Consider preprocessing instead

Use `--verbose-metrics` to see exact buffer usage and processing time for your data.

### Can I use wrappers with Python/WASM bindings?

**Not yet.** Wrapper support is currently CLI and Rust library only. Python and WASM bindings are planned for future phases based on user demand.

For now, preprocess your data externally before using language bindings:
```python
# Python workaround
import subprocess
subprocess.run(['jq', '.data | .[]', 'input.json'], stdout=open('flattened.ndjson', 'w'))
# Then use Python bindings on flattened.ndjson
```

### What's the difference between wrapper modes?

| Mode | Use Case | Example Input | Key Feature |
|------|----------|---------------|-------------|
| **Pointer** | Single nested array/object | `{"data": [...]}` | RFC 6901 path navigation |
| **Sections** | Multiple named arrays | `{"users": [...], "admins": [...]}` | Concatenation with labels |
| **KeyedMap** | Dictionary-style objects | `{"alice": {...}, "bob": {...}}` | Key injection as field |

### How do I debug wrapper issues?

Enable debug mode to see detailed preprocessing diagnostics:

```bash
JAC_DEBUG_WRAPPER=1 jac pack input.json -o output.jac --wrapper-pointer /data
```

Debug output shows:
- Wrapper configuration (mode, depth, buffer)
- Pointer paths and section details
- Actual buffer usage
- Processing duration
- Section/map record counts

### Are there security considerations for wrappers?

**Yes.** Wrappers enforce hard security limits to prevent resource exhaustion:
- Max depth: 10 (prevents deep recursion attacks)
- Max buffer: 128 MiB (prevents memory exhaustion)
- Max pointer length: 2048 characters (prevents malicious strings)

These limits cannot be exceeded even with CLI flags or config files. Untrusted input should still be validated before processing.

### Can wrapper modes be combined?

**No.** Only one wrapper mode can be active per compression request:
- `--wrapper-pointer` conflicts with `--wrapper-sections` and `--wrapper-map`
- `--wrapper-sections` conflicts with `--wrapper-pointer` and `--wrapper-map`
- `--wrapper-map` conflicts with `--wrapper-pointer` and `--wrapper-sections`

This is enforced by CLI argument validation. To process complex structures, use external preprocessing or multiple passes.

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
