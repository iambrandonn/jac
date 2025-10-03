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
pub mod writer;

// Re-export commonly used types
pub use jac_codec::{Codec, CompressOpts, DecompressOpts};
pub use jac_format::{FileHeader, JacError, Limits, Result, TypeTag};
use reader::BlockCursor;
pub use reader::{
    BlockHandle, FieldIterator, JacReader, ProjectionStream, RecordStream as ReaderRecordStream,
};
pub use writer::{JacWriter, WriterFinish, WriterMetrics};

use serde::Deserialize;
use serde_json::{Map, Value};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, Write};
use std::path::PathBuf;

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
    /// Emit index footer/pointer when finishing.
    pub emit_index: bool,
}

impl Default for CompressRequest {
    fn default() -> Self {
        Self {
            input: InputSource::Iterator(Box::new(std::iter::empty())),
            output: OutputSink::Writer(Box::new(Vec::new())),
            options: CompressOptions::default(),
            emit_index: true,
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
#[derive(Debug, Clone, Copy)]
pub struct CompressSummary {
    /// Metrics produced by the writer.
    pub metrics: WriterMetrics,
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

/// Execute a compression request.
pub fn execute_compress(request: CompressRequest) -> Result<CompressSummary> {
    let CompressRequest {
        input,
        output,
        options,
        emit_index,
    } = request;

    let stream = input.into_record_stream()?;
    let writer_target = output.into_writer()?;
    let buf_writer = BufWriter::new(writer_target);

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

    let header = FileHeader {
        flags,
        default_compressor: options.default_codec.compressor_id(),
        default_compression_level: options.default_codec.level(),
        block_size_hint_records: options.block_target_records,
        user_metadata: Vec::new(),
    };

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

    Ok(CompressSummary {
        metrics: finish.metrics,
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

    let mut buf_writer = BufWriter::new(output.into_writer()?);
    let mut record_stream = reader.record_stream()?;
    let mut summary = DecompressSummary {
        records_written: 0,
        blocks_processed: 0,
    };

    match format {
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
        emit_index: true,
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
    fn into_record_stream(self) -> Result<RecordStream> {
        match self {
            InputSource::NdjsonPath(path) => {
                let file = File::open(path)?;
                Ok(RecordStream::ndjson(BufReader::new(file)))
            }
            InputSource::JsonArrayPath(path) => {
                let file = File::open(path)?;
                RecordStream::json_array_reader(BufReader::new(file))
            }
            InputSource::NdjsonReader(reader) => Ok(RecordStream::ndjson(BufReader::new(reader))),
            InputSource::JsonArrayReader(reader) => {
                RecordStream::json_array_reader(BufReader::new(reader))
            }
            InputSource::Iterator(iter) => Ok(RecordStream::iter(iter)),
        }
    }
}

impl OutputSink {
    fn into_writer(self) -> Result<Box<dyn WriteSend>> {
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
}

enum RecordStreamInner {
    Ndjson(NdjsonStream),
    JsonArray(JsonArrayStream),
    Iterator(Box<dyn Iterator<Item = Map<String, Value>> + Send>),
}

impl RecordStream {
    fn ndjson<R: BufRead + Send + 'static>(reader: R) -> Self {
        Self {
            inner: RecordStreamInner::Ndjson(NdjsonStream {
                reader: Box::new(reader),
                buffer: String::new(),
            }),
        }
    }

    fn json_array_reader<R: BufRead + Send + 'static>(reader: R) -> Result<Self> {
        let stream = JsonArrayStream::from_reader(Box::new(reader))?;
        Ok(Self {
            inner: RecordStreamInner::JsonArray(stream),
        })
    }

    fn iter(iter: Box<dyn Iterator<Item = Map<String, Value>> + Send>) -> Self {
        Self {
            inner: RecordStreamInner::Iterator(iter),
        }
    }
}

impl Iterator for RecordStream {
    type Item = Result<Map<String, Value>>;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.inner {
            RecordStreamInner::Ndjson(stream) => stream.next(),
            RecordStreamInner::JsonArray(stream) => stream.next(),
            RecordStreamInner::Iterator(iter) => iter.next().map(Ok),
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
    finished: bool,
}

impl JsonArrayStream {
    fn from_reader(reader: Box<dyn BufRead + Send>) -> Result<Self> {
        let mut stream = Self {
            reader,
            finished: false,
        };
        stream.consume_array_start()?;
        Ok(stream)
    }

    fn consume_array_start(&mut self) -> Result<()> {
        self.skip_whitespace()?;
        match self.read_byte()? {
            Some(b'[') => Ok(()),
            Some(_) => Err(JacError::TypeMismatch),
            None => Err(JacError::UnexpectedEof),
        }
    }

    fn skip_whitespace(&mut self) -> Result<()> {
        loop {
            let buf = self.reader.fill_buf()?;
            if buf.is_empty() {
                return Ok(());
            }
            let mut consumed = 0;
            while consumed < buf.len() && buf[consumed].is_ascii_whitespace() {
                consumed += 1;
            }
            let has_more = consumed < buf.len();
            if consumed > 0 {
                self.reader.consume(consumed);
            }
            if has_more {
                return Ok(());
            }
        }
    }

    fn peek_byte(&mut self) -> Result<Option<u8>> {
        let buf = self.reader.fill_buf()?;
        if buf.is_empty() {
            return Ok(None);
        }
        Ok(Some(buf[0]))
    }

    fn read_byte(&mut self) -> Result<Option<u8>> {
        let buf = self.reader.fill_buf()?;
        if buf.is_empty() {
            return Ok(None);
        }
        let byte = buf[0];
        self.reader.consume(1);
        Ok(Some(byte))
    }

    fn read_next_map(&mut self) -> Result<Option<Map<String, Value>>> {
        self.skip_whitespace()?;
        match self.peek_byte()? {
            Some(b']') => {
                self.read_byte()?; // consume closing bracket
                self.finished = true;
                self.skip_whitespace()?;
                Ok(None)
            }
            Some(_) => {
                let map = self.read_map_value()?;
                Ok(Some(map))
            }
            None => Err(JacError::UnexpectedEof),
        }
    }

    fn read_map_value(&mut self) -> Result<Map<String, Value>> {
        let mut buf = Vec::new();
        loop {
            let byte = self.read_byte()?.ok_or_else(|| JacError::UnexpectedEof)?;
            buf.push(byte);

            let mut de = serde_json::Deserializer::from_slice(&buf);
            match Map::<String, Value>::deserialize(&mut de) {
                Ok(map) => {
                    if let Err(err) = de.end() {
                        if matches!(err.classify(), serde_json::error::Category::Eof) {
                            continue;
                        }
                        return Err(JacError::from(err));
                    }

                    self.skip_whitespace()?;
                    match self.peek_byte()? {
                        Some(b',') => {
                            self.read_byte()?; // consume comma
                        }
                        Some(b']') => {
                            self.read_byte()?; // consume closing bracket
                            self.finished = true;
                            self.skip_whitespace()?;
                        }
                        Some(_) => return Err(JacError::TypeMismatch),
                        None => return Err(JacError::UnexpectedEof),
                    }
                    return Ok(map);
                }
                Err(err) => match err.classify() {
                    serde_json::error::Category::Eof => continue,
                    _ => return Err(JacError::from(err)),
                },
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
        match self.read_next_map() {
            Ok(Some(map)) => Some(Ok(map)),
            Ok(None) => None,
            Err(err) => {
                self.finished = true;
                Some(Err(err))
            }
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
    use serde_json::json;
    use std::fs;
    use std::io::Cursor;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn ndjson_input_streams_records() {
        let data = "{\"a\":1}\n{\"b\":2}\n";
        let mut stream = InputSource::NdjsonReader(Box::new(data.as_bytes()))
            .into_record_stream()
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
            .into_record_stream()
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
            .into_record_stream()
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
            emit_index: true,
        };

        execute_compress(compress_request).unwrap();

        let decompress_request = DecompressRequest {
            input: JacInput::Path(paths.output_jac.clone()),
            output: OutputSink::Path(paths.output_json.clone()),
            format: DecompressFormat::Ndjson,
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
            emit_index: true,
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
                emit_index: true,
            };

            super::async_io::compress(compress_request)
                .await
                .expect("async compress");

            let decompress_request = DecompressRequest {
                input: JacInput::Path(paths.output_jac.clone()),
                output: OutputSink::Path(paths.output_json.clone()),
                format: DecompressFormat::Ndjson,
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
