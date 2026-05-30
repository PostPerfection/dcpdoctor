//! Netflix Photon integration for deep IMF Application 2/2E validation.
//!
//! Photon is automatically cloned and built to `~/.cache/dcpdoctor/photon/`
//! on first use. Requires Java 11+ and git.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{Code, Note, Severity};

const PHOTON_REPO: &str = "https://github.com/Netflix/photon.git";

/// Error returned when Photon cannot be used.
#[derive(Debug)]
pub enum PhotonError {
    /// Java runtime not found
    JavaNotFound,
    /// Failed to obtain Photon
    SetupFailed(String),
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
            PhotonError::SetupFailed(e) => write!(f, "Photon setup failed: {e}"),
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

/// Find the Photon libs directory, checking (in order):
/// 1. PHOTON_DIR environment variable (directory containing libs/*.jar)
/// 2. Well-known system paths
/// 3. Local cache (~/.cache/dcpdoctor/photon/build/libs/)
pub fn find_photon() -> Option<PathBuf> {
    // 1. Environment variable
    if let Ok(path) = std::env::var("PHOTON_DIR") {
        let p = PathBuf::from(&path);
        if has_photon_jars(&p) {
            return Some(p);
        }
        // Also check build/libs/ subdirectory
        let libs = p.join("build").join("libs");
        if has_photon_jars(&libs) {
            return Some(libs);
        }
    }

    // 2. Well-known locations
    let candidates = [
        "/usr/local/share/photon/libs",
        "/usr/share/photon/libs",
        "/opt/photon/build/libs",
    ];
    for candidate in &candidates {
        let p = PathBuf::from(candidate);
        if has_photon_jars(&p) {
            return Some(p);
        }
    }

    // 3. Local cache
    let cached = cache_dir().join("photon").join("build").join("libs");
    if has_photon_jars(&cached) {
        return Some(cached);
    }

    None
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

/// Clone and build Photon from source into the local cache.
/// Requires git and Java 11+.
fn bootstrap_photon() -> Result<PathBuf, PhotonError> {
    let photon_dir = cache_dir().join("photon");
    let libs_dir = photon_dir.join("build").join("libs");

    // Already built
    if has_photon_jars(&libs_dir) {
        return Ok(libs_dir);
    }

    eprintln!("[dcpdoctor] Bootstrapping Photon (one-time setup)...");

    // Clone if needed
    if !photon_dir.join(".git").exists() {
        let _ = std::fs::remove_dir_all(&photon_dir);
        std::fs::create_dir_all(cache_dir())
            .map_err(|e| PhotonError::SetupFailed(e.to_string()))?;

        let output = Command::new("git")
            .args(["clone", "--depth", "1", PHOTON_REPO])
            .arg(&photon_dir)
            .output()
            .map_err(|e| PhotonError::SetupFailed(format!("git not found: {e}")))?;

        if !output.status.success() {
            return Err(PhotonError::SetupFailed(format!(
                "git clone failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
    }

    // Build
    let gradlew = if cfg!(windows) {
        "gradlew.bat"
    } else {
        "./gradlew"
    };

    let output = Command::new(gradlew)
        .args(["build", "-x", "test"])
        .current_dir(&photon_dir)
        .output()
        .map_err(|e| PhotonError::SetupFailed(format!("gradle build failed to start: {e}")))?;

    if !output.status.success() {
        return Err(PhotonError::SetupFailed(format!(
            "gradle build failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    // Get dependencies
    let output = Command::new(gradlew)
        .args(["getDependencies"])
        .current_dir(&photon_dir)
        .output()
        .map_err(|e| PhotonError::SetupFailed(format!("getDependencies failed: {e}")))?;

    if !output.status.success() {
        return Err(PhotonError::SetupFailed(format!(
            "getDependencies failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    if has_photon_jars(&libs_dir) {
        eprintln!("[dcpdoctor] Photon ready.");
        Ok(libs_dir)
    } else {
        Err(PhotonError::SetupFailed(
            "build succeeded but no JARs found in build/libs/".to_string(),
        ))
    }
}

/// Ensure Photon is ready to use — build from source if needed, check Java.
pub fn ensure_photon() -> Result<PathBuf, PhotonError> {
    if !has_java() {
        return Err(PhotonError::JavaNotFound);
    }

    if let Some(path) = find_photon() {
        return Ok(path);
    }

    bootstrap_photon()
}

/// Run Photon against an IMP directory and return validation notes.
///
/// Auto-builds Photon from source if not cached. Returns error if Java
/// is missing or Photon cannot be obtained.
pub fn run_photon(imp_dir: &Path) -> Result<Vec<Note>, PhotonError> {
    let libs_dir = ensure_photon()?;

    // Build classpath: all JARs in libs directory
    let classpath = format!("{}/*:", libs_dir.display());

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
