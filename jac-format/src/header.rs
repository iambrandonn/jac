//! File header structures

use crate::constants::FILE_MAGIC;
use crate::varint::{decode_uleb128, encode_uleb128};

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
        let mut result = Vec::new();

        // Magic bytes
        result.extend_from_slice(&FILE_MAGIC);

        // Flags (little-endian u32)
        result.extend_from_slice(&self.flags.to_le_bytes());

        // Default compressor
        result.push(self.default_compressor);

        // Default compression level
        result.push(self.default_compression_level);

        // Block size hint (ULEB128)
        result.extend_from_slice(&encode_uleb128(self.block_size_hint_records as u64));

        // User metadata length (ULEB128)
        result.extend_from_slice(&encode_uleb128(self.user_metadata.len() as u64));

        // User metadata
        result.extend_from_slice(&self.user_metadata);

        Ok(result)
    }

    /// Decode header from bytes
    pub fn decode(bytes: &[u8]) -> Result<(Self, usize), crate::error::JacError> {
        let mut pos = 0;

        // Check minimum length
        if bytes.len() < 4 {
            return Err(crate::error::JacError::UnexpectedEof);
        }

        // Magic bytes
        if bytes[pos..pos + 4] != FILE_MAGIC {
            return Err(crate::error::JacError::InvalidMagic);
        }
        pos += 4;

        // Check remaining length for fixed fields
        if bytes.len() < pos + 4 + 1 + 1 {
            return Err(crate::error::JacError::UnexpectedEof);
        }

        // Flags (little-endian u32)
        let flags =
            u32::from_le_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]);
        pos += 4;

        // Default compressor
        let default_compressor = bytes[pos];
        pos += 1;

        // Default compression level
        let default_compression_level = bytes[pos];
        pos += 1;

        // Block size hint (ULEB128)
        let (block_size_hint_records, hint_bytes) = decode_uleb128(&bytes[pos..])?;
        pos += hint_bytes;

        // User metadata length (ULEB128)
        let (metadata_len, len_bytes) = decode_uleb128(&bytes[pos..])?;
        pos += len_bytes;

        // Check if we have enough bytes for metadata
        if pos + metadata_len as usize > bytes.len() {
            return Err(crate::error::JacError::UnexpectedEof);
        }

        // User metadata
        let user_metadata = bytes[pos..pos + metadata_len as usize].to_vec();
        pos += metadata_len as usize;

        Ok((
            Self {
                flags,
                default_compressor,
                default_compression_level,
                block_size_hint_records: block_size_hint_records as usize,
                user_metadata,
            },
            pos,
        ))
    }

    /// Check if canonicalize keys flag is set
    pub fn canonicalize_keys(&self) -> bool {
        self.flags & crate::constants::FLAG_CANONICALIZE_KEYS != 0
    }

    /// Check if canonicalize numbers flag is set
    pub fn canonicalize_numbers(&self) -> bool {
        self.flags & crate::constants::FLAG_CANONICALIZE_NUMBERS != 0
    }

    /// Check if nested opaque flag is set
    pub fn nested_opaque(&self) -> bool {
        self.flags & crate::constants::FLAG_NESTED_OPAQUE != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::*;

    #[test]
    fn test_file_header_roundtrip_basic() {
        let header = FileHeader {
            flags: 0,
            default_compressor: 1,
            default_compression_level: 15,
            block_size_hint_records: 100_000,
            user_metadata: vec![],
        };

        let encoded = header.encode().unwrap();
        let (decoded, bytes_consumed) = FileHeader::decode(&encoded).unwrap();

        assert_eq!(header.flags, decoded.flags);
        assert_eq!(header.default_compressor, decoded.default_compressor);
        assert_eq!(
            header.default_compression_level,
            decoded.default_compression_level
        );
        assert_eq!(
            header.block_size_hint_records,
            decoded.block_size_hint_records
        );
        assert_eq!(header.user_metadata, decoded.user_metadata);
        assert_eq!(bytes_consumed, encoded.len());
    }

    #[test]
    fn test_file_header_roundtrip_with_metadata() {
        let header = FileHeader {
            flags: FLAG_CANONICALIZE_KEYS | FLAG_CANONICALIZE_NUMBERS,
            default_compressor: 1,
            default_compression_level: 19,
            block_size_hint_records: 50_000,
            user_metadata: b"test metadata".to_vec(),
        };

        let encoded = header.encode().unwrap();
        let (decoded, bytes_consumed) = FileHeader::decode(&encoded).unwrap();

        assert_eq!(header.flags, decoded.flags);
        assert_eq!(header.default_compressor, decoded.default_compressor);
        assert_eq!(
            header.default_compression_level,
            decoded.default_compression_level
        );
        assert_eq!(
            header.block_size_hint_records,
            decoded.block_size_hint_records
        );
        assert_eq!(header.user_metadata, decoded.user_metadata);
        assert_eq!(bytes_consumed, encoded.len());
    }

    #[test]
    fn test_file_header_roundtrip_large_metadata() {
        let large_metadata = vec![0u8; 1024 * 1024]; // 1MB metadata
        let header = FileHeader {
            flags: FLAG_NESTED_OPAQUE,
            default_compressor: 0,
            default_compression_level: 0,
            block_size_hint_records: 1_000_000,
            user_metadata: large_metadata.clone(),
        };

        let encoded = header.encode().unwrap();
        let (decoded, bytes_consumed) = FileHeader::decode(&encoded).unwrap();

        assert_eq!(header.flags, decoded.flags);
        assert_eq!(header.user_metadata, decoded.user_metadata);
        assert_eq!(bytes_consumed, encoded.len());
    }

    #[test]
    fn test_file_header_invalid_magic() {
        let mut invalid_bytes = vec![0x00, 0x00, 0x00, 0x00]; // Wrong magic
        invalid_bytes.extend_from_slice(&0u32.to_le_bytes());
        invalid_bytes.push(1); // compressor
        invalid_bytes.push(15); // level
        invalid_bytes.extend_from_slice(&encode_uleb128(100_000));
        invalid_bytes.extend_from_slice(&encode_uleb128(0)); // no metadata

        let result = FileHeader::decode(&invalid_bytes);
        assert!(result.is_err());
        if let Err(crate::error::JacError::InvalidMagic) = result {
            // Expected error
        } else {
            panic!("Expected InvalidMagic error");
        }
    }

    #[test]
    fn test_file_header_truncated() {
        let truncated_bytes = vec![0x4A, 0x41, 0x43, 0x01]; // Just magic
        let result = FileHeader::decode(&truncated_bytes);
        assert!(result.is_err());
        if let Err(crate::error::JacError::UnexpectedEof) = result {
            // Expected error
        } else {
            panic!("Expected UnexpectedEof error");
        }
    }

    #[test]
    fn test_file_header_flag_accessors() {
        let header = FileHeader {
            flags: FLAG_CANONICALIZE_KEYS | FLAG_CANONICALIZE_NUMBERS | FLAG_NESTED_OPAQUE,
            default_compressor: 1,
            default_compression_level: 15,
            block_size_hint_records: 100_000,
            user_metadata: vec![],
        };

        assert!(header.canonicalize_keys());
        assert!(header.canonicalize_numbers());
        assert!(header.nested_opaque());

        let header_no_flags = FileHeader {
            flags: 0,
            default_compressor: 1,
            default_compression_level: 15,
            block_size_hint_records: 100_000,
            user_metadata: vec![],
        };

        assert!(!header_no_flags.canonicalize_keys());
        assert!(!header_no_flags.canonicalize_numbers());
        assert!(!header_no_flags.nested_opaque());
    }

    #[test]
    fn test_file_header_endianness() {
        let header = FileHeader {
            flags: 0x12345678, // Test endianness
            default_compressor: 1,
            default_compression_level: 15,
            block_size_hint_records: 100_000,
            user_metadata: vec![],
        };

        let encoded = header.encode().unwrap();

        // Check that flags are stored little-endian
        let magic_len = 4;
        let flags_start = magic_len;
        let flags_bytes = &encoded[flags_start..flags_start + 4];
        let expected_flags_bytes = 0x12345678u32.to_le_bytes();
        assert_eq!(flags_bytes, &expected_flags_bytes);
    }

    #[test]
    fn test_file_header_edge_cases() {
        // Test with zero block size hint
        let header = FileHeader {
            flags: 0,
            default_compressor: 1,
            default_compression_level: 15,
            block_size_hint_records: 0,
            user_metadata: vec![],
        };

        let encoded = header.encode().unwrap();
        let (decoded, _) = FileHeader::decode(&encoded).unwrap();
        assert_eq!(decoded.block_size_hint_records, 0);

        // Test with maximum u8 values
        let header = FileHeader {
            flags: 0,
            default_compressor: 255,
            default_compression_level: 255,
            block_size_hint_records: 100_000,
            user_metadata: vec![],
        };

        let encoded = header.encode().unwrap();
        let (decoded, _) = FileHeader::decode(&encoded).unwrap();
        assert_eq!(decoded.default_compressor, 255);
        assert_eq!(decoded.default_compression_level, 255);
    }
}
