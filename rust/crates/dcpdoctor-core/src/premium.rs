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
// 2. Dolby Vision 4.0 Metadata
// ════════════════════════════════════════════════════════════════════════════════

/// Dolby Vision metadata detected from MXF.
#[derive(Debug, Clone, Default)]
pub struct DolbyVisionMetadata {
    pub detected: bool,
    pub profile: u8,
    pub level: u8,
    pub bl_present_flag: u8,
    pub el_present_flag: u8,
    pub rpu_present_flag: u8,
    pub is_tunnel: bool,
    pub is_mef: bool,
    pub rpu_count: u32,
}

/// Parse Dolby Vision metadata from an MXF file.
pub fn parse_dolby_vision(mxf_path: &Path) -> DolbyVisionMetadata {
    let mut dv = DolbyVisionMetadata::default();

    // Use ffprobe to detect Dolby Vision configuration
    let cmd = format!(
        "ffprobe -v quiet -select_streams v:0 -show_entries \
         stream_side_data=side_data_type,dv_profile,dv_level,dv_bl_present_flag,\
         dv_el_present_flag,dv_rpu_present_flag -of csv=p=0 \"{}\" 2>/dev/null",
        mxf_path.display()
    );
    let mut output = run_cmd(&cmd);

    // Also try JSON format if CSV didn't find DOVI
    if !output.contains("DOVI") && !output.contains("dovi") && !output.contains("dolby_vision") {
        let json_cmd = format!(
            "ffprobe -v quiet -select_streams v:0 -show_streams -of json \"{}\" 2>/dev/null",
            mxf_path.display()
        );
        output = run_cmd(&json_cmd);
    }

    if output.contains("DOVI")
        || output.contains("dovi")
        || output.contains("dolby_vision")
        || output.contains("Dolby Vision")
    {
        dv.detected = true;

        let profile_re = regex_lite::Regex::new(r#""?dv_profile"?\s*[:=]\s*(\d+)"#).unwrap();
        if let Some(cap) = profile_re.captures(&output) {
            dv.profile = cap[1].parse().unwrap_or(0);
        }

        let level_re = regex_lite::Regex::new(r#""?dv_level"?\s*[:=]\s*(\d+)"#).unwrap();
        if let Some(cap) = level_re.captures(&output) {
            dv.level = cap[1].parse().unwrap_or(0);
        }

        let bl_re = regex_lite::Regex::new(r#""?dv_bl_present_flag"?\s*[:=]\s*(\d+)"#).unwrap();
        if let Some(cap) = bl_re.captures(&output) {
            dv.bl_present_flag = cap[1].parse().unwrap_or(0);
        }

        let el_re = regex_lite::Regex::new(r#""?dv_el_present_flag"?\s*[:=]\s*(\d+)"#).unwrap();
        if let Some(cap) = el_re.captures(&output) {
            dv.el_present_flag = cap[1].parse().unwrap_or(0);
        }

        let rpu_re = regex_lite::Regex::new(r#""?dv_rpu_present_flag"?\s*[:=]\s*(\d+)"#).unwrap();
        if let Some(cap) = rpu_re.captures(&output) {
            dv.rpu_present_flag = cap[1].parse().unwrap_or(0);
        }

        dv.is_tunnel = dv.profile == 5 || dv.el_present_flag > 0;
        dv.is_mef = dv.profile == 5 && dv.el_present_flag > 0;

        if dv.profile == 0 {
            dv.profile = 8; // Default to single-layer
            dv.bl_present_flag = 1;
        }
    }

    // Count RPU frames if detected
    if dv.detected && dv.rpu_present_flag > 0 {
        let count_cmd = format!(
            "ffprobe -v quiet -select_streams v:0 -count_packets -show_entries \
             stream=nb_read_packets -of csv=p=0 \"{}\" 2>/dev/null",
            mxf_path.display()
        );
        let count_out = run_cmd(&count_cmd);
        if let Ok(count) = count_out.trim().parse::<u32>() {
            dv.rpu_count = count;
        }
    }

    dv
}

/// Check Dolby Vision compliance for DCI theatrical.
pub fn check_dolby_vision_compliance(dv: &DolbyVisionMetadata, source: &Path) -> Vec<Note> {
    let mut notes = Vec::new();
    if !dv.detected {
        return notes;
    }

    let file = Some(source.to_path_buf());

    notes.push(Note {
        severity: Severity::Info,
        code: Code::MxfInvalidStructure,
        message: format!(
            "Dolby Vision detected: Profile {} ({})",
            dv.profile,
            if dv.is_tunnel {
                "dual-layer tunnel"
            } else {
                "single-layer"
            }
        ),
        file: file.clone(),
        line: 0,
    });

    if dv.is_mef {
        notes.push(Note {
            severity: Severity::Info,
            code: Code::MxfInvalidStructure,
            message: "Dolby Vision 4.0 MEF (Multi-resolution Enhancement) detected".into(),
            file: file.clone(),
            line: 0,
        });
    }

    if dv.profile == 5 {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::MxfInvalidStructure,
            message: "Dolby Vision Profile 5 (dual-layer) may not be supported by all servers"
                .into(),
            file: file.clone(),
            line: 0,
        });
    }

    if dv.rpu_present_flag > 0 && dv.rpu_count == 0 {
        notes.push(Note {
            severity: Severity::Info,
            code: Code::MxfInvalidStructure,
            message: "Dolby Vision RPU flagged but frame count not available from metadata".into(),
            file,
            line: 0,
        });
    }

    notes
}

// ════════════════════════════════════════════════════════════════════════════════
// 3. Dolby Atmos IAB Deep Inspection
// ════════════════════════════════════════════════════════════════════════════════

/// Dolby Atmos IAB analysis result.
#[derive(Debug, Clone, Default)]
pub struct AtmosIabInfo {
    pub detected: bool,
    pub channel_count: u32,
    pub sample_rate: f64,
    pub bit_depth: u8,
    pub frame_count: u32,
    pub bed_count: u32,
    pub object_count: u32,
    pub version: String,
}

/// Parse Atmos IAB from audio MXF.
pub fn parse_atmos_iab(mxf_path: &Path) -> AtmosIabInfo {
    let mut info = AtmosIabInfo::default();

    let cmd = format!(
        "ffprobe -v quiet -select_streams a:0 -show_entries \
         stream=channels,channel_layout,sample_rate,bits_per_raw_sample,codec_long_name,nb_frames \
         -show_entries stream_tags=handler_name -of json \"{}\" 2>/dev/null",
        mxf_path.display()
    );
    let output = run_cmd(&cmd);

    let mut channels: u32 = 0;
    let mut sample_rate: f64 = 0.0;
    let mut bit_depth: u8 = 0;
    let mut frame_count: u32 = 0;
    let mut is_atmos = false;

    if !output.is_empty() {
        let ch_re = regex_lite::Regex::new(r#""channels"\s*:\s*(\d+)"#).unwrap();
        if let Some(cap) = ch_re.captures(&output) {
            channels = cap[1].parse().unwrap_or(0);
        }

        let sr_re = regex_lite::Regex::new(r#""sample_rate"\s*:\s*"?(\d+)"#).unwrap();
        if let Some(cap) = sr_re.captures(&output) {
            sample_rate = cap[1].parse().unwrap_or(0.0);
        }

        let bd_re = regex_lite::Regex::new(r#""bits_per_raw_sample"\s*:\s*"?(\d+)"#).unwrap();
        if let Some(cap) = bd_re.captures(&output) {
            bit_depth = cap[1].parse().unwrap_or(0);
        }

        let fc_re = regex_lite::Regex::new(r#""nb_frames"\s*:\s*"?(\d+)"#).unwrap();
        if let Some(cap) = fc_re.captures(&output) {
            frame_count = cap[1].parse().unwrap_or(0);
        }

        if output.contains("Atmos") || output.contains("atmos") || output.contains("IAB") {
            is_atmos = true;
        }
        if channels >= 16 {
            is_atmos = true;
        }
    }

    if !is_atmos {
        return info;
    }

    info.detected = true;
    info.channel_count = channels;
    info.sample_rate = sample_rate;
    info.bit_depth = bit_depth;
    info.frame_count = frame_count;

    // Decompose beds/objects
    if channels >= 12 {
        info.bed_count = 12; // 7.1.4 bed
        info.object_count = channels - 12;
    } else if channels >= 10 {
        info.bed_count = 10; // 7.1.2 bed
        info.object_count = channels - 10;
    } else {
        info.bed_count = channels;
        info.object_count = 0;
    }

    // Estimate objects from IAB packet size
    let pkt_cmd = format!(
        "ffprobe -v quiet -select_streams a:0 -show_packets -read_intervals '%+#1' \
         -show_entries packet=size -of csv=p=0 \"{}\" 2>/dev/null",
        mxf_path.display()
    );
    let pkt_out = run_cmd(&pkt_cmd);
    if let Ok(pkt_size) = pkt_out.trim().parse::<u32>()
        && pkt_size > 100_000
        && info.object_count == 0
    {
        info.object_count = pkt_size.saturating_sub(2048) / 200;
    }

    info
}

/// Check Atmos IAB compliance (ST 2098-2).
pub fn check_atmos_compliance(info: &AtmosIabInfo, source: &Path) -> Vec<Note> {
    let mut notes = Vec::new();
    if !info.detected {
        return notes;
    }

    let file = Some(source.to_path_buf());

    notes.push(Note {
        severity: Severity::Info,
        code: Code::SoundInvalidChannelCount,
        message: format!(
            "Dolby Atmos IAB: {} channels, {} beds, ~{} objects",
            info.channel_count, info.bed_count, info.object_count
        ),
        file: file.clone(),
        line: 0,
    });

    if info.sample_rate != 48000.0 && info.sample_rate != 96000.0 {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::SoundInvalidSampleRate,
            message: format!(
                "Atmos IAB sample rate should be 48kHz or 96kHz, got {}Hz",
                info.sample_rate as u32
            ),
            file: file.clone(),
            line: 0,
        });
    }

    if info.bit_depth != 24 {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::SoundInvalidChannelCount,
            message: format!(
                "Atmos IAB typically uses 24-bit audio, got {}-bit",
                info.bit_depth
            ),
            file: file.clone(),
            line: 0,
        });
    }

    if info.object_count > 118 {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::SoundInvalidChannelCount,
            message: format!(
                "Atmos IAB exceeds maximum object count (118), has {}",
                info.object_count
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

/// HDR type classification.
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

/// HDR metadata from a picture MXF.
#[derive(Debug, Clone, Default)]
pub struct HdrMetadata {
    pub detected: bool,
    pub hdr_type: HdrType,
    pub transfer_function: String,
    pub color_primaries: String,
    pub max_cll: u16,
    pub max_fall: u16,
    pub master_display_max: f64,
    pub master_display_min: f64,
}

/// Detect HDR metadata from picture MXF using ffprobe.
pub fn detect_hdr_metadata(mxf_path: &Path) -> HdrMetadata {
    let mut hdr = HdrMetadata::default();

    let cmd = format!(
        "ffprobe -v quiet -select_streams v:0 -show_entries \
         stream=color_transfer,color_primaries,color_space,bits_per_raw_sample \
         -show_entries \
         side_data=side_data_type,max_content,max_average,red_x,red_y,green_x,\
         green_y,blue_x,blue_y,white_point_x,white_point_y,min_luminance,max_luminance \
         -of json \"{}\" 2>/dev/null",
        mxf_path.display()
    );
    let output = run_cmd(&cmd);

    if output.is_empty() {
        return hdr;
    }

    // Parse transfer characteristics
    let transfer_re = regex_lite::Regex::new(r#""color_transfer"\s*:\s*"([^"]+)""#).unwrap();
    if let Some(cap) = transfer_re.captures(&output) {
        let transfer = &cap[1];
        match transfer {
            "smpte2084" | "smpte-st-2084" => {
                hdr.detected = true;
                hdr.hdr_type = HdrType::Pq;
                hdr.transfer_function = "PQ (SMPTE ST 2084)".into();
            }
            "arib-std-b67" | "bt2020-10" | "bt2020-12" => {
                hdr.detected = true;
                hdr.hdr_type = HdrType::Hlg;
                hdr.transfer_function = "HLG (ARIB STD-B67)".into();
            }
            _ => {}
        }
    }

    // Parse color primaries
    let primaries_re = regex_lite::Regex::new(r#""color_primaries"\s*:\s*"([^"]+)""#).unwrap();
    if let Some(cap) = primaries_re.captures(&output) {
        hdr.color_primaries = cap[1].to_string();
        if hdr.color_primaries == "bt2020" {
            hdr.color_primaries = "BT.2020".into();
            if !hdr.detected {
                hdr.detected = true;
                hdr.hdr_type = HdrType::Pq;
                hdr.transfer_function = "unknown (BT.2020 primaries)".into();
            }
        }
    }

    // MaxCLL
    let max_content_re = regex_lite::Regex::new(r#""max_content"\s*:\s*(\d+)"#).unwrap();
    if let Some(cap) = max_content_re.captures(&output) {
        hdr.max_cll = cap[1].parse().unwrap_or(0);
        hdr.detected = true;
    }

    // MaxFALL
    let max_average_re = regex_lite::Regex::new(r#""max_average"\s*:\s*(\d+)"#).unwrap();
    if let Some(cap) = max_average_re.captures(&output) {
        hdr.max_fall = cap[1].parse().unwrap_or(0);
        hdr.detected = true;
    }

    // Mastering display luminance
    let max_lum_re = regex_lite::Regex::new(r#""max_luminance"\s*:\s*"?(\d+)"#).unwrap();
    if let Some(cap) = max_lum_re.captures(&output) {
        hdr.master_display_max = cap[1].parse::<f64>().unwrap_or(0.0) / 10000.0;
        hdr.detected = true;
    }

    let min_lum_re = regex_lite::Regex::new(r#""min_luminance"\s*:\s*"?(\d+)"#).unwrap();
    if let Some(cap) = min_lum_re.captures(&output) {
        hdr.master_display_min = cap[1].parse::<f64>().unwrap_or(0.0) / 10000.0;
    }

    // Classify if detected via metadata but no transfer function
    if hdr.detected && hdr.hdr_type == HdrType::None {
        if hdr.max_cll > 0 || hdr.master_display_max > 0.0 {
            hdr.hdr_type = HdrType::Hdr10;
        } else {
            hdr.hdr_type = HdrType::Pq;
        }
    }

    hdr
}

/// Check HDR compliance for DCI theatrical.
pub fn check_hdr_compliance(hdr: &HdrMetadata, source: &Path) -> Vec<Note> {
    let mut notes = Vec::new();
    if !hdr.detected {
        return notes;
    }

    let file = Some(source.to_path_buf());

    let type_str = match hdr.hdr_type {
        HdrType::Pq => "PQ (SMPTE ST 2084)",
        HdrType::Hlg => "HLG (ARIB STD-B67)",
        HdrType::Hdr10 => "HDR10",
        HdrType::Hdr10Plus => "HDR10+",
        HdrType::DolbyVision => "Dolby Vision",
        HdrType::None => "Unknown",
    };

    notes.push(Note {
        severity: Severity::Info,
        code: Code::PictureInvalidResolution,
        message: format!("HDR content: {type_str} ({})", hdr.transfer_function),
        file: file.clone(),
        line: 0,
    });

    if hdr.color_primaries == "BT.2020" {
        notes.push(Note {
            severity: Severity::Info,
            code: Code::PictureInvalidResolution,
            message: "Wide color gamut: BT.2020".into(),
            file: file.clone(),
            line: 0,
        });
    }

    if hdr.hdr_type == HdrType::Hlg {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::PictureInvalidResolution,
            message: "HLG transfer function uncommon for DCI theatrical release".into(),
            file: file.clone(),
            line: 0,
        });
    }

    if hdr.max_cll > 0 {
        notes.push(Note {
            severity: Severity::Info,
            code: Code::PictureInvalidResolution,
            message: format!(
                "MaxCLL: {} nits, MaxFALL: {} nits",
                hdr.max_cll, hdr.max_fall
            ),
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
}

/// Check Netflix delivery specification for an IMF package.
pub fn check_netflix_delivery(imf_dir: &Path) -> NetflixDeliveryResult {
    let mut result = NetflixDeliveryResult::default();

    // Netflix requires ASSETMAP.xml (not ASSETMAP without extension)
    if imf_dir.join("ASSETMAP").exists() && !imf_dir.join("ASSETMAP.xml").exists() {
        result
            .violations
            .push("Netflix requires ASSETMAP.xml (not ASSETMAP without extension)".into());
    }

    // Check CPL for ApplicationIdentification and EditRate
    let entries = match std::fs::read_dir(imf_dir) {
        Ok(e) => e,
        Err(_) => {
            result.compliant = result.violations.is_empty();
            return result;
        }
    };

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

        // Check ApplicationIdentification
        let app_re = regex_lite::Regex::new(
            r"<ApplicationIdentification>([^<]+)</ApplicationIdentification>",
        )
        .unwrap();
        if let Some(cap) = app_re.captures(&content) {
            result.app_id = cap[1].to_string();
            if !result.app_id.contains("2067-21") && !result.app_id.contains("2067-20") {
                result.violations.push(format!(
                    "ApplicationIdentification '{}' may not be Netflix-accepted (expected App2E/ST 2067-21)",
                    result.app_id
                ));
            }
        } else {
            result
                .violations
                .push("CPL missing ApplicationIdentification (Netflix requires App2E)".into());
        }

        // Check EditRate
        let rate_re = regex_lite::Regex::new(r"<EditRate>([^<]+)</EditRate>").unwrap();
        if let Some(cap) = rate_re.captures(&content) {
            let edit_rate = &cap[1];
            let accepted_rates = [
                "24000 1001",
                "24 1",
                "25 1",
                "30000 1001",
                "50 1",
                "60000 1001",
                "48 1",
            ];
            let rate_ok = accepted_rates.iter().any(|r| edit_rate.contains(r));
            if !rate_ok {
                result.violations.push(format!(
                    "Edit rate '{edit_rate}' not in Netflix accepted rates"
                ));
            }
        }

        break;
    }

    result.compliant = result.violations.is_empty();
    result
}

/// Convert Netflix result to notes.
pub fn netflix_to_notes(result: &NetflixDeliveryResult, source: &Path) -> Vec<Note> {
    let mut notes = Vec::new();
    let file = Some(source.to_path_buf());

    if result.compliant {
        notes.push(Note {
            severity: Severity::Info,
            code: Code::MissingAssetmap,
            message: "Netflix delivery spec: PASS".into(),
            file,
            line: 0,
        });
    } else {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::MissingAssetmap,
            message: format!(
                "Netflix delivery spec: {} violation(s)",
                result.violations.len()
            ),
            file: file.clone(),
            line: 0,
        });

        for v in &result.violations {
            notes.push(Note {
                severity: Severity::Warning,
                code: Code::MissingAssetmap,
                message: format!("[Netflix] {v}"),
                file: file.clone(),
                line: 0,
            });
        }
    }

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

    let cmd = format!(
        "ffprobe -v quiet -select_streams v:0 -show_entries \
         stream=codec_name,codec_long_name,width,height,r_frame_rate \
         -of json \"{}\" 2>/dev/null",
        mxf_path.display()
    );
    let output = run_cmd(&cmd);

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
// 7. Extended HFR / HBR
// ════════════════════════════════════════════════════════════════════════════════

/// Check for ultra-HFR content (>60fps).
pub fn check_extended_hfr(cpl_path: &Path) -> Vec<Note> {
    let mut notes = Vec::new();

    let content = match std::fs::read_to_string(cpl_path) {
        Ok(c) => c,
        Err(_) => return notes,
    };

    let rate_re = regex_lite::Regex::new(r"<EditRate>(\d+)\s+(\d+)</EditRate>").unwrap();
    let cap = match rate_re.captures(&content) {
        Some(c) => c,
        None => return notes,
    };

    let num: f64 = cap[1].parse().unwrap_or(0.0);
    let den: f64 = cap[2].parse().unwrap_or(1.0);
    if den <= 0.0 {
        return notes;
    }
    let fps = num / den;

    let file = Some(cpl_path.to_path_buf());

    if fps > 60.0 {
        notes.push(Note {
            severity: Severity::Info,
            code: Code::CplInvalidEditRate,
            message: format!("Ultra-HFR content: {} fps", fps as u32),
            file: file.clone(),
            line: 0,
        });

        if fps > 120.0 {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::CplInvalidEditRate,
                message: format!(
                    "Frame rate {} fps exceeds maximum supported rate (120fps)",
                    fps as u32
                ),
                file: file.clone(),
                line: 0,
            });
        }

        notes.push(Note {
            severity: Severity::Info,
            code: Code::J2kBitrateExceeded,
            message: "Ultra-HFR: DCI maximum bitrate is 500 Mbps".into(),
            file,
            line: 0,
        });
    }

    notes
}

// ════════════════════════════════════════════════════════════════════════════════
// 8. Accessibility Track Validation
// ════════════════════════════════════════════════════════════════════════════════

/// Check for accessibility tracks (AD, HI/SDH, CC) in a package.
pub fn check_accessibility(package_dir: &Path) -> Vec<Note> {
    let mut notes = Vec::new();

    let entries = match std::fs::read_dir(package_dir) {
        Ok(e) => e,
        Err(_) => return notes,
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

/// Generate a perceptual hash fingerprint from a picture MXF.
pub fn generate_fingerprint(mxf_path: &Path) -> ContentFingerprint {
    let mut fp = ContentFingerprint::default();

    // Get total frames and resolution
    let dur_cmd = format!(
        "ffprobe -v quiet -select_streams v:0 -show_entries stream=nb_frames,width,height \
         -of csv=p=0 \"{}\" 2>/dev/null",
        mxf_path.display()
    );
    let dur_out = run_cmd(&dur_cmd);
    let parts: Vec<&str> = dur_out.trim().split(',').collect();

    let total_frames: u32 = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
    fp.width = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    fp.height = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);

    // Sample at ~10% into content (skip leader/slate)
    let sample_frame = if total_frames > 10 {
        total_frames / 10
    } else {
        0
    };
    fp.frame_sampled = sample_frame;

    // Extract frame as 8x8 grayscale for a compact 64-bit perceptual hash
    let cmd = format!(
        "ffmpeg -v quiet -i \"{}\" -vf \"select=eq(n\\,{}),scale=8:8,format=gray\" \
         -frames:v 1 -f rawvideo pipe:1 2>/dev/null | xxd -p",
        mxf_path.display(),
        sample_frame
    );
    let output = run_cmd(&cmd);
    let hex = output.replace('\n', "");

    if hex.len() < 128 {
        // Need 64 bytes (8x8) = 128 hex chars
        return fp;
    }

    // Parse pixels and compute average hash
    let mut pixels = [0u8; 64];
    for (i, pixel) in pixels.iter_mut().enumerate() {
        let byte_hex = &hex[i * 2..i * 2 + 2];
        *pixel = u8::from_str_radix(byte_hex, 16).unwrap_or(0);
    }

    let sum: u64 = pixels.iter().map(|&p| p as u64).sum();
    let mean = (sum / 64) as u8;

    // Build 64-bit hash: 1 if pixel > mean, 0 otherwise
    let mut hash_val: u64 = 0;
    for &pixel in &pixels {
        hash_val <<= 1;
        if pixel > mean {
            hash_val |= 1;
        }
    }

    fp.hash = format!("{hash_val:016x}");
    fp
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
    distance as f64 / 64.0
}

// ════════════════════════════════════════════════════════════════════════════════
// Internal helpers
// ════════════════════════════════════════════════════════════════════════════════

fn run_cmd(cmd: &str) -> String {
    std::process::Command::new("sh")
        .args(["-c", cmd])
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
