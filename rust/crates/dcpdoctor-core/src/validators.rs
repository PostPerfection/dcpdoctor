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

/// True if an XML document carries an enveloped signature (same presence test
/// signature verification uses).
fn has_signature(content: &str) -> bool {
    content.contains("<Signature") || content.contains("<ds:Signature")
}

/// True if any CPL declares encrypted essence (a KeyId or an embedded
/// EncryptedDocumentKey), i.e. the package needs a KDM.
fn package_is_encrypted(cpl_paths: &[PathBuf]) -> bool {
    cpl_paths.iter().any(|p| {
        std::fs::read_to_string(p)
            .map(|c| c.contains("<KeyId>") || c.contains("<EncryptedDocumentKey"))
            .unwrap_or(false)
    })
}

/// ClairMeta `check_dcp_signed`: an encrypted DCP must carry a signed CPL and
/// PKL. The KDM only delivers content keys, so an unsigned CPL/PKL leaves the
/// package's authenticity unverifiable (SMPTE ST 429-7/-8 require the signature
/// on encrypted packages). Silent on unencrypted packages, where signing is
/// optional. Closes the gap where a KDM-present-but-unsigned package slipped
/// through only as `kdm_required`.
pub fn check_dcp_signed(cpl_paths: &[PathBuf], pkl_paths: &[PathBuf]) -> Vec<Note> {
    let mut notes = Vec::new();
    if !package_is_encrypted(cpl_paths) {
        return notes;
    }

    let mut check = |path: &PathBuf, kind: &str| {
        if let Ok(content) = std::fs::read_to_string(path)
            && !has_signature(&content)
        {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::DcpNotSigned,
                message: format!(
                    "encrypted DCP has an unsigned {kind}; encrypted packages must have a signed CPL and PKL"
                ),
                file: Some(path.clone()),
                line: 0,
            });
        }
    };
    for cpl in cpl_paths {
        check(cpl, "CPL");
    }
    for pkl in pkl_paths {
        check(pkl, "PKL");
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
                notes.push(stereo_err(format!(
                    "Stereoscopic reel {} missing LeftEye",
                    i + 1
                )));
            }
            if !has_right {
                notes.push(stereo_err(format!(
                    "Stereoscopic reel {} missing RightEye",
                    i + 1
                )));
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
        if let (Some((er_n, er_d)), Some((fr_n, fr_d))) = (
            extract_rate(block, "EditRate"),
            extract_rate(block, "FrameRate"),
        ) && !(fr_n == er_n * 2 && fr_d == er_d)
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
        // Required markers (SMPTE 429-7; FFOC/LFOC required by Bv2.1)
        let required = [
            ("FFMC", "First Frame of Moving Content"),
            ("LFMC", "Last Frame of Moving Content"),
            ("FFOC", "First Frame of Composition"),
            ("LFOC", "Last Frame of Composition"),
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

    // FFOC/LFOC offset checks (SMPTE Bv2.1, matching libdcp/DCP-o-matic): FFOC in
    // the first reel must be at offset 1, LFOC in the last reel one less than that
    // reel's duration. The markers above are collected globally, so the per-reel
    // offset rule needs a separate reel scan.
    let reel_re = regex_lite::Regex::new(r"<Reel>([\s\S]*?)</Reel>").unwrap();
    let reels: Vec<&str> = reel_re
        .captures_iter(&content)
        .map(|c| c.get(1).unwrap().as_str())
        .collect();
    if let Some(first) = reels.first()
        && let Some(off) = marker_offset(first, "FFOC")
        && off != 1
    {
        notes.push(
            Note::warning(
                Code::MarkerInvalid,
                format!("The FFOC marker is {off} instead of 1"),
            )
            .with_file(cpl_path),
        );
    }
    if let Some(last) = reels.last()
        && let Some(off) = marker_offset(last, "LFOC")
        && let Some(dur) = reel_picture_duration(last)
        && off != dur.saturating_sub(1)
    {
        notes.push(
            Note::warning(
                Code::MarkerInvalid,
                format!(
                    "The LFOC marker is {off} instead of 1 less than the duration of the last reel"
                ),
            )
            .with_file(cpl_path),
        );
    }

    notes
}

/// Offset of the first Marker carrying `label` in a reel, if present.
fn marker_offset(reel: &str, label: &str) -> Option<u64> {
    let marker_re = regex_lite::Regex::new(r"<Marker>([\s\S]*?)</Marker>").unwrap();
    for cap in marker_re.captures_iter(reel) {
        let m = cap.get(1).unwrap().as_str();
        if extract_tag(m, "Label").as_deref() == Some(label) {
            return extract_u64(m, "Offset");
        }
    }
    None
}

/// Play duration of a reel's picture track.
fn reel_picture_duration(reel: &str) -> Option<u64> {
    let pic_re = regex_lite::Regex::new(
        r"<(?:[\w-]+:)?(?:MainPicture|MainImage|MainStereoscopicPicture)(?:\s[^>]*)?>([\s\S]*?)</(?:[\w-]+:)?(?:MainPicture|MainImage|MainStereoscopicPicture)>",
    )
    .unwrap();
    let block = pic_re.captures(reel)?.get(1)?.as_str();
    extract_u64(block, "Duration")
}

/// Picture-track edit rate of a reel, as (num, den).
fn reel_picture_edit_rate(reel: &str) -> Option<(u64, u64)> {
    let pic_re = regex_lite::Regex::new(
        r"<(?:[\w-]+:)?(?:MainPicture|MainImage|MainStereoscopicPicture)(?:\s[^>]*)?>([\s\S]*?)</(?:[\w-]+:)?(?:MainPicture|MainImage|MainStereoscopicPicture)>",
    )
    .unwrap();
    let block = pic_re.captures(reel)?.get(1)?.as_str();
    extract_rate(block, "EditRate")
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

        let mut msg = format!(
            "Reel {} carries a ST 429-18 auxiliary-data track ({kind})",
            i + 1
        );
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
            Note::warning(Code::MissingRequiredElement, "CPL missing IssueDate")
                .with_file(cpl_path),
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

// ─── MainSoundConfiguration (ST 429-16) ───────────────────────────────────────

/// MCA/ISDCF channel labels valid in a MainSoundConfiguration channel slot.
/// '-' marks an unused slot (ST 429-16).
const SOUND_CHANNEL_LABELS: &[&str] = &[
    "L", "R", "C", "LFE", "Ls", "Rs", "Lss", "Rss", "Lrs", "Rrs", "Lc", "Rc", "Cs", "Ts", "Lw",
    "Rw", "Lsd", "Rsd", "Lts", "Rts", "HI", "VI", "VIN", "DBOX", "FSK", "SLVS", "Sign", "-",
];

/// Validate the SMPTE ST 429-16 MainSoundConfiguration in a CPL's
/// CompositionMetadataAsset: presence, a well-formed `<soundfield>/<channels>`
/// value with recognized MCA/ISDCF labels, and a channel count matching the
/// referenced MainSound MXF (`actual_channels`). Interop CPLs carry no
/// CompositionMetadataAsset, so this is SMPTE-only. Mirrors DCP-o-matic's checks;
/// garbage like "None" (a real easyDCP output) is an error.
pub fn check_main_sound_configuration(
    cpl_path: &Path,
    standard: Standard,
    actual_channels: Option<u32>,
) -> Vec<Note> {
    let mut notes = Vec::new();
    if standard != Standard::Smpte {
        return notes;
    }
    let Ok(content) = std::fs::read_to_string(cpl_path) else {
        return notes;
    };

    // act only where CompositionMetadataAsset is present; its absence is a
    // separate Bv2.1 concern handled elsewhere
    let meta_re = regex_lite::Regex::new(
        r"<(?:[\w-]+:)?CompositionMetadataAsset(?:\s[^>]*)?>([\s\S]*?)</(?:[\w-]+:)?CompositionMetadataAsset>",
    )
    .unwrap();
    let Some(meta) = meta_re
        .captures(&content)
        .map(|c| c.get(1).unwrap().as_str())
    else {
        return notes;
    };

    let Some(cfg) = extract_ns_tag(meta, "MainSoundConfiguration").filter(|v| !v.is_empty()) else {
        notes.push(
            Note::warning(
                Code::MainSoundConfigInvalid,
                "SMPTE CPL has CompositionMetadataAsset but no MainSoundConfiguration",
            )
            .with_file(cpl_path),
        );
        return notes;
    };

    let msc_err = |msg: String| Note::error(Code::MainSoundConfigInvalid, msg).with_file(cpl_path);

    // format: "<soundfield>/<ch>,<ch>,..." e.g. "51/L,R,C,LFE,Ls,Rs"
    let Some((soundfield, channels)) = cfg.split_once('/') else {
        notes.push(msc_err(format!(
            "MainSoundConfiguration '{cfg}' is not in <soundfield>/<channels> form"
        )));
        return notes;
    };
    let channel_list: Vec<&str> = channels.split(',').map(str::trim).collect();
    if soundfield.trim().is_empty() || channel_list.iter().any(|c| c.is_empty()) {
        notes.push(msc_err(format!(
            "MainSoundConfiguration '{cfg}' has an empty soundfield or channel"
        )));
        return notes;
    }
    for ch in &channel_list {
        if !SOUND_CHANNEL_LABELS.contains(ch) {
            notes.push(msc_err(format!(
                "MainSoundConfiguration '{cfg}' has unrecognized channel label '{ch}'"
            )));
            return notes;
        }
    }

    let declared = channel_list.len();
    if let Some(actual) = actual_channels
        && actual as usize != declared
    {
        notes.push(
            Note::error(
                Code::SoundInvalidChannelCount,
                format!(
                    "MainSoundConfiguration has {declared} channels but sound assets have {actual}"
                ),
            )
            .with_file(cpl_path),
        );
    }

    notes
}

/// Channel count of a CPL's first MainSound MXF, read from the essence. Used to
/// feed [`check_main_sound_configuration`]'s cross-check. `None` when no readable
/// sound essence is available (XML-only validation).
pub fn first_sound_channel_count_of_cpl(
    cpl_path: &Path,
    id_to_file: &HashMap<String, PathBuf>,
) -> Option<u32> {
    let content = std::fs::read_to_string(cpl_path).ok()?;
    let sound_re = regex_lite::Regex::new(
        r"<(?:[\w-]+:)?MainSound(?:\s[^>]*)?>([\s\S]*?)</(?:[\w-]+:)?MainSound>",
    )
    .unwrap();
    let block = sound_re.captures(&content)?.get(1)?.as_str();
    let path = asset_file(block, id_to_file)?;
    let s = path.to_str()?;
    let mut reader = asdcplib::pcm::MxfReader::new();
    reader.open_read(s).ok()?;
    Some(reader.audio_descriptor().ok()?.channel_count)
}

// ─── First Subtitle Timing (Bv2.1) ────────────────────────────────────────────

/// Bv2.1: the first displayable timed-text event should start at least 4s after
/// the composition start. Only the first reel matters, and subtitle assets with
/// zero Subtitle instances (empty placeholders, common in encrypted SMPTE DCPs)
/// are ignored, avoiding DCP-o-matic bug #2757's false positive.
pub fn check_first_subtitle_timing(
    cpl_path: &Path,
    standard: Standard,
    id_to_file: &HashMap<String, PathBuf>,
) -> Vec<Note> {
    let mut notes = Vec::new();
    let Ok(content) = std::fs::read_to_string(cpl_path) else {
        return notes;
    };

    let reel_re = regex_lite::Regex::new(r"<Reel>([\s\S]*?)</Reel>").unwrap();
    let Some(first_reel) = reel_re
        .captures(&content)
        .map(|c| c.get(1).unwrap().as_str())
    else {
        return notes;
    };

    let sub_re = regex_lite::Regex::new(
        r"<(?:[\w-]+:)?MainSubtitle(?:\s[^>]*)?>([\s\S]*?)</(?:[\w-]+:)?MainSubtitle>",
    )
    .unwrap();
    let Some(sub) = sub_re
        .captures(first_reel)
        .map(|c| c.get(1).unwrap().as_str())
    else {
        return notes; // no subtitle in the first reel
    };

    let Some(xml) = subtitle_xml(sub, id_to_file) else {
        return notes; // essence missing/encrypted/unreadable, so can't tell
    };

    // reel edit rate converts frame-based times / the entry point to seconds
    let fps = extract_rate(sub, "EditRate")
        .or_else(|| reel_picture_edit_rate(first_reel))
        .map(|(n, d)| n as f64 / d.max(1) as f64)
        .unwrap_or(24.0);
    let entry = extract_u64(sub, "EntryPoint").unwrap_or(0) as f64 / fps;

    let time_re = regex_lite::Regex::new(r#"TimeIn\s*=\s*"([^"]*)""#).unwrap();
    let mut earliest: Option<f64> = None;
    for cap in time_re.captures_iter(&xml) {
        let Some(t) = subtitle_time_seconds(&cap[1], standard, fps) else {
            continue;
        };
        let shown = t - entry;
        if shown < 0.0 {
            continue; // before the reel entry point, not displayed here
        }
        earliest = Some(earliest.map_or(shown, |e| e.min(shown)));
    }

    // no parseable Subtitle events means an empty placeholder, nothing to flag
    let Some(first) = earliest else {
        return notes;
    };
    if first < 4.0 {
        notes.push(
            Note::warning(
                Code::SubtitleFirstEventEarly,
                format!(
                    "First subtitle appears {first:.1}s after composition start, less than the Bv2.1 minimum of 4s"
                ),
            )
            .with_file(cpl_path),
        );
    }

    notes
}

/// Subtitle XML for a reel's MainSubtitle: the plain-XML asset, or the MXF-wrapped
/// ST 428-7 resource. `None` when the essence is missing, encrypted, or unreadable.
fn subtitle_xml(sub_block: &str, id_to_file: &HashMap<String, PathBuf>) -> Option<String> {
    let path = asset_file(sub_block, id_to_file)?;
    let is_xml = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("xml"));
    if is_xml {
        return std::fs::read_to_string(&path).ok();
    }
    let s = path.to_str()?;
    let mut reader = asdcplib::timed_text::MxfReader::new();
    reader.open_read(s).ok()?;
    let mut buf: Vec<u8> = Vec::new();
    match reader.read_timed_text_resource(&mut buf, None, None) {
        Ok(n) => Some(String::from_utf8_lossy(&buf[..n]).into_owned()),
        Err(asdcplib::Error::BufferTooSmall { needed, .. }) => {
            buf = vec![0u8; needed];
            let n = reader.read_timed_text_resource(&mut buf, None, None).ok()?;
            Some(String::from_utf8_lossy(&buf[..n]).into_owned())
        }
        Err(_) => None,
    }
}

/// Convert a subtitle TimeIn to seconds. SMPTE ST 428-7 uses HH:MM:SS:TTT ticks
/// (250/s); Interop uses HH:MM:SS:FF editable units; decimal HH:MM:SS.mmm is
/// taken as-is.
fn subtitle_time_seconds(s: &str, standard: Standard, fps: f64) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (hms, tail) = if s.matches(':').count() == 3 {
        let idx = s.rfind(':').unwrap();
        (&s[..idx], Some(&s[idx + 1..]))
    } else {
        (s, None)
    };
    let parts: Vec<&str> = hms.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let h: f64 = parts[0].parse().ok()?;
    let m: f64 = parts[1].parse().ok()?;
    let (sec, milli) = match parts[2].split_once('.') {
        Some((a, b)) => (a.parse::<f64>().ok()?, b),
        None => (parts[2].parse::<f64>().ok()?, ""),
    };
    let frac = if let Some(t) = tail {
        let v: f64 = t.parse().ok()?;
        match standard {
            Standard::Interop => v / fps,
            _ => v / 250.0, // SMPTE ticks are 1/250s
        }
    } else if !milli.is_empty() {
        let take = &milli[..milli.len().min(3)];
        format!("{take:0<3}").parse::<f64>().unwrap_or(0.0) / 1000.0
    } else {
        0.0
    };
    Some(h * 3600.0 + m * 60.0 + sec + frac)
}

// ─── Timed-text content (Bv2.1 §7.2.5-7.2.7) ──────────────────────────────────
// libdcp verify.cc thresholds: subtitle lines warn at 52 chars, "max" at 79 (both
// WARNING severity there); closed-caption lines error at 32 (BV21_ERROR); more
// than 3 lines at once is a line-count violation; a cue shorter than 15 frames or
// a gap under 2 frames warns. Character counts are unicode scalar values, not
// bytes (DoM bug #3097 counted '…' as 3).

const SUBTITLE_LINE_WARN: usize = 52;
const SUBTITLE_LINE_MAX: usize = 79;
const CCAP_LINE_MAX: usize = 32;
const MAX_LINES: usize = 3;
const MIN_DURATION_FRAMES: f64 = 15.0;
const MIN_SPACING_FRAMES: i64 = 2;

#[derive(Clone, Copy)]
enum TimedTextKind {
    Subtitle,
    ClosedCaption,
}

/// One `<Subtitle>` cue: its in/out in seconds and one string per `<Text>` line.
struct Cue {
    in_s: f64,
    out_s: f64,
    lines: Vec<String>,
}

/// Bv2.1 timed-text content checks over every MainSubtitle and ClosedCaption
/// asset in the CPL. Subtitle limits are looser (warnings), closed-caption limits
/// stricter (errors); assets whose essence is missing/encrypted are skipped.
pub fn check_timed_text_content(
    cpl_path: &Path,
    standard: Standard,
    id_to_file: &HashMap<String, PathBuf>,
) -> Vec<Note> {
    let mut notes = Vec::new();
    let Ok(content) = std::fs::read_to_string(cpl_path) else {
        return notes;
    };

    let reel_re = regex_lite::Regex::new(r"<Reel>([\s\S]*?)</Reel>").unwrap();
    let sub_re = regex_lite::Regex::new(
        r"<(?:[\w-]+:)?MainSubtitle(?:\s[^>]*)?>([\s\S]*?)</(?:[\w-]+:)?MainSubtitle>",
    )
    .unwrap();
    let ccap_re = regex_lite::Regex::new(
        r"<(?:[\w-]+:)?ClosedCaption(?:\s[^>]*)?>([\s\S]*?)</(?:[\w-]+:)?ClosedCaption>",
    )
    .unwrap();

    for reel_cap in reel_re.captures_iter(&content) {
        let reel = reel_cap.get(1).unwrap().as_str();
        for cap in sub_re.captures_iter(reel) {
            let block = cap.get(1).unwrap().as_str();
            content_notes(
                block,
                reel,
                TimedTextKind::Subtitle,
                standard,
                id_to_file,
                cpl_path,
                &mut notes,
            );
        }
        for cap in ccap_re.captures_iter(reel) {
            let block = cap.get(1).unwrap().as_str();
            content_notes(
                block,
                reel,
                TimedTextKind::ClosedCaption,
                standard,
                id_to_file,
                cpl_path,
                &mut notes,
            );
        }
    }
    notes
}

#[allow(clippy::too_many_arguments)]
fn content_notes(
    block: &str,
    reel: &str,
    kind: TimedTextKind,
    standard: Standard,
    id_to_file: &HashMap<String, PathBuf>,
    cpl_path: &Path,
    notes: &mut Vec<Note>,
) {
    let Some(xml) = subtitle_xml(block, id_to_file) else {
        return; // essence missing/encrypted/unreadable
    };
    let fps = extract_rate(block, "EditRate")
        .or_else(|| reel_picture_edit_rate(reel))
        .map(|(n, d)| n as f64 / d.max(1) as f64)
        .unwrap_or(24.0);
    let cues = parse_timed_text_cues(&xml, standard, fps);

    check_lines(&cues, kind, cpl_path, notes);
    check_durations_and_spacing(&cues, fps, cpl_path, notes);
    if let TimedTextKind::ClosedCaption = kind {
        check_ccap_charset(&cues, cpl_path, notes);
    }
}

/// Parse a subtitle/caption reel into cues, treating each `<Text>` element as one
/// displayed line (text of nested formatting elements is concatenated).
fn parse_timed_text_cues(xml: &str, standard: Standard, fps: f64) -> Vec<Cue> {
    use quick_xml::Reader;
    use quick_xml::events::Event;

    let mut reader = Reader::from_str(xml);
    let mut cues = Vec::new();
    let mut cur: Option<Cue> = None;
    let mut text_depth: u32 = 0; // >0 while inside a <Text>
    let mut line = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match local_name(e.name()).as_str() {
                "Subtitle" => {
                    let tin = attr_val(&e, "TimeIn")
                        .and_then(|s| subtitle_time_seconds(&s, standard, fps));
                    let tout = attr_val(&e, "TimeOut")
                        .and_then(|s| subtitle_time_seconds(&s, standard, fps));
                    if let (Some(i), Some(o)) = (tin, tout) {
                        cur = Some(Cue {
                            in_s: i,
                            out_s: o,
                            lines: Vec::new(),
                        });
                    }
                }
                "Text" if cur.is_some() => {
                    if text_depth == 0 {
                        line.clear();
                    }
                    text_depth += 1;
                }
                _ => {}
            },
            Ok(Event::Text(e)) => {
                if text_depth > 0 {
                    line.push_str(&dcpdoctor_parse::text_of(&e));
                }
            }
            Ok(Event::End(e)) => match local_name(e.name()).as_str() {
                "Text" if text_depth > 0 => {
                    text_depth -= 1;
                    if text_depth == 0
                        && let Some(c) = cur.as_mut()
                    {
                        c.lines.push(line.trim().to_string());
                    }
                }
                "Subtitle" => {
                    if let Some(c) = cur.take() {
                        cues.push(c);
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    cues
}

fn local_name(name: quick_xml::name::QName) -> String {
    String::from_utf8_lossy(name.local_name().as_ref()).into_owned()
}

fn attr_val(e: &quick_xml::events::BytesStart, name: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        (a.key.local_name().as_ref() == name.as_bytes())
            .then(|| String::from_utf8_lossy(&a.value).into_owned())
    })
}

/// Line-count and line-length checks. Counts unicode scalar values per line.
fn check_lines(cues: &[Cue], kind: TimedTextKind, cpl: &Path, notes: &mut Vec<Note>) {
    let (warn_len, max_len) = match kind {
        TimedTextKind::Subtitle => (SUBTITLE_LINE_WARN, SUBTITLE_LINE_MAX),
        TimedTextKind::ClosedCaption => (CCAP_LINE_MAX, CCAP_LINE_MAX),
    };

    let mut over_count = false;
    let mut over_warn = false;
    let mut over_max = false;
    let mut longest = 0usize;
    for c in cues {
        if c.lines.len() > MAX_LINES {
            over_count = true;
        }
        for line in &c.lines {
            let n = line.chars().count();
            longest = longest.max(n);
            if n > max_len {
                over_max = true;
            } else if n > warn_len {
                over_warn = true;
            }
        }
    }

    match kind {
        TimedTextKind::Subtitle => {
            if over_count {
                notes.push(
                    Note::warning(
                        Code::SubtitleLineCount,
                        "More than 3 subtitle lines are shown at once (Bv2.1 §7.2.7)",
                    )
                    .with_file(cpl),
                );
            }
            if over_max {
                notes.push(
                    Note::warning(
                        Code::SubtitleLineLength,
                        format!(
                            "A subtitle line exceeds the {SUBTITLE_LINE_MAX}-character maximum (longest {longest})"
                        ),
                    )
                    .with_file(cpl),
                );
            } else if over_warn {
                notes.push(
                    Note::warning(
                        Code::SubtitleLineLength,
                        format!(
                            "A subtitle line exceeds the recommended {SUBTITLE_LINE_WARN} characters (longest {longest})"
                        ),
                    )
                    .with_file(cpl),
                );
            }
        }
        TimedTextKind::ClosedCaption => {
            if over_count {
                notes.push(
                    Note::error(
                        Code::ClosedCaptionLineCount,
                        "More than 3 closed-caption lines are shown at once (Bv2.1 §7.2.6)",
                    )
                    .with_file(cpl),
                );
            }
            if over_max {
                notes.push(
                    Note::error(
                        Code::ClosedCaptionLineLength,
                        format!(
                            "A closed-caption line exceeds the {CCAP_LINE_MAX}-character limit (longest {longest})"
                        ),
                    )
                    .with_file(cpl),
                );
            }
        }
    }
}

/// Minimum-duration and minimum-gap checks, in editable units at the reel rate.
/// Both severities are warnings, matching libdcp for subtitles and captions.
fn check_durations_and_spacing(cues: &[Cue], fps: f64, cpl: &Path, notes: &mut Vec<Note>) {
    let mut too_short = false;
    let mut too_close = false;
    let mut last_out_frame: Option<i64> = None;

    for c in cues {
        // ceil the duration into frames, like libdcp's as_editable_units_ceil
        let dur_frames = ((c.out_s - c.in_s) * fps - 1e-6).ceil();
        if dur_frames < MIN_DURATION_FRAMES {
            too_short = true;
        }
        let in_frame = (c.in_s * fps - 1e-6).ceil() as i64;
        if let Some(prev_out) = last_out_frame {
            let distance = in_frame - prev_out;
            if (0..MIN_SPACING_FRAMES).contains(&distance) {
                too_close = true;
            }
        }
        last_out_frame = Some((c.out_s * fps + 1e-6).floor() as i64);
    }

    if too_short {
        notes.push(
            Note::warning(
                Code::SubtitleDuration,
                format!(
                    "A timed-text event is shorter than the Bv2.1 minimum of {} frames",
                    MIN_DURATION_FRAMES as i64
                ),
            )
            .with_file(cpl),
        );
    }
    if too_close {
        notes.push(
            Note::warning(
                Code::SubtitleSpacing,
                format!(
                    "Two timed-text events are separated by less than the Bv2.1 minimum of {MIN_SPACING_FRAMES} frames"
                ),
            )
            .with_file(cpl),
        );
    }
}

/// ISDCF Doc 9 recommends the ISO 8859-1 set plus U+266A (♪) for closed captions;
/// flag any character outside it as an Info note so authors can review portability.
fn check_ccap_charset(cues: &[Cue], cpl: &Path, notes: &mut Vec<Note>) {
    let mut out_of_set: std::collections::BTreeSet<char> = std::collections::BTreeSet::new();
    for c in cues {
        for ch in c.lines.iter().flat_map(|l| l.chars()) {
            if !isdcf_doc9_char(ch) {
                out_of_set.insert(ch);
            }
        }
    }
    if out_of_set.is_empty() {
        return;
    }
    let list = out_of_set
        .iter()
        .map(|c| format!("{c} (U+{:04X})", *c as u32))
        .collect::<Vec<_>>()
        .join(", ");
    notes.push(
        Note::info(
            Code::ClosedCaptionCharset,
            format!(
                "Closed-caption text uses characters outside the ISDCF Doc 9 recommended set (ISO 8859-1 plus U+266A): {list}"
            ),
        )
        .with_file(cpl),
    );
}

fn isdcf_doc9_char(c: char) -> bool {
    let u = c as u32;
    matches!(u, 0x20..=0x7E | 0xA0..=0xFF | 0x266A) || c == '\n' || c == '\t'
}

// ─── Reel Duration (ST 429-7) ─────────────────────────────────────────────────

/// SMPTE ST 429-7: every reel shall be at least one second long. Uses the
/// picture track's Duration and EditRate; reels with no readable rate are
/// skipped (DoM #2723).
pub fn check_reel_duration(cpl_path: &Path) -> Vec<Note> {
    let mut notes = Vec::new();
    let Ok(content) = std::fs::read_to_string(cpl_path) else {
        return notes;
    };
    let reel_re = regex_lite::Regex::new(r"<Reel>([\s\S]*?)</Reel>").unwrap();
    for (i, cap) in reel_re.captures_iter(&content).enumerate() {
        let reel = cap.get(1).unwrap().as_str();
        let (Some(dur), Some((n, d))) = (reel_picture_duration(reel), reel_picture_edit_rate(reel))
        else {
            continue;
        };
        if n == 0 {
            continue;
        }
        let seconds = dur as f64 * d as f64 / n as f64;
        if seconds + 1e-6 < 1.0 {
            notes.push(
                Note::warning(
                    Code::ReelTooShort,
                    format!(
                        "Reel {} is {seconds:.2}s long, shorter than the SMPTE ST 429-7 minimum of 1s",
                        i + 1
                    ),
                )
                .with_file(cpl_path),
            );
        }
    }
    notes
}

// ─── Sound Channel Configuration (Bv2.1 §10.3.1) ──────────────────────────────

/// Channel configuration of a reel's MainSound essence, read from the WAV
/// descriptor's ChannelAssignment UL. `None` when no readable sound essence is
/// available (XML-only validation).
fn sound_channel_format(
    block: &str,
    id_to_file: &HashMap<String, PathBuf>,
) -> Option<asdcplib::pcm::ChannelFormat> {
    let path = asset_file(block, id_to_file)?;
    let s = path.to_str()?;
    let mut reader = asdcplib::pcm::MxfReader::new();
    reader.open_read(s).ok()?;
    Some(reader.audio_descriptor().ok()?.channel_format)
}

/// SMPTE Bv2.1 (RDD 52) §10.3.1: sound track files shall use Static Container
/// Channel Configuration 4 (the "open" ChannelAssignment UL) together with
/// ST 377-4 MCA labels. A SMPTE sound asset declaring a legacy static
/// configuration (1/2/3/5) is flagged (DoM #1960). Read from the essence, so
/// XML-only validation is skipped; Interop is out of scope.
pub fn check_sound_channel_configuration(
    cpl_path: &Path,
    standard: Standard,
    id_to_file: &HashMap<String, PathBuf>,
) -> Vec<Note> {
    use asdcplib::pcm::ChannelFormat;
    let mut notes = Vec::new();
    if standard != Standard::Smpte {
        return notes;
    }
    let Ok(content) = std::fs::read_to_string(cpl_path) else {
        return notes;
    };
    let sound_re = regex_lite::Regex::new(
        r"<(?:[\w-]+:)?MainSound(?:\s[^>]*)?>([\s\S]*?)</(?:[\w-]+:)?MainSound>",
    )
    .unwrap();
    for (i, cap) in sound_re.captures_iter(&content).enumerate() {
        let block = cap.get(1).unwrap().as_str();
        let Some(fmt) = sound_channel_format(block, id_to_file) else {
            continue;
        };
        // Configuration 4 (open) and the MCA form are compliant; None means the
        // descriptor carries no ChannelAssignment, handled by the MCA check.
        let legacy = match fmt {
            ChannelFormat::Cfg1 => Some(1),
            ChannelFormat::Cfg2 => Some(2),
            ChannelFormat::Cfg3 => Some(3),
            ChannelFormat::Cfg5 => Some(5),
            _ => None,
        };
        if let Some(cfg) = legacy {
            notes.push(
                Note::warning(
                    Code::SoundChannelConfigInvalid,
                    format!(
                        "Reel {} sound uses legacy Channel Configuration {cfg}; Bv2.1 §10.3.1 requires Configuration 4 with MCA labels",
                        i + 1
                    ),
                )
                .with_file(cpl_path),
            );
        }
    }
    notes
}

// ─── Subtitle Frame Rate (ST 428-7 §5.9) ──────────────────────────────────────

/// Integer frame rate declared by a subtitle document: the SMPTE `<TimeCodeRate>`
/// element or the Interop `TimeCodeRate` attribute.
fn subtitle_time_code_rate(xml: &str) -> Option<u64> {
    if let Some(v) = extract_ns_tag(xml, "TimeCodeRate")
        && let Ok(n) = v.trim().parse::<u64>()
    {
        return Some(n);
    }
    let attr = regex_lite::Regex::new(r#"TimeCodeRate\s*=\s*"(\d+)""#).unwrap();
    attr.captures(xml)?.get(1)?.as_str().parse().ok()
}

/// SMPTE ST 428-7 §5.9: a subtitle document's frame rate (TimeCodeRate, the
/// EditRate rounded to the nearest integer) should match the composition edit
/// rate; a mismatch makes players mistime the cues (DoM #2994). Assets whose
/// essence is missing/encrypted/unreadable, or that declare no rate, are skipped.
pub fn check_subtitle_frame_rate(
    cpl_path: &Path,
    id_to_file: &HashMap<String, PathBuf>,
) -> Vec<Note> {
    let mut notes = Vec::new();
    let Ok(content) = std::fs::read_to_string(cpl_path) else {
        return notes;
    };
    let reel_re = regex_lite::Regex::new(r"<Reel>([\s\S]*?)</Reel>").unwrap();
    let sub_re = regex_lite::Regex::new(
        r"<(?:[\w-]+:)?MainSubtitle(?:\s[^>]*)?>([\s\S]*?)</(?:[\w-]+:)?MainSubtitle>",
    )
    .unwrap();
    for (i, reel_cap) in reel_re.captures_iter(&content).enumerate() {
        let reel = reel_cap.get(1).unwrap().as_str();
        let Some((n, d)) = reel_picture_edit_rate(reel) else {
            continue;
        };
        if d == 0 {
            continue;
        }
        // ST 428-7 §5.9: round to nearest, ties to the higher integer
        let expected = (n as f64 / d as f64).round() as u64;
        for cap in sub_re.captures_iter(reel) {
            let block = cap.get(1).unwrap().as_str();
            let Some(xml) = subtitle_xml(block, id_to_file) else {
                continue;
            };
            let Some(rate) = subtitle_time_code_rate(&xml) else {
                continue;
            };
            if rate != expected {
                notes.push(
                    Note::warning(
                        Code::SubtitleFrameRateMismatch,
                        format!(
                            "Reel {} subtitle frame rate {rate} does not match the composition edit rate {expected}",
                            i + 1
                        ),
                    )
                    .with_file(cpl_path),
                );
            }
        }
    }
    notes
}

// ─── Non-ASCII File Names ──────────────────────────────────────────────────────

/// Flag DCP folder, file, and sub-folder names that contain non-ASCII characters
/// (DoM #3016). Some ingest systems mishandle them, so this is a portability
/// warning, not a spec violation.
pub fn check_non_ascii_names(dcp_dir: &Path) -> Vec<Note> {
    let mut notes = Vec::new();

    if let Some(name) = dcp_dir.file_name().and_then(|n| n.to_str())
        && !name.is_ascii()
    {
        notes.push(
            Note::warning(
                Code::NonAsciiFilename,
                format!("DCP folder name contains non-ASCII characters: {name}"),
            )
            .with_file(dcp_dir),
        );
    }

    let mut stack = vec![dcp_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.is_ascii() {
                notes.push(
                    Note::warning(
                        Code::NonAsciiFilename,
                        format!("File or folder name contains non-ASCII characters: {name}"),
                    )
                    .with_file(&path),
                );
            }
            if path.is_dir() {
                stack.push(path);
            }
        }
    }

    notes
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Extract the first `<Tag>` or `<ns:Tag>` element's trimmed text.
fn extract_ns_tag(xml: &str, tag: &str) -> Option<String> {
    let re = regex_lite::Regex::new(&format!(
        r"<(?:[\w-]+:)?{tag}(?:\s[^>]*)?>([\s\S]*?)</(?:[\w-]+:)?{tag}>"
    ))
    .ok()?;
    re.captures(xml)
        .map(|c| c.get(1).unwrap().as_str().trim().to_string())
}

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
            notes
                .iter()
                .any(|n| n.code == Code::CplMismatchedDurations && n.severity == Severity::Warning),
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

    // ─── check_dcp_signed (ClairMeta) ──────────────────────────────────────

    #[test]
    fn encrypted_unsigned_package_fires() {
        let cpl = write_cpl("<CompositionPlaylist><KeyId>k</KeyId></CompositionPlaylist>");
        let pkl = write_cpl("<PackingList></PackingList>");
        let notes = check_dcp_signed(&[cpl.path().to_path_buf()], &[pkl.path().to_path_buf()]);
        // both the encrypted CPL and the PKL lack a signature
        assert_eq!(
            notes
                .iter()
                .filter(|n| n.code == Code::DcpNotSigned)
                .count(),
            2,
            "unsigned CPL and PKL of an encrypted package must both fire"
        );
    }

    #[test]
    fn encrypted_signed_package_silent() {
        let cpl = write_cpl(
            "<CompositionPlaylist><KeyId>k</KeyId><Signature>x</Signature></CompositionPlaylist>",
        );
        let pkl = write_cpl("<PackingList><ds:Signature>x</ds:Signature></PackingList>");
        assert!(
            check_dcp_signed(&[cpl.path().to_path_buf()], &[pkl.path().to_path_buf()]).is_empty(),
            "signed encrypted package must be clean"
        );
    }

    #[test]
    fn unencrypted_unsigned_package_silent() {
        let cpl = write_cpl("<CompositionPlaylist></CompositionPlaylist>");
        let pkl = write_cpl("<PackingList></PackingList>");
        assert!(
            check_dcp_signed(&[cpl.path().to_path_buf()], &[pkl.path().to_path_buf()]).is_empty(),
            "an unencrypted package need not be signed"
        );
    }

    // ─── MainSoundConfiguration (ST 429-16) ────────────────────────────────

    fn msc_cpl(value: &str) -> String {
        format!(
            r#"<CompositionPlaylist><meta:CompositionMetadataAsset xmlns:meta="http://www.smpte-ra.org/schemas/429-16/2014/CPL-Metadata">
  <meta:MainSoundConfiguration>{value}</meta:MainSoundConfiguration>
</meta:CompositionMetadataAsset></CompositionPlaylist>"#
        )
    }

    #[test]
    fn valid_main_sound_configuration_passes() {
        let f = write_cpl(&msc_cpl("51/L,R,C,LFE,Ls,Rs"));
        assert!(
            check_main_sound_configuration(f.path(), Standard::Smpte, Some(6)).is_empty(),
            "valid 5.1 config with matching channel count must be clean"
        );
    }

    #[test]
    fn garbage_main_sound_configuration_is_error() {
        let f = write_cpl(&msc_cpl("None"));
        let notes = check_main_sound_configuration(f.path(), Standard::Smpte, None);
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::MainSoundConfigInvalid && n.severity == Severity::Error),
            "\"None\" must be flagged as an error, got: {notes:?}"
        );
    }

    #[test]
    fn main_sound_configuration_channel_count_mismatch_is_error() {
        let f = write_cpl(&msc_cpl("51/L,R,C,LFE,Ls,Rs"));
        // 6 channels declared, MXF reports 2
        let notes = check_main_sound_configuration(f.path(), Standard::Smpte, Some(2));
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::SoundInvalidChannelCount
                    && n.severity == Severity::Error
                    && n.message.contains("6 channels but sound assets have 2")),
            "channel-count mismatch must be flagged, got: {notes:?}"
        );
    }

    #[test]
    fn missing_main_sound_configuration_warns_when_metadata_present() {
        let f = write_cpl(
            r#"<CompositionPlaylist><meta:CompositionMetadataAsset xmlns:meta="x"><meta:Other>1</meta:Other></meta:CompositionMetadataAsset></CompositionPlaylist>"#,
        );
        let notes = check_main_sound_configuration(f.path(), Standard::Smpte, None);
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::MainSoundConfigInvalid && n.severity == Severity::Warning),
            "missing MainSoundConfiguration must warn, got: {notes:?}"
        );
    }

    #[test]
    fn interop_cpl_skips_main_sound_configuration() {
        let f = write_cpl(&msc_cpl("None"));
        assert!(
            check_main_sound_configuration(f.path(), Standard::Interop, None).is_empty(),
            "Interop CPLs carry no CompositionMetadataAsset"
        );
    }

    // ─── FFOC / LFOC marker offsets ────────────────────────────────────────

    fn marker_cpl(ffoc: u64, lfoc: u64, last_duration: u64) -> String {
        format!(
            r#"<CompositionPlaylist><Reel><Id>urn:uuid:00000000-0000-0000-0000-0000000000f0</Id><AssetList>
  <MainMarkers><Id>urn:uuid:00000000-0000-0000-0000-0000000000m0</Id>
    <MarkerList>
      <Marker><Label>FFOC</Label><Offset>{ffoc}</Offset></Marker>
      <Marker><Label>LFOC</Label><Offset>{lfoc}</Offset></Marker>
    </MarkerList>
  </MainMarkers>
  <MainPicture><Id>urn:uuid:00000000-0000-0000-0000-0000000000b1</Id><Duration>{last_duration}</Duration></MainPicture>
</AssetList></Reel></CompositionPlaylist>"#
        )
    }

    #[test]
    fn correct_ffoc_lfoc_offsets_pass() {
        // FFOC = 1, LFOC = duration - 1 = 99
        let f = write_cpl(&marker_cpl(1, 99, 100));
        let notes = check_markers(f.path(), false);
        assert!(
            !notes.iter().any(|n| n.code == Code::MarkerInvalid),
            "correct FFOC/LFOC offsets must not be flagged, got: {notes:?}"
        );
    }

    #[test]
    fn wrong_ffoc_offset_is_flagged() {
        let f = write_cpl(&marker_cpl(5, 99, 100));
        let notes = check_markers(f.path(), false);
        assert!(
            notes.iter().any(|n| n.code == Code::MarkerInvalid
                && n.message == "The FFOC marker is 5 instead of 1"),
            "got: {notes:?}"
        );
    }

    #[test]
    fn wrong_lfoc_offset_is_flagged() {
        // LFOC should be 99 (duration 100 - 1), but is 50
        let f = write_cpl(&marker_cpl(1, 50, 100));
        let notes = check_markers(f.path(), false);
        assert!(
            notes.iter().any(|n| n.code == Code::MarkerInvalid
                && n.message.contains("The LFOC marker is 50 instead of")),
            "got: {notes:?}"
        );
    }

    #[test]
    fn strict_mode_flags_missing_ffoc_lfoc() {
        // a reel with only FFMC/LFMC markers: strict must warn FFOC and LFOC missing
        let cpl = r#"<CompositionPlaylist><Reel><Id>urn:uuid:00000000-0000-0000-0000-0000000000f0</Id><AssetList>
  <MainMarkers><Id>urn:uuid:00000000-0000-0000-0000-0000000000d0</Id>
    <MarkerList>
      <Marker><Label>FFMC</Label><Offset>0</Offset></Marker>
      <Marker><Label>LFMC</Label><Offset>99</Offset></Marker>
    </MarkerList>
  </MainMarkers>
  <MainPicture><Id>urn:uuid:00000000-0000-0000-0000-0000000000b1</Id><Duration>100</Duration></MainPicture>
</AssetList></Reel></CompositionPlaylist>"#;
        let f = write_cpl(cpl);
        let notes = check_markers(f.path(), true);
        for label in ["FFOC", "LFOC"] {
            assert!(
                notes.iter().any(|n| n.code == Code::MarkerMissing
                    && n.severity == Severity::Warning
                    && n.message.contains(label)),
                "strict must warn {label} missing, got: {notes:?}"
            );
        }
        // non-strict must not require presence
        assert!(
            !check_markers(f.path(), false)
                .iter()
                .any(|n| n.code == Code::MarkerMissing),
            "presence is strict-only"
        );
    }

    // ─── First subtitle timing (Bv2.1, DCP-o-matic bug #2757) ──────────────

    const SUB_ID: &str = "aaaaaaaa-0000-0000-0000-000000005500";

    /// Write a subtitle XML file and build the CPL + id_to_file map that point a
    /// single first reel's MainSubtitle at it. `second_reel_sub` optionally adds a
    /// second reel with a subtitle starting at time 0.
    fn subtitle_case(
        sub_xml: &str,
        second_reel_sub: Option<&str>,
    ) -> (
        tempfile::NamedTempFile,
        tempfile::TempDir,
        HashMap<String, PathBuf>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let sub_path = dir.path().join("sub.xml");
        std::fs::write(&sub_path, sub_xml).unwrap();

        let mut id_to_file = HashMap::new();
        id_to_file.insert(SUB_ID.to_string(), sub_path);

        let reel = |sub_id: &str| {
            format!(
                r#"<Reel><Id>urn:uuid:00000000-0000-0000-0000-0000000000f0</Id><AssetList>
  <MainPicture><Id>urn:uuid:00000000-0000-0000-0000-0000000000b1</Id><EditRate>24 1</EditRate><Duration>240</Duration></MainPicture>
  <MainSubtitle><Id>urn:uuid:{sub_id}</Id><EditRate>24 1</EditRate><Duration>240</Duration></MainSubtitle>
</AssetList></Reel>"#
            )
        };
        let mut reels = reel(SUB_ID);
        if let Some(second) = second_reel_sub {
            let second_path = dir.path().join("sub2.xml");
            std::fs::write(&second_path, second).unwrap();
            let second_id = "bbbbbbbb-0000-0000-0000-000000005502";
            id_to_file.insert(second_id.to_string(), second_path);
            reels.push_str(&reel(second_id));
        }
        let cpl = write_cpl(&format!(
            "<CompositionPlaylist>{reels}</CompositionPlaylist>"
        ));
        (cpl, dir, id_to_file)
    }

    fn smpte_sub(time_in: &str) -> String {
        format!(
            r#"<dcst:SubtitleReel xmlns:dcst="http://www.smpte-ra.org/schemas/428-7/2010/DCST">
  <dcst:SubtitleList><dcst:Subtitle SpotNumber="1" TimeIn="{time_in}" TimeOut="00:00:20:000"/></dcst:SubtitleList>
</dcst:SubtitleReel>"#
        )
    }

    const EMPTY_SUB: &str = r#"<dcst:SubtitleReel xmlns:dcst="http://www.smpte-ra.org/schemas/428-7/2010/DCST">
  <dcst:SubtitleList/>
</dcst:SubtitleReel>"#;

    #[test]
    fn first_subtitle_too_early_warns() {
        let (cpl, _dir, id_to_file) = subtitle_case(&smpte_sub("00:00:01:000"), None);
        let notes = check_first_subtitle_timing(cpl.path(), Standard::Smpte, &id_to_file);
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::SubtitleFirstEventEarly && n.severity == Severity::Warning),
            "1s subtitle must warn, got: {notes:?}"
        );
    }

    #[test]
    fn first_subtitle_after_four_seconds_passes() {
        let (cpl, _dir, id_to_file) = subtitle_case(&smpte_sub("00:00:05:000"), None);
        assert!(
            check_first_subtitle_timing(cpl.path(), Standard::Smpte, &id_to_file).is_empty(),
            "5s subtitle must be clean"
        );
    }

    #[test]
    fn empty_first_reel_subtitle_never_warns_even_with_later_subtitles() {
        // first reel has an empty placeholder; a second reel starts a subtitle at 0.
        // bug #2757: only the first reel matters, empty placeholders are ignored.
        let (cpl, _dir, id_to_file) = subtitle_case(EMPTY_SUB, Some(&smpte_sub("00:00:00:000")));
        assert!(
            check_first_subtitle_timing(cpl.path(), Standard::Smpte, &id_to_file).is_empty(),
            "empty first-reel subtitle must not warn regardless of later reels"
        );
    }

    // ─── Timed-text content (Bv2.1 §7.2.5-7.2.7) ───────────────────────────

    const TT_ID: &str = "cccccccc-0000-0000-0000-0000000000c1";

    /// Build a CPL whose single reel points `elem` (MainSubtitle or ClosedCaption)
    /// at the given reel XML on disk.
    fn tt_case(
        elem: &str,
        reel_xml: &str,
    ) -> (
        tempfile::NamedTempFile,
        tempfile::TempDir,
        HashMap<String, PathBuf>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tt.xml");
        std::fs::write(&path, reel_xml).unwrap();
        let mut id_to_file = HashMap::new();
        id_to_file.insert(TT_ID.to_string(), path);
        let cpl = write_cpl(&format!(
            r#"<CompositionPlaylist><Reel><AssetList>
  <MainPicture><Id>urn:uuid:00000000-0000-0000-0000-0000000000b1</Id><EditRate>24 1</EditRate><Duration>240</Duration></MainPicture>
  <{elem}><Id>urn:uuid:{TT_ID}</Id><EditRate>24 1</EditRate><Duration>240</Duration></{elem}>
</AssetList></Reel></CompositionPlaylist>"#
        ));
        (cpl, dir, id_to_file)
    }

    fn cue(tin: &str, tout: &str, lines: &[&str]) -> String {
        let texts: String = lines
            .iter()
            .map(|l| format!("<dcst:Text>{l}</dcst:Text>"))
            .collect();
        format!(
            r#"<dcst:Subtitle SpotNumber="1" TimeIn="{tin}" TimeOut="{tout}">{texts}</dcst:Subtitle>"#
        )
    }

    fn reel_of(cues: &str) -> String {
        format!(
            r#"<dcst:SubtitleReel xmlns:dcst="http://www.smpte-ra.org/schemas/428-7/2010/DCST"><dcst:SubtitleList>{cues}</dcst:SubtitleList></dcst:SubtitleReel>"#
        )
    }

    fn content(cpl: &Path, id_to_file: &HashMap<String, PathBuf>) -> Vec<Note> {
        check_timed_text_content(cpl, Standard::Smpte, id_to_file)
    }

    #[test]
    fn clean_subtitle_has_no_findings() {
        let xml = reel_of(&cue(
            "00:00:05:000",
            "00:00:07:000",
            &["Hello there", "second line"],
        ));
        let (cpl, _d, map) = tt_case("MainSubtitle", &xml);
        assert!(content(cpl.path(), &map).is_empty(), "expected clean");
    }

    #[test]
    fn subtitle_more_than_three_lines_warns() {
        let xml = reel_of(&cue("00:00:05:000", "00:00:07:000", &["a", "b", "c", "d"]));
        let (cpl, _d, map) = tt_case("MainSubtitle", &xml);
        assert!(
            content(cpl.path(), &map)
                .iter()
                .any(|n| n.code == Code::SubtitleLineCount && n.severity == Severity::Warning)
        );
    }

    #[test]
    fn subtitle_line_over_recommended_warns() {
        let long = "a".repeat(60); // 60 > 52, <= 79
        let xml = reel_of(&cue("00:00:05:000", "00:00:07:000", &[&long]));
        let (cpl, _d, map) = tt_case("MainSubtitle", &xml);
        let notes = content(cpl.path(), &map);
        assert!(notes.iter().any(|n| n.code == Code::SubtitleLineLength
            && n.severity == Severity::Warning
            && n.message.contains("recommended")));
    }

    #[test]
    fn subtitle_line_over_max_warns() {
        let long = "a".repeat(85); // > 79
        let xml = reel_of(&cue("00:00:05:000", "00:00:07:000", &[&long]));
        let (cpl, _d, map) = tt_case("MainSubtitle", &xml);
        let notes = content(cpl.path(), &map);
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::SubtitleLineLength && n.message.contains("maximum"))
        );
    }

    #[test]
    fn closed_caption_line_over_32_errors() {
        let long = "a".repeat(40); // > 32
        let xml = reel_of(&cue("00:00:05:000", "00:00:07:000", &[&long]));
        let (cpl, _d, map) = tt_case("ClosedCaption", &xml);
        assert!(
            content(cpl.path(), &map)
                .iter()
                .any(|n| n.code == Code::ClosedCaptionLineLength && n.severity == Severity::Error)
        );
    }

    #[test]
    fn closed_caption_more_than_three_lines_errors() {
        let xml = reel_of(&cue("00:00:05:000", "00:00:07:000", &["a", "b", "c", "d"]));
        let (cpl, _d, map) = tt_case("ClosedCaption", &xml);
        assert!(
            content(cpl.path(), &map)
                .iter()
                .any(|n| n.code == Code::ClosedCaptionLineCount && n.severity == Severity::Error)
        );
    }

    #[test]
    fn ccap_and_subtitle_apply_different_line_limits() {
        // 40 chars: fine for a 52-char subtitle line, over the 32-char caption limit.
        let line = "a".repeat(40);
        let xml = reel_of(&cue("00:00:05:000", "00:00:07:000", &[&line]));

        let (sub_cpl, _d1, sub_map) = tt_case("MainSubtitle", &xml);
        assert!(
            !content(sub_cpl.path(), &sub_map)
                .iter()
                .any(|n| n.code == Code::SubtitleLineLength),
            "40 chars is under the subtitle limit"
        );

        let (cc_cpl, _d2, cc_map) = tt_case("ClosedCaption", &xml);
        assert!(
            content(cc_cpl.path(), &cc_map)
                .iter()
                .any(|n| n.code == Code::ClosedCaptionLineLength),
            "40 chars is over the caption limit"
        );
    }

    #[test]
    fn short_subtitle_duration_warns() {
        // 100 SMPTE ticks = 0.4s = 9.6 frames at 24fps, under the 15-frame minimum.
        let xml = reel_of(&cue("00:00:05:000", "00:00:05:100", &["hi"]));
        let (cpl, _d, map) = tt_case("MainSubtitle", &xml);
        assert!(
            content(cpl.path(), &map)
                .iter()
                .any(|n| n.code == Code::SubtitleDuration)
        );
    }

    #[test]
    fn tight_subtitle_spacing_warns() {
        // first ends at 5.0s (frame 120); second starts at 5.04s (ceil frame 121),
        // a 1-frame gap, under the 2-frame minimum.
        let cues = format!(
            "{}{}",
            cue("00:00:04:000", "00:00:05:000", &["one"]),
            cue("00:00:05:010", "00:00:07:000", &["two"]),
        );
        let xml = reel_of(&cues);
        let (cpl, _d, map) = tt_case("MainSubtitle", &xml);
        assert!(
            content(cpl.path(), &map)
                .iter()
                .any(|n| n.code == Code::SubtitleSpacing)
        );
    }

    #[test]
    fn line_length_counts_chars_not_bytes() {
        // 42 'a' + 10 '…' = 52 chars but 72 bytes. Char counting must not warn;
        // byte counting would (DoM bug #3097 counted '…' as 3).
        let line = format!("{}{}", "a".repeat(42), "…".repeat(10));
        assert_eq!(line.chars().count(), 52);
        assert_eq!(line.len(), 72);
        let xml = reel_of(&cue("00:00:05:000", "00:00:07:000", &[&line]));
        let (cpl, _d, map) = tt_case("MainSubtitle", &xml);
        assert!(
            !content(cpl.path(), &map)
                .iter()
                .any(|n| n.code == Code::SubtitleLineLength),
            "52 unicode chars is at the limit, not over it"
        );
    }

    #[test]
    fn ccap_charset_flags_out_of_set_but_allows_music_note() {
        // ♪ (U+266A) is in the ISDCF Doc 9 set; ★ (U+2605) is not.
        let xml = reel_of(&cue("00:00:05:000", "00:00:07:000", &["♪ music ★"]));
        let (cpl, _d, map) = tt_case("ClosedCaption", &xml);
        let notes = content(cpl.path(), &map);
        let charset = notes
            .iter()
            .find(|n| n.code == Code::ClosedCaptionCharset && n.severity == Severity::Info)
            .expect("expected charset info note");
        assert!(charset.message.contains("U+2605"), "should list ★");
        assert!(!charset.message.contains('♪'), "must not list ♪");
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
            notes.iter().any(|n| n.code == Code::MissingRequiredElement
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

    // ─── Reel duration (ST 429-7) ──────────────────────────────────────────

    fn reel_dur_cpl(frames: u64, edit_rate: &str) -> String {
        format!(
            r#"<CompositionPlaylist><Reel><Id>urn:uuid:00000000-0000-0000-0000-0000000000f0</Id><AssetList>
  <MainPicture><Id>urn:uuid:00000000-0000-0000-0000-0000000000b1</Id><EditRate>{edit_rate}</EditRate><Duration>{frames}</Duration></MainPicture>
</AssetList></Reel></CompositionPlaylist>"#
        )
    }

    #[test]
    fn short_reel_flagged_long_reel_clean() {
        // 12 frames at 24 fps = 0.5 s -> too short
        let short = write_cpl(&reel_dur_cpl(12, "24 1"));
        assert!(
            check_reel_duration(short.path())
                .iter()
                .any(|n| n.code == Code::ReelTooShort),
            "half-second reel must be flagged"
        );
        // 48 frames at 24 fps = 2 s -> fine
        let ok = write_cpl(&reel_dur_cpl(48, "24 1"));
        assert!(
            check_reel_duration(ok.path()).is_empty(),
            "two-second reel must be clean"
        );
        // exactly one second is the boundary and passes
        let boundary = write_cpl(&reel_dur_cpl(24, "24 1"));
        assert!(check_reel_duration(boundary.path()).is_empty());
    }

    // ─── Subtitle frame rate (ST 428-7 §5.9) ───────────────────────────────

    #[test]
    fn subtitle_frame_rate_mismatch_flagged() {
        let dir = tempfile::tempdir().unwrap();
        let subid = "cccccccc-0000-0000-0000-000000000001";
        let sub_path = dir.path().join("sub.xml");
        let cpl = write_cpl(&format!(
            r#"<CompositionPlaylist><Reel><Id>urn:uuid:00000000-0000-0000-0000-0000000000f0</Id><AssetList>
  <MainPicture><Id>urn:uuid:00000000-0000-0000-0000-0000000000b1</Id><EditRate>24 1</EditRate><Duration>48</Duration></MainPicture>
  <MainSubtitle><Id>urn:uuid:{subid}</Id></MainSubtitle>
</AssetList></Reel></CompositionPlaylist>"#
        ));
        let map: HashMap<String, PathBuf> = HashMap::from([(subid.to_string(), sub_path.clone())]);

        // 25 fps subtitle against a 24 fps composition -> mismatch
        std::fs::write(
            &sub_path,
            r#"<dcst:SubtitleReel xmlns:dcst="http://www.smpte-ra.org/schemas/428-7/2010/DCST"><dcst:TimeCodeRate>25</dcst:TimeCodeRate></dcst:SubtitleReel>"#,
        )
        .unwrap();
        assert!(
            check_subtitle_frame_rate(cpl.path(), &map)
                .iter()
                .any(|n| n.code == Code::SubtitleFrameRateMismatch),
            "25 vs 24 must be flagged"
        );

        // matching rate is clean
        std::fs::write(
            &sub_path,
            r#"<dcst:SubtitleReel xmlns:dcst="x"><dcst:TimeCodeRate>24</dcst:TimeCodeRate></dcst:SubtitleReel>"#,
        )
        .unwrap();
        assert!(
            check_subtitle_frame_rate(cpl.path(), &map).is_empty(),
            "24 vs 24 must be clean"
        );
    }

    // ─── Non-ASCII names ────────────────────────────────────────────────────

    #[test]
    fn non_ascii_filename_flagged_ascii_clean() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("café.xml"), b"x").unwrap();
        std::fs::write(dir.path().join("plain.xml"), b"x").unwrap();
        let notes = check_non_ascii_names(dir.path());
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::NonAsciiFilename && n.message.contains("café")),
            "non-ASCII file name must be flagged, got: {notes:?}"
        );
        assert!(
            !notes.iter().any(|n| n.message.contains("plain.xml")),
            "ASCII file name must not be flagged"
        );
    }

    // ─── Sound channel configuration (Bv2.1 §10.3.1) ───────────────────────

    #[test]
    fn legacy_channel_config_flagged_config4_clean() {
        use asdcplib::pcm::{AudioDescriptor, ChannelFormat, MxfWriter};
        use asdcplib::{Rational, WriterInfo};

        let dir = tempfile::tempdir().unwrap();
        let id = "aaaaaaaa-0000-0000-0000-000000000001";
        let write_mxf = |path: &Path, fmt: ChannelFormat| {
            let desc = AudioDescriptor {
                edit_rate: Rational::new(24, 1),
                audio_sampling_rate: Rational::new(48_000, 1),
                locked: true,
                channel_count: 6,
                quantization_bits: 24,
                block_align: 18,
                avg_bps: 864_000,
                linked_track_id: 0,
                container_duration: 1,
                channel_format: fmt,
            };
            let info = WriterInfo {
                asset_uuid: [2; 16],
                ..Default::default()
            };
            let frame = vec![0u8; 36_000];
            let mut w = MxfWriter::new();
            w.open_write(path.to_str().unwrap(), &info, &desc, 16_384)
                .unwrap();
            w.write_frame(&frame, None, None).unwrap();
            w.finalize().unwrap();
        };

        let cpl = write_cpl(&format!(
            r#"<CompositionPlaylist><Reel><Id>urn:uuid:00000000-0000-0000-0000-0000000000f0</Id><AssetList>
  <MainSound><Id>urn:uuid:{id}</Id></MainSound>
</AssetList></Reel></CompositionPlaylist>"#
        ));

        // legacy static configuration 1 (5.1) -> flagged
        let legacy = dir.path().join("legacy.mxf");
        write_mxf(&legacy, ChannelFormat::Cfg1);
        let map: HashMap<String, PathBuf> = HashMap::from([(id.to_string(), legacy)]);
        let notes = check_sound_channel_configuration(cpl.path(), Standard::Smpte, &map);
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::SoundChannelConfigInvalid),
            "legacy Configuration 1 must be flagged, got: {notes:?}"
        );

        // configuration 4 (open) -> clean
        let ok = dir.path().join("ok.mxf");
        write_mxf(&ok, ChannelFormat::Cfg4);
        let map2: HashMap<String, PathBuf> = HashMap::from([(id.to_string(), ok)]);
        assert!(
            check_sound_channel_configuration(cpl.path(), Standard::Smpte, &map2).is_empty(),
            "Configuration 4 must be clean"
        );

        // Interop is out of scope
        assert!(check_sound_channel_configuration(cpl.path(), Standard::Interop, &map2).is_empty());
    }
}
