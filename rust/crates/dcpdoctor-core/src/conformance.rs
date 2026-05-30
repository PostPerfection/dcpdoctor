//! DCI conformance test suite — structured pass/fail reporting.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::Standard;

/// A single conformance test result.
#[derive(Debug, Clone, Serialize)]
pub struct ConformanceTest {
    pub test_id: String,
    pub description: String,
    pub spec_reference: String,
    pub passed: bool,
    pub detail: String,
}

/// Options for conformance testing.
pub struct ConformanceOptions {
    pub dcp_dir: PathBuf,
    pub check_picture_profile: bool,
    pub check_security: bool,
}

/// Full conformance report.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ConformanceReport {
    pub tool_version: String,
    pub report_date: String,
    pub dcp_dir: PathBuf,
    pub content_title: String,
    pub cpl_id: String,
    pub issue_date: String,
    pub detected_standard: Standard,
    pub conformant: bool,
    pub total_tests: u32,
    pub tests_passed: u32,
    pub tests_failed: u32,
    pub error: String,
    pub structure_tests: Vec<ConformanceTest>,
    pub cpl_tests: Vec<ConformanceTest>,
    pub picture_tests: Vec<ConformanceTest>,
    pub audio_tests: Vec<ConformanceTest>,
    pub security_tests: Vec<ConformanceTest>,
}

/// Run DCI conformance tests on a DCP.
pub fn run_conformance_tests(opts: &ConformanceOptions) -> ConformanceReport {
    let mut report = ConformanceReport {
        tool_version: "dcpdoctor 1.0".into(),
        report_date: today_iso(),
        dcp_dir: opts.dcp_dir.clone(),
        ..Default::default()
    };

    if !opts.dcp_dir.exists() || !opts.dcp_dir.is_dir() {
        report.error = format!("DCP directory not found: {}", opts.dcp_dir.display());
        return report;
    }

    // --- Structure tests (SMPTE ST 429-9) ---
    let has_assetmap =
        opts.dcp_dir.join("ASSETMAP").exists() || opts.dcp_dir.join("ASSETMAP.xml").exists();
    report.structure_tests.push(make_test(
        "DCI-STRUCT-1",
        "ASSETMAP present",
        "SMPTE ST 429-9:2014",
        has_assetmap,
        if has_assetmap { "Found" } else { "Missing" },
    ));

    let has_volindex =
        opts.dcp_dir.join("VOLINDEX").exists() || opts.dcp_dir.join("VOLINDEX.xml").exists();
    report.structure_tests.push(make_test(
        "DCI-STRUCT-2",
        "VOLINDEX present",
        "SMPTE ST 429-9:2014",
        has_volindex,
        if has_volindex { "Found" } else { "Missing" },
    ));

    // Find PKL and CPL
    let (pkls, cpls, first_cpl) = find_xml_components(&opts.dcp_dir);

    report.structure_tests.push(make_test(
        "DCI-STRUCT-3",
        "PackingList (PKL) present",
        "SMPTE ST 429-8:2014",
        !pkls.is_empty(),
        if pkls.is_empty() {
            "No PKL found".into()
        } else {
            format!("Found {}", pkls.len())
        },
    ));

    report.structure_tests.push(make_test(
        "DCI-STRUCT-4",
        "CompositionPlaylist (CPL) present",
        "SMPTE ST 429-7:2006",
        !cpls.is_empty(),
        if cpls.is_empty() {
            "No CPL found".into()
        } else {
            format!("Found {}", cpls.len())
        },
    ));

    // MXF files
    let mxf_count = count_mxf_files(&opts.dcp_dir);
    report.structure_tests.push(make_test(
        "DCI-STRUCT-5",
        "MXF track files present",
        "SMPTE ST 429-3:2006",
        mxf_count > 0,
        format!("{mxf_count} MXF file(s)"),
    ));

    // --- CPL tests ---
    if let Some(ref cpl) = first_cpl {
        report.content_title.clone_from(&cpl.title);
        report.cpl_id.clone_from(&cpl.id);
        report.issue_date.clone_from(&cpl.issue_date);

        let valid_id = cpl.id.starts_with("urn:uuid:") && cpl.id.len() > 20;
        report.cpl_tests.push(make_test(
            "DCI-CPL-1",
            "CPL Id is valid URN UUID",
            "SMPTE ST 429-7:2006 §6.1",
            valid_id,
            &cpl.id,
        ));

        report.cpl_tests.push(make_test(
            "DCI-CPL-2",
            "ContentTitleText present",
            "SMPTE ST 429-7:2006 §6.2",
            !cpl.title.is_empty(),
            &cpl.title,
        ));

        report.cpl_tests.push(make_test(
            "DCI-CPL-3",
            "ContentKind present",
            "SMPTE ST 429-7:2006 §6.4",
            !cpl.content_kind.is_empty(),
            &cpl.content_kind,
        ));

        report.cpl_tests.push(make_test(
            "DCI-CPL-4",
            "At least one Reel present",
            "SMPTE ST 429-7:2006 §6.10",
            cpl.reel_count > 0,
            format!("{} reel(s)", cpl.reel_count),
        ));

        report.cpl_tests.push(make_test(
            "DCI-CPL-5",
            "IssueDate present",
            "SMPTE ST 429-7:2006 §6.3",
            !cpl.issue_date.is_empty(),
            &cpl.issue_date,
        ));
    }

    // --- Picture tests ---
    if opts.check_picture_profile {
        let empty_mxf = find_empty_mxf(&opts.dcp_dir);
        if empty_mxf.is_empty() {
            report.picture_tests.push(make_test(
                "DCI-PIC-1",
                "All MXF files have non-zero size",
                "SMPTE ST 429-3:2006",
                mxf_count > 0,
                "",
            ));
        } else {
            for name in &empty_mxf {
                report.picture_tests.push(make_test(
                    "DCI-PIC-1",
                    "MXF file has non-zero size",
                    "SMPTE ST 429-3:2006",
                    false,
                    format!("{name} is empty"),
                ));
            }
        }
    }

    // --- Audio tests ---
    if let Some(ref cpl) = first_cpl {
        report.audio_tests.push(make_test(
            "DCI-AUD-1",
            "Audio track referenced in CPL",
            "SMPTE ST 429-7:2006",
            cpl.has_audio,
            if cpl.has_audio {
                "MainSound present"
            } else {
                "No MainSound in any reel"
            },
        ));
    }

    // --- Security tests ---
    if opts.check_security {
        let has_encryption = cpls.iter().any(|p| {
            std::fs::read_to_string(p)
                .map(|c| c.contains("KeyId"))
                .unwrap_or(false)
        });
        report.security_tests.push(make_test(
            "DCI-SEC-1",
            "Encryption status detected",
            "SMPTE ST 429-6:2006",
            true,
            if has_encryption {
                "Encrypted (KeyId found)"
            } else {
                "Not encrypted"
            },
        ));
    }

    // Detect standard
    for cpl_path in &cpls {
        if let Ok(content) = std::fs::read_to_string(cpl_path) {
            if content.contains("smpte-ra.org") {
                report.detected_standard = Standard::Smpte;
            } else if content.contains("digicine.com") || content.contains("cinecert.com") {
                report.detected_standard = Standard::Interop;
            }
        }
    }

    // Summarize
    let all_tests = [
        &report.structure_tests,
        &report.cpl_tests,
        &report.picture_tests,
        &report.audio_tests,
        &report.security_tests,
    ];
    for tests in all_tests {
        for t in tests {
            report.total_tests += 1;
            if t.passed {
                report.tests_passed += 1;
            } else {
                report.tests_failed += 1;
            }
        }
    }
    report.conformant = report.tests_failed == 0;
    report
}

// --- Helpers ---

fn make_test(
    id: &str,
    desc: &str,
    spec: &str,
    passed: bool,
    detail: impl Into<String>,
) -> ConformanceTest {
    ConformanceTest {
        test_id: id.into(),
        description: desc.into(),
        spec_reference: spec.into(),
        passed,
        detail: detail.into(),
    }
}

fn today_iso() -> String {
    let now = time::OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}",
        now.year(),
        now.month() as u8,
        now.day()
    )
}

struct CplInfo {
    id: String,
    title: String,
    content_kind: String,
    issue_date: String,
    reel_count: u32,
    has_audio: bool,
}

fn find_xml_components(dir: &Path) -> (Vec<PathBuf>, Vec<PathBuf>, Option<CplInfo>) {
    let mut pkls = Vec::new();
    let mut cpls = Vec::new();
    let mut first_cpl = None;

    let Ok(entries) = std::fs::read_dir(dir) else {
        return (pkls, cpls, first_cpl);
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().unwrap_or_default() != "xml" {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };

        if content.contains("PackingList") {
            pkls.push(path.clone());
        }
        if content.contains("<CompositionPlaylist") {
            cpls.push(path.clone());
            if first_cpl.is_none() {
                first_cpl = Some(parse_cpl_info(&content));
            }
        }
    }

    (pkls, cpls, first_cpl)
}

fn parse_cpl_info(content: &str) -> CplInfo {
    let id = extract_tag(content, "Id").unwrap_or_default();
    let title = extract_tag(content, "ContentTitleText").unwrap_or_default();
    let content_kind = extract_tag(content, "ContentKind").unwrap_or_default();
    let issue_date = extract_tag(content, "IssueDate").unwrap_or_default();

    let reel_re = regex_lite::Regex::new(r"<Reel>").unwrap();
    let reel_count = reel_re.find_iter(content).count() as u32;

    let has_audio = content.contains("<MainSound>");

    CplInfo {
        id,
        title,
        content_kind,
        issue_date,
        reel_count,
        has_audio,
    }
}

fn count_mxf_files(dir: &Path) -> u32 {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "mxf"))
                .count() as u32
        })
        .unwrap_or(0)
}

fn find_empty_mxf(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| {
                    e.path().extension().is_some_and(|ext| ext == "mxf")
                        && e.metadata().map(|m| m.len() == 0).unwrap_or(false)
                })
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn extract_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].trim().to_string())
}
