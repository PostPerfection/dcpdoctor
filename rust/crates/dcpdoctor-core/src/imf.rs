//! Native IMF (Interoperable Master Format) validation.
//!
//! Implements key checks from the ST 2067 family of standards:
//! - Application identification and namespace validation
//! - Virtual track continuity and completeness
//! - CPL/PKL/AssetMap cross-referencing
//! - Essence descriptor constraints per-application profile
//! - EssenceDescriptorList cross-referencing
//! - ContentKind, IssueDate, UUID validation
//! - Segment/Sequence structural constraints
//! - Marker sequence validation
//! - TTML subtitle track validation
//! - IAB (Immersive Audio) track support
//! - App 5 ACES colour constraints
//! - PKL ↔ CPL cross-referencing
//!
//! These checks complement (and eventually replace) the external Photon dependency.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use quick_xml::Reader;
use quick_xml::events::Event;

use crate::{Code, Note, Severity};

// ─── IMF Namespaces ────────────────────────────────────────────────────────────

const NS_CORE_2013: &str = "http://www.smpte-ra.org/schemas/2067-2/2013";
const NS_CORE_2016: &str = "http://www.smpte-ra.org/schemas/2067-2/2016";
const NS_APP2E: &str = "http://www.smpte-ra.org/ns/2067-21/2021";
const NS_APP2E_2016: &str = "http://www.smpte-ra.org/ns/2067-21/2016";
const NS_APP5: &str = "http://www.smpte-ra.org/ns/2067-50/2017";

// ─── Application profiles ──────────────────────────────────────────────────────

/// Known IMF Application profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImfApplication {
    App2e,
    App5Aces,
    #[default]
    Unknown,
}

impl std::fmt::Display for ImfApplication {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImfApplication::App2e => write!(f, "Application 2E (ST 2067-21)"),
            ImfApplication::App5Aces => write!(f, "Application 5 ACES (ST 2067-50)"),
            ImfApplication::Unknown => write!(f, "Unknown Application"),
        }
    }
}

// ─── CPL extended parsing for IMF ─────────────────────────────────────────────

/// IMF Composition Playlist with extended metadata.
#[derive(Debug, Clone, Default)]
pub struct ImfCpl {
    pub id: String,
    pub content_title: String,
    pub edit_rate: (u32, u32),
    pub namespaces: Vec<String>,
    pub application: ImfApplication,
    pub virtual_tracks: Vec<VirtualTrack>,
    pub total_duration: u64,
    pub issue_date: String,
    pub content_kind: String,
    pub annotation: String,
    pub creator: String,
    pub issuer: String,
    /// EssenceDescriptorList: maps TrackFileId → EssenceDescriptor element data.
    pub essence_descriptors: HashMap<String, EssenceDescriptor>,
    /// All UUIDs found in the CPL (for uniqueness checking).
    pub all_uuids: Vec<String>,
    /// Number of Segments in the CPL.
    pub segment_count: u32,
    /// Marker annotations (label, offset) per MarkerSequence.
    pub markers: Vec<Marker>,
}

/// Parsed essence descriptor from EssenceDescriptorList.
#[derive(Debug, Clone, Default)]
pub struct EssenceDescriptor {
    pub id: String,
    /// The track file ID this descriptor references
    pub linked_track_file_id: String,
    /// Descriptor type (e.g., "CDCIDescriptor", "WaveAudioDescriptor", "TimedTextDescriptor")
    pub descriptor_type: String,
    /// Container duration
    pub container_duration: u64,
    /// Sample rate (for audio)
    pub sample_rate: (u32, u32),
    /// Stored width (for picture)
    pub stored_width: u32,
    /// Stored height (for picture)
    pub stored_height: u32,
    /// Frame layout (0=full, 1=separate fields, 2=single field, 3=mixed, 4=segmented)
    pub frame_layout: u8,
    /// Color primaries UL
    pub color_primaries: String,
    /// Transfer characteristic (OETF) UL
    pub transfer_characteristic: String,
    /// Coding equations UL
    pub coding_equations: String,
    /// Component depth (bit depth)
    pub component_depth: u32,
    /// Quantization bits (audio)
    pub quantization_bits: u32,
    /// Audio channel count
    pub channel_count: u32,
    /// Audio sampling rate
    pub audio_sampling_rate: (u32, u32),
}

/// A marker annotation within a MarkerSequence.
#[derive(Debug, Clone, Default)]
pub struct Marker {
    pub label: String,
    pub scope: String,
    pub offset: u64,
}

/// A virtual track (sequence) in the IMF CPL.
#[derive(Debug, Clone, Default)]
pub struct VirtualTrack {
    pub id: String,
    pub track_type: TrackType,
    pub resources: Vec<TrackResource>,
}

/// Type of virtual track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrackType {
    #[default]
    MainImage,
    MainAudio,
    Subtitle,
    HearingImpaired,
    VisuallyImpaired,
    Commentary,
    Karaoke,
    ForcedNarrative,
    IAB,
    Marker,
    Other,
}

/// A resource (segment) within a virtual track.
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
    /// Effective duration (source_duration if set, else intrinsic - entry_point).
    pub fn effective_duration(&self) -> u64 {
        if self.source_duration > 0 {
            self.source_duration
        } else {
            self.intrinsic_duration.saturating_sub(self.entry_point)
        }
    }
}

// ─── IMF Validation ────────────────────────────────────────────────────────────

/// Validate an IMP (Interoperable Master Package) directory.
/// Returns a list of validation notes.
pub fn validate_imp(imp_dir: &Path) -> Vec<Note> {
    let mut notes = Vec::new();

    // Find CPL files
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

        let cpl = match parse_imf_cpl(&xml) {
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

        // Application identification
        validate_application(&cpl, cpl_path, &mut notes);

        // Virtual track continuity
        validate_virtual_tracks(&cpl, cpl_path, &mut notes);

        // Edit rate consistency
        validate_edit_rates(&cpl, cpl_path, &mut notes);

        // Track file references
        validate_track_file_refs(&cpl, imp_dir, cpl_path, &mut notes);

        // Application-specific constraints
        validate_app_constraints(&cpl, cpl_path, &mut notes);

        // Timeline alignment (all virtual tracks same duration)
        validate_timeline_alignment(&cpl, cpl_path, &mut notes);

        // Essence descriptor validation against application profile
        validate_essence_descriptors(&cpl, imp_dir, cpl_path, &mut notes);

        // UUID format and uniqueness
        validate_uuids(&cpl, cpl_path, &mut notes);

        // ContentKind validation
        validate_content_kind(&cpl, cpl_path, &mut notes);

        // IssueDate validation
        validate_issue_date(&cpl, cpl_path, &mut notes);

        // Segment structure
        validate_segment_structure(&cpl, cpl_path, &mut notes);

        // EssenceDescriptorList cross-referencing
        validate_essence_descriptor_list(&cpl, cpl_path, &mut notes);

        // Marker sequences
        validate_markers(&cpl, cpl_path, &mut notes);

        // TTML / subtitle tracks
        validate_ttml_tracks(&cpl, imp_dir, cpl_path, &mut notes);

        // IAB tracks
        validate_iab_tracks(&cpl, cpl_path, &mut notes);
    }

    // PKL ↔ CPL cross-referencing
    validate_pkl_cpl_refs(imp_dir, &cpl_files, &mut notes);

    notes
}

// ─── Sub-validators ────────────────────────────────────────────────────────────

fn validate_application(cpl: &ImfCpl, cpl_path: &Path, notes: &mut Vec<Note>) {
    if cpl.application == ImfApplication::Unknown {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::MissingRequiredElement,
            message: "No recognized IMF Application identified in CPL namespaces".to_string(),
            file: Some(cpl_path.to_path_buf()),
            line: 0,
        });
    }

    // Check for required core constraints namespace
    let has_core = cpl
        .namespaces
        .iter()
        .any(|ns| ns.contains("2067-2") || ns == NS_CORE_2013 || ns == NS_CORE_2016);
    if !has_core {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::SmpteNamespaceWrong,
            message: "CPL missing ST 2067-2 core constraints namespace".to_string(),
            file: Some(cpl_path.to_path_buf()),
            line: 0,
        });
    }
}

fn validate_virtual_tracks(cpl: &ImfCpl, cpl_path: &Path, notes: &mut Vec<Note>) {
    let mut has_main_image = false;

    for vt in &cpl.virtual_tracks {
        if vt.track_type == TrackType::MainImage {
            has_main_image = true;
        }

        if vt.resources.is_empty() {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::CplMissingReel,
                message: format!(
                    "Virtual track {} ({:?}) has no resources",
                    vt.id, vt.track_type
                ),
                file: Some(cpl_path.to_path_buf()),
                line: 0,
            });
            continue;
        }

        // Check for gaps/overlaps — resources must be contiguous
        let total: u64 = vt.resources.iter().map(|r| r.effective_duration()).sum();
        if cpl.total_duration > 0
            && total != cpl.total_duration
            && vt.track_type == TrackType::MainImage
        {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::ReelDiscontinuity,
                message: format!(
                    "MainImage virtual track duration ({}) does not match CPL duration ({})",
                    total, cpl.total_duration
                ),
                file: Some(cpl_path.to_path_buf()),
                line: 0,
            });
        }

        // Entry point must not exceed intrinsic duration
        for res in &vt.resources {
            if res.entry_point >= res.intrinsic_duration && res.intrinsic_duration > 0 {
                notes.push(Note {
                    severity: Severity::Error,
                    code: Code::CplInvalidDuration,
                    message: format!(
                        "Resource {} has entry_point ({}) >= intrinsic_duration ({})",
                        res.id, res.entry_point, res.intrinsic_duration
                    ),
                    file: Some(cpl_path.to_path_buf()),
                    line: 0,
                });
            }
            if res.source_duration > 0
                && res.entry_point + res.source_duration > res.intrinsic_duration
                && res.intrinsic_duration > 0
            {
                notes.push(Note {
                    severity: Severity::Error,
                    code: Code::CplInvalidDuration,
                    message: format!(
                        "Resource {} source range exceeds intrinsic duration ({} + {} > {})",
                        res.id, res.entry_point, res.source_duration, res.intrinsic_duration
                    ),
                    file: Some(cpl_path.to_path_buf()),
                    line: 0,
                });
            }
        }
    }

    if !has_main_image {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::MissingRequiredElement,
            message: "CPL has no MainImageSequence virtual track".to_string(),
            file: Some(cpl_path.to_path_buf()),
            line: 0,
        });
    }
}

fn validate_edit_rates(cpl: &ImfCpl, cpl_path: &Path, notes: &mut Vec<Note>) {
    if cpl.edit_rate == (0, 0) {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::CplInvalidEditRate,
            message: "CPL has no EditRate".to_string(),
            file: Some(cpl_path.to_path_buf()),
            line: 0,
        });
        return;
    }

    // All resources in MainImage must match or be an integer multiple of CPL edit rate
    for vt in &cpl.virtual_tracks {
        if vt.track_type != TrackType::MainImage {
            continue;
        }
        for res in &vt.resources {
            if res.edit_rate == (0, 0) {
                continue;
            }
            // Check if resource edit rate is compatible
            let cpl_fps = cpl.edit_rate.0 as f64 / cpl.edit_rate.1 as f64;
            let res_fps = res.edit_rate.0 as f64 / res.edit_rate.1 as f64;
            let ratio = res_fps / cpl_fps;
            if (ratio - ratio.round()).abs() > 0.001 {
                notes.push(Note {
                    severity: Severity::Error,
                    code: Code::CplInvalidEditRate,
                    message: format!(
                        "Resource edit rate {}/{} is not compatible with CPL edit rate {}/{}",
                        res.edit_rate.0, res.edit_rate.1, cpl.edit_rate.0, cpl.edit_rate.1
                    ),
                    file: Some(cpl_path.to_path_buf()),
                    line: 0,
                });
            }
        }
    }
}

fn validate_track_file_refs(cpl: &ImfCpl, imp_dir: &Path, cpl_path: &Path, notes: &mut Vec<Note>) {
    // Collect all track file IDs referenced in the CPL
    let referenced_ids: HashSet<&str> = cpl
        .virtual_tracks
        .iter()
        .flat_map(|vt| vt.resources.iter())
        .map(|r| r.track_file_id.as_str())
        .filter(|id| !id.is_empty())
        .collect();

    // Try to find referenced MXF files via AssetMap
    let assetmap_path = imp_dir.join("ASSETMAP.xml");
    if !assetmap_path.exists() {
        return; // AssetMap validation handled elsewhere
    }

    let assetmap_xml = match std::fs::read_to_string(&assetmap_path) {
        Ok(s) => s,
        Err(_) => return,
    };

    let asset_ids = parse_assetmap_ids(&assetmap_xml);

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

fn validate_app_constraints(cpl: &ImfCpl, cpl_path: &Path, notes: &mut Vec<Note>) {
    match cpl.application {
        ImfApplication::App2e => validate_app2e(cpl, cpl_path, notes),
        ImfApplication::App5Aces => validate_app5(cpl, cpl_path, notes),
        ImfApplication::Unknown => {}
    }
}

/// Application 2E constraints (ST 2067-21).
fn validate_app2e(cpl: &ImfCpl, cpl_path: &Path, notes: &mut Vec<Note>) {
    // App 2E requires edit rate from a specific set
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
            code: Code::CplInvalidEditRate,
            message: format!(
                "App 2E: invalid composition edit rate {}/{} (ST 2067-21 Section 5.2)",
                cpl.edit_rate.0, cpl.edit_rate.1
            ),
            file: Some(cpl_path.to_path_buf()),
            line: 0,
        });
    }

    // App 2E requires at least one MainImage and one MainAudio track
    let has_audio = cpl
        .virtual_tracks
        .iter()
        .any(|vt| vt.track_type == TrackType::MainAudio);
    if !has_audio {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::MissingRequiredElement,
            message: "App 2E: no MainAudioSequence found (recommended by ST 2067-21)".to_string(),
            file: Some(cpl_path.to_path_buf()),
            line: 0,
        });
    }
}

/// Application 5 ACES constraints (ST 2067-50).
fn validate_app5(cpl: &ImfCpl, cpl_path: &Path, notes: &mut Vec<Note>) {
    // App 5 requires specific edit rates
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
            code: Code::CplInvalidEditRate,
            message: format!(
                "App 5 ACES: invalid composition edit rate {}/{} (ST 2067-50)",
                cpl.edit_rate.0, cpl.edit_rate.1
            ),
            file: Some(cpl_path.to_path_buf()),
            line: 0,
        });
    }
}

/// Validate all virtual tracks cover the same timeline span.
fn validate_timeline_alignment(cpl: &ImfCpl, cpl_path: &Path, notes: &mut Vec<Note>) {
    if cpl.virtual_tracks.is_empty() {
        return;
    }

    // Get MainImage duration as reference
    let image_duration = cpl
        .virtual_tracks
        .iter()
        .find(|vt| vt.track_type == TrackType::MainImage)
        .map(|vt| {
            vt.resources
                .iter()
                .map(|r| r.effective_duration())
                .sum::<u64>()
        });

    let reference_duration = match image_duration {
        Some(d) if d > 0 => d,
        _ => return,
    };

    // All non-marker tracks must match the reference duration
    for vt in &cpl.virtual_tracks {
        if vt.track_type == TrackType::Marker || vt.resources.is_empty() {
            continue;
        }
        let track_duration: u64 = vt.resources.iter().map(|r| r.effective_duration()).sum();
        if track_duration > 0 && track_duration != reference_duration {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::CplMismatchedDurations,
                message: format!(
                    "{:?} track {} duration ({}) differs from MainImage duration ({})",
                    vt.track_type, vt.id, track_duration, reference_duration
                ),
                file: Some(cpl_path.to_path_buf()),
                line: 0,
            });
        }
    }
}

/// Validate MXF essence descriptors against the declared application profile.
fn validate_essence_descriptors(
    cpl: &ImfCpl,
    imp_dir: &Path,
    cpl_path: &Path,
    notes: &mut Vec<Note>,
) {
    // Build AssetMap ID→path mapping
    let assetmap_path = imp_dir.join("ASSETMAP.xml");
    if !assetmap_path.exists() {
        return;
    }
    let assetmap_xml = match std::fs::read_to_string(&assetmap_path) {
        Ok(s) => s,
        Err(_) => return,
    };
    let id_to_path = parse_assetmap_paths(&assetmap_xml);

    // Find all track file IDs referenced in picture/audio sequences
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

            // Only validate MXF files
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

/// Validate picture essence against application constraints.
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
        // ST 2067-21 picture constraints:
        // Resolution: HD (1920x1080), 2K (2048x1080), UHD (3840x2160), 4K (4096x2160)
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

        // Bit depth: 8, 10, or 12 bits per component
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

        // Frame rate must match CPL edit rate
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

/// Validate audio essence against application constraints.
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
        // ST 2067-21 audio constraints:
        // Sample rate: 48000 or 96000 Hz
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

        // Bit depth: 24 bits required
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

        // Channel count: standard configs (mono through 7.1.4)
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

// ─── UUID Validation ───────────────────────────────────────────────────────────

/// Validate all UUIDs in the CPL for format correctness and uniqueness.
fn validate_uuids(cpl: &ImfCpl, cpl_path: &Path, notes: &mut Vec<Note>) {
    let uuid_re = regex_lite::Regex::new(
        r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$",
    )
    .unwrap();

    let mut seen: HashSet<String> = HashSet::new();
    for uuid in &cpl.all_uuids {
        if !uuid_re.is_match(uuid) {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::InvalidUuid,
                message: format!("Malformed UUID: '{uuid}'"),
                file: Some(cpl_path.to_path_buf()),
                line: 0,
            });
        }
        let lower = uuid.to_lowercase();
        if !seen.insert(lower.clone()) {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::DuplicateAssetId,
                message: format!("Duplicate UUID in CPL: '{uuid}'"),
                file: Some(cpl_path.to_path_buf()),
                line: 0,
            });
        }
    }
}

// ─── ContentKind Validation ────────────────────────────────────────────────────

/// Valid ContentKind values per ST 2067-2.
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
    "highlights",
    "event",
    "supplemental",
    "preview",
];

fn validate_content_kind(cpl: &ImfCpl, cpl_path: &Path, notes: &mut Vec<Note>) {
    if cpl.content_kind.is_empty() {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::MissingRequiredElement,
            message: "CPL has no ContentKind element".to_string(),
            file: Some(cpl_path.to_path_buf()),
            line: 0,
        });
        return;
    }
    let kind_lower = cpl.content_kind.to_lowercase();
    if !VALID_CONTENT_KINDS.contains(&kind_lower.as_str()) {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::CplInvalidContentKind,
            message: format!(
                "ContentKind '{}' is not a recognized value (expected one of: {})",
                cpl.content_kind,
                VALID_CONTENT_KINDS.join(", ")
            ),
            file: Some(cpl_path.to_path_buf()),
            line: 0,
        });
    }
}

// ─── IssueDate Validation ──────────────────────────────────────────────────────

fn validate_issue_date(cpl: &ImfCpl, cpl_path: &Path, notes: &mut Vec<Note>) {
    if cpl.issue_date.is_empty() {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::MissingRequiredElement,
            message: "CPL has no IssueDate element".to_string(),
            file: Some(cpl_path.to_path_buf()),
            line: 0,
        });
        return;
    }
    // ISO 8601 format: YYYY-MM-DDThh:mm:ss[.sss][Z|+/-hh:mm]
    let valid = cpl.issue_date.len() >= 19
        && cpl.issue_date.chars().nth(4) == Some('-')
        && cpl.issue_date.chars().nth(7) == Some('-')
        && cpl.issue_date.chars().nth(10) == Some('T')
        && cpl.issue_date.chars().nth(13) == Some(':')
        && cpl.issue_date.chars().nth(16) == Some(':');
    if !valid {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::XmlSchemaViolation,
            message: format!(
                "IssueDate '{}' is not valid ISO 8601 (expected YYYY-MM-DDThh:mm:ss...)",
                cpl.issue_date
            ),
            file: Some(cpl_path.to_path_buf()),
            line: 0,
        });
    }
}

// ─── Segment Structure Validation ──────────────────────────────────────────────

fn validate_segment_structure(cpl: &ImfCpl, cpl_path: &Path, notes: &mut Vec<Note>) {
    if cpl.segment_count == 0 {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::MissingRequiredElement,
            message: "CPL has no Segment elements (at least one required)".to_string(),
            file: Some(cpl_path.to_path_buf()),
            line: 0,
        });
    }
}

// ─── EssenceDescriptorList Cross-referencing ───────────────────────────────────

fn validate_essence_descriptor_list(cpl: &ImfCpl, cpl_path: &Path, notes: &mut Vec<Note>) {
    if cpl.essence_descriptors.is_empty() {
        // EssenceDescriptorList is optional but recommended
        notes.push(Note {
            severity: Severity::Info,
            code: Code::MissingRequiredElement,
            message: "CPL has no EssenceDescriptorList (recommended for interoperability)"
                .to_string(),
            file: Some(cpl_path.to_path_buf()),
            line: 0,
        });
        return;
    }

    // Collect all track file IDs referenced in resources
    let referenced_ids: HashSet<&str> = cpl
        .virtual_tracks
        .iter()
        .flat_map(|vt| vt.resources.iter())
        .map(|r| r.track_file_id.as_str())
        .filter(|id| !id.is_empty())
        .collect();

    // Each referenced track file should have a corresponding EssenceDescriptor
    let descriptor_ids: HashSet<&str> =
        cpl.essence_descriptors.keys().map(|k| k.as_str()).collect();

    for ref_id in &referenced_ids {
        if !descriptor_ids.contains(ref_id) {
            notes.push(Note {
                severity: Severity::Warning,
                code: Code::CrossRefBroken,
                message: format!(
                    "Track file {} referenced in CPL has no matching EssenceDescriptor",
                    ref_id
                ),
                file: Some(cpl_path.to_path_buf()),
                line: 0,
            });
        }
    }

    // Each EssenceDescriptor should be referenced by at least one resource
    for desc_id in descriptor_ids {
        if !referenced_ids.contains(desc_id) {
            notes.push(Note {
                severity: Severity::Warning,
                code: Code::CrossRefBroken,
                message: format!(
                    "EssenceDescriptor for track file {} is not referenced by any resource",
                    desc_id
                ),
                file: Some(cpl_path.to_path_buf()),
                line: 0,
            });
        }
    }

    // Validate descriptor internals per application profile
    if cpl.application == ImfApplication::App2e {
        for (id, desc) in &cpl.essence_descriptors {
            validate_app2e_descriptor(desc, id, cpl_path, notes);
        }
    } else if cpl.application == ImfApplication::App5Aces {
        for (id, desc) in &cpl.essence_descriptors {
            validate_app5_descriptor(desc, id, cpl_path, notes);
        }
    }
}

/// Validate an EssenceDescriptor against App 2E constraints.
fn validate_app2e_descriptor(
    desc: &EssenceDescriptor,
    _id: &str,
    cpl_path: &Path,
    notes: &mut Vec<Note>,
) {
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
            notes.push(Note {
                severity: Severity::Error,
                code: Code::PictureInvalidResolution,
                message: format!(
                    "EssenceDescriptor: invalid resolution {}x{} for App 2E",
                    desc.stored_width, desc.stored_height
                ),
                file: Some(cpl_path.to_path_buf()),
                line: 0,
            });
        }
        if desc.component_depth > 0 && !matches!(desc.component_depth, 8 | 10 | 12) {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::MxfInvalidStructure,
                message: format!(
                    "EssenceDescriptor: invalid bit depth {} for App 2E (allowed: 8, 10, 12)",
                    desc.component_depth
                ),
                file: Some(cpl_path.to_path_buf()),
                line: 0,
            });
        }
    }

    // Audio descriptors
    if desc.descriptor_type.contains("Wave") || desc.descriptor_type.contains("Audio") {
        if desc.audio_sampling_rate.0 > 0 {
            let rate = desc.audio_sampling_rate.0 / desc.audio_sampling_rate.1.max(1);
            if !matches!(rate, 48000 | 96000) {
                notes.push(Note {
                    severity: Severity::Error,
                    code: Code::SoundInvalidSampleRate,
                    message: format!(
                        "EssenceDescriptor: invalid audio sample rate {} for App 2E (allowed: 48000, 96000)",
                        rate
                    ),
                    file: Some(cpl_path.to_path_buf()),
                    line: 0,
                });
            }
        }
        if desc.quantization_bits > 0 && desc.quantization_bits != 24 {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::SoundInvalidChannelCount,
                message: format!(
                    "EssenceDescriptor: invalid audio bit depth {} for App 2E (required: 24)",
                    desc.quantization_bits
                ),
                file: Some(cpl_path.to_path_buf()),
                line: 0,
            });
        }
    }
}

/// Validate an EssenceDescriptor against App 5 ACES constraints.
fn validate_app5_descriptor(
    desc: &EssenceDescriptor,
    _id: &str,
    cpl_path: &Path,
    notes: &mut Vec<Note>,
) {
    // App 5 ACES picture constraints:
    // Must use ACES color primaries (AP0 or AP1)
    // Transfer characteristic must be linear
    if desc.descriptor_type.contains("CDCI")
        || desc.descriptor_type.contains("RGBA")
        || desc.descriptor_type.contains("JPEG2000")
    {
        // SMPTE ST 2065-1 ACES primaries UL: 06.0e.2b.34.04.01.01.0d.04.01.01.01.03.07.00.00
        if !desc.color_primaries.is_empty() {
            let is_aces_primaries = desc.color_primaries.contains("03.07")
                || desc.color_primaries.contains("0307")
                || desc.color_primaries.to_lowercase().contains("aces");
            if !is_aces_primaries {
                notes.push(Note {
                    severity: Severity::Warning,
                    code: Code::MxfInvalidStructure,
                    message: format!(
                        "App 5 ACES: color primaries '{}' may not be ACES (expected AP0/AP1)",
                        desc.color_primaries
                    ),
                    file: Some(cpl_path.to_path_buf()),
                    line: 0,
                });
            }
        }

        // Transfer characteristic should be linear for ACES
        if !desc.transfer_characteristic.is_empty() {
            let is_linear = desc.transfer_characteristic.contains("01.01")
                || desc.transfer_characteristic.contains("0101")
                || desc
                    .transfer_characteristic
                    .to_lowercase()
                    .contains("linear");
            if !is_linear {
                notes.push(Note {
                    severity: Severity::Warning,
                    code: Code::MxfInvalidStructure,
                    message: format!(
                        "App 5 ACES: transfer characteristic '{}' should be linear",
                        desc.transfer_characteristic
                    ),
                    file: Some(cpl_path.to_path_buf()),
                    line: 0,
                });
            }
        }

        // Bit depth should be 16 (half-float) for ACES
        if desc.component_depth > 0 && desc.component_depth != 16 {
            notes.push(Note {
                severity: Severity::Info,
                code: Code::MxfInvalidStructure,
                message: format!(
                    "App 5 ACES: component depth {} (ACES typically uses 16-bit half-float)",
                    desc.component_depth
                ),
                file: Some(cpl_path.to_path_buf()),
                line: 0,
            });
        }
    }
}

// ─── Marker Validation ─────────────────────────────────────────────────────────

/// Known marker labels per ST 2067-3.
const VALID_MARKER_LABELS: &[&str] = &[
    "FFBT", "LFBT", "FFCR", "LFCR", "FFTC", "LFTC", "FFOI", "LFOI", "FFEC", "LFEC", "FFMC", "LFMC",
    "FFOB", "LFOB", "FFHS", "LFHS", "FFSW", "LFSW", "FFBW", "LFBW",
];

fn validate_markers(cpl: &ImfCpl, cpl_path: &Path, notes: &mut Vec<Note>) {
    for marker in &cpl.markers {
        if marker.label.is_empty() {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::MarkerInvalid,
                message: "Marker has empty label".to_string(),
                file: Some(cpl_path.to_path_buf()),
                line: 0,
            });
            continue;
        }

        // Check if label is recognized
        if !VALID_MARKER_LABELS.contains(&marker.label.as_str()) {
            notes.push(Note {
                severity: Severity::Info,
                code: Code::MarkerInvalid,
                message: format!(
                    "Marker label '{}' is not a standard ST 2067-3 marker",
                    marker.label
                ),
                file: Some(cpl_path.to_path_buf()),
                line: 0,
            });
        }

        // Offset must be within composition duration
        if cpl.total_duration > 0 && marker.offset > cpl.total_duration {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::MarkerInvalid,
                message: format!(
                    "Marker '{}' offset ({}) exceeds composition duration ({})",
                    marker.label, marker.offset, cpl.total_duration
                ),
                file: Some(cpl_path.to_path_buf()),
                line: 0,
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
        if let (Some(&ff), Some(&lf)) = (offsets.get(first), offsets.get(last))
            && ff > lf
        {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::MarkerInvalid,
                message: format!(
                    "Marker {} (offset {}) occurs after {} (offset {})",
                    first, ff, last, lf
                ),
                file: Some(cpl_path.to_path_buf()),
                line: 0,
            });
        }
    }
}

// ─── TTML Subtitle Track Validation ────────────────────────────────────────────

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

/// Validate a TTML file for basic structural correctness.
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

    // Check for IMSC1 profile (required for IMF per ST 2067-2)
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

// ─── IAB Track Validation ──────────────────────────────────────────────────────

fn validate_iab_tracks(cpl: &ImfCpl, cpl_path: &Path, notes: &mut Vec<Note>) {
    for vt in &cpl.virtual_tracks {
        if vt.track_type != TrackType::IAB {
            continue;
        }

        if vt.resources.is_empty() {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::CplMissingReel,
                message: format!("IAB virtual track {} has no resources", vt.id),
                file: Some(cpl_path.to_path_buf()),
                line: 0,
            });
            continue;
        }

        // IAB tracks must have same edit rate as MainImage
        let image_rate = cpl
            .virtual_tracks
            .iter()
            .find(|v| v.track_type == TrackType::MainImage)
            .and_then(|v| v.resources.first())
            .map(|r| r.edit_rate)
            .unwrap_or(cpl.edit_rate);

        for res in &vt.resources {
            if res.edit_rate != (0, 0) && res.edit_rate != image_rate && image_rate != (0, 0) {
                notes.push(Note {
                    severity: Severity::Error,
                    code: Code::CplInvalidEditRate,
                    message: format!(
                        "IAB track edit rate {}/{} must match MainImage edit rate {}/{}",
                        res.edit_rate.0, res.edit_rate.1, image_rate.0, image_rate.1
                    ),
                    file: Some(cpl_path.to_path_buf()),
                    line: 0,
                });
            }
        }

        let iab_duration: u64 = vt.resources.iter().map(|r| r.effective_duration()).sum();
        if cpl.total_duration > 0 && iab_duration > 0 && iab_duration != cpl.total_duration {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::CplMismatchedDurations,
                message: format!(
                    "IAB track duration ({}) differs from composition duration ({})",
                    iab_duration, cpl.total_duration
                ),
                file: Some(cpl_path.to_path_buf()),
                line: 0,
            });
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

    // Each CPL must be listed in a PKL
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

    // Validate MIME types in PKL against file extensions
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

/// Find CPL XML files in an IMP directory.
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
        // Quick check if it's a CPL
        if let Ok(content) = std::fs::read_to_string(&path)
            && content.contains("CompositionPlaylist")
        {
            cpls.push(path);
        }
    }
    cpls
}

/// Parse asset IDs from an ASSETMAP.xml.
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

/// Parse asset ID→path mapping from an ASSETMAP.xml.
fn parse_assetmap_paths(xml: &str) -> std::collections::HashMap<String, String> {
    use std::collections::HashMap;
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
                let text = e.unescape().unwrap_or_default().trim().to_string();
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

/// Find PKL files in an IMP directory.
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
                let text = e.unescape().unwrap_or_default().trim().to_string();
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
                let text = e.unescape().unwrap_or_default().trim().to_string();
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

/// Parse an IMF CPL from XML, extracting extended metadata.
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
                    // Collect namespace declarations
                    for attr in e.attributes().flatten() {
                        let value = String::from_utf8_lossy(attr.value.as_ref()).to_string();
                        if value.contains("smpte-ra.org") || value.contains("2067") {
                            cpl.namespaces.push(value);
                        }
                    }
                    break;
                }
            }
            Ok(Event::Eof) => {
                return Err("No CompositionPlaylist element found".to_string());
            }
            Err(e) => return Err(format!("XML parse error: {e}")),
            _ => {}
        }
    }

    // Determine application from namespaces
    cpl.application = detect_application(&cpl.namespaces, xml);

    // Full parse for structure
    let mut reader = Reader::from_str(xml);
    let mut tag_stack: Vec<String> = Vec::new();
    let mut current_vt: Option<VirtualTrack> = None;
    let mut current_resource: Option<TrackResource> = None;
    let mut current_tag = String::new();
    let mut current_marker: Option<Marker> = None;
    let mut in_essence_descriptor_list = false;
    let mut current_ed: Option<EssenceDescriptor> = None;
    let mut current_ed_tag = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                tag_stack.push(name.clone());
                current_tag = name.clone();

                match name.as_str() {
                    "Segment" => {
                        cpl.segment_count += 1;
                    }
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
                    "SubtitlesSequence" | "TimedTextSequence" => {
                        current_vt = Some(VirtualTrack {
                            track_type: TrackType::Subtitle,
                            ..Default::default()
                        });
                    }
                    "HearingImpairedCaptionsSequence" => {
                        current_vt = Some(VirtualTrack {
                            track_type: TrackType::HearingImpaired,
                            ..Default::default()
                        });
                    }
                    "VisuallyImpairedTextSequence" => {
                        current_vt = Some(VirtualTrack {
                            track_type: TrackType::VisuallyImpaired,
                            ..Default::default()
                        });
                    }
                    "CommentarySequence" => {
                        current_vt = Some(VirtualTrack {
                            track_type: TrackType::Commentary,
                            ..Default::default()
                        });
                    }
                    "KaraokeSequence" => {
                        current_vt = Some(VirtualTrack {
                            track_type: TrackType::Karaoke,
                            ..Default::default()
                        });
                    }
                    "ForcedNarrativeSequence" => {
                        current_vt = Some(VirtualTrack {
                            track_type: TrackType::ForcedNarrative,
                            ..Default::default()
                        });
                    }
                    "IABSequence" | "ImmersiveAudioSequence" => {
                        current_vt = Some(VirtualTrack {
                            track_type: TrackType::IAB,
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
                    "Marker" => {
                        current_marker = Some(Marker::default());
                    }
                    "EssenceDescriptorList" => {
                        in_essence_descriptor_list = true;
                    }
                    "EssenceDescriptor" if in_essence_descriptor_list => {
                        current_ed = Some(EssenceDescriptor::default());
                    }
                    _ => {
                        // Detect descriptor type within EssenceDescriptor
                        if let Some(ref mut ed) = current_ed
                            && ed.descriptor_type.is_empty()
                            && (name.contains("Descriptor")
                                || name.contains("JPEG2000")
                                || name.contains("CDCI")
                                || name.contains("RGBA")
                                || name.contains("Wave")
                                || name.contains("TimedText"))
                        {
                            ed.descriptor_type = name.clone();
                        }
                        if in_essence_descriptor_list && current_ed.is_some() {
                            current_ed_tag = name.clone();
                        }
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                tag_stack.pop();

                match name.as_str() {
                    "MainImageSequence"
                    | "MainAudioSequence"
                    | "SubtitlesSequence"
                    | "TimedTextSequence"
                    | "HearingImpairedCaptionsSequence"
                    | "VisuallyImpairedTextSequence"
                    | "CommentarySequence"
                    | "KaraokeSequence"
                    | "ForcedNarrativeSequence"
                    | "IABSequence"
                    | "ImmersiveAudioSequence"
                    | "MarkerSequence" => {
                        if let Some(vt) = current_vt.take() {
                            cpl.virtual_tracks.push(vt);
                        }
                    }
                    "Resource" => {
                        if let Some(res) = current_resource.take()
                            && let Some(ref mut vt) = current_vt
                        {
                            vt.resources.push(res);
                        }
                    }
                    "Marker" => {
                        if let Some(m) = current_marker.take() {
                            cpl.markers.push(m);
                        }
                    }
                    "EssenceDescriptorList" => {
                        in_essence_descriptor_list = false;
                    }
                    "EssenceDescriptor" if in_essence_descriptor_list => {
                        if let Some(ed) = current_ed.take() {
                            if !ed.linked_track_file_id.is_empty() {
                                cpl.essence_descriptors
                                    .insert(ed.linked_track_file_id.clone(), ed);
                            } else if !ed.id.is_empty() {
                                cpl.essence_descriptors.insert(ed.id.clone(), ed);
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

                // Collect UUIDs from all Id elements
                if current_tag == "Id" {
                    let uuid_val = text.strip_prefix("urn:uuid:").unwrap_or(&text).to_string();
                    if !uuid_val.is_empty() {
                        cpl.all_uuids.push(uuid_val.clone());
                    }
                }

                // EssenceDescriptor fields
                if let Some(ref mut ed) = current_ed {
                    match current_ed_tag.as_str() {
                        "Id" if ed.id.is_empty() => {
                            ed.id = text.strip_prefix("urn:uuid:").unwrap_or(&text).to_string();
                        }
                        "TrackFileId" | "LinkedTrackFileId" => {
                            ed.linked_track_file_id =
                                text.strip_prefix("urn:uuid:").unwrap_or(&text).to_string();
                        }
                        "ContainerDuration" => {
                            ed.container_duration = text.parse().unwrap_or(0);
                        }
                        "StoredWidth" => {
                            ed.stored_width = text.parse().unwrap_or(0);
                        }
                        "StoredHeight" => {
                            ed.stored_height = text.parse().unwrap_or(0);
                        }
                        "FrameLayout" => {
                            ed.frame_layout = text.parse().unwrap_or(0);
                        }
                        "ComponentDepth" => {
                            ed.component_depth = text.parse().unwrap_or(0);
                        }
                        "QuantizationBits" => {
                            ed.quantization_bits = text.parse().unwrap_or(0);
                        }
                        "ChannelCount" | "AudioChannelCount" => {
                            ed.channel_count = text.parse().unwrap_or(0);
                        }
                        "ColorPrimaries" => {
                            ed.color_primaries = text;
                        }
                        "TransferCharacteristic" => {
                            ed.transfer_characteristic = text;
                        }
                        "CodingEquations" => {
                            ed.coding_equations = text;
                        }
                        "SampleRate" | "AudioSamplingRate" => {
                            ed.audio_sampling_rate = parse_edit_rate(&text);
                        }
                        _ => {}
                    }
                    continue;
                }

                // Marker fields
                if let Some(ref mut m) = current_marker {
                    match current_tag.as_str() {
                        "Label" => m.label = text,
                        "Scope" => m.scope = text,
                        "Offset" => m.offset = text.parse().unwrap_or(0),
                        _ => {}
                    }
                    continue;
                }

                let in_resource = current_resource.is_some();
                let in_vt = current_vt.is_some();

                match current_tag.as_str() {
                    "Id" if !in_resource && !in_vt && cpl.id.is_empty() => {
                        cpl.id = text.strip_prefix("urn:uuid:").unwrap_or(&text).to_string();
                    }
                    "Id" if in_vt && !in_resource => {
                        if let Some(ref mut vt) = current_vt
                            && vt.id.is_empty()
                        {
                            vt.id = text.strip_prefix("urn:uuid:").unwrap_or(&text).to_string();
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
                    "IssueDate" => {
                        if cpl.issue_date.is_empty() {
                            cpl.issue_date = text;
                        }
                    }
                    "ContentKind" => {
                        if cpl.content_kind.is_empty() {
                            cpl.content_kind = text;
                        }
                    }
                    "Annotation" | "AnnotationText" => {
                        if cpl.annotation.is_empty() {
                            cpl.annotation = text;
                        }
                    }
                    "Creator" => {
                        if cpl.creator.is_empty() {
                            cpl.creator = text;
                        }
                    }
                    "Issuer" => {
                        if cpl.issuer.is_empty() {
                            cpl.issuer = text;
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
                        if let Some(ref mut res) = current_resource
                            && res.track_file_id.is_empty()
                        {
                            res.track_file_id =
                                text.strip_prefix("urn:uuid:").unwrap_or(&text).to_string();
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

    // Calculate total duration from MainImage track
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
    // Also check in the body for ApplicationIdentification elements
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
        let num = n.trim().parse().unwrap_or(0);
        let den = d.trim().parse().unwrap_or(0);
        (num, den)
    } else {
        (0, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_edit_rate() {
        assert_eq!(parse_edit_rate("24 1"), (24, 1));
        assert_eq!(parse_edit_rate("24000 1001"), (24000, 1001));
        assert_eq!(parse_edit_rate("24/1"), (24, 1));
        assert_eq!(parse_edit_rate(""), (0, 0));
    }

    #[test]
    fn test_detect_application() {
        let ns = vec!["http://www.smpte-ra.org/ns/2067-21/2021".to_string()];
        assert_eq!(detect_application(&ns, ""), ImfApplication::App2e);

        let ns = vec!["http://www.smpte-ra.org/ns/2067-50/2017".to_string()];
        assert_eq!(detect_application(&ns, ""), ImfApplication::App5Aces);

        let ns = vec!["http://www.smpte-ra.org/schemas/2067-2/2016".to_string()];
        assert_eq!(detect_application(&ns, ""), ImfApplication::Unknown);
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

        let cpl = parse_imf_cpl(xml).unwrap();
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
