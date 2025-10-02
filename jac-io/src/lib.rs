//! JAC I/O - Streaming file I/O and high-level APIs
//!
//! This crate provides the file I/O layer and high-level APIs for JAC:
//!
//! - Streaming writers and readers
//! - High-level compression/decompression functions
//! - Parallel processing support
//! - Field projection APIs

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod parallel;
pub mod reader;
pub mod writer;

// Re-export commonly used types
pub use jac_codec::{Codec, CompressOpts, DecompressOpts};
pub use jac_format::{FileHeader, JacError, Limits, Result, TypeTag};
pub use reader::{BlockHandle, FieldIterator, JacReader};
pub use writer::JacWriter;

use serde_json::{Map, Value};
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, Write};

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
            nested_opaque: true, // Must be true in v1
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

/// Compress JSON input to JAC format
pub fn compress<R: Read, W: Write>(input: R, output: W, opts: CompressOptions) -> Result<()> {
    // Create file header
    let mut flags = 0u32;
    if opts.canonicalize_keys {
        flags |= jac_format::constants::FLAG_CANONICALIZE_KEYS;
    }
    if opts.canonicalize_numbers {
        flags |= jac_format::constants::FLAG_CANONICALIZE_NUMBERS;
    }
    if opts.nested_opaque {
        flags |= jac_format::constants::FLAG_NESTED_OPAQUE;
    }

    let header = FileHeader {
        flags,
        default_compressor: opts.default_codec.compressor_id(),
        default_compression_level: opts.default_codec.level(),
        block_size_hint_records: opts.block_target_records,
        user_metadata: Vec::new(),
    };

    // Convert to codec options
    let codec_opts = CompressOpts {
        block_target_records: opts.block_target_records,
        default_codec: opts.default_codec,
        canonicalize_keys: opts.canonicalize_keys,
        canonicalize_numbers: opts.canonicalize_numbers,
        nested_opaque: opts.nested_opaque,
        max_dict_entries: opts.max_dict_entries,
        limits: opts.limits,
    };

    // Create writer
    let mut writer = JacWriter::new(output, header, codec_opts)?;

    // Detect input format and stream records
    let mut reader = BufReader::new(input);

    // Read all input as NDJSON (one JSON object per line)
    let mut line = String::new();
    while reader.read_line(&mut line)? > 0 {
        if !line.trim().is_empty() {
            let record: Map<String, Value> = serde_json::from_str(&line)?;
            writer.write_record(&record)?;
        }
        line.clear();
    }

    // Finish writing
    let _ = writer.finish(true)?; // Include index
    Ok(())
}

/// Decompress JAC file to full JSON output
pub fn decompress_full<R: Read + Seek, W: Write>(
    input: R,
    output: W,
    opts: DecompressOptions,
) -> Result<()> {
    let codec_opts = DecompressOpts {
        limits: opts.limits.clone(),
        verify_checksums: opts.verify_checksums,
    };

    let mut reader = JacReader::new(input, codec_opts)?;
    let mut writer = BufWriter::new(output);

    // Collect all blocks first to avoid borrowing issues
    let mut blocks = Vec::new();
    for block_result in reader.blocks() {
        blocks.push(block_result?);
    }

    // Write NDJSON output
    for block in blocks {
        let block_decoder = reader.decode_block(&block)?;
        let records = block_decoder.decode_records()?;

        // Write each record as NDJSON
        for record in records {
            writeln!(writer, "{}", serde_json::to_string(&record)?)?;
        }
    }

    writer.flush()?;
    Ok(())
}

/// Project specific fields from JAC file
pub fn project<R: Read + Seek, W: Write>(
    input: R,
    output: W,
    fields: &[&str],
    as_ndjson: bool,
) -> Result<()> {
    let opts = DecompressOptions::default();
    let codec_opts = DecompressOpts {
        limits: opts.limits.clone(),
        verify_checksums: opts.verify_checksums,
    };

    let mut reader = JacReader::new(input, codec_opts)?;
    let mut writer = BufWriter::new(output);

    // Collect all blocks first to avoid borrowing issues
    let mut blocks = Vec::new();
    for block_result in reader.blocks() {
        blocks.push(block_result?);
    }

    let mut wrote_any_record = false;

    if !as_ndjson {
        writer.write_all(b"[")?;
    }

    for block in blocks {
        let record_count = block.record_count;

        // Collect projected values for each requested field
        let mut field_values: Vec<Vec<Option<Value>>> = Vec::with_capacity(fields.len());
        for field_name in fields {
            let values = reader
                .project_field(&block, field_name)?
                .collect::<Result<Vec<_>>>()?;

            if values.len() != record_count {
                return Err(JacError::Internal(format!(
                    "Projected field '{}' returned {} values (expected {})",
                    field_name,
                    values.len(),
                    record_count
                )));
            }

            field_values.push(values);
        }

        for record_idx in 0..record_count {
            let mut record_map = Map::new();

            for (field_name, values) in fields.iter().zip(field_values.iter()) {
                if let Some(value) = values[record_idx].clone() {
                    record_map.insert((*field_name).to_string(), value);
                }
            }

            let record_value = Value::Object(record_map);

            if as_ndjson {
                serde_json::to_writer(&mut writer, &record_value)?;
                writer.write_all(b"\n")?;
            } else {
                if wrote_any_record {
                    writer.write_all(b",")?;
                } else {
                    wrote_any_record = true;
                }
                serde_json::to_writer(&mut writer, &record_value)?;
            }
        }
    }

    if !as_ndjson {
        writer.write_all(b"]")?;
    }

    writer.flush()?;
    Ok(())
}
