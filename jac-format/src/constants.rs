//! Constants and magic numbers for JAC format

/// File magic bytes: "JAC" + version 0x01
pub const FILE_MAGIC: [u8; 4] = [0x4A, 0x41, 0x43, 0x01]; // "JAC\x01"

/// Block magic: "BLK1"
pub const BLOCK_MAGIC: u32 = 0x314B4C42; // "BLK1"

/// Index magic: "IDX1"
pub const INDEX_MAGIC: u32 = 0x31584449; // "IDX1"

/// Compressor ID for uncompressed segments.
pub const COMPRESSOR_NONE: u8 = 0;
/// Compressor ID for Zstandard segments.
pub const COMPRESSOR_ZSTD: u8 = 1;
/// Compressor ID reserved for Brotli segments.
pub const COMPRESSOR_BROTLI: u8 = 2;
/// Compressor ID reserved for Deflate segments.
pub const COMPRESSOR_DEFLATE: u8 = 3;

/// Flag enabling deterministic key ordering.
pub const FLAG_CANONICALIZE_KEYS: u32 = 1 << 0;
/// Flag enabling canonical number formatting.
pub const FLAG_CANONICALIZE_NUMBERS: u32 = 1 << 1;
/// Flag indicating nested values remain opaque blobs.
pub const FLAG_NESTED_OPAQUE: u32 = 1 << 2;
/// Bit offset for the container format hint stored in the header flags.
pub const FLAG_CONTAINER_HINT_SHIFT: u32 = 3;
/// Mask covering the two bits reserved for the container format hint.
pub const FLAG_CONTAINER_HINT_MASK: u32 = 0b11 << FLAG_CONTAINER_HINT_SHIFT;

/// Type tag representing a `null` value.
pub const TAG_NULL: u8 = 0;
/// Type tag representing a boolean value.
pub const TAG_BOOL: u8 = 1;
/// Type tag representing a signed integer value.
pub const TAG_INT: u8 = 2;
/// Type tag representing an exact decimal value.
pub const TAG_DECIMAL: u8 = 3;
/// Type tag representing a UTF-8 string value.
pub const TAG_STRING: u8 = 4;
/// Type tag representing an embedded JSON object.
pub const TAG_OBJECT: u8 = 5;
/// Type tag representing an embedded JSON array.
pub const TAG_ARRAY: u8 = 6;
/// Reserved type tag; decoders must reject it.
pub const TAG_RESERVED: u8 = 7;

/// Field segment flag indicating dictionary encoding.
pub const ENCODING_FLAG_DICTIONARY: u64 = 1 << 0;
/// Field segment flag indicating delta encoding for integers.
pub const ENCODING_FLAG_DELTA: u64 = 1 << 1;
/// Reserved field segment flag for run-length encoding.
pub const ENCODING_FLAG_RLE: u64 = 1 << 2; // reserved
/// Reserved field segment flag for bit-packed payloads.
pub const ENCODING_FLAG_BIT_PACKED: u64 = 1 << 3; // reserved
