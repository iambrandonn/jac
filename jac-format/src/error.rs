//! Error types for JAC format

use thiserror::Error;

/// JAC error types
#[derive(Debug, Error)]
pub enum JacError {
    #[error("Invalid magic bytes")]
    InvalidMagic,
    #[error("Unsupported version: {0}")]
    UnsupportedVersion(u8),
    #[error("Corrupt header")]
    CorruptHeader,
    #[error("Corrupt block")]
    CorruptBlock,
    #[error("Checksum mismatch")]
    ChecksumMismatch,
    #[error("Unexpected end of file")]
    UnexpectedEof,
    #[error("Decompression error: {0}")]
    DecompressError(String),
    #[error("Limit exceeded: {0}")]
    LimitExceeded(String),
    #[error("Type mismatch")]
    TypeMismatch,
    #[error("Dictionary index out of range")]
    DictionaryError,
    #[error("Unsupported feature: {0}")]
    UnsupportedFeature(String),
    #[error("Unsupported compression codec: {0}")]
    UnsupportedCompression(u8),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Internal error: {0}")]
    Internal(String),
}

/// Result type alias
pub type Result<T> = std::result::Result<T, JacError>;
