//! XML schema validation and namespace checking.

use std::path::Path;

use crate::{Code, Note, Severity, Standard};

/// Validate XML namespace against expected standard (SMPTE or Interop).
pub fn validate_namespace(xml_file: &Path, expected: Standard) -> Vec<Note> {
    let mut notes = Vec::new();

    let content = match std::fs::read_to_string(xml_file) {
        Ok(c) => c,
        Err(_) => return notes,
    };

    let file_path = Some(xml_file.to_path_buf());

    // Find root element namespace using simple regex
    let ns_re = regex_lite::Regex::new(r#"xmlns="([^"]+)""#).unwrap();
    let Some(cap) = ns_re.captures(&content) else {
        return notes;
    };
    let ns = &cap[1];

    match expected {
        Standard::Smpte => {
            if !ns.contains("smpte-ra.org") && !ns.contains("smpte.org") {
                notes.push(Note {
                    severity: Severity::Warning,
                    code: Code::SmpteNamespaceWrong,
                    message: format!("Expected SMPTE namespace, found: {ns}"),
                    file: file_path,
                    line: 0,
                });
            }
        }
        Standard::Interop => {
            if !ns.contains("digicine.com") {
                notes.push(Note {
                    severity: Severity::Warning,
                    code: Code::InteropNamespaceWrong,
                    message: format!("Expected Interop namespace, found: {ns}"),
                    file: file_path,
                    line: 0,
                });
            }
        }
        Standard::Unknown => {}
    }

    notes
}

/// Check that all XML files in a directory use consistent namespaces.
pub fn check_namespace_consistency(dir: &Path) -> Vec<Note> {
    let mut notes = Vec::new();
    let mut found_smpte = false;
    let mut found_interop = false;

    let Ok(entries) = std::fs::read_dir(dir) else {
        return notes;
    };

    let ns_re = regex_lite::Regex::new(r#"xmlns="([^"]+)""#).unwrap();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().is_none_or(|ext| ext != "xml") {
            continue;
        }

        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };

        if let Some(cap) = ns_re.captures(&content) {
            let ns = &cap[1];
            if ns.contains("smpte-ra.org") || ns.contains("smpte.org") {
                found_smpte = true;
            } else if ns.contains("digicine.com") {
                found_interop = true;
            }
        }
    }

    if found_smpte && found_interop {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::SmpteNamespaceWrong,
            message: "Package contains both SMPTE and Interop namespaces".into(),
            file: Some(dir.to_path_buf()),
            line: 0,
        });
    }

    notes
}
