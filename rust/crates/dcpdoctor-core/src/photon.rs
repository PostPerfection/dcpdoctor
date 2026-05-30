//! Optional integration with Netflix Photon for deep IMF validation.
//!
//! When `photon.jar` is available on the system, dcpdoctor can delegate
//! IMF Application 2/2E conformance checks to Photon and merge its
//! findings into our unified report.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{Code, Note, Severity};

/// Check if Photon is available on the system.
/// Looks for the JAR at well-known locations or via PHOTON_JAR env var.
pub fn find_photon() -> Option<PathBuf> {
    // 1. Environment variable
    if let Ok(path) = std::env::var("PHOTON_JAR") {
        let p = PathBuf::from(&path);
        if p.exists() {
            return Some(p);
        }
    }

    // 2. Well-known locations
    let candidates = [
        "/usr/local/share/photon/photon.jar",
        "/usr/share/photon/photon.jar",
        "/opt/photon/photon.jar",
    ];
    for candidate in &candidates {
        let p = PathBuf::from(candidate);
        if p.exists() {
            return Some(p);
        }
    }

    // 3. Check if `photon` wrapper script is on PATH
    if let Ok(output) = Command::new("which").arg("photon").output()
        && output.status.success()
    {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Some(PathBuf::from(path));
        }
    }

    None
}

/// Run Photon against an IMP directory and return validation notes.
///
/// Requires Java to be installed. Returns an empty vec if Photon
/// is unavailable or fails to run.
pub fn run_photon(imp_dir: &Path) -> Vec<Note> {
    let jar_path = match find_photon() {
        Some(p) => p,
        None => return Vec::new(),
    };

    // Determine how to invoke — JAR directly or wrapper script
    let output = if jar_path.extension().and_then(|e| e.to_str()) == Some("jar") {
        Command::new("java")
            .args(["-jar", &jar_path.to_string_lossy()])
            .arg("--imp")
            .arg(imp_dir)
            .output()
    } else {
        // Wrapper script
        Command::new(&jar_path).arg("--imp").arg(imp_dir).output()
    };

    let output = match output {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!("Failed to run Photon: {}", e);
            return Vec::new();
        }
    };

    // Photon exits 0 on success, non-zero on validation failure
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let combined = format!("{}\n{}", stdout, stderr);
    parse_photon_output(&combined, imp_dir)
}

/// Parse Photon's text output into dcpdoctor Notes.
///
/// Photon output format (simplified):
/// ```text
/// ERROR: <message> (file: <path>, line: <n>)
/// WARNING: <message>
/// ```
fn parse_photon_output(output: &str, imp_dir: &Path) -> Vec<Note> {
    let mut notes = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let (severity, rest) = if let Some(rest) = trimmed.strip_prefix("ERROR:") {
            (Severity::Error, rest.trim())
        } else if let Some(rest) = trimmed.strip_prefix("FATAL:") {
            (Severity::Error, rest.trim())
        } else if let Some(rest) = trimmed.strip_prefix("WARNING:") {
            (Severity::Warning, rest.trim())
        } else {
            continue;
        };

        // Try to extract file path from the message
        let (message, file) = if let Some(idx) = rest.find("(file:") {
            let msg = rest[..idx].trim().to_string();
            let file_part = &rest[idx + 6..];
            let file_str = file_part
                .split(')')
                .next()
                .unwrap_or("")
                .split(',')
                .next()
                .unwrap_or("")
                .trim();
            let file_path = if Path::new(file_str).is_absolute() {
                PathBuf::from(file_str)
            } else {
                imp_dir.join(file_str)
            };
            (msg, Some(file_path))
        } else {
            (rest.to_string(), None)
        };

        notes.push(Note {
            severity,
            code: classify_photon_error(&message),
            message: format!("[Photon] {}", message),
            file,
            line: 0,
        });
    }

    notes
}

/// Map Photon error messages to dcpdoctor error codes.
fn classify_photon_error(message: &str) -> Code {
    let lower = message.to_lowercase();
    if lower.contains("hash") || lower.contains("digest") {
        Code::MxfHashMismatch
    } else if lower.contains("schema") || lower.contains("xsd") {
        Code::XmlSchemaViolation
    } else if lower.contains("uuid") {
        Code::InvalidUuid
    } else if lower.contains("duration") {
        Code::CplInvalidDuration
    } else if lower.contains("edit rate") || lower.contains("editrate") {
        Code::CplInvalidEditRate
    } else if lower.contains("resolution") {
        Code::PictureInvalidResolution
    } else if lower.contains("frame rate") || lower.contains("framerate") {
        Code::PictureInvalidFrameRate
    } else if lower.contains("sample rate") || lower.contains("samplerate") {
        Code::SoundInvalidSampleRate
    } else if lower.contains("channel") {
        Code::SoundInvalidChannelCount
    } else if lower.contains("namespace") {
        Code::SmpteNamespaceWrong
    } else if lower.contains("mxf") {
        Code::MxfInvalidStructure
    } else {
        Code::XmlSchemaViolation // fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_photon_not_installed() {
        // On most dev machines Photon won't be installed
        // Just verify the function doesn't panic
        let _ = find_photon();
    }

    #[test]
    fn test_parse_photon_output_errors() {
        let output = r#"
ERROR: Hash mismatch for asset abc123 (file: PKL.xml, line: 42)
WARNING: Non-standard edit rate detected
ERROR: Schema validation failed for CPL
"#;
        let notes = parse_photon_output(output, Path::new("/tmp/imp"));
        assert_eq!(notes.len(), 3);
        assert_eq!(notes[0].severity, Severity::Error);
        assert_eq!(notes[0].code, Code::MxfHashMismatch);
        assert!(notes[0].message.contains("[Photon]"));
        assert_eq!(notes[1].severity, Severity::Warning);
        assert_eq!(notes[1].code, Code::CplInvalidEditRate);
        assert_eq!(notes[2].severity, Severity::Error);
        assert_eq!(notes[2].code, Code::XmlSchemaViolation);
    }

    #[test]
    fn test_parse_photon_output_empty() {
        let notes = parse_photon_output("", Path::new("/tmp"));
        assert!(notes.is_empty());
    }

    #[test]
    fn test_classify_photon_error() {
        assert_eq!(
            classify_photon_error("Hash mismatch"),
            Code::MxfHashMismatch
        );
        assert_eq!(
            classify_photon_error("Invalid resolution 1920x1080"),
            Code::PictureInvalidResolution
        );
        assert_eq!(
            classify_photon_error("Something else"),
            Code::XmlSchemaViolation
        );
    }
}
