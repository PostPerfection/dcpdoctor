use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use std::collections::HashMap;

use crate::assetmap::{self, AssetMap};
use crate::cpl::{self, Cpl};
use crate::hash;
use crate::naming;
use crate::pkl::{self, Pkl};
use crate::{FileEntry, Note, Severity, Summary, ValidationResult};

/// Run full DCP validation on a set of in-memory files.
pub fn run_validation(files: &[FileEntry]) -> ValidationResult {
    let mut notes = Vec::new();
    let mut hashes_verified: usize = 0;
    let mut hashes_failed: usize = 0;

    // Build file map: path -> content
    let file_map: HashMap<&str, &FileEntry> = files.iter().map(|f| (f.path.as_str(), f)).collect();

    // 1. Find ASSETMAP
    let assetmap_entry = file_map
        .get("ASSETMAP.xml")
        .or_else(|| file_map.get("ASSETMAP"));

    let assetmap_entry = match assetmap_entry {
        Some(e) => *e,
        None => {
            notes.push(Note {
                severity: Severity::Error,
                code: "missing_assetmap".to_string(),
                message: "No ASSETMAP or ASSETMAP.xml found in DCP".to_string(),
                file: None,
            });
            return build_result(
                false,
                "unknown",
                notes,
                files.len(),
                0,
                0,
                0,
                HashMap::new(),
            );
        }
    };

    let standard = if file_map.contains_key("ASSETMAP.xml") {
        "smpte"
    } else {
        "interop"
    };

    // 2. Parse ASSETMAP
    let assetmap_xml = get_content(assetmap_entry);
    let assetmap = match assetmap::parse_assetmap(&assetmap_xml) {
        Ok(am) => am,
        Err(e) => {
            notes.push(Note {
                severity: Severity::Error,
                code: "xml_parse_error".to_string(),
                message: format!("Failed to parse ASSETMAP: {e}"),
                file: Some(assetmap_entry.path.clone()),
            });
            return build_result(false, standard, notes, files.len(), 0, 0, 0, HashMap::new());
        }
    };

    // 3. Check ASSETMAP references exist
    for asset in &assetmap.assets {
        if !asset.path.is_empty() && !file_map.contains_key(asset.path.as_str()) {
            notes.push(Note {
                severity: Severity::Error,
                code: "asset_not_found".to_string(),
                message: format!("Asset referenced in ASSETMAP not found: {}", asset.path),
                file: Some("ASSETMAP".to_string()),
            });
        }
    }

    // 4. Find and parse PKLs
    let pkl_paths: Vec<&str> = assetmap
        .assets
        .iter()
        .filter(|a| a.is_pkl)
        .map(|a| a.path.as_str())
        .collect();

    if pkl_paths.is_empty() {
        notes.push(Note {
            severity: Severity::Error,
            code: "missing_pkl".to_string(),
            message: "No Packing List (PKL) found in ASSETMAP".to_string(),
            file: None,
        });
        return build_result(false, standard, notes, files.len(), 0, 0, 0, HashMap::new());
    }

    let mut pkls: Vec<Pkl> = Vec::new();
    for pkl_path in &pkl_paths {
        if let Some(entry) = file_map.get(pkl_path) {
            let xml = get_content(entry);
            match pkl::parse_pkl(&xml) {
                Ok(p) => pkls.push(p),
                Err(e) => {
                    notes.push(Note {
                        severity: Severity::Error,
                        code: "xml_parse_error".to_string(),
                        message: format!("Failed to parse PKL {pkl_path}: {e}"),
                        file: Some(pkl_path.to_string()),
                    });
                }
            }
        }
    }

    // 5. Verify PKL hashes against file contents
    let mut hashes_skipped: usize = 0;
    let mut asset_hashes: HashMap<String, String> = HashMap::new();
    for pkl in &pkls {
        for asset in &pkl.assets {
            // Find the file by matching against assetmap paths
            let file_path = find_file_for_asset(&assetmap, &asset.id, &file_map);
            if let Some(path) = file_path {
                if let Some(entry) = file_map.get(path.as_str()) {
                    if entry.skipped {
                        hashes_skipped += 1;
                        // Store expected hash for browser hash queue
                        if !asset.hash.is_empty() {
                            asset_hashes.insert(path.clone(), asset.hash.clone());
                        }
                        continue;
                    }
                    let content_bytes = get_raw_bytes(entry);
                    let computed = hash::compute_sha1_base64(&content_bytes);
                    if computed == asset.hash {
                        hashes_verified += 1;
                    } else {
                        hashes_failed += 1;
                        notes.push(Note {
                            severity: Severity::Error,
                            code: "pkl_hash_mismatch".to_string(),
                            message: format!(
                                "Hash mismatch for {}: expected {}, computed {}",
                                path, asset.hash, computed
                            ),
                            file: Some(path),
                        });
                    }
                }
            }
        }
    }

    // 6. Find and parse CPLs
    let mut cpls: Vec<(String, Cpl)> = Vec::new();
    for pkl in &pkls {
        for asset in &pkl.assets {
            if asset.asset_type.contains("composition-playlist")
                || asset.asset_type.contains("CompositionPlaylist")
            {
                let file_path = find_file_for_asset(&assetmap, &asset.id, &file_map);
                if let Some(path) = file_path {
                    if let Some(entry) = file_map.get(path.as_str()) {
                        let xml = get_content(entry);
                        match cpl::parse_cpl(&xml) {
                            Ok(c) => cpls.push((path, c)),
                            Err(e) => {
                                notes.push(Note {
                                    severity: Severity::Error,
                                    code: "xml_parse_error".to_string(),
                                    message: format!("Failed to parse CPL: {e}"),
                                    file: Some(path.clone()),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    if cpls.is_empty() {
        // Try to find CPL files by extension
        for (path, entry) in &file_map {
            let lower = path.to_lowercase();
            if lower.contains("cpl") && lower.ends_with(".xml") {
                let xml = get_content(entry);
                if xml.contains("CompositionPlaylist") {
                    if let Ok(c) = cpl::parse_cpl(&xml) {
                        cpls.push((path.to_string(), c));
                    }
                }
            }
        }
        if cpls.is_empty() {
            notes.push(Note {
                severity: Severity::Error,
                code: "missing_cpl".to_string(),
                message: "No Composition Playlist (CPL) found".to_string(),
                file: None,
            });
        }
    }

    // 7. Validate CPLs
    for (path, cpl) in &cpls {
        validate_cpl(cpl, path, &mut notes);
    }

    // 7b. Native IMF validation (App 2E / App 5 checks)
    for (path, _cpl) in &cpls {
        if let Some(entry) = file_map.get(path.as_str()) {
            let xml = get_content(entry);
            if xml.contains("2067") {
                let am_xml = get_content(assetmap_entry);
                let imf_notes = crate::imf::validate_imf_cpl(&xml, Some(&am_xml), path);
                notes.extend(imf_notes);
            }
        }
    }

    // 8. Check for foreign files (files not in ASSETMAP)
    let known_paths: std::collections::HashSet<&str> = assetmap
        .assets
        .iter()
        .map(|a| a.path.as_str())
        .chain(std::iter::once(if standard == "smpte" {
            "ASSETMAP.xml"
        } else {
            "ASSETMAP"
        }))
        .collect();

    for path in file_map.keys() {
        if !known_paths.contains(path) && !path.starts_with('.') {
            notes.push(Note {
                severity: Severity::Warning,
                code: "foreign_file".to_string(),
                message: format!("File not referenced in ASSETMAP: {path}"),
                file: Some(path.to_string()),
            });
        }
    }

    // 9. ISDCF naming check
    for (_path, cpl) in &cpls {
        let naming_notes = naming::check_naming(&cpl.content_title);
        notes.extend(naming_notes);
    }

    if hashes_skipped > 0 {
        notes.push(Note {
            severity: Severity::Info,
            code: "hashes_skipped".to_string(),
            message: format!(
                "{hashes_skipped} binary file(s) skipped for hash verification (use CLI for full check)"
            ),
            file: None,
        });
    }

    let has_errors = notes.iter().any(|n| matches!(n.severity, Severity::Error));
    build_result(
        !has_errors,
        standard,
        notes,
        files.len(),
        hashes_verified,
        hashes_failed,
        hashes_skipped,
        asset_hashes,
    )
}

fn validate_cpl(cpl: &Cpl, path: &str, notes: &mut Vec<Note>) {
    // Check UUID
    if cpl.id.is_empty() {
        notes.push(Note {
            severity: Severity::Error,
            code: "cpl_missing_id".to_string(),
            message: "CPL has no Id".to_string(),
            file: Some(path.to_string()),
        });
    }

    // Check content title
    if cpl.content_title.is_empty() {
        notes.push(Note {
            severity: Severity::Warning,
            code: "cpl_missing_title".to_string(),
            message: "CPL has no ContentTitleText".to_string(),
            file: Some(path.to_string()),
        });
    }

    // Check reels
    if cpl.reels.is_empty() {
        notes.push(Note {
            severity: Severity::Error,
            code: "cpl_no_reels".to_string(),
            message: "CPL has no reels".to_string(),
            file: Some(path.to_string()),
        });
    }

    // Check edit rate: SMPTE CPLs carry it per asset, Interop may have it at CPL level
    const VALID_EDIT_RATES: [&str; 6] = ["24 1", "25 1", "30 1", "48 1", "50 1", "60 1"];
    let mut edit_rates: Vec<&str> = cpl
        .reels
        .iter()
        .map(|r| r.edit_rate.as_str())
        .filter(|r| !r.is_empty())
        .collect();
    if edit_rates.is_empty() && !cpl.edit_rate.is_empty() {
        edit_rates.push(&cpl.edit_rate);
    }

    if edit_rates.is_empty() {
        notes.push(Note {
            severity: Severity::Warning,
            code: "cpl_missing_edit_rate".to_string(),
            message: "CPL has no EditRate".to_string(),
            file: Some(path.to_string()),
        });
    }
    for rate in edit_rates {
        if !VALID_EDIT_RATES.contains(&rate) {
            notes.push(Note {
                severity: Severity::Error,
                code: "cpl_invalid_edit_rate".to_string(),
                message: format!("Non-DCI edit rate '{rate}' (allowed: 24, 25, 30, 48, 50, 60 fps)"),
                file: Some(path.to_string()),
            });
        }
    }

    // Check reel durations
    for (i, reel) in cpl.reels.iter().enumerate() {
        if reel.duration == 0 {
            notes.push(Note {
                severity: Severity::Warning,
                code: "cpl_zero_duration".to_string(),
                message: format!("Reel {} has zero duration", i + 1),
                file: Some(path.to_string()),
            });
        }
        if reel.picture_asset_id.is_empty() {
            notes.push(Note {
                severity: Severity::Warning,
                code: "cpl_no_picture".to_string(),
                message: format!("Reel {} has no picture asset", i + 1),
                file: Some(path.to_string()),
            });
        }
    }

    // Check content kind
    if !cpl.content_kind.is_empty() {
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
        if !valid_kinds.contains(&cpl.content_kind.to_lowercase().as_str()) {
            notes.push(Note {
                severity: Severity::Error,
                code: "cpl_invalid_content_kind".to_string(),
                message: format!(
                    "Invalid ContentKind '{}' (expected one of: {})",
                    cpl.content_kind,
                    valid_kinds.join(", ")
                ),
                file: Some(path.to_string()),
            });
        }
    }
}

fn find_file_for_asset(
    assetmap: &AssetMap,
    asset_id: &str,
    file_map: &HashMap<&str, &FileEntry>,
) -> Option<String> {
    for asset in &assetmap.assets {
        if asset.id == asset_id && file_map.contains_key(asset.path.as_str()) {
            return Some(asset.path.clone());
        }
    }
    None
}

fn get_content(entry: &FileEntry) -> String {
    match &entry.content {
        None => String::new(),
        Some(c) => {
            if entry.is_base64 {
                let bytes = BASE64.decode(c).unwrap_or_default();
                String::from_utf8_lossy(&bytes).to_string()
            } else {
                c.clone()
            }
        }
    }
}

fn get_raw_bytes(entry: &FileEntry) -> Vec<u8> {
    match &entry.content {
        None => Vec::new(),
        Some(c) => {
            if entry.is_base64 {
                BASE64.decode(c).unwrap_or_default()
            } else {
                c.as_bytes().to_vec()
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_result(
    valid: bool,
    standard: &str,
    notes: Vec<Note>,
    files_checked: usize,
    hashes_verified: usize,
    hashes_failed: usize,
    hashes_skipped: usize,
    asset_hashes: HashMap<String, String>,
) -> ValidationResult {
    let errors = notes
        .iter()
        .filter(|n| matches!(n.severity, Severity::Error))
        .count();
    let warnings = notes
        .iter()
        .filter(|n| matches!(n.severity, Severity::Warning))
        .count();
    let info = notes
        .iter()
        .filter(|n| matches!(n.severity, Severity::Info))
        .count();

    ValidationResult {
        valid,
        standard: standard.to_string(),
        notes,
        summary: Summary {
            errors,
            warnings,
            info,
            files_checked,
            hashes_verified,
            hashes_failed,
            hashes_skipped,
        },
        asset_hashes,
        mxf_info: HashMap::new(),
    }
}
