//! JSON wrapper preprocessing module
//!
//! This module provides support for preprocessing wrapped/enveloped JSON structures
//! before compression. It enables JAC to handle common JSON API patterns without
//! requiring external preprocessing tools.
//!
//! # Supported Wrapper Types
//!
//! - **Pointer**: Navigate to a nested array using RFC 6901 JSON Pointers
//! - **Sections**: Concatenate multiple named arrays from a single object
//! - **KeyedMap**: Flatten object-of-objects into records with key injection
//! - **ArrayWithHeaders**: Convert CSV-like arrays (first row = headers) to records
//! - **Plugin**: Custom wrapper implementations via the plugin system
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

pub mod array_headers;
pub mod error;
pub mod map;
pub mod plugin;
pub mod pointer;
pub mod sections;
pub mod utils;

pub use array_headers::ArrayHeadersStream;
pub use error::WrapperError;
pub use map::KeyedMapStream;
pub use plugin::{
    FieldHint, FieldType, SchemaHints, WrapperPlugin, WrapperPluginMetadata, WrapperPluginRegistry,
};
pub use pointer::PointerArrayStream;
pub use sections::SectionsStream;
