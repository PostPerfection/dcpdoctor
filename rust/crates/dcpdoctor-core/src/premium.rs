//! Premium DCP/IMF validation: TTML/IMSC subtitles, Dolby Vision, Atmos IAB,
//! HDR metadata, Netflix delivery, ProRes, extended HFR, accessibility, and
//! content fingerprinting.

use std::path::Path;

use crate::{Code, Note, Severity};

// ════════════════════════════════════════════════════════════════════════════════
// 1. TTML / IMSC Subtitle Validation
// ════════════════════════════════════════════════════════════════════════════════

/// A single TTML timing entry.
#[derive(Debug, Clone, Default)]
pub struct TtmlTimingEntry {
    pub begin: String,
    pub end: String,
    pub region: String,
    pub text_content: String,
    pub line_number: u32,
}

/// TTML file analysis result.
#[derive(Debug, Clone, Default)]
pub struct TtmlInfo {
    pub valid: bool,
    pub profile: String,
    pub language: String,
    pub subtitle_count: usize,
    pub region_count: u32,
    pub has_style_refs: bool,
    pub has_timing_errors: bool,
    pub entries: Vec<TtmlTimingEntry>,
    pub error: Option<String>,
}

/// Validate a TTML/IMSC subtitle file.
pub fn validate_ttml(ttml_path: &Path) -> TtmlInfo {
    let mut info = TtmlInfo::default();

    let content = match std::fs::read_to_string(ttml_path) {
        Ok(c) => c,
        Err(_) => {
            info.error = Some("Failed to read TTML file".into());
            return info;
        }
    };

    // Check root element
    if !content.contains("<tt") {
        info.error = Some("Not a TTML document".into());
        return info;
    }

    // Detect profile from namespace or ttp:profile attribute
    let profile_re = regex_lite::Regex::new(r#"profile="([^"]+)"#).unwrap();
    if let Some(cap) = profile_re.captures(&content) {
        info.profile = cap[1].to_string();
    } else if content.contains("imsc") {
        info.profile = "imsc1".into();
    } else if content.contains("smpte") {
        info.profile = "smpte-tt".into();
    }

    // Language
    let lang_re = regex_lite::Regex::new(r#"(?:xml:lang|lang)="([^"]+)"#).unwrap();
    if let Some(cap) = lang_re.captures(&content) {
        info.language = cap[1].to_string();
    }

    // Count regions
    let region_re = regex_lite::Regex::new(r"<region\b").unwrap();
    info.region_count = region_re.find_iter(&content).count() as u32;

    // Check for styling
    if content.contains("<styling") {
        info.has_style_refs = true;
    }

    // Parse timing entries (p and span elements with begin/end)
    let entry_re =
        regex_lite::Regex::new(r#"<(?:p|span)\b[^>]*begin="([^"]*)"[^>]*end="([^"]*)"[^>]*>"#)
            .unwrap();
    for cap in entry_re.captures_iter(&content) {
        let entry = TtmlTimingEntry {
            begin: cap[1].to_string(),
            end: cap[2].to_string(),
            ..Default::default()
        };
        info.entries.push(entry);
    }

    info.subtitle_count = info.entries.len();

    // Check timing order
    for entry in &info.entries {
        let begin = parse_ttml_time(&entry.begin);
        let end = parse_ttml_time(&entry.end);
        if begin >= 0.0 && end >= 0.0 && begin >= end {
            info.has_timing_errors = true;
            break;
        }
    }

    info.valid = true;
    info
}

/// Check IMSC compliance for a TTML file.
pub fn check_imsc_compliance(info: &TtmlInfo, ttml_path: &Path) -> Vec<Note> {
    let mut notes = Vec::new();
    let file = Some(ttml_path.to_path_buf());

    if !info.valid {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::SubtitleParseError,
            message: format!(
                "TTML parse error: {}",
                info.error.as_deref().unwrap_or("unknown")
            ),
            file,
            line: 0,
        });
        return notes;
    }

    notes.push(Note {
        severity: Severity::Info,
        code: Code::SubtitleParseError,
        message: format!(
            "TTML: {} subtitles, profile: {}",
            info.subtitle_count,
            if info.profile.is_empty() {
                "unknown"
            } else {
                &info.profile
            }
        ),
        file: file.clone(),
        line: 0,
    });

    if info.has_timing_errors {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::SubtitleInvalidTiming,
            message: "TTML has timing errors (begin >= end)".into(),
            file: file.clone(),
            line: 0,
        });
    }

    if info.language.is_empty() {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::SubtitleParseError,
            message: "TTML missing xml:lang attribute".into(),
            file: file.clone(),
            line: 0,
        });
    }

    if info.region_count == 0 {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::SubtitleParseError,
            message: "TTML has no region definitions".into(),
            file: file.clone(),
            line: 0,
        });
    }

    // IMSC-specific checks
    if info.profile.contains("imsc") && info.subtitle_count > 0 && info.region_count == 0 {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::SubtitleParseError,
            message: "IMSC requires at least one region definition".into(),
            file,
            line: 0,
        });
    }

    notes
}

// ════════════════════════════════════════════════════════════════════════════════
// 2. Dolby Vision RPU Metadata
// ════════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq)]
pub struct DolbyVisionMetadata {
    pub profile: u8,
    pub frames: usize,
    pub max_content_light_level_nits: Option<f32>,
    pub max_frame_average_light_level_nits: Option<f32>,
    pub peak_luminance_nits: f32,
    // postkit's reason this RPU cannot be decoded back to RGB, profile 5 only
    pub undecodable_reason: Option<String>,
}

pub fn parse_dolby_vision(track_file: &Path) -> Result<Option<DolbyVisionMetadata>, String> {
    let Some(summary) = postkit::dolby_vision::read_dolby_vision(track_file)? else {
        return Ok(None);
    };

    Ok(Some(DolbyVisionMetadata {
        profile: summary.profile,
        frames: summary.frames,
        max_content_light_level_nits: summary.max_content_light_level_nits,
        max_frame_average_light_level_nits: summary.max_frame_average_light_level_nits,
        peak_luminance_nits: summary.peak_luminance_nits,
        undecodable_reason: postkit::dolby_vision::refuse_undecodable_dolby_vision(&summary).err(),
    }))
}

pub fn check_dolby_vision_compliance(dv: &DolbyVisionMetadata, source: &Path) -> Vec<Note> {
    let file = Some(source.to_path_buf());
    let light_levels = match (
        dv.max_content_light_level_nits,
        dv.max_frame_average_light_level_nits,
    ) {
        (None, None) => "no level 6 block, so no MaxCLL or MaxFALL".to_string(),
        (max_cll, max_fall) => format!(
            "MaxCLL {}, MaxFALL {}",
            light_level_nits(max_cll),
            light_level_nits(max_fall)
        ),
    };

    let mut notes = vec![Note {
        severity: Severity::Info,
        code: Code::HdrMetadataSummary,
        message: format!(
            "Dolby Vision RPU: profile {}, {} frames, {light_levels}, peak {:.0} nits",
            dv.profile, dv.frames, dv.peak_luminance_nits
        ),
        file: file.clone(),
        line: 0,
    }];

    if let Some(reason) = &dv.undecodable_reason {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::HdrMetadataInvalid,
            message: format!("Dolby Vision profile {}: {reason}", dv.profile),
            file,
            line: 0,
        });
    }

    notes
}

fn light_level_nits(value: Option<f32>) -> String {
    match value {
        Some(nits) => format!("{nits:.0} nits"),
        None => "unset".to_string(),
    }
}

// ════════════════════════════════════════════════════════════════════════════════
// 3. Dolby Atmos IAB Deep Inspection
// ════════════════════════════════════════════════════════════════════════════════

// which immersive audio wrapping a track file carries, told apart by its essence
// descriptor: ffprobe lists no stream at all for either one
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImmersiveEssence {
    #[default]
    None,
    // ST 429-18 Dolby Atmos data essence in a DCP
    DolbyAtmos,
    // ST 2067-201 IAB essence in an IMP
    Iab,
}

#[derive(Debug, Clone, Default)]
pub struct AtmosIabInfo {
    pub essence: ImmersiveEssence,
    // None for IAB, whose descriptor carries no object count
    pub object_count: Option<u16>,
    pub channel_count: Option<u16>,
    pub version: Option<u8>,
    pub frame_count: u32,
    // why the descriptor could not be read, empty when it was
    pub error: String,
}

impl AtmosIabInfo {
    pub fn detected(&self) -> bool {
        self.essence != ImmersiveEssence::None
    }
}

// ST 429-18 caps an Atmos track file at 118 objects beside its 10-channel bed
const MAX_ATMOS_OBJECTS: u16 = 118;

pub fn parse_atmos_iab(mxf_path: &Path) -> AtmosIabInfo {
    let mut info = AtmosIabInfo::default();

    let Some(path) = mxf_path.to_str() else {
        return info;
    };
    let essence = match asdcplib::essence_type(path) {
        Ok(essence) => essence,
        Err(_) => return info,
    };

    match essence {
        asdcplib::EssenceType::As02Iab => {
            info.essence = ImmersiveEssence::Iab;
        }
        asdcplib::EssenceType::DcDataDolbyAtmos => {
            info.essence = ImmersiveEssence::DolbyAtmos;
            let mut reader = asdcplib::atmos::MxfReader::new();
            if let Err(e) = reader.open_read(path) {
                info.error = e.to_string();
                return info;
            }
            match reader.atmos_descriptor() {
                Ok(descriptor) => {
                    info.object_count = Some(descriptor.max_object_count);
                    info.channel_count = Some(descriptor.max_channel_count);
                    info.version = Some(descriptor.atmos_version);
                    info.frame_count = descriptor.container_duration;
                }
                Err(e) => info.error = e.to_string(),
            }
            let _ = reader.close();
        }
        _ => {}
    }

    info
}

pub fn check_atmos_compliance(info: &AtmosIabInfo, source: &Path) -> Vec<Note> {
    let mut notes = Vec::new();
    if !info.detected() {
        return notes;
    }

    let file = Some(source.to_path_buf());

    if !info.error.is_empty() {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::CheckSkipped,
            message: format!(
                "Immersive audio essence detected, its descriptor did not read: {}",
                info.error
            ),
            file,
            line: 0,
        });
        return notes;
    }

    match info.essence {
        ImmersiveEssence::DolbyAtmos => {
            notes.push(Note {
                severity: Severity::Info,
                code: Code::SoundInvalidChannelCount,
                message: format!(
                    "Dolby Atmos (ST 429-18): {} objects, {} channels, version {}, {} frames",
                    info.object_count.unwrap_or(0),
                    info.channel_count.unwrap_or(0),
                    info.version.unwrap_or(0),
                    info.frame_count
                ),
                file: file.clone(),
                line: 0,
            });
        }
        ImmersiveEssence::Iab => {
            notes.push(Note {
                severity: Severity::Info,
                code: Code::SoundInvalidChannelCount,
                message: "Immersive audio (IAB, ST 2067-201) essence detected, its descriptor carries no object count".into(),
                file: file.clone(),
                line: 0,
            });
        }
        ImmersiveEssence::None => {}
    }

    if let Some(objects) = info.object_count
        && objects > MAX_ATMOS_OBJECTS
    {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::SoundInvalidChannelCount,
            message: format!(
                "Dolby Atmos exceeds the maximum object count ({MAX_ATMOS_OBJECTS}), declares {objects}"
            ),
            file,
            line: 0,
        });
    }

    notes
}

// ════════════════════════════════════════════════════════════════════════════════
// 4. HDR Metadata (ST 2098)
// ════════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HdrType {
    #[default]
    None,
    Pq,
    Hlg,
    Hdr10,
    Hdr10Plus,
    DolbyVision,
}

#[derive(Debug, Clone, Default)]
pub struct HdrMetadata {
    pub detected: bool,
    pub hdr_type: HdrType,
    pub transfer_function: String,
    pub color_primaries: String,
    pub master_display_max: f64,
    pub master_display_min: f64,
    // set when the descriptor was read rather than ffprobe's stream metadata
    pub from_descriptor: bool,
}

// ST 2067-21 clause 7.5 content light levels, which a CPL carries and no essence
// descriptor asdcplib writes or reads has room for
#[derive(Debug, Clone, Copy, Default)]
pub struct ContentLightLevels {
    pub max_cll: u32,
    pub max_fall: u32,
}

// ST 2067-21:2020 TransferCharacteristic_HLG_OETF, absent from asdcplib's exports
pub const TRANSFER_CHARACTERISTIC_HLG: [u8; 16] = [
    0x06, 0x0e, 0x2b, 0x34, 0x04, 0x01, 0x01, 0x0d, 0x04, 0x01, 0x01, 0x01, 0x01, 0x0b, 0x00, 0x00,
];

// ST 2086 luminance is carried in units of 0.0001 cd/m2
const LUMINANCE_UNITS_PER_NIT: f64 = 10_000.0;

pub fn detect_hdr_metadata(mxf_path: &Path) -> HdrMetadata {
    match hdr_from_descriptor(mxf_path) {
        Some(hdr) => hdr,
        None => hdr_from_ffprobe(mxf_path),
    }
}

// the picture essence descriptor carries the transfer and primaries as ULs, so
// it settles the type without ffprobe having to map them first
fn hdr_from_descriptor(mxf_path: &Path) -> Option<HdrMetadata> {
    let path = mxf_path.to_str()?;
    let descriptor = read_dcp_hdr_descriptor(path).or_else(|| read_imf_hdr_descriptor(path))?;

    let mut hdr = HdrMetadata {
        from_descriptor: true,
        ..Default::default()
    };

    match descriptor.transfer_characteristic {
        Some(asdcplib::jp2k::TRANSFER_CHARACTERISTIC_ST2084) => {
            hdr.detected = true;
            hdr.hdr_type = HdrType::Pq;
            hdr.transfer_function = "PQ (SMPTE ST 2084)".into();
        }
        Some(TRANSFER_CHARACTERISTIC_HLG) => {
            hdr.detected = true;
            hdr.hdr_type = HdrType::Hlg;
            hdr.transfer_function = "HLG (ARIB STD-B67)".into();
        }
        _ => {}
    }

    hdr.color_primaries = match descriptor.color_primaries {
        Some(asdcplib::jp2k::COLOR_PRIMARIES_BT2020) => "BT.2020".into(),
        Some(asdcplib::jp2k::COLOR_PRIMARIES_P3D65) => "P3-D65".into(),
        Some(asdcplib::jp2k::COLOR_PRIMARIES_BT709) => "BT.709".into(),
        _ => String::new(),
    };

    if let Some(max) = descriptor.mastering_display_max_luminance {
        hdr.master_display_max = max as f64 / LUMINANCE_UNITS_PER_NIT;
    }
    if let Some(min) = descriptor.mastering_display_min_luminance {
        hdr.master_display_min = min as f64 / LUMINANCE_UNITS_PER_NIT;
    }

    Some(hdr)
}

fn read_dcp_hdr_descriptor(path: &str) -> Option<asdcplib::jp2k::HdrMetadata> {
    let mut reader = asdcplib::jp2k::MxfReader::new();
    reader.open_read(path).ok()?;
    let hdr = reader.hdr_metadata().ok();
    let _ = reader.close();
    hdr
}

fn read_imf_hdr_descriptor(path: &str) -> Option<asdcplib::jp2k::HdrMetadata> {
    let mut reader = asdcplib::as02::jp2k::MxfReader::new();
    reader.open_read(path).ok()?;
    let hdr = reader.hdr_metadata().ok();
    let _ = reader.close();
    hdr
}

// anything that is not a JPEG 2000 track file still answers through ffprobe
fn hdr_from_ffprobe(mxf_path: &Path) -> HdrMetadata {
    let mut hdr = HdrMetadata::default();

    let output = run_ffprobe(
        &[
            "-v",
            "quiet",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=color_transfer,color_primaries,color_space,bits_per_raw_sample",
            "-show_entries",
            "side_data=side_data_type,max_content,max_average,min_luminance,max_luminance",
            "-of",
            "json",
        ],
        mxf_path,
    );

    if output.is_empty() {
        return hdr;
    }

    let transfer_re = regex_lite::Regex::new(r#""color_transfer"\s*:\s*"([^"]+)""#).unwrap();
    if let Some(cap) = transfer_re.captures(&output) {
        match &cap[1] {
            "smpte2084" | "smpte-st-2084" => {
                hdr.detected = true;
                hdr.hdr_type = HdrType::Pq;
                hdr.transfer_function = "PQ (SMPTE ST 2084)".into();
            }
            "arib-std-b67" => {
                hdr.detected = true;
                hdr.hdr_type = HdrType::Hlg;
                hdr.transfer_function = "HLG (ARIB STD-B67)".into();
            }
            _ => {}
        }
    }

    let primaries_re = regex_lite::Regex::new(r#""color_primaries"\s*:\s*"([^"]+)""#).unwrap();
    if let Some(cap) = primaries_re.captures(&output) {
        hdr.color_primaries = match &cap[1] {
            "bt2020" => "BT.2020".into(),
            "smpte432" => "P3-D65".into(),
            other => other.to_string(),
        };
    }

    let max_lum_re = regex_lite::Regex::new(r#""max_luminance"\s*:\s*"?(\d+)"#).unwrap();
    if let Some(cap) = max_lum_re.captures(&output) {
        hdr.master_display_max = cap[1].parse::<f64>().unwrap_or(0.0) / LUMINANCE_UNITS_PER_NIT;
    }

    let min_lum_re = regex_lite::Regex::new(r#""min_luminance"\s*:\s*"?(\d+)"#).unwrap();
    if let Some(cap) = min_lum_re.captures(&output) {
        hdr.master_display_min = cap[1].parse::<f64>().unwrap_or(0.0) / LUMINANCE_UNITS_PER_NIT;
    }

    hdr
}

// MaxCLL and MaxFALL live in the CPL's ExtensionProperties, under whatever
// prefix the writer bound the App 2E namespace to
pub fn read_cpl_content_light(dcp_dir: &Path) -> Option<ContentLightLevels> {
    let entries = std::fs::read_dir(dcp_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("xml") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        if !content.contains("CompositionPlaylist") {
            continue;
        }
        let max_cll = content_light_element(&content, "MaxCLL");
        let max_fall = content_light_element(&content, "MaxFALL");
        if max_cll.is_none() && max_fall.is_none() {
            continue;
        }
        return Some(ContentLightLevels {
            max_cll: max_cll.unwrap_or(0),
            max_fall: max_fall.unwrap_or(0),
        });
    }
    None
}

fn content_light_element(cpl: &str, name: &str) -> Option<u32> {
    let pattern = format!(r"<(?:[A-Za-z0-9_.\-]+:)?{name}\b[^>]*>\s*(\d+)\s*<");
    let re = regex_lite::Regex::new(&pattern).ok()?;
    re.captures(cpl)?.get(1)?.as_str().parse().ok()
}

pub fn check_hdr_compliance(
    hdr: &HdrMetadata,
    light: Option<ContentLightLevels>,
    source: &Path,
) -> Vec<Note> {
    let mut notes = Vec::new();
    if !hdr.detected {
        return notes;
    }

    let file = Some(source.to_path_buf());
    let hdr_type = if hdr.hdr_type == HdrType::Pq && light.is_some() {
        HdrType::Hdr10
    } else {
        hdr.hdr_type
    };
    let type_str = match hdr_type {
        HdrType::Pq => "PQ (SMPTE ST 2084)",
        HdrType::Hlg => "HLG (ARIB STD-B67)",
        HdrType::Hdr10 => "HDR10",
        HdrType::Hdr10Plus => "HDR10+",
        HdrType::DolbyVision => "Dolby Vision",
        HdrType::None => "Unknown",
    };

    let primaries = if hdr.color_primaries.is_empty() {
        "no colour primaries".to_string()
    } else {
        format!("{} primaries", hdr.color_primaries)
    };
    notes.push(Note {
        severity: Severity::Info,
        code: Code::HdrMetadataSummary,
        message: format!("HDR: {type_str}, {primaries}"),
        file: file.clone(),
        line: 0,
    });

    if hdr.master_display_max > 0.0 {
        notes.push(Note {
            severity: Severity::Info,
            code: Code::HdrMetadataSummary,
            message: format!(
                "Mastering display: {:.4} to {:.1} nits",
                hdr.master_display_min, hdr.master_display_max
            ),
            file: file.clone(),
            line: 0,
        });
    }

    if hdr.hdr_type == HdrType::Hlg {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::HdrMetadataSummary,
            message: "HLG transfer function uncommon for DCI theatrical release".into(),
            file: file.clone(),
            line: 0,
        });
    }

    let Some(light) = light else {
        return notes;
    };

    notes.push(Note {
        severity: Severity::Info,
        code: Code::HdrMetadataSummary,
        message: format!(
            "MaxCLL: {} nits, MaxFALL: {} nits (CPL ExtensionProperties)",
            light.max_cll, light.max_fall
        ),
        file: file.clone(),
        line: 0,
    });

    if light.max_fall > light.max_cll {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::HdrMetadataInvalid,
            message: format!(
                "MaxFALL {} nits exceeds MaxCLL {} nits, no frame average can be brighter than the brightest pixel",
                light.max_fall, light.max_cll
            ),
            file: file.clone(),
            line: 0,
        });
    }

    if hdr.master_display_max > 0.0 && light.max_cll as f64 > hdr.master_display_max {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::HdrMetadataInvalid,
            message: format!(
                "MaxCLL {} nits exceeds the mastering display maximum of {:.1} nits",
                light.max_cll, hdr.master_display_max
            ),
            file: file.clone(),
            line: 0,
        });
    }

    if hdr.hdr_type == HdrType::Hlg {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::HdrMetadataInvalid,
            message: "ST 2067-21 clause 7.5 defines MaxCLL and MaxFALL for the PQ colour systems only, this composition is HLG".into(),
            file,
            line: 0,
        });
    }

    notes
}

// ════════════════════════════════════════════════════════════════════════════════
// 5. Netflix Delivery Specification
// ════════════════════════════════════════════════════════════════════════════════

/// Netflix delivery compliance result.
#[derive(Debug, Clone, Default)]
pub struct NetflixDeliveryResult {
    pub compliant: bool,
    pub app_id: String,
    pub violations: Vec<String>,
    /// reasons a rule could not be applied
    pub skipped: Vec<String>,
}

// The rules below are the ones Netflix's own IMF validator, Photon, applies to a
// package's XML, and each names the Photon class that carries it. Netflix's
// partner-facing delivery page sits behind a studio login, so nothing here is
// taken from a page that could not be read. The App 2E picture and sound
// constraints live in the essence descriptors rather than the XML, and the
// Photon pass is what reads those.

/// Photon requires exactly one file named `ASSETMAP.xml` at the package root.
/// Source: Netflix Photon, `BasicMapProfileV2MappedFileSet` (`ASSETMAP_FILE_NAME`).
const ASSETMAP_FILE_NAME: &str = "ASSETMAP.xml";
const ASSETMAP_RULE_SOURCE: &str = "https://github.com/Netflix/photon/blob/master/src/main/java/com/netflix/imflibrary/st0429_9/BasicMapProfileV2MappedFileSet.java";

/// The CPL namespaces Photon has a schema for. Source: Netflix Photon,
/// `IMFCompositionPlaylist.supportedCPLSchemas`.
const SUPPORTED_CPL_NAMESPACES: [&str; 2] = [
    "http://www.smpte-ra.org/schemas/2067-3/2013",
    "http://www.smpte-ra.org/schemas/2067-3/2016",
];
const CPL_NAMESPACE_RULE_SOURCE: &str = "https://github.com/Netflix/photon/blob/master/src/main/java/com/netflix/imflibrary/st2067_2/IMFCompositionPlaylist.java";

/// The App 2E ApplicationIdentification values Photon maps to core constraints.
/// Anything else leaves the composition with no App 2E constraints to check.
/// Source: Netflix Photon, `CoreConstraints.fromApplicationId`.
const APP_2E_IDENTIFICATIONS: [&str; 4] = [
    "http://www.smpte-ra.org/schemas/2067-21/2014",
    "http://www.smpte-ra.org/schemas/2067-21/2016",
    "http://www.smpte-ra.org/ns/2067-21/2020",
    "http://www.smpte-ra.org/ns/2067-21/2021",
];
const APP_2E_RULE_SOURCE: &str = "https://github.com/Netflix/photon/blob/master/src/main/java/com/netflix/imflibrary/st2067_2/CoreConstraints.java";

/// Every frame rate App 2E allows at some picture format: FPS_HD, FPS_UHD and
/// FPS_4K taken together. Which of the three sets applies depends on the
/// picture's size, colour model and bit depth, which the Photon pass resolves
/// from the essence descriptor. Source: Netflix Photon,
/// `IMFApp2E2020ConstraintsValidator`.
const APP_2E_EDIT_RATES: [(u64, u64); 9] = [
    (24, 1),
    (24000, 1001),
    (25, 1),
    (30, 1),
    (30000, 1001),
    (50, 1),
    (60, 1),
    (60000, 1001),
    (120, 1),
];
const EDIT_RATE_RULE_SOURCE: &str = "https://github.com/Netflix/photon/blob/master/src/main/java/com/netflix/imflibrary/validation/IMFApp2E2020ConstraintsValidator.java";

/// Check the offline-checkable Netflix IMF delivery rules on an IMF package.
pub fn check_netflix_delivery(imf_dir: &Path) -> NetflixDeliveryResult {
    let mut result = NetflixDeliveryResult::default();

    let entries = match std::fs::read_dir(imf_dir) {
        Ok(e) => e.flatten().map(|e| e.path()).collect::<Vec<_>>(),
        Err(e) => {
            result
                .skipped
                .push(format!("cannot read {}: {e}", imf_dir.display()));
            return result;
        }
    };

    let assetmaps = entries
        .iter()
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name == ASSETMAP_FILE_NAME)
        })
        .count();
    if assetmaps != 1 {
        result.violations.push(format!(
            "the package root holds {assetmaps} files named {ASSETMAP_FILE_NAME}, and exactly one is allowed ({ASSETMAP_RULE_SOURCE})"
        ));
    }

    let mut cpls = Vec::new();
    for path in &entries {
        if path
            .extension()
            .is_none_or(|ext| !ext.eq_ignore_ascii_case("xml"))
        {
            continue;
        }
        match std::fs::read_to_string(path) {
            Ok(content) => {
                if let Some(namespace) = composition_playlist_namespace(&content) {
                    cpls.push((path.clone(), content, namespace));
                }
            }
            Err(e) => result
                .skipped
                .push(format!("cannot read {}: {e}", path.display())),
        }
    }

    if cpls.is_empty() {
        result
            .skipped
            .push(format!("no CPL found in {}", imf_dir.display()));
    }

    for (path, content, namespace) in &cpls {
        let name = path.file_name().unwrap_or_default().to_string_lossy();

        if !SUPPORTED_CPL_NAMESPACES.contains(&namespace.as_str()) {
            result.violations.push(format!(
                "{name} is a CompositionPlaylist in {namespace}, and the accepted namespaces are {} ({CPL_NAMESPACE_RULE_SOURCE})",
                SUPPORTED_CPL_NAMESPACES.join(" and ")
            ));
        }

        match application_identifications(content) {
            Some(ids) => {
                result.app_id = ids.join(" ");
                if !ids.iter().any(|id| APP_2E_IDENTIFICATIONS.contains(&id.as_str())) {
                    result.violations.push(format!(
                        "{name} declares ApplicationIdentification '{}', which is no App 2E identification ({APP_2E_RULE_SOURCE})",
                        result.app_id
                    ));
                }
            }
            None => result.violations.push(format!(
                "{name} carries no ApplicationIdentification, so it declares no App 2E conformance ({APP_2E_RULE_SOURCE})"
            )),
        }

        match composition_edit_rate(content) {
            Some(rate) => {
                if !APP_2E_EDIT_RATES
                    .iter()
                    .any(|(n, d)| n * rate.1 == rate.0 * d)
                {
                    result.violations.push(format!(
                        "{name} declares an EditRate of {} {}, which is no App 2E frame rate ({EDIT_RATE_RULE_SOURCE})",
                        rate.0, rate.1
                    ));
                }
            }
            None => result
                .skipped
                .push(format!("{name} declares no composition EditRate")),
        }
    }

    result.compliant = result.violations.is_empty() && result.skipped.is_empty();
    result
}

/// The namespace of an XML document whose root element is a
/// `CompositionPlaylist`, resolved rather than read off the prefix.
fn composition_playlist_namespace(xml: &str) -> Option<String> {
    let (root, namespace) = crate::schema::root_element(xml)?;
    (root == "CompositionPlaylist").then_some(namespace)
}

/// The URIs an `ApplicationIdentification` element lists. ST 2067-2 types it as
/// a whitespace-separated list, so a CPL may name more than one.
fn application_identifications(content: &str) -> Option<Vec<String>> {
    let element = regex_lite::Regex::new(
        r"<(?:[\w-]+:)?ApplicationIdentification>([^<]*)</(?:[\w-]+:)?ApplicationIdentification>",
    )
    .unwrap();
    let value = element.captures(content)?.get(1)?.as_str();
    Some(value.split_whitespace().map(str::to_string).collect())
}

/// The composition's own EditRate, which in ST 2067-3 element order is the first
/// one in the document, ahead of the segments and their resources.
fn composition_edit_rate(content: &str) -> Option<(u64, u64)> {
    let element =
        regex_lite::Regex::new(r"<(?:[\w-]+:)?EditRate>([^<]*)</(?:[\w-]+:)?EditRate>").unwrap();
    let value = element.captures(content)?.get(1)?.as_str();
    let mut parts = value.split_whitespace();
    let numerator: u64 = parts.next()?.parse().ok()?;
    let denominator: u64 = match parts.next() {
        Some(d) => d.parse().ok()?,
        None => 1,
    };
    (denominator > 0).then_some((numerator, denominator))
}

/// Convert Netflix result to notes.
pub fn netflix_to_notes(result: &NetflixDeliveryResult, source: &Path) -> Vec<Note> {
    let mut notes = Vec::new();
    let file = Some(source.to_path_buf());

    for reason in &result.skipped {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::CheckSkipped,
            message: format!("Netflix delivery spec not verified: {reason}"),
            file: file.clone(),
            line: 0,
        });
    }

    for violation in &result.violations {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::NetflixDeliveryViolation,
            message: format!("Netflix delivery spec: {violation}"),
            file: file.clone(),
            line: 0,
        });
    }

    if result.compliant {
        notes.push(Note {
            severity: Severity::Info,
            code: Code::NetflixDeliveryViolation,
            message: "Netflix delivery spec: PASS".into(),
            file: file.clone(),
            line: 0,
        });
    }

    // the App 2E rules these checks do not reach are the descriptor ones
    notes.push(Note {
        severity: Severity::Info,
        code: Code::NetflixDeliveryViolation,
        message: "Netflix delivery spec: the App 2E colour model, bit depth, stored size, frame layout and JPEG 2000 profile live in the essence descriptors, which the Photon pass reads".into(),
        file,
        line: 0,
    });

    notes
}

// ════════════════════════════════════════════════════════════════════════════════
// 6. ProRes Detection
// ════════════════════════════════════════════════════════════════════════════════

/// ProRes codec info.
#[derive(Debug, Clone, Default)]
pub struct ProResInfo {
    pub detected: bool,
    pub codec_variant: String,
    pub width: u32,
    pub height: u32,
    pub frame_rate: f64,
}

/// Detect ProRes encoding in MXF.
pub fn detect_prores(mxf_path: &Path) -> ProResInfo {
    let mut info = ProResInfo::default();

    let output = run_ffprobe(
        &[
            "-v",
            "quiet",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=codec_name,codec_long_name,width,height,r_frame_rate",
            "-of",
            "json",
        ],
        mxf_path,
    );

    if output.contains("prores") || output.contains("ProRes") || output.contains("Apple") {
        info.detected = true;

        if output.contains("4444") {
            info.codec_variant = "ProRes 4444".into();
        } else if output.contains("422 HQ") || output.contains("422hq") {
            info.codec_variant = "ProRes 422 HQ".into();
        } else if output.contains("422") {
            info.codec_variant = "ProRes 422".into();
        } else {
            info.codec_variant = "ProRes".into();
        }

        let w_re = regex_lite::Regex::new(r#""width"\s*:\s*(\d+)"#).unwrap();
        if let Some(cap) = w_re.captures(&output) {
            info.width = cap[1].parse().unwrap_or(0);
        }

        let h_re = regex_lite::Regex::new(r#""height"\s*:\s*(\d+)"#).unwrap();
        if let Some(cap) = h_re.captures(&output) {
            info.height = cap[1].parse().unwrap_or(0);
        }

        let fr_re = regex_lite::Regex::new(r#""r_frame_rate"\s*:\s*"(\d+)/(\d+)""#).unwrap();
        if let Some(cap) = fr_re.captures(&output) {
            let num: f64 = cap[1].parse().unwrap_or(0.0);
            let den: f64 = cap[2].parse().unwrap_or(1.0);
            if den > 0.0 {
                info.frame_rate = num / den;
            }
        }
    }

    info
}

// ════════════════════════════════════════════════════════════════════════════════
// 8. Accessibility Track Validation
// ════════════════════════════════════════════════════════════════════════════════

/// Check for accessibility tracks (AD, HI/SDH, CC) in a package.
pub fn check_accessibility(package_dir: &Path) -> Vec<Note> {
    let mut notes = Vec::new();

    let entries = match std::fs::read_dir(package_dir) {
        Ok(e) => e,
        Err(e) => {
            notes.push(Note {
                severity: Severity::Warning,
                code: Code::CheckSkipped,
                message: format!(
                    "accessibility tracks not checked, cannot read {}: {e}",
                    package_dir.display()
                ),
                file: Some(package_dir.to_path_buf()),
                line: 0,
            });
            return notes;
        }
    };

    let mut has_audio_desc = false;
    let mut has_hi_subtitles = false;
    let mut has_closed_captions = false;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("xml") {
            continue;
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        if !content.contains("CompositionPlaylist") {
            continue;
        }

        // MCA labels for accessibility
        if content.contains("VisuallyImpaired")
            || content.contains("AudioDescription")
            || content.contains("chAD")
        {
            has_audio_desc = true;
        }
        if content.contains("HearingImpaired") || content.contains("chHI") {
            has_hi_subtitles = true;
        }

        // Closed captions
        if content.contains("MainClosedCaption") || content.contains("ClosedCaption") {
            has_closed_captions = true;
        }

        // Annotation text patterns
        if content.contains("-HI") || content.contains("_HI") || content.contains("SDH") {
            has_hi_subtitles = true;
        }
        if content.contains("_AD") || content.contains("-AD") {
            has_audio_desc = true;
        }

        // RFC5646 spoken language patterns
        if content.contains("audiodesc") || content.contains("audio-desc") {
            has_audio_desc = true;
        }
    }

    let file = Some(package_dir.to_path_buf());

    if has_audio_desc {
        notes.push(Note {
            severity: Severity::Info,
            code: Code::SoundInvalidChannelCount,
            message: "Accessibility: Audio Description (VI/AD) track present".into(),
            file: file.clone(),
            line: 0,
        });
    }
    if has_hi_subtitles {
        notes.push(Note {
            severity: Severity::Info,
            code: Code::SubtitleParseError,
            message: "Accessibility: Hearing Impaired (HI/SDH) subtitles present".into(),
            file: file.clone(),
            line: 0,
        });
    }
    if has_closed_captions {
        notes.push(Note {
            severity: Severity::Info,
            code: Code::SubtitleParseError,
            message: "Accessibility: Closed Captions present".into(),
            file: file.clone(),
            line: 0,
        });
    }

    if !has_audio_desc && !has_hi_subtitles && !has_closed_captions {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::SubtitleParseError,
            message: "No accessibility tracks detected (AD/HI/CC) — consider adding for compliance"
                .into(),
            file,
            line: 0,
        });
    }

    notes
}

// ════════════════════════════════════════════════════════════════════════════════
// 9. Content Fingerprinting (Perceptual Hash)
// ════════════════════════════════════════════════════════════════════════════════

/// Perceptual fingerprint of video content.
#[derive(Debug, Clone, Default)]
pub struct ContentFingerprint {
    pub hash: String,
    pub width: u32,
    pub height: u32,
    pub frame_sampled: u32,
}

/// Side of the grayscale grid the average hash is built from, so the hash is
/// 8x8 = 64 bits.
const HASH_GRID: u32 = 8;
const HASH_PIXELS: usize = (HASH_GRID * HASH_GRID) as usize;

/// The sampled frame sits this far into the content, past a leader or slate.
const SAMPLE_FRACTION: u32 = 10;

/// Normalized Hamming distance at or below which two fingerprints are the same
/// picture. Re-encoding the same frames at another bitrate moves no more than a
/// few of the 64 bits; unrelated pictures sit far above this.
pub const SAME_PICTURE_DISTANCE: f64 = 0.1;

/// Generate a perceptual hash fingerprint from a picture MXF.
pub fn generate_fingerprint(mxf_path: &Path) -> ContentFingerprint {
    let mut fp = ContentFingerprint::default();
    let stream = probe_picture_stream(mxf_path);
    fp.width = stream.width;
    fp.height = stream.height;
    fp.frame_sampled = if stream.frames > SAMPLE_FRACTION {
        stream.frames / SAMPLE_FRACTION
    } else {
        0
    };

    let Some(pixels) = sample_gray_grid(mxf_path, fp.frame_sampled) else {
        return fp;
    };

    let mean = (pixels.iter().map(|&p| p as u32).sum::<u32>() / HASH_PIXELS as u32) as u8;
    let mut hash = 0u64;
    for pixel in pixels {
        hash <<= 1;
        if pixel > mean {
            hash |= 1;
        }
    }

    fp.hash = format!("{hash:016x}");
    fp
}

/// What the picture stream declares, with the frame count derived from the
/// duration when the container states none. MXF states none.
struct PictureStream {
    width: u32,
    height: u32,
    frames: u32,
}

fn probe_picture_stream(mxf_path: &Path) -> PictureStream {
    let output = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,nb_frames,duration,r_frame_rate",
            // ffprobe emits the entries in its own order, so read them by key
            "-of",
            "default=noprint_wrappers=1",
        ])
        .arg(mxf_path)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();

    let value = |key: &str| -> Option<String> {
        output
            .lines()
            .find_map(|line| {
                line.strip_prefix(key)?
                    .strip_prefix('=')
                    .map(str::to_string)
            })
            .filter(|v| v != "N/A")
    };
    let number = |key: &str| value(key).and_then(|v| v.parse::<f64>().ok());

    let frames = number("nb_frames").or_else(|| {
        let rate = value("r_frame_rate")?;
        let (numerator, denominator) = rate.split_once('/')?;
        let numerator = numerator.parse::<f64>().ok()?;
        let denominator = denominator.parse::<f64>().ok()?;
        if denominator <= 0.0 {
            return None;
        }
        Some(number("duration")? * numerator / denominator)
    });

    PictureStream {
        width: number("width").unwrap_or(0.0) as u32,
        height: number("height").unwrap_or(0.0) as u32,
        frames: frames.unwrap_or(0.0) as u32,
    }
}

/// One frame decoded to an 8x8 grayscale grid, or None when ffmpeg could not
/// reach that frame.
fn sample_gray_grid(mxf_path: &Path, frame: u32) -> Option<[u8; HASH_PIXELS]> {
    let output = std::process::Command::new("ffmpeg")
        .args(["-v", "quiet", "-i"])
        .arg(mxf_path)
        .args([
            "-vf",
            // the comma inside the select expression is escaped for ffmpeg's
            // own filter parser, not for a shell
            &format!("select=eq(n\\,{frame}),scale={HASH_GRID}:{HASH_GRID},format=gray"),
            // without this, dropped frames are made up again at the output rate
            "-fps_mode",
            "passthrough",
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "pipe:1",
        ])
        .output()
        .ok()?;

    output.stdout.get(..HASH_PIXELS)?.try_into().ok()
}

/// Compare two fingerprints, returns normalized Hamming distance (0.0 = identical).
pub fn compare_fingerprints(a: &ContentFingerprint, b: &ContentFingerprint) -> f64 {
    if a.hash.is_empty() || b.hash.is_empty() {
        return 1.0;
    }
    if a.hash == b.hash {
        return 0.0;
    }

    let ha = u64::from_str_radix(&a.hash, 16).unwrap_or(0);
    let hb = u64::from_str_radix(&b.hash, 16).unwrap_or(0);

    let distance = (ha ^ hb).count_ones();
    distance as f64 / HASH_PIXELS as f64
}

// ════════════════════════════════════════════════════════════════════════════════
// Internal helpers
// ════════════════════════════════════════════════════════════════════════════════

// the path goes in as its own argument, so a quote or a $(...) in it stays a filename
fn run_ffprobe(args: &[&str], path: &Path) -> String {
    std::process::Command::new("ffprobe")
        .args(args)
        .arg(path)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

fn parse_ttml_time(time_str: &str) -> f64 {
    if time_str.is_empty() {
        return -1.0;
    }

    let parts: Vec<&str> = time_str.split(':').collect();
    if parts.len() >= 3 {
        let hours: f64 = parts[0].parse().unwrap_or(0.0);
        let minutes: f64 = parts[1].parse().unwrap_or(0.0);
        // Third part might be "SS.mmm" or "SS:FF"
        let sec_parts: Vec<&str> = parts[2].split('.').collect();
        let seconds: f64 = sec_parts[0].parse().unwrap_or(0.0);
        let frac: f64 = if sec_parts.len() > 1 {
            format!("0.{}", sec_parts[1]).parse().unwrap_or(0.0)
        } else if parts.len() > 3 {
            // Frame-based: HH:MM:SS:FF
            parts[3].parse::<f64>().unwrap_or(0.0) / 24.0
        } else {
            0.0
        };
        return hours * 3600.0 + minutes * 60.0 + seconds + frac;
    }

    -1.0
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use dolby_vision::rpu::extension_metadata::blocks::ExtMetadataBlockLevel6;
    use postkit::dolby_vision::{
        DOLBY_VISION_FIXTURE_FRAMES, DolbyVisionFixtureProfile, write_dolby_vision_fixture,
    };

    use super::*;
    use crate::track_fixtures::{
        SoundStretch, bt709, hlg_bt2020, pq_bt2020, write_atmos_track, write_iab_track,
        write_picture_track, write_sound_track,
    };
    #[cfg(unix)]
    use crate::track_fixtures::{assert_the_path_was_not_run, shell_injection_name};

    const FRAMES: u32 = 24;

    fn only_note(notes: &[Note]) -> &Note {
        assert_eq!(notes.len(), 1, "expected exactly one note, got: {notes:?}");
        &notes[0]
    }

    #[test]
    fn an_atmos_track_file_reports_the_object_count_its_descriptor_declares() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("atmos.mxf");
        write_atmos_track(&path, FRAMES, 42);

        let info = parse_atmos_iab(&path);
        assert_eq!(info.essence, ImmersiveEssence::DolbyAtmos);
        assert_eq!(info.object_count, Some(42));
        assert_eq!(info.frame_count, FRAMES);

        let notes = check_atmos_compliance(&info, &path);
        let note = only_note(&notes);
        assert_eq!(note.severity, Severity::Info);
        assert!(note.message.contains("42 objects"), "{}", note.message);
    }

    #[test]
    fn an_atmos_track_file_past_the_object_limit_is_an_error() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("atmos.mxf");
        write_atmos_track(&path, FRAMES, MAX_ATMOS_OBJECTS + 1);

        let notes = check_atmos_compliance(&parse_atmos_iab(&path), &path);

        assert!(
            notes
                .iter()
                .any(|n| n.severity == Severity::Error && n.message.contains("119")),
            "{notes:?}"
        );
    }

    #[test]
    fn an_iab_track_file_is_detected_and_says_it_carries_no_object_count() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("iab.mxf");
        write_iab_track(&path, FRAMES);

        let info = parse_atmos_iab(&path);
        assert_eq!(info.essence, ImmersiveEssence::Iab);
        assert_eq!(info.object_count, None);

        let notes = check_atmos_compliance(&info, &path);
        let note = only_note(&notes);
        assert!(note.message.contains("IAB"), "{}", note.message);
    }

    fn app2e_cpl(path: &std::path::Path, max_cll: u32, max_fall: u32) {
        std::fs::write(
            path,
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/2067-3/2016">
  <Id>urn:uuid:4b0c85d9-b65d-4b1a-9cfd-92f0b28ca5f0</Id>
  <ExtensionProperties>
    <app2e:MaxCLL xmlns:app2e="http://www.smpte-ra.org/ns/2067-21/2020">{max_cll}</app2e:MaxCLL>
    <app2e:MaxFALL xmlns:app2e="http://www.smpte-ra.org/ns/2067-21/2020">{max_fall}</app2e:MaxFALL>
  </ExtensionProperties>
</CompositionPlaylist>"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn a_pq_picture_track_and_its_cpl_report_the_transfer_and_both_light_levels() {
        let directory = tempfile::tempdir().unwrap();
        let picture = directory.path().join("PICTURE.mxf");
        write_picture_track(&picture, 2, Some(pq_bt2020()));
        app2e_cpl(&directory.path().join("CPL.xml"), 993, 362);

        let hdr = detect_hdr_metadata(&picture);
        assert!(hdr.detected);
        assert!(hdr.from_descriptor);
        assert_eq!(hdr.hdr_type, HdrType::Pq);
        assert_eq!(hdr.color_primaries, "BT.2020");
        assert_eq!(hdr.master_display_max, 1000.0);

        let light = read_cpl_content_light(directory.path()).expect("the CPL declares both");
        assert_eq!(light.max_cll, 993);
        assert_eq!(light.max_fall, 362);

        let notes = check_hdr_compliance(&hdr, Some(light), &picture);
        let messages: Vec<&str> = notes.iter().map(|n| n.message.as_str()).collect();
        assert!(
            messages.contains(&"HDR: HDR10, BT.2020 primaries"),
            "{messages:?}"
        );
        assert!(
            messages.contains(&"Mastering display: 0.0050 to 1000.0 nits"),
            "{messages:?}"
        );
        assert!(
            messages.contains(&"MaxCLL: 993 nits, MaxFALL: 362 nits (CPL ExtensionProperties)"),
            "{messages:?}"
        );
        assert!(
            notes.iter().all(|n| n.severity == Severity::Info),
            "{notes:?}"
        );
    }

    #[test]
    fn a_max_fall_above_max_cll_is_an_error() {
        let directory = tempfile::tempdir().unwrap();
        let picture = directory.path().join("PICTURE.mxf");
        write_picture_track(&picture, 2, Some(pq_bt2020()));
        app2e_cpl(&directory.path().join("CPL.xml"), 400, 900);

        let light = read_cpl_content_light(directory.path()).unwrap();
        let notes = check_hdr_compliance(&detect_hdr_metadata(&picture), Some(light), &picture);

        let note = notes
            .iter()
            .find(|n| n.code == Code::HdrMetadataInvalid)
            .unwrap_or_else(|| panic!("no HDR error: {notes:?}"));
        assert_eq!(note.severity, Severity::Error);
        assert!(note.message.contains("900"), "{}", note.message);
    }

    #[test]
    fn a_max_cll_above_the_mastering_display_is_a_warning() {
        let directory = tempfile::tempdir().unwrap();
        let picture = directory.path().join("PICTURE.mxf");
        write_picture_track(&picture, 2, Some(pq_bt2020()));
        app2e_cpl(&directory.path().join("CPL.xml"), 4000, 300);

        let light = read_cpl_content_light(directory.path()).unwrap();
        let notes = check_hdr_compliance(&detect_hdr_metadata(&picture), Some(light), &picture);

        assert!(
            notes.iter().any(|n| n.code == Code::HdrMetadataInvalid
                && n.severity == Severity::Warning
                && n.message.contains("1000.0 nits")),
            "{notes:?}"
        );
    }

    #[test]
    fn an_hlg_picture_track_is_reported_as_hlg() {
        let directory = tempfile::tempdir().unwrap();
        let picture = directory.path().join("PICTURE.mxf");
        write_picture_track(&picture, 2, Some(hlg_bt2020()));

        let hdr = detect_hdr_metadata(&picture);

        assert_eq!(hdr.hdr_type, HdrType::Hlg);
        let notes = check_hdr_compliance(&hdr, None, &picture);
        assert!(
            notes.iter().any(|n| n.message.contains("HDR: HLG")),
            "{notes:?}"
        );
    }

    #[test]
    fn a_rec_709_picture_track_draws_no_hdr_notes() {
        let directory = tempfile::tempdir().unwrap();
        let picture = directory.path().join("PICTURE.mxf");
        write_picture_track(&picture, 2, Some(bt709()));
        app2e_cpl(&directory.path().join("CPL.xml"), 993, 362);

        let hdr = detect_hdr_metadata(&picture);

        assert!(!hdr.detected, "{hdr:?}");
        let light = read_cpl_content_light(directory.path());
        assert!(check_hdr_compliance(&hdr, light, &picture).is_empty());
    }

    fn write_clip(path: &Path, encoder_arguments: &[&str]) {
        let output = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=64x64:rate=24:duration=0.5",
            ])
            .args(encoder_arguments)
            .arg(path)
            .output()
            .expect("ffmpeg has to be on PATH");
        assert!(
            output.status.success(),
            "ffmpeg failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // ffmpeg's own colour flags never reach the x265 VUI, so ffprobe reads back nothing
    fn write_pq_clip(path: &Path) {
        write_clip(
            path,
            &[
                "-c:v",
                "libx265",
                "-preset",
                "ultrafast",
                "-pix_fmt",
                "yuv420p10le",
                "-x265-params",
                "colorprim=bt2020:transfer=smpte2084:colormatrix=bt2020nc",
            ],
        );
    }

    fn write_prores_clip(path: &Path) {
        write_clip(path, &["-c:v", "prores_ks", "-profile:v", "3"]);
    }

    #[test]
    fn a_pq_tagged_clip_reports_pq_and_bt_2020() {
        let directory = tempfile::tempdir().unwrap();
        let clip = directory.path().join("pq.mkv");
        write_pq_clip(&clip);

        let hdr = hdr_from_ffprobe(&clip);

        assert!(hdr.detected, "{hdr:?}");
        assert_eq!(hdr.hdr_type, HdrType::Pq);
        assert_eq!(hdr.color_primaries, "BT.2020");
    }

    #[test]
    fn a_prores_clip_reports_prores_and_the_width_it_was_encoded_at() {
        let directory = tempfile::tempdir().unwrap();
        let clip = directory.path().join("prores.mov");
        write_prores_clip(&clip);

        let info = detect_prores(&clip);

        assert!(info.detected, "{info:?}");
        assert_eq!(info.codec_variant, "ProRes");
        assert_eq!(info.width, 64);
        assert_eq!(info.height, 64);
    }

    #[cfg(unix)]
    #[test]
    fn hdr_from_ffprobe_reads_a_path_holding_shell_text_instead_of_running_it() {
        let directory = tempfile::tempdir().unwrap();
        let clip = directory.path().join(shell_injection_name("mkv"));
        write_pq_clip(&clip);

        let hdr = hdr_from_ffprobe(&clip);

        assert_the_path_was_not_run();
        assert_eq!(hdr.hdr_type, HdrType::Pq);
    }

    #[cfg(unix)]
    #[test]
    fn detect_prores_reads_a_path_holding_shell_text_instead_of_running_it() {
        let directory = tempfile::tempdir().unwrap();
        let clip = directory.path().join(shell_injection_name("mov"));
        write_prores_clip(&clip);

        let info = detect_prores(&clip);

        assert_the_path_was_not_run();
        assert!(info.detected, "{info:?}");
        assert_eq!(info.width, 64);
    }

    #[test]
    fn a_pcm_sound_track_file_is_not_immersive_audio() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sound.mxf");
        write_sound_track(
            &path,
            &[SoundStretch {
                seconds: 1.0,
                amplitude: 0.5,
            }],
        );

        let info = parse_atmos_iab(&path);

        assert!(!info.detected());
        assert!(check_atmos_compliance(&info, &path).is_empty());
    }

    fn dolby_vision_fixture(
        directory: &Path,
        profile: DolbyVisionFixtureProfile,
        level6: Option<ExtMetadataBlockLevel6>,
    ) -> PathBuf {
        write_dolby_vision_fixture(directory, "dolby_vision.hevc", profile, level6, None).unwrap()
    }

    #[test]
    fn a_profile_81_rpu_reports_its_profile_frames_and_level_6_light_levels() {
        let directory = tempfile::tempdir().unwrap();
        let path = dolby_vision_fixture(
            directory.path(),
            DolbyVisionFixtureProfile::Profile81,
            Some(ExtMetadataBlockLevel6 {
                max_display_mastering_luminance: 1000,
                min_display_mastering_luminance: 1,
                max_content_light_level: 993,
                max_frame_average_light_level: 362,
            }),
        );

        let dv = parse_dolby_vision(&path)
            .unwrap()
            .expect("an RPU is present");

        assert_eq!(dv.profile, 8);
        assert_eq!(dv.frames, DOLBY_VISION_FIXTURE_FRAMES);
        assert_eq!(dv.max_content_light_level_nits, Some(993.0));
        assert_eq!(dv.max_frame_average_light_level_nits, Some(362.0));
        assert!(dv.undecodable_reason.is_none(), "{dv:?}");

        let notes = check_dolby_vision_compliance(&dv, &path);
        let note = only_note(&notes);
        assert_eq!(note.severity, Severity::Info);
        assert_eq!(note.code, Code::HdrMetadataSummary);
        assert!(
            note.message.contains("profile 8")
                && note.message.contains("6 frames")
                && note.message.contains("MaxCLL 993 nits")
                && note.message.contains("MaxFALL 362 nits"),
            "{}",
            note.message
        );
    }

    #[test]
    fn a_profile_5_rpu_warns_that_only_the_rpu_can_turn_it_back_into_rgb() {
        let directory = tempfile::tempdir().unwrap();
        let path =
            dolby_vision_fixture(directory.path(), DolbyVisionFixtureProfile::Profile5, None);

        let dv = parse_dolby_vision(&path)
            .unwrap()
            .expect("an RPU is present");

        assert_eq!(dv.profile, 5);

        let notes = check_dolby_vision_compliance(&dv, &path);
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(
            notes[0].message.contains("no level 6 block"),
            "{}",
            notes[0].message
        );
        assert_eq!(notes[1].severity, Severity::Warning);
        assert_eq!(notes[1].code, Code::HdrMetadataInvalid);
        assert!(
            notes[1].message.contains("profile 5")
                && notes[1].message.contains("IPT PQ c2")
                && notes[1].message.contains("profile 8.1"),
            "{}",
            notes[1].message
        );
    }

    #[test]
    fn a_jpeg_2000_track_file_carries_no_dolby_vision_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let picture = directory.path().join("PICTURE.mxf");
        write_picture_track(&picture, 2, None);

        assert_eq!(parse_dolby_vision(&picture).unwrap(), None);
    }
}

#[cfg(test)]
mod netflix_tests {
    use super::*;

    const APP_2E_2020: &str = "http://www.smpte-ra.org/ns/2067-21/2020";

    /// An IMP holding only the documents these rules read.
    fn write_imp(
        dir: &Path,
        assetmap_name: &str,
        cpl_namespace: &str,
        application: Option<&str>,
        edit_rate: &str,
    ) {
        std::fs::write(
            dir.join(assetmap_name),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:2fd93ab2-dab7-481d-bb43-5779ba62384d</Id>
</AssetMap>"#,
        )
        .unwrap();

        let application = match application {
            Some(id) => format!("<ApplicationIdentification>{id}</ApplicationIdentification>"),
            None => String::new(),
        };
        std::fs::write(
            dir.join("CPL.xml"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="{cpl_namespace}">
  <Id>urn:uuid:394080ca-5471-40e9-9827-e6e577753400</Id>
  <ContentTitle>Netflix rules</ContentTitle>
  <EditRate>{edit_rate}</EditRate>
  {application}
</CompositionPlaylist>"#
            ),
        )
        .unwrap();
    }

    fn app_2e_imp(dir: &Path) {
        write_imp(
            dir,
            "ASSETMAP.xml",
            "http://www.smpte-ra.org/schemas/2067-3/2016",
            Some(APP_2E_2020),
            "24 1",
        );
    }

    #[test]
    fn an_app_2e_imp_passes_every_rule() {
        let dir = tempfile::tempdir().unwrap();
        app_2e_imp(dir.path());

        let result = check_netflix_delivery(dir.path());
        assert!(
            result.compliant,
            "an App 2E IMP must pass, got: {:?} {:?}",
            result.violations, result.skipped
        );
        assert_eq!(result.app_id, APP_2E_2020);
    }

    #[test]
    fn an_assetmap_without_the_xml_extension_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        write_imp(
            dir.path(),
            "ASSETMAP",
            "http://www.smpte-ra.org/schemas/2067-3/2016",
            Some(APP_2E_2020),
            "24 1",
        );

        let result = check_netflix_delivery(dir.path());
        assert!(
            result
                .violations
                .iter()
                .any(|v| v.contains("0 files named ASSETMAP.xml")),
            "got: {:?}",
            result.violations
        );
    }

    #[test]
    fn a_cpl_that_is_not_st_2067_3_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        write_imp(
            dir.path(),
            "ASSETMAP.xml",
            "http://www.smpte-ra.org/schemas/429-7/2006/CPL",
            Some(APP_2E_2020),
            "24 1",
        );

        let result = check_netflix_delivery(dir.path());
        assert!(
            result.violations.iter().any(|v| v
                .contains("CompositionPlaylist in http://www.smpte-ra.org/schemas/429-7/2006/CPL")),
            "got: {:?}",
            result.violations
        );
    }

    #[test]
    fn a_cpl_with_no_application_identification_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        write_imp(
            dir.path(),
            "ASSETMAP.xml",
            "http://www.smpte-ra.org/schemas/2067-3/2016",
            None,
            "24 1",
        );

        let result = check_netflix_delivery(dir.path());
        assert!(
            result
                .violations
                .iter()
                .any(|v| v.contains("carries no ApplicationIdentification")),
            "got: {:?}",
            result.violations
        );
    }

    /// ST 2067-20 is Application 2, not the 2E Netflix takes.
    #[test]
    fn an_application_that_is_not_2e_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        write_imp(
            dir.path(),
            "ASSETMAP.xml",
            "http://www.smpte-ra.org/schemas/2067-3/2016",
            Some("http://www.smpte-ra.org/schemas/2067-20/2013"),
            "24 1",
        );

        let result = check_netflix_delivery(dir.path());
        assert!(
            result.violations.iter().any(|v| v.contains(
                "ApplicationIdentification 'http://www.smpte-ra.org/schemas/2067-20/2013'"
            )),
            "got: {:?}",
            result.violations
        );
    }

    #[test]
    fn the_app_2e_frame_rates_are_accepted_and_others_are_not() {
        for (rate, accepted) in [
            ("24 1", true),
            ("24000 1001", true),
            ("30000 1001", true),
            ("60000 1001", true),
            ("120 1", true),
            ("48 1", false),
            ("23 1", false),
        ] {
            let dir = tempfile::tempdir().unwrap();
            write_imp(
                dir.path(),
                "ASSETMAP.xml",
                "http://www.smpte-ra.org/schemas/2067-3/2016",
                Some(APP_2E_2020),
                rate,
            );

            let result = check_netflix_delivery(dir.path());
            assert_eq!(
                !result
                    .violations
                    .iter()
                    .any(|v| v.contains("is no App 2E frame rate")),
                accepted,
                "EditRate {rate}, got: {:?}",
                result.violations
            );
        }
    }

    /// The rules that need the essence descriptors are Photon's, and the notes
    /// have to say so rather than leave a package looking fully checked.
    #[test]
    fn the_notes_name_photon_as_the_descriptor_pass() {
        let dir = tempfile::tempdir().unwrap();
        app_2e_imp(dir.path());

        let notes = netflix_to_notes(&check_netflix_delivery(dir.path()), dir.path());
        assert!(
            notes
                .iter()
                .any(|n| n.message.contains("Photon pass reads")),
            "got: {notes:?}"
        );
    }
}
