# JAC v1 — JSON‑Aware Compression Format
**Status:** Draft 0.9 (implementation‑ready)
**Scope:** Archival storage; semantic (not byte‑identical) JSON round‑trip; partial/columnar decode
**Reference Implementation Language:** **Rust**

> **BCP‑14 Keywords:** The key words **MUST**, **MUST NOT**, **REQUIRED**, **SHALL**, **SHALL NOT**, **SHOULD**, **SHOULD NOT**, **RECOMMENDED**, **NOT RECOMMENDED**, **MAY**, and **OPTIONAL** in this document are to be interpreted as described in RFC 2119 / RFC 8174.

---

## 0. High‑Level Summary

**JAC** (JSON‑Aware Compression) is a binary container and encoding for JSON designed for **archival** workloads where **compression ratio** is the top priority, **semantic round‑trip** is required (not byte‑identical formatting), and **partial decoding** (field/column projection) is desirable.

Key properties:

- **Block + Columnar** layout for arrays/streams of objects.
- **Dictionary** encoding for keys and string values; **bit‑packing/RLE** for booleans; **varint** (LEB128) + **delta** for integers.
- **Union‑typed columns** (per field) with **type‑tags** so schema drift is tolerated without up‑front schemas.
- **Per‑field compressed segments** (default **Zstandard**), enabling **field‑only** extraction without scanning full blocks.
- **Semantic JSON** on decode: keys may be re‑ordered; whitespace, formatting, and numeric spellings may be canonicalized.

The spec defines: file/container format, block and field segment layouts, encodings, error handling, and an implementation blueprint in Rust (APIs, crate layout, concurrency, test vectors, and benchmark plan).

---

## 1. Use Model & Non‑Goals

**Primarily targeted input shape:**
- Top‑level **array of objects** or **NDJSON** (one object per line).
- Nested objects/arrays inside fields are supported (stored as opaque JSON subdocuments in v1; see §4.5).

**Non‑Goals v1:**
- Full schema‑driven columnarization of arbitrary nested JSON (planned for v2).
- Byte‑identical JSON regeneration (we ensure semantic equality only).

---

## 2. Terminology & Conventions

- **Record**: One top‑level object (an NDJSON line or an array element).
- **Field**: A key inside a record (e.g., `"userId"`).
- **Column**: Values of a single field across records in a block.
- **Absent**: Key not present in the record.
- **Null**: Key present with value `null`.
  - JAC **distinguishes** Absent vs Null.
- **ULEB128**: Unsigned LEB128 variable‑length integer.
- **ZigZag**: Mapping signed integers to unsigned to improve varint packing.

---

## 3. File & Block Structure

### 3.1. File Layout (big picture)
+———————+
| File Header         |  Magic, version, global opts
+———————+
| Block 0             |  Header + directory + field segments
+———————+
| Block 1             |
+———————+
| …                 |
+———————+
| Optional Index      |  Block offsets, record counts, metadata (footer)
+———————+

### 3.2. Magic & Version

- **Magic** (4 bytes): `0x4A 0x41 0x43 0x01`  → ASCII `"JAC"` + version `0x01`
- **Endianness:** All fixed‑width integers **little‑endian**. Counts/lengths **ULEB128** unless stated otherwise.

### 3.3. File Header

| Field                        | Type         | Description                                                                 |
|-----------------------------|--------------|-----------------------------------------------------------------------------|
| magic                       | [4]u8        | `JAC\x01`                                                                  |
| flags                       | u32          | Bit 0: canonicalize keys; Bit 1: canonicalize numbers; Bit 2: nested opaque; Bits 3-4: container hint (00=unknown, 01=ndjson, 10=json array, 11=reserved) |
| default_compressor          | u8           | 0=none, **1=zstd**, 2=brotli, 3=deflate (extensible)                       |
| default_compression_level   | u8           | Codec‑specific level hint (e.g., zstd 1..22)                               |
| block_size_hint_records     | ULEB128      | OPTIONAL; 0 means unknown                                                  |
| user_metadata_len           | ULEB128      | Length of optional metadata blob                                           |
| user_metadata               | bytes        | Opaque; recommended UTF-8 JSON/CBOR                                        |

If present, the `user_metadata` blob MAY contain UTF-8 JSON with the key `segment_max_bytes` (unsigned). Encoders that raise the segment ceiling above the default 64 MiB SHOULD record this value so decoders inheriting default limits can enforce the producer's ceiling.

Bits 3–4 of the `flags` field encode the container format hint observed during compression:

- `00` — Unknown (default; decoders treat as NDJSON when no override is provided)
- `01` — NDJSON input
- `10` — JSON array input
- `11` — Reserved; decoders MUST fail with `UnsupportedFeature`

Decoders use the hint to select a default wrapper when callers do not specify an output format. Implementations MUST still honour explicit caller overrides even when a hint is present.

**Note:** Container **MAY** omit index/footer; decoders **MUST** tolerate streaming scenarios.

### 3.4. Blocks

Each block independently compresses **N records** (N configurable). Blocks enable parallelism, seekability, and damage isolation.

**Block Layout**

+—————————––+
| Block Header                  |
|  - record_count               |
|  - field_count                |
|  - field directory entries… |
+—————————––+
| Field Segment 0 (compressed)  |
+—————————––+
| Field Segment 1 (compressed)  |
+—————————––+
| …                           |
+—————————––+
| Block CRC32C                  |
+—————————––+
**Block Header**

| Field                          | Type     | Description                                                          |
|--------------------------------|----------|----------------------------------------------------------------------|
| block_magic                    | u32      | `"BLK1"` = 0x314B4C42                                               |
| header_len                     | ULEB128  | Number of bytes from here to start of first field segment            |
| record_count                   | ULEB128  | Records in this block                                               |
| field_count                    | ULEB128  | Distinct fields present in this block                               |
| directory (per field)          | struct[] | See below                                                           |

**Directory Entry (per field)**

| Field                         | Type     | Description                                                                                 |
|------------------------------|----------|---------------------------------------------------------------------------------------------|
| field_name_len               | ULEB128  | Length of UTF‑8 field name                                                                  |
| field_name_utf8              | bytes    | The field name                                                                              |
| compressor                   | u8       | Overrides file default if non‑zero                                                          |
| compression_level            | u8       | Per‑field override                                                                          |
| presence_bytes               | ULEB128  | Size in bytes of presence bitmap inside the segment (uncompressed length)                   |
| tag_bytes                    | ULEB128  | Size in bytes of type‑tag stream inside the segment (for present values)                    |
| value_count_present          | ULEB128  | Number of **present** entries (sum of presence bits)                                        |
| encoding_flags               | ULEB128  | Bitfield (dictionary, delta, rle, etc.)                                                     |
| dict_entry_count             | ULEB128  | String dictionary entries **inside** segment (0 if none)                                    |
| segment_uncompressed_len     | ULEB128  | Uncompressed payload length (presence + tags + dict + substreams)                           |
| segment_compressed_len       | ULEB128  | Compressed payload length                                                                    |
| segment_offset               | ULEB128  | Byte offset from start of block to beginning of this field’s compressed segment             |

**Block CRC32C** (4 bytes): CRC over **header bytes + all field segments**. Decoders **MUST** verify.

---

## 4. Field Segments & Encodings

A field segment (after decompressing with its codec) is a **self‑contained payload** with:
[Presence Bitmap][Type Tags][Optional Dictionary][Typed Substreams …]
### 4.1. Presence Bitmap (Absent vs Present)

- Length = `presence_bytes` in directory.
- **Bit i** corresponds to **record i**; **1 = present**, **0 = absent**.
- If present=0 ⇒ no value to read for that record; type tag is **not** emitted for that position.

### 4.2. Type Tags (Union‑typed columns)

- Emitted **only for present positions**, contiguous and tightly packed.
- **3‑bit code** per value (packed LSB‑first into bytes):

| Code | Type     | Notes                                                |
|------|----------|------------------------------------------------------|
| 0    | null     | No substream payload read                            |
| 1    | bool     | Read from boolean substream (bit‑packed)             |
| 2    | int      | Read from integer substream                          |
| 3    | decimal  | Read from decimal substream (exact decimal)          |
| 4    | string   | Read from string substream (may be dict‑coded)       |
| 5    | object   | Opaque JSON subdocument (minified string)            |
| 6    | array    | Opaque JSON subdocument (minified string)            |
| 7    | reserved | Future extension                                     |

This design **tolerates schema drift** (a field may change type across records) without an up‑front schema.

### 4.3. Boolean Substream

- Values are **bit‑packed**: 8 values per byte, LSB‑first.
- Emitted in the order of **present** positions tagged as `bool`.

### 4.4. Integer Substream

- **Encoding selection (per field, per block):**
  - **ZigZag + ULEB128** (**RECOMMENDED default**).
  - **Delta + ZigZag + ULEB128** (if monotonic/incremental; indicated by `encoding_flags` and includes one base value as first varint).
  - Optional small‑bit‑width **bit‑packing** MAY be used (future flag).

- Only integers that fit in **signed 64‑bit** may use integer encoding.
  Values that exceed range **MUST** be encoded as **decimal** (see 4.5).

### 4.5. Decimal (Exact) Substream

To maintain **semantic equality**, non‑integer numbers are encoded **exactly**:

- **Canonical Decimal** representation:
  - Capture as **(sign, digits, exponent10)** where `digits` is a base‑10 big integer **with no leading zeros**, and `exponent10` is a signed 32‑bit integer.
  - On decode, format to a **minimal** JSON decimal string (no superfluous `+`, trim trailing zeros unless exponent used, use lowercase `e` if scientific notation is chosen).
  - Implementations **MAY** choose pretty‑printing rules, but **MUST** preserve numeric value exactly.
  - Wire format: `sign` (1 bit), `digits_len` (ULEB128), `digits` (that many base‑10 ASCII bytes), `exp10` (ZigZag+ULEB128).
- **Optimization:** If all decimals in the substream fit IEEE‑754 **exactly** (detected by round‑trip test), an encoder **MAY** set a flag to store as **f64 bytes** for better ratio; decoders must still regenerate **canonical** decimal strings, not raw IEEE formatting.

### 4.6. String Substream

- **Two modes** (signaled via `encoding_flags`):
  1. **Dictionary encoding** (recommended when cardinality is small):
     - A dictionary of **`dict_entry_count`** strings appears before the index stream.
     - Dictionary entry format: `len` (ULEB128) + UTF‑8 bytes.
     - Values are **indices** (ULEB128) into this dictionary.
     - **Null** is represented via type tag `null` (not a dictionary entry).
  2. **Raw strings**:
     - Each string: `len` (ULEB128) + UTF‑8 bytes.

- **Note:** For present values tagged `object` or `array` (type tags 5/6), the **string substream** carries a **minified JSON text** of the subdocument. Encoders **SHOULD** minify (remove whitespace) and **MAY** canonicalize key order within subdocuments if desired.

### 4.7. Segment Order & Sizes

Inside a field segment, the payload components appear **in this order**:

1. Presence bitmap (raw bytes).
2. Type tag stream (raw packed bytes).
3. Optional **String dictionary** (if any).
4. **Boolean substream** (byte‑aligned).
5. **Integer substream** (varints).
6. **Decimal substream**.
7. **String substream** (indices or raw strings).
8. **Object/Array substream** (if encoder separates from “plain” strings; otherwise share the string substream and identify by tags).

All bytes above are then **compressed as a single blob** with the field’s compressor.

---

## 5. Canonicalization & Semantic Round‑Trip

- **Key order** in objects MAY be changed (e.g., lexicographic) for compression stability.
- **Whitespace** is not preserved.
- **Numbers** MUST round‑trip as exact numeric values (see §4.5). Formatting (e.g., `1e6` vs `1000000`) MAY differ.
- **Strings** MUST round‑trip byte‑exact (UTF‑8).
- **Absent vs Null** MUST be preserved.

---

## 6. Compression Algorithms

- Default **compressor id = 1 (Zstandard)**.
  - Encoders **SHOULD** use a **high ratio** level for archival (e.g., 15–19).
  - Decoders **MUST** support at least id=0 (none) and id=1 (zstd).
- Per‑field `compressor`/`compression_level` can override the file default.
- Future: id=2 (Brotli), id=3 (Deflate). Decoders **MAY** support them.

---

## 7. Index / Footer (OPTIONAL)

To accelerate random access:

**Footer Layout**
+——————————+
| “IDX1” magic (u32)           |
| index_len (ULEB128)          |
| block_count (ULEB128)        |
| repeated {                   |
|   block_offset (ULEB128)     |
|   block_size   (ULEB128)     |
|   record_count (ULEB128)     |
| }* block_count               |
| footer_crc32c (u32)          |
+——————————+
If present, the file **SHOULD** end with a **8‑byte absolute pointer** (little‑endian u64) to the start of `"IDX1"` to allow locating the index without scanning.

---

## 8. Errors & Robustness

- **Integrity**: Block CRC32C **MUST** be verified. Footer CRC32C if present.
- **Bounds**: Decoders **MUST** enforce sane limits (e.g., max record_count, max field_count, max presence/tag lengths) to avoid OOM or decompression bombs.
- **Type Safety**: If a value exceeds 64‑bit integer, it **MUST** be encoded as decimal.
- **Dictionary**: Index out of range ⇒ **CorruptData** error.
- **Versioning**: Unknown magic or major version ⇒ **UnsupportedVersion**.

Suggested error taxonomy (Rust enum):
`InvalidMagic | UnsupportedVersion | CorruptHeader | CorruptBlock | ChecksumMismatch | UnexpectedEof | DecompressError | LimitExceeded | TypeMismatch | DictionaryError | Internal`

---

## 9. Reference Implementation (Rust)

### 9.1. Crate Topology
jac/
├─ jac-format/         # No-IO: spec types, varint, bitpack, CRC, constants
├─ jac-codec/          # Encoder/decoder engines (block builder, columnizer)
├─ jac-io/             # Streaming IO, file header/footer, block reader/writer
├─ jac-cli/            # CLI (compress/decompress, inspect, project)
├─ jac-wasm/           # WASM bindings (decoder + projection)
└─ jac-python/         # PyO3 bindings (optional)
**Core crates & major deps:**
- `serde` (for option JSON metadata), `simd-json` or `serde_json` (parsing),
- `zstd` (codec), `crc32c`, `bitvec`, `ahash`/`hashbrown`,
- `rayon` (parallelism), `bytes` (buffering), `smallvec` (avoid heap).

### 9.2. Public APIs

**High‑level (IO):**
```rust
// Compress NDJSON or a JSON array of objects
fn compress<R: Read, W: Write>(input: R, output: W, opts: CompressOpts) -> Result<()>;

// Decompress full JSON (as NDJSON or JSON array)
fn decompress_full<R: Read, W: Write>(input: R, output: W, opts: DecompressOpts) -> Result<()>;

// Project specific fields to a stream (values aligned to records)
fn project<R: Read, W: Write>(input: R, output: W, fields: &[&str], as_ndjson: bool) -> Result<()>;
```

`DecompressFormat` includes an `Auto` variant; when selected the decoder consults the stored container hint (flags bits 3–4) to choose NDJSON vs JSON array output, defaulting to NDJSON when the hint is `Unknown`. Callers can still override the wrapper explicitly with `Ndjson` or `JsonArray` when format conversion is desired.

**Low‑level (block/field):**
```rust
struct JacReader<R> { /* ... */ }
impl<R: Read + Seek> JacReader<R> {
    fn blocks(&mut self) -> impl Iterator<Item = Result<BlockHandle>>;
    fn project_field(&mut self, block: &BlockHandle, field: &str) -> Result<FieldIterator>;
}

struct JacWriter<W> { /* ... */ }
impl<W: Write> JacWriter<W> {
    fn new(mut w: W, header: FileHeader, opts: CompressOpts) -> Result<Self>;
    fn write_record(&mut self, rec: &serde_json::Map<String, Value>) -> Result<()>;
    fn finish(self, with_index: bool) -> Result<()>;
}
```

**Options:**
```rust
struct CompressOpts {
    pub block_target_records: usize,    // e.g., 100_000
    pub default_codec: Codec,           // Zstd(level)
    pub canonicalize_keys: bool,
    pub canonicalize_numbers: bool,     // uses §4.5 normalization
    pub nested_mode: NestedMode,        // Opaque (v1), Experimental (v2)
    pub max_dict_entries: usize,        // per field per block
    pub limits: Limits,                 // safety caps
}
```

### 9.3. Encoder Algorithm (per block)
	1.	Parse records (streaming) into a RowBuffer until thresholds (block_target_records or bytes).
	2.	Discover fields and build per‑field Presence bitmap.
	3.	Type analysis per field; emit type tags per present value.
	4.	Build substreams:
	•	bool → bitpack,
	•	int → choose varint/delta,
	•	decimal → exact decimal (or f64 if provably exact),
	•	string → decide dictionary vs raw (by cardinality & entropy),
	•	object/array → minify subdocument, store in string substream.
	5.	Assemble field segment payload (presence/tags/dict/substreams).
	6.	Compress each field payload independently (zstd high level).
	7.	Emit block header, field directory, segments, and CRC32C.

Heuristics:
	•	Use dictionary if distinct <= min(4096, value_count/8).
	•	Use delta when increasing_ratio >= 0.95.
	•	Cap max_dict_entries to protect memory.

### 9.4. Decoder Algorithm
	•	Read file header; iterate blocks.
	•	For projection:
	•	Scan block directory for the field; if present:
	•	Seek to segment_offset, decompress segment,
	•	Read presence, tags, then only the needed substream(s),
	•	Materialize per‑record values (emit null, absent, or value).
	•	For full decode:
	•	Decompress all field segments; reconstruct each record by combining per‑field values in record order; omit Absent fields and include Nulls.

### 9.5. Concurrency & Memory
	•	Compression: per‑block parallelism via rayon.
	•	Decompression: per‑block parallelism; in projection, per‑field can also parallelize.
	•	Memory budgeting:
	•	Presence bitmap = ceil(record_count/8) bytes per field.
	•	Tags = ceil(3 * present_count / 8) bytes per field.
	•	Dictionaries bounded by max_dict_entries.

### 9.6. WASM & Browser
	•	Provide decoder + projection in jac-wasm using wasm-bindgen:
project_fields(jac_bytes: Uint8Array, fields: Array<String>) -> JsValue
	•	Avoid huge blocks client‑side (recommend ≤ 10k–50k records per block for browsers).

⸻

## 10. CLI (jac-cli)
```
jac pack   [--block-records N] [--zstd-level L] [--project k1,k2,...] -o out.jac   input.json|.ndjson
jac unpack [--ndjson]                                                     out.json  in.jac
jac ls     in.jac                # list blocks, fields, counts
jac cat    --field userId        # stream values for a field (aligned per record)
```

## 11. Interoperability
	•	File extension: .jac
	•	MIME type (suggested): application/vnd.jac+binary
	•	Stability: Backward‑compatible within major version. Unknown encoding_flags/type codes MUST trigger UnsupportedFeature.

⸻

## 12. Test Vectors (Minimal)

### 12.1. Sample Input (NDJSON)
```json
{"ts":1623000000,"level":"INFO","msg":"Started","user":"alice"}
{"ts":1623000005,"level":"INFO","msg":"Step1","user":"alice"}
{"ts":1623000010,"level":"WARN","msg":"Low disk","user":"bob"}
{"ts":1623000020,"user":"carol","error":"Disk failure"}
```

Expected characteristics (per block):
	•	Fields: ts (int, delta), level (string, dict), msg (string, likely raw), user (string, dict), error (string, mostly absent).
	•	Presence bitmaps: error has 1 present; others vary.
	•	Tags: mostly int/string; some null none; no decimals.

A conformance test MUST verify:
	•	Full round‑trip equality (semantic) vs original JSON parse tree.
	•	Projection user yields ["alice","alice","bob","carol"].

⸻

## 13. Benchmarks & Datasets (Plan)
	•	Datasets: NDJSON logs (app/server), telemetry (IoT), public event streams (e.g., GitHub events), nested JSON (config dumps), synthetic high‑cardinality.
	•	Comparators: minified JSON + gzip/zstd/brotli; MessagePack/CBOR/Smile; Parquet (for AoO) where applicable.
	•	Metrics: compression ratio, compress/decompress time, projection time for hot fields.
	•	Targets:
	•	≥ 20–40% size reduction vs minified JSON+zstd on highly repetitive logs.
	•	O(records) projection time reading only targeted field segments.

⸻

## 14. Security Considerations
	•	Enforce hard caps on: record_count per block, field_count, dictionary size, segment_uncompressed_len.
	•	Validate all ULEB128 lengths; guard against integer overflow.
	•	Limit decompressed sizes from codecs (zstd frame headers are advisory; enforce our stored uncompressed lengths).
	•	Timeouts/cancellation hooks for long blocks.
	•	Avoid unsafe except in well‑audited hot loops.

⸻

## 15. Open/Extensible Areas
	•	Nested columnarization: repetition/definition levels (Parquet‑like) in v2.
	•	Global string dictionaries across blocks for very repetitive corpora.
	•	Bloom filters / min‑max per column for block‑level predicate pushdown.
	•	Alternative codecs (Brotli L11, LZMA) under archival profiles.

⸻

## 16. Implementation Notes (Rust)

### 16.1. Encoding Details (normative helpers)
	•	ULEB128:
	•	Encode: while x >= 0x80 → write (x & 0x7F) | 0x80, x >>= 7; finally write (x & 0x7F).
	•	Decode: accumulate 7‑bit chunks; MUST cap at 10 bytes for u64.
	•	ZigZag (i64 ↔ u64):
	•	enc = (v << 1) ^ (v >> 63); dec = ((u >> 1) as i64) ^ -((u & 1) as i64).
	•	Bit‑packing:
	•	Presence: byte‑aligned; bit i → record i (LSB‑first).
	•	Tags: 3‑bit values packed LSB‑first into bytes; last byte padded zero.

### 16.2. Column Builders
```rust
enum TypeTag { Null, Bool, Int, Decimal, String, Object, Array }

struct ColumnBuilder {
    presence: BitVec,          // records
    tags:     BitVec3,         // present values
    bools:    BitVec,          // for tags == Bool
    ints:     Vec<i64>,        // for tags == Int
    decimals: Vec<Decimal>,    // (digits: Vec<u8>, exp10: i32, sign: bool)
    strings:  StringIntern,    // counts + maybe dictionary
    objects:  Vec<Bytes>,      // minified JSON
    arrays:   Vec<Bytes>,      // minified JSON
}
```

Dictionary decision: After counting distinct strings:
	•	If distinct <= min(max_dict_entries, present/8) ⇒ build dict and emit indices; else raw strings.

### 16.3. Parsing
	•	Prefer simd-json for NDJSON.
	•	For large top‑level arrays, stream elements to the encoder (do not materialize entire DOM).

### 16.4. Testing
	•	Round‑trip corpus with tricky numerics: big ints, decimals (0.1, 1e-20, 1e+300), Unicode strings, nested subdocs.
	•	Fuzz block boundaries (1, 2, … N records).
	•	Corrupt CRC, wrong dict index, truncated frames → expect proper errors.

⸻

## 17. Worked Example (Directory Snapshot)

For field "level" in the sample:
	•	presence_bytes = 1 (1110_0000b → record3 absent)
	•	tag stream (present values): [string, string, string] → bytes 0b10010010 (3× code 4 packed)
	•	dict entries: ["INFO","WARN"] (count = 2)
	•	indices: [0,0,1] (ULEB128)
	•	segment_uncompressed_len = sum of above
	•	compressed with zstd@19

⸻

## 18. Compliance Checklist (for implementation agents)
	•	File header read/write (magic, flags, defaults).
	•	Block writer: directory entries populated with correct sizes/offsets.
	•	Presence bitmap packing/unpacking.
	•	Type‑tag packing/unpacking (3‑bit).
	•	Bool/int/decimal/string/object/array substreams encode/decode.
	•	Dictionary build thresholds & index encoding.
	•	Zstd compression per field; verify uncompressed lengths.
	•	CRC32C for blocks; error on mismatch.
	•	Full round‑trip tests (semantic).
	•	Projection API: reads only requested field segments.

⸻

## 19. License & IPR
	•	Reference implementation SHOULD be Apache‑2.0 or MIT to maximize interoperability.

⸻

## 20. Change Log
	•	0.9 (Draft): Initial archival-focused, union-typed columns, per-field compression, exact decimals, optional index footer.
	•	0.9.1 (Draft): Allocated header flag bits 3–4 for container-format hints and defined decoder auto-selection behaviour.



# **JAC v1 — Addendum & Clarifications (Draft 0.9.1)**
This addendum clarifies open questions, fills specification gaps, and proposes explicit defaults/limits. Section numbers below refer to the original Draft 0.9 spec.

## **1) Minor Technical Clarifications**
## **1.1 §4.7 Segment Order — Rationale & Normative Text**
**Why this order?**
1. **Presence → Tags first**: These are tiny and frequently accessed by projectors to decide which substreams to read. Putting them first lets a decoder quickly skip entire substreams or materialize “absent/null” without touching large payloads.
2. **Dictionary before string payload**: A decoder needs the dictionary to interpret incoming indices immediately; this avoids a second pass.
3. **Booleans before ints/decimals**: Booleans are bit‑packed and very small; placing them early improves cache locality for common predicates/filters.
4. **Numeric substreams before strings**: Numeric scans/predicates are common in analytics; placing numbers earlier may reduce I/O for numeric‑only projections.
5. **Strings and subdocuments last**: These are typically largest and least needed for structural queries.
**Normative (drop‑in)**
**§4.7 (amendment)** — *Ordering rationale*. The segment payload **MUST** appear as:
1. Presence bitmap, 2) Type‑tag stream, 3) String dictionary (if any), 4) Boolean substream, 5) Integer substream, 6) Decimal substream, 7) String substream(s).Encoders **MUST** follow this order. Decoders **MUST** NOT assume any further alignment. This order prioritizes fast projection/skipping by allowing early inspection of tiny structures (presence/tags) and dictionaries before dependent streams.

## **1.2 §4.5 Decimal Encoding — Wire Format Precision**
**Clarifications**
* **Sign**: 0 = non‑negative, 1 = negative. Zero **MUST** use sign 0.
* **Digits storage**: Digits are the canonical base‑10 representation of the **absolute value** with **no leading zeros** (except zero itself). They are stored as **ASCII bytes '0'..'9'**, most‑significant digit first (i.e., normal human order). **Not** a binary big‑endian integer.
* **Exponent**: Decimal exponent base‑10, signed, encoded with ZigZag+ULEB128.
**Normative (drop‑in)**
**§4.5 (amendment) — Decimal wire format**
```

sign_byte            : u8      // 0 = non‑negative, 1 = negative (zero MUST use 0)
digits_len           : ULEB128 // count of ASCII digits
digits_ascii         : bytes   // '0'..'9', MSB-first; no leading zeros unless digits_len=1 and digit='0'
exp10_zigzag_uleb128 : ULEB128 // ZigZag-encoded signed base-10 exponent (range: i32)

```
Decoders **MUST** reject sign_byte values other than 0 or 1 and **MUST** reject leading zeros unless the value is zero.

## **2) Specification Gaps (filled)**
## **2.1 Block Size & Segment Limits (recommended & hard caps)**
To bound memory and enable predictable resource usage, add the following to **§8 Security** and **§9.2 Options / Limits**.
**Normative (drop‑in)**
**Limits (default / hard maximum)**
* max_records_per_block: **100_000** (default) / **1_000_000** (hard reject).
* max_fields_per_block: **4_096** (default) / **65_535** (hard reject).
* max_segment_uncompressed_len: **64 MiB** (hard reject).
* max_block_uncompressed_total: **256 MiB** (hard reject; sum of all segments in a block).
* max_dict_entries_per_field: **4_096** (default) / **65_535** (hard reject).
* max_string_len_per_value: **16 MiB** (hard reject).
* max_decimal_digits_per_value: **65_536** (hard reject).
* max_presence_bytes_per_field: **32 MiB** (hard reject).
* max_tag_stream_bytes_per_field: **32 MiB** (hard reject).
Encoders **SHOULD** target 50k–150k records per block for logs/NDJSON. Decoders **MUST** enforce hard maxima.
**Notes**
* Presence bytes = ceil(record_count/8).
* Tag bytes = ceil(3 * present_count / 8) (see §2.3 below).

## **2.2 Dictionary Size Limits (explicit)**
**Normative (drop‑in)**
**§4.6 (amendment) — Dictionary bounds**
* Dictionary indices are **0‑based**.
* dict_entry_count **MUST NOT** exceed max_dict_entries_per_field.
* Encoders **SHOULD** choose dictionary mode when distinct <= min(max_dict_entries_per_field, present_count / 8).
* Each dictionary entry length **MUST NOT** exceed max_string_len_per_value.

## **2.3 Type‑Tag Packing (padding)**
**Clarifications**
* 3‑bit codes are packed **LSB‑first** and contiguous for **present** values only.
* If present_count is not a multiple of the packing quantum, the final byte contains unused high bits.
**Normative (drop‑in)**
**§4.2 (amendment) — Tag packing**
* Let present_count be the number of present values.
* tag_bytes = ((3 * present_count) + 7) >> 3.
* The unused high bits in the **final tag byte** (bits from 3*present_count up to 8*tag_bytes - 1) **MUST** be zero; decoders **MUST** ignore them.

## **3) Potential Enhancements (now specified)**
## **3.1 Versioning & Unknown Features**
**Normative (drop‑in)**
**§11 Interoperability & Versioning (extended)**
* **Major version** increments on any wire‑incompatible change (e.g., new type‑tag semantics, changes to block header fields that older decoders cannot skip, or redefinition of reserved codes).
* **Minor version** increments for strictly backward‑compatible extensions (e.g., new optional directory fields within header_len bounds, new optional footer sections, new compressor IDs).
* **Unknown type tags**: If a decoder encounters a type‑tag value not defined in this version (including 7), it **MUST** fail with UnsupportedFeature. The 7 code remains **reserved** in v1 and **MUST NOT** be emitted.
* **Unknown compressor IDs**: If a field segment uses a compressor ID the decoder does not support, decoding **MUST** fail with UnsupportedCompression.
* **Forward‑skipability**: Block headers and directory entries are length‑delimited via header_len. Decoders **MUST** skip unrecognized trailing fields within the header/directory based on header_len. New directory fields **MUST** be appended after existing ones to preserve this property.
* **Required features bitset (reserved)**: File Header bitfield required_features: u64 is reserved in v1 and **MUST** be zero. Future encoders **MUST** set a bit to indicate a required feature; decoders **MUST** fail if any required bit is unknown.
*(If you prefer not to add required_features now, keep the rule about header_len‑based skipping and unknown tag/codec failure; add the bitset in v1.1.)*

## **3.2 Streaming (writer/reader) — explicit behavior**
**Normative (drop‑in)**
**§5 Streamability (new subsection)**
* **Writer behavior**: Writers **MUST** only emit **complete blocks**. A block is complete if its header, all declared segments, and block CRC32C are written. Writers **MAY** flush a partial final block with fewer than the target records; they **MUST NOT** emit an incomplete block.
* **Reader behavior on partial data**: If EOF occurs before a block is complete, the reader **MUST** return UnexpectedEof and **MAY** expose all prior complete blocks.
* **Buffer management**: Implementations **SHOULD** parse input JSON in streaming fashion and **MAY** spill large values (strings/subdocuments) to a temporary buffer or mmap’d scratch file during block construction to respect memory limits; max_* limits still apply.
* **Error recovery (resync)**: On block checksum failure or corruption, decoders **MAY** attempt resynchronization by scanning for the next block_magic ("BLK1"), but **SHOULD** expose a strict mode that aborts on first corruption due to false positive risk.
* **Projection in streaming**: For projections, readers **MAY** start decoding a requested field as soon as (a) the block header is available and (b) the field’s compressed segment bytes have arrived; other segments need not be buffered.

## **4) Direct Answers to Technical Questions**
## **4.1 Decimal Precision (maximum supported)**
* The format is **arbitrary precision** by design (digits as ASCII).
* To prevent resource abuse, decoders **MUST** enforce a per‑value cap: max_decimal_digits_per_value (default **65_536**, hard reject). Implementations **MAY** lower this via configuration.
* Exponent range is **i32** (encoded via ZigZag+ULEB128); values outside this must fail.
## **4.2 Dictionary Indexing (base)**
* **0‑based** indices into the per‑segment dictionary.
* *Note*: Because presence is tracked separately (§4.1), **absent is never a dictionary entry** (the worked example has been updated accordingly below).
## **4.3 Type‑Tag Packing (padding)**
* See §2.3 above: last byte’s unused bits **MUST** be zero; decoders **MUST** ignore them.
## **4.4 Block / Segment Alignment**
* **No alignment is required.** Segments start at the exact segment_offset specified by the directory.
* Encoders **SHOULD** pack segments contiguously (no padding).
* Decoders **MUST** tolerate any byte offset per the directory.
* Any padding (if an encoder chooses to insert it) **MUST** be accounted for by segment_offset and is covered by the block CRC like any other byte.

## **5) Small Spec Patches (ready to paste)**
## **5.1 Presence & Tags — exact size formulas**
Add to §4.1 and §4.2:
presence_bytes = (record_count + 7) >> 3
tag_bytes = ((3 * present_count) + 7) >> 3
## **5.2 Worked Example (corrected to use presence, not “absent in dict”)**
Given 4 records:
```

{"ts":1623000000,"level":"INFO","msg":"Started","user":"alice"}
{"ts":1623000005,"level":"INFO","msg":"Step1","user":"alice"}
{"ts":1623000010,"level":"WARN","msg":"Low disk","user":"bob"}
{"ts":1623000020,"user":"carol","error":"Disk failure"}

```
* level presence bits (record order): **1 1 1 0** → presence byte 0b00001110
* level tags for present positions: all string (code **4**) → packed into tag_bytes = 2 (three 3‑bit codes = 9 bits; final unused bit = 0)
* level dictionary: ["INFO","WARN"] (count=2)
* level indices (0‑based for present positions): [0,0,1]
*(No “absent” entry in the dictionary; absence is encoded by the presence bitmap.)*

## **6) Quick “What to Change in Code” Checklist**
* Enforce **default/hard** limits from §2.1 in the decoder; expose overrides in Limits.
* Make **dictionary indices 0‑based**; ensure absence is **not** a dict entry.
* Implement **tag packing** with zeroed padding bits; verify with tests for edge counts.
* Implement **decimal wire** with sign_byte, ASCII digits (no leading zeros), and exp10 ZigZag+ULEB128; enforce digit count limit and exponent range.
* Ensure **segment order** is exactly as in §1.1; update readers to rely on directory offsets only (no implicit alignment).
* Add **strict mode** vs **resync mode** for streaming error recovery.
* Update worked example tests to use presence, not dictionary “absent”.

## **7) Phase 5 Performance Validation & Telemetry**
**Informative (implementation guidance)**

* Parallel compression **MUST** preserve byte-for-byte determinism across thread counts (validated via automated integration tests comparing sequential vs parallel output and round-tripping through the decoder).
* `CompressSummary` now exposes `runtime_stats`, capturing wall-clock duration and peak RSS usage. Implementations **SHOULD** sample RSS at ≤50 ms intervals while compression is active and report the observed peak alongside heuristic estimates in operator-facing tooling.
* Benchmarks in `jac-io/benches/compression.rs` include a `parallel_speedup_*` group covering thread counts {1,2,4,8} for both default Zstd and single-threaded Zstd variants. These benches **SHOULD** be run as part of Phase 5 validation to confirm the expected 6–7× speedup on 8-core hosts and to quantify the benefit of constraining Zstd’s internal threading.
* Operators **SHOULD** track the measured peak RSS against the heuristic estimate (`memory_reservation_factor × available_memory`). If the observed peak regularly exceeds the estimate, lower the factor (e.g., 0.75 → 0.65) and document the adjustment in deployment playbooks.
