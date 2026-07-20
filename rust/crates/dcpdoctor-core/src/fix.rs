//! Automatic repair of common DCP issues flagged by validation.

use std::path::Path;

use crate::dcp;
use crate::hash::sha1_base64;
use crate::{Code, Note, Severity, Standard, VerifyOptions};

/// A single repair action that was applied.
#[derive(Debug, Clone)]
pub struct Repair {
    pub code: Code,
    pub description: String,
    pub file: std::path::PathBuf,
}

/// Result of a fix operation.
#[derive(Debug, Default)]
pub struct FixResult {
    pub repairs: Vec<Repair>,
    pub skipped: Vec<Note>,
}

impl FixResult {
    pub fn repair_count(&self) -> usize {
        self.repairs.len()
    }
}

/// Fix all auto-repairable issues in the given DCP directory.
/// Returns a summary of what was fixed and what was skipped.
pub fn fix_dcp(dcp_dir: &Path) -> FixResult {
    let mut result = FixResult::default();

    let dcp = match dcp::open_dcp(dcp_dir) {
        Ok(d) => d,
        Err(notes) => {
            // Can't even parse the DCP — nothing to fix
            result.skipped = notes;
            return result;
        }
    };

    // First validate to find issues (use strict mode so all fixable issues surface)
    let opts = VerifyOptions {
        check_hashes: true,
        check_signatures: false,
        check_picture_details: false,
        strict_smpte: true,
        ov: None,
    };
    let verify_result = crate::validate::verify_dcp(dcp_dir, &opts);

    for note in &verify_result.notes {
        match note.code {
            Code::PklHashMismatch => {
                // Recompute hashes and rewrite PKL
                // Handled below in batch
            }
            Code::SmpteNamespaceWrong | Code::InteropNamespaceWrong => {
                if let Some(ref file) = note.file
                    && fix_namespace(file, dcp.standard)
                {
                    result.repairs.push(Repair {
                        code: note.code,
                        description: format!(
                            "Fixed namespace in {}",
                            file.file_name().unwrap_or_default().to_string_lossy()
                        ),
                        file: file.clone(),
                    });
                }
            }
            Code::CplInvalidContentKind => {
                if let Some(ref file) = note.file
                    && fix_content_kind(file)
                {
                    result.repairs.push(Repair {
                        code: note.code,
                        description: format!(
                            "Normalized ContentKind in {}",
                            file.file_name().unwrap_or_default().to_string_lossy()
                        ),
                        file: file.clone(),
                    });
                }
            }
            _ => {
                // Not auto-fixable
                result.skipped.push(note.clone());
            }
        }
    }

    // Batch fix: recompute PKL hashes
    let id_to_path: std::collections::HashMap<&str, &str> = dcp
        .assetmap
        .assets
        .iter()
        .map(|a| (a.id.as_str(), a.path.as_str()))
        .collect();

    for (pkl_path, pkl) in &dcp.pkls {
        let mut pkl_modified = false;
        let mut xml = match std::fs::read_to_string(pkl_path) {
            Ok(s) => s,
            Err(_) => continue,
        };

        for pkl_asset in &pkl.assets {
            if let Some(&asset_rel) = id_to_path.get(pkl_asset.id.as_str()) {
                let full_path = dcp_dir.join(asset_rel);
                if !full_path.exists() || pkl_asset.hash.is_empty() {
                    continue;
                }
                match sha1_base64(&full_path) {
                    Ok(computed) if computed != pkl_asset.hash && xml.contains(&pkl_asset.hash) => {
                        xml = xml.replacen(&pkl_asset.hash, &computed, 1);
                        pkl_modified = true;
                        result.repairs.push(Repair {
                            code: Code::PklHashMismatch,
                            description: format!("Updated hash for {} in PKL", asset_rel),
                            file: pkl_path.clone(),
                        });
                    }
                    _ => {}
                }
            }
        }

        if pkl_modified && let Err(e) = std::fs::write(pkl_path, &xml) {
            result.skipped.push(Note {
                severity: Severity::Error,
                code: Code::PklHashMismatch,
                message: format!("Failed to write updated PKL: {}", e),
                file: Some(pkl_path.clone()),
                line: 0,
            });
        }
    }

    result
}

/// Fix XML namespace to match detected standard.
fn fix_namespace(file: &Path, standard: Standard) -> bool {
    let xml = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(_) => return false,
    };

    let (wrong, correct) = match standard {
        Standard::Smpte => (
            "http://www.digicine.com/PROTO-ASDCP-CPL-20040511#",
            "http://www.smpte-ra.org/schemas/429-7/2006/CPL",
        ),
        Standard::Interop => (
            "http://www.smpte-ra.org/schemas/429-7/2006/CPL",
            "http://www.digicine.com/PROTO-ASDCP-CPL-20040511#",
        ),
        Standard::Unknown => return false,
    };

    if !xml.contains(wrong) {
        return false;
    }

    let fixed = xml.replace(wrong, correct);
    std::fs::write(file, &fixed).is_ok()
}

/// Normalize ContentKind to lowercase canonical form.
fn fix_content_kind(file: &Path) -> bool {
    let xml = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(_) => return false,
    };

    // Find <ContentKind>...</ContentKind> and normalize the value
    let start_tag = "<ContentKind>";
    let end_tag = "</ContentKind>";
    let start = match xml.find(start_tag) {
        Some(i) => i + start_tag.len(),
        None => return false,
    };
    let end = match xml[start..].find(end_tag) {
        Some(i) => start + i,
        None => return false,
    };

    let original = &xml[start..end];
    let normalized = normalize_content_kind(original.trim());

    if normalized == original.trim() {
        return false;
    }

    let fixed = format!("{}{}{}", &xml[..start], normalized, &xml[end..]);
    std::fs::write(file, &fixed).is_ok()
}

/// Map common misspellings/variants to the canonical SMPTE content kinds.
fn normalize_content_kind(kind: &str) -> &'static str {
    match kind.to_lowercase().as_str() {
        "feature" | "features" | "feature film" => "feature",
        "trailer" | "trailers" => "trailer",
        "test" | "testing" => "test",
        "teaser" | "teasers" => "teaser",
        "rating" | "ratings" | "rating card" => "rating",
        "advertisement" | "ad" | "advert" | "advertising" => "advertisement",
        "short" | "shorts" | "short film" => "short",
        "transitional" | "transition" => "transitional",
        "psa" | "public service" => "psa",
        "policy" | "policies" | "policy trailer" => "policy",
        "episode" | "episodes" => "episode",
        _ => "feature", // Default fallback for unknown kinds
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_normalize_content_kind_variants() {
        assert_eq!(normalize_content_kind("Feature"), "feature");
        assert_eq!(normalize_content_kind("TRAILER"), "trailer");
        assert_eq!(normalize_content_kind("ad"), "advertisement");
        assert_eq!(normalize_content_kind("short film"), "short");
        assert_eq!(normalize_content_kind("PSA"), "psa");
    }

    #[test]
    fn test_fix_content_kind_in_xml() {
        let dir = TempDir::new().unwrap();
        let cpl = dir.path().join("cpl.xml");
        fs::write(
            &cpl,
            r#"<?xml version="1.0"?><CompositionPlaylist><ContentKind>TRAILER</ContentKind></CompositionPlaylist>"#,
        )
        .unwrap();

        assert!(fix_content_kind(&cpl));
        let result = fs::read_to_string(&cpl).unwrap();
        assert!(result.contains("<ContentKind>trailer</ContentKind>"));
    }

    #[test]
    fn test_fix_content_kind_already_correct() {
        let dir = TempDir::new().unwrap();
        let cpl = dir.path().join("cpl.xml");
        fs::write(
            &cpl,
            r#"<?xml version="1.0"?><CompositionPlaylist><ContentKind>feature</ContentKind></CompositionPlaylist>"#,
        )
        .unwrap();

        assert!(!fix_content_kind(&cpl));
    }

    #[test]
    fn test_fix_namespace_smpte() {
        let dir = TempDir::new().unwrap();
        let cpl = dir.path().join("cpl.xml");
        fs::write(
            &cpl,
            r#"<CompositionPlaylist xmlns="http://www.digicine.com/PROTO-ASDCP-CPL-20040511#"><Id>urn:uuid:test</Id></CompositionPlaylist>"#,
        )
        .unwrap();

        assert!(fix_namespace(&cpl, Standard::Smpte));
        let result = fs::read_to_string(&cpl).unwrap();
        assert!(result.contains("http://www.smpte-ra.org/schemas/429-7/2006/CPL"));
    }
}
