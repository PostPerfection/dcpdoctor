use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

mod assetmap;
mod cpl;
mod hash;
mod naming;
mod pkl;
mod validate;

/// Severity level for validation notes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// A single validation finding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    pub file: Option<String>,
}

/// Validation result returned to JavaScript.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    pub valid: bool,
    pub standard: String,
    pub notes: Vec<Note>,
    pub summary: Summary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub errors: usize,
    pub warnings: usize,
    pub info: usize,
    pub files_checked: usize,
    pub hashes_verified: usize,
    pub hashes_failed: usize,
    pub hashes_skipped: usize,
}

/// A file entry passed from JavaScript.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    /// Relative path within the DCP folder (e.g. "ASSETMAP.xml", "PKL_abc.xml")
    pub path: String,
    /// File content as base64-encoded string (for binary) or raw UTF-8 (for XML).
    /// May be None if file was too large to read in browser.
    pub content: Option<String>,
    /// Whether content is base64-encoded
    pub is_base64: bool,
    /// File size in bytes
    pub size: u64,
    /// Whether content was skipped (file too large for browser)
    #[serde(default)]
    pub skipped: bool,
}

/// Main entry point: validate a DCP from a set of file entries.
///
/// Accepts a JSON array of FileEntry objects and returns a ValidationResult as JSON.
#[wasm_bindgen]
pub fn validate_dcp(files_json: &str) -> String {
    let files: Vec<FileEntry> = match serde_json::from_str(files_json) {
        Ok(f) => f,
        Err(e) => {
            let result = ValidationResult {
                valid: false,
                standard: "unknown".to_string(),
                notes: vec![Note {
                    severity: Severity::Error,
                    code: "parse_error".to_string(),
                    message: format!("Failed to parse input: {e}"),
                    file: None,
                }],
                summary: Summary {
                    errors: 1,
                    warnings: 0,
                    info: 0,
                    files_checked: 0,
                    hashes_verified: 0,
                    hashes_failed: 0,
                    hashes_skipped: 0,
                },
            };
            return serde_json::to_string(&result).unwrap();
        }
    };

    let result = validate::run_validation(&files);
    serde_json::to_string(&result).unwrap()
}

/// Compute SHA-1 hash of raw bytes, returned as base64.
#[wasm_bindgen]
pub fn sha1_base64(data: &[u8]) -> String {
    hash::compute_sha1_base64(data)
}

/// Get the version of dcpdoctor-wasm.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
