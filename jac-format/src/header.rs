//! File header structures

/// File header
#[derive(Debug, Clone)]
pub struct FileHeader {
    /// File flags
    pub flags: u32,
    /// Default compressor
    pub default_compressor: u8,
    /// Default compression level
    pub default_compression_level: u8,
    /// Block size hint in records
    pub block_size_hint_records: usize,
    /// User metadata
    pub user_metadata: Vec<u8>,
}

impl FileHeader {
    /// Encode header to bytes
    pub fn encode(&self) -> Result<Vec<u8>, crate::error::JacError> {
        // TODO: Implement encoding
        Ok(vec![])
    }

    /// Decode header from bytes
    pub fn decode(_bytes: &[u8]) -> Result<(Self, usize), crate::error::JacError> {
        // TODO: Implement decoding
        Ok((Self {
            flags: 0,
            default_compressor: 1,
            default_compression_level: 15,
            block_size_hint_records: 100_000,
            user_metadata: vec![],
        }, 0))
    }
}
