//! Block builder for aggregating columns

use crate::{
    column::{ColumnContribution, FieldSegment},
    Codec, ColumnBuilder, CompressOpts,
};
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
    /// Per-field count of early flushes triggered by this field
    per_field_flush_count: HashMap<String, u64>,
    /// Per-field count of record rejections caused by this field
    per_field_rejection_count: HashMap<String, u64>,
    /// Per-field maximum observed segment size (uncompressed)
    per_field_max_segment: HashMap<String, usize>,
}

/// Uncompressed block data produced by [`BlockBuilder::prepare_segments`].
///
/// This structure captures all columnar output for a block without performing
/// compression, enabling callers to hand the payload off to worker threads for
/// parallel compression.
#[derive(Debug)]
pub struct UncompressedBlockData {
    /// Field segments in deterministic (possibly canonicalized) order.
    pub field_segments: Vec<(String, FieldSegment)>,
    /// Record count for this block.
    pub record_count: usize,
    /// Aggregated count of early segment flushes triggered while building the block.
    pub segment_limit_flushes: usize,
    /// Aggregated count of per-record rejections while building the block.
    pub segment_limit_record_rejections: usize,
    /// Field-specific flush counts.
    pub per_field_flush_count: HashMap<String, u64>,
    /// Field-specific rejection counts.
    pub per_field_rejection_count: HashMap<String, u64>,
    /// Maximum uncompressed segment size observed per field.
    pub per_field_max_segment: HashMap<String, usize>,
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
            per_field_flush_count: HashMap::new(),
            per_field_rejection_count: HashMap::new(),
            per_field_max_segment: HashMap::new(),
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
                    *self
                        .per_field_rejection_count
                        .entry(field_name.clone())
                        .or_insert(0) += 1;
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
                    *self
                        .per_field_rejection_count
                        .entry(field_name.clone())
                        .or_insert(0) += 1;
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
                    *self
                        .per_field_rejection_count
                        .entry(field_name.clone())
                        .or_insert(0) += 1;
                    return Err(JacError::LimitExceeded(format!(
                        "Field '{}' segment ({}) exceeds max_segment_uncompressed_len ({})",
                        field_name, projected, max_segment_len
                    )));
                } else {
                    self.segment_limit_flushes += 1;
                    *self
                        .per_field_flush_count
                        .entry(field_name.clone())
                        .or_insert(0) += 1;
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
                    *self
                        .per_field_rejection_count
                        .entry(field_name.clone())
                        .or_insert(0) += 1;
                    return Err(JacError::LimitExceeded(format!(
                        "Field '{}' segment ({}) exceeds max_segment_uncompressed_len ({})",
                        field_name, projected, max_segment_len
                    )));
                } else {
                    self.segment_limit_flushes += 1;
                    *self
                        .per_field_flush_count
                        .entry(field_name.clone())
                        .or_insert(0) += 1;
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

    /// Returns true when no records have been buffered into the block.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Number of times segment limit triggered a flush suggestion.
    pub fn segment_limit_flushes(&self) -> usize {
        self.segment_limit_flushes
    }

    /// Number of records rejected because a single field exceeded the segment limit.
    pub fn segment_limit_record_rejections(&self) -> usize {
        self.segment_limit_record_rejections
    }

    /// Prepare field segments without performing compression.
    ///
    /// This method consumes the builder, finalizes each column, and packages
    /// the resulting `FieldSegment`s along with all accumulated metrics.
    /// Callers are expected to pass the returned value to
    /// [`compress_block_segments`] (potentially on a worker thread).
    pub fn prepare_segments(mut self) -> Result<UncompressedBlockData> {
        let record_count = self.records.len();

        let mut sorted_field_names = self.field_names.clone();
        if self.opts.canonicalize_keys {
            sorted_field_names.sort();
        }

        let mut field_segments = Vec::with_capacity(sorted_field_names.len());

        for field_name in &sorted_field_names {
            if let Some(column_builder) = self.column_builders.get(field_name) {
                let field_segment = column_builder.clone().finalize(&self.opts, record_count)?;

                let segment_size = field_segment.uncompressed_payload.len();
                let current_max = self
                    .per_field_max_segment
                    .get(field_name)
                    .copied()
                    .unwrap_or(0);
                if segment_size > current_max {
                    self.per_field_max_segment
                        .insert(field_name.clone(), segment_size);
                }

                field_segments.push((field_name.clone(), field_segment));
            }
        }

        Ok(UncompressedBlockData {
            field_segments,
            record_count,
            segment_limit_flushes: self.segment_limit_flushes,
            segment_limit_record_rejections: self.segment_limit_record_rejections,
            per_field_flush_count: self.per_field_flush_count,
            per_field_rejection_count: self.per_field_rejection_count,
            per_field_max_segment: self.per_field_max_segment,
        })
    }

    /// Finalize block and create block data with metrics.
    ///
    /// This remains the sequential helper used by existing callers. Internally
    /// it routes through [`prepare_segments`] followed by
    /// [`compress_block_segments`].
    pub fn finalize(self) -> Result<BlockFinish> {
        let codec = self.opts.default_codec;
        let uncompressed = self.prepare_segments()?;
        compress_block_segments(uncompressed, codec)
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

/// Compress prepared block segments using the provided codec.
///
/// Consumers that prepare blocks on a producer thread can invoke this helper
/// inside a worker context to obtain the final `BlockFinish` payload without
/// re-implementing directory construction or CRC accounting.
pub fn compress_block_segments(
    uncompressed: UncompressedBlockData,
    codec: Codec,
) -> Result<BlockFinish> {
    let record_count = uncompressed.record_count;
    let mut field_entries = Vec::with_capacity(uncompressed.field_segments.len());
    let mut segments = Vec::with_capacity(uncompressed.field_segments.len());
    let mut current_offset = 0usize;

    let compressor_id = codec.compressor_id();
    let compression_level = codec.level();

    for (field_name, field_segment) in uncompressed.field_segments {
        let compressed = field_segment.compress(codec)?;

        let entry = FieldDirectoryEntry {
            field_name,
            compressor: compressor_id,
            compression_level,
            presence_bytes: (record_count + 7) >> 3,
            tag_bytes: ((3 * field_segment.value_count_present) + 7) >> 3,
            value_count_present: field_segment.value_count_present,
            encoding_flags: field_segment.encoding_flags,
            dict_entry_count: field_segment.dict_entry_count,
            segment_uncompressed_len: field_segment.uncompressed_payload.len(),
            segment_compressed_len: compressed.len(),
            segment_offset: current_offset,
        };

        current_offset += compressed.len();
        field_entries.push(entry);
        segments.push(compressed);
    }

    let header = BlockHeader {
        record_count,
        fields: field_entries,
    };

    let header_bytes = header.encode()?;

    let mut crc_data = header_bytes.clone();
    for segment in &segments {
        crc_data.extend_from_slice(segment);
    }
    let crc32c = compute_crc32c(&crc_data);

    let data = BlockData {
        header,
        segments,
        crc32c,
    };

    Ok(BlockFinish {
        data,
        segment_limit_flushes: uncompressed.segment_limit_flushes,
        segment_limit_record_rejections: uncompressed.segment_limit_record_rejections,
        per_field_flush_count: uncompressed.per_field_flush_count,
        per_field_rejection_count: uncompressed.per_field_rejection_count,
        per_field_max_segment: uncompressed.per_field_max_segment,
    })
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

/// Metrics returned after finalizing a block.
#[derive(Debug, Clone)]
pub struct BlockFinish {
    /// The encoded block data
    pub data: BlockData,
    /// Number of early flushes in this block (aggregated)
    pub segment_limit_flushes: usize,
    /// Number of record rejections in this block (aggregated)
    pub segment_limit_record_rejections: usize,
    /// Per-field flush counts
    pub per_field_flush_count: HashMap<String, u64>,
    /// Per-field rejection counts
    pub per_field_rejection_count: HashMap<String, u64>,
    /// Per-field maximum segment sizes observed (uncompressed)
    pub per_field_max_segment: HashMap<String, usize>,
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

        let block_data = builder.finalize().unwrap().data;
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

        let block_data = builder.finalize().unwrap().data;
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

        let block_data = builder.finalize().unwrap().data;
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

        let block_data = builder.finalize().unwrap().data;
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
    fn test_prepare_segments_equivalence() {
        let mut opts_seq = CompressOpts::default();
        opts_seq.default_codec = Codec::None;
        let mut builder_seq = BlockBuilder::new(opts_seq);

        let mut opts_split = CompressOpts::default();
        opts_split.default_codec = Codec::None;
        let mut builder_split = BlockBuilder::new(opts_split);

        let mut record1 = serde_json::Map::new();
        record1.insert("id".to_string(), json!(1));
        record1.insert("name".to_string(), json!("alice"));

        let mut record2 = serde_json::Map::new();
        record2.insert("id".to_string(), json!(2));
        record2.insert("name".to_string(), json!("bob"));

        let records = vec![record1, record2];

        for record in records.iter().cloned() {
            add_record_expect_added(&mut builder_seq, record);
        }
        for record in records.into_iter() {
            add_record_expect_added(&mut builder_split, record);
        }

        let expected = builder_seq.finalize().expect("sequential finalize");
        let uncompressed = builder_split.prepare_segments().expect("prepare segments");
        let rebuilt =
            compress_block_segments(uncompressed, Codec::None).expect("compress segments");

        assert_eq!(
            rebuilt.data.header.record_count,
            expected.data.header.record_count
        );
        assert_eq!(rebuilt.data.segments, expected.data.segments);
        assert_eq!(rebuilt.data.crc32c, expected.data.crc32c);
        assert_eq!(
            rebuilt.segment_limit_flushes,
            expected.segment_limit_flushes
        );
        assert_eq!(
            rebuilt.segment_limit_record_rejections,
            expected.segment_limit_record_rejections
        );
        assert_eq!(
            rebuilt.per_field_flush_count,
            expected.per_field_flush_count
        );
        assert_eq!(
            rebuilt.per_field_rejection_count,
            expected.per_field_rejection_count
        );
        assert_eq!(
            rebuilt.per_field_max_segment,
            expected.per_field_max_segment
        );
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

        let block_finish = builder.finalize().unwrap();
        assert_eq!(block_finish.data.header.record_count, 0);
        assert_eq!(block_finish.data.header.fields.len(), 0);
        assert_eq!(block_finish.data.segments.len(), 0);
    }
}
