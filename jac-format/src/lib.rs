//! JAC Format - Core primitives for JSON-Aware Compression
//!
//! This crate provides the fundamental encoding/decoding utilities for the JAC format
//! with no I/O dependencies. It includes:
//!
//! - Magic numbers and constants
//! - Variable-length integer encoding (ULEB128/ZigZag)
//! - Bit packing utilities
//! - CRC32C checksums
//! - Error types
//! - Security limits
//! - File/block structures
//! - Decimal encoding
//! - Type tags

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod bitpack;
pub mod block;
pub mod checksum;
pub mod constants;
pub mod decimal;
pub mod error;
pub mod footer;
pub mod header;
pub mod limits;
pub mod types;
pub mod varint;

// Re-export commonly used types
pub use block::{BlockHeader, FieldDirectoryEntry};
pub use decimal::Decimal;
pub use error::{JacError, Result};
pub use footer::{BlockIndexEntry, IndexFooter};
pub use header::{ContainerFormat, FileHeader};
pub use limits::Limits;
pub use types::TypeTag;

/// Compression codec options
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    /// No compression
    None,
    /// Zstandard compression with specified level (1-22)
    Zstd(u8),
    /// Brotli compression (unimplemented in v0.1.0)
    Brotli(u8),
    /// Deflate compression (unimplemented in v0.1.0)
    Deflate(u8),
}

impl Codec {
    /// Get the compressor ID for this codec
    pub fn compressor_id(&self) -> u8 {
        match self {
            Codec::None => constants::COMPRESSOR_NONE,
            Codec::Zstd(_) => constants::COMPRESSOR_ZSTD,
            Codec::Brotli(_) => constants::COMPRESSOR_BROTLI,
            Codec::Deflate(_) => constants::COMPRESSOR_DEFLATE,
        }
    }

    /// Get the compression level for this codec
    pub fn level(&self) -> u8 {
        match self {
            Codec::None => 0,
            Codec::Zstd(level) => *level,
            Codec::Brotli(level) => *level,
            Codec::Deflate(level) => *level,
        }
    }

    /// Check if this codec is supported in v0.1.0
    pub fn is_supported(&self) -> bool {
        matches!(self, Codec::None | Codec::Zstd(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_codec_compressor_id() {
        assert_eq!(Codec::None.compressor_id(), constants::COMPRESSOR_NONE);
        assert_eq!(Codec::Zstd(15).compressor_id(), constants::COMPRESSOR_ZSTD);
        assert_eq!(
            Codec::Brotli(11).compressor_id(),
            constants::COMPRESSOR_BROTLI
        );
        assert_eq!(
            Codec::Deflate(6).compressor_id(),
            constants::COMPRESSOR_DEFLATE
        );
    }

    #[test]
    fn test_codec_level() {
        assert_eq!(Codec::None.level(), 0);
        assert_eq!(Codec::Zstd(15).level(), 15);
        assert_eq!(Codec::Brotli(11).level(), 11);
        assert_eq!(Codec::Deflate(6).level(), 6);
    }

    #[test]
    fn test_codec_support() {
        assert!(Codec::None.is_supported());
        assert!(Codec::Zstd(15).is_supported());
        assert!(!Codec::Brotli(11).is_supported());
        assert!(!Codec::Deflate(6).is_supported());
    }
}
