//! Field segment encoding

/// Field segment encoding implementation
pub struct FieldSegment {
    pub uncompressed_payload: Vec<u8>,
    pub encoding_flags: u64,
    pub dict_entry_count: usize,
    pub value_count_present: usize,
}

impl FieldSegment {
    /// Compress segment
    pub fn compress(&self, _codec: u8, _level: u8) -> Result<Vec<u8>, jac_format::JacError> {
        // TODO: Implement compression
        Ok(self.uncompressed_payload.clone())
    }
}
