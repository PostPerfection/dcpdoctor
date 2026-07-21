//! Individual DCP validators — encryption, reel continuity, stereo, markers,
//! cross-references, supplemental detection, audio channels.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::{Code, Note, Severity, Standard};

// ─── Native MXF essence probes (asdcplib) ─────────────────────────────────────
// These read only header metadata and are used to confirm CPL-declared essence
// against the real MXF. They return None when the file is absent or unreadable,
// so callers fall back to the CPL-only (XML) check.

/// First `<Id>urn:uuid:...</Id>` in an essence block, resolved to its MXF path.
fn asset_file(block: &str, id_to_file: &HashMap<String, PathBuf>) -> Option<PathBuf> {
    let id_re = regex_lite::Regex::new(r"<Id>urn:uuid:([0-9a-fA-F-]+)</Id>").unwrap();
    let id = id_re.captures(block)?.get(1)?.as_str().to_lowercase();
    id_to_file.get(&id).cloned()
}

/// Probe a PCM MXF for ST 429-12 MCA label subdescriptors. `Some(true)` when the
/// header carries channel/soundfield labels, `Some(false)` when it reads as PCM
/// with none, `None` when the essence can't be read.
fn probe_sound_has_mca(path: &Path) -> Option<bool> {
    let path_str = path.to_str()?;
    let mut reader = asdcplib::pcm::MxfReader::new();
    reader.open_read(path_str).ok()?;
    let mca = reader.mca_labels().ok()?;
    Some(mca.channel_labels > 0 || mca.soundfield_groups > 0 || mca.has_mca_channel_assignment)
}

/// Probe a picture MXF essence type. `Some(true)` for stereoscopic J2K,
/// `Some(false)` for mono J2K, `None` when it can't be identified.
fn probe_picture_is_stereo(path: &Path) -> Option<bool> {
    let path_str = path.to_str()?;
    match asdcplib::essence_type(path_str).ok()? {
        asdcplib::EssenceType::Jpeg2000Stereo => Some(true),
        asdcplib::EssenceType::Jpeg2000 | asdcplib::EssenceType::As02Jpeg2000 => Some(false),
        _ => None,
    }
}

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

/// First matching essence block in a reel (handles the stereoscopic alias and
/// the msp-cpl/axd namespaced forms).
fn essence_block<'a>(reel: &'a str, tags: &[&str]) -> Option<&'a str> {
    for tag in tags {
        let re = regex_lite::Regex::new(&format!(
            r"<(?:[\w-]+:)?{tag}(?:\s[^>]*)?>([\s\S]*?)</(?:[\w-]+:)?{tag}>"
        ))
        .unwrap();
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

/// Check stereoscopic (ST 429-10) reels: eye consistency, duration, the doubled
/// FrameRate = 2x EditRate relationship, and that the essence is really a
/// stereoscopic J2K track where the MXF is available.
pub fn check_stereo(cpl_path: &Path, id_to_file: &HashMap<String, PathBuf>) -> Vec<Note> {
    let mut notes = Vec::new();

    let Ok(content) = std::fs::read_to_string(cpl_path) else {
        return notes;
    };

    // tolerate the msp-cpl namespaced form real 3D DCPs emit
    let stereo_re = regex_lite::Regex::new(
        r"<(?:[\w-]+:)?MainStereoscopicPicture(?:\s[^>]*)?>([\s\S]*?)</(?:[\w-]+:)?MainStereoscopicPicture>",
    )
    .unwrap();
    let stereo_reels: Vec<&str> = stereo_re
        .captures_iter(&content)
        .map(|c| c.get(1).unwrap().as_str())
        .collect();

    if stereo_reels.is_empty() {
        return notes; // Not a 3D DCP
    }

    let stereo_err = |msg: String| Note {
        severity: Severity::Error,
        code: Code::StereoMismatch,
        message: msg,
        file: Some(cpl_path.to_path_buf()),
        line: 0,
    };

    for (i, block) in stereo_reels.iter().enumerate() {
        // Interop: check for LeftEye / RightEye sub-elements
        let has_left = block.contains("<LeftEye>");
        let has_right = block.contains("<RightEye>");

        if has_left || has_right {
            if !has_left {
                notes.push(stereo_err(format!("Stereoscopic reel {} missing LeftEye", i + 1)));
            }
            if !has_right {
                notes.push(stereo_err(format!("Stereoscopic reel {} missing RightEye", i + 1)));
            }
        }

        // Check Duration <= IntrinsicDuration
        let duration = extract_u64(block, "Duration");
        let intrinsic = extract_u64(block, "IntrinsicDuration");
        if let (Some(d), Some(intr)) = (duration, intrinsic)
            && d > intr
        {
            notes.push(stereo_err(format!(
                "Stereoscopic reel {} Duration exceeds IntrinsicDuration",
                i + 1
            )));
        }

        // ST 429-10: the interleaved L/R essence carries two frames per edit
        // unit, so FrameRate must be exactly twice EditRate.
        if let (Some((er_n, er_d)), Some((fr_n, fr_d))) =
            (extract_rate(block, "EditRate"), extract_rate(block, "FrameRate"))
            && !(fr_n == er_n * 2 && fr_d == er_d)
        {
            notes.push(stereo_err(format!(
                "Stereoscopic reel {} FrameRate {fr_n} {fr_d} is not twice EditRate {er_n} {er_d}",
                i + 1
            )));
        }

        // Confirm the referenced essence is a stereoscopic J2K MXF when present.
        if let Some(path) = asset_file(block, id_to_file)
            && probe_picture_is_stereo(&path) == Some(false)
        {
            notes.push(stereo_err(format!(
                "Stereoscopic reel {} references a non-stereoscopic J2K essence",
                i + 1
            )));
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
        // is deliberately excluded to avoid false CrossRefBroken errors. The
        // optional prefix/attributes match the msp-cpl (429-10 stereo) and axd
        // (429-18 aux data) namespaced forms real DCPs emit.
        let asset_block_re = regex_lite::Regex::new(
            r"<(?:[\w-]+:)?(?:MainPicture|MainSound|MainSubtitle|MainStereoscopicPicture|ClosedCaption|MainImage|AuxData)(?:\s[^>]*)?>([\s\S]*?)</(?:[\w-]+:)?(?:MainPicture|MainSound|MainSubtitle|MainStereoscopicPicture|ClosedCaption|MainImage|AuxData)>",
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

/// Warn if sound reels lack MCA (ST 429-12) channel labeling. The labels live in
/// the sound MXF header, not the CPL, so when the essence is available its MCA
/// subdescriptors are read directly; only for XML-only validation (no readable
/// MXF) does this fall back to grepping the CPL for the label markers.
pub fn check_audio_channels(cpl_path: &Path, id_to_file: &HashMap<String, PathBuf>) -> Vec<Note> {
    let mut notes = Vec::new();

    let Ok(content) = std::fs::read_to_string(cpl_path) else {
        return notes;
    };

    let sound_re = regex_lite::Regex::new(
        r"<(?:[\w-]+:)?(?:MainSound|MainAudio)(?:\s[^>]*)?>([\s\S]*?)</(?:[\w-]+:)?(?:MainSound|MainAudio)>",
    )
    .unwrap();

    for (i, cap) in sound_re.captures_iter(&content).enumerate() {
        let block = cap.get(1).unwrap().as_str();

        // prefer the real MXF subdescriptors; fall back to the CPL markers only
        // when the essence is missing or unreadable
        let has_labels = match asset_file(block, id_to_file).and_then(|p| probe_sound_has_mca(&p)) {
            Some(present) => present,
            None => {
                block.contains("MCALabelDictionaryId") || block.contains("SoundFieldGroupLinkId")
            }
        };

        if !has_labels {
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

// ─── Auxiliary Data (ST 429-18) ───────────────────────────────────────────────

/// Surface an INFO for each ST 429-18 auxiliary-data track (e.g. Dolby Atmos
/// IAB). The AuxData element carries a DataType UL; where the MXF is available
/// its essence type enriches the note. Also checks the aux duration against the
/// reel's picture duration (ClairMeta `check_cpl_reel_duration_picture_aux`).
/// Cross-refs and PKL hashes for the aux asset are covered by the generic
/// asset checks.
pub fn check_aux_data(cpl_path: &Path, id_to_file: &HashMap<String, PathBuf>) -> Vec<Note> {
    let mut notes = Vec::new();

    let Ok(content) = std::fs::read_to_string(cpl_path) else {
        return notes;
    };

    let reel_re = regex_lite::Regex::new(r"<Reel>([\s\S]*?)</Reel>").unwrap();
    let aux_re = regex_lite::Regex::new(
        r"<(?:[\w-]+:)?AuxData(?:\s[^>]*)?>([\s\S]*?)</(?:[\w-]+:)?AuxData>",
    )
    .unwrap();
    let pic_re = regex_lite::Regex::new(
        r"<(?:MainPicture|MainStereoscopicPicture|MainImage)(?:\s[^>]*)?>([\s\S]*?)</(?:MainPicture|MainStereoscopicPicture|MainImage)>",
    )
    .unwrap();

    for (i, reel_cap) in reel_re.captures_iter(&content).enumerate() {
        let reel = reel_cap.get(1).unwrap().as_str();
        let Some(cap) = aux_re.captures(reel) else {
            continue;
        };
        let block = cap.get(1).unwrap().as_str();
        let is_atmos = block.contains("0e090604") // Dolby Atmos IAB data-essence UL
            || content.contains("dolby.com/schemas/2012/AD");
        let kind = if is_atmos {
            "Dolby Atmos / IAB"
        } else {
            "data-essence"
        };

        let mut msg = format!("Reel {} carries a ST 429-18 auxiliary-data track ({kind})", i + 1);
        if let Some(path) = asset_file(block, id_to_file)
            && let Some(s) = path.to_str()
            && let Ok(etype) = asdcplib::essence_type(s)
        {
            msg.push_str(&format!("; essence {etype:?}"));
        }

        notes.push(Note::info(Code::AuxDataDetected, msg).with_file(cpl_path));

        let aux_dur = extract_u64(block, "Duration");
        let pic_dur = pic_re
            .captures(reel)
            .and_then(|c| extract_u64(c.get(1).unwrap().as_str(), "Duration"));
        if let (Some(a), Some(p)) = (aux_dur, pic_dur)
            && a != p
        {
            notes.push(
                Note::warning(
                    Code::CplMismatchedDurations,
                    format!(
                        "Reel {} aux-data duration {a} differs from picture duration {p}",
                        i + 1
                    ),
                )
                .with_file(cpl_path),
            );
        }
    }

    notes
}

// ─── CPL Label / Metadata ─────────────────────────────────────────────────────

/// Check a CPL for the identifying metadata every DCI CPL carries (ClairMeta
/// `check_cpl_metadata`). ContentVersion is SMPTE-only (Interop CPLs have none),
/// so it is scoped to that standard to avoid false positives on Interop.
pub fn check_cpl_metadata(cpl_path: &Path, standard: Standard) -> Vec<Note> {
    let mut notes = Vec::new();

    let Ok(content) = std::fs::read_to_string(cpl_path) else {
        return notes;
    };
    if !content.contains("CompositionPlaylist") {
        return notes;
    }

    let has = |tag: &str| extract_tag(&content, tag).is_some_and(|v| !v.trim().is_empty());

    if !has("ContentTitleText") {
        notes.push(
            Note::error(Code::MissingRequiredElement, "CPL missing ContentTitleText")
                .with_file(cpl_path),
        );
    }
    if !has("IssueDate") {
        notes.push(
            Note::warning(Code::MissingRequiredElement, "CPL missing IssueDate").with_file(cpl_path),
        );
    }
    if standard == Standard::Smpte && !has("ContentVersion") {
        notes.push(
            Note::info(
                Code::MissingRequiredElement,
                "SMPTE CPL missing ContentVersion metadata",
            )
            .with_file(cpl_path),
        );
    }

    notes
}

// ─── Package File Hygiene ─────────────────────────────────────────────────────

/// Flag package-directory files that are neither referenced by the ASSETMAP nor
/// a standard package descriptor (ASSETMAP/VOLINDEX), plus any zero-byte file.
/// Mirrors ClairMeta's foreign-file and empty-file hygiene checks.
pub fn check_package_files(dcp_dir: &Path, referenced_paths: &[String]) -> Vec<Note> {
    let mut notes = Vec::new();
    let referenced: HashSet<&str> = referenced_paths.iter().map(String::as_str).collect();

    let Ok(entries) = std::fs::read_dir(dcp_dir) else {
        return notes;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();

        // zero-byte files are always a hard error, descriptor or not
        if entry.metadata().map(|m| m.len()).unwrap_or(1) == 0 {
            notes.push(
                Note::error(
                    Code::EmptyFileInPackage,
                    format!("Zero-byte file in package: {name}"),
                )
                .with_file(&path),
            );
            continue;
        }

        // ASSETMAP/VOLINDEX are legitimate but never listed in the ASSETMAP
        let is_descriptor = matches!(
            name.to_ascii_uppercase().as_str(),
            "ASSETMAP" | "ASSETMAP.XML" | "VOLINDEX" | "VOLINDEX.XML"
        );
        if is_descriptor {
            continue;
        }

        if !referenced.contains(name.as_str()) {
            notes.push(
                Note::warning(
                    Code::ForeignFileInPackage,
                    format!("File in package not referenced by the ASSETMAP: {name}"),
                )
                .with_file(&path),
            );
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

/// Parse a SMPTE rational tag like `<EditRate>24 1</EditRate>` into (num, den).
fn extract_rate(xml: &str, tag: &str) -> Option<(u64, u64)> {
    let v = extract_tag(xml, tag)?;
    let mut it = v.split_whitespace();
    let n = it.next()?.parse().ok()?;
    let d = it.next()?.parse().ok()?;
    Some((n, d))
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

    // stereo (429-10) reel in the msp-cpl prefixed form dcpwizard emits
    fn stereo_cpl(edit_rate: &str, frame_rate: &str) -> String {
        format!(
            r#"<CompositionPlaylist><Reel><Id>urn:uuid:00000000-0000-0000-0000-0000000000f0</Id><AssetList>
  <msp-cpl:MainStereoscopicPicture xmlns:msp-cpl="http://www.smpte-ra.org/schemas/429-10/2008/Main-Stereo-Picture-CPL">
    <Id>urn:uuid:00000000-0000-0000-0000-000000000001</Id>
    <EditRate>{edit_rate}</EditRate>
    <IntrinsicDuration>48</IntrinsicDuration>
    <Duration>48</Duration>
    <FrameRate>{frame_rate}</FrameRate>
  </msp-cpl:MainStereoscopicPicture>
</AssetList></Reel></CompositionPlaylist>"#
        )
    }

    #[test]
    fn stereo_prefixed_form_with_doubled_framerate_passes() {
        let f = write_cpl(&stereo_cpl("24 1", "48 1"));
        // no id_to_file entry so essence probing is skipped (xml-only)
        assert!(check_stereo(f.path(), &HashMap::new()).is_empty());
    }

    #[test]
    fn stereo_framerate_not_doubled_flags_mismatch() {
        let f = write_cpl(&stereo_cpl("24 1", "24 1"));
        let notes = check_stereo(f.path(), &HashMap::new());
        assert!(
            notes.iter().any(|n| n.code == Code::StereoMismatch
                && n.message.contains("not twice EditRate")),
            "got: {notes:?}"
        );
    }

    #[test]
    fn aux_data_axd_form_surfaces_atmos_info() {
        let cpl = r#"<CompositionPlaylist><Reel><Id>urn:uuid:00000000-0000-0000-0000-0000000000f0</Id><AssetList>
  <axd:AuxData xmlns:axd="http://www.dolby.com/schemas/2012/AD">
    <Id>urn:uuid:00000000-0000-0000-0000-0000000000a1</Id>
    <EditRate>24 1</EditRate>
    <Duration>48</Duration>
    <axd:DataType>urn:smpte:ul:060e2b34.04010105.0e090604.00000000</axd:DataType>
  </axd:AuxData>
</AssetList></Reel></CompositionPlaylist>"#;
        let f = write_cpl(cpl);
        let notes = check_aux_data(f.path(), &HashMap::new());
        assert!(
            notes.iter().any(|n| n.code == Code::AuxDataDetected
                && n.severity == Severity::Info
                && n.message.contains("Atmos")),
            "got: {notes:?}"
        );
    }

    #[test]
    fn aux_data_duration_mismatch_warns() {
        let cpl = r#"<CompositionPlaylist><Reel><Id>urn:uuid:00000000-0000-0000-0000-0000000000f0</Id><AssetList>
  <MainPicture><Id>urn:uuid:00000000-0000-0000-0000-0000000000b1</Id><Duration>48</Duration></MainPicture>
  <axd:AuxData xmlns:axd="http://www.dolby.com/schemas/2012/AD">
    <Id>urn:uuid:00000000-0000-0000-0000-0000000000a1</Id>
    <Duration>1</Duration>
    <axd:DataType>urn:smpte:ul:060e2b34.04010105.0e090604.00000000</axd:DataType>
  </axd:AuxData>
</AssetList></Reel></CompositionPlaylist>"#;
        let f = write_cpl(cpl);
        let notes = check_aux_data(f.path(), &HashMap::new());
        assert!(
            notes.iter().any(|n| n.code == Code::CplMismatchedDurations
                && n.severity == Severity::Warning),
            "got: {notes:?}"
        );
        // matching durations stay clean
        let ok = write_cpl(&cpl.replace("<Duration>1</Duration>", "<Duration>48</Duration>"));
        assert!(
            !check_aux_data(ok.path(), &HashMap::new())
                .iter()
                .any(|n| n.code == Code::CplMismatchedDurations)
        );
    }

    #[test]
    fn audio_channels_xml_only_falls_back_to_cpl_markers() {
        // no MXF available, so the CPL markers decide: labeled clears, bare fires
        let bare = write_cpl(
            r#"<CompositionPlaylist><MainSound><Id>urn:uuid:00000000-0000-0000-0000-000000000002</Id></MainSound></CompositionPlaylist>"#,
        );
        assert_eq!(check_audio_channels(bare.path(), &HashMap::new()).len(), 1);

        let labeled = write_cpl(
            r#"<CompositionPlaylist><MainSound><Id>urn:uuid:00000000-0000-0000-0000-000000000002</Id><MCALabelDictionaryId>urn:smpte:ul:x</MCALabelDictionaryId></MainSound></CompositionPlaylist>"#,
        );
        assert!(check_audio_channels(labeled.path(), &HashMap::new()).is_empty());
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

    #[test]
    fn cpl_missing_content_title_is_flagged() {
        let f = write_cpl(
            r#"<CompositionPlaylist><Id>x</Id><IssueDate>2020</IssueDate><ContentVersion>1</ContentVersion></CompositionPlaylist>"#,
        );
        let notes = check_cpl_metadata(f.path(), Standard::Smpte);
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::MissingRequiredElement
                    && n.message.contains("ContentTitleText")),
            "got: {notes:?}"
        );
    }

    #[test]
    fn full_smpte_cpl_metadata_is_clean() {
        let f = write_cpl(
            r#"<CompositionPlaylist><Id>x</Id><ContentTitleText>Movie</ContentTitleText><IssueDate>2020</IssueDate><ContentVersion>1</ContentVersion></CompositionPlaylist>"#,
        );
        assert!(check_cpl_metadata(f.path(), Standard::Smpte).is_empty());
    }

    #[test]
    fn interop_cpl_without_content_version_is_not_flagged() {
        // Interop CPLs have no ContentVersion; the SMPTE-only check must not fire.
        let f = write_cpl(
            r#"<CompositionPlaylist><Id>x</Id><ContentTitleText>Movie</ContentTitleText><IssueDate>2020</IssueDate></CompositionPlaylist>"#,
        );
        let notes = check_cpl_metadata(f.path(), Standard::Interop);
        assert!(!notes.iter().any(|n| n.message.contains("ContentVersion")));
    }

    #[test]
    fn foreign_and_empty_files_are_flagged_referenced_are_not() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ASSETMAP.xml"), "x").unwrap();
        std::fs::write(dir.path().join("good.mxf"), "data").unwrap();
        std::fs::write(dir.path().join("stray.txt"), "junk").unwrap();
        std::fs::write(dir.path().join("empty.mxf"), "").unwrap();

        let notes = check_package_files(dir.path(), &["good.mxf".to_string()]);

        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::ForeignFileInPackage && n.message.contains("stray.txt")),
            "unreferenced file must be flagged, got: {notes:?}"
        );
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::EmptyFileInPackage && n.message.contains("empty.mxf")),
            "zero-byte file must be flagged, got: {notes:?}"
        );
        assert!(
            !notes.iter().any(|n| n.message.contains("good.mxf")),
            "referenced file must not be flagged"
        );
        assert!(
            !notes.iter().any(|n| n.message.ends_with(": ASSETMAP.xml")),
            "package descriptor must not be flagged"
        );
    }
}
