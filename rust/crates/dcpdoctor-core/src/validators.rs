//! Individual DCP validators — encryption, reel continuity, stereo, markers,
//! cross-references, supplemental detection, audio channels, color space.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::{Code, Note, Severity};

// ─── Encryption Detection ─────────────────────────────────────────────────────

/// Check CPLs for encrypted content and whether a KDM is present.
pub fn check_encryption(dcp_dir: &Path, cpl_paths: &[PathBuf]) -> Vec<Note> {
    let mut notes = Vec::new();

    for cpl_path in cpl_paths {
        let Ok(content) = std::fs::read_to_string(cpl_path) else {
            continue;
        };

        let key_id_count = content.matches("<KeyId>").count();
        let has_enc_keys = content.contains("<EncryptedDocumentKey");

        if key_id_count > 0 || has_enc_keys {
            notes.push(Note {
                severity: Severity::Info,
                code: Code::EncryptionDetected,
                message: format!(
                    "CPL contains encrypted content ({key_id_count} encrypted assets)"
                ),
                file: Some(cpl_path.clone()),
                line: 0,
            });

            // Check for KDM in the DCP directory
            let kdm_found = std::fs::read_dir(dcp_dir)
                .map(|entries| {
                    entries.flatten().any(|e| {
                        let name = e.file_name().to_string_lossy().to_lowercase();
                        name.contains("kdm")
                    })
                })
                .unwrap_or(false);

            if !kdm_found {
                notes.push(Note {
                    severity: Severity::Info,
                    code: Code::KdmRequired,
                    message: "No KDM file found in DCP directory for encrypted content".into(),
                    file: Some(cpl_path.clone()),
                    line: 0,
                });
            }
        }
    }

    notes
}

// ─── Reel Continuity ──────────────────────────────────────────────────────────

/// Check that reel entry-points form a continuous chain.
pub fn check_reel_continuity(cpl_path: &Path) -> Vec<Note> {
    let mut notes = Vec::new();

    let Ok(content) = std::fs::read_to_string(cpl_path) else {
        return notes;
    };

    // Find all reels and their MainPicture/MainImage entry+duration
    let reel_re = regex_lite::Regex::new(r"<Reel>([\s\S]*?)</Reel>").unwrap();
    let reels: Vec<&str> = reel_re
        .captures_iter(&content)
        .map(|c| c.get(1).unwrap().as_str())
        .collect();

    if reels.len() < 2 {
        return notes; // Single reel, nothing to check
    }

    let mut expected_entry: u64 = 0;

    for (i, reel) in reels.iter().enumerate() {
        // Look for MainPicture or MainImage block
        let pic_re = regex_lite::Regex::new(
            r"<(?:MainPicture|MainImage)>([\s\S]*?)</(?:MainPicture|MainImage)>",
        )
        .unwrap();
        let Some(pic_cap) = pic_re.captures(reel) else {
            continue;
        };
        let pic_block = pic_cap.get(1).unwrap().as_str();

        let entry_point = extract_u64(pic_block, "EntryPoint").unwrap_or(0);
        let duration = extract_u64(pic_block, "Duration").unwrap_or(0);

        if i > 0 && entry_point != 0 && entry_point != expected_entry {
            notes.push(Note {
                severity: Severity::Warning,
                code: Code::ReelDiscontinuity,
                message: format!(
                    "Reel {} EntryPoint {entry_point} does not follow previous reel (expected {expected_entry})",
                    i + 1
                ),
                file: Some(cpl_path.to_path_buf()),
                line: 0,
            });
        }

        expected_entry = entry_point + duration;
    }

    notes
}

// ─── Reel Coherence ───────────────────────────────────────────────────────────

/// Check that essence parameters are coherent across all reels of a composition.
///
/// Mirrors ClairMeta's `check_cpl_reel_coherence` (SMPTE ST 429-2 8.7): every
/// per-reel essence parameter that ClairMeta derives from the CPL must hold one
/// value across all reels that carry it. A parameter with two differing values is
/// "Mixed" and reported as an error. Encryption coherence is included: ClairMeta
/// keys it off `<KeyId>` presence, which is what makes ECL32 (one clear picture
/// reel among encrypted ones) incoherent. Values are only collected from reels
/// where the essence is present, so a track missing on some reels is not a
/// mismatch. Reports the first divergent reel per parameter.
pub fn check_reel_coherence(cpl_path: &Path) -> Vec<Note> {
    let mut notes = Vec::new();

    let Ok(content) = std::fs::read_to_string(cpl_path) else {
        return notes;
    };

    let reel_re = regex_lite::Regex::new(r"<Reel>([\s\S]*?)</Reel>").unwrap();
    let reels: Vec<&str> = reel_re
        .captures_iter(&content)
        .map(|c| c.get(1).unwrap().as_str())
        .collect();

    if reels.len() < 2 {
        return notes; // single reel is trivially coherent
    }

    // (label, essence tag alternatives, value extractor). Only the CPL-derivable
    // essence keys ClairMeta uses; MXF-probe keys (resolution, channel count,
    // sample rate) are read where present but the CPL rarely carries them.
    type ValueFn = fn(&str) -> Option<String>;
    let params: &[(&str, &[&str], ValueFn)] = &[
        (
            "picture edit rate",
            &["MainPicture", "MainStereoscopicPicture"],
            |b| extract_tag(b, "EditRate"),
        ),
        (
            "picture frame rate",
            &["MainPicture", "MainStereoscopicPicture"],
            |b| extract_tag(b, "FrameRate"),
        ),
        (
            "picture frame size",
            &["MainPicture", "MainStereoscopicPicture"],
            |b| extract_tag(b, "ScreenAspectRatio"),
        ),
        (
            "picture encryption",
            &["MainPicture", "MainStereoscopicPicture"],
            |b| Some(encrypted_str(b)),
        ),
        (
            "picture resolution",
            &["MainPicture", "MainStereoscopicPicture"],
            |b| extract_tag(b, "Resolution"),
        ),
        ("sound edit rate", &["MainSound"], |b| {
            extract_tag(b, "EditRate")
        }),
        ("sound encryption", &["MainSound"], |b| {
            Some(encrypted_str(b))
        }),
        ("sound channel count", &["MainSound"], |b| {
            extract_tag(b, "ChannelCount")
        }),
        ("sound sample rate", &["MainSound"], |b| {
            extract_tag(b, "SampleRate")
        }),
        ("subtitle edit rate", &["MainSubtitle"], |b| {
            extract_tag(b, "EditRate")
        }),
    ];

    let mut established: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
    let mut flagged: HashSet<&str> = HashSet::new();

    for (i, reel) in reels.iter().enumerate() {
        for (label, tags, value_of) in params {
            let Some(block) = essence_block(reel, tags) else {
                continue;
            };
            let Some(value) = value_of(block) else {
                continue;
            };
            match established.get(label) {
                None => {
                    established.insert(label, value);
                }
                Some(first) if *first != value && !flagged.contains(label) => {
                    notes.push(Note {
                        severity: Severity::Error,
                        code: Code::ReelIncoherent,
                        message: format!(
                            "Reel {} {label} '{value}' is not coherent with earlier reels ('{first}')",
                            i + 1
                        ),
                        file: Some(cpl_path.to_path_buf()),
                        line: 0,
                    });
                    flagged.insert(label);
                }
                _ => {}
            }
        }
    }

    notes
}

/// First matching essence block in a reel (handles the stereoscopic alias).
fn essence_block<'a>(reel: &'a str, tags: &[&str]) -> Option<&'a str> {
    for tag in tags {
        let re = regex_lite::Regex::new(&format!(r"<{tag}>([\s\S]*?)</{tag}>")).unwrap();
        if let Some(cap) = re.captures(reel) {
            return Some(cap.get(1).unwrap().as_str());
        }
    }
    None
}

/// ClairMeta keys per-asset encryption off `<KeyId>` presence in the CPL.
fn encrypted_str(block: &str) -> String {
    if block.contains("<KeyId>") {
        "encrypted".into()
    } else {
        "clear".into()
    }
}

// ─── Stereoscopic 3D ──────────────────────────────────────────────────────────

/// Check stereoscopic reels for left/right eye and duration consistency.
pub fn check_stereo(cpl_path: &Path) -> Vec<Note> {
    let mut notes = Vec::new();

    let Ok(content) = std::fs::read_to_string(cpl_path) else {
        return notes;
    };

    let stereo_re =
        regex_lite::Regex::new(r"<MainStereoscopicPicture>([\s\S]*?)</MainStereoscopicPicture>")
            .unwrap();
    let stereo_reels: Vec<&str> = stereo_re
        .captures_iter(&content)
        .map(|c| c.get(1).unwrap().as_str())
        .collect();

    if stereo_reels.is_empty() {
        return notes; // Not a 3D DCP
    }

    for (i, block) in stereo_reels.iter().enumerate() {
        // Interop: check for LeftEye / RightEye sub-elements
        let has_left = block.contains("<LeftEye>");
        let has_right = block.contains("<RightEye>");

        if has_left || has_right {
            if !has_left {
                notes.push(Note {
                    severity: Severity::Error,
                    code: Code::StereoMismatch,
                    message: format!("Stereoscopic reel {} missing LeftEye", i + 1),
                    file: Some(cpl_path.to_path_buf()),
                    line: 0,
                });
            }
            if !has_right {
                notes.push(Note {
                    severity: Severity::Error,
                    code: Code::StereoMismatch,
                    message: format!("Stereoscopic reel {} missing RightEye", i + 1),
                    file: Some(cpl_path.to_path_buf()),
                    line: 0,
                });
            }
        }

        // Check Duration <= IntrinsicDuration
        let duration = extract_u64(block, "Duration");
        let intrinsic = extract_u64(block, "IntrinsicDuration");
        if let (Some(d), Some(intr)) = (duration, intrinsic)
            && d > intr
        {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::StereoMismatch,
                message: format!(
                    "Stereoscopic reel {} Duration exceeds IntrinsicDuration",
                    i + 1
                ),
                file: Some(cpl_path.to_path_buf()),
                line: 0,
            });
        }
    }

    notes
}

// ─── Marker Validation ────────────────────────────────────────────────────────

/// Validate markers (FFMC, LFMC, etc.) are present and well-formed.
pub fn check_markers(cpl_path: &Path, strict: bool) -> Vec<Note> {
    let mut notes = Vec::new();

    let Ok(content) = std::fs::read_to_string(cpl_path) else {
        return notes;
    };

    // Find MainMarkers / MarkerAsset blocks
    let markers_re = regex_lite::Regex::new(
        r"<(?:MainMarkers|MarkerAsset)>([\s\S]*?)</(?:MainMarkers|MarkerAsset)>",
    )
    .unwrap();
    let marker_blocks: Vec<&str> = markers_re
        .captures_iter(&content)
        .map(|c| c.get(1).unwrap().as_str())
        .collect();

    if marker_blocks.is_empty() {
        return notes;
    }

    let mut found_markers = HashSet::new();

    let marker_re = regex_lite::Regex::new(r"<Marker>([\s\S]*?)</Marker>").unwrap();
    for block in &marker_blocks {
        for cap in marker_re.captures_iter(block) {
            let m_block = cap.get(1).unwrap().as_str();
            let label = extract_tag(m_block, "Label").unwrap_or_default();
            if !label.is_empty() {
                found_markers.insert(label.clone());
            }

            // Check Offset is present
            if extract_tag(m_block, "Offset").is_none() {
                notes.push(Note {
                    severity: Severity::Warning,
                    code: Code::MarkerInvalid,
                    message: format!("Marker '{label}' missing Offset"),
                    file: Some(cpl_path.to_path_buf()),
                    line: 0,
                });
            }
        }
    }

    if strict {
        // Required markers (SMPTE 429-7)
        let required = [
            ("FFMC", "First Frame of Moving Content"),
            ("LFMC", "Last Frame of Moving Content"),
        ];
        for (label, desc) in required {
            if !found_markers.contains(label) {
                notes.push(Note {
                    severity: Severity::Warning,
                    code: Code::MarkerMissing,
                    message: format!("Required marker missing: {label} ({desc})"),
                    file: Some(cpl_path.to_path_buf()),
                    line: 0,
                });
            }
        }

        // Recommended markers
        let recommended = [
            ("FFTC", "First Frame of Title Credits"),
            ("LFTC", "Last Frame of Title Credits"),
            ("FFOI", "First Frame of Intermission"),
            ("LFOI", "Last Frame of Intermission"),
            ("FFEC", "First Frame of End Credits"),
            ("LFEC", "Last Frame of End Credits"),
        ];
        for (label, desc) in recommended {
            if !found_markers.contains(label) {
                notes.push(Note {
                    severity: Severity::Info,
                    code: Code::MarkerMissing,
                    message: format!("Recommended marker not present: {label} ({desc})"),
                    file: Some(cpl_path.to_path_buf()),
                    line: 0,
                });
            }
        }
    }

    notes
}

// ─── Cross-Reference Integrity ────────────────────────────────────────────────

/// Verify that asset IDs referenced in CPLs exist in the known set (ASSETMAP/PKL).
///
/// OV-aware: when `ov_asset_ids` is `Some`, a reference that resolves in the OV
/// package passes and a reference in neither package is a real break. When no OV
/// is supplied, an unresolved ref means the package is a version file (VF)
/// referencing an external OV: this matches ClairMeta, which classifies any CPL
/// asset missing locally as VF. Since a legitimate VF ref and a corrupt one are
/// indistinguishable without the OV, unresolved refs are reported once as
/// [`Code::SupplementalOvNotProvided`] (a warning), not a hard error. Supply
/// `--ov` to turn genuinely broken refs back into errors.
pub fn check_cross_references(
    known_asset_ids: &[String],
    ov_asset_ids: Option<&HashSet<String>>,
    cpl_paths: &[PathBuf],
) -> Vec<Note> {
    use dcpdoctor_imf::{RefStatus, resolve_track_ref};

    let mut notes = Vec::new();

    let known_ids: HashSet<String> = known_asset_ids
        .iter()
        .map(|id| id.strip_prefix("urn:uuid:").unwrap_or(id).to_string())
        .collect();
    let empty = HashSet::new();
    let ov_ids = ov_asset_ids.unwrap_or(&empty);
    let ov_provided = ov_asset_ids.is_some();

    let id_re = regex_lite::Regex::new(r"<Id>(urn:uuid:[^<]+)</Id>").unwrap();

    let mut needs_ov = 0usize;
    for cpl_path in cpl_paths {
        let Ok(content) = std::fs::read_to_string(cpl_path) else {
            continue;
        };

        // Only check IDs within blocks that reference external asset files.
        // MainMarkers is an inline marker track (no file in the ASSETMAP), so it
        // is deliberately excluded to avoid false CrossRefBroken errors.
        let asset_block_re = regex_lite::Regex::new(
            r"<(?:MainPicture|MainSound|MainSubtitle|MainStereoscopicPicture|ClosedCaption|MainImage|AuxData)>([\s\S]*?)</(?:MainPicture|MainSound|MainSubtitle|MainStereoscopicPicture|ClosedCaption|MainImage|AuxData)>",
        ).unwrap();

        for block_cap in asset_block_re.captures_iter(&content) {
            let block = block_cap.get(1).unwrap().as_str();
            for id_cap in id_re.captures_iter(block) {
                let raw_id = &id_cap[1];
                let normalized = raw_id
                    .strip_prefix("urn:uuid:")
                    .unwrap_or(raw_id)
                    .to_string();
                match resolve_track_ref(&normalized, &known_ids, ov_ids, ov_provided) {
                    RefStatus::Local | RefStatus::Ov => {}
                    RefStatus::BrokenWithOv => notes.push(Note {
                        severity: Severity::Error,
                        code: Code::CrossRefBroken,
                        message: format!(
                            "CPL references asset {normalized} not found in this package or the OV"
                        ),
                        file: Some(cpl_path.clone()),
                        line: 0,
                    }),
                    RefStatus::UnresolvedNoOv => needs_ov += 1,
                }
            }
        }
    }

    if needs_ov > 0 {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::SupplementalOvNotProvided,
            message: format!(
                "Supplemental DCP references {needs_ov} asset(s) not in this package; supply the OV with --ov to fully validate"
            ),
            file: None,
            line: 0,
        });
    }

    notes
}

// ─── Supplemental DCP Detection ───────────────────────────────────────────────

/// Whether a CPL's XML marks it as a supplemental/version-file package.
fn cpl_is_supplemental(content: &str) -> bool {
    content.contains("<OPL>")
        || content.contains("<OriginalPackagingList")
        || content.contains("<OriginalFileName")
}

/// Detect if CPLs are supplemental/version-file packages (OPL references).
pub fn check_supplemental(cpl_paths: &[PathBuf]) -> Vec<Note> {
    let mut notes = Vec::new();

    for cpl_path in cpl_paths {
        let Ok(content) = std::fs::read_to_string(cpl_path) else {
            continue;
        };

        if cpl_is_supplemental(&content) {
            notes.push(Note {
                severity: Severity::Info,
                code: Code::SupplementalOplMissing,
                message: "CPL appears to be a supplemental/version file package".into(),
                file: Some(cpl_path.clone()),
                line: 0,
            });
        }
    }

    notes
}

// ─── Audio Channel Labeling ───────────────────────────────────────────────────

/// Warn if sound reels lack MCA channel labeling metadata.
pub fn check_audio_channels(cpl_path: &Path) -> Vec<Note> {
    let mut notes = Vec::new();

    let Ok(content) = std::fs::read_to_string(cpl_path) else {
        return notes;
    };

    let sound_re =
        regex_lite::Regex::new(r"<(?:MainSound|MainAudio)>([\s\S]*?)</(?:MainSound|MainAudio)>")
            .unwrap();

    for (i, cap) in sound_re.captures_iter(&content).enumerate() {
        let block = cap.get(1).unwrap().as_str();
        let has_mca = block.contains("MCALabelDictionaryId");
        let has_sfg = block.contains("SoundFieldGroupLinkId");

        if !has_mca && !has_sfg {
            notes.push(Note {
                severity: Severity::Info,
                code: Code::SoundInvalidChannelCount,
                message: format!("Reel {} sound has no MCA channel labeling metadata", i + 1),
                file: Some(cpl_path.to_path_buf()),
                line: 0,
            });
        }
    }

    notes
}

// ─── Color Space ──────────────────────────────────────────────────────────────

/// Check CPL color metadata for DCI XYZ compliance.
pub fn check_color_space(cpl_path: &Path) -> Vec<Note> {
    let mut notes = Vec::new();

    let Ok(content) = std::fs::read_to_string(cpl_path) else {
        return notes;
    };

    // Look for TransferCharacteristic elements
    let tc_re = regex_lite::Regex::new(r"<TransferCharacteristic>([^<]+)</TransferCharacteristic>")
        .unwrap();

    for cap in tc_re.captures_iter(&content) {
        let val = cap[1].trim();
        // DCI XYZ uses gamma 2.6
        if !val.contains("2.6") && !val.to_uppercase().contains("XYZ") {
            notes.push(Note {
                severity: Severity::Info,
                code: Code::PictureInvalidResolution,
                message: format!("TransferCharacteristic indicates non-DCI color: {val}"),
                file: Some(cpl_path.to_path_buf()),
                line: 0,
            });
        }
    }

    notes
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn extract_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].trim().to_string())
}

fn extract_u64(xml: &str, tag: &str) -> Option<u64> {
    extract_tag(xml, tag)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // two-reel CPL; {enc0}/{enc1} inject a picture KeyId, {fr0}/{fr1} the frame rate
    fn two_reel_cpl(enc0: &str, enc1: &str, fr0: &str, fr1: &str) -> String {
        let reel = |enc: &str, fr: &str| {
            format!(
                r#"<Reel><Id>urn:uuid:{id}</Id><AssetList>
  <MainPicture>
    <Id>urn:uuid:{id}</Id>
    <EditRate>24 1</EditRate>
    <FrameRate>{fr}</FrameRate>
    <ScreenAspectRatio>2048 858</ScreenAspectRatio>
    {enc}
  </MainPicture>
  <MainSound>
    <Id>urn:uuid:{id}</Id>
    <EditRate>24 1</EditRate>
  </MainSound>
</AssetList></Reel>"#,
                id = "00000000-0000-0000-0000-000000000000",
                enc = enc,
                fr = fr,
            )
        };
        format!(
            "<CompositionPlaylist>{}{}</CompositionPlaylist>",
            reel(enc0, fr0),
            reel(enc1, fr1),
        )
    }

    fn write_cpl(xml: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(xml.as_bytes()).unwrap();
        f
    }

    #[test]
    fn coherent_reels_pass() {
        let xml = two_reel_cpl("<KeyId>k</KeyId>", "<KeyId>k</KeyId>", "24 1", "24 1");
        let f = write_cpl(&xml);
        assert!(check_reel_coherence(f.path()).is_empty());
    }

    #[test]
    fn mixed_encryption_flags_incoherent() {
        // one encrypted picture reel, one clear: this is exactly ECL32
        let xml = two_reel_cpl("<KeyId>k</KeyId>", "", "24 1", "24 1");
        let f = write_cpl(&xml);
        let notes = check_reel_coherence(f.path());
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].code, Code::ReelIncoherent);
        assert_eq!(notes[0].severity, Severity::Error);
    }

    #[test]
    fn mixed_frame_rate_flags_incoherent() {
        let xml = two_reel_cpl("<KeyId>k</KeyId>", "<KeyId>k</KeyId>", "24 1", "48 1");
        let f = write_cpl(&xml);
        let notes = check_reel_coherence(f.path());
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].code, Code::ReelIncoherent);
    }
}
