#![deny(unsafe_code)]
#![warn(missing_docs)]

//! JAC I/O - Streaming file I/O and high-level APIs
//!
//! This crate provides the file I/O layer and high-level APIs for JAC:
//!
//! - Streaming writers and readers
//! - High-level compression/decompression functions
//! - Parallel processing support
//! - Field projection APIs

pub mod parallel;
pub mod reader;
pub(crate) mod runtime;
pub mod wrapper;
pub mod writer;

// Re-export commonly used types
pub use jac_codec::{Codec, CompressOpts, DecompressOpts};
pub use jac_format::{ContainerFormat, FileHeader, JacError, Limits, Result, TypeTag};
use reader::BlockCursor;
pub use reader::{
    BlockHandle, FieldIterator, JacReader, ProjectionStream, RecordStream as ReaderRecordStream,
};
pub use wrapper::{
    ArrayHeadersStream, FieldHint, FieldType, KeyedMapStream, PointerArrayStream, SchemaHints,
    SectionsStream, WrapperError, WrapperPlugin, WrapperPluginMetadata, WrapperPluginRegistry,
};
pub use writer::{JacWriter, WriterFinish, WriterMetrics};

use runtime::RuntimeMeasurement;

use serde::{Deserialize, Serialize};
use serde_json::{Deserializer, Map, Value};
use std::convert::TryFrom;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, Write};
use std::path::PathBuf;

#[derive(Debug, Default, Serialize, Deserialize)]
struct HeaderMetadata {
    #[serde(default)]
    segment_max_bytes: Option<u64>,
}

pub(crate) fn encode_header_metadata(limits: &Limits) -> Result<Vec<u8>> {
    let default_limit = Limits::default().max_segment_uncompressed_len as u64;
    let current_limit = limits.max_segment_uncompressed_len as u64;
    if current_limit == default_limit {
        return Ok(Vec::new());
    }
    let metadata = HeaderMetadata {
        segment_max_bytes: Some(current_limit),
    };
    serde_json::to_vec(&metadata).map_err(JacError::from)
}

pub(crate) fn decode_segment_limit(metadata: &[u8]) -> Option<usize> {
    if metadata.is_empty() {
        return None;
    }
    let parsed: HeaderMetadata = serde_json::from_slice(metadata).ok()?;
    let value = parsed.segment_max_bytes?;
    usize::try_from(value).ok()
}

/// Convenience alias for trait objects that need `Read + Seek + Send` bounds.
pub trait ReadSeekSend: Read + Seek + Send {}
impl<T: Read + Seek + Send> ReadSeekSend for T {}

/// Convenience alias for trait objects that need `Write + Send` bounds.
pub trait WriteSend: Write + Send {}
impl<T: Write + Send> WriteSend for T {}

/// High-level compression options
#[derive(Debug, Clone)]
pub struct CompressOptions {
    /// Target number of records per block
    pub block_target_records: usize,
    /// Default compression codec
    pub default_codec: Codec,
    /// Canonicalize keys (lexicographic order)
    pub canonicalize_keys: bool,
    /// Canonicalize numbers (scientific notation, trim trailing zeros)
    pub canonicalize_numbers: bool,
    /// Nested objects/arrays are opaque (v1 behavior)
    pub nested_opaque: bool,
    /// Maximum dictionary entries per field
    pub max_dict_entries: usize,
    /// Security limits
    pub limits: Limits,
    /// Parallel execution tuning parameters.
    pub parallel_config: parallel::ParallelConfig,
}

impl Default for CompressOptions {
    fn default() -> Self {
        Self {
            block_target_records: 100_000,
            default_codec: Codec::Zstd(15),
            canonicalize_keys: false,
            canonicalize_numbers: false,
            nested_opaque: true,
            max_dict_entries: 4_096,
            limits: Limits::default(),
            parallel_config: parallel::ParallelConfig::default(),
        }
    }
}

/// High-level decompression options
#[derive(Debug, Clone)]
pub struct DecompressOptions {
    /// Security limits
    pub limits: Limits,
    /// Verify block CRC32C (recommended)
    pub verify_checksums: bool,
}

impl Default for DecompressOptions {
    fn default() -> Self {
        Self {
            limits: Limits::default(),
            verify_checksums: true,
        }
    }
}

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
    /// Hard maximum limits that cannot be exceeded
    pub fn hard_maximums() -> Self {
        Self {
            max_depth: 10,
            max_buffer_bytes: 128 * 1024 * 1024,
            max_pointer_length: 2048,
        }
    }

    /// Validate limits against hard maximums
    pub fn validate(&self) -> std::result::Result<(), WrapperError> {
        let hard = Self::hard_maximums();

        if self.max_depth > hard.max_depth {
            return Err(WrapperError::ConfigurationExceedsHardLimits {
                reason: format!("max_depth {} exceeds {}", self.max_depth, hard.max_depth),
                max_depth: hard.max_depth,
                max_buffer: hard.max_buffer_bytes,
                max_ptr_len: hard.max_pointer_length,
            });
        }

        if self.max_buffer_bytes > hard.max_buffer_bytes {
            return Err(WrapperError::ConfigurationExceedsHardLimits {
                reason: format!(
                    "max_buffer_bytes {} exceeds {}",
                    self.max_buffer_bytes, hard.max_buffer_bytes
                ),
                max_depth: hard.max_depth,
                max_buffer: hard.max_buffer_bytes,
                max_ptr_len: hard.max_pointer_length,
            });
        }

        if self.max_pointer_length > hard.max_pointer_length {
            return Err(WrapperError::ConfigurationExceedsHardLimits {
                reason: format!(
                    "max_pointer_length {} exceeds {}",
                    self.max_pointer_length, hard.max_pointer_length
                ),
                max_depth: hard.max_depth,
                max_buffer: hard.max_buffer_bytes,
                max_ptr_len: hard.max_pointer_length,
            });
        }

        Ok(())
    }
}

/// Specification for a single section in multi-section wrapper mode.
#[derive(Debug, Clone)]
pub struct SectionSpec {
    /// Name of the section (used for identification)
    pub name: String,
    /// JSON Pointer path to the section data
    pub pointer: String,
    /// Optional label to inject into records from this section
    pub label: Option<String>,
}

/// Behavior when a section is not found in the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingSectionBehavior {
    /// Skip the section silently (default)
    Skip,
    /// Return an error
    Error,
}

impl Default for MissingSectionBehavior {
    fn default() -> Self {
        MissingSectionBehavior::Skip
    }
}

/// Behavior when a key field collision occurs in map mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCollisionMode {
    /// Return an error (default)
    Error,
    /// Overwrite the existing field with the map key
    Overwrite,
}

impl Default for KeyCollisionMode {
    fn default() -> Self {
        KeyCollisionMode::Error
    }
}

/// Configuration for JSON wrapper preprocessing.
#[derive(Debug, Clone)]
pub enum WrapperConfig {
    /// No wrapper preprocessing
    None,
    /// JSON Pointer-based envelope extraction
    Pointer {
        /// RFC 6901 JSON Pointer path
        path: String,
        /// Limits for this wrapper
        limits: WrapperLimits,
    },
    /// Multi-section array concatenation
    Sections {
        /// Section specifications (order determines output order)
        entries: Vec<SectionSpec>,
        /// Limits for this wrapper
        limits: WrapperLimits,
        /// Field name for injected section label (default: "_section")
        label_field: Option<String>,
        /// Whether to inject section labels into records (default: true)
        inject_label: bool,
        /// Behavior when a section is not found
        missing_behavior: MissingSectionBehavior,
    },
    /// Keyed map object flattening (object-of-objects to records)
    KeyedMap {
        /// JSON Pointer to the map object (empty string for root)
        pointer: String,
        /// Field name for the injected key (default: "_key")
        key_field: String,
        /// Limits for this wrapper
        limits: WrapperLimits,
        /// Behavior when key field already exists in a record
        collision_mode: KeyCollisionMode,
    },
    /// Array-with-headers wrapper (CSV-like format)
    ArrayWithHeaders {
        /// Limits for this wrapper
        limits: WrapperLimits,
    },
    /// Custom plugin-based wrapper
    Plugin {
        /// Name of the registered plugin to use
        plugin_name: String,
        /// Plugin-specific configuration (JSON value)
        config: Value,
        /// Limits for this wrapper
        limits: WrapperLimits,
    },
}

impl Default for WrapperConfig {
    fn default() -> Self {
        WrapperConfig::None
    }
}

/// Metrics captured during wrapper preprocessing.
#[derive(Debug, Clone)]
pub struct WrapperMetrics {
    /// Wrapper mode used
    pub mode: String,
    /// Peak buffer bytes used
    pub buffer_peak_bytes: usize,
    /// Records emitted after unwrapping
    pub records_emitted: usize,
    /// Time spent in wrapper processing
    pub processing_duration: std::time::Duration,
    /// Pointer path (if applicable)
    pub pointer_path: Option<String>,
    /// Section-specific record counts (if applicable)
    pub section_counts: Option<Vec<(String, usize)>>,
    /// Map entry count (if applicable)
    pub map_entry_count: Option<usize>,
    /// Header field count for array-with-headers mode (if applicable)
    pub header_field_count: Option<usize>,
    /// Plugin name (if applicable)
    pub plugin_name: Option<String>,
}

/// Sources that can feed records into compression.
pub enum InputSource {
    /// NDJSON file path.
    NdjsonPath(PathBuf),
    /// JSON array file path.
    JsonArrayPath(PathBuf),
    /// NDJSON data from a reader.
    NdjsonReader(Box<dyn Read + Send>),
    /// JSON array data from a reader.
    JsonArrayReader(Box<dyn Read + Send>),
    /// Iterator yielding JSON objects.
    Iterator(Box<dyn Iterator<Item = Map<String, Value>> + Send>),
}

/// Outputs supported by high-level APIs.
pub enum OutputSink {
    /// Write to a file path (created or truncated).
    Path(PathBuf),
    /// Write to an arbitrary `Write` implementation.
    Writer(Box<dyn WriteSend>),
}

/// Input sources that require random access (for readers).
pub enum JacInput {
    /// Input from a file path.
    Path(PathBuf),
    /// Input from an arbitrary `Read + Seek` source.
    Reader(Box<dyn ReadSeekSend>),
}

/// Output formats for full decompression.
#[derive(Debug, Clone, Copy)]
pub enum DecompressFormat {
    /// Follow the container hint stored in the file header (default to NDJSON).
    Auto,
    /// Emit NDJSON (one object per line).
    Ndjson,
    /// Emit a JSON array (`[ {...}, {...} ]`).
    JsonArray,
}

/// Output formats for projection operations.
#[derive(Debug, Clone, Copy)]
pub enum ProjectFormat {
    /// Emit NDJSON (one object per line).
    Ndjson,
    /// Emit JSON array.
    JsonArray,
    /// Emit CSV rows.
    Csv {
        /// Emit a header row containing field names when `true`.
        headers: bool,
    },
}

/// Compression request describing input, output and options.
pub struct CompressRequest {
    /// Source of JSON records.
    pub input: InputSource,
    /// Destination for the resulting JAC payload.
    pub output: OutputSink,
    /// Compression options.
    pub options: CompressOptions,
    /// Optional container format hint supplied by the caller.
    pub container_hint: Option<ContainerFormat>,
    /// Emit index footer/pointer when finishing.
    pub emit_index: bool,
    /// Wrapper configuration for input preprocessing.
    pub wrapper_config: WrapperConfig,
}

impl Default for CompressRequest {
    fn default() -> Self {
        Self {
            input: InputSource::Iterator(Box::new(std::iter::empty())),
            output: OutputSink::Writer(Box::new(Vec::new())),
            options: CompressOptions::default(),
            container_hint: None,
            emit_index: true,
            wrapper_config: WrapperConfig::None,
        }
    }
}

/// Decompression request describing input, output and options.
pub struct DecompressRequest {
    /// Source JAC file or reader.
    pub input: JacInput,
    /// Destination for decompressed JSON.
    pub output: OutputSink,
    /// Output format.
    pub format: DecompressFormat,
    /// Decompression options.
    pub options: DecompressOptions,
}

/// Projection request across selected fields.
pub struct ProjectRequest {
    /// Source JAC file or reader.
    pub input: JacInput,
    /// Destination for projected data.
    pub output: OutputSink,
    /// Fields to project.
    pub fields: Vec<String>,
    /// Output format.
    pub format: ProjectFormat,
    /// Decompression options.
    pub options: DecompressOptions,
}

/// Summary returned after a compression request.
#[derive(Debug, Clone)]
pub struct CompressSummary {
    /// Metrics produced by the writer.
    pub metrics: WriterMetrics,
    /// Parallel decision taken for this compression request.
    pub parallel_decision: Option<parallel::ParallelDecision>,
    /// Wall-clock and memory statistics gathered during compression.
    pub runtime_stats: CompressionRuntimeStats,
    /// Wrapper preprocessing metrics (if wrapper was used).
    pub wrapper_metrics: Option<WrapperMetrics>,
}

/// Runtime statistics captured during compression.
#[derive(Debug, Clone, Copy)]
pub struct CompressionRuntimeStats {
    /// Total wall-clock time spent in the compression pipeline.
    pub wall_time: std::time::Duration,
    /// Observed peak resident set size in bytes (if available).
    pub peak_rss_bytes: Option<u64>,
}

impl Default for CompressionRuntimeStats {
    fn default() -> Self {
        Self {
            wall_time: std::time::Duration::default(),
            peak_rss_bytes: None,
        }
    }
}

/// Summary returned after decompression.
#[derive(Debug, Clone, Copy, Default)]
pub struct DecompressSummary {
    /// Number of records emitted.
    pub records_written: u64,
    /// Number of blocks processed.
    pub blocks_processed: usize,
}

/// Summary returned after projection.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProjectSummary {
    /// Number of rows written to the sink.
    pub rows_written: u64,
}

/// Build a `FileHeader` configured according to the provided compression options.
pub(crate) fn build_file_header(
    options: &CompressOptions,
    container_hint: Option<ContainerFormat>,
) -> Result<FileHeader> {
    let mut flags = 0u32;
    if options.canonicalize_keys {
        flags |= jac_format::constants::FLAG_CANONICALIZE_KEYS;
    }
    if options.canonicalize_numbers {
        flags |= jac_format::constants::FLAG_CANONICALIZE_NUMBERS;
    }
    if options.nested_opaque {
        flags |= jac_format::constants::FLAG_NESTED_OPAQUE;
    }

    let mut header = FileHeader {
        flags,
        default_compressor: options.default_codec.compressor_id(),
        default_compression_level: options.default_codec.level(),
        block_size_hint_records: options.block_target_records,
        user_metadata: encode_header_metadata(&options.limits)?,
    };

    if let Some(hint) = container_hint {
        header.set_container_format_hint(hint);
    }

    Ok(header)
}

/// Execute a compression request.
pub fn execute_compress(request: CompressRequest) -> Result<CompressSummary> {
    let decision = crate::parallel::should_use_parallel(
        &request.input,
        &request.options.limits,
        &request.options.parallel_config,
    )?;

    if decision.use_parallel {
        let mut summary =
            crate::parallel::execute_compress_parallel(request, decision.thread_count)?;
        summary.parallel_decision = Some(decision);
        return Ok(summary);
    }

    let mut summary = execute_compress_sequential(request)?;
    summary.parallel_decision = Some(decision);
    Ok(summary)
}

pub(crate) fn execute_compress_sequential(request: CompressRequest) -> Result<CompressSummary> {
    let measurement = RuntimeMeasurement::begin();
    let CompressRequest {
        input,
        output,
        options,
        container_hint,
        emit_index,
        wrapper_config,
    } = request;

    let mut stream = input.into_record_stream(&wrapper_config)?;
    let detected_hint = stream.container_format();
    let final_hint = container_hint.unwrap_or(detected_hint);
    let wrapper_metrics = stream.take_wrapper_metrics();
    let writer_target = output.into_writer()?;
    let buf_writer = BufWriter::new(writer_target);
    let header = build_file_header(&options, Some(final_hint))?;

    let codec_opts = CompressOpts {
        block_target_records: options.block_target_records,
        default_codec: options.default_codec,
        canonicalize_keys: options.canonicalize_keys,
        canonicalize_numbers: options.canonicalize_numbers,
        nested_opaque: options.nested_opaque,
        max_dict_entries: options.max_dict_entries,
        limits: options.limits,
    };

    let mut jac_writer = JacWriter::new(buf_writer, header, codec_opts)?;

    for record in stream {
        let record = record?;
        jac_writer.write_record(&record)?;
    }

    let finish = if emit_index {
        jac_writer.finish_with_index()?
    } else {
        jac_writer.finish_without_index()?
    };

    let mut buf_writer = finish.writer;
    buf_writer.flush()?;
    drop(buf_writer);

    let runtime_stats = measurement.finish();

    Ok(CompressSummary {
        metrics: finish.metrics,
        parallel_decision: None,
        runtime_stats,
        wrapper_metrics,
    })
}

/// Execute a decompression request.
pub fn execute_decompress(request: DecompressRequest) -> Result<DecompressSummary> {
    let DecompressRequest {
        input,
        output,
        format,
        options,
    } = request;

    let reader_source = input.into_reader()?;
    let codec_opts = DecompressOpts {
        limits: options.limits.clone(),
        verify_checksums: options.verify_checksums,
    };
    let mut reader = JacReader::new(reader_source, codec_opts)?;
    let header_hint = reader.file_header().container_format_hint()?;
    let resolved_format = match format {
        DecompressFormat::Auto => match header_hint {
            ContainerFormat::JsonArray => DecompressFormat::JsonArray,
            _ => DecompressFormat::Ndjson,
        },
        other => other,
    };

    let mut buf_writer = BufWriter::new(output.into_writer()?);
    let mut record_stream = reader.record_stream()?;
    let mut summary = DecompressSummary {
        records_written: 0,
        blocks_processed: 0,
    };

    match resolved_format {
        DecompressFormat::Ndjson => {
            for record in record_stream.by_ref() {
                let record = record?;
                serde_json::to_writer(&mut buf_writer, &Value::Object(record))?;
                buf_writer.write_all(b"\n")?;
                summary.records_written += 1;
            }
        }
        DecompressFormat::JsonArray => {
            buf_writer.write_all(b"[")?;
            let mut first = true;
            for record in record_stream.by_ref() {
                let record = record?;
                if first {
                    first = false;
                } else {
                    buf_writer.write_all(b",")?;
                }
                serde_json::to_writer(&mut buf_writer, &Value::Object(record))?;
                summary.records_written += 1;
            }
            buf_writer.write_all(b"]")?;
        }
        DecompressFormat::Auto => unreachable!("auto must resolve to a concrete format"),
    }

    summary.blocks_processed = record_stream.blocks_processed();
    buf_writer.flush()?;
    Ok(summary)
}

/// Execute a projection request.
pub fn execute_project(request: ProjectRequest) -> Result<ProjectSummary> {
    let ProjectRequest {
        input,
        output,
        fields,
        format,
        options,
    } = request;

    if fields.is_empty() {
        return Err(JacError::Internal(
            "project request requires at least one field".to_string(),
        ));
    }

    let reader_source = input.into_reader()?;
    let codec_opts = DecompressOpts {
        limits: options.limits.clone(),
        verify_checksums: options.verify_checksums,
    };
    let mut reader = JacReader::new(reader_source, codec_opts)?;

    let mut buf_writer = BufWriter::new(output.into_writer()?);
    let mut cursor = BlockCursor::new(&reader);
    let mut summary = ProjectSummary { rows_written: 0 };

    match format {
        ProjectFormat::Ndjson => {
            while let Some(block) = reader.next_block_handle(&mut cursor) {
                let block = block?;
                let decoder = reader.decode_block(&block)?;
                let mut columns = Vec::with_capacity(fields.len());
                for field in &fields {
                    columns.push(decoder.project_field(field)?);
                }

                for record_idx in 0..block.record_count {
                    let mut projected = Map::new();
                    for (field, column) in fields.iter().zip(columns.iter()) {
                        if let Some(Some(value)) = column.get(record_idx) {
                            projected.insert(field.clone(), value.clone());
                        }
                    }
                    serde_json::to_writer(&mut buf_writer, &Value::Object(projected))?;
                    buf_writer.write_all(b"\n")?;
                    summary.rows_written += 1;
                }
            }
        }
        ProjectFormat::JsonArray => {
            buf_writer.write_all(b"[")?;
            let mut first = true;
            while let Some(block) = reader.next_block_handle(&mut cursor) {
                let block = block?;
                let decoder = reader.decode_block(&block)?;
                let mut columns = Vec::with_capacity(fields.len());
                for field in &fields {
                    columns.push(decoder.project_field(field)?);
                }

                for record_idx in 0..block.record_count {
                    if first {
                        first = false;
                    } else {
                        buf_writer.write_all(b",")?;
                    }
                    let mut projected = Map::new();
                    for (field, column) in fields.iter().zip(columns.iter()) {
                        if let Some(Some(value)) = column.get(record_idx) {
                            projected.insert(field.clone(), value.clone());
                        }
                    }
                    serde_json::to_writer(&mut buf_writer, &Value::Object(projected))?;
                    summary.rows_written += 1;
                }
            }
            buf_writer.write_all(b"]")?;
        }
        ProjectFormat::Csv { headers } => {
            if headers {
                write_csv_row(&mut buf_writer, fields.iter().map(|s| s.as_str()))?;
            }

            while let Some(block) = reader.next_block_handle(&mut cursor) {
                let block = block?;
                let decoder = reader.decode_block(&block)?;
                let mut columns = Vec::with_capacity(fields.len());
                for field in &fields {
                    columns.push(decoder.project_field(field)?);
                }

                for record_idx in 0..block.record_count {
                    let mut row = Vec::with_capacity(fields.len());
                    for column in &columns {
                        let cell = column
                            .get(record_idx)
                            .and_then(|opt| opt.as_ref())
                            .map(csv_cell_value)
                            .unwrap_or_default();
                        row.push(cell);
                    }
                    write_csv_row(&mut buf_writer, row.iter().map(|s| s.as_str()))?;
                    summary.rows_written += 1;
                }
            }
        }
    }

    buf_writer.flush()?;
    Ok(summary)
}

#[deprecated(note = "use `execute_compress` with `CompressRequest` instead")]
/// Backward-compatible compression helper (NDJSON input, NDJSON output).
pub fn compress<R, W>(input: R, output: W, opts: CompressOptions) -> Result<()>
where
    R: Read + Send + 'static,
    W: Write + Send + 'static,
{
    let request = CompressRequest {
        input: InputSource::NdjsonReader(Box::new(input)),
        output: OutputSink::Writer(Box::new(output)),
        options: opts,
        container_hint: Some(ContainerFormat::Ndjson),
        emit_index: true,
        wrapper_config: WrapperConfig::None,
    };
    execute_compress(request).map(|_| ())
}

#[deprecated(note = "use `execute_decompress` with `DecompressRequest` instead")]
/// Backward-compatible decompression helper (NDJSON output).
pub fn decompress_full<R, W>(input: R, output: W, opts: DecompressOptions) -> Result<()>
where
    R: Read + Seek + Send + 'static,
    W: Write + Send + 'static,
{
    let request = DecompressRequest {
        input: JacInput::Reader(Box::new(input)),
        output: OutputSink::Writer(Box::new(output)),
        format: DecompressFormat::Ndjson,
        options: opts,
    };
    execute_decompress(request).map(|_| ())
}

#[deprecated(note = "use `execute_project` with `ProjectRequest` instead")]
/// Backward-compatible projection helper (NDJSON output).
pub fn project<R, W>(input: R, output: W, fields: &[&str], as_ndjson: bool) -> Result<()>
where
    R: Read + Seek + Send + 'static,
    W: Write + Send + 'static,
{
    let request = ProjectRequest {
        input: JacInput::Reader(Box::new(input)),
        output: OutputSink::Writer(Box::new(output)),
        fields: fields.iter().map(|s| s.to_string()).collect(),
        format: if as_ndjson {
            ProjectFormat::Ndjson
        } else {
            ProjectFormat::JsonArray
        },
        options: DecompressOptions::default(),
    };
    execute_project(request).map(|_| ())
}

impl InputSource {
    pub(crate) fn into_record_stream(self, wrapper_config: &WrapperConfig) -> Result<RecordStream> {
        // Apply wrapper if configured
        match wrapper_config {
            WrapperConfig::None => {
                // No wrapper, use standard streams
                match self {
                    InputSource::NdjsonPath(path) => {
                        let file = File::open(path)?;
                        Ok(RecordStream::ndjson(BufReader::new(file)))
                    }
                    InputSource::JsonArrayPath(path) => {
                        let file = File::open(path)?;
                        RecordStream::json_array_reader(BufReader::new(file))
                    }
                    InputSource::NdjsonReader(reader) => {
                        Ok(RecordStream::ndjson(BufReader::new(reader)))
                    }
                    InputSource::JsonArrayReader(reader) => {
                        RecordStream::json_array_reader(BufReader::new(reader))
                    }
                    InputSource::Iterator(iter) => Ok(RecordStream::iter(iter)),
                }
            }
            WrapperConfig::Pointer { path, limits } => {
                // Apply pointer wrapper
                use wrapper::pointer::{PointerArrayStream, PointerLimits};

                let reader: Box<dyn Read + Send> = match self {
                    InputSource::NdjsonPath(p) => Box::new(File::open(p)?),
                    InputSource::JsonArrayPath(p) => Box::new(File::open(p)?),
                    InputSource::NdjsonReader(r) => r,
                    InputSource::JsonArrayReader(r) => r,
                    InputSource::Iterator(_) => {
                        return Err(JacError::Internal(
                            "Wrapper configuration cannot be applied to Iterator input source"
                                .to_string(),
                        ));
                    }
                };

                let pointer_limits = PointerLimits {
                    max_depth: limits.max_depth,
                    max_buffer_bytes: limits.max_buffer_bytes,
                    max_pointer_length: limits.max_pointer_length,
                };

                let stream = PointerArrayStream::new(reader, path.clone(), pointer_limits)
                    .map_err(|e| JacError::Internal(format!("Wrapper error: {}", e)))?;

                let metrics = WrapperMetrics {
                    mode: "pointer".to_string(),
                    buffer_peak_bytes: stream.metrics().peak_buffer_bytes,
                    records_emitted: stream.metrics().records_emitted,
                    processing_duration: stream.metrics().processing_duration,
                    pointer_path: Some(path.clone()),
                    section_counts: None,
                    map_entry_count: None,
                    header_field_count: None,
                    plugin_name: None,
                };

                Ok(RecordStream::wrapper(Box::new(stream), metrics))
            }
            WrapperConfig::Sections {
                entries,
                limits,
                label_field,
                inject_label,
                missing_behavior,
            } => {
                // Apply sections wrapper
                use wrapper::sections::SectionsStream;

                let reader: Box<dyn Read + Send> = match self {
                    InputSource::NdjsonPath(p) => Box::new(File::open(p)?),
                    InputSource::JsonArrayPath(p) => Box::new(File::open(p)?),
                    InputSource::NdjsonReader(r) => r,
                    InputSource::JsonArrayReader(r) => r,
                    InputSource::Iterator(_) => {
                        return Err(JacError::Internal(
                            "Wrapper configuration cannot be applied to Iterator input source"
                                .to_string(),
                        ));
                    }
                };

                let stream = SectionsStream::new(
                    reader,
                    entries.clone(),
                    limits.clone(),
                    label_field.clone(),
                    *inject_label,
                    *missing_behavior,
                )
                .map_err(|e| JacError::Internal(format!("Wrapper error: {}", e)))?;

                let metrics = WrapperMetrics {
                    mode: "sections".to_string(),
                    buffer_peak_bytes: stream.metrics().peak_buffer_bytes,
                    records_emitted: stream.metrics().records_emitted,
                    processing_duration: stream.metrics().processing_duration,
                    pointer_path: None,
                    section_counts: Some(stream.metrics().section_counts.clone()),
                    map_entry_count: None,
                    header_field_count: None,
                    plugin_name: None,
                };

                Ok(RecordStream::wrapper(Box::new(stream), metrics))
            }
            WrapperConfig::KeyedMap {
                pointer,
                key_field,
                limits,
                collision_mode,
            } => {
                // Apply keyed map wrapper
                use wrapper::map::KeyedMapStream;

                let reader: Box<dyn Read + Send> = match self {
                    InputSource::NdjsonPath(p) => Box::new(File::open(p)?),
                    InputSource::JsonArrayPath(p) => Box::new(File::open(p)?),
                    InputSource::NdjsonReader(r) => r,
                    InputSource::JsonArrayReader(r) => r,
                    InputSource::Iterator(_) => {
                        return Err(JacError::Internal(
                            "Wrapper configuration cannot be applied to Iterator input source"
                                .to_string(),
                        ));
                    }
                };

                let stream = KeyedMapStream::new(
                    reader,
                    pointer.clone(),
                    key_field.clone(),
                    limits.clone(),
                    *collision_mode,
                )
                .map_err(|e| JacError::Internal(format!("Wrapper error: {}", e)))?;

                let metrics = WrapperMetrics {
                    mode: "map".to_string(),
                    buffer_peak_bytes: stream.metrics().peak_buffer_bytes,
                    records_emitted: stream.metrics().records_emitted,
                    processing_duration: stream.metrics().processing_duration,
                    pointer_path: if pointer.is_empty() {
                        None
                    } else {
                        Some(pointer.clone())
                    },
                    section_counts: None,
                    map_entry_count: Some(stream.metrics().map_entry_count),
                    header_field_count: None,
                    plugin_name: None,
                };

                Ok(RecordStream::wrapper(Box::new(stream), metrics))
            }
            WrapperConfig::ArrayWithHeaders { limits } => {
                // Apply array-with-headers wrapper
                use wrapper::array_headers::ArrayHeadersStream;

                let reader: Box<dyn Read + Send> = match self {
                    InputSource::NdjsonPath(p) => Box::new(File::open(p)?),
                    InputSource::JsonArrayPath(p) => Box::new(File::open(p)?),
                    InputSource::NdjsonReader(r) => r,
                    InputSource::JsonArrayReader(r) => r,
                    InputSource::Iterator(_) => {
                        return Err(JacError::Internal(
                            "Wrapper configuration cannot be applied to Iterator input source"
                                .to_string(),
                        ));
                    }
                };

                let stream = ArrayHeadersStream::new(reader, limits.clone())
                    .map_err(|e| JacError::Internal(format!("Wrapper error: {}", e)))?;

                let metrics = WrapperMetrics {
                    mode: "array-with-headers".to_string(),
                    buffer_peak_bytes: stream.metrics().peak_buffer_bytes,
                    records_emitted: stream.metrics().records_emitted,
                    processing_duration: stream.metrics().processing_duration,
                    pointer_path: None,
                    section_counts: None,
                    map_entry_count: None,
                    header_field_count: Some(stream.metrics().header_field_count),
                    plugin_name: None,
                };

                Ok(RecordStream::wrapper(Box::new(stream), metrics))
            }
            WrapperConfig::Plugin {
                plugin_name,
                config,
                limits,
            } => {
                // Apply plugin wrapper
                use wrapper::plugin::WrapperPluginRegistry;

                let reader: Box<dyn Read + Send> = match self {
                    InputSource::NdjsonPath(p) => Box::new(File::open(p)?),
                    InputSource::JsonArrayPath(p) => Box::new(File::open(p)?),
                    InputSource::NdjsonReader(r) => r,
                    InputSource::JsonArrayReader(r) => r,
                    InputSource::Iterator(_) => {
                        return Err(JacError::Internal(
                            "Wrapper configuration cannot be applied to Iterator input source"
                                .to_string(),
                        ));
                    }
                };

                let registry = WrapperPluginRegistry::global();
                let plugin = registry.get(plugin_name).ok_or_else(|| {
                    JacError::Internal(format!("Plugin '{}' not found", plugin_name))
                })?;

                // Validate configuration
                plugin
                    .validate_config(config)
                    .map_err(|e| JacError::Internal(format!("Wrapper error: {}", e)))?;

                // Process with plugin
                let start = std::time::Instant::now();
                let iterator = plugin
                    .process(reader, config, limits)
                    .map_err(|e| JacError::Internal(format!("Wrapper error: {}", e)))?;

                let processing_duration = start.elapsed();

                let metrics = WrapperMetrics {
                    mode: "plugin".to_string(),
                    buffer_peak_bytes: 0, // Plugins must track their own buffer usage
                    records_emitted: 0,   // Will be updated during iteration
                    processing_duration,
                    pointer_path: None,
                    section_counts: None,
                    map_entry_count: None,
                    header_field_count: None,
                    plugin_name: Some(plugin_name.clone()),
                };

                Ok(RecordStream::wrapper(iterator, metrics))
            }
        }
    }
}

impl OutputSink {
    pub(crate) fn into_writer(self) -> Result<Box<dyn WriteSend>> {
        match self {
            OutputSink::Path(path) => Ok(Box::new(File::create(path)?)),
            OutputSink::Writer(writer) => Ok(writer),
        }
    }
}

impl JacInput {
    fn into_reader(self) -> Result<Box<dyn ReadSeekSend>> {
        match self {
            JacInput::Path(path) => Ok(Box::new(File::open(path)?)),
            JacInput::Reader(reader) => Ok(reader),
        }
    }
}

fn write_csv_row<'a, I, W>(writer: &mut W, cells: I) -> Result<()>
where
    I: IntoIterator<Item = &'a str>,
    W: Write,
{
    let mut first = true;
    for cell in cells {
        if first {
            first = false;
        } else {
            writer.write_all(b",")?;
        }
        write_csv_cell(writer, cell)?;
    }
    writer.write_all(b"\n")?;
    Ok(())
}

fn write_csv_cell<W: Write>(writer: &mut W, cell: &str) -> Result<()> {
    let needs_quotes = cell.contains([',', '"', '\n']);
    if needs_quotes {
        writer.write_all(b"\"")?;
        for ch in cell.chars() {
            if ch == '"' {
                writer.write_all(b"\"")?;
            }
            let mut buf = [0u8; 4];
            writer.write_all(ch.encode_utf8(&mut buf).as_bytes())?;
        }
        writer.write_all(b"\"")?;
    } else {
        writer.write_all(cell.as_bytes())?;
    }
    Ok(())
}

fn csv_cell_value(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}

/// Record stream used during compression.
struct RecordStream {
    inner: RecordStreamInner,
    format: ContainerFormat,
    wrapper_metrics: Option<WrapperMetrics>,
}

enum RecordStreamInner {
    Ndjson(NdjsonStream),
    JsonArray(JsonArrayStream),
    Iterator(Box<dyn Iterator<Item = Map<String, Value>> + Send>),
    Wrapper(Box<dyn Iterator<Item = std::result::Result<Map<String, Value>, WrapperError>> + Send>),
}

impl RecordStream {
    fn ndjson<R: BufRead + Send + 'static>(reader: R) -> Self {
        Self {
            inner: RecordStreamInner::Ndjson(NdjsonStream {
                reader: Box::new(reader),
                buffer: String::new(),
            }),
            format: ContainerFormat::Ndjson,
            wrapper_metrics: None,
        }
    }

    fn json_array_reader<R: BufRead + Send + 'static>(reader: R) -> Result<Self> {
        let stream = JsonArrayStream::from_reader(Box::new(reader))?;
        Ok(Self {
            inner: RecordStreamInner::JsonArray(stream),
            format: ContainerFormat::JsonArray,
            wrapper_metrics: None,
        })
    }

    fn iter(iter: Box<dyn Iterator<Item = Map<String, Value>> + Send>) -> Self {
        Self {
            inner: RecordStreamInner::Iterator(iter),
            format: ContainerFormat::Unknown,
            wrapper_metrics: None,
        }
    }

    fn wrapper(
        iter: Box<
            dyn Iterator<Item = std::result::Result<Map<String, Value>, WrapperError>> + Send,
        >,
        metrics: WrapperMetrics,
    ) -> Self {
        Self {
            inner: RecordStreamInner::Wrapper(iter),
            format: ContainerFormat::JsonArray, // Wrappers always produce array-like output
            wrapper_metrics: Some(metrics),
        }
    }

    fn container_format(&self) -> ContainerFormat {
        self.format
    }

    fn take_wrapper_metrics(&mut self) -> Option<WrapperMetrics> {
        self.wrapper_metrics.take()
    }
}

impl Iterator for RecordStream {
    type Item = Result<Map<String, Value>>;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.inner {
            RecordStreamInner::Ndjson(stream) => stream.next(),
            RecordStreamInner::JsonArray(stream) => stream.next(),
            RecordStreamInner::Iterator(iter) => iter.next().map(Ok),
            RecordStreamInner::Wrapper(iter) => iter
                .next()
                .map(|r| r.map_err(|e| JacError::Internal(format!("Wrapper error: {}", e)))),
        }
    }
}

struct NdjsonStream {
    reader: Box<dyn BufRead + Send>,
    buffer: String,
}

impl Iterator for NdjsonStream {
    type Item = Result<Map<String, Value>>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            self.buffer.clear();
            match self.reader.read_line(&mut self.buffer) {
                Ok(0) => return None,
                Ok(_) => {
                    if self.buffer.starts_with('\u{feff}') {
                        let bom_len = '\u{feff}'.len_utf8();
                        if self.buffer.len() >= bom_len {
                            self.buffer.drain(..bom_len);
                        }
                    }
                    if self.buffer.trim().is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<Map<String, Value>>(&self.buffer) {
                        Ok(map) => return Some(Ok(map)),
                        Err(err) => return Some(Err(JacError::from(err))),
                    }
                }
                Err(err) => return Some(Err(JacError::from(err))),
            }
        }
    }
}

struct JsonArrayStream {
    reader: Box<dyn BufRead + Send>,
    mode: JsonArrayMode,
    finished: bool,
    array_expect_value: bool,
    emitted_any: bool,
    consumed_single_object: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JsonArrayMode {
    Array,
    SingleObject,
}

impl JsonArrayStream {
    fn from_reader(reader: Box<dyn BufRead + Send>) -> Result<Self> {
        let mut stream = Self {
            reader,
            mode: JsonArrayMode::Array,
            finished: false,
            array_expect_value: true,
            emitted_any: false,
            consumed_single_object: false,
        };
        stream.consume_bom()?;
        stream.initialize_mode()?;
        Ok(stream)
    }

    fn consume_bom(&mut self) -> Result<()> {
        let buf = self.reader.fill_buf()?;
        if buf.starts_with(&[0xEF, 0xBB, 0xBF]) {
            self.reader.consume(3);
        }
        Ok(())
    }

    fn initialize_mode(&mut self) -> Result<()> {
        let first = self.peek_non_whitespace()?;
        match first {
            Some(b'[') => {
                self.reader.consume(1);
                self.mode = JsonArrayMode::Array;
                self.finished = false;
                self.array_expect_value = true;
                self.emitted_any = false;
                Ok(())
            }
            Some(b'{') => {
                self.mode = JsonArrayMode::SingleObject;
                self.finished = false;
                self.consumed_single_object = false;
                self.array_expect_value = false;
                self.emitted_any = false;
                Ok(())
            }
            Some(_) => Err(JacError::TypeMismatch),
            None => Err(JacError::UnexpectedEof),
        }
    }

    fn peek_non_whitespace(&mut self) -> Result<Option<u8>> {
        loop {
            let (consumed, next_byte) = {
                let buf = self.reader.fill_buf()?;
                if buf.is_empty() {
                    return Ok(None);
                }

                let mut idx = 0;
                while idx < buf.len() && buf[idx].is_ascii_whitespace() {
                    idx += 1;
                }

                if idx < buf.len() {
                    (idx, Some(buf[idx]))
                } else {
                    (idx, None)
                }
            };

            if consumed > 0 {
                self.reader.consume(consumed);
            }

            if let Some(byte) = next_byte {
                return Ok(Some(byte));
            }
        }
    }

    fn parse_object(&mut self) -> Result<Map<String, Value>> {
        let mut de = Deserializer::from_reader(&mut self.reader);
        Map::<String, Value>::deserialize(&mut de).map_err(JacError::from)
    }

    fn next_from_array(&mut self) -> Result<Option<Map<String, Value>>> {
        loop {
            let next = match self.peek_non_whitespace()? {
                Some(byte) => byte,
                None => return Err(JacError::UnexpectedEof),
            };

            if self.array_expect_value {
                if next == b']' {
                    if self.emitted_any {
                        return Err(JacError::TypeMismatch);
                    }
                    self.reader.consume(1);
                    self.finished = true;
                    return Ok(None);
                }

                let map = self.parse_object()?;
                self.array_expect_value = false;
                self.emitted_any = true;
                return Ok(Some(map));
            } else {
                match next {
                    b',' => {
                        self.reader.consume(1);
                        self.array_expect_value = true;
                        continue;
                    }
                    b']' => {
                        self.reader.consume(1);
                        self.finished = true;
                        return Ok(None);
                    }
                    _ => return Err(JacError::TypeMismatch),
                }
            }
        }
    }
}

impl Iterator for JsonArrayStream {
    type Item = Result<Map<String, Value>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }

        let outcome = match self.mode {
            JsonArrayMode::SingleObject => {
                if self.consumed_single_object {
                    self.finished = true;
                    return None;
                }
                Some(match self.parse_object() {
                    Ok(map) => {
                        self.consumed_single_object = true;
                        self.finished = true;
                        Ok(map)
                    }
                    Err(err) => Err(err),
                })
            }
            JsonArrayMode::Array => match self.next_from_array() {
                Ok(Some(map)) => Some(Ok(map)),
                Ok(None) => {
                    self.finished = true;
                    return None;
                }
                Err(err) => Some(Err(err)),
            },
        };

        match outcome {
            Some(Ok(map)) => Some(Ok(map)),
            Some(Err(err)) => {
                self.finished = true;
                Some(Err(err))
            }
            None => None,
        }
    }
}
impl JacReader<File> {
    /// Convenience helper to create a reader from a file path.
    pub fn open(path: impl Into<PathBuf>, opts: DecompressOpts) -> Result<Self> {
        let file = File::open(path.into())?;
        JacReader::new(file, opts)
    }
}

#[cfg(feature = "async")]
pub mod async_io {
    //! Async facade wrapping the blocking high-level APIs.
    use super::{
        execute_compress, execute_decompress, execute_project, CompressRequest, DecompressRequest,
        ProjectRequest,
    };
    use jac_format::{JacError, Result};
    use tokio::task;

    /// Execute compression on a blocking thread pool.
    pub async fn compress(request: CompressRequest) -> Result<super::CompressSummary> {
        task::spawn_blocking(move || execute_compress(request))
            .await
            .map_err(|err| JacError::Internal(format!("spawn_blocking join error: {err}")))?
    }

    /// Execute decompression on a blocking thread pool.
    pub async fn decompress(request: DecompressRequest) -> Result<super::DecompressSummary> {
        task::spawn_blocking(move || execute_decompress(request))
            .await
            .map_err(|err| JacError::Internal(format!("spawn_blocking join error: {err}")))?
    }

    /// Execute projection on a blocking thread pool.
    pub async fn project(request: ProjectRequest) -> Result<super::ProjectSummary> {
        task::spawn_blocking(move || execute_project(request))
            .await
            .map_err(|err| JacError::Internal(format!("spawn_blocking join error: {err}")))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Map, Value};
    use std::fs;
    use std::io::Cursor;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tempfile::tempdir;

    #[test]
    fn ndjson_input_streams_records() {
        let data = "{\"a\":1}\n{\"b\":2}\n";
        let mut stream = InputSource::NdjsonReader(Box::new(data.as_bytes()))
            .into_record_stream(&WrapperConfig::None)
            .unwrap();
        let first = stream.next().unwrap().unwrap();
        assert_eq!(first.get("a").unwrap(), &Value::from(1));
        let second = stream.next().unwrap().unwrap();
        assert_eq!(second.get("b").unwrap(), &Value::from(2));
        assert!(stream.next().is_none());
    }

    #[test]
    fn json_array_input_streams_records() {
        let data = r#"[{"a":1},{"b":2}]"#;
        let reader = Cursor::new(data.as_bytes().to_vec());
        let mut stream = InputSource::JsonArrayReader(Box::new(reader))
            .into_record_stream(&WrapperConfig::None)
            .unwrap();
        let first = stream.next().unwrap().unwrap();
        assert_eq!(first.get("a").unwrap(), &Value::from(1));
        let second = stream.next().unwrap().unwrap();
        assert_eq!(second.get("b").unwrap(), &Value::from(2));
        assert!(stream.next().is_none());
    }

    #[test]
    fn ndjson_input_handles_bom_and_mixed_newlines() {
        let data = "\u{feff}{\"a\":1}\r\n\r\n{\"b\":2}\n{\"c\":3}";
        let reader = Cursor::new(data.as_bytes().to_vec());
        let mut stream = InputSource::NdjsonReader(Box::new(reader))
            .into_record_stream(&WrapperConfig::None)
            .unwrap();

        let first = stream.next().unwrap().unwrap();
        assert_eq!(first.get("a"), Some(&Value::from(1)));

        let second = stream.next().unwrap().unwrap();
        assert_eq!(second.get("b"), Some(&Value::from(2)));

        let third = stream.next().unwrap().unwrap();
        assert_eq!(third.get("c"), Some(&Value::from(3)));

        assert!(stream.next().is_none());
    }

    #[test]
    fn compress_and_decompress_roundtrip_ndjson() {
        let data = "{\"id\":1}\n{\"id\":2,\"name\":\"alice\"}\n";
        let paths = TempPaths::new("roundtrip");

        fs::write(&paths.input_ndjson, data).unwrap();

        let compress_request = CompressRequest {
            input: InputSource::NdjsonPath(paths.input_ndjson.clone()),
            output: OutputSink::Path(paths.output_jac.clone()),
            options: CompressOptions::default(),
            container_hint: None,
            emit_index: true,
            wrapper_config: WrapperConfig::None,
        };

        execute_compress(compress_request).unwrap();

        let decompress_request = DecompressRequest {
            input: JacInput::Path(paths.output_jac.clone()),
            output: OutputSink::Path(paths.output_json.clone()),
            format: DecompressFormat::Auto,
            options: DecompressOptions::default(),
        };

        execute_decompress(decompress_request).unwrap();

        let result = fs::read_to_string(&paths.output_json).unwrap();
        assert_eq!(normalize_ndjson(&result), normalize_ndjson(data));
    }

    #[test]
    fn project_to_json_array_and_csv() {
        let data = "{\"user\":\"alice\",\"visits\":3}\n{\"user\":\"bob\",\"visits\":5}\n";
        let paths = TempPaths::new("project");

        fs::write(&paths.input_ndjson, data).unwrap();

        let compress_request = CompressRequest {
            input: InputSource::NdjsonPath(paths.input_ndjson.clone()),
            output: OutputSink::Path(paths.output_jac.clone()),
            options: CompressOptions::default(),
            container_hint: None,
            emit_index: true,
            wrapper_config: WrapperConfig::None,
        };
        execute_compress(compress_request).unwrap();

        let projection_json = paths.output_json.with_extension("projection.json");
        let projection_csv = paths.output_json.with_extension("projection.csv");

        let project_request_json = ProjectRequest {
            input: JacInput::Path(paths.output_jac.clone()),
            output: OutputSink::Path(projection_json.clone()),
            fields: vec!["user".to_string()],
            format: ProjectFormat::JsonArray,
            options: DecompressOptions::default(),
        };
        execute_project(project_request_json).unwrap();

        let project_request_csv = ProjectRequest {
            input: JacInput::Path(paths.output_jac.clone()),
            output: OutputSink::Path(projection_csv.clone()),
            fields: vec!["user".to_string(), "visits".to_string()],
            format: ProjectFormat::Csv { headers: true },
            options: DecompressOptions::default(),
        };
        execute_project(project_request_csv).unwrap();

        let json_output = fs::read_to_string(&projection_json).unwrap();
        let parsed: Value = serde_json::from_str(&json_output).unwrap();
        assert_eq!(parsed, json!([{ "user": "alice" }, { "user": "bob" }]));

        let csv_output = fs::read_to_string(&projection_csv).unwrap();
        let lines: Vec<_> = csv_output.lines().collect();
        assert_eq!(lines[0], "user,visits");
        assert_eq!(lines[1], "alice,3");
        assert_eq!(lines[2], "bob,5");

        let _ = fs::remove_file(&projection_json);
        let _ = fs::remove_file(&projection_csv);
    }

    #[test]
    fn compress_json_array_and_auto_decompress_to_array() {
        let dir = tempdir().unwrap();
        let input_path = dir.path().join("input.json");
        let jac_path = dir.path().join("output.jac");
        let output_path = dir.path().join("decoded.json");

        fs::write(&input_path, r#"[{"name":"alice"},{"name":"bob"}]"#).unwrap();

        let compress_request = CompressRequest {
            input: InputSource::JsonArrayPath(input_path.clone()),
            output: OutputSink::Path(jac_path.clone()),
            options: CompressOptions::default(),
            container_hint: Some(ContainerFormat::JsonArray),
            emit_index: true,
            wrapper_config: WrapperConfig::None,
        };
        execute_compress(compress_request).unwrap();

        let decompress_request = DecompressRequest {
            input: JacInput::Path(jac_path.clone()),
            output: OutputSink::Path(output_path.clone()),
            format: DecompressFormat::Auto,
            options: DecompressOptions::default(),
        };
        execute_decompress(decompress_request).unwrap();

        let decoded = fs::read_to_string(&output_path).unwrap();
        let value: Value = serde_json::from_str(&decoded).unwrap();
        assert!(value.is_array());
        assert_eq!(value.as_array().unwrap().len(), 2);
    }

    #[test]
    fn compress_summary_reports_segment_limit_metrics() {
        let mut options = CompressOptions::default();
        options.block_target_records = 10;
        options.default_codec = Codec::None;
        options.limits.max_segment_uncompressed_len = 70;

        let make_record = |payload: &str| -> Map<String, Value> {
            let mut map = Map::new();
            map.insert("field".to_string(), Value::String(payload.to_string()));
            map
        };

        let records = vec![make_record(&"x".repeat(40)), make_record(&"y".repeat(40))];

        let request = CompressRequest {
            input: InputSource::Iterator(Box::new(records.into_iter())),
            output: OutputSink::Writer(Box::new(Vec::new())),
            options,
            container_hint: Some(ContainerFormat::Ndjson),
            emit_index: false,
            wrapper_config: WrapperConfig::None,
        };

        let summary = execute_compress(request).expect("compress succeeds");

        assert_eq!(summary.metrics.segment_limit_record_rejections, 0);
        assert_eq!(summary.metrics.segment_limit_flushes, 1);
        assert_eq!(summary.metrics.blocks_written, 2);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn parallel_pipeline_matches_sequential_output() {
        let dir = tempdir().unwrap();
        let seq_path = dir.path().join("sequential.jac");
        let par_path = dir.path().join("parallel.jac");

        let mut base_options = CompressOptions::default();
        base_options.block_target_records = 4;
        base_options.default_codec = Codec::None;
        base_options.canonicalize_keys = true;
        base_options.canonicalize_numbers = true;
        base_options.nested_opaque = true;

        let make_records = || {
            let mut out = Vec::new();
            for i in 0..12 {
                let mut record = Map::new();
                record.insert("id".to_string(), Value::from(i as i64));
                record.insert(
                    "user".to_string(),
                    Value::String(format!("user_{:02}", i % 5)),
                );
                record.insert("active".to_string(), Value::Bool(i % 2 == 0));
                record.insert(
                    "message".to_string(),
                    Value::String(format!("event {} occurred", i)),
                );
                record.insert("score".to_string(), Value::Number(((i * i) as i64).into()));
                out.push(record);
            }
            out
        };

        let records = make_records();
        let record_count = records.len();
        let seq_records = records.clone();

        let sequential_request = CompressRequest {
            input: InputSource::Iterator(Box::new(seq_records.into_iter())),
            output: OutputSink::Path(seq_path.clone()),
            options: base_options.clone(),
            container_hint: Some(ContainerFormat::Ndjson),
            emit_index: true,
            wrapper_config: WrapperConfig::None,
        };
        let seq_summary = execute_compress_sequential(sequential_request).unwrap();

        let parallel_request = CompressRequest {
            input: InputSource::Iterator(Box::new(records.into_iter())),
            output: OutputSink::Path(par_path.clone()),
            options: base_options,
            container_hint: Some(ContainerFormat::Ndjson),
            emit_index: true,
            wrapper_config: WrapperConfig::None,
        };
        let par_summary = crate::parallel::execute_compress_parallel(parallel_request, 2).unwrap();

        let seq_bytes = fs::read(&seq_path).unwrap();
        let par_bytes = fs::read(&par_path).unwrap();

        assert_eq!(
            seq_bytes, par_bytes,
            "parallel output must match sequential bytes"
        );
        assert_eq!(
            seq_summary.metrics.records_written,
            par_summary.metrics.records_written
        );
        assert_eq!(par_summary.metrics.records_written, record_count as u64);
        assert_eq!(
            seq_summary.metrics.blocks_written,
            par_summary.metrics.blocks_written
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn parallel_pipeline_decompression_matches_original() {
        fn read_records(bytes: &[u8]) -> Vec<Map<String, Value>> {
            let mut reader =
                JacReader::new(Cursor::new(bytes.to_vec()), DecompressOpts::default()).unwrap();
            reader
                .record_stream()
                .unwrap()
                .map(|res| res.unwrap())
                .collect()
        }

        let dir = tempdir().unwrap();
        let seq_path = dir.path().join("seq_decompress.jac");
        let par_path = dir.path().join("par_decompress.jac");

        let mut options = CompressOptions::default();
        options.block_target_records = 5;
        options.default_codec = Codec::None;
        options.canonicalize_keys = true;
        options.canonicalize_numbers = true;

        let records: Vec<Map<String, Value>> = (0..9)
            .map(|i| {
                let mut record = Map::new();
                record.insert("id".to_string(), Value::from(i as i64));
                record.insert("user".to_string(), Value::String(format!("user-{}", i % 3)));
                record.insert("active".to_string(), Value::Bool(i % 2 == 0));
                record.insert(
                    "payload".to_string(),
                    Value::String(format!("payload-{:04}", i * 7)),
                );
                record
            })
            .collect();
        let original = records.clone();

        let sequential_request = CompressRequest {
            input: InputSource::Iterator(Box::new(records.clone().into_iter())),
            output: OutputSink::Path(seq_path.clone()),
            options: options.clone(),
            container_hint: Some(ContainerFormat::Ndjson),
            emit_index: true,
            wrapper_config: WrapperConfig::None,
        };
        execute_compress_sequential(sequential_request).unwrap();

        let parallel_request = CompressRequest {
            input: InputSource::Iterator(Box::new(records.into_iter())),
            output: OutputSink::Path(par_path.clone()),
            options,
            container_hint: Some(ContainerFormat::Ndjson),
            emit_index: true,
            wrapper_config: WrapperConfig::None,
        };
        crate::parallel::execute_compress_parallel(parallel_request, 2).unwrap();

        let seq_bytes = fs::read(&seq_path).unwrap();
        let par_bytes = fs::read(&par_path).unwrap();

        let seq_records = read_records(&seq_bytes);
        let par_records = read_records(&par_bytes);

        assert_eq!(seq_records, original);
        assert_eq!(par_records, original);
        assert_eq!(seq_records, par_records);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn parallel_pipeline_propagates_limit_errors() {
        let mut options = CompressOptions::default();
        options.block_target_records = 4;
        options.default_codec = Codec::None;
        options.limits.max_segment_uncompressed_len = 8;

        let mut record = Map::new();
        record.insert(
            "field".to_string(),
            Value::String("abcdefghijk".to_string()),
        );
        let records = vec![record.clone()];

        let sequential_request = CompressRequest {
            input: InputSource::Iterator(Box::new(records.clone().into_iter())),
            output: OutputSink::Writer(Box::new(Vec::new())),
            options: options.clone(),
            container_hint: Some(ContainerFormat::Ndjson),
            emit_index: false,
            wrapper_config: WrapperConfig::None,
        };

        let seq_error = execute_compress_sequential(sequential_request).unwrap_err();
        match seq_error {
            JacError::LimitExceeded(_) => {}
            other => panic!("expected limit exceeded error, got {:?}", other),
        }

        let parallel_request = CompressRequest {
            input: InputSource::Iterator(Box::new(vec![record].into_iter())),
            output: OutputSink::Writer(Box::new(Vec::new())),
            options,
            container_hint: Some(ContainerFormat::Ndjson),
            emit_index: false,
            wrapper_config: WrapperConfig::None,
        };

        let par_error =
            crate::parallel::execute_compress_parallel(parallel_request, 2).unwrap_err();
        match par_error {
            JacError::LimitExceeded(_) => {}
            other => panic!("expected parallel limit exceeded error, got {:?}", other),
        }
    }

    #[test]
    fn jac_reader_applies_header_segment_limit() {
        use std::io::Cursor;

        let mut options = CompressOptions::default();
        options.default_codec = Codec::None;
        options.limits.max_segment_uncompressed_len = 128;

        let mut header = FileHeader {
            flags: 0,
            default_compressor: options.default_codec.compressor_id(),
            default_compression_level: options.default_codec.level(),
            block_size_hint_records: options.block_target_records,
            user_metadata: encode_header_metadata(&options.limits).unwrap(),
        };
        header.set_container_format_hint(ContainerFormat::Ndjson);

        let codec_opts = CompressOpts {
            block_target_records: options.block_target_records,
            default_codec: options.default_codec,
            canonicalize_keys: options.canonicalize_keys,
            canonicalize_numbers: options.canonicalize_numbers,
            nested_opaque: options.nested_opaque,
            max_dict_entries: options.max_dict_entries,
            limits: options.limits,
        };

        let mut writer = JacWriter::new(Vec::new(), header, codec_opts).unwrap();
        let mut record = Map::new();
        record.insert("value".to_string(), Value::from(1));
        writer.write_record(&record).unwrap();
        let WriterFinish { writer: bytes, .. } = writer.finish_without_index().unwrap();

        let reader = JacReader::new(Cursor::new(bytes.clone()), DecompressOpts::default()).unwrap();
        assert_eq!(reader.limits().max_segment_uncompressed_len, 128);

        let mut custom_opts = DecompressOpts::default();
        custom_opts.limits.max_segment_uncompressed_len = 32;
        let reader_custom = JacReader::new(Cursor::new(bytes), custom_opts).unwrap();
        assert_eq!(reader_custom.limits().max_segment_uncompressed_len, 32);
    }

    #[test]
    fn json_array_nested_input_roundtrip_preserves_nested_objects() {
        let dir = tempdir().unwrap();
        let input_path = dir.path().join("nested.json");
        let jac_path = dir.path().join("nested.jac");
        let output_path = dir.path().join("nested_out.json");

        let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../testdata/json_array_nested_repro.json");
        let fixture = fs::read_to_string(&fixture_path).unwrap_or_else(|err| {
            panic!("failed to read fixture {}: {err}", fixture_path.display())
        });

        fs::write(&input_path, &fixture).unwrap();

        let compress_request = CompressRequest {
            input: InputSource::JsonArrayPath(input_path.clone()),
            output: OutputSink::Path(jac_path.clone()),
            options: CompressOptions::default(),
            container_hint: Some(ContainerFormat::JsonArray),
            emit_index: true,
            wrapper_config: WrapperConfig::None,
        };
        execute_compress(compress_request).unwrap();

        let decompress_request = DecompressRequest {
            input: JacInput::Path(jac_path.clone()),
            output: OutputSink::Path(output_path.clone()),
            format: DecompressFormat::JsonArray,
            options: DecompressOptions::default(),
        };
        execute_decompress(decompress_request).unwrap();

        let decoded = fs::read_to_string(&output_path).unwrap();
        let original_json: Value =
            serde_json::from_str(&fixture).expect("fixture should be valid JSON array");
        let roundtrip_json: Value =
            serde_json::from_str(&decoded).expect("round-trip output should parse as JSON");

        assert_eq!(
            roundtrip_json, original_json,
            "round-trip JSON array should preserve nested objects"
        );
    }

    #[cfg(feature = "async")]
    mod async_tests {
        use super::*;

        #[tokio::test]
        async fn async_compress_and_decompress_roundtrip() {
            let data = "{\"id\":1}\n{\"id\":2}\n";
            let paths = TempPaths::new("async_roundtrip");

            fs::write(&paths.input_ndjson, data).unwrap();

            let compress_request = CompressRequest {
                input: InputSource::NdjsonPath(paths.input_ndjson.clone()),
                output: OutputSink::Path(paths.output_jac.clone()),
                options: CompressOptions::default(),
                container_hint: None,
                emit_index: true,
                wrapper_config: WrapperConfig::None,
            };

            super::async_io::compress(compress_request)
                .await
                .expect("async compress");

            let decompress_request = DecompressRequest {
                input: JacInput::Path(paths.output_jac.clone()),
                output: OutputSink::Path(paths.output_json.clone()),
                format: DecompressFormat::Auto,
                options: DecompressOptions::default(),
            };

            super::async_io::decompress(decompress_request)
                .await
                .expect("async decompress");

            let result = fs::read_to_string(&paths.output_json).unwrap();
            assert_eq!(normalize_ndjson(&result), normalize_ndjson(data));
        }
    }

    struct TempPaths {
        input_ndjson: PathBuf,
        output_jac: PathBuf,
        output_json: PathBuf,
    }

    impl TempPaths {
        fn new(label: &str) -> Self {
            let base = std::env::temp_dir();
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let input_ndjson = base.join(format!("{}_{}_input.ndjson", label, unique));
            let output_jac = base.join(format!("{}_{}_output.jac", label, unique));
            let output_json = base.join(format!("{}_{}_output.ndjson", label, unique));
            Self {
                input_ndjson,
                output_jac,
                output_json,
            }
        }
    }

    impl Drop for TempPaths {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.input_ndjson);
            let _ = fs::remove_file(&self.output_jac);
            let _ = fs::remove_file(&self.output_json);
        }
    }

    fn normalize_ndjson(input: &str) -> Vec<String> {
        input
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| line.trim().to_string())
            .collect()
    }
}
