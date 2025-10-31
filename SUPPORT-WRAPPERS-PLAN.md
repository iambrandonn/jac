# JSON Wrapper Support Plan (2025 Refresh)

## Motivation & Current Behavior
- The streaming readers already cover three of the six JSON shapes we need:
  - **Single object**: `JsonArrayStream` treats a top-level `{ ... }` as a single record.
  - **Array of objects**: `JsonArrayStream` iterates `[ { ... }, … ]`.
  - **NDJSON**: `NdjsonStream` consumes newline-delimited objects (with BOM handling).
- Wrapper heuristics were removed after the nested-object corruption bug. Since then, any “enveloped” or otherwise structured JSON must be pre-flattened before calling JAC.
- `CompressRequest` (and therefore the CLI) has no hook for wrapper configuration; `InputSource::into_record_stream()` only yields NDJSON, JSON array, or a caller-supplied iterator.
- We now need first-class support for three additional patterns:
  1. **Wrapped/enveloped object** with a data array (e.g., `{ "data": [ ... ] }`).
  2. **Multi-section object** where several named arrays should be concatenated (e.g., `{ "users": [ ... ], "admins": [ ... ] }`).
  3. **Keyed map object** mapping IDs to record objects (e.g., `{ "alice": { ... }, "bob": { ... } }`).

## Architectural Baseline (Feb 2025)
- `jac-io::InputSource::into_record_stream()` constructs either `NdjsonStream`, `JsonArrayStream`, or wraps a caller iterator; the resulting `RecordStream` is consumed before parallel block building kicks in (`jac-io/src/lib.rs`).
- `CompressRequest` has no wrapper metadata, but `CompressSummary` now includes an optional `ParallelDecision` diagnostic; wrapper work must keep that reporting intact.
- CLI input detection flows through `resolve_input_source()` and container hints (`ContainerFormat`) before invoking `execute_compress()` (`jac-cli/src/main.rs`). Wrapper configuration must integrate cleanly with these hints and continue to support shortcut invocation (`jac <file>`).
- Parallel compression heuristics rely on data available after `RecordStream` creation, so wrapper buffering still must complete before we decide on worker counts.
- Zstd segment limits, flush accounting, and verbose metrics (Phase 8 additions) depend on accurate record counts; wrapper layers must not distort these metrics.

## Cross-Cutting Requirements
1. **Streaming-first**: Only buffer what is necessary to reach the target data; once we enter the record sequence we must stream iteratively.
2. **Memory limits**: Enforce wrapper-specific ceilings alongside `Limits::max_segment_*`: default buffer 16 MiB, hard maximum 128 MiB (CLI refuses larger). Pointers longer than 2048 characters are rejected; default maximum pointer length is 256.
3. **Explicit opt-in**: No implicit heuristics. Users (CLI) and library callers must request wrapper handling through new flags or API fields.
4. **Error clarity**: Errors should name the pointer/config path, explain why the wrapper failed, and suggest remediation (e.g., `jq` commands or buffer increases).
5. **Metrics & logging**: Preserve `ParallelDecision` in summaries and add wrapper diagnostics (mode, pointer depth, peak buffer, processing duration) for debugging.
6. **Container hints**: After unwrapping, record the effective container format (`ContainerFormat::JsonArray`) in the file header so unpack auto-detection stays accurate.
7. **Transform transparency**: Wrapper modes are preprocessing transformations. Plan, CLI, and docs must warn that `jac unpack` yields flattened records; the original envelope structure is not recoverable unless archived separately.
8. **Backwards compatibility**: Existing NDJSON/array/single-object flows continue to behave exactly as today unless a wrapper is requested.

## Reference Data Structures

```rust
/// Wrapper-specific limits enforced during input preprocessing.
#[derive(Debug, Clone)]
pub struct WrapperLimits {
    /// Maximum JSON pointer depth (default: 3, hard max: 10).
    pub max_depth: usize,
    /// Maximum bytes buffered before reaching target (default: 16 MiB, hard max: 128 MiB).
    pub max_buffer_bytes: usize,
    /// Maximum pointer string length (default: 256, hard max: 2048).
    pub max_pointer_length: usize,
}

impl Default for WrapperLimits {
    fn default() -> Self {
        Self {
            max_depth: 3,
            max_buffer_bytes: 16 * 1024 * 1024,
            max_pointer_length: 256,
        }
    }
}

impl WrapperLimits {
    pub fn hard_maximums() -> Self {
        Self {
            max_depth: 10,
            max_buffer_bytes: 128 * 1024 * 1024,
            max_pointer_length: 2048,
        }
    }

    // WrapperError is the new error enum introduced in Phase 1 to capture wrapper-specific failures.
    pub fn validate(&self) -> Result<(), WrapperError> {
        let hard = Self::hard_maximums();
        if self.max_depth > hard.max_depth {
            return Err(WrapperError::ConfigurationExceedsHardLimits {
                reason: format!("max_depth {} exceeds {}", self.max_depth, hard.max_depth),
            });
        }
        if self.max_buffer_bytes > hard.max_buffer_bytes {
            return Err(WrapperError::ConfigurationExceedsHardLimits {
                reason: format!(
                    "max_buffer_bytes {} exceeds {}",
                    self.max_buffer_bytes, hard.max_buffer_bytes
                ),
            });
        }
        if self.max_pointer_length > hard.max_pointer_length {
            return Err(WrapperError::ConfigurationExceedsHardLimits {
                reason: format!(
                    "max_pointer_length {} exceeds {}",
                    self.max_pointer_length, hard.max_pointer_length
                ),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum WrapperConfig {
    None,
    Pointer { path: String, limits: WrapperLimits },
    Sections {
        entries: Vec<SectionSpec>,
        limits: WrapperLimits,
        label_field: Option<String>,
        inject_label: bool,
        missing_behavior: MissingSectionBehavior,
    },
    KeyedMap {
        pointer: String,
        key_field: String,
        limits: WrapperLimits,
        collision_mode: KeyCollisionMode,
    },
}

#[derive(Debug, Clone)]
pub struct SectionSpec {
    pub name: String,
    pub pointer: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum MissingSectionBehavior {
    Skip,
    Error,
}

#[derive(Debug, Clone, Copy)]
pub enum KeyCollisionMode {
    Error,
    Overwrite,
}

#[derive(Debug, Clone)]
pub struct WrapperMetrics {
    pub mode: String,
    pub buffer_peak_bytes: usize,
    pub records_emitted: usize,
    pub processing_duration: std::time::Duration,
    pub pointer_path: Option<String>,
    pub section_counts: Option<Vec<(String, usize)>>,
    pub map_entry_count: Option<usize>,
}
```

> WrapperLimits apply to input preprocessing; they complement (but do not replace) `jac_format::Limits`, which govern encoded output. Wrapper code should still validate keys/values against `Limits::max_string_len_per_value` to fail fast on oversized data.

## Target Input Support Matrix

| Format                                   | Current Status | Desired Outcome                     |
|-----------------------------------------|----------------|-------------------------------------|
| Single object                            | ✅ Works       | Keep current behaviour              |
| Array of objects                         | ✅ Works       | Keep current behaviour              |
| NDJSON                                   | ✅ Works       | Keep current behaviour              |
| Wrapped/enveloped object (`data`)        | ❌ Missing     | `--wrapper-pointer /data`           |
| Multi-section object (named groups)      | ❌ Missing     | `--wrapper-sections users admins`   |
| Keyed map object (`{id: {...}}`)         | ❌ Missing     | `--wrapper-map key=<field>`         |

## Phased Delivery Plan

### Phase 1 – Wrapper Infrastructure & JSON Pointer Envelope
**Goal:** Provide the foundational plumbing and support JSON Pointer wrappers (default depth ≤ 3, hard max 10) so enveloped `{"data":[...]}` inputs work without preprocessing.

**Key Tasks**
1. Introduce `WrapperConfig` and `WrapperLimits` in `jac-io/src/lib.rs` (defaults: depth 3, buffer 16 MiB, pointer length 256; hard caps: depth 10, buffer 128 MiB, pointer length 2048). Add the configuration to `CompressRequest` and propagate through `execute_compress()`. Reject configs that exceed hard caps.
2. Extend `RecordStreamInner` with a `Wrapper(Box<dyn RecordIterator>)` variant so InputSource can substitute wrapped streams without breaking existing code; ensure `container_format()` reflects the effective format.
3. Implement `pointer` module (new file) housing `PointerArrayStream`:
   - RFC 6901 parsing with `~0/~1` unescaping and pointer length validation.
   - Depth enforcement driven by `WrapperLimits`.
   - Bounded buffering of sibling data using serde’s streaming de-serializer; enforce default and hard limits, emitting descriptive errors with remediation tips.
   - Allow array targets (stream elements) and object targets (emit single record); reject scalar/null targets.
   - Delegate to `JsonArrayStream` once the target payload is found.
4. CLI integration:
   - Flags `--wrapper-pointer`, `--wrapper-pointer-depth`, `--wrapper-pointer-buffer`; depth flag allows up to 10, buffer flag up to 128 MiB. Library callers can override via `WrapperLimits`.
   - Size parsing via existing helpers (or a new shared parser) with validation against hard caps.
   - Enforce mutual exclusivity: exactly one wrapper mode per invocation. Mode-specific flags require their base flag (e.g., `--wrapper-pointer-depth` requires `--wrapper-pointer`). Extend CLI help text with clear conflicts and requires metadata.
   - Ensure shortcut invocation (`jac <file>`) remains unchanged unless a wrapper flag is provided.
5. Diagnostics:
   - Add `CompressSummary::wrapper_metrics: Option<WrapperMetrics>` capturing mode, pointer path, peak buffer bytes, emitted record count, and processing duration.
   - Emit wrapper information in verbose CLI summaries (e.g., “Wrapper buffered 12.3 MiB before reaching /data”).
   - Record effective container hint (`ContainerFormat::JsonArray`) when wrapper mode is active.
6. Tests:
   - Unit coverage for pointer parsing, depth checks (including depth 0 and boundary cases), pointer length checks (≤256 default, >256 rejection), buffer overflow, wrong target type, and RFC 6901 escaping/malformed escapes.
   - Security tests for exceeding hard caps (depth, buffer, pointer length) and malicious pointer patterns.
   - Integration tests (`jac-cli/tests/wrapper_pointer.rs`) covering success, empty arrays, missing path, missing intermediate segments, null targets, scalar targets, escaped characters, unicode keys, buffer boundary equality, buffer overruns with remediation suggestions, malformed escape sequences, container hint propagation, CLI flag validation (requires/conflicts), and verbose metric output.
   - Regression test ensuring NDJSON/array inputs are untouched when wrapper config is `None`.
   - Utilize fixtures outlined in Appendix C.
7. Documentation:
   - Update README, CLI help text, and AGENTS.md with examples, safety notes, security limits, container hint behaviour, and round-trip warnings (“Wrapper modes emit flattened records; original envelopes are not reconstructed by `jac unpack`.”).
   - Add troubleshooting guidance mirroring new error messages plus guidance on when to preprocess externally (e.g., envelope >50 MiB before target data).
   - Document CLI examples explicitly (basic pointer, nested pointer, custom buffer, decompression, jq alternative) and highlight wrapper vs preprocessing decision table.
   - Reference Appendices A–D for error taxonomy, CLI flag matrix, fixture guidance, and documentation templates to ensure consistent implementation artifacts.

**Performance Characteristics**
- Wrapper traversal runs before block building; large envelopes increase serial wall time and memory usage, but once inside the target array streaming reverts to O(1) per record.
- Best case: target array appears early (minimal buffer, wrapper time ≪ total compression time).
- Worst case: target array near EOF; wrapper buffers nearly entire file (up to configured limit). Documented guidance: if the envelope routinely exceeds ~50 MiB before target data, prefer preprocessing with `jq`/`mlr`.
- CLI verbose mode should report buffer usage and traversal duration so users can gauge overhead.
- Benchmarks: add micro-benchmarks comparing baseline NDJSON vs wrappers (early/medium/late target envelopes) and document observed overhead in README. Include memory measurement integration test to confirm peak ≈ buffer limit + compression budget.
- WrapperError: define explicit error variants (buffer limit exceeded, depth exceeded, pointer too long, pointer not found, wrong type, invalid pointer syntax) with remediation suggestions, and integrate with existing `JacError`.

### Phase 2 – Multi-Section Aggregation
**Goal:** Stream multiple named sections out of a single top-level object (e.g., `users`, `admins`) as one logical record sequence while preserving section provenance when needed.

**Key Tasks**
1. Extend `WrapperConfig` with `Sections` variant:
   ```rust
   WrapperConfig::Sections {
       entries: Vec<SectionSpec>, // SectionSpec { pointer: String, label: Option<String> }
       max_depth: usize,
       buffer_limit_bytes: usize,
       label_field: Option<String>,
       inject_label: bool, // default true
       missing_behavior: MissingSectionBehavior, // default Skip
    }
    ```
2. Implement `SectionsStream` that:
   - Parses the top-level object into a bounded `serde_json::Value` (honouring wrapper buffer limits) to keep implementation simple in Phase 2; document that extremely large envelopes should be preprocessed.
   - Re-uses pointer traversal utilities from Phase 1 to extract each section value.
   - Streams array sections element-by-element; optionally treat objects as single-element arrays when explicitly requested.
   - Annotates each emitted record with an optional section label (e.g., `_section` field) when configured and inject_label is true.
3. CLI flags:
   - `--wrapper-sections users admins` (simple case) mapping to default pointer `/users` etc.
   - `--wrapper-section-pointer users=/payload/users` for custom pointers.
   - `--wrapper-section-label-field _section` to name the injected field (default `_section`), and `--wrapper-section-no-label` to disable label injection.
   - `--wrapper-sections-missing-error` to opt into failing when sections are absent (default skip).
4. Metrics & limits:
   - Track per-section buffer usage and record counts; expose in verbose mode and wrapper metrics.
   - Validate combined record count against block/segment limits and ensure top-level object size respects buffer hard cap.
5. Tests:
   - Section ordering, missing sections, mixed empty/non-empty arrays.
   - Label injection on/off plus conflict handling when injected field already exists (error by default, optional overwrite flag).
   - Interaction with parallel compression (ensuring section expansion completes before worker allocation).
   - Error cases: missing section in error mode, scalar sections, label collisions with/without injection, unicode section names.
6. Docs:
   - Examples for multi-role exports (e.g., admin/user lists) with on-disk CLI usage and recommendations for label fields.
   - Highlight that multi-section mode buffers the entire top-level object; recommend preprocessing when structure exceeds practical limits.
   - Document missing-section behaviour options and label collision guidance (see Appendices B–D for flag reference and documentation templates).

### Phase 3 – Keyed Map Object Support
**Goal:** Flatten `{ "id": { ... } }` style JSON into records while retaining the key (optionally under a configurable column name).

**Key Tasks**
1. Add `WrapperConfig::KeyedMap { pointer, key_field, buffer_limit_bytes }`.
   - Default pointer `""` (top-level object) for the simplest case.
   - Permit nesting via pointer + object at target.
   - Include `KeyCollisionMode` (default `Error`, optional `Overwrite`) and reuse `WrapperLimits`.
2. Implement `KeyedMapStream`:
   - Navigates to the target object (using pointer utilities).
   - Iterates key/value pairs lazily, inserting `key_field` into each emitted record (default `_key`).
   - Validates each value is an object (error on null/scalar/array) and each key length ≤ `Limits::max_string_len_per_value`.
3. CLI flags:
   - `--wrapper-map` to enable.
   - `--wrapper-map-pointer /data/byId` (optional).
   - `--wrapper-map-key-field id` (default `_key`) and `--wrapper-map-overwrite-key` to enable overwrite mode.
4. Error handling:
   - Reject non-object targets, non-object values, or duplicate key collisions when user-provided field conflicts with existing data (unless overwrite mode is active).
   - Emit explicit errors for overlong keys, invalid pointer targets, or missing map entries.
5. Tests:
   - Basic map flattening, nested pointer map, conflict handling (error vs overwrite), large key counts (ensuring streaming), empty map handling.
6. Docs:
   - Explain how to convert dictionary-style API responses without preprocessing, include jq fallback guidance.
   - Clarify map entry ordering expectations (serde_json iteration order up to block building; final compression may reorder keys lexicographically as per spec) and advise on preprocessing for deterministic ordering if required.
   - Note that wrappers remain a Rust/CLI feature only; bindings (Python/WASM) will not expose wrappers until explicitly scheduled (Phase 5+ if demand exists).
   - Reference Appendices B–D for CLI flag behaviours, fixture guidance, and documentation patterns.

### Phase 4 – Polish, Metrics, and Extensibility
**Goal:** Round out wrapper support with observability, documentation, and optional developer extensions.

**Key Tasks**
1. Metrics:
   - Promote `WrapperMetrics` to capture mode, peak buffer, emitted records, section counts, map entry counts, and processing duration.
   - Surface metrics via CLI `--verbose-metrics` and expose via API.
2. Observability:
   - Add debug logging hooks gated by verbosity (e.g., `JAC_DEBUG_WRAPPER=1`).
3. Configuration:
   - Allow default wrapper settings via config file (`~/.jac/config.toml`) or environment variables.
4. Auto-detect (optional, opt-in):
   - `--wrapper-auto-detect` to run lightweight heuristics that suggest wrapper flags but still require confirmation (ensure it never silently changes behaviour).
5. Documentation:
   - Comprehensive “Wrapper cookbook” in README/SPEC addendum, including when to preprocess vs. use wrappers and explicit round-trip caveats.
   - Update SPEC references if new behaviour needs normative language.
   - Include wrapper CLI help snippets, FAQ entries (“Can I recover envelopes?” → no), and performance tables derived from benchmarks.

### Phase 5+ (Deferred Ideas)
- Plugin registry for custom wrapper implementations (e.g., dynamic loading in CLI).
- Support for array-of-arrays with header rows (CSV-like) if customer demand resurfaces.
- Hooks for schema-aware wrappers once Phase 9 benchmarking is complete.

## Implementation Checklist (Phase 1 Emphasis)
1. **Design Review**
   - Walk through updated plan with maintainers; ensure depth/buffer defaults align with limit policies.
2. **Scaffolding**
   - Implement `WrapperConfig` and `WrapperLimits`, extend `CompressRequest`, wire into CLI argument parsing (reject out-of-range buffers/depths/pointers).
   - Establish `jac-io/src/wrapper/` module structure (`mod.rs`, `error.rs`, `pointer.rs`, future `sections.rs`, `map.rs`, and shared `utils.rs`) to keep wrapper logic isolated.
3. **Pointer Stream**
   - Write iterator + unit and security tests; ensure serde streaming usage fits current borrow rules and enforces hard caps.
4. **RecordStream Integration**
   - Add new enum variant, update iterator impl, adapt tests (`ndjson_input_streams_records`, etc.) to confirm nothing regresses.
5. **CLI & Summary**
   - Parse size strings, enforce flag exclusivity, validate limits, render wrapper metrics (mode, buffer, duration) in verbose mode, and ensure container hints reflect wrapper usage.
6. **Documentation**
   - README, CLI help, AGENTS.md summarising new support, security limits, round-trip caveats, and preprocessing guidance. Add FAQ entry explaining wrapper transformations.
   - Expand README with wrapper examples, performance guidance table, and preprocess vs wrapper decision chart; update AGENTS with wrapper module map, limit relationships, and testing checklist.
7. **Validation**
   - `cargo fmt`, `cargo clippy`, targeted `cargo test -p jac-io` and CLI integration tests.
   - Manual smoke tests with real JSON fixtures (GitHub API, GraphQL responses).
   - Add micro-benchmarks measuring wrapper overhead (early/medium/late target) and document results.

## Risks & Mitigations
- **Memory blow-ups**: large buffer limits set by users. → Ship warnings when limits exceed 64 MiB, enforce 128 MiB hard cap, document DoS trade-offs.
- **Complex CLI UX**: many wrapper flags. → Enforce mutual exclusivity and add `jac pack --help` groups; consider `jac pack --wrapper help`.
- **Metric skew**: wrapper injections (e.g., section label field) might change schema drift heuristics. → Keep label field optional and clearly named; adjust tests accordingly and warn users in docs.
- **Parallel interactions**: ensure wrapper traversal completes before `ParallelDecision`; add asserts/tests to catch regressions.
- **Maintenance overhead**: multiple stream types. → Share pointer parsing utilities across variants; keep trait-based abstraction simple.
- **Semantics misunderstandings**: Users might expect envelopes to survive round-trip. → Prominent warnings in CLI/docs, recommend archiving original files when the wrapper structure matters.
- **Binding scope confusion**: Users may expect wrappers in Python/WASM. → Document that wrappers are CLI/Rust-only for now; evaluate bindings in Phase 5 if demand exists.

## Next Actions
1. Circulate this refreshed plan (including security/documentation updates) for stakeholder approval.
2. Begin Phase 1 implementation (wrapper scaffolding + pointer stream) starting with hard-limit enforcement and CLI validation.
3. Prepare fixture data covering each target format to speed up Phase 2/3 work.
4. Draft README/AGENTS updates in parallel so warning language lands with the feature.

## Appendices

### Appendix A – WrapperError Reference

Implementation should introduce `WrapperError` (likely in `jac-io/src/wrapper/error.rs`) to encapsulate wrapper-specific failures and present actionable diagnostics. Recommended variants include:

- `BufferLimitExceeded { limit_bytes, buffered_bytes, pointer, suggested_size, jq_expr }`
- `DepthLimitExceeded { depth, max_depth, suggested_depth }`
- `PointerTooLong { pointer, length, max_length }`
- `PointerNotFound { pointer, reached_path, available_keys }`
- `PointerTargetWrongType { pointer, expected_type, found_type }`
- `InvalidPointer { pointer, reason }`
- `SectionNotFound { section, pointer, available_keys }`
- `SectionLabelCollision { field, section }`
- `MapKeyTooLong { key, length, max_length }`
- `KeyFieldCollision { field, map_key }`
- `MapValueNotObject { key, found_type }`
- `ConfigurationExceedsHardLimits { reason, max_depth, max_buffer, max_ptr_len }`
- `JsonParse { context, source }`
- `Io { source }`

Helper methods (e.g., `suggest_buffer_size`, `pointer_to_jq`) can live on the error impl to keep remediation hints consistent.

### Appendix B – CLI Flag Reference

**Pointer Mode (Phase 1)**
- `--wrapper-pointer <POINTER>` (RFC 6901 path). Conflicts: sections/map.
- `--wrapper-pointer-depth <N>` (default 3, max 10). Requires pointer.
- `--wrapper-pointer-buffer <SIZE>` (default 16M, max 128M). Requires pointer.

**Sections Mode (Phase 2)**
- `--wrapper-sections <NAME> [NAME…]`. Conflicts: pointer/map.
- `--wrapper-section-pointer name=pointer`. Repeats allowed; requires sections.
- `--wrapper-section-label-field <FIELD>` (default `_section`). Requires sections.
- `--wrapper-section-no-label`. Requires sections; conflicts with label field flag.
- `--wrapper-sections-missing-error` (default skip). Requires sections.

**Map Mode (Phase 3)**
- `--wrapper-map`. Conflicts: pointer/sections.
- `--wrapper-map-pointer <POINTER>` (default root). Requires map.
- `--wrapper-map-key-field <FIELD>` (default `_key`). Requires map.
- `--wrapper-map-overwrite-key`. Requires map.

**Phase 4+**
- `--wrapper-auto-detect` (suggest-only). Opt-in; never implicit.
- `JAC_DEBUG_WRAPPER=1` environment variable for verbose logging.

Ensure clap `conflicts_with`/`requires` attributes encode these relationships.

### Appendix C – Suggested Test Fixtures

Create `jac-cli/tests/fixtures/wrapper/` containing representative JSON inputs:
- **Pointer**: basic envelope, nested envelopes, early/late targets, empty arrays, object targets, null/scalar targets, missing pointers, escaped keys, unicode, depth-limit case, buffer boundary/exceeded, malicious pointer strings.
- **Sections**: basic multi-section, missing section, empty arrays, mixed types, label collisions, custom pointers.
- **Map**: basic map, empty map, single entry, key collisions, null/array values, nested pointer maps, unicode keys, long keys.
- Include corresponding expected NDJSON outputs in `fixtures/wrapper/expected/`.
- Incorporate real-world samples (e.g., GitHub API, GraphQL responses) for smoke tests.

### Appendix D – Documentation Templates

Leverage the following when updating README/CLI help:
- Wrapper overview with warning banner about irreversible transformations.
- Pointer/Sections/Map usage examples (commands + sample input/output).
- Wrapper vs preprocessing decision table.
- Performance characteristics table (envelope size vs recommended action) populated with measured benchmark results.
- FAQ entries addressing envelope recovery, pointer failures, bindings scope, and streaming behaviour.
- CLI help snippets showing flag usage and remediation tips.
