//! Index footer structures

use crate::checksum::{compute_crc32c, verify_crc32c};
use crate::constants::INDEX_MAGIC;
use crate::varint::{decode_uleb128, encode_uleb128};

/// Index footer
#[derive(Debug, Clone)]
pub struct IndexFooter {
    /// Block index entries
    pub blocks: Vec<BlockIndexEntry>,
}

/// Block index entry
#[derive(Debug, Clone)]
pub struct BlockIndexEntry {
    /// Block offset in file
    pub block_offset: u64,
    /// Block size in bytes
    pub block_size: usize,
    /// Record count in block
    pub record_count: usize,
}

impl IndexFooter {
    /// Encode index footer to bytes
    pub fn encode(&self) -> Result<Vec<u8>, crate::error::JacError> {
        let mut result = Vec::new();

        // Index magic (little-endian u32)
        result.extend_from_slice(&INDEX_MAGIC.to_le_bytes());

        // Placeholder for index_len (will be filled in later)
        let index_len_pos = result.len();
        result.extend_from_slice(&[0u8; 8]); // Reserve space for ULEB128

        // Block count (ULEB128)
        result.extend_from_slice(&encode_uleb128(self.blocks.len() as u64));

        // Block index entries
        for block in &self.blocks {
            // Block offset (ULEB128)
            result.extend_from_slice(&encode_uleb128(block.block_offset));

            // Block size (ULEB128)
            result.extend_from_slice(&encode_uleb128(block.block_size as u64));

            // Record count (ULEB128)
            result.extend_from_slice(&encode_uleb128(block.record_count as u64));
        }

        // Calculate and write index_len
        let index_len = result.len() - index_len_pos - 8; // Length after the index_len field
        let index_len_bytes = encode_uleb128(index_len as u64);

        // Replace the placeholder with actual index_len
        result[index_len_pos..index_len_pos + index_len_bytes.len()]
            .copy_from_slice(&index_len_bytes);

        // Remove any excess bytes if index_len_bytes is shorter than 8
        if index_len_bytes.len() < 8 {
            let excess = 8 - index_len_bytes.len();
            for _ in 0..excess {
                result.remove(index_len_pos + index_len_bytes.len());
            }
        }

        // Compute CRC32C over the entire footer (excluding the CRC itself)
        let crc = compute_crc32c(&result);
        result.extend_from_slice(&crc.to_le_bytes());

        Ok(result)
    }

    /// Decode index footer from bytes
    pub fn decode(bytes: &[u8]) -> Result<Self, crate::error::JacError> {
        let mut pos = 0;

        // Check minimum length for magic
        if bytes.len() < 4 {
            return Err(crate::error::JacError::UnexpectedEof);
        }

        // Index magic (little-endian u32)
        let magic =
            u32::from_le_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]);
        if magic != INDEX_MAGIC {
            return Err(crate::error::JacError::CorruptBlock);
        }
        pos += 4;

        // Index length (ULEB128)
        let (_index_len, index_len_bytes) = decode_uleb128(&bytes[pos..])?;
        pos += index_len_bytes;

        // Block count (ULEB128)
        let (block_count, count_bytes) = decode_uleb128(&bytes[pos..])?;
        pos += count_bytes;

        // Decode block index entries
        let mut blocks = Vec::new();
        for _ in 0..block_count {
            // Block offset (ULEB128)
            let (block_offset, offset_bytes) = decode_uleb128(&bytes[pos..])?;
            pos += offset_bytes;

            // Block size (ULEB128)
            let (block_size, size_bytes) = decode_uleb128(&bytes[pos..])?;
            pos += size_bytes;

            // Record count (ULEB128)
            let (record_count, record_count_bytes) = decode_uleb128(&bytes[pos..])?;
            pos += record_count_bytes;

            blocks.push(BlockIndexEntry {
                block_offset,
                block_size: block_size as usize,
                record_count: record_count as usize,
            });
        }

        // Verify CRC32C
        if pos + 4 > bytes.len() {
            return Err(crate::error::JacError::UnexpectedEof);
        }

        let expected_crc =
            u32::from_le_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]);
        let footer_without_crc = &bytes[0..pos];
        verify_crc32c(footer_without_crc, expected_crc)?;

        Ok(Self { blocks })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_block_entry() -> BlockIndexEntry {
        BlockIndexEntry {
            block_offset: 1000,
            block_size: 5000,
            record_count: 1000,
        }
    }

    #[test]
    fn test_index_footer_roundtrip_basic() {
        let footer = IndexFooter {
            blocks: vec![create_test_block_entry()],
        };

        let encoded = footer.encode().unwrap();
        let decoded = IndexFooter::decode(&encoded).unwrap();

        assert_eq!(footer.blocks.len(), decoded.blocks.len());
        assert_eq!(
            footer.blocks[0].block_offset,
            decoded.blocks[0].block_offset
        );
        assert_eq!(footer.blocks[0].block_size, decoded.blocks[0].block_size);
        assert_eq!(
            footer.blocks[0].record_count,
            decoded.blocks[0].record_count
        );
    }

    #[test]
    fn test_index_footer_roundtrip_multiple_blocks() {
        let blocks = vec![
            BlockIndexEntry {
                block_offset: 0,
                block_size: 1000,
                record_count: 100,
            },
            BlockIndexEntry {
                block_offset: 1000,
                block_size: 2000,
                record_count: 200,
            },
            BlockIndexEntry {
                block_offset: 3000,
                block_size: 1500,
                record_count: 150,
            },
        ];

        let footer = IndexFooter { blocks };
        let encoded = footer.encode().unwrap();
        let decoded = IndexFooter::decode(&encoded).unwrap();

        assert_eq!(footer.blocks.len(), decoded.blocks.len());

        for (original, decoded) in footer.blocks.iter().zip(decoded.blocks.iter()) {
            assert_eq!(original.block_offset, decoded.block_offset);
            assert_eq!(original.block_size, decoded.block_size);
            assert_eq!(original.record_count, decoded.record_count);
        }
    }

    #[test]
    fn test_index_footer_empty_blocks() {
        let footer = IndexFooter { blocks: vec![] };

        let encoded = footer.encode().unwrap();
        let decoded = IndexFooter::decode(&encoded).unwrap();

        assert_eq!(footer.blocks.len(), decoded.blocks.len());
    }

    #[test]
    fn test_index_footer_invalid_magic() {
        let mut invalid_bytes = vec![0x00, 0x00, 0x00, 0x00]; // Wrong magic
        invalid_bytes.extend_from_slice(&encode_uleb128(10)); // index_len
        invalid_bytes.extend_from_slice(&encode_uleb128(0)); // block_count
        invalid_bytes.extend_from_slice(&0u32.to_le_bytes()); // CRC32C

        let result = IndexFooter::decode(&invalid_bytes);
        assert!(result.is_err());
        if let Err(crate::error::JacError::CorruptBlock) = result {
            // Expected error
        } else {
            panic!("Expected CorruptBlock error");
        }
    }

    #[test]
    fn test_index_footer_truncated() {
        let truncated_bytes = vec![0x49, 0x44, 0x58, 0x31]; // Just magic
        let result = IndexFooter::decode(&truncated_bytes);
        assert!(result.is_err());
        if let Err(crate::error::JacError::UnexpectedEof) = result {
            // Expected error
        } else {
            panic!("Expected UnexpectedEof error");
        }
    }

    #[test]
    fn test_index_footer_crc_mismatch() {
        let footer = IndexFooter {
            blocks: vec![create_test_block_entry()],
        };

        let mut encoded = footer.encode().unwrap();

        // Corrupt the CRC32C
        let crc_pos = encoded.len() - 4;
        encoded[crc_pos] = 0xFF;
        encoded[crc_pos + 1] = 0xFF;
        encoded[crc_pos + 2] = 0xFF;
        encoded[crc_pos + 3] = 0xFF;

        let result = IndexFooter::decode(&encoded);
        assert!(result.is_err());
        if let Err(crate::error::JacError::ChecksumMismatch) = result {
            // Expected error
        } else {
            panic!("Expected ChecksumMismatch error");
        }
    }

    #[test]
    fn test_index_footer_endianness() {
        let footer = IndexFooter {
            blocks: vec![create_test_block_entry()],
        };

        let encoded = footer.encode().unwrap();

        // Check that index magic is stored little-endian
        let magic_bytes = &encoded[0..4];
        let expected_magic_bytes = INDEX_MAGIC.to_le_bytes();
        assert_eq!(magic_bytes, &expected_magic_bytes);

        // Check that CRC32C is stored little-endian
        let crc_pos = encoded.len() - 4;
        let crc_bytes = &encoded[crc_pos..crc_pos + 4];
        let expected_crc = compute_crc32c(&encoded[0..crc_pos]);
        let expected_crc_bytes = expected_crc.to_le_bytes();
        assert_eq!(crc_bytes, &expected_crc_bytes);
    }

    #[test]
    fn test_index_footer_large_values() {
        let blocks = vec![
            BlockIndexEntry {
                block_offset: u64::MAX,
                block_size: usize::MAX,
                record_count: 1_000_000,
            },
            BlockIndexEntry {
                block_offset: 0,
                block_size: 0,
                record_count: 0,
            },
        ];

        let footer = IndexFooter { blocks };
        let encoded = footer.encode().unwrap();
        let decoded = IndexFooter::decode(&encoded).unwrap();

        assert_eq!(footer.blocks.len(), decoded.blocks.len());
        assert_eq!(
            footer.blocks[0].block_offset,
            decoded.blocks[0].block_offset
        );
        assert_eq!(footer.blocks[0].block_size, decoded.blocks[0].block_size);
        assert_eq!(
            footer.blocks[0].record_count,
            decoded.blocks[0].record_count
        );
        assert_eq!(
            footer.blocks[1].block_offset,
            decoded.blocks[1].block_offset
        );
        assert_eq!(footer.blocks[1].block_size, decoded.blocks[1].block_size);
        assert_eq!(
            footer.blocks[1].record_count,
            decoded.blocks[1].record_count
        );
    }

    #[test]
    fn test_index_footer_many_blocks() {
        let mut blocks = Vec::new();
        for i in 0..1000 {
            blocks.push(BlockIndexEntry {
                block_offset: i as u64 * 1000,
                block_size: 1000 + i,
                record_count: 100 + i,
            });
        }

        let footer = IndexFooter { blocks };
        let encoded = footer.encode().unwrap();
        let decoded = IndexFooter::decode(&encoded).unwrap();

        assert_eq!(footer.blocks.len(), decoded.blocks.len());

        for (original, decoded) in footer.blocks.iter().zip(decoded.blocks.iter()) {
            assert_eq!(original.block_offset, decoded.block_offset);
            assert_eq!(original.block_size, decoded.block_size);
            assert_eq!(original.record_count, decoded.record_count);
        }
    }

    #[test]
    fn test_index_footer_crc_verification() {
        let footer = IndexFooter {
            blocks: vec![create_test_block_entry()],
        };

        let encoded = footer.encode().unwrap();

        // Verify that the CRC32C is correct by manually computing it
        let crc_pos = encoded.len() - 4;
        let footer_without_crc = &encoded[0..crc_pos];
        let expected_crc = compute_crc32c(footer_without_crc);
        let stored_crc = u32::from_le_bytes([
            encoded[crc_pos],
            encoded[crc_pos + 1],
            encoded[crc_pos + 2],
            encoded[crc_pos + 3],
        ]);

        assert_eq!(expected_crc, stored_crc);
    }
}
