//! Advanced DCP validation: BV2.1 compliance.

use std::path::Path;

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
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    notes.push(
                        Note::warning(
                            Code::CheckSkipped,
                            format!(
                                "BV2.1 CPL element checks did not run, cannot read {}: {e}",
                                path.display()
                            ),
                        )
                        .with_file(&path),
                    );
                    continue;
                }
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
