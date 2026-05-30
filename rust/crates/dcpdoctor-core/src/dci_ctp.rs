//! DCI Compliance Test Plan (CTP) automated tests.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// CTP test category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CtpCategory {
    Packaging,
    Composition,
    Picture,
    Audio,
    Security,
    Presentation,
}

/// A single CTP test result.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CtpTestResult {
    pub test_id: String,
    pub category: Option<CtpCategory>,
    pub description: String,
    pub requirement: String,
    pub passed: bool,
    pub skipped: bool,
    pub detail: String,
}

/// Options for CTP test execution.
pub struct CtpOptions {
    pub dcp_dir: PathBuf,
    pub test_packaging: bool,
    pub test_composition: bool,
    pub test_picture: bool,
    pub test_audio: bool,
    pub test_security: bool,
    pub test_presentation: bool,
}

/// Overall CTP result.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CtpResult {
    pub error: String,
    pub compliant: bool,
    pub total: u32,
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub results: Vec<CtpTestResult>,
}

fn make_ctp(
    id: &str,
    cat: CtpCategory,
    desc: &str,
    req: &str,
    passed: bool,
    detail: &str,
) -> CtpTestResult {
    CtpTestResult {
        test_id: id.into(),
        category: Some(cat),
        description: desc.into(),
        requirement: req.into(),
        passed,
        detail: detail.into(),
        ..Default::default()
    }
}

fn make_skipped(id: &str, cat: CtpCategory, desc: &str, req: &str, reason: &str) -> CtpTestResult {
    CtpTestResult {
        test_id: id.into(),
        category: Some(cat),
        description: desc.into(),
        requirement: req.into(),
        skipped: true,
        detail: reason.into(),
        ..Default::default()
    }
}

/// Run DCI CTP tests against a DCP directory.
pub fn run_ctp_tests(opts: &CtpOptions) -> CtpResult {
    let mut result = CtpResult::default();

    if !opts.dcp_dir.exists() || !opts.dcp_dir.is_dir() {
        result.error = format!("DCP directory not found: {}", opts.dcp_dir.display());
        return result;
    }

    // --- Packaging tests (CTP Section 4) ---
    if opts.test_packaging {
        let has_volindex =
            opts.dcp_dir.join("VOLINDEX").exists() || opts.dcp_dir.join("VOLINDEX.xml").exists();
        result.results.push(make_ctp(
            "CTP-PKG-001",
            CtpCategory::Packaging,
            "VOLINDEX file present",
            "DCI DCSS §9.3.1: Each volume shall contain a VOLINDEX file",
            has_volindex,
            "",
        ));

        let has_assetmap =
            opts.dcp_dir.join("ASSETMAP").exists() || opts.dcp_dir.join("ASSETMAP.xml").exists();
        result.results.push(make_ctp(
            "CTP-PKG-002",
            CtpCategory::Packaging,
            "ASSETMAP file present",
            "DCI DCSS §9.3.2: Each volume shall contain an ASSETMAP",
            has_assetmap,
            "",
        ));

        let pkls = find_pkls(&opts.dcp_dir);
        result.results.push(make_ctp(
            "CTP-PKG-003",
            CtpCategory::Packaging,
            "Packing List (PKL) present",
            "DCI DCSS §9.2: A DCP shall contain at least one PKL",
            !pkls.is_empty(),
            "",
        ));

        let mxf_count = count_files_with_ext(&opts.dcp_dir, "mxf");
        result.results.push(make_ctp(
            "CTP-PKG-004",
            CtpCategory::Packaging,
            "MXF track files present with .mxf extension",
            "SMPTE ST 429-3: Track files shall use MXF container",
            mxf_count > 0,
            &format!("{mxf_count} MXF file(s) found"),
        ));
    }

    // --- Composition tests (CTP Section 5) ---
    if opts.test_composition {
        let cpls = find_cpls(&opts.dcp_dir);

        result.results.push(make_ctp(
            "CTP-CPL-001",
            CtpCategory::Composition,
            "Composition Playlist present",
            "DCI DCSS §8.4: A DCP shall contain at least one CPL",
            !cpls.is_empty(),
            "",
        ));

        if let Some(cpl_content) = cpls.first().and_then(|p| std::fs::read_to_string(p).ok()) {
            // Check UUID format
            let id_re = regex_lite::Regex::new(r"<Id>(urn:uuid:[^<]+)</Id>").unwrap();
            let has_uuid = id_re
                .captures(&cpl_content)
                .is_some_and(|c| c[1].starts_with("urn:uuid:"));
            result.results.push(make_ctp(
                "CTP-CPL-002",
                CtpCategory::Composition,
                "CPL uses URN:UUID identifier format",
                "SMPTE ST 429-7 §6.1: Id shall be a UUID in URN form",
                has_uuid,
                "",
            ));

            // Check ContentKind
            let kind_re = regex_lite::Regex::new(r"<ContentKind>([^<]+)</ContentKind>").unwrap();
            if let Some(cap) = kind_re.captures(&cpl_content) {
                let kind = cap[1].to_lowercase();
                let valid_kinds = [
                    "feature",
                    "trailer",
                    "test",
                    "teaser",
                    "rating",
                    "advertisement",
                    "short",
                    "transitional",
                    "psa",
                    "policy",
                    "episode",
                ];
                let valid_kind = valid_kinds.contains(&kind.as_str());
                result.results.push(make_ctp(
                    "CTP-CPL-003",
                    CtpCategory::Composition,
                    "ContentKind uses approved value",
                    "DCI DCSS §8.4.1: ContentKind shall be from approved list",
                    valid_kind,
                    &kind,
                ));
            }

            // Check reels with picture
            let has_picture = cpl_content.contains("MainPicture")
                || cpl_content.contains("MainStereoscopicPicture");
            result.results.push(make_ctp(
                "CTP-CPL-004",
                CtpCategory::Composition,
                "At least one reel contains picture essence",
                "DCI DCSS §8.4.2: Each CPL shall reference picture essence",
                has_picture,
                "",
            ));

            // Check EditRate
            let rate_re = regex_lite::Regex::new(r"<EditRate>(\d+)\s+(\d+)</EditRate>").unwrap();
            let valid_rates = ["24 1", "25 1", "30 1", "48 1", "50 1", "60 1"];
            let mut valid_rate = true;
            for cap in rate_re.captures_iter(&cpl_content) {
                let rate = format!("{} {}", &cap[1], &cap[2]);
                if !valid_rates.contains(&rate.as_str()) {
                    valid_rate = false;
                    break;
                }
            }
            result.results.push(make_ctp(
                "CTP-CPL-005",
                CtpCategory::Composition,
                "EditRate uses DCI-approved frame rate",
                "DCI DCSS §3.2.1: Frame rates shall be 24, 25, 30, 48, 50, or 60 fps",
                valid_rate,
                "",
            ));
        }
    }

    // --- Picture tests (CTP Section 6) ---
    if opts.test_picture {
        result.results.push(make_skipped(
            "CTP-PIC-001",
            CtpCategory::Picture,
            "JPEG 2000 Profile: 9-7 irreversible DWT",
            "DCI DCSS §3.2.1.1: Only 9-7 irreversible wavelet",
            "Requires J2K frame decode",
        ));

        result.results.push(make_skipped(
            "CTP-PIC-002",
            CtpCategory::Picture,
            "JPEG 2000 code-block size 32x32",
            "DCI DCSS §3.2.1.1: Code-block size shall be 32x32",
            "Requires J2K codestream analysis",
        ));

        result.results.push(make_skipped(
            "CTP-PIC-003",
            CtpCategory::Picture,
            "Maximum bitrate 250 Mbit/s (2K) / 500 Mbit/s (4K)",
            "DCI DCSS §3.2.1: Peak bitrate limits",
            "Requires per-frame bitrate analysis",
        ));

        // Check for zero-byte MXF files
        let any_zero = has_zero_byte_mxf(&opts.dcp_dir);
        result.results.push(make_ctp(
            "CTP-PIC-004",
            CtpCategory::Picture,
            "All MXF track files non-empty",
            "SMPTE ST 378: MXF file shall contain valid essence",
            !any_zero,
            "",
        ));
    }

    // --- Audio tests (CTP Section 7) ---
    if opts.test_audio {
        result.results.push(make_skipped(
            "CTP-AUD-001",
            CtpCategory::Audio,
            "Audio PCM: 24-bit, 48kHz or 96kHz",
            "DCI DCSS §3.3.1: Audio shall be 24-bit LPCM at 48/96kHz",
            "Requires MXF audio essence analysis",
        ));
    }

    // --- Security tests (CTP Section 8) ---
    if opts.test_security {
        let has_encryption = dir_xml_contains(&opts.dcp_dir, "KeyId");
        result.results.push(make_ctp(
            "CTP-SEC-001",
            CtpCategory::Security,
            "Encryption status",
            "DCI DCSS §5: Content security",
            true,
            if has_encryption {
                "Encrypted"
            } else {
                "Unencrypted"
            },
        ));
    }

    // --- Presentation tests (CTP Section 9) ---
    if opts.test_presentation {
        result.results.push(make_skipped(
            "CTP-PRE-001",
            CtpCategory::Presentation,
            "FFMC/LFMC markers present",
            "SMPTE ST 429-7 §6.10.1.4: Markers for automation",
            "Marker validation requires full CPL marker analysis",
        ));
    }

    // --- Summarize ---
    for r in &result.results {
        result.total += 1;
        if r.skipped {
            result.skipped += 1;
        } else if r.passed {
            result.passed += 1;
        } else {
            result.failed += 1;
        }
    }
    result.compliant = result.failed == 0;
    result
}

/// Serialize CTP result to JSON.
pub fn ctp_to_json(result: &CtpResult) -> String {
    serde_json::to_string_pretty(result).unwrap_or_default()
}

fn find_pkls(dir: &Path) -> Vec<PathBuf> {
    let mut pkls = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return pkls;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().is_none_or(|ext| ext != "xml") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        if content[..content.len().min(2048)].contains("PackingList") {
            pkls.push(path);
        }
    }
    pkls
}

fn find_cpls(dir: &Path) -> Vec<PathBuf> {
    let mut cpls = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return cpls;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().is_none_or(|ext| ext != "xml") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        if content[..content.len().min(2048)].contains("CompositionPlaylist") {
            cpls.push(path);
        }
    }
    cpls
}

fn count_files_with_ext(dir: &Path, ext: &str) -> usize {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == ext))
        .count()
}

fn has_zero_byte_mxf(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| {
            let p = e.path();
            p.extension().is_some_and(|x| x == "mxf")
                && std::fs::metadata(&p).is_ok_and(|m| m.len() == 0)
        })
}

fn dir_xml_contains(dir: &Path, needle: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().is_none_or(|ext| ext != "xml") {
            continue;
        }
        if std::fs::read_to_string(&path).is_ok_and(|content| content.contains(needle)) {
            return true;
        }
    }
    false
}
