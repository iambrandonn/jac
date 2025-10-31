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
pub use block_builder::{
    compress_block_segments, BlockBuilder, BlockData, BlockFinish, TryAddRecordOutcome,
    UncompressedBlockData,
};
pub use block_decode::{BlockDecoder, DecompressOpts};
pub use column::{ColumnBuilder, FieldSegment};
pub use segment::FieldSegment as Segment;
pub use segment_decode::FieldSegmentDecoder;

use std::convert::TryFrom;

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
    /// Zstandard compression with explicit thread count
    ZstdWithThreads {
        /// Compression level (compatible with FileHeader metadata)
        level: i32,
        /// Requested encoder threads (>= 1)
        threads: usize,
    },
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
            Codec::ZstdWithThreads { .. } => 1,
            Codec::Brotli(_) => 2,
            Codec::Deflate(_) => 3,
        }
    }

    /// Get compression level
    pub fn level(&self) -> u8 {
        match self {
            Codec::None => 0,
            Codec::Zstd(level) => *level,
            Codec::ZstdWithThreads { level, .. } => {
                u8::try_from(*level).unwrap_or(if *level < 0 { 0 } else { u8::MAX })
            }
            Codec::Brotli(level) => *level,
            Codec::Deflate(level) => *level,
        }
    }

    /// Return the zstd compression level as `i32` when applicable.
    pub fn zstd_level_i32(&self) -> Option<i32> {
        match self {
            Codec::Zstd(level) => Some(i32::from(*level)),
            Codec::ZstdWithThreads { level, .. } => Some(*level),
            _ => None,
        }
    }

    /// Return the configured encoder thread count when applicable.
    pub fn zstd_threads(&self) -> Option<usize> {
        match self {
            Codec::ZstdWithThreads { threads, .. } => Some(*threads),
            _ => None,
        }
    }
}

/// Configure codec for sequential or parallel usage.
///
/// When `single_threaded` is true, zstd codecs are wrapped to force the encoder
/// to use a single internal thread. This prevents oversubscription when the
/// caller executes multiple compression tasks in parallel (e.g., via Rayon).
pub fn configure_codec_for_parallel(codec: Codec, single_threaded: bool) -> Codec {
    match codec {
        Codec::Zstd(level) if single_threaded => Codec::ZstdWithThreads {
            level: i32::from(level),
            threads: 1,
        },
        Codec::ZstdWithThreads { level, .. } if single_threaded => {
            Codec::ZstdWithThreads { level, threads: 1 }
        }
        _ => codec,
    }
}
