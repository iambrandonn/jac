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

pub mod column;
pub mod segment;
pub mod block_builder;
pub mod segment_decode;
pub mod block_decode;

// Re-export commonly used types
pub use jac_format::{JacError, Result, Limits, TypeTag};

