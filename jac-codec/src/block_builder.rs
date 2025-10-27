//! Block builder for aggregating columns

use crate::{column::ColumnContribution, ColumnBuilder, CompressOpts};
use jac_format::{checksum::compute_crc32c, BlockHeader, FieldDirectoryEntry, JacError, Result};
use serde_json;
use std::collections::HashMap;

/// Block builder for aggregating records into a block
pub struct BlockBuilder {
    /// Compression options
    opts: CompressOpts,
    /// Records in this block
    records: Vec<serde_json::Map<String, serde_json::Value>>,
    /// Field names discovered across all records
    field_names: Vec<String>,
    /// Column builders for each field
    column_builders: HashMap<String, ColumnBuilder>,
    /// Current memory usage estimate
    estimated_memory: usize,
    /// Number of times segment limit forced an early flush
    segment_limit_flushes: usize,
    /// Number of times a single record exceeded segment limit
    segment_limit_record_rejections: usize,
}

/// Result of attempting to add a record to the current block.
#[derive(Debug)]
pub enum TryAddRecordOutcome {
    /// Record was added successfully to the current block.
    Added,
    /// Current block must be flushed before adding the record; contains the original record.
    BlockFull {
        /// Record that needs to be retried after flushing the current block.
        record: serde_json::Map<String, serde_json::Value>,
    },
}

impl BlockBuilder {
    /// Create new block builder
    pub fn new(opts: CompressOpts) -> Self {
        Self {
            opts,
            records: Vec::new(),
            field_names: Vec::new(),
            column_builders: HashMap::new(),
            estimated_memory: 0,
            segment_limit_flushes: 0,
            segment_limit_record_rejections: 0,
        }
    }

    /// Attempt to add a record to the block, returning whether it fit or requires flushing.
    pub fn try_add_record(
        &mut self,
        rec: serde_json::Map<String, serde_json::Value>,
    ) -> Result<TryAddRecordOutcome> {
        // Quick check for target record count or block memory before deeper analysis.
        if self.is_full() {
            return Ok(TryAddRecordOutcome::BlockFull { record: rec });
        }

        let limits = &self.opts.limits;
        let max_segment_len = limits.max_segment_uncompressed_len;
        let current_record_count = self.records.len();
        let next_record_count = current_record_count + 1;

        let record = rec;
        let mut existing_contribs: HashMap<String, ColumnContribution> = HashMap::new();
        let mut new_field_contribs: HashMap<String, ColumnContribution> = HashMap::new();

        // Precompute contributions for fields present in this record.
        for (field_name, value) in &record {
            if let Some(builder) = self.column_builders.get(field_name) {
                let contrib = builder.contribution_for_value(value)?;
                let single_upper = builder.estimated_single_value_upper_bound(value)?;
                if single_upper > max_segment_len {
                    self.segment_limit_record_rejections += 1;
                    return Err(JacError::LimitExceeded(format!(
                        "Field '{}' single-record payload ({} bytes) exceeds max_segment_uncompressed_len ({})",
                        field_name, single_upper, max_segment_len
                    )));
                }
                existing_contribs.insert(field_name.clone(), contrib);
            } else {
                let temp_builder = ColumnBuilder::new(self.opts.block_target_records, &self.opts);
                let contrib = temp_builder.contribution_for_value(value)?;
                let single_upper = temp_builder.estimated_single_value_upper_bound(value)?;
                if single_upper > max_segment_len {
                    self.segment_limit_record_rejections += 1;
                    return Err(JacError::LimitExceeded(format!(
                        "Field '{}' single-record payload ({} bytes) exceeds max_segment_uncompressed_len ({})",
                        field_name, single_upper, max_segment_len
                    )));
                }
                new_field_contribs.insert(field_name.clone(), contrib);
            }
        }

        // Evaluate existing fields (including those absent in this record) for projected size.
        for (field_name, builder) in &self.column_builders {
            let contrib = existing_contribs
                .get(field_name)
                .cloned()
                .unwrap_or_default();
            let projected = builder.estimated_uncompressed_size_with(&contrib, next_record_count);
            if projected > max_segment_len {
                if current_record_count == 0 {
                    self.segment_limit_record_rejections += 1;
                    return Err(JacError::LimitExceeded(format!(
                        "Field '{}' segment ({}) exceeds max_segment_uncompressed_len ({})",
                        field_name, projected, max_segment_len
                    )));
                } else {
                    self.segment_limit_flushes += 1;
                    return Ok(TryAddRecordOutcome::BlockFull { record });
                }
            }
        }

        // Evaluate new fields similarly (no existing state).
        for (field_name, contrib) in &new_field_contribs {
            let presence_bytes = (next_record_count + 7) >> 3;
            let tag_bytes = ((3 * contrib.present_delta) + 7) >> 3;
            let bool_bytes = ((contrib.bool_delta) + 7) >> 3;
            let projected = presence_bytes
                + tag_bytes
                + bool_bytes
                + contrib.int_encoded_bytes
                + contrib.decimal_encoded_bytes
                + contrib.string_raw_bytes
                + contrib.object_raw_bytes
                + contrib.array_raw_bytes;
            if projected > max_segment_len {
                if current_record_count == 0 {
                    self.segment_limit_record_rejections += 1;
                    return Err(JacError::LimitExceeded(format!(
                        "Field '{}' segment ({}) exceeds max_segment_uncompressed_len ({})",
                        field_name, projected, max_segment_len
                    )));
                } else {
                    self.segment_limit_flushes += 1;
                    return Ok(TryAddRecordOutcome::BlockFull { record });
                }
            }
        }

        // All checks passed; commit record.
        let record_memory = self.estimate_record_memory(&record);
        let record_idx = self.records.len();
        self.records.push(record.clone());

        for (field_name, value) in &record {
            if !self.field_names.contains(field_name) {
                self.field_names.push(field_name.clone());
            }

            let block_target_records = self.opts.block_target_records;
            let opts_clone = self.opts.clone();
            let column_builder = self
                .column_builders
                .entry(field_name.clone())
                .or_insert_with(move || ColumnBuilder::new(block_target_records, &opts_clone));

            column_builder.add_value(record_idx, value)?;
        }

        self.estimated_memory += record_memory;

        Ok(TryAddRecordOutcome::Added)
    }

    /// Check if block is full
    pub fn is_full(&self) -> bool {
        self.records.len() >= self.opts.block_target_records
            || self.estimated_memory >= self.opts.limits.max_block_uncompressed_total
    }

    /// Get current record count
    pub fn record_count(&self) -> usize {
        self.records.len()
    }

    /// Number of times segment limit triggered a flush suggestion.
    pub fn segment_limit_flushes(&self) -> usize {
        self.segment_limit_flushes
    }

    /// Number of records rejected because a single field exceeded the segment limit.
    pub fn segment_limit_record_rejections(&self) -> usize {
        self.segment_limit_record_rejections
    }

    /// Finalize block and create block data
    pub fn finalize(self) -> Result<BlockData> {
        let record_count = self.records.len();

        // Sort field names for deterministic output
        let mut sorted_field_names = self.field_names.clone();
        if self.opts.canonicalize_keys {
            sorted_field_names.sort();
        }

        // Build field segments
        let mut field_entries = Vec::new();
        let mut segments = Vec::new();
        let mut current_offset = 0;

        for field_name in &sorted_field_names {
            if let Some(column_builder) = self.column_builders.get(field_name) {
                // Finalize column to get field segment
                let field_segment = column_builder.clone().finalize(&self.opts, record_count)?;

                // Compress segment
                let compressed = field_segment.compress(
                    self.opts.default_codec.compressor_id(),
                    self.opts.default_codec.level(),
                )?;

                // Create directory entry
                let entry = FieldDirectoryEntry {
                    field_name: field_name.clone(),
                    compressor: self.opts.default_codec.compressor_id(),
                    compression_level: self.opts.default_codec.level(),
                    presence_bytes: (record_count + 7) >> 3,
                    tag_bytes: ((3 * field_segment.value_count_present) + 7) >> 3,
                    value_count_present: field_segment.value_count_present,
                    encoding_flags: field_segment.encoding_flags,
                    dict_entry_count: field_segment.dict_entry_count,
                    segment_uncompressed_len: field_segment.uncompressed_payload.len(),
                    segment_compressed_len: compressed.len(),
                    segment_offset: current_offset,
                };

                field_entries.push(entry);
                current_offset += compressed.len();
                segments.push(compressed);
            }
        }

        // Create block header
        let header = BlockHeader {
            record_count,
            fields: field_entries,
        };

        // Encode header
        let header_bytes = header.encode()?;

        // Calculate CRC32C over header + all segments
        let mut crc_data = header_bytes.clone();
        for segment in &segments {
            crc_data.extend_from_slice(segment);
        }
        let crc32c = compute_crc32c(&crc_data);

        Ok(BlockData {
            header,
            segments,
            crc32c,
        })
    }

    /// Estimate memory usage for a record
    fn estimate_record_memory(&self, record: &serde_json::Map<String, serde_json::Value>) -> usize {
        let mut size = 0;
        for (key, value) in record {
            size += key.len() + Self::estimate_value_memory(value);
        }
        size
    }

    /// Estimate memory usage for a JSON value
    fn estimate_value_memory(value: &serde_json::Value) -> usize {
        match value {
            serde_json::Value::Null => 0,
            serde_json::Value::Bool(_) => 1,
            serde_json::Value::Number(n) => n.to_string().len(),
            serde_json::Value::String(s) => s.len(),
            serde_json::Value::Array(arr) => {
                arr.iter().map(Self::estimate_value_memory).sum::<usize>() + 8 // overhead
            }
            serde_json::Value::Object(obj) => {
                obj.iter()
                    .map(|(k, v)| k.len() + Self::estimate_value_memory(v))
                    .sum::<usize>()
                    + 8 // overhead
            }
        }
    }
}

/// Block data containing header, segments, and CRC
#[derive(Debug, Clone)]
pub struct BlockData {
    /// Block header
    pub header: BlockHeader,
    /// Compressed field segments
    pub segments: Vec<Vec<u8>>,
    /// CRC32C checksum
    pub crc32c: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Codec;
    use serde_json::json;

    fn add_record_expect_added(
        builder: &mut BlockBuilder,
        record: serde_json::Map<String, serde_json::Value>,
    ) {
        match builder.try_add_record(record).expect("try add record") {
            TryAddRecordOutcome::Added => {}
            TryAddRecordOutcome::BlockFull { .. } => {
                panic!("unexpected block flush during test")
            }
        }
    }

    #[test]
    fn test_block_builder_basic() {
        let opts = CompressOpts::default();
        let mut builder = BlockBuilder::new(opts);

        // Add some test records
        let mut record1 = serde_json::Map::new();
        record1.insert("id".to_string(), json!(1));
        record1.insert("name".to_string(), json!("alice"));
        add_record_expect_added(&mut builder, record1);

        let mut record2 = serde_json::Map::new();
        record2.insert("id".to_string(), json!(2));
        record2.insert("name".to_string(), json!("bob"));
        add_record_expect_added(&mut builder, record2);

        let block_data = builder.finalize().unwrap();
        assert_eq!(block_data.header.record_count, 2);
        assert_eq!(block_data.header.fields.len(), 2); // "id" and "name" fields
        assert_eq!(block_data.segments.len(), 2);
        assert!(block_data.crc32c != 0);
    }

    #[test]
    fn test_block_builder_schema_drift() {
        let opts = CompressOpts::default();
        let mut builder = BlockBuilder::new(opts);

        // Add records with schema drift (field changes type)
        let mut record1 = serde_json::Map::new();
        record1.insert("value".to_string(), json!(42)); // integer
        add_record_expect_added(&mut builder, record1);

        let mut record2 = serde_json::Map::new();
        record2.insert("value".to_string(), json!("hello")); // string
        add_record_expect_added(&mut builder, record2);

        let block_data = builder.finalize().unwrap();
        assert_eq!(block_data.header.record_count, 2);
        assert_eq!(block_data.header.fields.len(), 1); // "value" field
        assert_eq!(block_data.segments.len(), 1);
    }

    #[test]
    fn test_block_builder_missing_fields() {
        let opts = CompressOpts::default();
        let mut builder = BlockBuilder::new(opts);

        // Add records with different field sets
        let mut record1 = serde_json::Map::new();
        record1.insert("id".to_string(), json!(1));
        record1.insert("name".to_string(), json!("alice"));
        add_record_expect_added(&mut builder, record1);

        let mut record2 = serde_json::Map::new();
        record2.insert("id".to_string(), json!(2));
        // Missing "name" field
        add_record_expect_added(&mut builder, record2);

        let block_data = builder.finalize().unwrap();
        assert_eq!(block_data.header.record_count, 2);
        assert_eq!(block_data.header.fields.len(), 2); // "id" and "name" fields
        assert_eq!(block_data.segments.len(), 2);
    }

    #[test]
    fn test_block_builder_canonicalize_keys() {
        let mut opts = CompressOpts::default();
        opts.canonicalize_keys = true;
        let mut builder = BlockBuilder::new(opts);

        // Add record with unsorted keys
        let mut record = serde_json::Map::new();
        record.insert("zebra".to_string(), json!("last"));
        record.insert("apple".to_string(), json!("first"));
        record.insert("banana".to_string(), json!("middle"));
        add_record_expect_added(&mut builder, record);

        let block_data = builder.finalize().unwrap();
        assert_eq!(block_data.header.record_count, 1);

        // Check that fields are sorted
        let field_names: Vec<&String> = block_data
            .header
            .fields
            .iter()
            .map(|f| &f.field_name)
            .collect();
        assert_eq!(field_names, vec!["apple", "banana", "zebra"]);
    }

    #[test]
    fn test_block_builder_unsupported_brotli_codec() {
        let mut opts = CompressOpts::default();
        opts.default_codec = Codec::Brotli(11);
        let mut builder = BlockBuilder::new(opts);

        let mut record = serde_json::Map::new();
        record.insert("value".to_string(), json!(1));
        add_record_expect_added(&mut builder, record);

        let err = builder.finalize().unwrap_err();
        assert!(matches!(err, JacError::UnsupportedCompression(2)));
    }

    #[test]
    fn test_block_builder_unsupported_deflate_codec() {
        let mut opts = CompressOpts::default();
        opts.default_codec = Codec::Deflate(6);
        let mut builder = BlockBuilder::new(opts);

        let mut record = serde_json::Map::new();
        record.insert("value".to_string(), json!(1));
        add_record_expect_added(&mut builder, record);

        let err = builder.finalize().unwrap_err();
        assert!(matches!(err, JacError::UnsupportedCompression(3)));
    }

    #[test]
    fn try_add_record_signals_block_full_on_segment_limit() {
        let mut opts = CompressOpts::default();
        opts.block_target_records = 10;
        opts.default_codec = Codec::None;
        opts.limits.max_segment_uncompressed_len = 38;

        let mut builder = BlockBuilder::new(opts.clone());

        let mut small = serde_json::Map::new();
        small.insert("field".to_string(), json!("short"));
        add_record_expect_added(&mut builder, small);

        let mut large = serde_json::Map::new();
        large.insert("field".to_string(), json!("012345678901234567890123456789"));

        match builder
            .try_add_record(large.clone())
            .expect("try add record")
        {
            TryAddRecordOutcome::Added => panic!("expected BlockFull outcome"),
            TryAddRecordOutcome::BlockFull { record } => {
                assert_eq!(record.get("field"), large.get("field"));
            }
        }

        assert_eq!(builder.segment_limit_flushes(), 1);
    }

    #[test]
    fn try_add_record_rejects_single_record_exceeding_limit() {
        let mut opts = CompressOpts::default();
        opts.block_target_records = 10;
        opts.default_codec = Codec::None;
        opts.limits.max_segment_uncompressed_len = 16;

        let mut builder = BlockBuilder::new(opts);

        let mut record = serde_json::Map::new();
        record.insert(
            "field".to_string(),
            json!("this string is far too long to fit"),
        );

        let err = builder
            .try_add_record(record)
            .expect_err("expected limit exceeded");
        assert!(matches!(err, JacError::LimitExceeded(_)));
        assert_eq!(builder.segment_limit_record_rejections(), 1);
    }

    #[test]
    fn test_block_builder_empty() {
        let opts = CompressOpts::default();
        let builder = BlockBuilder::new(opts);

        let block_data = builder.finalize().unwrap();
        assert_eq!(block_data.header.record_count, 0);
        assert_eq!(block_data.header.fields.len(), 0);
        assert_eq!(block_data.segments.len(), 0);
    }
}
