//! Block header and directory structures

use crate::constants::BLOCK_MAGIC;
use crate::limits::Limits;
use crate::varint::{decode_uleb128, encode_uleb128};
use std::convert::TryFrom;

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
        let mut header_body = Vec::new();

        // Record count (ULEB128)
        header_body.extend_from_slice(&encode_uleb128(self.record_count as u64));

        // Field count (ULEB128)
        header_body.extend_from_slice(&encode_uleb128(self.fields.len() as u64));

        // Field directory entries
        for field in &self.fields {
            header_body.extend_from_slice(&encode_uleb128(field.field_name.len() as u64));
            header_body.extend_from_slice(field.field_name.as_bytes());
            header_body.push(field.compressor);
            header_body.push(field.compression_level);
            header_body.extend_from_slice(&encode_uleb128(field.presence_bytes as u64));
            header_body.extend_from_slice(&encode_uleb128(field.tag_bytes as u64));
            header_body.extend_from_slice(&encode_uleb128(field.value_count_present as u64));
            header_body.extend_from_slice(&encode_uleb128(field.encoding_flags));
            header_body.extend_from_slice(&encode_uleb128(field.dict_entry_count as u64));
            header_body.extend_from_slice(&encode_uleb128(field.segment_uncompressed_len as u64));
            header_body.extend_from_slice(&encode_uleb128(field.segment_compressed_len as u64));
            header_body.extend_from_slice(&encode_uleb128(field.segment_offset as u64));
        }

        let mut result = Vec::with_capacity(4 + 10 + header_body.len());
        result.extend_from_slice(&BLOCK_MAGIC.to_le_bytes());
        result.extend_from_slice(&encode_uleb128(header_body.len() as u64));
        result.extend_from_slice(&header_body);

        Ok(result)
    }

    /// Decode block header from bytes
    pub fn decode(bytes: &[u8], limits: &Limits) -> Result<(Self, usize), crate::error::JacError> {
        let mut pos = 0;

        // Check minimum length for magic
        if bytes.len() < 4 {
            return Err(crate::error::JacError::UnexpectedEof);
        }

        // Block magic (little-endian u32)
        let magic =
            u32::from_le_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]);
        if magic != BLOCK_MAGIC {
            return Err(crate::error::JacError::CorruptBlock);
        }
        pos += 4;

        // Header length (ULEB128)
        let (header_len_u64, header_len_bytes) = decode_uleb128(&bytes[pos..])?;
        let header_len = usize::try_from(header_len_u64).map_err(|_| {
            crate::error::JacError::LimitExceeded("header_len exceeds supported size".to_string())
        })?;
        pos += header_len_bytes;

        let header_body_start = pos;
        let header_body_end = header_body_start
            .checked_add(header_len)
            .ok_or_else(|| crate::error::JacError::CorruptBlock)?;
        if header_body_end > bytes.len() {
            return Err(crate::error::JacError::UnexpectedEof);
        }

        // Record count (ULEB128)
        let (record_count_u64, count_bytes) = decode_uleb128(&bytes[pos..header_body_end])?;
        pos += count_bytes;
        let record_count = usize::try_from(record_count_u64).map_err(|_| {
            crate::error::JacError::LimitExceeded(
                "record_count exceeds supported size".to_string(),
            )
        })?;

        // Enforce record count limit
        if record_count > limits.max_records_per_block {
            return Err(crate::error::JacError::LimitExceeded(format!(
                "Record count {} exceeds limit {}",
                record_count, limits.max_records_per_block
            )));
        }

        // Field count (ULEB128)
        let (field_count_u64, field_count_bytes) = decode_uleb128(&bytes[pos..header_body_end])?;
        pos += field_count_bytes;
        let field_count = usize::try_from(field_count_u64).map_err(|_| {
            crate::error::JacError::LimitExceeded(
                "field_count exceeds supported size".to_string(),
            )
        })?;

        // Enforce field count limit
        if field_count > limits.max_fields_per_block {
            return Err(crate::error::JacError::LimitExceeded(format!(
                "Field count {} exceeds limit {}",
                field_count, limits.max_fields_per_block
            )));
        }

        // Decode field directory entries
        let mut fields = Vec::new();
        for _ in 0..field_count {
            if pos >= header_body_end {
                return Err(crate::error::JacError::UnexpectedEof);
            }

            // Field name length (ULEB128)
            let (name_len_u64, name_len_bytes) = decode_uleb128(&bytes[pos..header_body_end])?;
            pos += name_len_bytes;
            let name_len = usize::try_from(name_len_u64).map_err(|_| {
                crate::error::JacError::LimitExceeded(
                    "Field name length exceeds supported size".to_string(),
                )
            })?;

            // Check field name length limit
            if name_len > limits.max_string_len_per_value {
                return Err(crate::error::JacError::LimitExceeded(format!(
                    "Field name length {} exceeds limit {}",
                    name_len, limits.max_string_len_per_value
                )));
            }

            // Check if we have enough bytes for the field name
            if pos + name_len > header_body_end {
                return Err(crate::error::JacError::UnexpectedEof);
            }

            // Field name (UTF-8)
            let field_name = String::from_utf8(bytes[pos..pos + name_len].to_vec())
                .map_err(|_| crate::error::JacError::CorruptBlock)?;
            pos += name_len;

            // Check remaining length for fixed fields
            if pos + 1 + 1 > bytes.len() {
                // compressor + compression_level
                return Err(crate::error::JacError::UnexpectedEof);
            }

            // Compressor
            let compressor = bytes[pos];
            pos += 1;

            // Compression level
            let compression_level = bytes[pos];
            pos += 1;

            // Presence bytes (ULEB128)
            let (presence_bytes_u64, presence_bytes_len) = decode_uleb128(&bytes[pos..header_body_end])?;
            pos += presence_bytes_len;
            let presence_bytes = usize::try_from(presence_bytes_u64).map_err(|_| {
                crate::error::JacError::LimitExceeded(
                    "presence_bytes exceeds supported size".to_string(),
                )
            })?;
            if presence_bytes > limits.max_presence_bytes {
                return Err(crate::error::JacError::LimitExceeded(format!(
                    "Presence bytes {} exceeds limit {}",
                    presence_bytes, limits.max_presence_bytes
                )));
            }

            // Tag bytes (ULEB128)
            let (tag_bytes_u64, tag_bytes_len) = decode_uleb128(&bytes[pos..header_body_end])?;
            pos += tag_bytes_len;
            let tag_bytes = usize::try_from(tag_bytes_u64).map_err(|_| {
                crate::error::JacError::LimitExceeded(
                    "tag_bytes exceeds supported size".to_string(),
                )
            })?;
            if tag_bytes > limits.max_tag_bytes {
                return Err(crate::error::JacError::LimitExceeded(format!(
                    "Tag bytes {} exceeds limit {}",
                    tag_bytes, limits.max_tag_bytes
                )));
            }

            // Value count present (ULEB128)
            let (value_count_present_u64, value_count_present_len) =
                decode_uleb128(&bytes[pos..header_body_end])?;
            pos += value_count_present_len;
            let value_count_present = usize::try_from(value_count_present_u64).map_err(|_| {
                crate::error::JacError::LimitExceeded(
                    "value_count_present exceeds supported size".to_string(),
                )
            })?;
            if value_count_present > record_count {
                return Err(crate::error::JacError::CorruptBlock);
            }

            // Encoding flags (ULEB128)
            let (encoding_flags, encoding_flags_len) = decode_uleb128(&bytes[pos..header_body_end])?;
            pos += encoding_flags_len;

            // Dictionary entry count (ULEB128)
            let (dict_entry_count_u64, dict_entry_count_len) =
                decode_uleb128(&bytes[pos..header_body_end])?;
            pos += dict_entry_count_len;
            let dict_entry_count = usize::try_from(dict_entry_count_u64).map_err(|_| {
                crate::error::JacError::LimitExceeded(
                    "dict_entry_count exceeds supported size".to_string(),
                )
            })?;

            // Enforce dictionary entry count limit
            if dict_entry_count > limits.max_dict_entries_per_field {
                return Err(crate::error::JacError::LimitExceeded(format!(
                    "Dictionary entry count {} exceeds limit {}",
                    dict_entry_count, limits.max_dict_entries_per_field
                )));
            }

            // Segment uncompressed length (ULEB128)
            let (segment_uncompressed_len_u64, segment_uncompressed_len_len) =
                decode_uleb128(&bytes[pos..header_body_end])?;
            pos += segment_uncompressed_len_len;
            let segment_uncompressed_len =
                usize::try_from(segment_uncompressed_len_u64).map_err(|_| {
                    crate::error::JacError::LimitExceeded(
                        "segment_uncompressed_len exceeds supported size".to_string(),
                    )
                })?;

            // Enforce segment uncompressed length limit
            if segment_uncompressed_len > limits.max_segment_uncompressed_len {
                return Err(crate::error::JacError::LimitExceeded(format!(
                    "Segment uncompressed length {} exceeds limit {}",
                    segment_uncompressed_len, limits.max_segment_uncompressed_len
                )));
            }

            // Segment compressed length (ULEB128)
            let (segment_compressed_len_u64, segment_compressed_len_len) =
                decode_uleb128(&bytes[pos..header_body_end])?;
            pos += segment_compressed_len_len;
            let segment_compressed_len =
                usize::try_from(segment_compressed_len_u64).map_err(|_| {
                    crate::error::JacError::LimitExceeded(
                        "segment_compressed_len exceeds supported size".to_string(),
                    )
                })?;

            // Segment offset (ULEB128)
            let (segment_offset_u64, segment_offset_len) =
                decode_uleb128(&bytes[pos..header_body_end])?;
            pos += segment_offset_len;
            let segment_offset = usize::try_from(segment_offset_u64).map_err(|_| {
                crate::error::JacError::LimitExceeded(
                    "segment_offset exceeds supported size".to_string(),
                )
            })?;

            fields.push(FieldDirectoryEntry {
                field_name,
                compressor,
                compression_level,
                presence_bytes,
                tag_bytes,
                value_count_present,
                encoding_flags,
                dict_entry_count,
                segment_uncompressed_len,
                segment_compressed_len,
                segment_offset,
            });
        }

        if pos != header_body_end {
            return Err(crate::error::JacError::CorruptBlock);
        }

        Ok((
            Self {
                record_count,
                fields,
            },
            pos,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::*;
    use crate::error::JacError;
    use crate::varint::{decode_uleb128, encode_uleb128};

    fn create_test_limits() -> Limits {
        Limits {
            max_records_per_block: 100_000,
            max_fields_per_block: 4_096,
            max_segment_uncompressed_len: 64 * 1024 * 1024,
            max_block_uncompressed_total: 256 * 1024 * 1024,
            max_dict_entries_per_field: 4_096,
            max_string_len_per_value: 16 * 1024 * 1024,
            max_decimal_digits_per_value: 65_536,
            max_presence_bytes: 32 * 1024 * 1024,
            max_tag_bytes: 32 * 1024 * 1024,
        }
    }

    fn create_test_field_entry() -> FieldDirectoryEntry {
        FieldDirectoryEntry {
            field_name: "test_field".to_string(),
            compressor: 1,
            compression_level: 15,
            presence_bytes: 100,
            tag_bytes: 50,
            value_count_present: 80,
            encoding_flags: ENCODING_FLAG_DICTIONARY,
            dict_entry_count: 10,
            segment_uncompressed_len: 1000,
            segment_compressed_len: 500,
            segment_offset: 2000,
        }
    }

    #[test]
    fn test_block_header_roundtrip_basic() {
        let header = BlockHeader {
            record_count: 1000,
            fields: vec![create_test_field_entry()],
        };

        let encoded = header.encode().unwrap();
        let (decoded, bytes_consumed) =
            BlockHeader::decode(&encoded, &create_test_limits()).unwrap();

        assert_eq!(header.record_count, decoded.record_count);
        assert_eq!(header.fields.len(), decoded.fields.len());
        assert_eq!(bytes_consumed, encoded.len());
    }

    #[test]
    fn test_block_header_roundtrip_multiple_fields() {
        let fields = vec![
            FieldDirectoryEntry {
                field_name: "field1".to_string(),
                compressor: 1,
                compression_level: 15,
                presence_bytes: 50,
                tag_bytes: 25,
                value_count_present: 40,
                encoding_flags: ENCODING_FLAG_DICTIONARY,
                dict_entry_count: 5,
                segment_uncompressed_len: 500,
                segment_compressed_len: 250,
                segment_offset: 1000,
            },
            FieldDirectoryEntry {
                field_name: "field2".to_string(),
                compressor: 0,
                compression_level: 0,
                presence_bytes: 50,
                tag_bytes: 25,
                value_count_present: 40,
                encoding_flags: 0,
                dict_entry_count: 0,
                segment_uncompressed_len: 500,
                segment_compressed_len: 500,
                segment_offset: 1500,
            },
        ];

        let header = BlockHeader {
            record_count: 2000,
            fields,
        };

        let encoded = header.encode().unwrap();
        let (decoded, bytes_consumed) =
            BlockHeader::decode(&encoded, &create_test_limits()).unwrap();

        assert_eq!(header.record_count, decoded.record_count);
        assert_eq!(header.fields.len(), decoded.fields.len());

        for (original, decoded) in header.fields.iter().zip(decoded.fields.iter()) {
            assert_eq!(original.field_name, decoded.field_name);
            assert_eq!(original.compressor, decoded.compressor);
            assert_eq!(original.compression_level, decoded.compression_level);
            assert_eq!(original.presence_bytes, decoded.presence_bytes);
            assert_eq!(original.tag_bytes, decoded.tag_bytes);
            assert_eq!(original.value_count_present, decoded.value_count_present);
            assert_eq!(original.encoding_flags, decoded.encoding_flags);
            assert_eq!(original.dict_entry_count, decoded.dict_entry_count);
            assert_eq!(
                original.segment_uncompressed_len,
                decoded.segment_uncompressed_len
            );
            assert_eq!(
                original.segment_compressed_len,
                decoded.segment_compressed_len
            );
            assert_eq!(original.segment_offset, decoded.segment_offset);
        }

        assert_eq!(bytes_consumed, encoded.len());
    }

    #[test]
    fn test_block_header_empty_fields() {
        let header = BlockHeader {
            record_count: 0,
            fields: vec![],
        };

        let encoded = header.encode().unwrap();
        let (decoded, bytes_consumed) =
            BlockHeader::decode(&encoded, &create_test_limits()).unwrap();

        assert_eq!(header.record_count, decoded.record_count);
        assert_eq!(header.fields.len(), decoded.fields.len());
        assert_eq!(bytes_consumed, encoded.len());
    }

    #[test]
    fn test_block_header_invalid_magic() {
        let mut invalid_bytes = vec![0x00, 0x00, 0x00, 0x00]; // Wrong magic
        invalid_bytes.extend_from_slice(&encode_uleb128(10)); // header_len
        invalid_bytes.extend_from_slice(&encode_uleb128(100)); // record_count
        invalid_bytes.extend_from_slice(&encode_uleb128(0)); // field_count

        let result = BlockHeader::decode(&invalid_bytes, &create_test_limits());
        assert!(result.is_err());
        if let Err(crate::error::JacError::CorruptBlock) = result {
            // Expected error
        } else {
            panic!("Expected CorruptBlock error");
        }
    }

    #[test]
    fn test_block_header_truncated() {
        let truncated_bytes = vec![0x42, 0x4C, 0x4B, 0x31]; // Just magic
        let result = BlockHeader::decode(&truncated_bytes, &create_test_limits());
        assert!(result.is_err());
        if let Err(crate::error::JacError::UnexpectedEof) = result {
            // Expected error
        } else {
            panic!("Expected UnexpectedEof error");
        }
    }

    #[test]
    fn test_block_header_limit_exceeded_records() {
        let header = BlockHeader {
            record_count: 1_000_001, // Exceeds limit
            fields: vec![],
        };

        let encoded = header.encode().unwrap();
        let result = BlockHeader::decode(&encoded, &create_test_limits());
        assert!(result.is_err());
        if let Err(crate::error::JacError::LimitExceeded(msg)) = result {
            assert!(msg.contains("Record count"));
        } else {
            panic!("Expected LimitExceeded error");
        }
    }

    #[test]
    fn test_block_header_limit_exceeded_fields() {
        let mut fields = Vec::new();
        for i in 0..5000 {
            // Exceeds limit
            fields.push(FieldDirectoryEntry {
                field_name: format!("field_{}", i),
                compressor: 1,
                compression_level: 15,
                presence_bytes: 10,
                tag_bytes: 5,
                value_count_present: 8,
                encoding_flags: 0,
                dict_entry_count: 0,
                segment_uncompressed_len: 100,
                segment_compressed_len: 50,
                segment_offset: i * 100,
            });
        }

        let header = BlockHeader {
            record_count: 1000,
            fields,
        };

        let encoded = header.encode().unwrap();
        let result = BlockHeader::decode(&encoded, &create_test_limits());
        assert!(result.is_err());
        if let Err(crate::error::JacError::LimitExceeded(msg)) = result {
            assert!(msg.contains("Field count"));
        } else {
            panic!("Expected LimitExceeded error");
        }
    }

    #[test]
    fn test_block_header_limit_exceeded_field_name() {
        let long_name = "x".repeat(17 * 1024 * 1024); // Exceeds limit
        let field = FieldDirectoryEntry {
            field_name: long_name,
            compressor: 1,
            compression_level: 15,
            presence_bytes: 10,
            tag_bytes: 5,
            value_count_present: 8,
            encoding_flags: 0,
            dict_entry_count: 0,
            segment_uncompressed_len: 100,
            segment_compressed_len: 50,
            segment_offset: 0,
        };

        let header = BlockHeader {
            record_count: 1000,
            fields: vec![field],
        };

        let encoded = header.encode().unwrap();
        let result = BlockHeader::decode(&encoded, &create_test_limits());
        assert!(result.is_err());
        if let Err(crate::error::JacError::LimitExceeded(msg)) = result {
            assert!(msg.contains("Field name length"));
        } else {
            panic!("Expected LimitExceeded error");
        }
    }

    #[test]
    fn test_block_header_presence_bytes_limit_enforced() {
        let mut limits = create_test_limits();
        limits.max_presence_bytes = 4;

        let field = FieldDirectoryEntry {
            field_name: "field".to_string(),
            compressor: 1,
            compression_level: 15,
            presence_bytes: 5,
            tag_bytes: 1,
            value_count_present: 1,
            encoding_flags: 0,
            dict_entry_count: 0,
            segment_uncompressed_len: 100,
            segment_compressed_len: 50,
            segment_offset: 0,
        };

        let header = BlockHeader {
            record_count: 1,
            fields: vec![field],
        };

        let encoded = header.encode().unwrap();
        let result = BlockHeader::decode(&encoded, &limits);
        assert!(matches!(result, Err(JacError::LimitExceeded(msg)) if msg.contains("Presence bytes")));
    }

    #[test]
    fn test_block_header_tag_bytes_limit_enforced() {
        let mut limits = create_test_limits();
        limits.max_tag_bytes = 4;

        let field = FieldDirectoryEntry {
            field_name: "field".to_string(),
            compressor: 1,
            compression_level: 15,
            presence_bytes: 1,
            tag_bytes: 5,
            value_count_present: 5,
            encoding_flags: 0,
            dict_entry_count: 0,
            segment_uncompressed_len: 100,
            segment_compressed_len: 50,
            segment_offset: 0,
        };

        let header = BlockHeader {
            record_count: 10,
            fields: vec![field],
        };

        let encoded = header.encode().unwrap();
        let result = BlockHeader::decode(&encoded, &limits);
        assert!(matches!(result, Err(JacError::LimitExceeded(msg)) if msg.contains("Tag bytes")));
    }

    #[test]
    fn test_block_header_value_count_exceeds_record_count() {
        let field = FieldDirectoryEntry {
            field_name: "field".to_string(),
            compressor: 1,
            compression_level: 15,
            presence_bytes: 1,
            tag_bytes: 1,
            value_count_present: 11,
            encoding_flags: 0,
            dict_entry_count: 0,
            segment_uncompressed_len: 100,
            segment_compressed_len: 50,
            segment_offset: 0,
        };

        let header = BlockHeader {
            record_count: 10,
            fields: vec![field],
        };

        let encoded = header.encode().unwrap();
        let result = BlockHeader::decode(&encoded, &create_test_limits());
        assert!(matches!(result, Err(JacError::CorruptBlock)));
    }

    #[test]
    fn test_block_header_header_len_mismatch_detected() {
        let header = BlockHeader {
            record_count: 10,
            fields: vec![create_test_field_entry()],
        };

        let mut encoded = header.encode().unwrap();
        let (original_len, len_bytes) = decode_uleb128(&encoded[4..]).unwrap();
        let new_len = original_len + 3;
        let replacement = encode_uleb128(new_len);
        encoded.splice(4..4 + len_bytes, replacement.iter().copied());
        encoded.extend_from_slice(&[0u8; 3]);

        let result = BlockHeader::decode(&encoded, &create_test_limits());
        assert!(matches!(result, Err(JacError::CorruptBlock)));
    }

    #[test]
    fn test_block_header_limit_exceeded_dict_entries() {
        let field = FieldDirectoryEntry {
            field_name: "test_field".to_string(),
            compressor: 1,
            compression_level: 15,
            presence_bytes: 10,
            tag_bytes: 5,
            value_count_present: 8,
            encoding_flags: ENCODING_FLAG_DICTIONARY,
            dict_entry_count: 10_000, // Exceeds limit
            segment_uncompressed_len: 100,
            segment_compressed_len: 50,
            segment_offset: 0,
        };

        let header = BlockHeader {
            record_count: 1000,
            fields: vec![field],
        };

        let encoded = header.encode().unwrap();
        let result = BlockHeader::decode(&encoded, &create_test_limits());
        assert!(result.is_err());
        if let Err(crate::error::JacError::LimitExceeded(msg)) = result {
            assert!(msg.contains("Dictionary entry count"));
        } else {
            panic!("Expected LimitExceeded error");
        }
    }

    #[test]
    fn test_block_header_limit_exceeded_segment_length() {
        let field = FieldDirectoryEntry {
            field_name: "test_field".to_string(),
            compressor: 1,
            compression_level: 15,
            presence_bytes: 10,
            tag_bytes: 5,
            value_count_present: 8,
            encoding_flags: 0,
            dict_entry_count: 0,
            segment_uncompressed_len: 100 * 1024 * 1024, // Exceeds limit
            segment_compressed_len: 50 * 1024 * 1024,
            segment_offset: 0,
        };

        let header = BlockHeader {
            record_count: 1000,
            fields: vec![field],
        };

        let encoded = header.encode().unwrap();
        let result = BlockHeader::decode(&encoded, &create_test_limits());
        assert!(result.is_err());
        if let Err(crate::error::JacError::LimitExceeded(msg)) = result {
            assert!(msg.contains("Segment uncompressed length"));
        } else {
            panic!("Expected LimitExceeded error");
        }
    }

    #[test]
    fn test_block_header_endianness() {
        let header = BlockHeader {
            record_count: 1000,
            fields: vec![create_test_field_entry()],
        };

        let encoded = header.encode().unwrap();

        // Check that block magic is stored little-endian
        let magic_bytes = &encoded[0..4];
        let expected_magic_bytes = BLOCK_MAGIC.to_le_bytes();
        assert_eq!(magic_bytes, &expected_magic_bytes);
    }

    #[test]
    fn test_block_header_unicode_field_name() {
        let field = FieldDirectoryEntry {
            field_name: "测试字段_🚀".to_string(), // Unicode field name
            compressor: 1,
            compression_level: 15,
            presence_bytes: 10,
            tag_bytes: 5,
            value_count_present: 8,
            encoding_flags: 0,
            dict_entry_count: 0,
            segment_uncompressed_len: 100,
            segment_compressed_len: 50,
            segment_offset: 0,
        };

        let header = BlockHeader {
            record_count: 1000,
            fields: vec![field],
        };

        let encoded = header.encode().unwrap();
        let (decoded, _) = BlockHeader::decode(&encoded, &create_test_limits()).unwrap();

        assert_eq!(header.fields[0].field_name, decoded.fields[0].field_name);
    }

    #[test]
    fn test_block_header_large_values() {
        let field = FieldDirectoryEntry {
            field_name: "large_field".to_string(),
            compressor: 255,
            compression_level: 255,
            presence_bytes: 1_000_000,
            tag_bytes: 500_000,
            value_count_present: 80_000,
            encoding_flags: u64::MAX,
            dict_entry_count: 4_000,
            segment_uncompressed_len: 50 * 1024 * 1024,
            segment_compressed_len: 25 * 1024 * 1024,
            segment_offset: u64::MAX as usize,
        };

        let header = BlockHeader {
            record_count: 100_000,
            fields: vec![field],
        };

        let encoded = header.encode().unwrap();
        let (decoded, bytes_consumed) =
            BlockHeader::decode(&encoded, &create_test_limits()).unwrap();

        assert_eq!(header.record_count, decoded.record_count);
        assert_eq!(header.fields[0].field_name, decoded.fields[0].field_name);
        assert_eq!(header.fields[0].compressor, decoded.fields[0].compressor);
        assert_eq!(
            header.fields[0].compression_level,
            decoded.fields[0].compression_level
        );
        assert_eq!(
            header.fields[0].presence_bytes,
            decoded.fields[0].presence_bytes
        );
        assert_eq!(header.fields[0].tag_bytes, decoded.fields[0].tag_bytes);
        assert_eq!(
            header.fields[0].value_count_present,
            decoded.fields[0].value_count_present
        );
        assert_eq!(
            header.fields[0].encoding_flags,
            decoded.fields[0].encoding_flags
        );
        assert_eq!(
            header.fields[0].dict_entry_count,
            decoded.fields[0].dict_entry_count
        );
        assert_eq!(
            header.fields[0].segment_uncompressed_len,
            decoded.fields[0].segment_uncompressed_len
        );
        assert_eq!(
            header.fields[0].segment_compressed_len,
            decoded.fields[0].segment_compressed_len
        );
        assert_eq!(
            header.fields[0].segment_offset,
            decoded.fields[0].segment_offset
        );
        assert_eq!(bytes_consumed, encoded.len());
    }
}
