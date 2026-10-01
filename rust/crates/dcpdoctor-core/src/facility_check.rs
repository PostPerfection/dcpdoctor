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
    let pkls = find_xml_rooted_at(&opts.dcp_dir, "PackingList");
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
    let cpls = find_xml_rooted_at(&opts.dcp_dir, "CompositionPlaylist");
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

    // --- Hash verification ---
    if opts.check_hashes {
        let checksum = crate::checksum_verify::verify_package_checksums(
            &crate::checksum_verify::ChecksumVerifyOptions {
                package_dir: opts.dcp_dir.clone(),
                ..Default::default()
            },
        );
        let detail = if !checksum.success {
            format!("Hashes not verified: {}", checksum.error)
        } else if checksum.all_valid {
            String::new()
        } else {
            format!(
                "{} of {} asset(s) failed: {}",
                checksum.hash_mismatches + checksum.size_mismatches + checksum.missing_files,
                checksum.total_assets,
                failed_asset_details(&checksum).join(", ")
            )
        };
        result.items.push(make_item(
            "integrity",
            "PKL hash verification",
            checksum.success && checksum.all_valid,
            &detail,
            "error",
        ));
    } else {
        result.items.push(make_item(
            "integrity",
            "PKL hash verification",
            false,
            "Not checked (--no-hashes)",
            "info",
        ));
    }

    // --- Signature and signing certificates ---
    let mut signature_notes = Vec::new();
    let mut signed_documents = 0;
    for document in pkls.iter().chain(cpls.iter()) {
        let Ok(content) = std::fs::read_to_string(document) else {
            continue;
        };
        if !crate::signature::has_signature(&content) {
            continue;
        }
        signed_documents += 1;
        signature_notes.extend(crate::signature::verify_signature(document, false));
    }
    if signed_documents == 0 {
        result.items.push(make_item(
            "security",
            "Signing certificates",
            false,
            "Not checked: no CPL or PKL in the package carries a signature",
            "info",
        ));
    } else {
        let severity = if signature_notes
            .iter()
            .any(|note| note.severity == crate::Severity::Error)
        {
            "error"
        } else {
            "warning"
        };
        let detail = signature_notes
            .iter()
            .map(|note| note.message.clone())
            .collect::<Vec<_>>()
            .join(", ");
        result.items.push(make_item(
            "security",
            "Signing certificates",
            signature_notes.is_empty(),
            &detail,
            severity,
        ));
    }

    // --- Composition metadata ---
    for cpl_path in &cpls {
        let Ok(content) = std::fs::read_to_string(cpl_path) else {
            continue;
        };
        result
            .items
            .extend(check_composition_metadata(cpl_path, &content));
    }

    // --- ISDCF naming ---
    if opts.check_naming {
        for cpl_path in &cpls {
            let content = match std::fs::read_to_string(cpl_path) {
                Ok(c) => c,
                Err(e) => {
                    result.items.push(make_item(
                        "naming",
                        "ISDCF naming compliance",
                        false,
                        &format!("Not checked, cannot read {}: {e}", cpl_path.display()),
                        "warning",
                    ));
                    continue;
                }
            };
            let title_re =
                regex_lite::Regex::new(r"<ContentTitleText>([^<]+)</ContentTitleText>").unwrap();
            let Some(cap) = title_re.captures(&content) else {
                result.items.push(make_item(
                    "naming",
                    "ISDCF naming compliance",
                    false,
                    &format!("Not checked, no ContentTitleText in {}", cpl_path.display()),
                    "warning",
                ));
                continue;
            };
            let naming_notes = crate::isdcf::check_isdcf_naming(&cap[1], cpl_path);
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

// optional in ST 429-16 but required by TIFF and Deluxe QC
const FACILITY_METADATA_ELEMENTS: [&str; 4] = ["Chain", "Distributor", "Facility", "Luminance"];

const LUMINANCE_UNITS: [&str; 2] = ["foot-lambert", "candela-per-square-metre"];

fn check_composition_metadata(cpl_path: &Path, content: &str) -> Vec<CheckItem> {
    // Interop CPLs have no CompositionMetadataAsset
    if crate::dcp::standard_of_root_namespace(content) != crate::Standard::Smpte {
        return Vec::new();
    }
    let cpl_name = cpl_path.file_name().unwrap_or_default().to_string_lossy();
    let metadata_block =
        metadata_element(content, "CompositionMetadataAsset").map_or("", |element| element.text);

    FACILITY_METADATA_ELEMENTS
        .iter()
        .map(|name| {
            let check_name = format!("CompositionMetadataAsset {name}");
            let Some(element) = metadata_element(metadata_block, name) else {
                return make_item(
                    "metadata",
                    &check_name,
                    false,
                    &format!(
                        "{cpl_name}: no <{name}> in the CompositionMetadataAsset, some facilities' QC requires it"
                    ),
                    "warning",
                );
            };
            match metadata_element_defect(name, &element) {
                Some(defect) => make_item(
                    "metadata",
                    &check_name,
                    false,
                    &format!("{cpl_name}: {defect}"),
                    "error",
                ),
                None => make_item("metadata", &check_name, true, "", "error"),
            }
        })
        .collect()
}

struct MetadataElement<'a> {
    attributes: &'a str,
    text: &'a str,
}

// a self-closing `<meta:Chain/>` counts as present and empty
fn metadata_element<'a>(xml: &'a str, name: &str) -> Option<MetadataElement<'a>> {
    let element_re = regex_lite::Regex::new(&format!(
        r"<(?:[\w-]+:)?{name}(\s[^>]*?)?(?:/>|>([\s\S]*?)</(?:[\w-]+:)?{name}>)"
    ))
    .unwrap();
    let captures = element_re.captures(xml)?;
    Some(MetadataElement {
        attributes: captures.get(1).map_or("", |group| group.as_str()),
        text: captures.get(2).map_or("", |group| group.as_str().trim()),
    })
}

fn metadata_element_defect(name: &str, element: &MetadataElement) -> Option<String> {
    if name != "Luminance" {
        return element
            .text
            .is_empty()
            .then(|| format!("<{name}> is empty"));
    }
    let units_re = regex_lite::Regex::new(r#"\bunits\s*=\s*["']([^"']*)["']"#).unwrap();
    let units = units_re
        .captures(element.attributes)
        .map(|captures| captures.get(1).unwrap().as_str());
    if !units.is_some_and(|units| LUMINANCE_UNITS.contains(&units)) {
        return Some(format!(
            "<Luminance> units is {}, it must be {}",
            units.map_or_else(|| "missing".to_string(), |units| format!("'{units}'")),
            LUMINANCE_UNITS.join(" or ")
        ));
    }
    let value_is_positive = element
        .text
        .parse::<f64>()
        .is_ok_and(|value| value.is_finite() && value > 0.0);
    if !value_is_positive {
        return Some(format!(
            "<Luminance> value '{}' is not a positive number",
            element.text
        ));
    }
    None
}

/// One line per asset that failed the checksum pass, naming the file and what
/// was wrong with it.
fn failed_asset_details(checksum: &crate::checksum_verify::ChecksumVerifyResult) -> Vec<String> {
    checksum
        .entries
        .iter()
        .filter_map(|entry| {
            let name = if entry.filename.is_empty() {
                entry.asset_id.clone()
            } else {
                entry.filename.clone()
            };
            if !entry.file_exists {
                return Some(format!("{name} is missing"));
            }
            if !entry.hash_match {
                return Some(format!("{name} hash mismatch"));
            }
            if !entry.size_match {
                return Some(format!(
                    "{name} is {} bytes, the PKL declares {}",
                    entry.actual_size, entry.expected_size
                ));
            }
            None
        })
        .collect()
}

/// Serialize facility check result to JSON.
pub fn facility_check_to_json(result: &FacilityCheckResult) -> String {
    serde_json::to_string_pretty(result).unwrap_or_default()
}

/// Package XML documents whose root element carries `root_name`. An ASSETMAP
/// holds a `<PackingList>` flag element of its own, so a substring search finds
/// it in place of a PKL that was never delivered.
fn find_xml_rooted_at(dir: &Path, root_name: &str) -> Vec<PathBuf> {
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
            && crate::schema::root_element(&content).is_some_and(|(root, _)| root == root_name)
        {
            found.push(path);
        }
    }
    found.sort();
    found
}
