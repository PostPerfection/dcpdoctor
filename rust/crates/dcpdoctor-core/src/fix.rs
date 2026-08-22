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
        ..Default::default()
    };
    let verify_result = crate::validate::verify_dcp(dcp_dir, &opts);

    // PKL hash mismatches are repaired in the batch pass below. a note that
    // reaches neither list would vanish from the report.
    let mut pending_hash_mismatches: Vec<Note> = Vec::new();

    for note in &verify_result.notes {
        match note.code {
            Code::PklHashMismatch => {
                pending_hash_mismatches.push(note.clone());
            }
            Code::SmpteNamespaceWrong | Code::InteropNamespaceWrong => {
                let fixed = note
                    .file
                    .as_ref()
                    .is_some_and(|file| fix_namespace(file, dcp.standard));
                match (fixed, &note.file) {
                    (true, Some(file)) => result.repairs.push(Repair {
                        code: note.code,
                        description: format!(
                            "Fixed namespace in {}",
                            file.file_name().unwrap_or_default().to_string_lossy()
                        ),
                        file: file.clone(),
                    }),
                    _ => result.skipped.push(note.clone()),
                }
            }
            Code::CplInvalidContentKind => {
                let fixed = note
                    .file
                    .as_ref()
                    .is_some_and(|file| fix_content_kind(file));
                match (fixed, &note.file) {
                    (true, Some(file)) => result.repairs.push(Repair {
                        code: note.code,
                        description: format!(
                            "Normalized ContentKind in {}",
                            file.file_name().unwrap_or_default().to_string_lossy()
                        ),
                        file: file.clone(),
                    }),
                    _ => result.skipped.push(note.clone()),
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

    let mut repaired_assets: std::collections::HashSet<std::path::PathBuf> =
        std::collections::HashSet::new();
    let mut failure_reasons: std::collections::HashMap<std::path::PathBuf, String> =
        std::collections::HashMap::new();

    for (pkl_path, pkl) in &dcp.pkls {
        let mut pkl_modified = false;
        let repairs_before_this_pkl = result.repairs.len();
        let mut rewritten_in_this_pkl: Vec<std::path::PathBuf> = Vec::new();
        let mut xml = match std::fs::read_to_string(pkl_path) {
            Ok(s) => s,
            Err(e) => {
                result.skipped.push(Note {
                    severity: Severity::Error,
                    code: Code::PklHashMismatch,
                    message: format!("Cannot read PKL to rewrite hashes: {e}"),
                    file: Some(pkl_path.clone()),
                    line: 0,
                });
                continue;
            }
        };

        for pkl_asset in &pkl.assets {
            if let Some(&asset_rel) = id_to_path.get(pkl_asset.id.as_str()) {
                let full_path = dcp_dir.join(asset_rel);
                if !full_path.exists() {
                    failure_reasons.insert(full_path, "asset file not found".into());
                    continue;
                }
                if pkl_asset.hash.is_empty() {
                    failure_reasons.insert(full_path, "the PKL records no hash".into());
                    continue;
                }
                match sha1_base64(&full_path) {
                    Ok(computed) => {
                        if computed == pkl_asset.hash {
                            continue;
                        }
                        if !xml.contains(&pkl_asset.hash) {
                            failure_reasons.insert(
                                full_path,
                                "the hash the PKL records was not found in its text".into(),
                            );
                            continue;
                        }
                        xml = xml.replacen(&pkl_asset.hash, &computed, 1);
                        pkl_modified = true;
                        repaired_assets.insert(full_path.clone());
                        rewritten_in_this_pkl.push(full_path);
                        result.repairs.push(Repair {
                            code: Code::PklHashMismatch,
                            description: format!("Updated hash for {} in PKL", asset_rel),
                            file: pkl_path.clone(),
                        });
                    }
                    Err(e) => {
                        failure_reasons.insert(full_path, format!("could not hash the asset: {e}"));
                    }
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
            // the new hashes never reached the file, so take the repairs back
            result.repairs.truncate(repairs_before_this_pkl);
            for asset in rewritten_in_this_pkl {
                repaired_assets.remove(&asset);
                failure_reasons.insert(asset, format!("the updated PKL could not be written: {e}"));
            }
        }
    }

    for note in pending_hash_mismatches {
        let repaired = note
            .file
            .as_ref()
            .is_some_and(|f| repaired_assets.contains(f));
        if repaired {
            continue;
        }
        let reason = note
            .file
            .as_ref()
            .and_then(|f| failure_reasons.get(f))
            .cloned()
            .unwrap_or_else(|| "no PKL entry matched this asset".into());
        result.skipped.push(Note {
            message: format!("{}, not repaired: {reason}", note.message),
            ..note
        });
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

    /// Copy the committed SMPTE package into a temp dir with `edits` applied to
    /// `file`, so each case is one deliberate deviation from a real package.
    fn mutated_package(file: &str, edits: &[(&str, &str)]) -> TempDir {
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/valid_smpte");
        let dir = TempDir::new().unwrap();
        for entry in fs::read_dir(source).unwrap().flatten() {
            fs::copy(entry.path(), dir.path().join(entry.file_name())).unwrap();
        }
        let target = dir.path().join(file);
        let mut xml = fs::read_to_string(&target).unwrap();
        for (from, to) in edits {
            assert!(xml.contains(from), "the package's {file} has no {from:?}");
            xml = xml.replace(from, to);
        }
        fs::write(&target, xml).unwrap();
        dir
    }

    #[test]
    fn a_content_kind_the_repair_cannot_reach_is_reported_as_skipped() {
        // a scope attribute is legal in SMPTE CPLs and hides the element from
        // the repair, which searches for the bare "<ContentKind>" tag
        let dir = mutated_package(
            "cpl.xml",
            &[(
                "<ContentKind>test</ContentKind>",
                r#"<ContentKind scope="http://www.smpte-ra.org/schemas/429-7/2006/CPL#standard-content">nonsense</ContentKind>"#,
            )],
        );

        let result = fix_dcp(dir.path());

        assert!(
            !result
                .repairs
                .iter()
                .any(|r| r.code == Code::CplInvalidContentKind),
            "nothing was rewritten, so no repair may be claimed: {:?}",
            result.repairs
        );
        assert!(
            result
                .skipped
                .iter()
                .any(|n| n.code == Code::CplInvalidContentKind),
            "the unrepaired ContentKind must reach the report: {:?}",
            result.skipped
        );
    }

    #[test]
    fn a_hash_mismatch_the_repair_cannot_write_is_reported_as_skipped() {
        let dir = mutated_package(
            "pkl.xml",
            &[(
                "<Hash>pDjIK8UaYOLZLpbbBBI0hFVQbXE=</Hash>",
                "<Hash>AAAAK8UaYOLZLpbbBBI0hFVQbXE=</Hash>",
            )],
        );
        let pkl = dir.path().join("pkl.xml");
        let mut permissions = fs::metadata(&pkl).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&pkl, permissions).unwrap();
        if fs::OpenOptions::new().write(true).open(&pkl).is_ok() {
            // running as root, where read-only says nothing about writability
            return;
        }

        let result = fix_dcp(dir.path());

        assert!(
            !result
                .repairs
                .iter()
                .any(|r| r.code == Code::PklHashMismatch),
            "the PKL was never written, so no repair may be claimed: {:?}",
            result.repairs
        );
        assert!(
            result
                .skipped
                .iter()
                .any(|n| n.code == Code::PklHashMismatch && n.message.contains("not repaired")),
            "the unrepaired hash mismatch must reach the report: {:?}",
            result.skipped
        );
    }

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
