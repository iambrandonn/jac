# JAC v1 Implementation Plan

**Project:** JSON-Aware Compression Library (Rust)
**Spec Version:** Draft 0.9.1
**Target:** Production-ready archival compression with columnar projection support

---

## Overview

This plan breaks down the implementation of JAC into manageable phases, prioritizing core functionality and establishing a solid foundation before adding advanced features. The implementation will be test-driven, with each phase including specific test milestones.

---

## Definition of Done (applies to all phases)

- All tasks for the phase checked off
- All relevant tests pass (unit and integration)
- Clippy clean (no warnings) and rustfmt applied
- Code reviewed (if team project)
- Milestone is demo‑able
- AGENTS.md updated with current status, known issues, and performance notes (if applicable)
- SPEC-COMPLIANCE.md updated with requirements implemented/tested in this phase

---

## Phase 0: Project Setup & Infrastructure

### 0.1 Repository Structure

**Tasks:**
- [ ] Initialize Rust workspace with Cargo.toml at root
- [ ] Create crate directories matching spec topology:
  ```
  jac/
  ├─ Cargo.toml (workspace)
  ├─ SPEC.md (existing)
  ├─ PLAN.md (this file)
  ├─ README.md
  ├─ LICENSE (Apache-2.0 or MIT)
  ├─ .gitignore
  ├─ jac-format/
  │  └─ Cargo.toml
  ├─ jac-codec/
  │  └─ Cargo.toml
  ├─ jac-io/
  │  └─ Cargo.toml
  └─ jac-cli/
     └─ Cargo.toml
  ```
- [ ] Set up CI/CD (GitHub Actions or equivalent):
  - Rust version: stable + nightly for benchmarks
  - Jobs: test, clippy (with `-D warnings`), fmt, docs
  - Matrix: Linux, macOS, Windows
- [ ] Pin Rust toolchain:
  - Add `rust-version = "1.70"` (or current MSRV) to workspace `Cargo.toml`
  - Optional: Add `rust-toolchain.toml` to lock version for local/CI consistency
- [ ] Review/update `AGENTS.md` at root with project context

**Dependencies:**
- Workspace dependencies (versions managed at root):
  - `serde` (1.x with derive)
  - `serde_json` (for metadata; later eval `simd-json`)
  - `zstd` (0.13+)
  - `crc32c` (0.6+)
  - `bitvec` (1.x)
  - `ahash` or `hashbrown` (0.14+)
  - `rayon` (1.x for parallelism)
  - `bytes` (1.x)
  - `smallvec` (1.x)
  - `thiserror` (1.x for error handling)
  - Dev: `criterion` (benchmarks), `proptest` (fuzzing)
  - `clap` (4.x with derive) for CLI parsing
  - Optional: `tracing` (0.1) or `log` (0.4) + `env_logger` for debugging

**Milestone:** Workspace builds; CI passes on empty crates.

**Additional Setup:**
- Create `testdata/` directory with fixtures:
  - `conformance.ndjson` (spec §12.1)
  - `edge-cases.ndjson` (Unicode, large numbers, nested)
  - `benchmark-logs.ndjson` (synthetic/perf)
- Add `CHANGELOG.md` and tag commits with spec version compliance (e.g., `spec-0.9.1`)
- Create `SPEC-COMPLIANCE.md` skeleton with sections for each spec MUST/SHOULD:
  - Each phase will update this file to link requirements to implementation tasks/tests
  - Template: `| Requirement | Spec Ref | Implementation | Test | Status |`
  - This provides lightweight traceability without requiring full upfront matrix

---

## Phase 1: Core Primitives (`jac-format`)

**Goal:** Implement low-level encoding/decoding utilities with no I/O dependencies.

### 1.1 Constants & Magic Numbers

**File:** `src/constants.rs`

```rust
pub const FILE_MAGIC: [u8; 4] = [0x4A, 0x41, 0x43, 0x01]; // "JAC\x01"
pub const BLOCK_MAGIC: u32 = 0x314B4C42; // "BLK1"
pub const INDEX_MAGIC: u32 = 0x31584449; // "IDX1"

pub const COMPRESSOR_NONE: u8 = 0;
pub const COMPRESSOR_ZSTD: u8 = 1;
pub const COMPRESSOR_BROTLI: u8 = 2;
pub const COMPRESSOR_DEFLATE: u8 = 3;

// File header flags (bit masks)
pub const FLAG_CANONICALIZE_KEYS: u32 = 1 << 0;
pub const FLAG_CANONICALIZE_NUMBERS: u32 = 1 << 1;
pub const FLAG_NESTED_OPAQUE: u32 = 1 << 2;

// Type tag codes (3-bit)
pub const TAG_NULL: u8 = 0;
pub const TAG_BOOL: u8 = 1;
pub const TAG_INT: u8 = 2;
pub const TAG_DECIMAL: u8 = 3;
pub const TAG_STRING: u8 = 4;
pub const TAG_OBJECT: u8 = 5;
pub const TAG_ARRAY: u8 = 6;
pub const TAG_RESERVED: u8 = 7;

// Encoding flags bitfield (per field segment)
pub const ENCODING_FLAG_DICTIONARY: u64 = 1 << 0;
pub const ENCODING_FLAG_DELTA: u64 = 1 << 1;
pub const ENCODING_FLAG_RLE: u64 = 1 << 2;      // reserved
pub const ENCODING_FLAG_BIT_PACKED: u64 = 1 << 3; // reserved
```

### 1.2 Variable-Length Integer Encoding (ULEB128 / ZigZag)

**File:** `src/varint.rs`

**Tasks:**
- [ ] `encode_uleb128(val: u64) -> SmallVec<[u8; 10]>`
- [ ] `decode_uleb128(bytes: &[u8]) -> Result<(u64, usize)>` — returns (value, bytes_consumed)
- [ ] Cap decode at 10 bytes for u64 (security: `LimitExceeded`)
- [ ] `zigzag_encode(v: i64) -> u64`
- [ ] `zigzag_decode(u: u64) -> i64`

**Tests:**
- Round-trip for 0, 1, 127, 128, 16383, 16384, u64::MAX
- Decode truncated stream → `UnexpectedEof`
- Decode 11+ bytes → `LimitExceeded`
- **Property-based tests** (using `proptest`):
  - `forall u64: encode then decode returns original value`
  - `forall i64: zigzag encode then decode returns original value`
  - `encoded length always <= 10 bytes`

### 1.3 Bit Packing Utilities

**File:** `src/bitpack.rs`

**Tasks:**
- [ ] Presence bitmap: wrapper around `BitVec` (LSB-first, byte-aligned)
  - **Pin bit order to `Lsb0`** explicitly (avoid accidental `Msb0` defaults)
  - `set_present(record_idx: usize, present: bool)`
  - `is_present(record_idx: usize) -> bool`
  - `to_bytes() -> Vec<u8>` (ceil(count/8))
  - `from_bytes(bytes: &[u8], count: usize) -> Self`
- [ ] 3-bit type-tag packing:
  - `TagPacker` struct: accumulates 3-bit values, packs LSB-first
  - `push(tag: u8)` — asserts tag < 8
  - `finish() -> Vec<u8>` — pads unused bits to zero
  - `TagUnpacker`: iterator over packed tags
- [ ] Boolean bit-packing (8 per byte, LSB-first):
  - **Pin bit order to `Lsb0`** explicitly
  - Similar interface as presence

**Tests:**
- Presence: set/get for various record counts (1, 7, 8, 9, 100)
- Tag packing: [4,4,4] → 0b01001001_00000000 (9 bits → 2 bytes)
- Round-trip for edge counts (3, 8, 10, 100 tags)
- **Bit ordering regression test**: Verify LSB-first ordering is maintained
- **Property-based tests** (using `proptest`):
  - `forall Vec<bool>: presence bitmap set/get round-trips`
  - `forall Vec<u8 in 0..8>: tag packing/unpacking round-trips with correct padding`
  - `forall Vec<bool>: boolean bit-packing round-trips`

### 1.4 CRC32C

**File:** `src/checksum.rs`

**Tasks:**
- [ ] Wrapper around `crc32c` crate
- [ ] `compute_crc32c(data: &[u8]) -> u32`
- [ ] `verify_crc32c(data: &[u8], expected: u32) -> Result<()>`

**Tests:**
- Known vectors from RFC or zstd test suite
- Mismatch → `ChecksumMismatch`

### 1.5 Error Types

**File:** `src/error.rs`

```rust
#[derive(Debug, thiserror::Error)]
pub enum JacError {
    #[error("Invalid magic bytes")]
    InvalidMagic,
    #[error("Unsupported version: {0}")]
    UnsupportedVersion(u8),
    #[error("Corrupt header")]
    CorruptHeader,
    #[error("Corrupt block")]
    CorruptBlock,
    #[error("Checksum mismatch")]
    ChecksumMismatch,
    #[error("Unexpected end of file")]
    UnexpectedEof,
    #[error("Decompression error: {0}")]
    DecompressError(String),
    #[error("Limit exceeded: {0}")]
    LimitExceeded(String),
    #[error("Type mismatch")]
    TypeMismatch,
    #[error("Dictionary index out of range")]
    DictionaryError,
    #[error("Unsupported feature: {0}")]
    UnsupportedFeature(String),
    #[error("Unsupported compression codec: {0}")]
    UnsupportedCompression(u8),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, JacError>;
```

### 1.6 Limits & Configuration

**File:** `src/limits.rs`

```rust
#[derive(Debug, Clone)]
pub struct Limits {
    pub max_records_per_block: usize,           // 1_000_000 hard
    pub max_fields_per_block: usize,            // 65_535 hard
    pub max_segment_uncompressed_len: usize,    // 64 MiB
    pub max_block_uncompressed_total: usize,    // 256 MiB
    pub max_dict_entries_per_field: usize,      // 65_535 hard
    pub max_string_len_per_value: usize,        // 16 MiB
    pub max_decimal_digits_per_value: usize,    // 65_536
    pub max_presence_bytes: usize,              // 32 MiB
    pub max_tag_bytes: usize,                   // 32 MiB
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_records_per_block: 100_000,
            max_fields_per_block: 4_096,
            max_segment_uncompressed_len: 64 * 1024 * 1024,
            max_block_uncompressed_total: 256 * 1024 * 1024,
            max_dict_entries_per_field: 4_096,
            max_string_len_per_value: 16 * 1024 * 1024,
            max_decimal_digits_per_value: 65_536,
            max_presence_bytes: 32 * 1024 * 1024,
            max_tag_bytes: 32 * 1024 * 1024,
        }
    }
}
```

**Milestone:** `jac-format` compiles; all unit tests pass; 100% coverage on varint/bitpack; `#![deny(unsafe_code)]` added to crate root.

### 1.7 Configuration Types and Public Exports

**File:** `src/lib.rs` (in `jac-format`) — re-export core types; define options in `jac-io` later and re-export for CLI.

```rust
#[derive(Debug, Clone, Copy)]
pub enum Codec {
    None,
    Zstd(u8),   // level
    Brotli(u8), // optional, returns UnsupportedCompression in v0.1.0
    Deflate(u8), // optional, returns UnsupportedCompression in v0.1.0
}
```

**Tasks:**
- [ ] Define `Codec` enum with all variants
- [ ] Document Brotli/Deflate as unimplemented in v0.1.0 (rustdoc)
- [ ] Implement stub that returns `UnsupportedCompression` for ids 2/3

**Tests:**
- [ ] Unit test: Attempt to use `Codec::Brotli(11)` → verify returns `UnsupportedCompression`
- [ ] Unit test: Attempt to use `Codec::Deflate(6)` → verify returns `UnsupportedCompression`

Notes:
- Define `CompressOpts` and `DecompressOpts` in `jac-io` (Phase 7) and re-export from crate root.
- Keep low-level constants/enums in `jac-format`.
- **v0.1.0 scope:** `Codec::Brotli` and `Codec::Deflate` variants exist in the enum but immediately return `UnsupportedCompression` when used. Full support in Phase 12.

---

## Phase 2: File & Block Structures (`jac-format`)

### 2.1 File Header

**File:** `src/header.rs`

```rust
#[derive(Debug, Clone)]
pub struct FileHeader {
    pub flags: u32,
    pub default_compressor: u8,
    pub default_compression_level: u8,
    pub block_size_hint_records: usize,
    pub user_metadata: Vec<u8>,
}

impl FileHeader {
    pub fn encode(&self) -> Result<Vec<u8>>;
    pub fn decode(bytes: &[u8]) -> Result<(Self, usize)>; // (header, bytes_consumed)
}
```

**Tasks:**
- [ ] Encode/decode with ULEB128 for lengths
- [ ] Validate magic on decode
- [ ] Flag accessors using bit masks: `canonicalize_keys()`, `canonicalize_numbers()`, `nested_opaque()`

**Tests:**
- Round-trip with various metadata sizes (0, 100, 1MB)
- Invalid magic → `InvalidMagic`
- **Flag accessor tests**: Verify flag accessors (`canonicalize_keys()`, etc.) return correct values
- **Endianness tests**: Verify flags (u32) are little-endian on read/write
- **Note:** Actual canonicalization behavior (key sorting, number formatting) is tested in Phase 4/7 encoder tests

### 2.2 Block Header & Directory

**File:** `src/block.rs`

```rust
#[derive(Debug, Clone)]
pub struct BlockHeader {
    pub record_count: usize,
    pub fields: Vec<FieldDirectoryEntry>,
}

#[derive(Debug, Clone)]
pub struct FieldDirectoryEntry {
    pub field_name: String,
    pub compressor: u8,
    pub compression_level: u8,
    pub presence_bytes: usize,
    pub tag_bytes: usize,
    pub value_count_present: usize,
    pub encoding_flags: u64,
    pub dict_entry_count: usize,
    pub segment_uncompressed_len: usize,
    pub segment_compressed_len: usize,
    pub segment_offset: usize,
}

impl BlockHeader {
    pub fn encode(&self) -> Result<Vec<u8>>;
    pub fn decode(bytes: &[u8], limits: &Limits) -> Result<(Self, usize)>;
}
```

**Tasks:**
- [ ] Encode: write `block_magic`, `header_len` (compute after), fields
- [ ] Decode: verify block_magic, enforce limits on field_count, field_name_len
- [ ] **Limit enforcement order**: (1) Read lengths, (2) Validate against `Limits`, (3) Allocate buffers, (4) Read data
- [ ] Encoding flags bitfield helpers (dictionary, delta, etc.)

**Tests:**
- Round-trip with 1, 10, 100 fields
- Exceed `max_fields_per_block` → `LimitExceeded`
- Corrupt magic → `CorruptBlock`
- **Endianness tests**: Verify `block_magic` (u32) is little-endian

### 2.3 Index Footer (Optional)

**File:** `src/footer.rs`

```rust
#[derive(Debug, Clone)]
pub struct IndexFooter {
    pub blocks: Vec<BlockIndexEntry>,
}

#[derive(Debug, Clone)]
pub struct BlockIndexEntry {
    pub block_offset: u64,
    pub block_size: usize,
    pub record_count: usize,
}

impl IndexFooter {
    pub fn encode(&self) -> Result<Vec<u8>>;
    pub fn decode(bytes: &[u8]) -> Result<Self>;
}
```

**Tests:**
- Round-trip with 0, 1, 100 blocks
- CRC32C verification
- **Endianness tests**: Verify `index_magic` (u32) and trailing footer pointer (u64) are little-endian

**Milestone:** File/block/footer structures serialize/deserialize correctly; limits enforced.

---

## Phase 3: Decimal & Type-Tag Support (`jac-format`)

### 3.1 Decimal Type

**File:** `src/decimal.rs`

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decimal {
    pub sign: bool,          // false = non-negative, true = negative
    pub digits: Vec<u8>,     // ASCII '0'..'9', MSB-first, no leading zeros
    pub exponent: i32,       // base-10
}

impl Decimal {
    // Primary: Parse JSON number string directly (preserves exact semantics)
    pub fn from_str_exact(s: &str) -> Result<Self>;

    // Optional: Detect when f64 storage is safe (per spec §4.5 optimization)
    pub fn from_f64_if_exact(val: f64) -> Option<Self>;

    // For testing/verification
    pub fn to_f64_if_exact(&self) -> Option<f64>;

    pub fn to_json_string(&self) -> String;     // canonical decimal output
    pub fn encode(&self) -> Result<Vec<u8>>;
    pub fn decode(bytes: &[u8]) -> Result<(Self, usize)>;
}
```

**Tasks:**
- [ ] Validate: sign in {0,1}, digits ASCII, no leading zeros (except "0")
- [ ] Enforce `max_decimal_digits_per_value`
- [ ] Exponent range: i32 (ZigZag+ULEB128)
- [ ] `to_json_string`: minimal formatting (no `+`, lowercase `e`, trim trailing zeros)

**Tests:**
- Round-trip: 0, 0.1, 1e-20, 1e+300, -123.456e10
- Large digits (65k limit)
- Invalid sign → `CorruptBlock`
- **Edge cases**:
  - Sign=0 for zero (MUST be non-negative)
  - No leading zeros in digits (except "0" itself)
  - Maximum digit count enforcement
  - Exponent range enforcement (i32 min/max)
  - Numbers exceeding i64 route to decimal (not int)
- **f64 exactness test**:
  - `0.1` is NOT exact in f64 → `to_f64_if_exact()` returns `None`
  - `0.5` IS exact in f64 → `to_f64_if_exact()` returns `Some(0.5)`
  - Encoder uses `from_str_exact()` as primary path, optionally detects f64 exactness

### 3.2 Type Tag Enum

**File:** `src/types.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TypeTag {
    Null = 0,
    Bool = 1,
    Int = 2,
    Decimal = 3,
    String = 4,
    Object = 5,
    Array = 6,
}

impl TypeTag {
    pub fn from_u8(val: u8) -> Result<Self>; // 7 → UnsupportedFeature
}
```

**Milestone:** Decimal encoding works; type-tag conversions safe.

---

## Phase 4: Column Builder & Encoder (`jac-codec`)

**Goal:** Build columnar representation from JSON records; emit compressed field segments.

**Dependencies:**
- Phase 1 (varint, bitpack, error types)
- Phase 2 (BlockHeader, FieldDirectoryEntry)
- Phase 3 (Decimal, TypeTag)

### 4.1 Column Builder

**File:** `src/column.rs`

```rust
pub struct ColumnBuilder {
    presence: BitVec,
    tags: Vec<TypeTag>,           // only for present positions
    bools: BitVec,
    ints: Vec<i64>,
    decimals: Vec<Decimal>,
    strings: Vec<String>,         // raw or for dict building
    objects: Vec<Vec<u8>>,        // minified JSON
    arrays: Vec<Vec<u8>>,         // minified JSON
}

impl ColumnBuilder {
    pub fn new(record_count: usize) -> Self;
    pub fn add_value(&mut self, record_idx: usize, value: &serde_json::Value);
    pub fn finalize(self, opts: &CompressOpts) -> Result<FieldSegment>;
}
```

**Tasks:**
- [ ] `add_value`: inspect type, set presence, push to appropriate substream
- [ ] **Integer detection** (classify JSON numbers):
  - If `serde_json::Number::is_i64()` → use INT
  - If `serde_json::Number::is_u64()`:
    - If `u64 <= i64::MAX` → convert to i64, use INT
    - If `u64 > i64::MAX` → use DECIMAL (cannot fit in signed 64-bit)
  - If `serde_json::Number::is_f64()` or non-integer → parse as DECIMAL using `Decimal::from_str_exact()`
- [ ] **FLAG_CANONICALIZE_NUMBERS behavior** (if flag set):
  - For decimals: Use scientific notation for |exponent| > 6, trim trailing zeros in mantissa
  - For integers: No change (already canonical)
  - Document exact rules in `CompressOpts` rustdoc
- [ ] Minify nested objects/arrays with `serde_json::to_string` (no whitespace)
- [ ] **FLAG_NESTED_OPAQUE behavior** (always true in v1):
  - Objects/arrays are stored as minified JSON strings (not recursively columnarized)
  - This flag reserved for future v2 nested columnarization
  - Must be set to true in v0.1.0; decoder should reject if false
- [ ] **Object/array handling**: Share string substream for tags 5/6 (objects/arrays stored as minified JSON strings)
- [ ] **Per-value limit enforcement** (during `add_value`):
  - Validate string length <= `max_string_len_per_value` before pushing
  - Validate decimal digit count <= `max_decimal_digits_per_value` before pushing
  - Track dictionary entry count; reject if exceeds `max_dict_entries_per_field`
- [ ] `finalize`:
  - Build presence bitmap
  - Pack type tags (3-bit)
  - **Validate computed sizes**:
    - presence_bytes <= `max_presence_bytes`
    - tag_bytes <= `max_tag_bytes`
    - dict_entry_count <= `max_dict_entries_per_field`
  - **Deterministic dictionary ordering**: Use **first-occurrence order** (preserves insertion order via `IndexMap` or similar). This is simpler and provides better compression due to locality. Dictionary order is implementation-defined and may change across encoder versions
  - Decide dictionary vs raw for strings (heuristic: distinct <= min(4096, present/8))
  - Encode substreams in spec order (§4.7 / §1.1 addendum)
  - Delta encoding for integers if `increasing_ratio >= 0.95` (first varint is base; subsequent are ZigZag+ULEB128 deltas)
  - **Record key order**: Lexicographic ordering when canonicalize_keys flag is set

**Tests:**
- Single column with mixed types: null, bool, int, string
- **Integer boundary cases**:
  - `i64::MAX` (9223372036854775807) → INT
  - `i64::MAX + 1` (9223372036854775808) → DECIMAL (u64 > i64::MAX)
  - Large float (1e100) → DECIMAL
  - Non-integer (1.5) → DECIMAL
- Dictionary threshold: cardinality 10 → dict, 10000 → raw
- Delta vs varint for monotonic integer sequences
- **Mixed string/object/array tags**: Test that objects and arrays share the string substream correctly
- **Per-value limit tests**:
  - String exceeding `max_string_len_per_value` → `LimitExceeded`
  - Decimal exceeding `max_decimal_digits_per_value` → `LimitExceeded`
  - Dictionary exceeding `max_dict_entries_per_field` → `LimitExceeded`
  - presence_bytes exceeding `max_presence_bytes` → `LimitExceeded`
  - tag_bytes exceeding `max_tag_bytes` → `LimitExceeded`

### 4.2 Field Segment Encoding

**File:** `src/segment.rs`

```rust
pub struct FieldSegment {
    pub uncompressed_payload: Vec<u8>, // ordered per §4.7
    pub encoding_flags: u64,
    pub dict_entry_count: usize,
    pub value_count_present: usize,
}

impl FieldSegment {
    pub fn compress(&self, codec: u8, level: u8) -> Result<Vec<u8>>;
}
```

**Tasks:**
- [ ] Assemble payload: presence | tags | dict | bool | int | decimal | string
- [ ] `compress`: match codec (0=none, 1=zstd); return compressed bytes
- [ ] Zstd: use `zstd::encode_all` with specified level
- [ ] **v0.1.0 scope**: Only implement codec ids 0 (none) and 1 (zstd). Return `UnsupportedCompression` for 2 (Brotli) and 3 (Deflate) — those are future work (Phase 12)

**Tests:**
- Segment with all type combinations
- Compression round-trip (zstd level 3, 15, 19)
- Unknown codec id → `UnsupportedCompression`

### 4.3 Block Builder

**File:** `src/block_builder.rs`

```rust
pub struct BlockBuilder {
    records: Vec<serde_json::Map<String, serde_json::Value>>,
    opts: CompressOpts,
}

impl BlockBuilder {
    pub fn new(opts: CompressOpts) -> Self;
    pub fn add_record(&mut self, rec: serde_json::Map<String, serde_json::Value>) -> Result<()>;
    pub fn is_full(&self) -> bool;
    pub fn finalize(self) -> Result<BlockData>;
}

pub struct BlockData {
    pub header: BlockHeader,
    pub segments: Vec<Vec<u8>>, // compressed
    pub crc32c: u32,
}
```

**Tasks:**
- [ ] Discover all fields across records
- [ ] Build `ColumnBuilder` per field
- [ ] **Memory budgeting**: Track estimated uncompressed size per column as records accumulate
- [ ] Early flush when approaching `max_block_uncompressed_total` (before record count limit)
- [ ] Finalize all columns → segments (produces uncompressed payloads)
- [ ] **Compress all segments** (two-pass approach required):
  - Pass 1: Compress each segment, collect compressed bytes
  - Pass 2: Compute offsets after knowing compressed sizes
- [ ] Populate `BlockHeader` with directory entries:
  - Compute `value_count_present` by counting set bits in presence bitmap (must match actual present count)
  - Compute `segment_offset` as absolute byte offset from start of block (after header)
  - `offset[0] = header_len`, `offset[i+1] = offset[i] + compressed_len[i]`
  - Ensure offsets are monotonically increasing and segments are contiguous
- [ ] Compute CRC32C over header + all segments

**Tests:**
- 1 record, 1 field
- 100 records, 10 fields with mixed presence
- Schema drift: field changes type across records
- **Memory budget test**: Add 50k records with large sparse string fields → verify early flush before exceeding `max_block_uncompressed_total`
- **Segment offset contiguity**: Verify `offset[i] + compressed_len[i] == offset[i+1]` for all segments
- Block size limits

**Milestones:**
- End of Week 5: ColumnBuilder builds columns from records with correct presence/tags/substreams
- End of Week 6: BlockBuilder produces valid blocks (header + segments + CRC)

---

## Phase 5: Segment Decoder (`jac-codec`)

### 5.1 Field Segment Decoder

**File:** `src/segment_decode.rs`

```rust
pub struct FieldSegmentDecoder {
    presence: BitVec,
    tags: Vec<TypeTag>,
    // Decompressed substreams (lazily parsed)
}

impl FieldSegmentDecoder {
    pub fn new(compressed: &[u8], dir_entry: &FieldDirectoryEntry, limits: &Limits) -> Result<Self>;
    pub fn get_value(&self, record_idx: usize) -> Result<Option<serde_json::Value>>;
}
```

**Tasks:**
- [ ] **Pre-decompression guard**: Validate `segment_uncompressed_len` against limits BEFORE allocating or decompressing
- [ ] **Limit enforcement order**: (1) Read `segment_uncompressed_len` from directory, (2) Validate against `max_segment_uncompressed_len`, (3) Allocate output buffer, (4) Decompress
- [ ] Decompress with zstd (verify actual decompressed size matches `segment_uncompressed_len`)
- [ ] **Note:** Per-segment checksum verification is handled at block level (see Phase 5.2); block CRC32C covers header + all segments
- [ ] **Note:** Decompression happens per-call in v0.1.0. Future optimization (Phase 10) may cache decompressed segments within `BlockDecoder` for repeated projections
- [ ] **Validate size formulas and per-field limits** (after decompression, before parsing):
  - `presence_bytes = (record_count + 7) >> 3` and <= `max_presence_bytes`
  - `tag_bytes = ((3 * present_count) + 7) >> 3` and <= `max_tag_bytes`
  - `dict_entry_count` (from directory) <= `max_dict_entries_per_field`
  - Verify unused high bits in final tag byte are zero
- [ ] Parse presence bitmap
- [ ] **Validate `value_count_present`**: Count set bits in presence bitmap and verify matches directory entry (mismatch → `CorruptBlock`)
- [ ] Unpack type tags (3-bit)
- [ ] Parse dictionary (if present)
- [ ] Cursors for each substream (bool, int, decimal, string)
- [ ] **Delta encoding decoder** (if `ENCODING_FLAG_DELTA` set):
  - Read base value (first ZigZag+ULEB128 varint)
  - For each subsequent value: read delta, accumulate: `value = prev_value + delta`
  - Return reconstructed sequence
- [ ] `get_value`: check presence, read tag, fetch from appropriate substream
  - Strings: validate length <= `max_string_len_per_value` before returning
  - Decimals: validate digit count <= `max_decimal_digits_per_value` before returning
  - Objects/arrays: parse minified JSON from string substream (shared storage)

**Tests:**
- Decode segment with all types
- Dictionary lookups (valid index, out-of-range → `DictionaryError`)
- Corrupted zstd → `DecompressError`
- **Reserved tag test**: Tag value 7 → `UnsupportedFeature`
- **Size formula validation**: Test edge counts to ensure formulas and padding are correct
- **`value_count_present` mismatch**: Directory claims 100 present values but bitmap has 50 → `CorruptBlock`
- **Delta encoding round-trip**: Monotonic sequence [1000, 1001, 1002, 1003] → encode with delta → decode → verify original values
- **Per-field/per-value limit tests** (malicious/crafted inputs):
  - presence_bytes exceeding `max_presence_bytes` → `LimitExceeded`
  - tag_bytes exceeding `max_tag_bytes` → `LimitExceeded`
  - dict_entry_count exceeding `max_dict_entries_per_field` → `LimitExceeded`
  - String value exceeding `max_string_len_per_value` → `LimitExceeded`
  - Decimal digits exceeding `max_decimal_digits_per_value` → `LimitExceeded`

### 5.2 Block Decoder

**File:** `src/block_decode.rs`

```rust
pub struct BlockDecoder {
    header: BlockHeader,
    segment_bytes: Vec<Vec<u8>>, // per field
    opts: DecompressOpts,
}

impl BlockDecoder {
    pub fn new(block_bytes: &[u8], opts: &DecompressOpts) -> Result<Self>;
    pub fn decode_records(&self) -> Result<Vec<serde_json::Map<String, serde_json::Value>>>;
    pub fn project_field(&self, field_name: &str) -> Result<Vec<Option<serde_json::Value>>>;
}
```

**Tasks:**
- [ ] Parse block header
- [ ] Store `opts: DecompressOpts` in struct; use `opts.limits` for limit checks and `opts.verify_checksums` for CRC verification
- [ ] **Enforce `max_block_uncompressed_total`**: Check sum of all `segment_uncompressed_len` before allocating
- [ ] **Validate segment layout** (before decompressing):
  - Verify `segment_offset` and `segment_compressed_len` are within `block_bytes.len()`
  - Verify offsets are monotonically increasing (no overlap)
  - Verify segments are contiguous: `offset[i] + compressed_len[i] = offset[i+1]` (or end of block for last segment)
  - Mismatch → `CorruptBlock`
- [ ] **Verify block CRC32C**: If `opts.verify_checksums` is true (default), compute and verify CRC32C; if false, skip verification
- [ ] Split segments by directory offsets
- [ ] Forward-compatibility: Skip unknown directory fields using `header_len` bounds
- [ ] `decode_records`: decode all fields, reconstruct records (omit Absent, include Null); pass `opts.limits` to `FieldSegmentDecoder`
- [ ] `project_field`: decode only requested field segment; pass `opts.limits` to `FieldSegmentDecoder`

**Tests:**
- Full round-trip: JSON → encode block → decode block → JSON (semantic equality)
- Projection: extract single field
- CRC mismatch → `ChecksumMismatch`
- **`verify_checksums` flag**:
  - Decode with `verify_checksums=true` and corrupted block CRC → `ChecksumMismatch`
  - Decode with `verify_checksums=false` and corrupted block CRC → succeeds (no verification)
  - Note: Only block-level CRC exists per spec §3.4; it covers header + all segments
- **Block size limit**: Test exceeding `max_block_uncompressed_total` → `LimitExceeded`
- **Forward-compatibility**: Add extra unknown directory field and verify decoder skips it
- **Segment layout validation** (malicious/crafted blocks):
  - Overlapping segments (offset[1] < offset[0] + len[0]) → `CorruptBlock`
  - Segment past block end (offset + len > block_bytes.len()) → `CorruptBlock`
  - Non-contiguous segments (gap between segments) → `CorruptBlock`
  - Truncated block (last segment extends past available bytes) → `CorruptBlock`

**Milestone:** Encoder + decoder working end-to-end for blocks.

**Intermediate Conformance Check:**
- [ ] Run spec §12.1 conformance test (4 NDJSON records) at end of Phase 5:
  - Encode the 4-record fixture
  - Decode and verify semantic equality
  - Project "user" field → verify `["alice","alice","bob","carol"]`
- This early check catches holistic regressions before higher-level crates (Phase 6-8) add complexity

---

## Phase 6: File I/O Layer (`jac-io`)

### 6.1 Writer

**File:** `src/writer.rs`

```rust
pub struct JacWriter<W: Write> {
    writer: W,
    opts: CompressOpts,
    block_builder: BlockBuilder,
    block_index: Vec<BlockIndexEntry>,
}

impl<W: Write> JacWriter<W> {
    pub fn new(writer: W, header: FileHeader, opts: CompressOpts) -> Result<Self>;
    pub fn write_record(&mut self, rec: &serde_json::Map<String, serde_json::Value>) -> Result<()>;
    pub fn finish(self, with_index: bool) -> Result<()>;
}
```

**Tasks:**
- [ ] Write file header on `new`
- [ ] Accumulate records in `block_builder`; flush when full
- [ ] Track block offsets for index
- [ ] `finish`: flush final block, optionally write footer + u64 pointer
- [ ] Streamability: Only emit complete blocks (header, all segments, CRC). Provide `flush()` to emit partial final block safely
- [ ] **Drop guard**: Implement `Drop` that warns (via `tracing`/`log`) or panics in debug mode if `finish()` wasn't called
- [ ] Document that `finish()` is required to avoid silent data loss

**Tests:**
- Write 0, 1, 1000, 100k records
- With/without index
- Verify file structure: header | blocks | footer
- Test that dropping without `finish()` triggers warning/panic in debug mode

### 6.2 Reader

**File:** `src/reader.rs`

```rust
// Block handle (opaque reference to a block for projection/decoding)
pub struct BlockHandle {
    offset: u64,                        // file offset of block start
    size: usize,                        // total block size (header + segments + CRC)
    record_count: usize,                // records in this block
    // Internal: cached BlockHeader for efficiency
}

// Field value iterator (for projection queries)
pub struct FieldIterator {
    decoder: FieldSegmentDecoder,       // segment decoder for the field
    record_count: usize,                // total records in block
    current_idx: usize,                 // iteration state
}

impl Iterator for FieldIterator {
    type Item = Result<Option<serde_json::Value>>;  // None = absent
    fn next(&mut self) -> Option<Self::Item>;
}

pub struct JacReader<R: Read + Seek> {
    reader: R,
    file_header: FileHeader,
    index: Option<IndexFooter>,
    opts: DecompressOpts,
    strict_mode: bool, // default true
}

impl<R: Read + Seek> JacReader<R> {
    pub fn new(reader: R, opts: DecompressOpts) -> Result<Self>;
    pub fn with_strict_mode(reader: R, opts: DecompressOpts, strict: bool) -> Result<Self>;
    pub fn blocks(&mut self) -> impl Iterator<Item = Result<BlockHandle>>;
    pub fn project_field(&mut self, block: &BlockHandle, field: &str) -> Result<FieldIterator>;
}
```

**Tasks:**
- [ ] Read & validate file header
- [ ] Store `DecompressOpts` in reader state; pass to `BlockDecoder` constructor (which handles block CRC verification using `opts.verify_checksums`)
- [ ] Optionally read index footer (seek to u64 pointer)
- [ ] `blocks`: iterator that reads block headers sequentially
- [ ] **Streaming error recovery** (when `strict_mode = false`):
  - On block CRC failure or corruption, attempt to resync by scanning for next `BLOCK_MAGIC` (0x314B4C42)
  - If resync succeeds, continue from next block
  - If strict_mode = true (default), abort on first error
  - Rationale: False positive risk, but useful for damage isolation in archival scenarios
- [ ] `project_field`: seek to field segment, decompress, return iterator (pass `&opts.limits` to `FieldSegmentDecoder::new()`)

**Tests:**
- Read file written by `JacWriter`
- Streaming (no index): iterate all blocks
- Random access (with index): jump to block N
- Projection: extract field from all blocks
- **Index/footer behavior**:
  - Files with 8-byte footer pointer
  - Files without footer (streaming mode)
  - Verify footer CRC32C
  - Reader gracefully handles both cases
- **Error recovery**:
  - Corrupt block mid-file with `strict_mode=false` → resync to next block, continue
  - Corrupt block mid-file with `strict_mode=true` → abort on first error

**Milestone:** Full file I/O working; can compress/decompress NDJSON files.

---

## Phase 7: High-Level API & JSON Streaming (`jac-io`)

### 7.1 High-Level Functions

**File:** `src/lib.rs` (in `jac-io`)

```rust
// High-level compression options (defined in jac-io, re-exported from root)
pub struct CompressOpts {
    pub block_target_records: usize,    // default: 100_000
    pub default_codec: Codec,           // default: Zstd(15)
    pub canonicalize_keys: bool,        // FLAG_CANONICALIZE_KEYS: lexicographic key order
    pub canonicalize_numbers: bool,     // FLAG_CANONICALIZE_NUMBERS: scientific notation, trim trailing zeros
    pub nested_opaque: bool,            // FLAG_NESTED_OPAQUE: must be true in v1 (always set; reserved for v2 columnarization)
    pub max_dict_entries: usize,        // default: 4_096
    pub limits: Limits,                 // default: Limits::default()
}

// Decompression options
pub struct DecompressOpts {
    pub limits: Limits,                 // default: Limits::default()
    pub verify_checksums: bool,         // default: true (always verify CRC32C)
}

pub fn compress<R: Read, W: Write>(input: R, output: W, opts: CompressOpts) -> Result<()>;
pub fn decompress_full<R: Read + Seek, W: Write>(input: R, output: W, opts: DecompressOpts) -> Result<()>;
pub fn project<R: Read + Seek, W: Write>(input: R, output: W, fields: &[&str], as_ndjson: bool) -> Result<()>;
```

**Tasks:**
- [ ] Define `CompressOpts` with all flag fields
- [ ] Validate `nested_opaque` must be true in v0.1.0 (constructor or helper method rejects if false)
- [ ] Map `CompressOpts` flags to `FileHeader` bitfield during compression
- [ ] Note: `CompressOpts` is passed down to `BlockBuilder` and `ColumnBuilder::finalize()` (no separate `EncodeOpts` type)
- [ ] `compress`: detect NDJSON vs JSON array; stream records to `JacWriter`
- [ ] Input detection:
  - First non-whitespace '[' ⇒ stream as JSON array (use StreamDeserializer)
  - Otherwise treat as NDJSON (line-delimited objects)
  - Optional: explicit mode to accept a single JSON object, otherwise return clear error
- [ ] Use `simd-json` or `serde_json::StreamDeserializer` for efficiency
- [ ] `decompress_full`: read all blocks, emit NDJSON or JSON array
- [ ] `project`: extract specified fields, emit as NDJSON or JSON array

**Tests:**
- Compress sample NDJSON (spec §12.1) → verify output
- Decompress → semantic equality
- Project "user" field → ["alice","alice","bob","carol"]
- **FLAG_CANONICALIZE_NUMBERS**:
  - Encode `{"val": 1.50000}` with flag set → decode → verify `1.5` (trimmed trailing zeros)
  - Encode `{"val": 10000000.0}` with flag set → decode → verify `1e7` (scientific notation)
- **FLAG_NESTED_OPAQUE validation**:
  - Attempt to construct `CompressOpts { nested_opaque: false, ... }` → verify error (`UnsupportedFeature` or validation error)
  - Verify encoder always sets to true in v0.1.0
  - Test decoder rejects files with flag=false → `UnsupportedFeature`

### 7.2 Concurrency (Rayon)

**File:** `src/parallel.rs`

**Tasks:**
- [x] Parallel block compression: use `rayon` to encode blocks in parallel
- [x] Parallel decompression: decode multiple blocks concurrently
- [x] Ensure deterministic output (block order preserved)

**Tests:**
- Compress 10 blocks in parallel → same output as sequential (JAC-layer determinism: presence/tags/dicts/ordering)
- If bit-identical output is required, pin zstd version and single-threaded encoder; verify frame params
- **Determinism regression test**:
  - Run encoder twice on identical input (same data, same opts)
  - Verify block ordering is deterministic
  - Verify projected field results match between sequential and parallel modes
  - If bit-identical output is required, verify byte-for-byte match
- Benchmark: 1 thread vs N threads

**Milestone:** High-level API complete; concurrency working; ready for CLI.

---

## Phase 8: CLI Tool (`jac-cli`)

### 8.1 Commands

**File:** `src/main.rs`

```rust
// Using clap for CLI parsing
Commands:
- pack: compress JSON/NDJSON → .jac
- unpack: decompress .jac → JSON/NDJSON
- ls: list blocks, fields, record counts
- cat: stream values for a field
```

**Tasks:**
- [ ] `jac pack input.ndjson -o output.jac --block-records 100000 --zstd-level 15`
- [ ] **Expose canonicalization flags as CLI options**:
  - `--canonicalize-keys`: Enable lexicographic key ordering (sets FLAG_CANONICALIZE_KEYS)
  - `--canonicalize-numbers`: Enable scientific notation and trailing zero trimming (sets FLAG_CANONICALIZE_NUMBERS)
  - Both flags map to `CompressOpts` fields
- [ ] `jac unpack input.jac -o output.ndjson --ndjson`
- [ ] `jac ls input.jac` → table of blocks with stats
- [ ] `jac cat input.jac --field userId` → stream values
- [ ] **`pack --project` semantics**: Document that this filters input records to only include specified fields before encoding (reduces .jac size)
  - Document precedence of per-field codec overrides vs global defaults
- [x] Persist source container hint in header flags and auto-select NDJSON vs JSON array on unpack (CLI default follows hint).

**Tests:**
- End-to-end: pack → unpack → diff (semantic)
- **Projection via pack**: `jac pack --project user,ts -o projected.jac input.ndjson` → verify only those fields in output
- Integration test to prevent semantic drift between pack projection and read projection
- **Canonicalization flags**:
  - `jac pack --canonicalize-keys input.ndjson -o out.jac` → verify keys are lexicographic on decode
  - `jac pack --canonicalize-numbers input.ndjson -o out.jac` → verify trailing zeros trimmed, scientific notation used

**Documentation:**
- [ ] README.md with usage examples
- [ ] `--help` text for each command
- [ ] Clarify difference between `pack --project` (writes subset) vs `cat --field` (reads subset)

**Milestone:** CLI functional; can be used for real workloads.

---

## Phase 9: Testing & Validation

### 9.1 Conformance Tests

**File:** `jac-codec/tests/conformance.rs`

**Tasks:**
- [ ] Implement spec §12.1 test case (4 NDJSON records)
- [ ] Verify field encodings: ts (delta int), level (dict), error (absent)
- [ ] Verify projection output
- [ ] Round-trip equality for diverse JSON corpus:
  - Unicode strings, large numbers, nested objects
  - Edge cases: empty array, single record, 1M records

### 9.2 Fuzz Testing

**File:** `jac-codec/fuzz/`

**Tasks:**
- [ ] Set up `cargo-fuzz` or `proptest`
- [ ] Fuzz varint decode (malformed input)
- [ ] Fuzz block decode (corrupted headers, segments)
- [ ] Fuzz dictionary indices (out of bounds)

### 9.3 Error Handling Tests

**Tasks:**
- [ ] Corrupt CRC → `ChecksumMismatch`
- [ ] Truncated file → `UnexpectedEof`
- [ ] Exceed limits → `LimitExceeded`
- [ ] Unknown compressor → `UnsupportedCompression`
- [ ] Tag value 7 → `UnsupportedFeature`

### 9.4 Specification Compliance Checklist

**Tasks:**
- [ ] Create compliance matrix mapping each spec MUST/SHOULD requirement to implementation task
- [ ] Verify all MUST requirements have passing tests
- [ ] Document any SHOULD requirements deferred to future versions

**Milestone:** All conformance tests pass; fuzzer runs without crashes; error cases handled gracefully.

---

## Phase 10: Benchmarks & Optimization

### 10.1 Benchmark Suite

**File:** `jac-codec/benches/`

**Tasks:**
- [ ] Datasets (download or generate):
  - Server logs (NDJSON, 1M records)
  - GitHub events (nested JSON)
  - Synthetic high-cardinality
- [ ] **Dataset reproducibility**: Script synthetic dataset generation (`testdata/generate_bench.sh`) or mirror small versions of public datasets for offline/CI use
- [ ] Metrics:
  - Compression ratio vs minified JSON+zstd
  - Compress time (1 thread, N threads)
  - Decompress time (full, projected field)
- [ ] Comparators: gzip, brotli, MessagePack+zstd

**Tools:**
- [ ] Use `criterion` for microbenchmarks
- [ ] CI integration: track performance over commits

### 10.2 Optimizations

**Potential areas (after profiling):**
- [ ] Evaluate segment decompression caching: Profile repeated projections on same block; if bottleneck, implement cache in `BlockDecoder`
- [ ] SIMD for varint encoding/decoding
- [ ] Zero-copy deserialization where possible
- [ ] Dictionary hashing (ahash)
- [ ] Reuse allocations (object pools)

**Milestone:** Benchmark results documented; meets or exceeds spec targets (20-40% reduction vs JSON+zstd).

---

## Phase 11: Documentation & Release Prep

### 11.1 API Documentation

**Tasks:**
- [ ] Rustdoc for all public APIs
- [ ] Examples in doc comments
- [ ] `cargo doc --no-deps --open` renders correctly

### 11.2 User Guide

**File:** `docs/guide.md`

**Sections:**
- Getting started
- Compression options (block size, zstd level)
- Projection queries
- Performance tuning
- Security considerations (limits)

### 11.3 Crate Publishing

**Tasks:**
- [ ] Version all crates: 0.1.0
- [ ] README.md badges (CI, crates.io, docs.rs)
- [ ] `cargo publish --dry-run` for all crates
- [ ] Publish order: `jac-format` → `jac-codec` → `jac-io` → `jac-cli`

**Milestone:** v0.1.0 released to crates.io; docs live on docs.rs.

### 11.4 Examples & Versioning Notes

**Examples (`examples/`):**
- `compress_ndjson.rs` - Basic compression with default options (demonstrates simple use case)
- `decompress_full.rs` - Full decompression to JSON/NDJSON (demonstrates round-trip)
- `project_field.rs` - Selective field extraction without full decompression (demonstrates projection performance)
- `custom_limits.rs` - Custom security limits for untrusted input (demonstrates limit configuration)

**Versioning:**
- Rust crates: semantic versions (start at 0.1.0; 1.0.0 for spec v1.0 final)
- Wire format version is encoded in `FILE_MAGIC` (0x01 for v1)
- Crate v0.x implements spec Draft 0.9.1 (pre-stable)

---

## Phase 12: Optional Extensions (Future)

### 12.1 WASM Bindings (`jac-wasm`)

**Goal:** Decoder + projection for browser use.

**Tasks:**
- [ ] Set up `wasm-bindgen`
- [ ] Export `project_fields(Uint8Array, fields: string[]) -> JsValue`
- [ ] Optimize for small block sizes (10k-50k records)
- [ ] NPM package

### 12.2 Python Bindings (`jac-python`)

**Goal:** PyO3 wrapper for Python users.

**Tasks:**
- [ ] Set up `maturin`
- [ ] Expose `compress`, `decompress`, `project` APIs
- [ ] PyPI package

### 12.3 Advanced Features (v2)

- Nested columnarization (Parquet-like repetition/definition levels)
- Global dictionaries across blocks
- Bloom filters for predicate pushdown
- Brotli/LZMA codec support

---

## Success Criteria

✅ **Spec Compliance:**
- All MUST requirements from spec implemented
- Conformance test (§12.1) passes
- Semantic round-trip equality for diverse JSON

✅ **Performance:**
- ≥20% smaller than minified JSON+zstd (on logs/NDJSON)
- Projection ~O(records) time (only reads target segments)

✅ **Quality:**
- All tests pass (unit, integration, fuzz)
- Clippy clean (no warnings)
- rustfmt applied
- No unsafe outside audited hot paths

✅ **Usability:**
- CLI intuitive (matches spec §10)
- API docs complete
- Compiles on stable Rust

---

## Risk Mitigation

**Risk:** Decimal encoding complexity
**Mitigation:** Implement early (Phase 3); extensive test coverage for edge cases

**Risk:** Compression ratio doesn't meet targets
**Mitigation:** Profile and optimize dictionary heuristics; try alternative string encodings

**Risk:** Concurrency bugs (data races, ordering)
**Mitigation:** Use safe Rust primitives (rayon); determinism tests; TSan in CI

**Risk:** Decoder security (decompression bombs)
**Mitigation:** Enforce all limits from spec §2.1; fuzz testing; cap decompressed sizes

---

## Next Steps

1. Create Phase 0 (setup) branch; initialize workspace
2. Complete project setup; ensure CI passes
3. Begin Phase 1 (primitives) with comprehensive testing

**Best Practices:**
- Commit frequently with clear messages
- Run tests and clippy after each change
- Review code before merging phases

---

**Questions or clarifications needed?** Please update this plan as implementation progresses and new insights emerge.
