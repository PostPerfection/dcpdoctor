//! Advanced DCP validation: BV2.1 compliance, manifest comparison, batch summary.

use std::path::Path;

use serde::Serialize;

use crate::{Code, Note, Severity, Standard};

/// Check BV2.1 compliance for a DCP directory.
pub fn check_bv21_compliance(dcp_dir: &Path, standard: Standard) -> Vec<Note> {
    let mut notes = Vec::new();
    let path_buf = Some(dcp_dir.to_path_buf());

    if standard != Standard::Smpte {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::SmpteNamespaceWrong,
            message: "BV2.1 requires SMPTE standard; this DCP uses Interop".into(),
            file: path_buf,
            line: 0,
        });
        return notes;
    }

    // 1. ASSETMAP must be named ASSETMAP.xml
    if !dcp_dir.join("ASSETMAP.xml").exists() {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::SmpteNamingViolation,
            message: "BV2.1 requires ASSETMAP.xml filename".into(),
            file: path_buf.clone(),
            line: 0,
        });
    }

    // 2. PKL must have .xml extension
    if let Ok(entries) = std::fs::read_dir(dcp_dir) {
        for entry in entries.flatten() {
            let fname = entry.file_name().to_string_lossy().to_string();
            let lower = fname.to_lowercase();
            if lower.contains("pkl") && !fname.ends_with(".xml") {
                notes.push(Note {
                    severity: Severity::Warning,
                    code: Code::SmpteNamingViolation,
                    message: format!("BV2.1: PKL file should have .xml extension: {fname}"),
                    file: Some(entry.path()),
                    line: 0,
                });
            }
        }
    }

    // 3. CPL checks
    if let Ok(entries) = std::fs::read_dir(dcp_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() || path.extension().is_none_or(|ext| ext != "xml") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            if !content.contains("CompositionPlaylist") {
                continue;
            }

            let cpl_path = Some(path.clone());

            // ContentVersion required
            if !content.contains("<ContentVersion>") {
                notes.push(Note {
                    severity: Severity::Warning,
                    code: Code::MissingRequiredElement,
                    message: "BV2.1 requires ContentVersion in CPL".into(),
                    file: cpl_path.clone(),
                    line: 0,
                });
            }

            // ExtensionMetadata recommended
            if !content.contains("<ExtensionMetadata") {
                notes.push(Note {
                    severity: Severity::Info,
                    code: Code::MissingRequiredElement,
                    message: "BV2.1 recommends ExtensionMetadata in CPL".into(),
                    file: cpl_path.clone(),
                    line: 0,
                });
            }

            // MainMarkers in first reel
            if !content.contains("<MainMarkers>") {
                notes.push(Note {
                    severity: Severity::Warning,
                    code: Code::MarkerMissing,
                    message: "BV2.1 requires MainMarkers in first reel".into(),
                    file: cpl_path.clone(),
                    line: 0,
                });
            }

            // EditRate check
            let rate_re = regex_lite::Regex::new(r"<EditRate>(\d+)\s+(\d+)</EditRate>").unwrap();
            if let Some(cap) = rate_re.captures(&content) {
                let num: f64 = cap[1].parse().unwrap_or(0.0);
                let den: f64 = cap[2].parse().unwrap_or(1.0);
                if den > 0.0 {
                    let fps = num / den;
                    let valid =
                        fps == 24.0 || fps == 25.0 || fps == 30.0 || fps == 48.0 || fps == 60.0;
                    if !valid {
                        notes.push(Note {
                            severity: Severity::Warning,
                            code: Code::CplInvalidEditRate,
                            message: format!(
                                "BV2.1: EditRate {} {} is not an approved rate",
                                &cap[1], &cap[2]
                            ),
                            file: cpl_path,
                            line: 0,
                        });
                    }
                }
            }
        }
    }

    notes
}

/// Compare DCP contents against a delivery manifest (JSON).
pub fn compare_manifest(dcp_dir: &Path, manifest_path: &Path) -> Vec<Note> {
    let mut notes = Vec::new();

    let content = match std::fs::read_to_string(manifest_path) {
        Ok(c) => c,
        Err(_) => {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::AssetNotFound,
                message: format!("Cannot open manifest file: {}", manifest_path.display()),
                file: Some(manifest_path.to_path_buf()),
                line: 0,
            });
            return notes;
        }
    };

    let filename_re = regex_lite::Regex::new(r#""filename"\s*:\s*"([^"]+)""#).unwrap();
    let size_re = regex_lite::Regex::new(r#""size"\s*:\s*(\d+)"#).unwrap();

    let mut manifest_files = std::collections::HashSet::new();

    for cap in filename_re.captures_iter(&content) {
        let filename = &cap[1];
        manifest_files.insert(filename.to_string());

        let full_path = dcp_dir.join(filename);
        if !full_path.exists() {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::AssetNotFound,
                message: format!("Manifest asset not found in DCP: {filename}"),
                file: Some(dcp_dir.to_path_buf()),
                line: 0,
            });
        }
    }

    // Check sizes
    for cap in size_re.captures_iter(&content) {
        let _expected_size: u64 = cap[1].parse().unwrap_or(0);
        // Size checking would require matching each size to its filename
        // which needs more complex JSON parsing — skip for now
    }

    // Check for extra files
    if let Ok(entries) = std::fs::read_dir(dcp_dir) {
        for entry in entries.flatten() {
            if !entry.path().is_file() {
                continue;
            }
            let fname = entry.file_name().to_string_lossy().to_string();
            if !manifest_files.contains(&fname) {
                notes.push(Note {
                    severity: Severity::Info,
                    code: Code::AssetNotFound,
                    message: format!("File in DCP not listed in manifest: {fname}"),
                    file: Some(entry.path()),
                    line: 0,
                });
            }
        }
    }

    notes
}

/// Batch validation result for a single DCP.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BatchResult {
    pub dcp_path: String,
    pub passed: bool,
    pub errors: u32,
    pub warnings: u32,
    pub standard: String,
}

/// Generate a text summary for batch validation results.
pub fn write_batch_summary(results: &[BatchResult]) -> String {
    let total = results.len();
    let passed = results.iter().filter(|r| r.passed).count();

    let mut out = String::new();
    out.push_str("DcpDoctor Batch Summary\n");
    out.push_str("=======================\n\n");
    out.push_str(&format!(
        "Total: {total}  Passed: {passed}  Failed: {}\n\n",
        total - passed
    ));

    out.push_str(&format!(
        "{:<50} {:<10} {:<10} {:<10} {}\n",
        "DCP Path", "Status", "Errors", "Warnings", "Standard"
    ));
    out.push_str(&"-".repeat(90));
    out.push('\n');

    for r in results {
        let mut path_str = r.dcp_path.clone();
        if path_str.len() > 48 {
            path_str.truncate(45);
            path_str.push_str("...");
        }
        out.push_str(&format!(
            "{:<50} {:<10} {:<10} {:<10} {}\n",
            path_str,
            if r.passed { "PASS" } else { "FAIL" },
            r.errors,
            r.warnings,
            r.standard
        ));
    }

    out
}
