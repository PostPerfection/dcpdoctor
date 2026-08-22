//! Native IMF (Interoperable Master Format) validation.
//!
//! Uses `dcpdoctor_imf` for shared parsing and pure validation logic.
//! This module adds filesystem-specific validation (MXF essence, TTML,
//! PKL cross-referencing, etc.) on top.

use dcpdoctor_parse::text_of as decode_text;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use quick_xml::events::Event;
use quick_xml::name::ResolveResult;
use quick_xml::{NsReader, Reader};

use crate::{Code, Note, Severity};

// Re-export shared types for use by the rest of the crate.
pub use dcpdoctor_imf::{
    EssenceDescriptor, ImfApplication, ImfCpl, ImfNote, ImfSeverity, Marker, TrackResource,
    TrackType, VirtualTrack,
};

// Shared OV-aware cross-ref resolver (also used by the DCP path in validators).
use dcpdoctor_imf::{RefStatus, resolve_track_ref};

// ─── Note Conversion ───────────────────────────────────────────────────────────

/// Convert a shared `ImfNote` into the core's `Note` type, attaching a file path.
fn convert_note(n: &ImfNote, file: &Path) -> Note {
    let severity = match n.severity {
        ImfSeverity::Error => Severity::Error,
        ImfSeverity::Warning => Severity::Warning,
        ImfSeverity::Info => Severity::Info,
    };
    let code = match n.code {
        "missing_required_element" => Code::MissingRequiredElement,
        "smpte_namespace_wrong" => Code::SmpteNamespaceWrong,
        "cpl_missing_reel" => Code::CplMissingReel,
        "reel_discontinuity" => Code::ReelDiscontinuity,
        "cpl_invalid_duration" => Code::CplInvalidDuration,
        "cpl_invalid_edit_rate" => Code::CplInvalidEditRate,
        "cpl_mismatched_durations" => Code::CplMismatchedDurations,
        "invalid_uuid" => Code::InvalidUuid,
        "duplicate_asset_id" => Code::DuplicateAssetId,
        "cpl_invalid_content_kind" => Code::CplInvalidContentKind,
        "xml_schema_violation" => Code::XmlSchemaViolation,
        "cross_ref_broken" => Code::CrossRefBroken,
        "marker_invalid" => Code::MarkerInvalid,
        "picture_invalid_resolution" => Code::PictureInvalidResolution,
        "mxf_invalid_structure" => Code::MxfInvalidStructure,
        "sound_invalid_sample_rate" => Code::SoundInvalidSampleRate,
        "sound_invalid_channel_count" => Code::SoundInvalidChannelCount,
        _ => Code::MissingRequiredElement,
    };
    Note {
        severity,
        code,
        message: n.message.clone(),
        file: Some(file.to_path_buf()),
        line: 0,
    }
}

// ─── IMF Validation ────────────────────────────────────────────────────────────

/// Return whether a directory contains an IMF Composition Playlist.
pub fn is_imf_package(package_dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(package_dir) else {
        return false;
    };

    entries.flatten().any(|entry| {
        let path = entry.path();
        if !path.is_file()
            || path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_none_or(|extension| !extension.eq_ignore_ascii_case("xml"))
        {
            return false;
        }

        std::fs::read_to_string(path).is_ok_and(|xml| is_imf_composition_playlist(&xml))
    })
}

fn is_imf_composition_playlist(xml: &str) -> bool {
    let mut reader = NsReader::from_str(xml);

    loop {
        match reader.read_resolved_event() {
            Ok((namespace, Event::Start(element) | Event::Empty(element))) => {
                if element.local_name().as_ref() != b"CompositionPlaylist" {
                    return false;
                }

                return matches!(
                    namespace,
                    ResolveResult::Bound(namespace)
                        if String::from_utf8_lossy(namespace.as_ref()).contains("/2067-3/")
                );
            }
            Ok((_, Event::Eof)) | Err(_) => return false,
            Ok(_) => {}
        }
    }
}

/// Validate an IMP (Interoperable Master Package) directory.
///
/// `ov_dir` is the Original Version IMP for a supplemental package. When set,
/// cross-references that resolve in the OV pass; a reference in neither package
/// is a real break. When unset, references missing locally are reported as
/// [`Code::SupplementalOvNotProvided`] instead of hard errors, since we cannot
/// tell a legitimate supplemental reference from a corrupt one without the OV.
///
/// `check_picture_details` turns on the checks that read every picture frame,
/// the same gate the DCP path puts them behind, and `scan_every_frame` carries
/// the codestream scan past frame 0 the way it does for a DCP. `keys` are the
/// content keys those readers decrypt encrypted track files with.
pub fn validate_imp(
    imp_dir: &Path,
    ov_dir: Option<&Path>,
    check_picture_details: bool,
    scan_every_frame: bool,
    keys: &crate::kdm::ContentKeys,
) -> Vec<Note> {
    let mut notes = Vec::new();

    // Asset ids available in the OV package (its ASSETMAP is authoritative for
    // physically-present track files).
    let mut ov_ids: HashSet<String> = HashSet::new();
    if let Some(ov_assetmap) = ov_dir.map(|dir| dir.join("ASSETMAP.xml"))
        && let Ok(xml) = std::fs::read_to_string(&ov_assetmap)
    {
        match dcpdoctor_imf::parse_assetmap_ids(&xml) {
            Ok(ids) => ov_ids = ids,
            Err(e) => notes.push(
                Note::error(
                    Code::XmlParseError,
                    format!("Cannot read the asset ids of the OV ASSETMAP: {e}"),
                )
                .with_file(&ov_assetmap),
            ),
        }
    }
    let ov_provided = ov_dir.is_some();

    let cpl_files = find_cpls(imp_dir);
    if cpl_files.is_empty() {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::MissingCpl,
            message: "No IMF Composition Playlist found in IMP".to_string(),
            file: Some(imp_dir.to_path_buf()),
            line: 0,
        });
        return notes;
    }

    for cpl_path in &cpl_files {
        let xml = match std::fs::read_to_string(cpl_path) {
            Ok(s) => s,
            Err(e) => {
                notes.push(Note {
                    severity: Severity::Error,
                    code: Code::XmlParseError,
                    message: format!("Cannot read CPL: {e}"),
                    file: Some(cpl_path.clone()),
                    line: 0,
                });
                continue;
            }
        };

        let cpl = match dcpdoctor_imf::parse_imf_cpl(&xml) {
            Ok(c) => c,
            Err(e) => {
                notes.push(Note {
                    severity: Severity::Error,
                    code: Code::XmlParseError,
                    message: format!("Failed to parse IMF CPL: {e}"),
                    file: Some(cpl_path.clone()),
                    line: 0,
                });
                continue;
            }
        };

        if !cpl.integer_parse_failures.is_empty() {
            notes.push(
                Note::error(
                    Code::CheckSkipped,
                    format!(
                        "CPL element(s) {} do not hold an integer, so the duration, offset and alignment checks that read them ran against 0",
                        cpl.integer_parse_failures.join(", ")
                    ),
                )
                .with_file(cpl_path),
            );
        }

        // Run all pure validators from the shared crate
        let imf_notes = dcpdoctor_imf::validate_imf_cpl_pure(&cpl);
        for n in &imf_notes {
            notes.push(convert_note(n, cpl_path));
        }

        // Track file references (filesystem, OV-aware)
        validate_track_file_refs(&cpl, imp_dir, &ov_ids, ov_provided, cpl_path, &mut notes);

        // MXF essence validation (filesystem)
        validate_essence_descriptors(
            &cpl,
            imp_dir,
            cpl_path,
            check_picture_details,
            scan_every_frame,
            keys,
            &mut notes,
        );

        // TTML subtitle tracks (filesystem)
        validate_ttml_tracks(&cpl, imp_dir, cpl_path, &mut notes);
    }

    // PKL ↔ CPL cross-referencing
    validate_pkl_cpl_refs(imp_dir, &cpl_files, &mut notes);
    validate_pkl_hash_algorithm(imp_dir, &mut notes);

    notes
}

// ─── Filesystem-specific Validators ────────────────────────────────────────────

fn validate_track_file_refs(
    cpl: &ImfCpl,
    imp_dir: &Path,
    ov_ids: &HashSet<String>,
    ov_provided: bool,
    cpl_path: &Path,
    notes: &mut Vec<Note>,
) {
    let referenced_ids: HashSet<&str> = cpl
        .virtual_tracks
        .iter()
        .flat_map(|vt| vt.resources.iter())
        .map(|r| r.track_file_id.as_str())
        .filter(|id| !id.is_empty())
        .collect();

    let assetmap_path = imp_dir.join("ASSETMAP.xml");
    if !assetmap_path.exists() {
        return;
    }

    let assetmap_xml = match std::fs::read_to_string(&assetmap_path) {
        Ok(s) => s,
        Err(_) => return,
    };

    let local_ids = match dcpdoctor_imf::parse_assetmap_ids(&assetmap_xml) {
        Ok(ids) => ids,
        Err(e) => {
            notes.push(
                Note::error(
                    Code::XmlParseError,
                    format!(
                        "Cannot read the asset ids of {}, so the track file references in this CPL were not checked: {e}",
                        assetmap_path.display()
                    ),
                )
                .with_file(cpl_path),
            );
            return;
        }
    };

    let mut needs_ov = 0usize;
    for ref_id in &referenced_ids {
        match resolve_track_ref(ref_id, &local_ids, ov_ids, ov_provided) {
            RefStatus::Local | RefStatus::Ov => {}
            RefStatus::BrokenWithOv => {
                notes.push(Note {
                    severity: Severity::Error,
                    code: Code::CrossRefBroken,
                    message: format!(
                        "Track file {} referenced in CPL not found in this package or the OV",
                        ref_id
                    ),
                    file: Some(cpl_path.to_path_buf()),
                    line: 0,
                });
            }
            RefStatus::UnresolvedNoOv => needs_ov += 1,
        }
    }

    if needs_ov > 0 {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::SupplementalOvNotProvided,
            message: format!(
                "CPL references {needs_ov} asset(s) not in this package; supply the OV with --ov to fully validate"
            ),
            file: Some(cpl_path.to_path_buf()),
            line: 0,
        });
    }
}

fn validate_essence_descriptors(
    cpl: &ImfCpl,
    imp_dir: &Path,
    cpl_path: &Path,
    check_picture_details: bool,
    scan_every_frame: bool,
    keys: &crate::kdm::ContentKeys,
    notes: &mut Vec<Note>,
) {
    let assetmap_path = imp_dir.join("ASSETMAP.xml");
    if !assetmap_path.exists() {
        return;
    }
    let assetmap_xml = match std::fs::read_to_string(&assetmap_path) {
        Ok(s) => s,
        Err(_) => return,
    };
    let id_to_path = parse_assetmap_paths(&assetmap_xml);

    for vt in &cpl.virtual_tracks {
        for res in &vt.resources {
            if res.track_file_id.is_empty() {
                continue;
            }
            let rel_path = match id_to_path.get(&res.track_file_id) {
                Some(p) => p,
                None => continue,
            };
            let full_path = imp_dir.join(rel_path);
            if !full_path.exists() {
                continue;
            }

            let ext = full_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if ext != "mxf" {
                continue;
            }

            let mxf_info = crate::mxf::read_mxf_info(&full_path);
            if !mxf_info.valid {
                notes.push(Note {
                    severity: Severity::Error,
                    code: Code::MxfUnreadable,
                    message: format!("Cannot read MXF track file: {}", mxf_info.error),
                    file: Some(full_path.clone()),
                    line: 0,
                });
                continue;
            }

            match vt.track_type {
                TrackType::MainImage => {
                    validate_picture_essence(
                        &mxf_info,
                        cpl,
                        &full_path,
                        check_picture_details,
                        scan_every_frame,
                        keys,
                        notes,
                    );
                }
                TrackType::MainAudio => {
                    validate_audio_essence(&mxf_info, cpl, &full_path, cpl_path, notes);
                }
                _ => {}
            }
        }
    }
}

/// The note for a picture track file whose essence is encrypted and whose content
/// key the run does not hold, so neither the bitrate measurement nor the
/// codestream scan can read it. Informational when no KDM was supplied, which is
/// a normal way to validate an IMP, and a warning when the KDM that was supplied
/// does not carry the track's key: the same split the timed-text pass reports.
/// `None` means the essence is readable, or is not AS-02 picture essence at all.
fn missing_content_key_note(mxf_path: &Path, keys: &crate::kdm::ContentKeys) -> Option<Note> {
    let mut reader = asdcplib::as02::jp2k::MxfReader::new();
    reader.open_read(mxf_path.to_str()?).ok()?;
    let info = reader.writer_info().ok()?;
    let crate::kdm::EssenceKey::Missing { key_id, had_kdm } = keys.resolve(&info) else {
        return None;
    };
    let message = format!(
        "bitrate and codestream checks did not run on encrypted picture essence: {}",
        if had_kdm {
            format!("the KDM does not carry the content key for KeyId {key_id}")
        } else {
            "no KDM and recipient key were supplied".to_string()
        }
    );
    let note = if had_kdm {
        Note::warning(Code::KdmRequired, message)
    } else {
        Note::info(Code::KdmRequired, message)
    };
    Some(note.with_file(mxf_path))
}

fn validate_picture_essence(
    mxf: &crate::mxf::MxfInfo,
    cpl: &ImfCpl,
    mxf_path: &Path,
    check_picture_details: bool,
    scan_every_frame: bool,
    keys: &crate::kdm::ContentKeys,
    notes: &mut Vec<Note>,
) {
    // measured through asdcplib, so it runs even when ffprobe gave no descriptor
    if check_picture_details {
        match missing_content_key_note(mxf_path, keys) {
            Some(note) => notes.push(note),
            None => {
                let bitrate = crate::bitrate::analyze_picture_bitrate(mxf_path, keys);
                notes.extend(crate::bitrate::report_measured_bitrate(&bitrate, mxf_path));

                let (codestream_notes, _forensics) = crate::j2k::check_picture_j2k_mxf(
                    mxf_path,
                    keys,
                    crate::j2k::PictureEssenceFamily::Imf,
                    scan_every_frame,
                );
                notes.extend(codestream_notes);
            }
        }
    }

    let pic = match &mxf.picture {
        Some(p) => p,
        None => {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::MxfInvalidStructure,
                message: "MainImage track file has no picture descriptor".to_string(),
                file: Some(mxf_path.to_path_buf()),
                line: 0,
            });
            return;
        }
    };

    if cpl.application == ImfApplication::App2e {
        let valid_resolutions = [(1920, 1080), (2048, 1080), (3840, 2160), (4096, 2160)];
        if pic.width > 0 && pic.height > 0 && !valid_resolutions.contains(&(pic.width, pic.height))
        {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::PictureInvalidResolution,
                message: format!(
                    "App 2E: invalid resolution {}x{} (allowed: 1920x1080, 2048x1080, 3840x2160, 4096x2160)",
                    pic.width, pic.height
                ),
                file: Some(mxf_path.to_path_buf()),
                line: 0,
            });
        }

        if pic.bit_depth > 0 && !matches!(pic.bit_depth, 8 | 10 | 12) {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::MxfInvalidStructure,
                message: format!(
                    "App 2E: invalid picture bit depth {} (allowed: 8, 10, 12)",
                    pic.bit_depth
                ),
                file: Some(mxf_path.to_path_buf()),
                line: 0,
            });
        }

        if pic.frame_rate_num > 0 && pic.frame_rate_den > 0 && cpl.edit_rate != (0, 0) {
            let pic_fps = pic.frame_rate_num as f64 / pic.frame_rate_den as f64;
            let cpl_fps = cpl.edit_rate.0 as f64 / cpl.edit_rate.1 as f64;
            if (pic_fps - cpl_fps).abs() > 0.01 {
                notes.push(Note {
                    severity: Severity::Warning,
                    code: Code::PictureInvalidFrameRate,
                    message: format!(
                        "Picture frame rate ({}/{} = {:.3} fps) differs from CPL edit rate ({}/{} = {:.3} fps)",
                        pic.frame_rate_num, pic.frame_rate_den, pic_fps,
                        cpl.edit_rate.0, cpl.edit_rate.1, cpl_fps
                    ),
                    file: Some(mxf_path.to_path_buf()),
                    line: 0,
                });
            }
        }
    }
}

fn validate_audio_essence(
    mxf: &crate::mxf::MxfInfo,
    cpl: &ImfCpl,
    mxf_path: &Path,
    _cpl_path: &Path,
    notes: &mut Vec<Note>,
) {
    let snd = match &mxf.sound {
        Some(s) => s,
        None => {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::MxfInvalidStructure,
                message: "MainAudio track file has no sound descriptor".to_string(),
                file: Some(mxf_path.to_path_buf()),
                line: 0,
            });
            return;
        }
    };

    if cpl.application == ImfApplication::App2e {
        if snd.sample_rate > 0 && !matches!(snd.sample_rate, 48000 | 96000) {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::SoundInvalidSampleRate,
                message: format!(
                    "App 2E: invalid audio sample rate {} Hz (allowed: 48000, 96000)",
                    snd.sample_rate
                ),
                file: Some(mxf_path.to_path_buf()),
                line: 0,
            });
        }

        if snd.bit_depth > 0 && snd.bit_depth != 24 {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::SoundInvalidChannelCount,
                message: format!(
                    "App 2E: invalid audio bit depth {} (required: 24)",
                    snd.bit_depth
                ),
                file: Some(mxf_path.to_path_buf()),
                line: 0,
            });
        }

        let valid_channels = [1, 2, 6, 8, 10, 12, 16, 24];
        if snd.channels > 0 && !valid_channels.contains(&snd.channels) {
            notes.push(Note {
                severity: Severity::Warning,
                code: Code::SoundInvalidChannelCount,
                message: format!(
                    "App 2E: unusual channel count {} (typical: 1, 2, 6, 8, 10, 12, 16, 24)",
                    snd.channels
                ),
                file: Some(mxf_path.to_path_buf()),
                line: 0,
            });
        }
    }
}

fn validate_ttml_tracks(cpl: &ImfCpl, imp_dir: &Path, cpl_path: &Path, notes: &mut Vec<Note>) {
    let assetmap_path = imp_dir.join("ASSETMAP.xml");
    let id_to_path = if assetmap_path.exists() {
        std::fs::read_to_string(&assetmap_path)
            .ok()
            .map(|xml| parse_assetmap_paths(&xml))
            .unwrap_or_default()
    } else {
        HashMap::new()
    };

    for vt in &cpl.virtual_tracks {
        if vt.track_type != TrackType::Subtitle {
            continue;
        }

        for res in &vt.resources {
            if res.edit_rate == (0, 0) {
                notes.push(Note {
                    severity: Severity::Error,
                    code: Code::CplInvalidEditRate,
                    message: format!("Subtitle resource {} has no EditRate", res.id),
                    file: Some(cpl_path.to_path_buf()),
                    line: 0,
                });
            }

            if !res.track_file_id.is_empty()
                && let Some(rel_path) = id_to_path.get(&res.track_file_id)
            {
                let full_path = imp_dir.join(rel_path);
                if full_path.exists() {
                    let ext = full_path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    if ext == "xml" || ext == "ttml" {
                        validate_ttml_file(&full_path, cpl_path, notes);
                    }
                } else {
                    notes.push(Note {
                        severity: Severity::Error,
                        code: Code::AssetNotFound,
                        message: format!("Subtitle track file not found: {}", rel_path),
                        file: Some(cpl_path.to_path_buf()),
                        line: 0,
                    });
                }
            }
        }
    }
}

fn validate_ttml_file(ttml_path: &Path, cpl_path: &Path, notes: &mut Vec<Note>) {
    let xml = match std::fs::read_to_string(ttml_path) {
        Ok(s) => s,
        Err(e) => {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::SubtitleParseError,
                message: format!("Cannot read TTML file: {e}"),
                file: Some(ttml_path.to_path_buf()),
                line: 0,
            });
            return;
        }
    };

    if !xml.contains("<tt") && !xml.contains("<tt:tt") {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::SubtitleParseError,
            message: "TTML file has no <tt> root element".to_string(),
            file: Some(ttml_path.to_path_buf()),
            line: 0,
        });
        return;
    }

    if !xml.contains("<body") && !xml.contains("<tt:body") {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::SubtitleParseError,
            message: "TTML file has no <body> element".to_string(),
            file: Some(ttml_path.to_path_buf()),
            line: 0,
        });
    }

    let has_imsc = xml.contains("imsc1") || xml.contains("IMSC");
    let has_ttml_ns =
        xml.contains("http://www.w3.org/ns/ttml") || xml.contains("http://www.w3.org/2006/10/ttaf");
    if !has_imsc && has_ttml_ns {
        notes.push(Note {
            severity: Severity::Info,
            code: Code::SubtitleParseError,
            message: "TTML file does not declare IMSC1 profile (recommended for IMF)".to_string(),
            file: Some(cpl_path.to_path_buf()),
            line: 0,
        });
    }

    let has_timing = xml.contains("begin=") || xml.contains("dur=") || xml.contains("end=");
    if !has_timing {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::SubtitleInvalidTiming,
            message: "TTML file has no timed elements (no begin/end/dur attributes)".to_string(),
            file: Some(ttml_path.to_path_buf()),
            line: 0,
        });
    }
}

/// ST 2067-2:2016 makes HashAlgorithm the last element of every PKL asset, so an
/// IMF PKL that omits it does not say which digest its Hash values are, and one
/// that binds it to the wrong namespace has not declared that element at all.
/// The DCP PKL schema (ST 429-8) has no such element, which is why this runs on
/// the IMF path only.
fn validate_pkl_hash_algorithm(imp_dir: &Path, notes: &mut Vec<Note>) {
    for pkl_path in find_pkls(imp_dir) {
        let Ok(xml) = std::fs::read_to_string(&pkl_path) else {
            continue;
        };
        let Some(pkl) = dcpdoctor_parse::parse_pkl(&xml) else {
            continue;
        };
        for asset in &pkl.assets {
            if asset.hash_algorithm.is_empty() {
                notes.push(Note {
                    severity: Severity::Error,
                    code: Code::MissingRequiredElement,
                    message: format!(
                        "PKL asset {} has no HashAlgorithm; ST 2067-2 requires it on every asset",
                        asset.id
                    ),
                    file: Some(pkl_path.clone()),
                    line: 0,
                });
                continue;
            }
            // ST 2067-2 Annex J: the Asset element "shall conform to Table J.1",
            // which declares HashAlgorithm in the PKL schema. Its ds:DigestMethodType
            // type invites binding the element to xmldsig instead, which the local
            // name alone cannot tell apart.
            if asset.hash_algorithm_namespace != pkl.namespace {
                notes.push(Note {
                    severity: Severity::Error,
                    code: Code::MissingRequiredElement,
                    message: format!(
                        "PKL asset {} binds HashAlgorithm to namespace \"{}\" instead of the PKL namespace \"{}\"",
                        asset.id, asset.hash_algorithm_namespace, pkl.namespace
                    ),
                    file: Some(pkl_path.clone()),
                    line: 0,
                });
            }
        }
    }
}

// ─── PKL ↔ CPL Cross-referencing ──────────────────────────────────────────────

fn validate_pkl_cpl_refs(imp_dir: &Path, cpl_files: &[PathBuf], notes: &mut Vec<Note>) {
    let pkl_files = find_pkls(imp_dir);
    if pkl_files.is_empty() {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::MissingPkl,
            message: "No Packing List (PKL) found in IMP".to_string(),
            file: Some(imp_dir.to_path_buf()),
            line: 0,
        });
        return;
    }

    let mut pkl_asset_ids: HashSet<String> = HashSet::new();
    let mut pkl_asset_types: HashMap<String, String> = HashMap::new();

    for pkl_path in &pkl_files {
        let xml = match std::fs::read_to_string(pkl_path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        parse_pkl_assets(&xml, &mut pkl_asset_ids, &mut pkl_asset_types);
    }

    for cpl_path in cpl_files {
        let cpl_xml = match std::fs::read_to_string(cpl_path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let cpl_id = extract_cpl_id(&cpl_xml);
        if !cpl_id.is_empty() && !pkl_asset_ids.contains(&cpl_id) {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::PklMissingAssetReference,
                message: format!("CPL {} is not listed in any PKL", cpl_id),
                file: Some(cpl_path.to_path_buf()),
                line: 0,
            });
        }
    }

    let assetmap_path = imp_dir.join("ASSETMAP.xml");
    if assetmap_path.exists()
        && let Ok(am_xml) = std::fs::read_to_string(&assetmap_path)
    {
        let id_to_path = parse_assetmap_paths(&am_xml);
        for (asset_id, mime_type) in &pkl_asset_types {
            if let Some(rel_path) = id_to_path.get(asset_id) {
                let ext = Path::new(rel_path)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                let expected_mime = match ext.as_str() {
                    "mxf" => "application/mxf",
                    "xml" => "text/xml",
                    "ttml" => "application/ttml+xml",
                    _ => "",
                };
                if !expected_mime.is_empty()
                    && !mime_type.is_empty()
                    && !mime_type.contains(expected_mime)
                {
                    notes.push(Note {
                        severity: Severity::Warning,
                        code: Code::PklMissingAssetReference,
                        message: format!(
                            "PKL asset {} has MIME type '{}' but file extension suggests '{}'",
                            asset_id, mime_type, expected_mime
                        ),
                        file: Some(imp_dir.to_path_buf()),
                        line: 0,
                    });
                }
            }
        }
    }
}

// ─── Helpers ───────────────────────────────────────────────────────────────────

fn find_cpls(imp_dir: &Path) -> Vec<PathBuf> {
    let mut cpls = Vec::new();
    let entries = match std::fs::read_dir(imp_dir) {
        Ok(e) => e,
        Err(_) => return cpls,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("xml") {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&path)
            && content.contains("CompositionPlaylist")
        {
            cpls.push(path);
        }
    }
    cpls
}

fn find_pkls(imp_dir: &Path) -> Vec<PathBuf> {
    let mut pkls = Vec::new();
    let entries = match std::fs::read_dir(imp_dir) {
        Ok(e) => e,
        Err(_) => return pkls,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("xml") {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&path)
            && content.contains("PackingList")
        {
            pkls.push(path);
        }
    }
    pkls
}

/// Parse asset ID→path mapping from an ASSETMAP.xml.
fn parse_assetmap_paths(xml: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut reader = Reader::from_str(xml);
    let mut in_asset = false;
    let mut current_id = String::new();
    let mut current_path = String::new();
    let mut current_tag = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let local = e.local_name();
                let name = String::from_utf8_lossy(local.as_ref()).to_string();
                match name.as_str() {
                    "Asset" => {
                        in_asset = true;
                        current_id.clear();
                        current_path.clear();
                    }
                    _ => current_tag = name,
                }
            }
            Ok(Event::End(ref e)) => {
                let local = e.local_name();
                let name = String::from_utf8_lossy(local.as_ref());
                if name == "Asset" && in_asset {
                    if !current_id.is_empty() && !current_path.is_empty() {
                        map.insert(current_id.clone(), current_path.clone());
                    }
                    in_asset = false;
                }
            }
            Ok(Event::Text(ref e)) if in_asset => {
                let text = decode_text(e).trim().to_string();
                if text.is_empty() {
                    continue;
                }
                match current_tag.as_str() {
                    "Id" if current_id.is_empty() => {
                        current_id = text.strip_prefix("urn:uuid:").unwrap_or(&text).to_string();
                    }
                    "Path" => {
                        current_path = text;
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    map
}

/// Parse PKL assets: collect IDs and their MIME types.
fn parse_pkl_assets(xml: &str, ids: &mut HashSet<String>, types: &mut HashMap<String, String>) {
    let mut reader = Reader::from_str(xml);
    let mut in_asset = false;
    let mut current_id = String::new();
    let mut current_type = String::new();
    let mut current_tag = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let local = e.local_name();
                let name = String::from_utf8_lossy(local.as_ref()).to_string();
                if name == "Asset" {
                    in_asset = true;
                    current_id.clear();
                    current_type.clear();
                } else {
                    current_tag = name;
                }
            }
            Ok(Event::End(ref e)) => {
                let local = e.local_name();
                let name = String::from_utf8_lossy(local.as_ref());
                if name == "Asset" && in_asset {
                    if !current_id.is_empty() {
                        ids.insert(current_id.clone());
                        if !current_type.is_empty() {
                            types.insert(current_id.clone(), current_type.clone());
                        }
                    }
                    in_asset = false;
                }
            }
            Ok(Event::Text(ref e)) if in_asset => {
                let text = decode_text(e).trim().to_string();
                if text.is_empty() {
                    continue;
                }
                match current_tag.as_str() {
                    "Id" if current_id.is_empty() => {
                        current_id = text.strip_prefix("urn:uuid:").unwrap_or(&text).to_string();
                    }
                    "Type" | "MIMEType" => {
                        current_type = text;
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
}

/// Extract the CPL Id from XML.
fn extract_cpl_id(xml: &str) -> String {
    let mut reader = Reader::from_str(xml);
    let mut in_cpl = false;
    let mut looking_for_id = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let local = e.local_name();
                let name = String::from_utf8_lossy(local.as_ref());
                if name == "CompositionPlaylist" {
                    in_cpl = true;
                } else if in_cpl && name == "Id" {
                    looking_for_id = true;
                }
            }
            Ok(Event::Text(ref e)) if looking_for_id => {
                let text = decode_text(e).trim().to_string();
                return text.strip_prefix("urn:uuid:").unwrap_or(&text).to_string();
            }
            Ok(Event::End(_)) => {
                looking_for_id = false;
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── IMF PKL HashAlgorithm (ST 2067-2:2016) ───────────────────────────────

    fn imf_pkl(hash_algorithm: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<PackingList xmlns="http://www.smpte-ra.org/schemas/2067-2/2016/PKL">
  <Id>urn:uuid:7281a71b-0dcb-4ed7-93a4-97b7929e2a7c</Id>
  <IssueDate>2016-06-30T18:19:27-00:00</IssueDate>
  <Issuer>dcpdoctor</Issuer>
  <Creator>dcpdoctor</Creator>
  <AssetList>
    <Asset>
      <Id>urn:uuid:88b5b453-a342-46eb-bc0a-4c9645f4d627</Id>
      <Hash>oQjE4GVsXTeawQOL//tMJ3HAMzk=</Hash>
      <Size>1024</Size>
      <Type>application/mxf</Type>
      <OriginalFileName>1.mxf</OriginalFileName>
      {hash_algorithm}
    </Asset>
  </AssetList>
</PackingList>"#
        )
    }

    fn pkl_notes(pkl_xml: &str) -> Vec<Note> {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("PKL.xml"), pkl_xml).unwrap();
        let mut notes = Vec::new();
        validate_pkl_hash_algorithm(dir.path(), &mut notes);
        notes
    }

    #[test]
    fn imf_pkl_with_hash_algorithm_draws_no_note() {
        let notes = pkl_notes(&imf_pkl(
            r#"<HashAlgorithm Algorithm="http://www.w3.org/2000/09/xmldsig#sha1"/>"#,
        ));
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn imf_pkl_without_hash_algorithm_is_an_error() {
        let notes = pkl_notes(&imf_pkl(""));
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].code, Code::MissingRequiredElement);
        assert_eq!(notes[0].severity, Severity::Error);
        assert!(
            notes[0].message.contains("HashAlgorithm")
                && notes[0]
                    .message
                    .contains("88b5b453-a342-46eb-bc0a-4c9645f4d627"),
            "{}",
            notes[0].message
        );
    }

    #[test]
    fn imf_pkl_with_hash_algorithm_bound_to_xmldsig_is_an_error() {
        let notes = pkl_notes(&imf_pkl(
            r#"<ds:HashAlgorithm xmlns:ds="http://www.w3.org/2000/09/xmldsig#" Algorithm="http://www.w3.org/2000/09/xmldsig#sha1"/>"#,
        ));
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert_eq!(notes[0].code, Code::MissingRequiredElement);
        assert_eq!(notes[0].severity, Severity::Error);
        assert!(
            notes[0]
                .message
                .contains("http://www.w3.org/2000/09/xmldsig#")
                && notes[0]
                    .message
                    .contains("http://www.smpte-ra.org/schemas/2067-2/2016/PKL"),
            "{}",
            notes[0].message
        );
    }

    #[test]
    fn test_parse_imf_cpl_minimal() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/2067-3/2016"
                     xmlns:cc="http://www.smpte-ra.org/schemas/2067-2/2016">
  <Id>urn:uuid:12345678-1234-1234-1234-123456789abc</Id>
  <ContentTitle>Test IMF</ContentTitle>
  <EditRate>24 1</EditRate>
  <SegmentList>
    <Segment>
      <MainImageSequence>
        <Id>urn:uuid:aaaaaaaa-1111-1111-1111-aaaaaaaaaaaa</Id>
        <ResourceList>
          <Resource>
            <Id>urn:uuid:bbbbbbbb-2222-2222-2222-bbbbbbbbbbbb</Id>
            <TrackFileId>urn:uuid:cccccccc-3333-3333-3333-cccccccccccc</TrackFileId>
            <EditRate>24 1</EditRate>
            <IntrinsicDuration>240</IntrinsicDuration>
            <EntryPoint>0</EntryPoint>
            <SourceDuration>240</SourceDuration>
          </Resource>
        </ResourceList>
      </MainImageSequence>
    </Segment>
  </SegmentList>
</CompositionPlaylist>"#;

        let cpl = dcpdoctor_imf::parse_imf_cpl(xml).unwrap();
        assert_eq!(cpl.id, "12345678-1234-1234-1234-123456789abc");
        assert_eq!(cpl.content_title, "Test IMF");
        assert_eq!(cpl.edit_rate, (24, 1));
        assert_eq!(cpl.virtual_tracks.len(), 1);
        assert_eq!(cpl.virtual_tracks[0].track_type, TrackType::MainImage);
        assert_eq!(cpl.virtual_tracks[0].resources.len(), 1);
        assert_eq!(cpl.virtual_tracks[0].resources[0].intrinsic_duration, 240);
        assert_eq!(cpl.total_duration, 240);
    }

    // ─── OV-aware supplemental cross-reference ─────────────────────────────

    const VIDEO_ID: &str = "aaaaaaaa-1111-1111-1111-aaaaaaaaaaaa";
    const AUDIO_ID: &str = "bbbbbbbb-2222-2222-2222-bbbbbbbbbbbb";

    fn no_keys() -> crate::kdm::ContentKeys {
        crate::kdm::ContentKeys::none()
    }

    #[test]
    fn resolve_track_ref_covers_local_ov_broken_and_needs_ov() {
        let local: HashSet<String> = [AUDIO_ID.to_string()].into();
        let ov: HashSet<String> = [VIDEO_ID.to_string()].into();

        assert_eq!(
            resolve_track_ref(AUDIO_ID, &local, &ov, true),
            RefStatus::Local
        );
        assert_eq!(
            resolve_track_ref(VIDEO_ID, &local, &ov, true),
            RefStatus::Ov
        );
        assert_eq!(
            resolve_track_ref("dead", &local, &ov, true),
            RefStatus::BrokenWithOv
        );
        assert_eq!(
            resolve_track_ref("dead", &local, &HashSet::new(), false),
            RefStatus::UnresolvedNoOv
        );
    }

    /// Write a minimal IMP dir: ASSETMAP listing `asset_ids` as track files plus
    /// a supplemental CPL that references `cpl_track_ids`.
    fn write_imp(dir: &Path, cpl_id: &str, asset_ids: &[&str], cpl_track_ids: &[&str]) {
        let mut asset_entries = format!(
            r#"<Asset><Id>urn:uuid:{cpl_id}</Id><ChunkList><Chunk><Path>CPL.xml</Path></Chunk></ChunkList></Asset>"#
        );
        for id in asset_ids {
            asset_entries.push_str(&format!(
                r#"<Asset><Id>urn:uuid:{id}</Id><ChunkList><Chunk><Path>{id}.mxf</Path></Chunk></ChunkList></Asset>"#
            ));
        }
        std::fs::write(
            dir.join("ASSETMAP.xml"),
            format!(
                r#"<?xml version="1.0"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:cccccccc-0000-0000-0000-000000000000</Id>
  <AssetList>
    <Asset><Id>urn:uuid:dddddddd-0000-0000-0000-000000000000</Id><PackingList>true</PackingList><ChunkList><Chunk><Path>PKL.xml</Path></Chunk></ChunkList></Asset>
    {asset_entries}
  </AssetList>
</AssetMap>"#
            ),
        )
        .unwrap();

        std::fs::write(
            dir.join("PKL.xml"),
            format!(
                r#"<?xml version="1.0"?>
<PackingList xmlns="http://www.smpte-ra.org/schemas/2067-2/2016/PKL">
  <Id>urn:uuid:dddddddd-0000-0000-0000-000000000000</Id>
  <AssetList><Asset><Id>urn:uuid:{cpl_id}</Id><Type>text/xml</Type></Asset></AssetList>
</PackingList>"#
            ),
        )
        .unwrap();

        // Two virtual tracks: image references the first track id, audio the second.
        let img = cpl_track_ids.first().copied().unwrap_or(VIDEO_ID);
        let aud = cpl_track_ids.get(1).copied().unwrap_or(AUDIO_ID);
        std::fs::write(
            dir.join("CPL.xml"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/2067-3/2016"
                     xmlns:cc="http://www.smpte-ra.org/schemas/2067-2/2016">
  <Id>urn:uuid:{cpl_id}</Id>
  <ContentTitle>Supp</ContentTitle>
  <EditRate>24 1</EditRate>
  <SegmentList>
    <Segment>
      <MainImageSequence>
        <Id>urn:uuid:eeeeeeee-0000-0000-0000-000000000000</Id>
        <ResourceList><Resource>
          <Id>urn:uuid:11110000-0000-0000-0000-000000000000</Id>
          <TrackFileId>urn:uuid:{img}</TrackFileId>
          <EditRate>24 1</EditRate><IntrinsicDuration>24</IntrinsicDuration>
          <EntryPoint>0</EntryPoint><SourceDuration>24</SourceDuration>
        </Resource></ResourceList>
      </MainImageSequence>
      <MainAudioSequence>
        <Id>urn:uuid:ffffffff-0000-0000-0000-000000000000</Id>
        <ResourceList><Resource>
          <Id>urn:uuid:22220000-0000-0000-0000-000000000000</Id>
          <TrackFileId>urn:uuid:{aud}</TrackFileId>
          <EditRate>24 1</EditRate><IntrinsicDuration>24</IntrinsicDuration>
          <EntryPoint>0</EntryPoint><SourceDuration>24</SourceDuration>
        </Resource></ResourceList>
      </MainAudioSequence>
    </Segment>
  </SegmentList>
</CompositionPlaylist>"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn supplemental_with_ov_resolves_cross_package_refs() {
        let ov = tempfile::tempdir().unwrap();
        write_imp(
            ov.path(),
            "0f0f0f0f-0000-0000-0000-000000000000",
            &[VIDEO_ID],
            &[],
        );
        let supp = tempfile::tempdir().unwrap();
        // supp physically holds only AUDIO_ID; its CPL references OV's VIDEO_ID + local AUDIO_ID
        write_imp(
            supp.path(),
            "1a1a1a1a-0000-0000-0000-000000000000",
            &[AUDIO_ID],
            &[VIDEO_ID, AUDIO_ID],
        );

        let notes = validate_imp(supp.path(), Some(ov.path()), false, false, &no_keys());
        assert!(
            !notes.iter().any(|n| n.code == Code::CrossRefBroken),
            "OV must satisfy the video ref, got: {notes:?}"
        );
        assert!(
            !notes
                .iter()
                .any(|n| n.code == Code::SupplementalOvNotProvided),
            "everything resolves, no OV-missing note expected"
        );
    }

    #[test]
    fn supplemental_alone_reports_missing_ov_not_broken() {
        let supp = tempfile::tempdir().unwrap();
        write_imp(
            supp.path(),
            "1a1a1a1a-0000-0000-0000-000000000000",
            &[AUDIO_ID],
            &[VIDEO_ID, AUDIO_ID],
        );

        let notes = validate_imp(supp.path(), None, false, false, &no_keys());
        assert!(
            !notes.iter().any(|n| n.code == Code::CrossRefBroken),
            "no OV -> must not hard-fail as broken, got: {notes:?}"
        );
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::SupplementalOvNotProvided),
            "expected SupplementalOvNotProvided diagnostic, got: {notes:?}"
        );
    }

    #[test]
    fn supplemental_with_ov_still_catches_genuinely_broken_ref() {
        let ov = tempfile::tempdir().unwrap();
        write_imp(
            ov.path(),
            "0f0f0f0f-0000-0000-0000-000000000000",
            &[VIDEO_ID],
            &[],
        );
        let supp = tempfile::tempdir().unwrap();
        // CPL references an id present in neither OV nor supp
        write_imp(
            supp.path(),
            "1a1a1a1a-0000-0000-0000-000000000000",
            &[AUDIO_ID],
            &["deadbeef-0000-0000-0000-000000000000", AUDIO_ID],
        );

        let notes = validate_imp(supp.path(), Some(ov.path()), false, false, &no_keys());
        assert!(
            notes.iter().any(|n| n.code == Code::CrossRefBroken),
            "a ref in neither package is a real break even with --ov, got: {notes:?}"
        );
    }

    #[test]
    fn an_unreadable_assetmap_says_the_track_file_references_were_not_checked() {
        let imp = tempfile::tempdir().unwrap();
        write_imp(
            imp.path(),
            "1a1a1a1a-0000-0000-0000-000000000000",
            &[VIDEO_ID, AUDIO_ID],
            &[VIDEO_ID, AUDIO_ID],
        );
        // the last Asset closes under a different name: a lenient walk would
        // return the ids before it and call the rest broken references
        let assetmap_path = imp.path().join("ASSETMAP.xml");
        let broken =
            std::fs::read_to_string(&assetmap_path)
                .unwrap()
                .replacen("</Asset>", "</Assset>", 1);
        std::fs::write(&assetmap_path, broken).unwrap();

        let notes = validate_imp(imp.path(), None, false, false, &no_keys());
        let skipped = notes
            .iter()
            .find(|n| n.code == Code::XmlParseError)
            .unwrap_or_else(|| panic!("an unreadable ASSETMAP must be reported: {notes:?}"));
        assert!(
            skipped.message.contains("ASSETMAP.xml") && skipped.message.contains("not checked"),
            "{}",
            skipped.message
        );
        assert!(
            !notes.iter().any(
                |n| n.code == Code::CrossRefBroken || n.code == Code::SupplementalOvNotProvided
            ),
            "a partial id set must not turn into reference findings, got: {notes:?}"
        );
    }

    #[test]
    fn a_cpl_duration_that_is_no_integer_says_the_checks_reading_it_ran_against_zero() {
        let imp = tempfile::tempdir().unwrap();
        write_imp(
            imp.path(),
            "1a1a1a1a-0000-0000-0000-000000000000",
            &[VIDEO_ID, AUDIO_ID],
            &[VIDEO_ID, AUDIO_ID],
        );
        let cpl_path = imp.path().join("CPL.xml");
        let garbled = std::fs::read_to_string(&cpl_path).unwrap().replace(
            "<IntrinsicDuration>24</IntrinsicDuration>",
            "<IntrinsicDuration>twenty-four</IntrinsicDuration>",
        );
        std::fs::write(&cpl_path, garbled).unwrap();

        let notes = validate_imp(imp.path(), None, false, false, &no_keys());
        let skipped = notes
            .iter()
            .find(|n| n.code == Code::CheckSkipped)
            .unwrap_or_else(|| panic!("a coerced duration must be reported: {notes:?}"));
        assert!(
            skipped.message.contains("IntrinsicDuration"),
            "{}",
            skipped.message
        );
    }

    /// Write an AS-02 picture track file of `frames` frames of `frame_bytes`
    /// each, the wrapping an IMP uses for picture essence.
    fn write_as02_picture(path: &Path, frames: u32, frame_bytes: usize) {
        use asdcplib::jp2k::PictureDescriptor;
        use asdcplib::{LabelSet, Rational, WriterInfo};

        let info = WriterInfo {
            asset_uuid: *uuid::Uuid::new_v4().as_bytes(),
            context_id: *uuid::Uuid::new_v4().as_bytes(),
            label_set: LabelSet::Smpte,
            ..Default::default()
        };
        let descriptor = PictureDescriptor {
            edit_rate: Rational::new(24, 1),
            sample_rate: Rational::new(24, 1),
            stored_width: 2048,
            stored_height: 1080,
            aspect_ratio: Rational::new(2048, 1080),
            container_duration: frames,
            component_count: 3,
        };
        let mut writer = asdcplib::as02::jp2k::MxfWriter::new();
        writer
            .open_write(path.to_str().unwrap(), &info, &descriptor, 16384)
            .unwrap();
        let frame = vec![0u8; frame_bytes];
        for _ in 0..frames {
            writer.write_frame(&frame, None, None).unwrap();
        }
        writer.finalize().unwrap();
    }

    // 500_000 bytes per frame at 24 fps is 96.0 Mb/s.
    #[test]
    fn imp_picture_track_reports_its_measured_peak() {
        let imp = tempfile::tempdir().unwrap();
        write_imp(
            imp.path(),
            "1a1a1a1a-0000-0000-0000-000000000000",
            &[VIDEO_ID, AUDIO_ID],
            &[VIDEO_ID, AUDIO_ID],
        );
        write_as02_picture(&imp.path().join(format!("{VIDEO_ID}.mxf")), 3, 500_000);

        let measured = validate_imp(imp.path(), None, true, false, &no_keys());
        let note = measured
            .iter()
            .find(|n| n.code == Code::PictureBitrateMeasured)
            .unwrap_or_else(|| panic!("expected a measured bitrate note, got: {measured:?}"));
        assert_eq!(note.severity, Severity::Info);
        assert!(note.message.contains("96.0"), "{}", note.message);

        let unmeasured = validate_imp(imp.path(), None, false, false, &no_keys());
        assert!(
            !unmeasured
                .iter()
                .any(|n| n.code == Code::PictureBitrateMeasured),
            "reading every frame stays behind the picture-details gate"
        );
    }

    /// Decomposition levels the codestream forensics fixture holds constant.
    const FIXTURE_DECOMPOSITION_LEVELS: u8 = 5;
    const FIXTURE_PAYLOAD_BYTES: usize = 64;

    #[test]
    fn imp_picture_track_reports_its_codestream_summary_when_every_frame_is_scanned() {
        let imp = tempfile::tempdir().unwrap();
        write_imp(
            imp.path(),
            "1a1a1a1a-0000-0000-0000-000000000000",
            &[VIDEO_ID, AUDIO_ID],
            &[VIDEO_ID, AUDIO_ID],
        );
        let frames = vec![(FIXTURE_DECOMPOSITION_LEVELS, FIXTURE_PAYLOAD_BYTES); 4];
        crate::j2k::frame_scan_tests::write_as02_picture_mxf(
            imp.path(),
            &format!("{VIDEO_ID}.mxf"),
            1920,
            1080,
            &frames,
        );

        let scanned = validate_imp(imp.path(), None, true, true, &no_keys());
        let summary = scanned
            .iter()
            .find(|n| n.code == Code::J2kCodestreamSummary)
            .unwrap_or_else(|| panic!("expected a codestream summary, got: {scanned:?}"));
        assert_eq!(summary.severity, Severity::Info);
        assert!(
            summary.message.contains("1920x1080")
                && summary
                    .message
                    .contains("parameters identical across 4 frames"),
            "{}",
            summary.message
        );
        assert!(
            !summary.message.contains("DCI cap"),
            "IMF essence is held to no DCI cap, got: {}",
            summary.message
        );

        let frame_zero_only = validate_imp(imp.path(), None, true, false, &no_keys());
        assert!(
            !frame_zero_only
                .iter()
                .any(|n| n.code == Code::J2kCodestreamSummary),
            "the frame-0 scan has nothing to summarise"
        );
    }

    /// An IMP whose picture track file is encrypted, with the KDM that carries its
    /// content key. `wrong_key` is a private key the KDM was not issued to.
    struct EncryptedImp {
        imp: tempfile::TempDir,
        kdm: PathBuf,
        recipient_key: PathBuf,
        wrong_key: PathBuf,
        /// keeps the generated certificate chain on disk while the KDM is in use
        _certs: tempfile::TempDir,
    }

    /// The KDM names a CPL, so the package has to be written with the id the KDM
    /// was issued for or the cross-reference check would call it a different CPL.
    const ENCRYPTED_IMP_CPL_ID: &str = "2b2b2b2b-0000-0000-0000-000000000000";

    fn encrypted_imp() -> EncryptedImp {
        let key_id = uuid::Uuid::new_v4();
        let content_key = [0x66; 16];
        let (kdm, recipient_key, wrong_key, certs) =
            crate::kdm::decrypt_tests::make_kdm(key_id, content_key, ENCRYPTED_IMP_CPL_ID);

        let imp = tempfile::tempdir().unwrap();
        write_imp(
            imp.path(),
            ENCRYPTED_IMP_CPL_ID,
            &[VIDEO_ID, AUDIO_ID],
            &[VIDEO_ID, AUDIO_ID],
        );
        let frames = vec![(FIXTURE_DECOMPOSITION_LEVELS, FIXTURE_PAYLOAD_BYTES); 4];
        crate::j2k::frame_scan_tests::write_encrypted_as02_picture_mxf(
            imp.path(),
            &format!("{VIDEO_ID}.mxf"),
            1920,
            1080,
            &frames,
            key_id,
            content_key,
        );

        EncryptedImp {
            imp,
            kdm,
            recipient_key,
            wrong_key,
            _certs: certs,
        }
    }

    // an encrypted track file used to be read with no keys at all, so an IMP with
    // a KDM got neither of the two things a cleartext one gets.
    #[test]
    fn encrypted_imp_picture_track_is_measured_and_scanned_with_its_kdm() {
        let fixture = encrypted_imp();
        let keys = crate::kdm::ContentKeys::from_kdm(&fixture.kdm, &fixture.recipient_key)
            .expect("the fixture KDM opens with its recipient key");

        let notes = validate_imp(fixture.imp.path(), None, true, true, &keys);

        let bitrate = notes
            .iter()
            .find(|n| n.code == Code::PictureBitrateMeasured)
            .unwrap_or_else(|| panic!("expected a measured bitrate, got: {notes:?}"));
        assert_eq!(bitrate.severity, Severity::Info);

        let summary = notes
            .iter()
            .find(|n| n.code == Code::J2kCodestreamSummary)
            .unwrap_or_else(|| panic!("expected a codestream summary, got: {notes:?}"));
        assert!(
            summary.message.contains("1920x1080")
                && summary
                    .message
                    .contains("parameters identical across 4 frames"),
            "{}",
            summary.message
        );

        assert!(
            !notes.iter().any(|n| n.code == Code::KdmRequired),
            "nothing is skipped once the content key is available, got: {notes:?}"
        );
    }

    #[test]
    fn encrypted_imp_picture_track_without_a_key_says_the_checks_did_not_run() {
        let fixture = encrypted_imp();

        let notes = validate_imp(fixture.imp.path(), None, true, true, &no_keys());

        let skipped = notes
            .iter()
            .find(|n| n.code == Code::KdmRequired)
            .unwrap_or_else(|| {
                panic!("a skipped encrypted picture track must be reported: {notes:?}")
            });
        assert_eq!(
            skipped.severity,
            Severity::Info,
            "validating an encrypted IMP without its KDM is a normal thing to do"
        );
        assert!(
            skipped.message.contains("did not run") && skipped.message.contains("no KDM"),
            "{}",
            skipped.message
        );
        assert!(
            !notes
                .iter()
                .any(|n| n.code == Code::PictureBitrateMeasured
                    || n.code == Code::J2kCodestreamSummary),
            "ciphertext frames must yield neither a rate nor forensics, got: {notes:?}"
        );
    }

    // a KDM the recipient key cannot open is an operator error, and continuing
    // keyless would report an encrypted IMP as if no keys had been asked for.
    #[test]
    fn a_kdm_the_recipient_key_cannot_open_fails_loud() {
        let fixture = encrypted_imp();

        let result = crate::validate::verify_dcp(
            fixture.imp.path(),
            &crate::VerifyOptions {
                kdm: Some(fixture.kdm.clone()),
                recipient_key: Some(fixture.wrong_key.clone()),
                check_picture_details: true,
                ..Default::default()
            },
        );

        let failure = result
            .notes
            .iter()
            .find(|n| n.code == Code::KdmRequired && n.severity == Severity::Error)
            .unwrap_or_else(|| panic!("a KDM that will not unwrap must error: {:?}", result.notes));
        assert!(
            failure.message.contains("failed to unwrap KDM"),
            "{}",
            failure.message
        );
    }

    #[test]
    fn test_resource_effective_duration() {
        let res = TrackResource {
            intrinsic_duration: 100,
            entry_point: 10,
            source_duration: 50,
            ..Default::default()
        };
        assert_eq!(res.effective_duration(), 50);

        let res = TrackResource {
            intrinsic_duration: 100,
            entry_point: 10,
            source_duration: 0,
            ..Default::default()
        };
        assert_eq!(res.effective_duration(), 90);
    }
}
