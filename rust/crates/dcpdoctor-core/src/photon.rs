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

/// Where the Photon classpath can come from. `explicit` and `PHOTON_DIR` may
/// name a single jar or a directory of jars; the rest are directories.
pub fn find_photon(explicit: Option<&Path>) -> Option<PhotonClasspath> {
    if let Some(classpath) = explicit.and_then(classpath_at) {
        return Some(classpath);
    }
    if let Some(classpath) =
        std::env::var_os("PHOTON_DIR").and_then(|path| classpath_at(Path::new(&path)))
    {
        return Some(classpath);
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

fn classpath_at(configured: &Path) -> Option<PhotonClasspath> {
    if configured.is_file() && configured.extension() == Some("jar".as_ref()) {
        return Some(PhotonClasspath::Jar(configured.to_path_buf()));
    }
    [
        configured.to_path_buf(),
        configured.join("build").join("libs"),
    ]
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
pub fn ensure_photon(explicit: Option<&Path>) -> Result<PhotonClasspath, PhotonError> {
    if !has_java() {
        return Err(PhotonError::JavaNotFound);
    }
    find_photon(explicit).ok_or(PhotonError::NotInstalled)
}

/// Run Photon against an IMP directory and return validation notes. Errors when
/// Java is missing or no Photon jars were fetched.
pub fn run_photon(imp_dir: &Path, explicit: Option<&Path>) -> Result<Vec<Note>, PhotonError> {
    let classpath = ensure_photon(explicit)?.argument();

    let output = Command::new("java")
        .args(["-cp", &classpath, "com.netflix.imflibrary.app.IMPAnalyzer"])
        .arg(imp_dir)
        .output()
        .map_err(|e| PhotonError::ExecutionFailed(e.to_string()))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let combined = format!("{}\n{}", stdout, stderr);
    let mut notes = parse_photon_output(&combined, imp_dir);
    if !output.status.success() {
        notes.push(incomplete_run_note(&combined, imp_dir));
    }
    Ok(notes)
}

/// Photon exits non-zero when the analysis threw instead of finishing, and the
/// documents it had not reached yet were never checked.
fn incomplete_run_note(output: &str, imp_dir: &Path) -> Note {
    let cause = output
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("Exception in thread"))
        .unwrap_or("Photon printed no cause");
    Note::warning(
        Code::CheckSkipped,
        format!("[Photon] deep IMF checks did not finish: {cause}"),
    )
    .with_file(imp_dir)
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

/// Split a Photon 5 log line, `[main] ERROR com.netflix.imflibrary.app.IMPAnalyzer
/// - <payload>`, into its level and payload. `None` for a line slf4j did not write.
fn split_log_line(line: &str) -> Option<(&str, &str)> {
    let after_thread = line.strip_prefix('[')?.split_once("] ")?.1;
    let (level, rest) = after_thread.split_once(' ')?;
    let (logger, payload) = rest.split_once(" - ")?;
    (!logger.contains(' ')).then_some((level, payload))
}

/// The document a `<name> has N errors and M warnings` line is about, which is
/// the one the findings printed under it belong to.
fn analysed_document(payload: &str) -> Option<&str> {
    let (name, counts) = payload.split_once(" has ")?;
    let is_count = counts.starts_with("no errors") || counts.contains(" errors and ");
    (is_count && !name.contains(' ')).then_some(name)
}

/// The severity a line reports at, from the prefix Photon puts on the finding
/// itself or, failing that, the level it logged at.
fn finding<'a>(payload: &'a str, level: Option<&str>) -> Option<(Severity, &'a str)> {
    const FINDING_PREFIXES: &[(&str, Severity)] = &[
        ("ERROR:", Severity::Error),
        ("FATAL:", Severity::Error),
        ("WARNING:", Severity::Warning),
        ("ERROR-", Severity::Error),
        ("FATAL-", Severity::Error),
        ("WARNING-", Severity::Warning),
    ];
    for (prefix, severity) in FINDING_PREFIXES {
        if let Some(rest) = payload.strip_prefix(prefix) {
            return Some((*severity, rest.trim()));
        }
    }
    match level? {
        "ERROR" | "FATAL" => Some((Severity::Error, payload)),
        "WARN" | "WARNING" => Some((Severity::Warning, payload)),
        _ => None,
    }
}

/// Photon closes every finding with `[Photon version: 5.0.1]`.
fn without_version_suffix(message: &str) -> &str {
    match message.rfind(" [Photon version:") {
        Some(index) if message.ends_with(']') => message[..index].trim_end(),
        _ => message,
    }
}

/// Parse Photon's text output into dcpdoctor Notes.
///
/// Photon 5 logs through slf4j-simple, so a finding reads
/// `[main] ERROR com.netflix.imflibrary.app.IMPAnalyzer - ERROR-<message> [Photon
/// version: 5.0.1]` under an INFO line naming the document. The bare
/// `ERROR: <message> (file: <path>, line: <n>)` form is read as well.
fn parse_photon_output(output: &str, imp_dir: &Path) -> Vec<Note> {
    let mut notes = Vec::new();
    let mut analysed: Option<PathBuf> = None;

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let (level, payload) = match split_log_line(trimmed) {
            Some((level, payload)) => (Some(level), payload.trim()),
            None => (None, trimmed),
        };

        if let Some(name) = analysed_document(payload) {
            analysed = Some(imp_dir.join(name));
            continue;
        }

        let Some((severity, rest)) = finding(payload, level) else {
            continue;
        };
        let rest = without_version_suffix(rest);

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
            (rest.to_string(), analysed.clone())
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
    } else if lower.contains("cannot find asset") {
        Code::AssetNotFound
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
    fn an_explicit_jar_wins_over_the_search_paths() {
        let dir = tempfile::tempdir().unwrap();
        let jar = dir.path().join("photon.jar");
        std::fs::write(&jar, b"").unwrap();
        match find_photon(Some(&jar)) {
            Some(PhotonClasspath::Jar(found)) => assert_eq!(found, jar),
            other => panic!("expected the jar itself, got {other:?}"),
        }
        match find_photon(Some(dir.path())) {
            Some(PhotonClasspath::Directory(found)) => assert_eq!(found, dir.path()),
            other => panic!("expected the directory of jars, got {other:?}"),
        }
    }

    #[test]
    fn an_explicit_path_without_jars_is_not_a_classpath() {
        let dir = tempfile::tempdir().unwrap();
        assert!(classpath_at(dir.path()).is_none());
        assert!(classpath_at(&dir.path().join("missing")).is_none());
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

    /// Photon 5.0.1's own stderr for `IMPAnalyzer /tmp/c14nrun/imp`, an IMP whose
    /// sound Resource carries no SourceEncoding. Every line comes through
    /// slf4j-simple, so nothing here starts with the bare `ERROR:`.
    const PHOTON_5_OUTPUT: &str = r#"SLF4J(I): Connected with provider of type [org.slf4j.simple.SimpleServiceProvider]
[main] INFO com.netflix.imflibrary.app.IMPAnalyzer - ==========================================================================
[main] INFO com.netflix.imflibrary.app.IMPAnalyzer - Analyzing IMF delivery: /tmp/c14nrun/imp
[main] INFO com.netflix.imflibrary.app.IMPAnalyzer - AUDIO_033c76d9-321f-4184-ba77-25df3a53c81b.mxf has no errors or warnings
[main] INFO com.netflix.imflibrary.app.IMPAnalyzer - VIDEO_c7d75d7b-7cec-4974-a665-b91536bec4cd.mxf has no errors or warnings
[main] INFO com.netflix.imflibrary.app.IMPAnalyzer - CPL_394080ca-5471-40e9-9827-e6e577753400.xml has 1 errors and 0 warnings
[main] ERROR com.netflix.imflibrary.app.IMPAnalyzer - 		ERROR-Line Number : 105 - cvc-complex-type.2.4.a: Invalid content was found starting with element '{"http://www.smpte-ra.org/schemas/2067-3/2016":TrackFileId}'. One of '{"http://www.smpte-ra.org/schemas/2067-3/2016":RepeatCount, "http://www.smpte-ra.org/schemas/2067-3/2016":SourceEncoding}' is expected. [Photon version: 5.0.1]
[main] INFO com.netflix.imflibrary.app.IMPAnalyzer - ASSETMAP.xml has no errors or warnings
[main] INFO com.netflix.imflibrary.app.IMPAnalyzer - PKL_d74e5590-b9fd-4482-8fd3-ddb7fe496e64.xml has no errors or warnings"#;

    #[test]
    fn a_photon_5_schema_error_is_a_note_on_the_document_it_names() {
        let notes = parse_photon_output(PHOTON_5_OUTPUT, Path::new("/tmp/c14nrun/imp"));
        assert_eq!(notes.len(), 1, "got: {notes:?}");

        let note = &notes[0];
        assert_eq!(note.severity, Severity::Error);
        assert_eq!(note.code, Code::XmlSchemaViolation);
        assert_eq!(
            note.message,
            "[Photon] Line Number : 105 - cvc-complex-type.2.4.a: Invalid content was found starting with element '{\"http://www.smpte-ra.org/schemas/2067-3/2016\":TrackFileId}'. One of '{\"http://www.smpte-ra.org/schemas/2067-3/2016\":RepeatCount, \"http://www.smpte-ra.org/schemas/2067-3/2016\":SourceEncoding}' is expected."
        );
        assert_eq!(
            note.file.as_deref(),
            Some(Path::new(
                "/tmp/c14nrun/imp/CPL_394080ca-5471-40e9-9827-e6e577753400.xml"
            )),
            "the INFO line above the finding says which document it is about"
        );
    }

    /// The same IMP with its picture track file deleted.
    const PHOTON_5_MISSING_ASSET: &str = r#"[main] INFO com.netflix.imflibrary.app.IMPAnalyzer - Analyzing IMF delivery: /tmp/impcheck/missing
[main] ERROR com.netflix.imflibrary.app.IMPAnalyzer - 		ERROR-Cannot find asset with id: urn:uuid:c7d75d7b-7cec-4974-a665-b91536bec4cd (path according to asset map: /tmp/impcheck/missing/VIDEO_c7d75d7b-7cec-4974-a665-b91536bec4cd.mxf) [Photon version: 5.0.1]"#;

    #[test]
    fn a_photon_5_missing_asset_error_names_the_track_file() {
        let notes = parse_photon_output(PHOTON_5_MISSING_ASSET, Path::new("/tmp/impcheck/missing"));
        assert_eq!(notes.len(), 1, "got: {notes:?}");
        assert_eq!(notes[0].severity, Severity::Error);
        assert_eq!(notes[0].code, Code::AssetNotFound);
        assert!(
            notes[0]
                .message
                .contains("Cannot find asset with id: urn:uuid:c7d75d7b"),
            "{}",
            notes[0].message
        );
        assert!(
            !notes[0].message.contains("Photon version"),
            "the version suffix repeats on every finding: {}",
            notes[0].message
        );
    }

    #[test]
    fn a_photon_progress_line_is_not_a_finding() {
        let notes = parse_photon_output(
            "[main] INFO com.netflix.imflibrary.app.IMPAnalyzer - CPL.xml has 3 errors and 1 warnings",
            Path::new("/tmp/imp"),
        );
        assert!(notes.is_empty(), "got: {notes:?}");
    }

    #[test]
    fn a_photon_run_that_threw_says_the_checks_did_not_finish() {
        let output = r#"[main] INFO com.netflix.imflibrary.app.IMPAnalyzer - Analyzing IMF delivery: /tmp/impcheck/truncated
Exception in thread "main" java.io.IOException: Invalid range request: rangeStart = 1184207958 is not <= 3999999 rangeEnd
	at com.netflix.imflibrary.utils.FileByteRangeProvider.getByteRangeAsBytes(FileByteRangeProvider.java:152)"#;
        let note = incomplete_run_note(output, Path::new("/tmp/impcheck/truncated"));
        assert_eq!(note.severity, Severity::Warning);
        assert_eq!(note.code, Code::CheckSkipped);
        assert!(
            note.message
                .contains("java.io.IOException: Invalid range request"),
            "{}",
            note.message
        );
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
