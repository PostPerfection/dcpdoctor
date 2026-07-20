use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wasm_bindgen::prelude::*;

mod hash;
pub mod imf;
pub mod j2k;
mod j2k_validate;
pub mod mxf;
mod mxf_validate;
mod naming;
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
    /// Map of file path -> expected SHA-1 base64 hash (from PKL)
    #[serde(default)]
    pub asset_hashes: HashMap<String, String>,
    /// MXF metadata for each binary asset (path -> metadata)
    #[serde(default)]
    pub mxf_info: HashMap<String, mxf::MxfMetadata>,
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
                asset_hashes: HashMap::new(),
                mxf_info: HashMap::new(),
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

/// Streaming SHA-1 hasher for incremental hashing of large files.
#[wasm_bindgen]
pub struct Sha1Hasher {
    inner: sha1::Sha1,
}

impl Default for Sha1Hasher {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl Sha1Hasher {
    /// Create a new streaming SHA-1 hasher.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Sha1Hasher {
        use sha1::Digest;
        Sha1Hasher {
            inner: sha1::Sha1::new(),
        }
    }

    /// Feed a chunk of bytes into the hasher.
    pub fn update(&mut self, chunk: &[u8]) {
        use sha1::Digest;
        self.inner.update(chunk);
    }

    /// Finalize and return the SHA-1 digest as base64.
    pub fn finalize(self) -> String {
        use base64::engine::general_purpose::STANDARD as BASE64;
        use base64::Engine;
        use sha1::Digest;
        let result = self.inner.finalize();
        BASE64.encode(result)
    }
}

/// Parse MXF header metadata from raw bytes.
///
/// Accepts the first ~1 MB of an MXF file and returns MxfMetadata as JSON.
/// This extracts picture/sound descriptors and writer info without any native deps.
#[wasm_bindgen]
pub fn parse_mxf_header(data: &[u8]) -> String {
    let metadata = mxf::parse_mxf(data);
    serde_json::to_string(&metadata).unwrap()
}

/// Validate MXF header bytes and return notes as JSON.
///
/// Accepts the first ~1 MB of an MXF file and a path identifier.
/// Returns a JSON array of Note objects with DCI compliance findings.
/// Also parses embedded J2K codestream headers for JPEG 2000 profile validation.
#[wasm_bindgen]
pub fn validate_mxf_file(data: &[u8], path: &str) -> String {
    let metadata = mxf::parse_mxf(data);
    let mut notes = mxf_validate::validate_mxf(path, &metadata);

    // If this is a picture MXF, try to parse J2K codestream header
    if metadata.essence_type == mxf::EssenceType::Jpeg2000 {
        if let Some(j2k_header) = j2k::parse_j2k_from_mxf(data) {
            let j2k_notes = j2k_validate::validate_j2k(path, &j2k_header);
            notes.extend(j2k_notes);
        }
    }

    serde_json::to_string(&notes).unwrap()
}

/// OV-aware IMF supplemental validation for the browser.
///
/// Since the browser has no filesystem, the OV can't be a path: pass its
/// available asset ids (a JSON array of uuid strings, `urn:uuid:` prefix
/// optional) alongside the supplemental CPL + ASSETMAP XML. A ref that resolves
/// in the OV passes; a ref in neither package is a `cross_ref_broken`. Returns
/// a JSON array of Note objects.
///
/// UI wiring (a second OV-folder drop in web/) is not yet built; this exposes
/// the core capability at the binding so a future OV-upload flow can call it.
#[wasm_bindgen]
pub fn validate_imf_supplemental(
    cpl_xml: &str,
    assetmap_xml: &str,
    ov_asset_ids_json: &str,
    cpl_path: &str,
) -> String {
    let ov_ids: std::collections::HashSet<String> =
        serde_json::from_str::<Vec<String>>(ov_asset_ids_json)
            .unwrap_or_default()
            .into_iter()
            .map(|id| id.strip_prefix("urn:uuid:").unwrap_or(&id).to_string())
            .collect();
    let notes = imf::validate_imf_supplemental(cpl_xml, assetmap_xml, &ov_ids, cpl_path);
    serde_json::to_string(&notes).unwrap()
}

/// Get the version of dcpdoctor-wasm.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
