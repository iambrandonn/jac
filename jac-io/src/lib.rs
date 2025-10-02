//! JAC I/O - Streaming file I/O and high-level APIs
//!
//! This crate provides the file I/O layer and high-level APIs for JAC:
//!
//! - Streaming writers and readers
//! - High-level compression/decompression functions
//! - Parallel processing support
//! - Field projection APIs

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod writer;
pub mod reader;
pub mod parallel;

// Re-export commonly used types
pub use jac_format::{JacError, Result, Limits, TypeTag};

