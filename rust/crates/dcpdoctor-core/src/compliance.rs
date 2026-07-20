//! SMPTE/Interop compliance checks for DCP packages.

use std::path::Path;

use crate::{Code, Note, Severity, Standard};

const SMPTE_CPL_NS: &str = "http://www.smpte-ra.org/schemas/429-7/2006/CPL";
const SMPTE_PKL_NS: &str = "http://www.smpte-ra.org/schemas/429-8/2007/PKL";
const SMPTE_AM_NS: &str = "http://www.smpte-ra.org/schemas/429-9/2007/AM";
const INTEROP_CPL_NS: &str = "http://www.digicine.com/PROTO-ASDCP-CPL-20040511#";
const INTEROP_PKL_NS: &str = "http://www.digicine.com/PROTO-ASDCP-PKL-20040311#";

const VALID_EDIT_RATES: &[&str] = &[
    "24 1",
    "25 1",
    "30 1",
    "48 1",
    "60 1",
    "24000 1001",
    "30000 1001",
    "60000 1001",
];

const VALID_CONTENT_KINDS: &[&str] = &[
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

const UUID_PATTERN: &str =
    r"^urn:uuid:[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$";

/// Run SMPTE/Interop compliance checks on a DCP directory.
pub fn check_smpte_compliance(dcp_dir: &Path, standard: Standard, strict: bool) -> Vec<Note> {
    let mut notes = Vec::new();

    // Check ASSETMAP naming
    let am_xml = dcp_dir.join("ASSETMAP.xml");
    let am_plain = dcp_dir.join("ASSETMAP");

    if standard == Standard::Smpte && strict && am_plain.exists() && !am_xml.exists() {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::SmpteNamingViolation,
            message: "SMPTE DCP should use ASSETMAP.xml (not ASSETMAP)".into(),
            file: Some(dcp_dir.to_path_buf()),
            line: 0,
        });
    }

    // Check VOLINDEX.xml
    if standard == Standard::Smpte && strict {
        let volindex = dcp_dir.join("VOLINDEX.xml");
        let volindex2 = dcp_dir.join("VOLINDEX");
        if !volindex.exists() && !volindex2.exists() {
            notes.push(Note {
                severity: Severity::Warning,
                code: Code::SmpteNamingViolation,
                message: "SMPTE DCP missing VOLINDEX.xml".into(),
                file: Some(dcp_dir.to_path_buf()),
                line: 0,
            });
        }
    }

    // Validate ASSETMAP
    let am_path = if am_xml.exists() { am_xml } else { am_plain };
    if am_path.exists() {
        check_assetmap_compliance(&am_path, standard, &mut notes);
    }

    // Find and validate PKLs and CPLs
    let Ok(entries) = std::fs::read_dir(dcp_dir) else {
        return notes;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().unwrap_or_default().to_string_lossy();
        if ext != "xml" && ext != "XML" {
            continue;
        }

        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };

        if content.contains("<PackingList") || content.contains("<PackingList>") {
            check_pkl_compliance(&path, &content, standard, &mut notes);
        } else if content.contains("<CompositionPlaylist") {
            check_cpl_compliance(&path, &content, standard, strict, &mut notes);
        }
    }

    notes
}

/// Validate that every `urn:uuid:` identifier in the DCP's XML documents is a
/// well-formed RFC 4122 UUID. Cheap and standard-agnostic; wired into the
/// default validate path (ClairMeta: check_cpl_id_rfc4122 / check_assets_*_uuid).
/// Only tokens that already carry the `urn:uuid:` prefix are checked, so bare
/// non-UUID identifiers in other schemas are not falsely flagged.
pub fn check_uuids(dcp_dir: &Path) -> Vec<Note> {
    let mut notes = Vec::new();
    let uuid_re = regex_lite::Regex::new(UUID_PATTERN).unwrap();
    let token_re = regex_lite::Regex::new(r"urn:uuid:[0-9A-Za-z._-]+").unwrap();

    let Ok(entries) = std::fs::read_dir(dcp_dir) else {
        return notes;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let is_xml = name.ends_with(".xml") || name.ends_with(".XML");
        if !is_xml && name != "ASSETMAP" && name != "VOLINDEX" {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let mut seen = std::collections::HashSet::new();
        for m in token_re.find_iter(&content) {
            let tok = m.as_str();
            if seen.insert(tok) && !uuid_re.is_match(tok) {
                notes.push(Note {
                    severity: Severity::Warning,
                    code: Code::InvalidUuid,
                    message: format!("Malformed UUID: {tok}"),
                    file: Some(path.clone()),
                    line: 0,
                });
            }
        }
    }
    notes
}

fn check_uuid(value: &str, context: &str, file: &Path, notes: &mut Vec<Note>) {
    if value.is_empty() {
        return;
    }
    let re = regex_lite::Regex::new(UUID_PATTERN).unwrap();
    if !re.is_match(value) {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::InvalidUuid,
            message: format!("Invalid UUID format in {context}: {value}"),
            file: Some(file.to_path_buf()),
            line: 0,
        });
    }
}

fn check_namespace(
    content: &str,
    expected_cpl: &str,
    expected_pkl: &str,
    file: &Path,
    notes: &mut Vec<Note>,
) {
    // Check if namespace in the document matches expected
    if content.contains("<CompositionPlaylist") && !content.contains(expected_cpl) {
        let code = if expected_cpl == SMPTE_CPL_NS {
            Code::SmpteNamespaceWrong
        } else {
            Code::InteropNamespaceWrong
        };
        notes.push(Note {
            severity: Severity::Error,
            code,
            message: "CPL uses wrong namespace for detected standard".into(),
            file: Some(file.to_path_buf()),
            line: 0,
        });
    }
    if content.contains("<PackingList") && !content.contains(expected_pkl) {
        let code = if expected_pkl == SMPTE_PKL_NS {
            Code::SmpteNamespaceWrong
        } else {
            Code::InteropNamespaceWrong
        };
        notes.push(Note {
            severity: Severity::Error,
            code,
            message: "PKL uses wrong namespace for detected standard".into(),
            file: Some(file.to_path_buf()),
            line: 0,
        });
    }
}

fn check_assetmap_compliance(am_path: &Path, standard: Standard, notes: &mut Vec<Note>) {
    let Ok(content) = std::fs::read_to_string(am_path) else {
        return;
    };

    // Namespace check
    if standard == Standard::Smpte && !content.contains(SMPTE_AM_NS) {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::SmpteNamespaceWrong,
            message: "ASSETMAP uses non-SMPTE namespace".into(),
            file: Some(am_path.to_path_buf()),
            line: 0,
        });
    }

    // Check Id
    if let Some(id) = extract_tag(&content, "Id") {
        check_uuid(&id, "ASSETMAP Id", am_path, notes);
    }

    // Check VolumeCount
    if let Some(vc_str) = extract_tag(&content, "VolumeCount")
        && let Ok(vc) = vc_str.parse::<u32>()
        && vc != 1
    {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::SmpteNamingViolation,
            message: format!("VolumeCount != 1 (multi-volume DCPs are unusual): {vc}"),
            file: Some(am_path.to_path_buf()),
            line: 0,
        });
    }

    // Check for duplicate asset Ids
    let id_re = regex_lite::Regex::new(r"<Id>(urn:uuid:[^<]+)</Id>").unwrap();
    let mut seen_ids = std::collections::HashSet::new();
    // Skip the first Id (the ASSETMAP's own Id)
    let mut first = true;
    for cap in id_re.captures_iter(&content) {
        if first {
            first = false;
            continue;
        }
        let id = &cap[1];
        check_uuid(id, "ASSETMAP Asset Id", am_path, notes);
        if !seen_ids.insert(id.to_string()) {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::DuplicateAssetId,
                message: format!("Duplicate asset Id: {id}"),
                file: Some(am_path.to_path_buf()),
                line: 0,
            });
        }
    }
}

fn check_pkl_compliance(pkl_path: &Path, content: &str, standard: Standard, notes: &mut Vec<Note>) {
    // Namespace check
    if standard == Standard::Smpte {
        check_namespace(content, SMPTE_CPL_NS, SMPTE_PKL_NS, pkl_path, notes);
    } else if standard == Standard::Interop {
        check_namespace(content, INTEROP_CPL_NS, INTEROP_PKL_NS, pkl_path, notes);
    }

    // Check PKL Id
    if let Some(id) = extract_tag(content, "Id") {
        check_uuid(&id, "PKL Id", pkl_path, notes);
    }

    // Required elements
    if extract_tag(content, "IssueDate").is_none() {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::MissingRequiredElement,
            message: "PKL missing IssueDate".into(),
            file: Some(pkl_path.to_path_buf()),
            line: 0,
        });
    }
    if extract_tag(content, "Issuer").is_none() {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::MissingRequiredElement,
            message: "PKL missing Issuer".into(),
            file: Some(pkl_path.to_path_buf()),
            line: 0,
        });
    }
    if extract_tag(content, "Creator").is_none() {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::MissingRequiredElement,
            message: "PKL missing Creator".into(),
            file: Some(pkl_path.to_path_buf()),
            line: 0,
        });
    }

    // Check asset entries have Hash and Type
    let asset_re = regex_lite::Regex::new(r"<Asset>([\s\S]*?)</Asset>").unwrap();
    for cap in asset_re.captures_iter(content) {
        let block = &cap[1];
        if let Some(id) = extract_tag(block, "Id") {
            check_uuid(&id, "PKL Asset Id", pkl_path, notes);
        }
        if extract_tag(block, "Hash").is_none() {
            notes.push(Note {
                severity: Severity::Warning,
                code: Code::MissingRequiredElement,
                message: "PKL Asset missing Hash element".into(),
                file: Some(pkl_path.to_path_buf()),
                line: 0,
            });
        }
        if extract_tag(block, "Type").is_none() {
            notes.push(Note {
                severity: Severity::Warning,
                code: Code::MissingRequiredElement,
                message: "PKL Asset missing Type element".into(),
                file: Some(pkl_path.to_path_buf()),
                line: 0,
            });
        }
    }
}

fn check_cpl_compliance(
    cpl_path: &Path,
    content: &str,
    standard: Standard,
    strict: bool,
    notes: &mut Vec<Note>,
) {
    // Namespace check
    if standard == Standard::Smpte && !content.contains(SMPTE_CPL_NS) {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::SmpteNamespaceWrong,
            message: "CPL uses non-SMPTE namespace".into(),
            file: Some(cpl_path.to_path_buf()),
            line: 0,
        });
    } else if standard == Standard::Interop && !content.contains(INTEROP_CPL_NS) {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::InteropNamespaceWrong,
            message: "CPL uses non-Interop namespace".into(),
            file: Some(cpl_path.to_path_buf()),
            line: 0,
        });
    }

    // Check CPL Id
    if let Some(id) = extract_tag(content, "Id") {
        check_uuid(&id, "CPL Id", cpl_path, notes);
    }

    // ContentTitleText
    if extract_tag(content, "ContentTitleText").is_none() {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::MissingRequiredElement,
            message: "CPL missing ContentTitleText".into(),
            file: Some(cpl_path.to_path_buf()),
            line: 0,
        });
    }

    // IssueDate
    if extract_tag(content, "IssueDate").is_none() {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::MissingRequiredElement,
            message: "CPL missing IssueDate".into(),
            file: Some(cpl_path.to_path_buf()),
            line: 0,
        });
    }

    // ContentKind
    if strict
        && let Some(kind) = extract_tag(content, "ContentKind")
        && !VALID_CONTENT_KINDS.contains(&kind.as_str())
    {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::CplInvalidContentKind,
            message: format!("Non-standard ContentKind: {kind}"),
            file: Some(cpl_path.to_path_buf()),
            line: 0,
        });
    }

    // Check reels for valid edit rates, durations
    if strict {
        check_reel_compliance(cpl_path, content, notes);
    }
}

fn check_reel_compliance(cpl_path: &Path, content: &str, notes: &mut Vec<Note>) {
    // Extract edit rates from MainPicture elements
    let edit_rate_re = regex_lite::Regex::new(r"<EditRate>([^<]+)</EditRate>").unwrap();
    for cap in edit_rate_re.captures_iter(content) {
        let rate = cap[1].trim();
        if !VALID_EDIT_RATES.contains(&rate) {
            notes.push(Note {
                severity: Severity::Warning,
                code: Code::CplInvalidEditRate,
                message: format!("Non-standard EditRate: {rate}"),
                file: Some(cpl_path.to_path_buf()),
                line: 0,
            });
        }
    }

    // Check for zero/negative durations
    let dur_re = regex_lite::Regex::new(r"<Duration>(\d+)</Duration>").unwrap();
    for cap in dur_re.captures_iter(content) {
        if let Ok(dur) = cap[1].parse::<i32>()
            && dur <= 0
        {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::CplInvalidDuration,
                message: "Zero or negative Duration found".into(),
                file: Some(cpl_path.to_path_buf()),
                line: 0,
            });
        }
    }

    // Check EntryPoint + Duration <= IntrinsicDuration
    let resource_re = regex_lite::Regex::new(
        r"<IntrinsicDuration>(\d+)</IntrinsicDuration>[\s\S]*?<EntryPoint>(\d+)</EntryPoint>[\s\S]*?<Duration>(\d+)</Duration>",
    ).unwrap();
    for cap in resource_re.captures_iter(content) {
        let intrinsic: i32 = cap[1].parse().unwrap_or(0);
        let entry: i32 = cap[2].parse().unwrap_or(0);
        let dur: i32 = cap[3].parse().unwrap_or(0);
        if entry + dur > intrinsic {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::CplInvalidDuration,
                message: "EntryPoint + Duration exceeds IntrinsicDuration".into(),
                file: Some(cpl_path.to_path_buf()),
                line: 0,
            });
        }
    }
}

fn extract_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].trim().to_string())
}
