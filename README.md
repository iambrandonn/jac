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
jac pack input.ndjson -o output.jac

# Decompress JAC to NDJSON
jac unpack output.jac -o decompressed.ndjson --ndjson

# List blocks and fields
jac ls output.jac

# Extract specific field values
jac cat output.jac --field userId
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

**Current Phase:** Phase 0 (Project Setup) - ✅ Complete

**Next Steps:**
- Phase 1: Core primitives (varint, bitpack, CRC)
- Phase 2: File & block structures
- Phase 3: Decimal & type-tag support

See [PLAN.md](PLAN.md) for the complete implementation roadmap.

## Contributing

We welcome contributions! Please see our [Contributing Guide](CONTRIBUTING.md) for details.

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for details.

## Related Projects

- [Zstandard](https://github.com/facebook/zstd) - Fast compression algorithm
- [Parquet](https://parquet.apache.org/) - Columnar storage format
- [MessagePack](https://msgpack.org/) - Binary serialization format

