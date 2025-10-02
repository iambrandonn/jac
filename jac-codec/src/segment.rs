//! Field segment encoding

use jac_format::{Result, JacError};

/// Field segment containing encoded data
#[derive(Debug, Clone)]
pub struct FieldSegment {
    /// Uncompressed payload bytes
    pub uncompressed_payload: Vec<u8>,
    /// Encoding flags bitfield
    pub encoding_flags: u64,
    /// Number of dictionary entries
    pub dict_entry_count: usize,
    /// Number of present values
    pub value_count_present: usize,
}

impl FieldSegment {
    /// Compress segment using specified codec and level
    pub fn compress(&self, codec: u8, level: u8) -> Result<Vec<u8>> {
        match codec {
            0 => {
                // No compression
                Ok(self.uncompressed_payload.clone())
            }
            1 => {
                // Zstandard compression
                zstd::encode_all(self.uncompressed_payload.as_slice(), level as i32)
                    .map_err(|e| JacError::DecompressError(format!("Zstd compression failed: {}", e)))
            }
            2 => {
                // Brotli (not implemented in v0.1.0)
                Err(JacError::UnsupportedCompression(2))
            }
            3 => {
                // Deflate (not implemented in v0.1.0)
                Err(JacError::UnsupportedCompression(3))
            }
            _ => Err(JacError::UnsupportedCompression(codec)),
        }
    }
}
