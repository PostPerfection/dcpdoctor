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
                    code: Code::MissingRecommendedElement,
                    message: "BV2.1 recommends ExtensionMetadata in CPL".into(),
                    file: cpl_path.clone(),
                    line: 0,
                });
            }

            if !first_reel_has_main_markers(&content) {
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

fn first_reel_has_main_markers(cpl: &str) -> bool {
    let reel_pattern = regex_lite::Regex::new(r"<Reel>([\s\S]*?)</Reel>").unwrap();
    reel_pattern
        .captures(cpl)
        .is_some_and(|reel| reel[1].contains("<MainMarkers>"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN_MARKERS: &str =
        "<MainMarkers><Id>urn:uuid:2f0d6f4e-5f3a-4c7e-9b1a-0c8d7e6f5a4b</Id></MainMarkers>";

    fn write_two_reel_package(
        first_reel_assets: &str,
        second_reel_assets: &str,
    ) -> tempfile::TempDir {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("ASSETMAP.xml"), "<AssetMap/>").unwrap();
        std::fs::write(
            dir.path().join("cpl.xml"),
            format!(
                r#"<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL">
  <ContentVersion><Id>urn:uuid:1</Id></ContentVersion>
  <EditRate>24 1</EditRate>
  <ReelList>
    <Reel><AssetList>{first_reel_assets}</AssetList></Reel>
    <Reel><AssetList>{second_reel_assets}</AssetList></Reel>
  </ReelList>
</CompositionPlaylist>"#
            ),
        )
        .unwrap();
        dir
    }

    fn warns_missing_main_markers(dir: &Path) -> bool {
        check_bv21_compliance(dir, Standard::Smpte)
            .iter()
            .any(|note| note.code == Code::MarkerMissing)
    }

    #[test]
    fn main_markers_only_in_the_second_reel_warn() {
        let dir = write_two_reel_package("", MAIN_MARKERS);
        assert!(warns_missing_main_markers(dir.path()));
    }

    #[test]
    fn main_markers_in_the_first_reel_do_not_warn() {
        let dir = write_two_reel_package(MAIN_MARKERS, "");
        assert!(!warns_missing_main_markers(dir.path()));
    }

    #[test]
    fn missing_extension_metadata_is_a_recommendation() {
        let dir = write_two_reel_package(MAIN_MARKERS, "");
        let notes = check_bv21_compliance(dir.path(), Standard::Smpte);
        let note = notes
            .iter()
            .find(|note| note.message.contains("ExtensionMetadata"))
            .expect("the CPL has no ExtensionMetadata");
        assert_eq!(note.code, Code::MissingRecommendedElement);
        assert_eq!(note.code.as_str(), "missing_recommended_element");
    }
}
