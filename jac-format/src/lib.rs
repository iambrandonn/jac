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

pub mod constants;
pub mod varint;
pub mod bitpack;
pub mod checksum;
pub mod error;
pub mod limits;
pub mod header;
pub mod block;
pub mod footer;
pub mod decimal;
pub mod types;

// Re-export commonly used types
pub use error::{JacError, Result};
pub use limits::Limits;
pub use types::TypeTag;
pub use header::FileHeader;
pub use block::{BlockHeader, FieldDirectoryEntry};
pub use footer::{IndexFooter, BlockIndexEntry};
pub use decimal::Decimal;
