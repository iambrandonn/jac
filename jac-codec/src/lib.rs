//! JAC Codec - Encoder/decoder engines
//!
//! This crate provides the core encoding and decoding engines for JAC:
//!
//! - Column builders for converting records to columnar format
//! - Field segment encoding/decoding
//! - Block builders for aggregating columns
//! - Segment decoders for field projection

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod block_builder;
pub mod block_decode;
pub mod column;
pub mod segment;
pub mod segment_decode;

// Re-export commonly used types
pub use jac_format::{
    BlockHeader, Decimal, FieldDirectoryEntry, JacError, Limits, Result, TypeTag,
};

// Re-export our own types
pub use block_builder::{BlockBuilder, BlockData, BlockFinish, TryAddRecordOutcome};
pub use block_decode::{BlockDecoder, DecompressOpts};
pub use column::{ColumnBuilder, FieldSegment};
pub use segment::FieldSegment as Segment;
pub use segment_decode::FieldSegmentDecoder;

// Compression options

/// Compression options for encoding
#[derive(Debug, Clone)]
pub struct CompressOpts {
    /// Target number of records per block
    pub block_target_records: usize,
    /// Default compression codec
    pub default_codec: Codec,
    /// Canonicalize keys (lexicographic order)
    pub canonicalize_keys: bool,
    /// Canonicalize numbers (scientific notation, trim trailing zeros)
    pub canonicalize_numbers: bool,
    /// Nested objects/arrays are opaque (v1 behavior)
    pub nested_opaque: bool,
    /// Maximum dictionary entries per field
    pub max_dict_entries: usize,
    /// Security limits
    pub limits: Limits,
}

impl Default for CompressOpts {
    fn default() -> Self {
        Self {
            block_target_records: 100_000,
            default_codec: Codec::Zstd(6),
            canonicalize_keys: false,
            canonicalize_numbers: false,
            nested_opaque: true, // Must be true in v1
            max_dict_entries: 4_096,
            limits: Limits::default(),
        }
    }
}

/// Compression codec
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    /// No compression
    None,
    /// Zstandard compression with level
    Zstd(u8),
    /// Brotli compression (not implemented in v0.1.0)
    Brotli(u8),
    /// Deflate compression (not implemented in v0.1.0)
    Deflate(u8),
}

impl Codec {
    /// Get compressor ID for wire format
    pub fn compressor_id(&self) -> u8 {
        match self {
            Codec::None => 0,
            Codec::Zstd(_) => 1,
            Codec::Brotli(_) => 2,
            Codec::Deflate(_) => 3,
        }
    }

    /// Get compression level
    pub fn level(&self) -> u8 {
        match self {
            Codec::None => 0,
            Codec::Zstd(level) => *level,
            Codec::Brotli(level) => *level,
            Codec::Deflate(level) => *level,
        }
    }
}
