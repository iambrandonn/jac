//! Field segment encoding

use jac_format::{JacError, Result};

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
                zstd::encode_all(self.uncompressed_payload.as_slice(), level as i32).map_err(|e| {
                    JacError::DecompressError(format!("Zstd compression failed: {}", e))
                })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_segment() -> FieldSegment {
        FieldSegment {
            uncompressed_payload: b"payload".to_vec(),
            encoding_flags: 0,
            dict_entry_count: 0,
            value_count_present: 0,
        }
    }

    #[test]
    fn test_compress_brotli_returns_unsupported() {
        let segment = sample_segment();
        let err = segment.compress(2, 4).unwrap_err();
        assert!(matches!(err, JacError::UnsupportedCompression(2)));
    }

    #[test]
    fn test_compress_deflate_returns_unsupported() {
        let segment = sample_segment();
        let err = segment.compress(3, 4).unwrap_err();
        assert!(matches!(err, JacError::UnsupportedCompression(3)));
    }
}
