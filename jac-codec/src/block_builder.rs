//! Block builder for aggregating columns

use crate::{ColumnBuilder, CompressOpts};
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
        }
    }

    /// Add record to block
    pub fn add_record(&mut self, rec: serde_json::Map<String, serde_json::Value>) -> Result<()> {
        // Check if block is full
        if self.is_full() {
            return Err(JacError::Internal("Block is full".to_string()));
        }

        let record_idx = self.records.len();
        self.records.push(rec);

        // Discover new fields and add values to column builders
        for (field_name, value) in &self.records[record_idx] {
            if !self.field_names.contains(field_name) {
                self.field_names.push(field_name.clone());
            }

            // Get or create column builder for this field
            let column_builder = self
                .column_builders
                .entry(field_name.clone())
                .or_insert_with(|| ColumnBuilder::new(self.opts.block_target_records));

            // Add value to column
            column_builder.add_value(record_idx, value)?;
        }

        // Update memory estimate
        self.estimated_memory += self.estimate_record_memory(&self.records[record_idx]);

        Ok(())
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
    use serde_json::json;

    #[test]
    fn test_block_builder_basic() {
        let opts = CompressOpts::default();
        let mut builder = BlockBuilder::new(opts);

        // Add some test records
        let mut record1 = serde_json::Map::new();
        record1.insert("id".to_string(), json!(1));
        record1.insert("name".to_string(), json!("alice"));
        builder.add_record(record1).unwrap();

        let mut record2 = serde_json::Map::new();
        record2.insert("id".to_string(), json!(2));
        record2.insert("name".to_string(), json!("bob"));
        builder.add_record(record2).unwrap();

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
        builder.add_record(record1).unwrap();

        let mut record2 = serde_json::Map::new();
        record2.insert("value".to_string(), json!("hello")); // string
        builder.add_record(record2).unwrap();

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
        builder.add_record(record1).unwrap();

        let mut record2 = serde_json::Map::new();
        record2.insert("id".to_string(), json!(2));
        // Missing "name" field
        builder.add_record(record2).unwrap();

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
        builder.add_record(record).unwrap();

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
    fn test_block_builder_empty() {
        let opts = CompressOpts::default();
        let builder = BlockBuilder::new(opts);

        let block_data = builder.finalize().unwrap();
        assert_eq!(block_data.header.record_count, 0);
        assert_eq!(block_data.header.fields.len(), 0);
        assert_eq!(block_data.segments.len(), 0);
    }
}
