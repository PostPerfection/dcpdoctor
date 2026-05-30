//! Shared IMF validation logic.
//!
//! This crate contains all pure data structures and validation functions
//! for IMF (Interoperable Master Format) that are shared between
//! dcpdoctor-core (filesystem) and dcpdoctor-wasm (browser).
//!
//! No filesystem or WASM-specific dependencies — only operates on
//! parsed data and in-memory XML strings.

pub mod parse;
pub mod types;
pub mod validate;

pub use parse::{detect_application, parse_assetmap_ids, parse_edit_rate, parse_imf_cpl};
pub use types::*;
pub use validate::*;
