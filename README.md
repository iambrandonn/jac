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
jac unpack output.jac -o decompressed.ndjson --ndjson --progress

# List blocks and fields (table or JSON)
jac ls output.jac
jac ls output.jac --format json --verbose

# Extract specific field values (NDJSON/JSON-array/CSV)
jac cat output.jac --field userId
jac cat output.jac --field userId --format csv --blocks 2-5

# Compute detailed statistics
jac ls output.jac --format json --stats
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
| `jac pack` | Compress NDJSON/JSON into `.jac` | `--block-records`, `--zstd-level`, `--ndjson`, `--json-array`, `--progress` |
| `jac unpack` | Decompress `.jac` back to JSON | `--ndjson`, `--json-array`, `--progress` |
| `jac ls` | Inspect blocks and field statistics | `--format {table,json}`, `--verbose`, `--fields-only`, `--blocks-only` |
| `jac ls --stats` | Opt-in deep field analysis (samples ≤50k values/field) | `--stats`, `--verbose` |
| `jac cat` | Stream values for a field | `--field <name>`, `--format {ndjson,json-array,csv}`, `--blocks <range>`, `--progress` |

`jac ls` surfaces per-block summaries including field presence counts and compression ratios, while `jac cat` streams projected values without loading entire blocks, optionally showing progress for long-running reads.

> **Sampling note:** `jac ls --stats` inspects up to 50k values per field when computing type distribution metrics to avoid re-reading massive segments; verbose output and JSON/table stats will indicate when sampling occurred.

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
- Maximum segment size: 64 MiB (hard limit)
- Maximum string length: 16 MiB (hard limit)

## Development Status

**Current Phase:** Phase 8 (CLI Tool Completion) – polishing progress/stats UX

- ✅ Phases 0–7 delivered core format, codec, I/O, and request-based APIs
- ✅ Phase 8 Week 1: `jac ls` / `jac cat` implemented with field statistics, block filtering, and progress spinners
- ✅ Phase 8 Week 2: progress/timing summaries for pack/unpack/cat/ls, sampling-aware `--stats` output, refreshed docs & tests
- 🔜 Next: CLI doc polish & configuration knobs for stats sampling, then move to benchmarking (see [PLAN8.md](PLAN8.md))

See [PLAN.md](PLAN.md) for the complete roadmap and phase breakdown.

## Contributing

We welcome contributions! Please see our [Contributing Guide](CONTRIBUTING.md) for details.

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for details.

## Related Projects

- [Zstandard](https://github.com/facebook/zstd) - Fast compression algorithm
- [Parquet](https://parquet.apache.org/) - Columnar storage format
- [MessagePack](https://msgpack.org/) - Binary serialization format
