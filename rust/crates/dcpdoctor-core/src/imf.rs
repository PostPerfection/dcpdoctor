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
pub fn validate_imp(imp_dir: &Path) -> Vec<Note> {
    let mut notes = Vec::new();

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

        // Run all pure validators from the shared crate
        let imf_notes = dcpdoctor_imf::validate_imf_cpl_pure(&cpl);
        for n in &imf_notes {
            notes.push(convert_note(n, cpl_path));
        }

        // Track file references (filesystem)
        validate_track_file_refs(&cpl, imp_dir, cpl_path, &mut notes);

        // MXF essence validation (filesystem)
        validate_essence_descriptors(&cpl, imp_dir, cpl_path, &mut notes);

        // TTML subtitle tracks (filesystem)
        validate_ttml_tracks(&cpl, imp_dir, cpl_path, &mut notes);
    }

    // PKL ↔ CPL cross-referencing
    validate_pkl_cpl_refs(imp_dir, &cpl_files, &mut notes);

    notes
}

// ─── Filesystem-specific Validators ────────────────────────────────────────────

fn validate_track_file_refs(cpl: &ImfCpl, imp_dir: &Path, cpl_path: &Path, notes: &mut Vec<Note>) {
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

    let asset_ids = dcpdoctor_imf::parse_assetmap_ids(&assetmap_xml);

    for ref_id in &referenced_ids {
        if !asset_ids.contains(*ref_id) {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::CrossRefBroken,
                message: format!(
                    "Track file {} referenced in CPL not found in AssetMap",
                    ref_id
                ),
                file: Some(cpl_path.to_path_buf()),
                line: 0,
            });
        }
    }
}

fn validate_essence_descriptors(
    cpl: &ImfCpl,
    imp_dir: &Path,
    cpl_path: &Path,
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
                    validate_picture_essence(&mxf_info, cpl, &full_path, cpl_path, notes);
                }
                TrackType::MainAudio => {
                    validate_audio_essence(&mxf_info, cpl, &full_path, cpl_path, notes);
                }
                _ => {}
            }
        }
    }
}

fn validate_picture_essence(
    mxf: &crate::mxf::MxfInfo,
    cpl: &ImfCpl,
    mxf_path: &Path,
    _cpl_path: &Path,
    notes: &mut Vec<Note>,
) {
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
