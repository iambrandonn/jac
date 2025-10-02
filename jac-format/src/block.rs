//! Block header and directory structures

use crate::limits::Limits;

/// Block header
#[derive(Debug, Clone)]
pub struct BlockHeader {
    /// Number of records in this block
    pub record_count: usize,
    /// Field directory entries
    pub fields: Vec<FieldDirectoryEntry>,
}

/// Field directory entry
#[derive(Debug, Clone)]
pub struct FieldDirectoryEntry {
    /// Field name
    pub field_name: String,
    /// Compressor override
    pub compressor: u8,
    /// Compression level override
    pub compression_level: u8,
    /// Presence bitmap size in bytes
    pub presence_bytes: usize,
    /// Type tag stream size in bytes
    pub tag_bytes: usize,
    /// Number of present values
    pub value_count_present: usize,
    /// Encoding flags
    pub encoding_flags: u64,
    /// Dictionary entry count
    pub dict_entry_count: usize,
    /// Uncompressed segment length
    pub segment_uncompressed_len: usize,
    /// Compressed segment length
    pub segment_compressed_len: usize,
    /// Segment offset from block start
    pub segment_offset: usize,
}

impl BlockHeader {
    /// Encode block header to bytes
    pub fn encode(&self) -> Result<Vec<u8>, crate::error::JacError> {
        // TODO: Implement encoding
        Ok(vec![])
    }

    /// Decode block header from bytes
    pub fn decode(_bytes: &[u8], _limits: &Limits) -> Result<(Self, usize), crate::error::JacError> {
        // TODO: Implement decoding
        Ok((Self {
            record_count: 0,
            fields: vec![],
        }, 0))
    }
}
