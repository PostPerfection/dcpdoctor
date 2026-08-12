//! Netflix Photon integration for deep IMF Application 2/2E validation.
//!
//! Photon has to be fetched beforehand: point `PHOTON_DIR` at a jar or at a
//! directory of jars (imfwizard's `scripts/fetch_photon.sh` reads the same
//! variable and pulls them from Maven Central). dcpdoctor does not build Photon:
//! Netflix pins Gradle 8.5, which cannot read Java 25 class files, so building
//! from source fails on a current JDK.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{Code, Note, Severity};

/// Error returned when Photon cannot be used.
#[derive(Debug)]
pub enum PhotonError {
    /// Java runtime not found
    JavaNotFound,
    /// No Photon jars on any of the searched paths
    NotInstalled,
    /// Failed to run Photon
    ExecutionFailed(String),
}

impl std::fmt::Display for PhotonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PhotonError::JavaNotFound => write!(
                f,
                "Java runtime not found. Install Java 11+ (e.g. `apt install default-jre`)"
            ),
            PhotonError::NotInstalled => write!(
                f,
                "Photon jars not found. Set PHOTON_DIR to a Photon jar or a directory of jars"
            ),
            PhotonError::ExecutionFailed(e) => write!(f, "Photon execution failed: {e}"),
        }
    }
}

/// Return the cache directory for dcpdoctor.
fn cache_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        PathBuf::from(xdg).join("dcpdoctor")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".cache").join("dcpdoctor")
    } else {
        PathBuf::from("/tmp/dcpdoctor-cache")
    }
}

/// Where the Photon classpath can come from. `PHOTON_DIR` may name a single jar
/// or a directory of jars; the rest are directories.
pub fn find_photon() -> Option<PhotonClasspath> {
    if let Ok(path) = std::env::var("PHOTON_DIR") {
        let configured = PathBuf::from(&path);
        if configured.is_file() && configured.extension() == Some("jar".as_ref()) {
            return Some(PhotonClasspath::Jar(configured));
        }
        for dir in [configured.clone(), configured.join("build").join("libs")] {
            if has_photon_jars(&dir) {
                return Some(PhotonClasspath::Directory(dir));
            }
        }
    }

    let candidates = [
        PathBuf::from("/usr/local/share/photon/libs"),
        PathBuf::from("/usr/share/photon/libs"),
        PathBuf::from("/opt/photon/build/libs"),
        cache_dir().join("photon"),
        cache_dir().join("photon").join("build").join("libs"),
    ];
    candidates
        .into_iter()
        .find(|dir| has_photon_jars(dir))
        .map(PhotonClasspath::Directory)
}

/// A Photon classpath entry, ready for `java -cp`.
#[derive(Debug, Clone)]
pub enum PhotonClasspath {
    Jar(PathBuf),
    Directory(PathBuf),
}

impl PhotonClasspath {
    /// The `-cp` argument. A directory expands with the wildcard java itself
    /// understands, so every jar the fetch script dropped there is picked up.
    fn argument(&self) -> String {
        match self {
            PhotonClasspath::Jar(path) => path.display().to_string(),
            PhotonClasspath::Directory(dir) => format!("{}/*", dir.display()),
        }
    }
}

/// Check if a directory contains Photon JAR files.
fn has_photon_jars(dir: &Path) -> bool {
    dir.is_dir()
        && std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .flatten()
                    .any(|e| e.path().extension() == Some("jar".as_ref()))
            })
            .unwrap_or(false)
}

/// Check if Java is available on the system.
pub fn has_java() -> bool {
    Command::new("java")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Locate a usable Photon install, or say why there isn't one.
pub fn ensure_photon() -> Result<PhotonClasspath, PhotonError> {
    if !has_java() {
        return Err(PhotonError::JavaNotFound);
    }
    find_photon().ok_or(PhotonError::NotInstalled)
}

/// Run Photon against an IMP directory and return validation notes. Errors when
/// Java is missing or no Photon jars were fetched.
pub fn run_photon(imp_dir: &Path) -> Result<Vec<Note>, PhotonError> {
    let classpath = ensure_photon()?.argument();

    let output = Command::new("java")
        .args(["-cp", &classpath, "com.netflix.imflibrary.app.IMPAnalyzer"])
        .arg(imp_dir)
        .output()
        .map_err(|e| PhotonError::ExecutionFailed(e.to_string()))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let combined = format!("{}\n{}", stdout, stderr);
    Ok(parse_photon_output(&combined, imp_dir))
}

/// Note for a Photon pass that could not run. A Photon that was never fetched is
/// not a defect in the package, so that reports as Info. Only the first line of a
/// failure is kept: a java or build failure can run to hundreds of lines and a
/// Note is one line.
pub fn unavailable_note(error: &PhotonError) -> Note {
    let severity = match error {
        PhotonError::ExecutionFailed(_) => Severity::Warning,
        PhotonError::JavaNotFound | PhotonError::NotInstalled => Severity::Info,
    };
    let detail = error.to_string();
    let first_line = detail.lines().next().unwrap_or_default().trim();
    Note {
        severity,
        code: Code::MissingRequiredElement,
        message: format!("[Photon] deep IMF checks skipped: {first_line}"),
        file: None,
        line: 0,
    }
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
