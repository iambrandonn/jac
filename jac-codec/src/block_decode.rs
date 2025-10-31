//! Block decoder and projection utilities

use std::collections::HashMap;
use std::convert::TryInto;

use jac_format::{checksum::compute_crc32c, BlockHeader, JacError, Limits, Result};
use serde_json::{self, Map, Value};

use crate::segment_decode::FieldSegmentDecoder;

/// Decoder options controlling limits and validation
#[derive(Debug, Clone)]
pub struct DecompressOpts {
    /// Security limits enforced during decoding
    pub limits: Limits,
    /// Verify block CRC32C (recommended)
    pub verify_checksums: bool,
}

impl Default for DecompressOpts {
    fn default() -> Self {
        Self {
            limits: Limits::default(),
            verify_checksums: true,
        }
    }
}

/// Block decoder capable of reconstructing records or projecting individual fields
#[derive(Debug)]
pub struct BlockDecoder {
    header: BlockHeader,
    segments: Vec<Vec<u8>>,
    field_index: HashMap<String, usize>,
    opts: DecompressOpts,
}

impl BlockDecoder {
    /// Create a new block decoder from raw block bytes
    pub fn new(block_bytes: &[u8], opts: &DecompressOpts) -> Result<Self> {
        if block_bytes.len() < 4 {
            return Err(JacError::UnexpectedEof);
        }

        let crc_offset = block_bytes.len() - 4;
        let stored_crc = u32::from_le_bytes(block_bytes[crc_offset..].try_into().unwrap());

        if opts.verify_checksums {
            let computed_crc = compute_crc32c(&block_bytes[..crc_offset]);
            if computed_crc != stored_crc {
                return Err(JacError::ChecksumMismatch);
            }
        }

        // Decode block header (enforces per-field limits)
        let (header, header_len) = BlockHeader::decode(block_bytes, &opts.limits)?;

        if header_len > crc_offset {
            return Err(JacError::CorruptBlock);
        }

        // Enforce total uncompressed limit
        let mut total_uncompressed = 0usize;
        for field in &header.fields {
            total_uncompressed = total_uncompressed
                .checked_add(field.segment_uncompressed_len)
                .ok_or_else(|| {
                    JacError::LimitExceeded("Total uncompressed size overflow".to_string())
                })?;
        }
        if total_uncompressed > opts.limits.max_block_uncompressed_total {
            return Err(JacError::LimitExceeded(format!(
                "Block uncompressed total {} exceeds limit {}",
                total_uncompressed, opts.limits.max_block_uncompressed_total
            )));
        }

        let segments_region_start = header_len;
        let segments_region_end = crc_offset;
        let segments_region_len = segments_region_end - segments_region_start;

        // Validate segment layout (offsets, contiguity, bounds)
        let mut sorted_indices: Vec<usize> = (0..header.fields.len()).collect();
        sorted_indices.sort_by_key(|&idx| header.fields[idx].segment_offset);

        let mut expected_offset = 0usize;
        for &idx in &sorted_indices {
            let field = &header.fields[idx];
            if field.segment_offset != expected_offset {
                return Err(JacError::CorruptBlock);
            }

            let start = segments_region_start
                .checked_add(field.segment_offset)
                .ok_or_else(|| JacError::CorruptBlock)?;
            let end = start
                .checked_add(field.segment_compressed_len)
                .ok_or_else(|| JacError::CorruptBlock)?;

            if end > segments_region_end {
                return Err(JacError::CorruptBlock);
            }

            expected_offset = field
                .segment_offset
                .checked_add(field.segment_compressed_len)
                .ok_or_else(|| JacError::CorruptBlock)?;
        }

        if expected_offset != segments_region_len {
            return Err(JacError::CorruptBlock);
        }

        // Materialize segment bytes in header order for later decoding
        let mut segments = Vec::with_capacity(header.fields.len());
        for field in &header.fields {
            let start = segments_region_start + field.segment_offset;
            let end = start + field.segment_compressed_len;
            if end > segments_region_end {
                return Err(JacError::CorruptBlock);
            }
            segments.push(block_bytes[start..end].to_vec());
        }

        let mut field_index = HashMap::new();
        for (idx, field) in header.fields.iter().enumerate() {
            field_index.insert(field.field_name.clone(), idx);
        }

        Ok(Self {
            header,
            segments,
            field_index,
            opts: opts.clone(),
        })
    }

    /// Decode all records in the block into JSON maps
    pub fn decode_records(&self) -> Result<Vec<Map<String, Value>>> {
        let record_count = self.header.record_count;
        let mut records = vec![Map::new(); record_count];

        for (idx, entry) in self.header.fields.iter().enumerate() {
            let decoder = FieldSegmentDecoder::new(
                &self.segments[idx],
                entry,
                record_count,
                &self.opts.limits,
            )?;

            for (record_idx, record) in records.iter_mut().enumerate() {
                if let Some(value) = decoder.get_value(record_idx)? {
                    record.insert(entry.field_name.clone(), value);
                }
            }
        }

        Ok(records)
    }

    /// Project a single field across all records
    pub fn project_field(&self, field_name: &str) -> Result<Vec<Option<Value>>> {
        let record_count = self.header.record_count;
        let Some(&idx) = self.field_index.get(field_name) else {
            return Ok(vec![None; record_count]);
        };

        let decoder = FieldSegmentDecoder::new(
            &self.segments[idx],
            &self.header.fields[idx],
            record_count,
            &self.opts.limits,
        )?;

        (0..record_count)
            .map(|idx| decoder.get_value(idx))
            .collect::<Result<Vec<Option<Value>>>>()
    }

    /// Access the block header
    pub fn header(&self) -> &BlockHeader {
        &self.header
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        block_builder::{BlockBuilder, BlockData},
        Codec, CompressOpts, TryAddRecordOutcome,
    };
    use serde_json::{json, Map, Value};
    use std::{fs, path::PathBuf};

    fn block_data_from_records(
        mut opts: CompressOpts,
        records: &[Map<String, Value>],
    ) -> BlockData {
        opts.block_target_records = records.len().max(1);
        let mut builder = BlockBuilder::new(opts);
        for record in records {
            match builder
                .try_add_record(record.clone())
                .expect("try add record")
            {
                TryAddRecordOutcome::Added => {}
                TryAddRecordOutcome::BlockFull { .. } => {
                    panic!("unexpected block flush in decoder test helper")
                }
            }
        }
        builder.finalize().unwrap().data
    }

    fn assemble_bytes(data: &crate::block_builder::BlockData) -> Vec<u8> {
        let header_bytes = data.header.encode().unwrap();
        let mut bytes = header_bytes;
        for segment in &data.segments {
            bytes.extend_from_slice(segment);
        }
        bytes.extend_from_slice(&data.crc32c.to_le_bytes());
        bytes
    }

    fn default_records() -> Vec<Map<String, Value>> {
        vec![
            serde_json::from_value::<Map<String, Value>>(json!({"id": 1, "name": "alice"}))
                .unwrap(),
            serde_json::from_value::<Map<String, Value>>(json!({"id": 2, "name": "bob"})).unwrap(),
            serde_json::from_value::<Map<String, Value>>(json!({"id": 3})).unwrap(),
        ]
    }

    #[test]
    fn test_block_decoder_roundtrip() {
        let records = default_records();
        let data = block_data_from_records(CompressOpts::default(), &records);
        let bytes = assemble_bytes(&data);
        let opts = DecompressOpts::default();

        let decoder = BlockDecoder::new(&bytes, &opts).unwrap();
        let decoded = decoder.decode_records().unwrap();

        assert_eq!(decoded.len(), records.len());
        assert_eq!(decoded[0]["name"], json!("alice"));
        assert_eq!(decoded[2].get("name"), None);
    }

    #[test]
    fn test_block_decoder_project_field() {
        let records = default_records();
        let data = block_data_from_records(CompressOpts::default(), &records);
        let bytes = assemble_bytes(&data);

        let decoder = BlockDecoder::new(&bytes, &DecompressOpts::default()).unwrap();
        let values = decoder.project_field("name").unwrap();

        assert_eq!(values.len(), records.len());
        assert_eq!(values[0], Some(json!("alice")));
        assert_eq!(values[2], None);

        let missing = decoder.project_field("missing").unwrap();
        assert_eq!(missing, vec![None, None, None]);
    }

    #[test]
    fn test_block_decoder_crc_verification() {
        let records = default_records();
        let data = block_data_from_records(CompressOpts::default(), &records);
        let mut bytes = assemble_bytes(&data);

        // Corrupt CRC
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;

        let err = BlockDecoder::new(&bytes, &DecompressOpts::default()).unwrap_err();
        assert!(matches!(err, JacError::ChecksumMismatch));

        let mut opts = DecompressOpts::default();
        opts.verify_checksums = false;
        BlockDecoder::new(&bytes, &opts).unwrap();
    }

    #[test]
    fn test_block_decoder_layout_validation() {
        let records = default_records();
        let data = block_data_from_records(CompressOpts::default(), &records);
        let mut bytes = assemble_bytes(&data);

        // Truncate part of the data region to trigger CorruptBlock
        if bytes.len() > 10 {
            bytes.truncate(bytes.len() - 10);
        } else {
            bytes.truncate(bytes.len() / 2);
        }

        let err = BlockDecoder::new(&bytes, &DecompressOpts::default()).unwrap_err();
        assert!(matches!(
            err,
            JacError::CorruptBlock | JacError::UnexpectedEof | JacError::ChecksumMismatch
        ));
    }

    #[test]
    fn test_block_decoder_block_limit() {
        let mut encode_opts = CompressOpts::default();
        encode_opts.default_codec = Codec::None;

        let records =
            vec![
                serde_json::from_value::<Map<String, Value>>(json!({"blob": "abcdefghij"}))
                    .unwrap(),
            ];
        let data = block_data_from_records(encode_opts, &records);
        let bytes = assemble_bytes(&data);

        let mut dec_opts = DecompressOpts::default();
        dec_opts.limits.max_block_uncompressed_total = 5; // Smaller than actual segment payload

        let err = BlockDecoder::new(&bytes, &dec_opts).unwrap_err();
        assert!(matches!(err, JacError::LimitExceeded(_)));
    }

    #[test]
    fn test_block_decoder_conformance_vector() {
        let conformance_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("testdata")
            .join("spec")
            .join("v12_1.jsonl");
        let data =
            fs::read_to_string(conformance_path).expect("failed to read conformance fixture");

        let mut records = Vec::new();
        for line in data.lines().filter(|line| !line.trim().is_empty()) {
            let value: Value = serde_json::from_str(line).expect("invalid JSON record");
            records.push(value.as_object().unwrap().clone());
        }

        let block_data = block_data_from_records(CompressOpts::default(), &records);
        let bytes = assemble_bytes(&block_data);

        let decoder = BlockDecoder::new(&bytes, &DecompressOpts::default()).unwrap();
        let decoded_records = decoder.decode_records().unwrap();

        let original_values: Vec<Value> = records.iter().cloned().map(Value::Object).collect();
        let decoded_values: Vec<Value> = decoded_records.into_iter().map(Value::Object).collect();
        assert_eq!(decoded_values, original_values);

        let projected = decoder.project_field("user").unwrap();
        let expected_users = ["alice", "alice", "bob", "carol"];
        assert_eq!(projected.len(), expected_users.len());
        for (value, expected) in projected.iter().zip(expected_users.iter()) {
            assert_eq!(value, &Some(Value::String((*expected).to_string())));
        }
    }
}
