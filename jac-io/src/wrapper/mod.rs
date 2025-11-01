//! JSON wrapper preprocessing module
//!
//! This module provides support for preprocessing wrapped/enveloped JSON structures
//! before compression. It enables JAC to handle common JSON API patterns without
//! requiring external preprocessing tools.
//!
//! # Supported Wrapper Types
//!
//! - **Pointer**: Navigate to a nested array using RFC 6901 JSON Pointers
//! - **Sections** (Phase 2): Concatenate multiple named arrays
//! - **KeyedMap** (Phase 3): Flatten object-of-objects into records
//!
//! # Security & Limits
//!
//! All wrapper modes enforce configurable limits to prevent resource exhaustion:
//! - Maximum pointer depth (default: 3, hard max: 10)
//! - Maximum buffer size (default: 16 MiB, hard max: 128 MiB)
//! - Maximum pointer length (default: 256, hard max: 2048)
//!
//! # Important Caveats
//!
//! Wrapper modes are **preprocessing transformations**. The original envelope
//! structure is **not preserved** and cannot be recovered from `jac unpack`.
//! If you need to preserve the original structure, archive the source file separately.

pub mod error;
pub mod pointer;
pub mod utils;

pub use error::WrapperError;
pub use pointer::PointerArrayStream;
