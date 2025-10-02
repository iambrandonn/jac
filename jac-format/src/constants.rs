//! Constants and magic numbers for JAC format

/// File magic bytes: "JAC" + version 0x01
pub const FILE_MAGIC: [u8; 4] = [0x4A, 0x41, 0x43, 0x01]; // "JAC\x01"

/// Block magic: "BLK1"
pub const BLOCK_MAGIC: u32 = 0x314B4C42; // "BLK1"

/// Index magic: "IDX1"
pub const INDEX_MAGIC: u32 = 0x31584449; // "IDX1"

/// Compressor codes
pub const COMPRESSOR_NONE: u8 = 0;
pub const COMPRESSOR_ZSTD: u8 = 1;
pub const COMPRESSOR_BROTLI: u8 = 2;
pub const COMPRESSOR_DEFLATE: u8 = 3;

/// File header flags (bit masks)
pub const FLAG_CANONICALIZE_KEYS: u32 = 1 << 0;
pub const FLAG_CANONICALIZE_NUMBERS: u32 = 1 << 1;
pub const FLAG_NESTED_OPAQUE: u32 = 1 << 2;

/// Type tag codes (3-bit)
pub const TAG_NULL: u8 = 0;
pub const TAG_BOOL: u8 = 1;
pub const TAG_INT: u8 = 2;
pub const TAG_DECIMAL: u8 = 3;
pub const TAG_STRING: u8 = 4;
pub const TAG_OBJECT: u8 = 5;
pub const TAG_ARRAY: u8 = 6;
pub const TAG_RESERVED: u8 = 7;

/// Encoding flags bitfield (per field segment)
pub const ENCODING_FLAG_DICTIONARY: u64 = 1 << 0;
pub const ENCODING_FLAG_DELTA: u64 = 1 << 1;
pub const ENCODING_FLAG_RLE: u64 = 1 << 2;      // reserved
pub const ENCODING_FLAG_BIT_PACKED: u64 = 1 << 3; // reserved

