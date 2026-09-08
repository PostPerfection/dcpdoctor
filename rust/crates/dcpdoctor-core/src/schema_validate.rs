//! Namespace consistency across a package's XML files.

use std::path::Path;

use crate::{Code, Note, Severity, Standard};

/// A CPL's root namespace must name the same authority the asset map does. A
/// SMPTE package carrying an Interop CPL, or the reverse, is what `fix`
/// rewrites, and nothing reported it before.
pub fn check_cpl_namespace(cpl_path: &Path, standard: Standard) -> Option<Note> {
    let xml = std::fs::read_to_string(cpl_path).ok()?;
    let declared = crate::dcp::standard_of_root_namespace(&xml);
    if declared == Standard::Unknown || declared == standard {
        return None;
    }
    let (code, message) = match standard {
        Standard::Smpte => (
            Code::SmpteNamespaceWrong,
            "CPL declares the Interop namespace in a SMPTE package",
        ),
        Standard::Interop => (
            Code::InteropNamespaceWrong,
            "CPL declares the SMPTE namespace in an Interop package",
        ),
        Standard::Unknown => return None,
    };
    Some(Note {
        severity: Severity::Error,
        code,
        message: message.into(),
        file: Some(cpl_path.to_path_buf()),
        line: 0,
    })
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
