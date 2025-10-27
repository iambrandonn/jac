# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Initial project setup and workspace structure
- Basic crate topology (jac-format, jac-codec, jac-io, jac-cli)
- CI/CD pipeline setup
- Test data fixtures for conformance testing
- CLI progress spinners and throughput summaries for `pack`, `unpack`, `ls`, and `cat`
- `jac ls --stats` sampling-based field analysis with JSON/table outputs
- Configurable `--stats-sample` limit for tuning per-field sampling in `jac ls --stats`
- Integration tests covering CLI inspection and projection workflows
- File header stores container format hints (NDJSON vs JSON array) and `jac unpack` now defaults to the recorded wrapper when no format flag is provided.
- `jac pack --max-segment-bytes` flag (with `--allow-large-segments`) to opt into larger segment ceilings, surfaced in writer metrics and CLI summaries, plus tests around limit overrides.
- File headers now encode the effective segment limit in metadata so decoders mirror the producer's ceiling when the reader uses default limits.

### Changed
- Enhanced CLI documentation (README/PLAN/AGENTS) to reflect Phase 8 capabilities

### Deprecated
- N/A

### Removed
- N/A

### Fixed
- N/A

### Security
- Segment-limit overrides are warning-gated and recorded in header metadata so decoders still enforce the producer's ceiling; defaults remain 64 MiB unless explicitly raised.

---

## [0.1.0] - TBD

### Added
- Initial implementation of JAC v1 format (Draft 0.9.1)
- Core primitives (varint, bitpack, CRC32C)
- File and block structures
- Columnar encoding/decoding
- CLI tool for pack/unpack/ls/cat operations
- Comprehensive test suite and conformance tests

---

## Spec Compliance

- **v0.1.0**: Implements JAC v1 Draft 0.9.1 specification
- **v1.0.0**: Will implement JAC v1.0 final specification (when released)
