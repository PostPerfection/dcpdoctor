//! Native IMF validation for the browser (WASM).
//!
//! Validates IMF Composition Playlists using only in-memory XML.
//! No filesystem access needed — all data comes from the browser.

use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::HashSet;

use crate::{Note, Severity};

// ─── IMF Namespaces ────────────────────────────────────────────────────────────

const NS_APP2E: &str = "http://www.smpte-ra.org/ns/2067-21/2021";
const NS_APP2E_2016: &str = "http://www.smpte-ra.org/ns/2067-21/2016";
const NS_APP5: &str = "http://www.smpte-ra.org/ns/2067-50/2017";

// ─── Application profiles ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImfApplication {
    App2e,
    App5Aces,
    #[default]
    Unknown,
}

// ─── Data structures ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct ImfCpl {
    pub id: String,
    pub content_title: String,
    pub edit_rate: (u32, u32),
    pub namespaces: Vec<String>,
    pub application: ImfApplication,
    pub virtual_tracks: Vec<VirtualTrack>,
    pub total_duration: u64,
}

#[derive(Debug, Clone, Default)]
pub struct VirtualTrack {
    pub id: String,
    pub track_type: TrackType,
    pub resources: Vec<TrackResource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrackType {
    #[default]
    MainImage,
    MainAudio,
    Subtitle,
    Marker,
    Other,
}

#[derive(Debug, Clone, Default)]
pub struct TrackResource {
    pub id: String,
    pub track_file_id: String,
    pub edit_rate: (u32, u32),
    pub intrinsic_duration: u64,
    pub entry_point: u64,
    pub source_duration: u64,
}

impl TrackResource {
    pub fn effective_duration(&self) -> u64 {
        if self.source_duration > 0 {
            self.source_duration
        } else {
            self.intrinsic_duration.saturating_sub(self.entry_point)
        }
    }
}

// ─── Public API ────────────────────────────────────────────────────────────────

/// Validate an IMF CPL from its XML content.
/// Also takes optional AssetMap XML for cross-referencing.
pub fn validate_imf_cpl(cpl_xml: &str, assetmap_xml: Option<&str>, cpl_path: &str) -> Vec<Note> {
    let mut notes = Vec::new();

    let cpl = match parse_imf_cpl(cpl_xml) {
        Ok(c) => c,
        Err(e) => {
            notes.push(Note {
                severity: Severity::Error,
                code: "xml_parse_error".to_string(),
                message: format!("Failed to parse IMF CPL: {e}"),
                file: Some(cpl_path.to_string()),
            });
            return notes;
        }
    };

    // Application identification
    if cpl.application == ImfApplication::Unknown {
        notes.push(Note {
            severity: Severity::Warning,
            code: "missing_required_element".to_string(),
            message: "No recognized IMF Application identified in CPL namespaces".to_string(),
            file: Some(cpl_path.to_string()),
        });
    }

    // Core constraints namespace
    let has_core = cpl.namespaces.iter().any(|ns| ns.contains("2067-2"));
    if !has_core {
        notes.push(Note {
            severity: Severity::Warning,
            code: "smpte_namespace_wrong".to_string(),
            message: "CPL missing ST 2067-2 core constraints namespace".to_string(),
            file: Some(cpl_path.to_string()),
        });
    }

    // Virtual track validation
    validate_virtual_tracks(&cpl, cpl_path, &mut notes);

    // Edit rate validation
    validate_edit_rates(&cpl, cpl_path, &mut notes);

    // Track file cross-references
    if let Some(am_xml) = assetmap_xml {
        validate_track_refs(&cpl, am_xml, cpl_path, &mut notes);
    }

    // Application-specific constraints
    validate_app_constraints(&cpl, cpl_path, &mut notes);

    // Timeline alignment
    validate_timeline_alignment(&cpl, cpl_path, &mut notes);

    notes
}

// ─── Sub-validators ────────────────────────────────────────────────────────────

fn validate_virtual_tracks(cpl: &ImfCpl, cpl_path: &str, notes: &mut Vec<Note>) {
    let mut has_main_image = false;

    for vt in &cpl.virtual_tracks {
        if vt.track_type == TrackType::MainImage {
            has_main_image = true;
        }

        if vt.resources.is_empty() {
            notes.push(Note {
                severity: Severity::Error,
                code: "cpl_missing_reel".to_string(),
                message: format!("Virtual track {} has no resources", vt.id),
                file: Some(cpl_path.to_string()),
            });
            continue;
        }

        // Entry point / duration bounds
        for res in &vt.resources {
            if res.entry_point >= res.intrinsic_duration && res.intrinsic_duration > 0 {
                notes.push(Note {
                    severity: Severity::Error,
                    code: "cpl_invalid_duration".to_string(),
                    message: format!(
                        "Resource {} entry_point ({}) >= intrinsic_duration ({})",
                        res.id, res.entry_point, res.intrinsic_duration
                    ),
                    file: Some(cpl_path.to_string()),
                });
            }
            if res.source_duration > 0
                && res.entry_point + res.source_duration > res.intrinsic_duration
                && res.intrinsic_duration > 0
            {
                notes.push(Note {
                    severity: Severity::Error,
                    code: "cpl_invalid_duration".to_string(),
                    message: format!(
                        "Resource {} source range exceeds intrinsic duration ({} + {} > {})",
                        res.id, res.entry_point, res.source_duration, res.intrinsic_duration
                    ),
                    file: Some(cpl_path.to_string()),
                });
            }
        }
    }

    if !has_main_image {
        notes.push(Note {
            severity: Severity::Error,
            code: "missing_required_element".to_string(),
            message: "CPL has no MainImageSequence virtual track".to_string(),
            file: Some(cpl_path.to_string()),
        });
    }
}

fn validate_edit_rates(cpl: &ImfCpl, cpl_path: &str, notes: &mut Vec<Note>) {
    if cpl.edit_rate == (0, 0) {
        notes.push(Note {
            severity: Severity::Error,
            code: "cpl_invalid_edit_rate".to_string(),
            message: "CPL has no EditRate".to_string(),
            file: Some(cpl_path.to_string()),
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
                notes.push(Note {
                    severity: Severity::Error,
                    code: "cpl_invalid_edit_rate".to_string(),
                    message: format!(
                        "Resource edit rate {}/{} incompatible with CPL {}/{}",
                        res.edit_rate.0, res.edit_rate.1, cpl.edit_rate.0, cpl.edit_rate.1
                    ),
                    file: Some(cpl_path.to_string()),
                });
            }
        }
    }
}

fn validate_track_refs(cpl: &ImfCpl, assetmap_xml: &str, cpl_path: &str, notes: &mut Vec<Note>) {
    let asset_ids = parse_assetmap_ids(assetmap_xml);

    let referenced_ids: HashSet<&str> = cpl
        .virtual_tracks
        .iter()
        .flat_map(|vt| vt.resources.iter())
        .map(|r| r.track_file_id.as_str())
        .filter(|id| !id.is_empty())
        .collect();

    for ref_id in &referenced_ids {
        if !asset_ids.contains(*ref_id) {
            notes.push(Note {
                severity: Severity::Error,
                code: "cross_ref_broken".to_string(),
                message: format!("Track file {} not found in AssetMap", ref_id),
                file: Some(cpl_path.to_string()),
            });
        }
    }
}

fn validate_app_constraints(cpl: &ImfCpl, cpl_path: &str, notes: &mut Vec<Note>) {
    match cpl.application {
        ImfApplication::App2e => {
            let valid_rates: &[(u32, u32)] = &[
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
            if cpl.edit_rate != (0, 0) && !valid_rates.contains(&cpl.edit_rate) {
                notes.push(Note {
                    severity: Severity::Error,
                    code: "cpl_invalid_edit_rate".to_string(),
                    message: format!(
                        "App 2E: invalid edit rate {}/{} (ST 2067-21)",
                        cpl.edit_rate.0, cpl.edit_rate.1
                    ),
                    file: Some(cpl_path.to_string()),
                });
            }
            let has_audio = cpl
                .virtual_tracks
                .iter()
                .any(|vt| vt.track_type == TrackType::MainAudio);
            if !has_audio {
                notes.push(Note {
                    severity: Severity::Warning,
                    code: "missing_required_element".to_string(),
                    message: "App 2E: no MainAudioSequence found".to_string(),
                    file: Some(cpl_path.to_string()),
                });
            }
        }
        ImfApplication::App5Aces => {
            let valid_rates: &[(u32, u32)] = &[
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
            if cpl.edit_rate != (0, 0) && !valid_rates.contains(&cpl.edit_rate) {
                notes.push(Note {
                    severity: Severity::Error,
                    code: "cpl_invalid_edit_rate".to_string(),
                    message: format!(
                        "App 5 ACES: invalid edit rate {}/{} (ST 2067-50)",
                        cpl.edit_rate.0, cpl.edit_rate.1
                    ),
                    file: Some(cpl_path.to_string()),
                });
            }
        }
        ImfApplication::Unknown => {}
    }
}

fn validate_timeline_alignment(cpl: &ImfCpl, cpl_path: &str, notes: &mut Vec<Note>) {
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
            notes.push(Note {
                severity: Severity::Error,
                code: "cpl_mismatched_durations".to_string(),
                message: format!(
                    "{:?} track duration ({}) differs from MainImage ({})",
                    vt.track_type, track_duration, reference_duration
                ),
                file: Some(cpl_path.to_string()),
            });
        }
    }
}

// ─── Parsing ───────────────────────────────────────────────────────────────────

fn parse_assetmap_ids(xml: &str) -> HashSet<String> {
    let mut ids = HashSet::new();
    let mut reader = Reader::from_str(xml);
    let mut in_id = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let local = e.local_name();
                let name = String::from_utf8_lossy(local.as_ref());
                in_id = name == "Id";
            }
            Ok(Event::Text(ref e)) if in_id => {
                let text = e.unescape().unwrap_or_default().trim().to_string();
                let id = text.strip_prefix("urn:uuid:").unwrap_or(&text).to_string();
                if !id.is_empty() {
                    ids.insert(id);
                }
                in_id = false;
            }
            Ok(Event::End(_)) => {
                in_id = false;
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    ids
}

pub fn parse_imf_cpl(xml: &str) -> Result<ImfCpl, String> {
    let mut cpl = ImfCpl::default();

    // Extract namespaces from root element
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let local = e.local_name();
                let name = String::from_utf8_lossy(local.as_ref());
                if name == "CompositionPlaylist" {
                    for attr in e.attributes().flatten() {
                        let value = String::from_utf8_lossy(attr.value.as_ref()).to_string();
                        if value.contains("smpte-ra.org") || value.contains("2067") {
                            cpl.namespaces.push(value);
                        }
                    }
                    break;
                }
            }
            Ok(Event::Eof) => return Err("No CompositionPlaylist element found".to_string()),
            Err(e) => return Err(format!("XML parse error: {e}")),
            _ => {}
        }
    }

    // Determine application
    cpl.application = detect_application(&cpl.namespaces, xml);

    // Full parse
    let mut reader = Reader::from_str(xml);
    let mut tag_stack: Vec<String> = Vec::new();
    let mut current_vt: Option<VirtualTrack> = None;
    let mut current_resource: Option<TrackResource> = None;
    let mut current_tag = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let local = e.local_name();
                let name = String::from_utf8_lossy(local.as_ref()).to_string();
                tag_stack.push(name.clone());
                current_tag = name.clone();

                match name.as_str() {
                    "MainImageSequence" => {
                        current_vt = Some(VirtualTrack {
                            track_type: TrackType::MainImage,
                            ..Default::default()
                        });
                    }
                    "MainAudioSequence" => {
                        current_vt = Some(VirtualTrack {
                            track_type: TrackType::MainAudio,
                            ..Default::default()
                        });
                    }
                    "SubtitlesSequence" => {
                        current_vt = Some(VirtualTrack {
                            track_type: TrackType::Subtitle,
                            ..Default::default()
                        });
                    }
                    "MarkerSequence" => {
                        current_vt = Some(VirtualTrack {
                            track_type: TrackType::Marker,
                            ..Default::default()
                        });
                    }
                    "Resource" => {
                        current_resource = Some(TrackResource::default());
                    }
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => {
                let local = e.local_name();
                let name = String::from_utf8_lossy(local.as_ref()).to_string();
                tag_stack.pop();

                match name.as_str() {
                    "MainImageSequence" | "MainAudioSequence" | "SubtitlesSequence"
                    | "MarkerSequence" => {
                        if let Some(vt) = current_vt.take() {
                            cpl.virtual_tracks.push(vt);
                        }
                    }
                    "Resource" => {
                        if let Some(res) = current_resource.take() {
                            if let Some(ref mut vt) = current_vt {
                                vt.resources.push(res);
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(ref e)) => {
                let text = e.unescape().unwrap_or_default().trim().to_string();
                if text.is_empty() {
                    continue;
                }

                let in_resource = current_resource.is_some();
                let in_vt = current_vt.is_some();

                match current_tag.as_str() {
                    "Id" if !in_resource && !in_vt && cpl.id.is_empty() => {
                        cpl.id = text.strip_prefix("urn:uuid:").unwrap_or(&text).to_string();
                    }
                    "Id" if in_vt && !in_resource => {
                        if let Some(ref mut vt) = current_vt {
                            if vt.id.is_empty() {
                                vt.id = text.strip_prefix("urn:uuid:").unwrap_or(&text).to_string();
                            }
                        }
                    }
                    "Id" if in_resource => {
                        if let Some(ref mut res) = current_resource {
                            res.id = text.strip_prefix("urn:uuid:").unwrap_or(&text).to_string();
                        }
                    }
                    "ContentTitle" | "ContentTitleText" => {
                        if cpl.content_title.is_empty() {
                            cpl.content_title = text;
                        }
                    }
                    "EditRate" => {
                        let rate = parse_edit_rate(&text);
                        if in_resource {
                            if let Some(ref mut res) = current_resource {
                                res.edit_rate = rate;
                            }
                        } else if cpl.edit_rate == (0, 0) {
                            cpl.edit_rate = rate;
                        }
                    }
                    "TrackFileId" | "SourceEncoding" => {
                        if let Some(ref mut res) = current_resource {
                            if res.track_file_id.is_empty() {
                                res.track_file_id =
                                    text.strip_prefix("urn:uuid:").unwrap_or(&text).to_string();
                            }
                        }
                    }
                    "IntrinsicDuration" => {
                        if let Some(ref mut res) = current_resource {
                            res.intrinsic_duration = text.parse().unwrap_or(0);
                        }
                    }
                    "EntryPoint" => {
                        if let Some(ref mut res) = current_resource {
                            res.entry_point = text.parse().unwrap_or(0);
                        }
                    }
                    "SourceDuration" => {
                        if let Some(ref mut res) = current_resource {
                            res.source_duration = text.parse().unwrap_or(0);
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("XML parse error: {e}")),
            _ => {}
        }
    }

    // Calculate total duration
    for vt in &cpl.virtual_tracks {
        if vt.track_type == TrackType::MainImage {
            cpl.total_duration = vt.resources.iter().map(|r| r.effective_duration()).sum();
            break;
        }
    }

    Ok(cpl)
}

fn detect_application(namespaces: &[String], xml: &str) -> ImfApplication {
    for ns in namespaces {
        if ns.contains("2067-21") || ns == NS_APP2E || ns == NS_APP2E_2016 {
            return ImfApplication::App2e;
        }
        if ns.contains("2067-50") || ns == NS_APP5 {
            return ImfApplication::App5Aces;
        }
    }
    if xml.contains("2067-21") {
        return ImfApplication::App2e;
    }
    if xml.contains("2067-50") {
        return ImfApplication::App5Aces;
    }
    ImfApplication::Unknown
}

fn parse_edit_rate(s: &str) -> (u32, u32) {
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() == 2 {
        let num = parts[0].parse().unwrap_or(0);
        let den = parts[1].parse().unwrap_or(0);
        (num, den)
    } else if let Some((n, d)) = s.split_once('/') {
        (n.trim().parse().unwrap_or(0), d.trim().parse().unwrap_or(0))
    } else {
        (0, 0)
    }
}
