//! Field segment encoding

use crate::Codec;
use jac_format::{JacError, Result};
use std::convert::TryFrom;
use std::io::Write;

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
    pub fn compress(&self, codec: Codec) -> Result<Vec<u8>> {
        match codec {
            Codec::None => Ok(self.uncompressed_payload.clone()),
            Codec::Zstd(level) => {
                zstd::encode_all(self.uncompressed_payload.as_slice(), i32::from(level)).map_err(
                    |e| JacError::DecompressError(format!("Zstd compression failed: {}", e)),
                )
            }
            Codec::ZstdWithThreads { level, threads } => {
                if threads == 0 {
                    return Err(JacError::Internal(
                        "Zstd thread count must be at least 1".to_string(),
                    ));
                }

                let threads_u32 = u32::try_from(threads).map_err(|_| {
                    JacError::Internal(format!(
                        "Zstd thread count {} exceeds supported range",
                        threads
                    ))
                })?;

                let mut encoder = zstd::Encoder::new(Vec::new(), level).map_err(|e| {
                    JacError::DecompressError(format!("Zstd encoder init failed: {}", e))
                })?;
                encoder.multithread(threads_u32).map_err(|e| {
                    JacError::DecompressError(format!(
                        "Zstd multithread configuration failed: {}",
                        e
                    ))
                })?;
                encoder
                    .write_all(&self.uncompressed_payload)
                    .map_err(|e| JacError::DecompressError(format!("Zstd write failed: {}", e)))?;
                encoder.finish().map_err(|e| {
                    JacError::DecompressError(format!("Zstd compression failed: {}", e))
                })
            }
            Codec::Brotli(_) => {
                // Brotli (not implemented in v0.1.0)
                Err(JacError::UnsupportedCompression(2))
            }
            Codec::Deflate(_) => {
                // Deflate (not implemented in v0.1.0)
                Err(JacError::UnsupportedCompression(3))
            }
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
        let err = segment.compress(Codec::Brotli(11)).unwrap_err();
        assert!(matches!(err, JacError::UnsupportedCompression(2)));
    }

    #[test]
    fn test_compress_deflate_returns_unsupported() {
        let segment = sample_segment();
        let err = segment.compress(Codec::Deflate(6)).unwrap_err();
        assert!(matches!(err, JacError::UnsupportedCompression(3)));
    }
}
