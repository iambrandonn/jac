//! Parallel processing support

use jac_codec::{
    BlockBuilder, BlockData, BlockFinish, CompressOpts, DecompressOpts, TryAddRecordOutcome,
};
use jac_format::{FileHeader, JacError, Result};
use rayon::prelude::*;
use serde_json::Map;
use serde_json::Value;
use std::io::{Read, Write};

/// Compress blocks in parallel using `rayon`.
pub fn compress_blocks_parallel(
    records: Vec<Map<String, Value>>,
    opts: &CompressOpts,
) -> Result<Vec<BlockFinish>> {
    let block_size = opts.block_target_records;
    let chunks: Vec<Vec<Map<String, Value>>> = records
        .chunks(block_size)
        .map(|chunk| chunk.to_vec())
        .collect();

    chunks
        .into_par_iter()
        .map(|chunk| {
            let mut builder = BlockBuilder::new(opts.clone());
            for record in chunk {
                match builder.try_add_record(record)? {
                    TryAddRecordOutcome::Added => {}
                    TryAddRecordOutcome::BlockFull { record } => {
                        return Err(JacError::Internal(format!(
                            "Block limit exceeded while compressing record with fields: {}",
                            record.keys().cloned().collect::<Vec<_>>().join(", ")
                        )));
                    }
                }
            }
            builder.finalize()
        })
        .collect()
}

/// Decompress blocks in parallel (placeholder implementation).
pub fn decompress_blocks_parallel(
    block_data: Vec<BlockData>,
    _opts: &DecompressOpts,
) -> Result<Vec<Vec<Map<String, Value>>>> {
    block_data
        .into_par_iter()
        .map(|data| {
            // This would need the full BlockDecoder implementation
            // For now, return empty records
            Ok(vec![Map::new(); data.header.record_count])
        })
        .collect()
}

/// Parallel file compression
pub fn compress_parallel<R: Read, W: Write>(
    input: R,
    _output: W,
    opts: crate::CompressOptions,
) -> Result<()> {
    // Read all records first (for parallel processing)
    let records = read_all_records(input)?;

    // Convert to codec options
    let codec_opts = CompressOpts {
        block_target_records: opts.block_target_records,
        default_codec: opts.default_codec,
        canonicalize_keys: opts.canonicalize_keys,
        canonicalize_numbers: opts.canonicalize_numbers,
        nested_opaque: opts.nested_opaque,
        max_dict_entries: opts.max_dict_entries,
        limits: opts.limits.clone(),
    };

    // Compress blocks in parallel
    let _block_data = compress_blocks_parallel(records, &codec_opts)?;

    // Write file header
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
        user_metadata: super::encode_header_metadata(&opts.limits)?,
    };

    let _header_bytes = header.encode()?;
    // Note: In a real implementation, we'd need to write to the output
    // For now, this is a placeholder

    Ok(())
}

/// Read all records from input
fn read_all_records<R: Read>(input: R) -> Result<Vec<Map<String, Value>>> {
    use std::io::BufRead;
    let reader = std::io::BufReader::new(input);
    let mut records = Vec::new();

    for line in reader.lines() {
        let line = line?;
        if !line.trim().is_empty() {
            let record: Map<String, Value> = serde_json::from_str(&line)?;
            records.push(record);
        }
    }

    Ok(records)
}
