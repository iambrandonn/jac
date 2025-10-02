//! Error types for JAC format

use thiserror::Error;

/// JAC error types
#[derive(Debug, Error)]
pub enum JacError {
    /// Input does not start with the expected file magic bytes.
    #[error("Invalid magic bytes")]
    InvalidMagic,
    /// File version is not supported by this decoder.
    #[error("Unsupported version: {0}")]
    UnsupportedVersion(u8),
    /// File header contents are inconsistent or corrupt.
    #[error("Corrupt header")]
    CorruptHeader,
    /// Block data is corrupt or fails validation.
    #[error("Corrupt block")]
    CorruptBlock,
    /// CRC32C verification failed for a block or footer.
    #[error("Checksum mismatch")]
    ChecksumMismatch,
    /// Encountered unexpected end of input.
    #[error("Unexpected end of file")]
    UnexpectedEof,
    /// Underlying compression codec reported an error.
    #[error("Decompression error: {0}")]
    DecompressError(String),
    /// A configured security limit was exceeded.
    #[error("Limit exceeded: {0}")]
    LimitExceeded(String),
    /// Value could not be interpreted as the expected type.
    #[error("Type mismatch")]
    TypeMismatch,
    /// Dictionary access referenced an invalid entry.
    #[error("Dictionary index out of range")]
    DictionaryError,
    /// Encountered a feature that the implementation does not support.
    #[error("Unsupported feature: {0}")]
    UnsupportedFeature(String),
    /// Encountered an unknown or unsupported compression codec.
    #[error("Unsupported compression codec: {0}")]
    UnsupportedCompression(u8),
    /// I/O operation failed while reading or writing data.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// JSON parsing or serialization failed.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    /// Internal invariant was violated.
    #[error("Internal error: {0}")]
    Internal(String),
}

/// Result type alias
pub type Result<T> = std::result::Result<T, JacError>;
