//! Pure IMF CPL validation logic (no I/O, no file paths).
//!
//! All functions take an `&ImfCpl` and return `Vec<ImfNote>`.
//! Consumer crates wrap these into their own Note types with file info.

use std::collections::{HashMap, HashSet};

use crate::types::*;

/// Valid ContentKind values per ST 2067-2.
pub const VALID_CONTENT_KINDS: &[&str] = &[
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
    "highlights",
    "event",
    "supplemental",
    "preview",
];

/// Known marker labels per ST 2067-3.
pub const VALID_MARKER_LABELS: &[&str] = &[
    "FFBT", "LFBT", "FFCR", "LFCR", "FFTC", "LFTC", "FFOI", "LFOI", "FFEC", "LFEC", "FFMC", "LFMC",
    "FFOB", "LFOB", "FFHS", "LFHS", "FFSW", "LFSW", "FFBW", "LFBW",
];

/// Valid App 2E edit rates (ST 2067-21).
pub const APP2E_VALID_RATES: &[(u32, u32)] = &[
    (24, 1),
    (25, 1),
    (30, 1),
    (48, 1),
    (50, 1),
    (60, 1),
    (24000, 1001),
    (30000, 1001),
    (48000, 1001),
    (60000, 1001),
];

/// Valid App 5 ACES edit rates (ST 2067-50).
pub const APP5_VALID_RATES: &[(u32, u32)] = &[
    (24, 1),
    (25, 1),
    (30, 1),
    (48, 1),
    (50, 1),
    (60, 1),
    (24000, 1001),
    (30000, 1001),
    (60000, 1001),
];

/// Run all pure CPL validators and return combined notes.
pub fn validate_imf_cpl_pure(cpl: &ImfCpl) -> Vec<ImfNote> {
    let mut notes = Vec::new();
    validate_application(cpl, &mut notes);
    validate_virtual_tracks(cpl, &mut notes);
    validate_edit_rates(cpl, &mut notes);
    validate_app_constraints(cpl, &mut notes);
    validate_timeline_alignment(cpl, &mut notes);
    validate_uuids(cpl, &mut notes);
    validate_content_kind(cpl, &mut notes);
    validate_issue_date(cpl, &mut notes);
    validate_segment_structure(cpl, &mut notes);
    validate_markers(cpl, &mut notes);
    validate_iab_tracks(cpl, &mut notes);
    validate_essence_descriptor_list(cpl, &mut notes);
    notes
}

/// Validate track file references against a set of AssetMap IDs.
pub fn validate_track_refs(cpl: &ImfCpl, asset_ids: &HashSet<String>) -> Vec<ImfNote> {
    let mut notes = Vec::new();
    let referenced_ids: HashSet<&str> = cpl
        .virtual_tracks
        .iter()
        .flat_map(|vt| vt.resources.iter())
        .map(|r| r.track_file_id.as_str())
        .filter(|id| !id.is_empty())
        .collect();

    for ref_id in &referenced_ids {
        if !asset_ids.contains(*ref_id) {
            notes.push(ImfNote {
                severity: ImfSeverity::Error,
                code: "cross_ref_broken",
                message: format!(
                    "Track file {} referenced in CPL not found in AssetMap",
                    ref_id
                ),
            });
        }
    }
    notes
}

/// Where a CPL-referenced track-file id resolves across a package and its OV.
/// Shared by the IMF and DCP cross-ref checkers.
#[derive(Debug, PartialEq, Eq)]
pub enum RefStatus {
    /// present in this package
    Local,
    /// present only in the OV package
    Ov,
    /// present in neither, and an OV was supplied: a genuine broken reference
    BrokenWithOv,
    /// present in neither and no OV supplied: likely a supplemental reference
    UnresolvedNoOv,
}

pub fn resolve_track_ref(
    id: &str,
    local: &HashSet<String>,
    ov: &HashSet<String>,
    ov_provided: bool,
) -> RefStatus {
    if local.contains(id) {
        RefStatus::Local
    } else if ov.contains(id) {
        RefStatus::Ov
    } else if ov_provided {
        RefStatus::BrokenWithOv
    } else {
        RefStatus::UnresolvedNoOv
    }
}

/// OV-aware track-ref validation for a supplemental CPL. Refs resolving in the
/// OV pass; a ref in neither package is a hard error when the OV is present,
/// and a single `supplemental_ov_not_provided` warning when it is not (a
/// legitimate supplemental ref and a corrupt one are indistinguishable without
/// the OV).
pub fn validate_track_refs_ov(
    cpl: &ImfCpl,
    local_ids: &HashSet<String>,
    ov_ids: &HashSet<String>,
    ov_provided: bool,
) -> Vec<ImfNote> {
    let mut notes = Vec::new();
    let referenced_ids: HashSet<&str> = cpl
        .virtual_tracks
        .iter()
        .flat_map(|vt| vt.resources.iter())
        .map(|r| r.track_file_id.as_str())
        .filter(|id| !id.is_empty())
        .collect();

    let mut needs_ov = 0usize;
    for ref_id in &referenced_ids {
        match resolve_track_ref(ref_id, local_ids, ov_ids, ov_provided) {
            RefStatus::Local | RefStatus::Ov => {}
            RefStatus::BrokenWithOv => notes.push(ImfNote {
                severity: ImfSeverity::Error,
                code: "cross_ref_broken",
                message: format!(
                    "Track file {} referenced in CPL not found in this package or the OV",
                    ref_id
                ),
            }),
            RefStatus::UnresolvedNoOv => needs_ov += 1,
        }
    }

    if needs_ov > 0 {
        notes.push(ImfNote {
            severity: ImfSeverity::Warning,
            code: "supplemental_ov_not_provided",
            message: format!(
                "CPL references {needs_ov} asset(s) not in this package; supply the OV to fully validate"
            ),
        });
    }

    notes
}

// ─── Individual Validators ─────────────────────────────────────────────────────

fn validate_application(cpl: &ImfCpl, notes: &mut Vec<ImfNote>) {
    if cpl.application == ImfApplication::Unknown {
        notes.push(ImfNote {
            severity: ImfSeverity::Warning,
            code: "missing_required_element",
            message: "No recognized IMF Application identified in CPL namespaces".to_string(),
        });
    }

    let has_core = cpl.namespaces.iter().any(|ns| ns.contains("2067-2"));
    if !has_core {
        notes.push(ImfNote {
            severity: ImfSeverity::Warning,
            code: "smpte_namespace_wrong",
            message: "CPL missing ST 2067-2 core constraints namespace".to_string(),
        });
    }
}

fn validate_virtual_tracks(cpl: &ImfCpl, notes: &mut Vec<ImfNote>) {
    let mut has_main_image = false;

    for vt in &cpl.virtual_tracks {
        if vt.track_type == TrackType::MainImage {
            has_main_image = true;
        }

        if vt.resources.is_empty() {
            notes.push(ImfNote {
                severity: ImfSeverity::Error,
                code: "cpl_missing_reel",
                message: format!(
                    "Virtual track {} ({:?}) has no resources",
                    vt.id, vt.track_type
                ),
            });
            continue;
        }

        // MainImage total duration check
        let total: u64 = vt.resources.iter().map(|r| r.effective_duration()).sum();
        if cpl.total_duration > 0
            && total != cpl.total_duration
            && vt.track_type == TrackType::MainImage
        {
            notes.push(ImfNote {
                severity: ImfSeverity::Error,
                code: "reel_discontinuity",
                message: format!(
                    "MainImage virtual track duration ({}) does not match CPL duration ({})",
                    total, cpl.total_duration
                ),
            });
        }

        // Entry point / duration bounds
        for res in &vt.resources {
            if res.entry_point >= res.intrinsic_duration && res.intrinsic_duration > 0 {
                notes.push(ImfNote {
                    severity: ImfSeverity::Error,
                    code: "cpl_invalid_duration",
                    message: format!(
                        "Resource {} has entry_point ({}) >= intrinsic_duration ({})",
                        res.id, res.entry_point, res.intrinsic_duration
                    ),
                });
            }
            if res.source_duration > 0
                && res.entry_point + res.source_duration > res.intrinsic_duration
                && res.intrinsic_duration > 0
            {
                notes.push(ImfNote {
                    severity: ImfSeverity::Error,
                    code: "cpl_invalid_duration",
                    message: format!(
                        "Resource {} source range exceeds intrinsic duration ({} + {} > {})",
                        res.id, res.entry_point, res.source_duration, res.intrinsic_duration
                    ),
                });
            }
        }
    }

    if !has_main_image {
        notes.push(ImfNote {
            severity: ImfSeverity::Error,
            code: "missing_required_element",
            message: "CPL has no MainImageSequence virtual track".to_string(),
        });
    }
}

fn validate_edit_rates(cpl: &ImfCpl, notes: &mut Vec<ImfNote>) {
    if cpl.edit_rate == (0, 0) {
        notes.push(ImfNote {
            severity: ImfSeverity::Error,
            code: "cpl_invalid_edit_rate",
            message: "CPL has no EditRate".to_string(),
        });
        return;
    }

    for vt in &cpl.virtual_tracks {
        if vt.track_type != TrackType::MainImage {
            continue;
        }
        for res in &vt.resources {
            if res.edit_rate == (0, 0) {
                continue;
            }
            let cpl_fps = cpl.edit_rate.0 as f64 / cpl.edit_rate.1 as f64;
            let res_fps = res.edit_rate.0 as f64 / res.edit_rate.1 as f64;
            let ratio = res_fps / cpl_fps;
            if (ratio - ratio.round()).abs() > 0.001 {
                notes.push(ImfNote {
                    severity: ImfSeverity::Error,
                    code: "cpl_invalid_edit_rate",
                    message: format!(
                        "Resource edit rate {}/{} is not compatible with CPL edit rate {}/{}",
                        res.edit_rate.0, res.edit_rate.1, cpl.edit_rate.0, cpl.edit_rate.1
                    ),
                });
            }
        }
    }
}

fn validate_app_constraints(cpl: &ImfCpl, notes: &mut Vec<ImfNote>) {
    match cpl.application {
        ImfApplication::App2e => validate_app2e(cpl, notes),
        ImfApplication::App5Aces => validate_app5(cpl, notes),
        ImfApplication::Unknown => {}
    }
}

fn validate_app2e(cpl: &ImfCpl, notes: &mut Vec<ImfNote>) {
    if cpl.edit_rate != (0, 0) && !APP2E_VALID_RATES.contains(&cpl.edit_rate) {
        notes.push(ImfNote {
            severity: ImfSeverity::Error,
            code: "cpl_invalid_edit_rate",
            message: format!(
                "App 2E: invalid composition edit rate {}/{} (ST 2067-21 Section 5.2)",
                cpl.edit_rate.0, cpl.edit_rate.1
            ),
        });
    }

    let has_audio = cpl
        .virtual_tracks
        .iter()
        .any(|vt| vt.track_type == TrackType::MainAudio);
    if !has_audio {
        notes.push(ImfNote {
            severity: ImfSeverity::Warning,
            code: "missing_required_element",
            message: "App 2E: no MainAudioSequence found (recommended by ST 2067-21)".to_string(),
        });
    }
}

fn validate_app5(cpl: &ImfCpl, notes: &mut Vec<ImfNote>) {
    if cpl.edit_rate != (0, 0) && !APP5_VALID_RATES.contains(&cpl.edit_rate) {
        notes.push(ImfNote {
            severity: ImfSeverity::Error,
            code: "cpl_invalid_edit_rate",
            message: format!(
                "App 5 ACES: invalid composition edit rate {}/{} (ST 2067-50)",
                cpl.edit_rate.0, cpl.edit_rate.1
            ),
        });
    }
}

fn validate_timeline_alignment(cpl: &ImfCpl, notes: &mut Vec<ImfNote>) {
    if cpl.virtual_tracks.is_empty() {
        return;
    }

    let reference_duration = cpl
        .virtual_tracks
        .iter()
        .find(|vt| vt.track_type == TrackType::MainImage)
        .map(|vt| {
            vt.resources
                .iter()
                .map(|r| r.effective_duration())
                .sum::<u64>()
        });

    let reference_duration = match reference_duration {
        Some(d) if d > 0 => d,
        _ => return,
    };

    for vt in &cpl.virtual_tracks {
        if vt.track_type == TrackType::Marker || vt.resources.is_empty() {
            continue;
        }
        let track_duration: u64 = vt.resources.iter().map(|r| r.effective_duration()).sum();
        if track_duration > 0 && track_duration != reference_duration {
            notes.push(ImfNote {
                severity: ImfSeverity::Error,
                code: "cpl_mismatched_durations",
                message: format!(
                    "{:?} track {} duration ({}) differs from MainImage duration ({})",
                    vt.track_type, vt.id, track_duration, reference_duration
                ),
            });
        }
    }
}

fn validate_uuids(cpl: &ImfCpl, notes: &mut Vec<ImfNote>) {
    let mut seen: HashSet<String> = HashSet::new();
    for uuid in &cpl.all_uuids {
        let parts: Vec<&str> = uuid.split('-').collect();
        let valid_format = parts.len() == 5
            && parts[0].len() == 8
            && parts[1].len() == 4
            && parts[2].len() == 4
            && parts[3].len() == 4
            && parts[4].len() == 12
            && uuid.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
        if !valid_format {
            notes.push(ImfNote {
                severity: ImfSeverity::Error,
                code: "invalid_uuid",
                message: format!("Malformed UUID: '{uuid}'"),
            });
        }
        let lower = uuid.to_lowercase();
        if !seen.insert(lower) {
            notes.push(ImfNote {
                severity: ImfSeverity::Error,
                code: "duplicate_asset_id",
                message: format!("Duplicate UUID in CPL: '{uuid}'"),
            });
        }
    }
}

fn validate_content_kind(cpl: &ImfCpl, notes: &mut Vec<ImfNote>) {
    if cpl.content_kind.is_empty() {
        notes.push(ImfNote {
            severity: ImfSeverity::Warning,
            code: "missing_required_element",
            message: "CPL has no ContentKind element".to_string(),
        });
        return;
    }
    let kind_lower = cpl.content_kind.to_lowercase();
    if !VALID_CONTENT_KINDS.contains(&kind_lower.as_str()) {
        notes.push(ImfNote {
            severity: ImfSeverity::Warning,
            code: "cpl_invalid_content_kind",
            message: format!(
                "ContentKind '{}' is not a recognized value (expected one of: {})",
                cpl.content_kind,
                VALID_CONTENT_KINDS.join(", ")
            ),
        });
    }
}

fn validate_issue_date(cpl: &ImfCpl, notes: &mut Vec<ImfNote>) {
    if cpl.issue_date.is_empty() {
        notes.push(ImfNote {
            severity: ImfSeverity::Warning,
            code: "missing_required_element",
            message: "CPL has no IssueDate element".to_string(),
        });
        return;
    }
    let valid = cpl.issue_date.len() >= 19
        && cpl.issue_date.chars().nth(4) == Some('-')
        && cpl.issue_date.chars().nth(7) == Some('-')
        && cpl.issue_date.chars().nth(10) == Some('T')
        && cpl.issue_date.chars().nth(13) == Some(':')
        && cpl.issue_date.chars().nth(16) == Some(':');
    if !valid {
        notes.push(ImfNote {
            severity: ImfSeverity::Error,
            code: "xml_schema_violation",
            message: format!(
                "IssueDate '{}' is not valid ISO 8601 (expected YYYY-MM-DDThh:mm:ss...)",
                cpl.issue_date
            ),
        });
    }
}

fn validate_segment_structure(cpl: &ImfCpl, notes: &mut Vec<ImfNote>) {
    if cpl.segment_count == 0 {
        notes.push(ImfNote {
            severity: ImfSeverity::Error,
            code: "missing_required_element",
            message: "CPL has no Segment elements (at least one required)".to_string(),
        });
    }
}

fn validate_markers(cpl: &ImfCpl, notes: &mut Vec<ImfNote>) {
    for marker in &cpl.markers {
        if marker.label.is_empty() {
            notes.push(ImfNote {
                severity: ImfSeverity::Error,
                code: "marker_invalid",
                message: "Marker has empty label".to_string(),
            });
            continue;
        }

        if !VALID_MARKER_LABELS.contains(&marker.label.as_str()) {
            notes.push(ImfNote {
                severity: ImfSeverity::Info,
                code: "marker_invalid",
                message: format!(
                    "Marker label '{}' is not a standard ST 2067-3 marker",
                    marker.label
                ),
            });
        }

        if cpl.total_duration > 0 && marker.offset > cpl.total_duration {
            notes.push(ImfNote {
                severity: ImfSeverity::Error,
                code: "marker_invalid",
                message: format!(
                    "Marker '{}' offset ({}) exceeds composition duration ({})",
                    marker.label, marker.offset, cpl.total_duration
                ),
            });
        }
    }

    // Validate paired markers (FFXX must appear before LFXX)
    let offsets: HashMap<&str, u64> = cpl
        .markers
        .iter()
        .map(|m| (m.label.as_str(), m.offset))
        .collect();

    let pairs = [
        ("FFBT", "LFBT"),
        ("FFCR", "LFCR"),
        ("FFTC", "LFTC"),
        ("FFOI", "LFOI"),
        ("FFEC", "LFEC"),
        ("FFMC", "LFMC"),
        ("FFOB", "LFOB"),
    ];

    for (first, last) in pairs {
        if let (Some(&ff), Some(&lf)) = (offsets.get(first), offsets.get(last)) {
            if ff > lf {
                notes.push(ImfNote {
                    severity: ImfSeverity::Error,
                    code: "marker_invalid",
                    message: format!(
                        "Marker {} (offset {}) occurs after {} (offset {})",
                        first, ff, last, lf
                    ),
                });
            }
        }
    }
}

fn validate_iab_tracks(cpl: &ImfCpl, notes: &mut Vec<ImfNote>) {
    for vt in &cpl.virtual_tracks {
        if vt.track_type != TrackType::IAB {
            continue;
        }
        if vt.resources.is_empty() {
            notes.push(ImfNote {
                severity: ImfSeverity::Error,
                code: "cpl_missing_reel",
                message: format!("IAB virtual track {} has no resources", vt.id),
            });
            continue;
        }

        let image_rate = cpl
            .virtual_tracks
            .iter()
            .find(|v| v.track_type == TrackType::MainImage)
            .and_then(|v| v.resources.first())
            .map(|r| r.edit_rate)
            .unwrap_or(cpl.edit_rate);

        for res in &vt.resources {
            if res.edit_rate != (0, 0) && res.edit_rate != image_rate && image_rate != (0, 0) {
                notes.push(ImfNote {
                    severity: ImfSeverity::Error,
                    code: "cpl_invalid_edit_rate",
                    message: format!(
                        "IAB track edit rate {}/{} must match MainImage edit rate {}/{}",
                        res.edit_rate.0, res.edit_rate.1, image_rate.0, image_rate.1
                    ),
                });
            }
        }

        // IAB track duration must match MainImage
        let iab_duration: u64 = vt.resources.iter().map(|r| r.effective_duration()).sum();
        if cpl.total_duration > 0 && iab_duration != cpl.total_duration {
            notes.push(ImfNote {
                severity: ImfSeverity::Error,
                code: "cpl_mismatched_durations",
                message: format!(
                    "IAB track duration ({}) differs from MainImage ({})",
                    iab_duration, cpl.total_duration
                ),
            });
        }
    }
}

fn validate_essence_descriptor_list(cpl: &ImfCpl, notes: &mut Vec<ImfNote>) {
    if cpl.essence_descriptors.is_empty() {
        notes.push(ImfNote {
            severity: ImfSeverity::Info,
            code: "missing_required_element",
            message: "CPL has no EssenceDescriptorList (recommended for interoperability)"
                .to_string(),
        });
        return;
    }

    let referenced_ids: HashSet<&str> = cpl
        .virtual_tracks
        .iter()
        .flat_map(|vt| vt.resources.iter())
        .map(|r| r.track_file_id.as_str())
        .filter(|id| !id.is_empty())
        .collect();

    let descriptor_ids: HashSet<&str> =
        cpl.essence_descriptors.keys().map(|k| k.as_str()).collect();

    for ref_id in &referenced_ids {
        if !descriptor_ids.contains(ref_id) {
            notes.push(ImfNote {
                severity: ImfSeverity::Warning,
                code: "cross_ref_broken",
                message: format!(
                    "Track file {} referenced in CPL has no matching EssenceDescriptor",
                    ref_id
                ),
            });
        }
    }

    for desc_id in &descriptor_ids {
        if !referenced_ids.contains(desc_id) {
            notes.push(ImfNote {
                severity: ImfSeverity::Warning,
                code: "cross_ref_broken",
                message: format!(
                    "EssenceDescriptor for track file {} is not referenced by any resource",
                    desc_id
                ),
            });
        }
    }

    // Validate descriptor internals per application
    if cpl.application == ImfApplication::App2e {
        for desc in cpl.essence_descriptors.values() {
            validate_app2e_descriptor(desc, notes);
        }
    } else if cpl.application == ImfApplication::App5Aces {
        for desc in cpl.essence_descriptors.values() {
            validate_app5_descriptor(desc, notes);
        }
    }
}

fn validate_app2e_descriptor(desc: &EssenceDescriptor, notes: &mut Vec<ImfNote>) {
    // Picture descriptors
    if desc.descriptor_type.contains("CDCI")
        || desc.descriptor_type.contains("RGBA")
        || desc.descriptor_type.contains("JPEG2000")
    {
        let valid_resolutions = [(1920, 1080), (2048, 1080), (3840, 2160), (4096, 2160)];
        if desc.stored_width > 0
            && desc.stored_height > 0
            && !valid_resolutions.contains(&(desc.stored_width, desc.stored_height))
        {
            notes.push(ImfNote {
                severity: ImfSeverity::Error,
                code: "picture_invalid_resolution",
                message: format!(
                    "EssenceDescriptor: invalid resolution {}x{} for App 2E",
                    desc.stored_width, desc.stored_height
                ),
            });
        }
        if desc.component_depth > 0 && !matches!(desc.component_depth, 8 | 10 | 12) {
            notes.push(ImfNote {
                severity: ImfSeverity::Error,
                code: "mxf_invalid_structure",
                message: format!(
                    "EssenceDescriptor: invalid bit depth {} for App 2E (allowed: 8, 10, 12)",
                    desc.component_depth
                ),
            });
        }
    }

    // Audio descriptors
    if desc.descriptor_type.contains("Wave") || desc.descriptor_type.contains("Audio") {
        if desc.audio_sampling_rate.0 > 0 {
            let rate = desc.audio_sampling_rate.0 / desc.audio_sampling_rate.1.max(1);
            if !matches!(rate, 48000 | 96000) {
                notes.push(ImfNote {
                    severity: ImfSeverity::Error,
                    code: "sound_invalid_sample_rate",
                    message: format!(
                        "EssenceDescriptor: invalid audio sample rate {} for App 2E (allowed: 48000, 96000)",
                        rate
                    ),
                });
            }
        }
        if desc.quantization_bits > 0 && desc.quantization_bits != 24 {
            notes.push(ImfNote {
                severity: ImfSeverity::Error,
                code: "sound_invalid_channel_count",
                message: format!(
                    "EssenceDescriptor: invalid audio bit depth {} for App 2E (required: 24)",
                    desc.quantization_bits
                ),
            });
        }
    }
}

fn validate_app5_descriptor(desc: &EssenceDescriptor, notes: &mut Vec<ImfNote>) {
    if desc.descriptor_type.contains("CDCI")
        || desc.descriptor_type.contains("RGBA")
        || desc.descriptor_type.contains("JPEG2000")
    {
        if !desc.color_primaries.is_empty() {
            let is_aces_primaries = desc.color_primaries.contains("03.07")
                || desc.color_primaries.contains("0307")
                || desc.color_primaries.to_lowercase().contains("aces");
            if !is_aces_primaries {
                notes.push(ImfNote {
                    severity: ImfSeverity::Warning,
                    code: "mxf_invalid_structure",
                    message: format!(
                        "App 5 ACES: color primaries '{}' may not be ACES (expected AP0/AP1)",
                        desc.color_primaries
                    ),
                });
            }
        }

        if !desc.transfer_characteristic.is_empty() {
            let is_linear = desc.transfer_characteristic.contains("01.01")
                || desc.transfer_characteristic.contains("0101")
                || desc
                    .transfer_characteristic
                    .to_lowercase()
                    .contains("linear");
            if !is_linear {
                notes.push(ImfNote {
                    severity: ImfSeverity::Warning,
                    code: "mxf_invalid_structure",
                    message: format!(
                        "App 5 ACES: transfer characteristic '{}' should be linear",
                        desc.transfer_characteristic
                    ),
                });
            }
        }

        if desc.component_depth > 0 && desc.component_depth != 16 {
            notes.push(ImfNote {
                severity: ImfSeverity::Info,
                code: "mxf_invalid_structure",
                message: format!(
                    "App 5 ACES: component depth {} (ACES typically uses 16-bit half-float)",
                    desc.component_depth
                ),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_missing_main_image() {
        let cpl = ImfCpl {
            segment_count: 1,
            edit_rate: (24, 1),
            content_kind: "feature".to_string(),
            issue_date: "2024-01-15T10:30:00+00:00".to_string(),
            namespaces: vec!["http://www.smpte-ra.org/schemas/2067-2/2016".to_string()],
            ..Default::default()
        };
        let notes = validate_imf_cpl_pure(&cpl);
        assert!(notes
            .iter()
            .any(|n| n.message.contains("MainImageSequence")));
    }

    #[test]
    fn test_validate_invalid_edit_rate_app2e() {
        let cpl = ImfCpl {
            edit_rate: (23, 1), // Invalid for App 2E
            application: ImfApplication::App2e,
            segment_count: 1,
            content_kind: "feature".to_string(),
            issue_date: "2024-01-15T10:30:00+00:00".to_string(),
            namespaces: vec![
                "http://www.smpte-ra.org/schemas/2067-2/2016".to_string(),
                "http://www.smpte-ra.org/ns/2067-21/2021".to_string(),
            ],
            virtual_tracks: vec![VirtualTrack {
                track_type: TrackType::MainImage,
                resources: vec![TrackResource {
                    intrinsic_duration: 100,
                    source_duration: 100,
                    edit_rate: (23, 1),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            total_duration: 100,
            ..Default::default()
        };
        let notes = validate_imf_cpl_pure(&cpl);
        assert!(notes
            .iter()
            .any(|n| n.message.contains("App 2E") && n.message.contains("invalid")));
    }

    #[test]
    fn test_validate_duplicate_uuid() {
        let cpl = ImfCpl {
            all_uuids: vec![
                "12345678-1234-1234-1234-123456789abc".to_string(),
                "12345678-1234-1234-1234-123456789abc".to_string(),
            ],
            segment_count: 1,
            edit_rate: (24, 1),
            content_kind: "feature".to_string(),
            issue_date: "2024-01-15T10:30:00+00:00".to_string(),
            namespaces: vec!["http://www.smpte-ra.org/schemas/2067-2/2016".to_string()],
            virtual_tracks: vec![VirtualTrack {
                track_type: TrackType::MainImage,
                resources: vec![TrackResource {
                    intrinsic_duration: 100,
                    source_duration: 100,
                    edit_rate: (24, 1),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            total_duration: 100,
            ..Default::default()
        };
        let notes = validate_imf_cpl_pure(&cpl);
        assert!(notes.iter().any(|n| n.code == "duplicate_asset_id"));
    }

    #[test]
    fn test_validate_markers_paired() {
        let cpl = ImfCpl {
            markers: vec![
                Marker {
                    label: "LFBT".to_string(),
                    offset: 10,
                    ..Default::default()
                },
                Marker {
                    label: "FFBT".to_string(),
                    offset: 50, // FFBT should come before LFBT
                    ..Default::default()
                },
            ],
            segment_count: 1,
            edit_rate: (24, 1),
            total_duration: 100,
            content_kind: "feature".to_string(),
            issue_date: "2024-01-15T10:30:00+00:00".to_string(),
            namespaces: vec!["http://www.smpte-ra.org/schemas/2067-2/2016".to_string()],
            virtual_tracks: vec![VirtualTrack {
                track_type: TrackType::MainImage,
                resources: vec![TrackResource {
                    intrinsic_duration: 100,
                    source_duration: 100,
                    edit_rate: (24, 1),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let notes = validate_imf_cpl_pure(&cpl);
        assert!(notes
            .iter()
            .any(|n| n.code == "marker_invalid" && n.message.contains("FFBT")));
    }

    #[test]
    fn test_validate_track_refs() {
        let cpl = ImfCpl {
            virtual_tracks: vec![VirtualTrack {
                track_type: TrackType::MainImage,
                resources: vec![TrackResource {
                    track_file_id: "missing-id".to_string(),
                    intrinsic_duration: 100,
                    source_duration: 100,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let asset_ids: HashSet<String> = ["other-id".to_string()].into_iter().collect();
        let notes = validate_track_refs(&cpl, &asset_ids);
        assert!(notes.iter().any(|n| n.code == "cross_ref_broken"));
    }

    #[test]
    fn ov_track_refs_resolve_local_ov_broken_and_needs_ov() {
        let cpl = ImfCpl {
            virtual_tracks: vec![VirtualTrack {
                track_type: TrackType::MainImage,
                resources: vec![TrackResource {
                    track_file_id: "pic".to_string(),
                    intrinsic_duration: 100,
                    source_duration: 100,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let local: HashSet<String> = HashSet::new();
        let ov: HashSet<String> = ["pic".to_string()].into();

        // resolves in OV -> clean
        assert!(validate_track_refs_ov(&cpl, &local, &ov, true).is_empty());
        // OV given but ref in neither -> hard break
        let broken = validate_track_refs_ov(&cpl, &local, &HashSet::new(), true);
        assert!(broken.iter().any(|n| n.code == "cross_ref_broken"));
        // no OV -> soft supplemental warning, not a break
        let soft = validate_track_refs_ov(&cpl, &local, &HashSet::new(), false);
        assert!(soft
            .iter()
            .any(|n| n.code == "supplemental_ov_not_provided"));
        assert!(!soft.iter().any(|n| n.code == "cross_ref_broken"));
    }
}
