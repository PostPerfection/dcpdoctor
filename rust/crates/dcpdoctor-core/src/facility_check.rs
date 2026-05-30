//! Pre-delivery facility check: comprehensive DCP validation for theater ingest.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// A single check item result.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CheckItem {
    pub category: String,
    pub check_name: String,
    pub passed: bool,
    pub detail: String,
    pub severity: String,
}

/// Options for facility check.
pub struct FacilityCheckOptions {
    pub dcp_dir: PathBuf,
    pub expected_standard: crate::Standard,
    pub strict: bool,
    pub check_naming: bool,
    pub check_hashes: bool,
}

/// Result of facility check.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FacilityCheckResult {
    pub error: String,
    pub ready: bool,
    pub summary: String,
    pub checks_total: u32,
    pub checks_passed: u32,
    pub errors: u32,
    pub warnings: u32,
    pub info_count: u32,
    pub items: Vec<CheckItem>,
}

fn make_item(category: &str, name: &str, passed: bool, detail: &str, severity: &str) -> CheckItem {
    CheckItem {
        category: category.into(),
        check_name: name.into(),
        passed,
        detail: detail.into(),
        severity: severity.into(),
    }
}

/// Run a comprehensive facility check on a DCP directory.
pub fn run_facility_check(opts: &FacilityCheckOptions) -> FacilityCheckResult {
    let mut result = FacilityCheckResult::default();

    if !opts.dcp_dir.exists() || !opts.dcp_dir.is_dir() {
        result.error = format!("DCP directory not found: {}", opts.dcp_dir.display());
        return result;
    }

    // --- Structure checks ---
    let has_assetmap =
        opts.dcp_dir.join("ASSETMAP").exists() || opts.dcp_dir.join("ASSETMAP.xml").exists();
    result.items.push(make_item(
        "structure",
        "ASSETMAP present",
        has_assetmap,
        if has_assetmap {
            ""
        } else {
            "Missing ASSETMAP or ASSETMAP.xml"
        },
        "error",
    ));

    let has_volindex =
        opts.dcp_dir.join("VOLINDEX").exists() || opts.dcp_dir.join("VOLINDEX.xml").exists();
    result.items.push(make_item(
        "structure",
        "VOLINDEX present",
        has_volindex,
        if has_volindex {
            ""
        } else {
            "Missing VOLINDEX or VOLINDEX.xml"
        },
        "error",
    ));

    // Check PKL
    let pkls = find_xml_containing(&opts.dcp_dir, "PackingList");
    result.items.push(make_item(
        "structure",
        "PKL present",
        !pkls.is_empty(),
        if pkls.is_empty() {
            "No PackingList XML found"
        } else {
            ""
        },
        "error",
    ));

    // Check CPL
    let cpls = find_xml_containing(&opts.dcp_dir, "CompositionPlaylist");
    result.items.push(make_item(
        "structure",
        "CPL present",
        !cpls.is_empty(),
        if cpls.is_empty() {
            "No CompositionPlaylist XML found"
        } else {
            ""
        },
        "error",
    ));

    // Check MXF files exist
    let has_mxf = std::fs::read_dir(&opts.dcp_dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| e.path().extension().is_some_and(|x| x == "mxf"));
    result.items.push(make_item(
        "structure",
        "MXF essence files present",
        has_mxf,
        if has_mxf {
            ""
        } else {
            "No .mxf files found in DCP directory"
        },
        "error",
    ));

    // --- Namespace consistency ---
    let ns_notes = crate::schema_validate::check_namespace_consistency(&opts.dcp_dir);
    let ns_ok = ns_notes.is_empty();
    result.items.push(make_item(
        "compliance",
        "Namespace consistency",
        ns_ok,
        if ns_ok {
            ""
        } else {
            "Mixed SMPTE/Interop namespaces"
        },
        "warning",
    ));

    // --- ISDCF naming ---
    if opts.check_naming {
        for cpl_path in &cpls {
            if let Ok(content) = std::fs::read_to_string(cpl_path) {
                let title_re =
                    regex_lite::Regex::new(r"<ContentTitleText>([^<]+)</ContentTitleText>")
                        .unwrap();
                if let Some(cap) = title_re.captures(&content) {
                    let title = &cap[1];
                    let naming_notes = crate::isdcf::check_isdcf_naming(title, cpl_path);
                    let naming_ok = naming_notes.is_empty();
                    result.items.push(make_item(
                        "naming",
                        "ISDCF naming compliance",
                        naming_ok,
                        if naming_ok {
                            ""
                        } else {
                            "ISDCF naming issues found"
                        },
                        "warning",
                    ));
                }
            }
        }
    }

    // --- Summarize ---
    for item in &result.items {
        result.checks_total += 1;
        if item.passed {
            result.checks_passed += 1;
        } else {
            match item.severity.as_str() {
                "error" => result.errors += 1,
                "warning" => result.warnings += 1,
                _ => result.info_count += 1,
            }
        }
    }

    result.ready = result.errors == 0;
    result.summary = format!(
        "{}/{} checks passed",
        result.checks_passed, result.checks_total
    );
    if result.errors > 0 {
        result.summary += &format!(", {} error(s)", result.errors);
    }
    if result.warnings > 0 {
        result.summary += &format!(", {} warning(s)", result.warnings);
    }

    result
}

/// Serialize facility check result to JSON.
pub fn facility_check_to_json(result: &FacilityCheckResult) -> String {
    serde_json::to_string_pretty(result).unwrap_or_default()
}

fn find_xml_containing(dir: &Path, needle: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().is_none_or(|ext| ext != "xml") {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&path)
            && content[..content.len().min(2048)].contains(needle)
        {
            found.push(path);
        }
    }
    found
}
