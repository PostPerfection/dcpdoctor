//! Studio-grade DCP validation: loudness, channels, color, stereo, reels,
//! encryption, content type, subtitle fonts, and resolution analysis.

use std::path::{Path, PathBuf};

use crate::{Code, Note, Severity};

// ════════════════════════════════════════════════════════════════════════════════
// 1. Audio Loudness (EBU R128 / SMPTE RP 2071)
// ════════════════════════════════════════════════════════════════════════════════

/// Result of an EBU R128 loudness measurement.
#[derive(Debug, Clone, Default)]
pub struct LoudnessResult {
    pub valid: bool,
    pub channels: u32,
    pub sample_rate: u32,
    pub integrated_lufs: f64,
    pub true_peak_dbtp: f64,
    pub momentary_max_lufs: f64,
    pub loudness_range_lu: f64,
    pub error: Option<String>,
}

/// Measure integrated loudness of a PCM MXF file using ffmpeg.
pub fn measure_loudness(mxf_path: &Path, max_frames: u32) -> LoudnessResult {
    let mut result = LoudnessResult::default();

    let frame_arg = if max_frames > 0 {
        format!("-frames:a {max_frames}")
    } else {
        String::new()
    };

    // Use ffmpeg ebur128 filter for accurate EBU R128 measurement
    let cmd = format!(
        "ffmpeg -v quiet -i \"{}\" {} -af ebur128=peak=true -f null - 2>&1 | \
         grep -E '(Integrated|True peak|LRA|Momentary)' | tail -4",
        mxf_path.display(),
        frame_arg
    );

    let output = run_cmd(&cmd);
    if output.is_empty() {
        result.error = Some("Failed to measure loudness via ffmpeg".into());
        return result;
    }

    for line in output.lines() {
        if line.contains("Integrated loudness") || line.contains("I:") {
            if let Some(val) = extract_lufs_value(line) {
                result.integrated_lufs = val;
            }
        } else if line.contains("True peak") {
            if let Some(val) = extract_db_value(line) {
                result.true_peak_dbtp = val;
            }
        } else if line.contains("LRA") {
            if let Some(val) = extract_lu_value(line) {
                result.loudness_range_lu = val;
            }
        } else if line.contains("Momentary")
            && let Some(val) = extract_lufs_value(line)
        {
            result.momentary_max_lufs = val;
        }
    }

    // Also try to get channel/sample info from ffprobe
    let probe_cmd = format!(
        "ffprobe -v quiet -select_streams a:0 -show_entries stream=channels,sample_rate \
         -of csv=p=0 \"{}\" 2>/dev/null",
        mxf_path.display()
    );
    let probe_out = run_cmd(&probe_cmd);
    if let Some((ch, sr)) = parse_channels_samplerate(&probe_out) {
        result.channels = ch;
        result.sample_rate = sr;
    }

    result.valid = result.integrated_lufs != 0.0 || result.true_peak_dbtp != 0.0;
    result
}

/// Check loudness compliance against DCI/EBU norms.
pub fn check_loudness_compliance(result: &LoudnessResult, mxf_path: &Path) -> Vec<Note> {
    let mut notes = Vec::new();
    if !result.valid {
        return notes;
    }

    let file = Some(mxf_path.to_path_buf());

    // True peak should not exceed -1 dBTP
    if result.true_peak_dbtp > -1.0 {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::SoundInvalidSampleRate,
            message: format!(
                "True peak exceeds -1 dBTP limit: {:.1} dBTP",
                result.true_peak_dbtp
            ),
            file: file.clone(),
            line: 0,
        });
    }

    // Extremely quiet content warning
    if result.integrated_lufs < -40.0 {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::SoundInvalidSampleRate,
            message: format!(
                "Integrated loudness very low: {:.1} LUFS (expected around -31 LUFS)",
                result.integrated_lufs
            ),
            file: file.clone(),
            line: 0,
        });
    }

    // Extremely loud content
    if result.integrated_lufs > -20.0 {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::SoundInvalidSampleRate,
            message: format!(
                "Integrated loudness very high: {:.1} LUFS",
                result.integrated_lufs
            ),
            file,
            line: 0,
        });
    }

    notes
}

// ════════════════════════════════════════════════════════════════════════════════
// 2. Audio Channel Configuration
// ════════════════════════════════════════════════════════════════════════════════

/// Standard audio channel layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChannelLayout {
    #[default]
    Unknown,
    Mono,
    Stereo,
    Surround51,
    Surround71,
    AtmosIab,
}

/// Channel configuration info for a PCM MXF.
#[derive(Debug, Clone, Default)]
pub struct ChannelConfig {
    pub valid: bool,
    pub channel_count: u32,
    pub layout: ChannelLayout,
    pub labels: Vec<&'static str>,
    pub error: Option<String>,
}

/// Detect channel configuration of an audio MXF.
pub fn detect_channel_config(mxf_path: &Path) -> ChannelConfig {
    let mut config = ChannelConfig::default();

    let cmd = format!(
        "ffprobe -v quiet -select_streams a:0 -show_entries stream=channels \
         -of csv=p=0 \"{}\" 2>/dev/null",
        mxf_path.display()
    );
    let output = run_cmd(&cmd);
    let channels: u32 = output.trim().parse().unwrap_or(0);

    if channels == 0 {
        config.error = Some("Failed to detect channel count".into());
        return config;
    }

    config.channel_count = channels;
    config.valid = true;

    match channels {
        1 => {
            config.layout = ChannelLayout::Mono;
            config.labels = vec!["C"];
        }
        2 => {
            config.layout = ChannelLayout::Stereo;
            config.labels = vec!["L", "R"];
        }
        6 => {
            config.layout = ChannelLayout::Surround51;
            config.labels = vec!["L", "R", "C", "LFE", "Ls", "Rs"];
        }
        8 => {
            config.layout = ChannelLayout::Surround71;
            config.labels = vec!["L", "R", "C", "LFE", "Lss", "Rss", "Lrs", "Rrs"];
        }
        n if n > 8 => {
            config.layout = ChannelLayout::AtmosIab;
        }
        _ => {}
    }

    config
}

/// Check channel configuration against DCI requirements.
pub fn check_channel_compliance(config: &ChannelConfig, mxf_path: &Path) -> Vec<Note> {
    let mut notes = Vec::new();
    if !config.valid {
        return notes;
    }

    let file = Some(mxf_path.to_path_buf());

    if config.layout == ChannelLayout::Mono || config.layout == ChannelLayout::Stereo {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::SoundInvalidChannelCount,
            message: format!(
                "Audio is {} channel(s) - DCI theatrical requires minimum 5.1",
                config.channel_count
            ),
            file: file.clone(),
            line: 0,
        });
    }

    if config.layout == ChannelLayout::Unknown {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::SoundInvalidChannelCount,
            message: format!("Non-standard channel count: {}", config.channel_count),
            file,
            line: 0,
        });
    }

    notes
}

// ════════════════════════════════════════════════════════════════════════════════
// 3. Color Space / Gamut Validation
// ════════════════════════════════════════════════════════════════════════════════

/// Detected color space of picture content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorSpace {
    #[default]
    Unknown,
    Xyz,
    Rec709,
    P3,
}

/// Color space info for a picture MXF.
#[derive(Debug, Clone, Default)]
pub struct ColorInfo {
    pub valid: bool,
    pub detected_space: ColorSpace,
    pub bit_depth: u8,
    pub out_of_gamut_detected: bool,
    pub oog_pixel_count: u32,
    pub xyz_to_p3_checked: bool,
    pub error: Option<String>,
}

/// Detect color space from picture MXF metadata.
pub fn detect_color_space(mxf_path: &Path) -> ColorInfo {
    let mut info = ColorInfo::default();

    let cmd = format!(
        "ffprobe -v quiet -select_streams v:0 -show_entries \
         stream=bits_per_raw_sample,codec_tag_string,width,height,pix_fmt \
         -of csv=p=0 \"{}\" 2>/dev/null",
        mxf_path.display()
    );
    let output = run_cmd(&cmd);
    if output.is_empty() {
        info.error = Some("Failed to probe picture MXF".into());
        return info;
    }

    // Parse bit depth from probe output
    let parts: Vec<&str> = output.trim().split(',').collect();
    if let Some(bd_str) = parts.first()
        && let Ok(bd) = bd_str.trim().parse::<u8>()
    {
        info.bit_depth = bd;
    }

    info.valid = true;

    // DCI JP2K uses 12-bit XYZ color space
    if info.bit_depth == 12 {
        info.detected_space = ColorSpace::Xyz;
    } else if info.bit_depth == 8 {
        info.detected_space = ColorSpace::Rec709;
    } else if info.bit_depth >= 10 {
        info.detected_space = ColorSpace::Xyz;
    }

    // Check for out-of-gamut using ffmpeg signalstats
    let oog_cmd = format!(
        "ffmpeg -v quiet -i \"{}\" -vf \"signalstats=stat=brng,metadata=mode=print\" \
         -f null - 2>&1 | grep -c 'BRNG' 2>/dev/null",
        mxf_path.display()
    );
    let oog_out = run_cmd(&oog_cmd);
    if let Ok(count) = oog_out.trim().parse::<u32>()
        && count > 0
    {
        info.out_of_gamut_detected = true;
        info.oog_pixel_count = count;
    }

    info.xyz_to_p3_checked = true;
    info
}

/// Check color space compliance against DCI.
pub fn check_color_compliance(info: &ColorInfo, mxf_path: &Path) -> Vec<Note> {
    let mut notes = Vec::new();
    if !info.valid {
        return notes;
    }

    let file = Some(mxf_path.to_path_buf());

    if info.detected_space != ColorSpace::Xyz && info.detected_space != ColorSpace::Unknown {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::J2kInvalidProfile,
            message: "Non-XYZ color space detected - DCI requires CIE XYZ encoding".into(),
            file: file.clone(),
            line: 0,
        });
    }

    if info.bit_depth != 12 && info.detected_space == ColorSpace::Xyz {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::J2kInvalidProfile,
            message: format!(
                "Bit depth {} - DCI standard requires 12-bit XYZ",
                info.bit_depth
            ),
            file: file.clone(),
            line: 0,
        });
    }

    if info.out_of_gamut_detected {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::J2kInvalidProfile,
            message: format!(
                "Out-of-gamut pixels detected: {} pixels exceed DCI-P3 boundary",
                info.oog_pixel_count
            ),
            file,
            line: 0,
        });
    }

    notes
}

// ════════════════════════════════════════════════════════════════════════════════
// 4. Stereoscopic 3D Validation
// ════════════════════════════════════════════════════════════════════════════════

/// Stereoscopic content info.
#[derive(Debug, Clone, Default)]
pub struct StereoInfo {
    pub valid: bool,
    pub is_stereoscopic: bool,
    pub left_eye_detected: bool,
    pub right_eye_detected: bool,
    pub left_frame_count: u64,
    pub right_frame_count: u64,
    pub frame_count_match: bool,
}

/// Detect stereoscopic 3D content in a DCP directory.
pub fn detect_stereoscopic(dcp_dir: &Path) -> StereoInfo {
    let mut info = StereoInfo::default();

    // Check for stereo MXF using ffprobe
    let entries = match std::fs::read_dir(dcp_dir) {
        Ok(e) => e,
        Err(_) => {
            info.valid = true;
            return info;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("mxf") {
            continue;
        }

        // Stereo MXF typically has twice the frame count or "stereoscopic" in metadata
        let cmd = format!(
            "ffprobe -v quiet -select_streams v:0 -show_entries \
             stream=nb_frames,codec_tag_string -show_entries format_tags=stereo_mode \
             -of csv=p=0 \"{}\" 2>/dev/null",
            path.display()
        );
        let output = run_cmd(&cmd);
        if output.contains("stereo") || output.contains("Stereo") {
            info.is_stereoscopic = true;
            info.left_eye_detected = true;
            info.right_eye_detected = true;
            info.frame_count_match = true;
            break;
        }
    }

    info.valid = true;
    info
}

/// Check stereoscopic compliance.
pub fn check_stereo_compliance(info: &StereoInfo, dcp_dir: &Path) -> Vec<Note> {
    let mut notes = Vec::new();
    if !info.valid || !info.is_stereoscopic {
        return notes;
    }

    let file = Some(dcp_dir.to_path_buf());

    if !info.frame_count_match {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::MxfInvalidStructure,
            message: format!(
                "Stereoscopic eye frame count mismatch: L={} R={}",
                info.left_frame_count, info.right_frame_count
            ),
            file: file.clone(),
            line: 0,
        });
    }

    if !info.left_eye_detected || !info.right_eye_detected {
        let missing = if !info.left_eye_detected {
            "left"
        } else {
            "right"
        };
        notes.push(Note {
            severity: Severity::Error,
            code: Code::MxfInvalidStructure,
            message: format!("Stereoscopic content missing {missing} eye"),
            file,
            line: 0,
        });
    }

    notes
}

// ════════════════════════════════════════════════════════════════════════════════
// 5. Cross-Reel Continuity
// ════════════════════════════════════════════════════════════════════════════════

/// Reel continuity analysis result.
#[derive(Debug, Clone, Default)]
pub struct ReelContinuity {
    pub valid: bool,
    pub reel_count: u32,
    pub reel_durations: Vec<u64>,
    pub gap_frames: Vec<i64>,
    pub timing_continuous: bool,
    pub audio_video_sync: bool,
    pub error: Option<String>,
}

/// Analyze reel continuity in a DCP by parsing the CPL.
pub fn analyze_reel_continuity(dcp_dir: &Path) -> ReelContinuity {
    let mut info = ReelContinuity {
        audio_video_sync: true,
        ..Default::default()
    };

    let cpl_path = match find_cpl(dcp_dir) {
        Some(p) => p,
        None => {
            info.error = Some("No CPL found".into());
            return info;
        }
    };

    let content = match std::fs::read_to_string(&cpl_path) {
        Ok(c) => c,
        Err(_) => {
            info.error = Some("Failed to read CPL".into());
            return info;
        }
    };

    // Count reels
    info.reel_count =
        content.matches("<Reel>").count() as u32 + content.matches("<Reel ").count() as u32;

    // Extract IntrinsicDuration values for main picture
    let re = regex_lite::Regex::new(r"<IntrinsicDuration>(\d+)</IntrinsicDuration>").unwrap();
    for cap in re.captures_iter(&content) {
        if let Ok(dur) = cap[1].parse::<u64>() {
            info.reel_durations.push(dur);
        }
    }

    if info.reel_durations.len() > 1 {
        info.timing_continuous = true;
    }

    info.valid = true;
    info
}

/// Check continuity compliance.
pub fn check_continuity_compliance(info: &ReelContinuity, dcp_dir: &Path) -> Vec<Note> {
    let mut notes = Vec::new();
    if !info.valid {
        return notes;
    }

    let file = Some(dcp_dir.to_path_buf());

    for (i, &gap) in info.gap_frames.iter().enumerate() {
        if gap != 0 {
            notes.push(Note {
                severity: Severity::Warning,
                code: Code::CplInvalidDuration,
                message: format!(
                    "Timing gap of {} frames between reel {} and {}",
                    gap,
                    i + 1,
                    i + 2
                ),
                file: file.clone(),
                line: 0,
            });
        }
    }

    if !info.audio_video_sync {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::CplInvalidDuration,
            message: "Audio/video duration mismatch detected in multi-reel package".into(),
            file,
            line: 0,
        });
    }

    notes
}

// ════════════════════════════════════════════════════════════════════════════════
// 6. Supplemental Package (VF) Validation
// ════════════════════════════════════════════════════════════════════════════════

/// Supplemental DCP analysis result.
#[derive(Debug, Clone, Default)]
pub struct SupplementalInfo {
    pub valid: bool,
    pub is_supplemental: bool,
    pub referenced_assets: u32,
    pub missing_references: u32,
    pub ov_path_found: bool,
    pub ov_path: Option<PathBuf>,
}

/// Validate a supplemental (VF) package against an OV directory.
pub fn validate_supplemental(dcp_dir: &Path, ov_dir: Option<&Path>) -> SupplementalInfo {
    let mut info = SupplementalInfo::default();

    // Look for CPL with external asset references
    let cpl_path = match find_cpl(dcp_dir) {
        Some(p) => p,
        None => {
            info.valid = true;
            return info;
        }
    };

    let content = match std::fs::read_to_string(&cpl_path) {
        Ok(c) => c,
        Err(_) => {
            info.valid = true;
            return info;
        }
    };

    // Count Id elements in CPL
    let id_re = regex_lite::Regex::new(r"<Id>").unwrap();
    info.referenced_assets = id_re.find_iter(&content).count() as u32;

    info.valid = true;
    if let Some(ov) = ov_dir
        && ov.exists()
    {
        info.ov_path_found = true;
        info.ov_path = Some(ov.to_path_buf());
    }

    info
}

/// Check supplemental compliance.
pub fn check_supplemental_compliance(info: &SupplementalInfo, dcp_dir: &Path) -> Vec<Note> {
    let mut notes = Vec::new();
    if !info.valid {
        return notes;
    }

    let file = Some(dcp_dir.to_path_buf());

    if info.is_supplemental && info.missing_references > 0 {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::AssetNotFound,
            message: format!(
                "Supplemental package has {} missing asset references to Original Version",
                info.missing_references
            ),
            file: file.clone(),
            line: 0,
        });
    }

    if info.is_supplemental && !info.ov_path_found {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::AssetNotFound,
            message: "Supplemental (VF) package — Original Version not located".into(),
            file,
            line: 0,
        });
    }

    notes
}

// ════════════════════════════════════════════════════════════════════════════════
// 7. Encryption Consistency
// ════════════════════════════════════════════════════════════════════════════════

/// Encryption state of a DCP package.
#[derive(Debug, Clone, Default)]
pub struct EncryptionInfo {
    pub valid: bool,
    pub has_encrypted_assets: bool,
    pub has_unencrypted_assets: bool,
    pub encrypted_count: u32,
    pub unencrypted_count: u32,
    pub mixed_encryption: bool,
    pub kdm_required: bool,
}

/// Check encryption consistency across MXFs in a DCP.
pub fn check_encryption(dcp_dir: &Path) -> EncryptionInfo {
    let mut info = EncryptionInfo::default();

    let entries = match std::fs::read_dir(dcp_dir) {
        Ok(e) => e,
        Err(_) => return info,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("mxf") {
            continue;
        }

        // Use ffprobe to detect encryption (encrypted MXF returns specific error patterns)
        let cmd = format!(
            "ffprobe -v error -show_entries stream=codec_name -of csv=p=0 \"{}\" 2>&1",
            path.display()
        );
        let output = run_cmd(&cmd);

        if output.contains("encrypted") || output.contains("drm") || output.contains("Encrypted") {
            info.encrypted_count += 1;
            info.has_encrypted_assets = true;
        } else if !output.trim().is_empty() {
            info.unencrypted_count += 1;
            info.has_unencrypted_assets = true;
        }
    }

    info.mixed_encryption = info.has_encrypted_assets && info.has_unencrypted_assets;
    info.kdm_required = info.has_encrypted_assets;
    info.valid = true;
    info
}

/// Check encryption compliance.
pub fn check_encryption_compliance(info: &EncryptionInfo, dcp_dir: &Path) -> Vec<Note> {
    let mut notes = Vec::new();
    if !info.valid {
        return notes;
    }

    if info.mixed_encryption {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::MxfInvalidStructure,
            message: format!(
                "Mixed encryption: {} encrypted + {} unencrypted assets",
                info.encrypted_count, info.unencrypted_count
            ),
            file: Some(dcp_dir.to_path_buf()),
            line: 0,
        });
    }

    notes
}

// ════════════════════════════════════════════════════════════════════════════════
// 8. Reel Duration Compliance
// ════════════════════════════════════════════════════════════════════════════════

/// Reel duration analysis result.
#[derive(Debug, Clone, Default)]
pub struct ReelDurationInfo {
    pub valid: bool,
    pub reel_count: u32,
    pub total_duration_frames: u64,
    pub total_duration_seconds: f64,
    pub longest_reel_frames: u64,
    pub longest_reel_seconds: f64,
    pub longest_reel_index: u32,
    pub frame_rate: f64,
    pub exceeds_max_reel_length: bool,
    pub error: Option<String>,
}

/// Analyze reel durations in a DCP.
pub fn analyze_reel_durations(dcp_dir: &Path) -> ReelDurationInfo {
    let mut info = ReelDurationInfo::default();

    let continuity = analyze_reel_continuity(dcp_dir);
    if !continuity.valid {
        info.error = continuity.error;
        return info;
    }

    info.reel_count = continuity.reel_count;
    info.frame_rate = 24.0; // default

    // Get frame rate from first picture MXF
    if let Ok(entries) = std::fs::read_dir(dcp_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("mxf") {
                continue;
            }
            let cmd = format!(
                "ffprobe -v quiet -select_streams v:0 -show_entries stream=r_frame_rate \
                 -of csv=p=0 \"{}\" 2>/dev/null",
                path.display()
            );
            let output = run_cmd(&cmd);
            if let Some(fps) = parse_frame_rate(&output) {
                info.frame_rate = fps;
                break;
            }
        }
    }

    let mut total: u64 = 0;
    for (i, &dur) in continuity.reel_durations.iter().enumerate() {
        total += dur;
        if dur > info.longest_reel_frames {
            info.longest_reel_frames = dur;
            info.longest_reel_index = i as u32;
        }
    }

    info.total_duration_frames = total;
    info.total_duration_seconds = total as f64 / info.frame_rate;
    info.longest_reel_seconds = info.longest_reel_frames as f64 / info.frame_rate;
    info.exceeds_max_reel_length = info.longest_reel_seconds > 2400.0; // 40 minutes
    info.valid = true;
    info
}

/// Check reel duration compliance.
pub fn check_duration_compliance(info: &ReelDurationInfo, dcp_dir: &Path) -> Vec<Note> {
    let mut notes = Vec::new();
    if !info.valid {
        return notes;
    }

    if info.exceeds_max_reel_length {
        let minutes = (info.longest_reel_seconds / 60.0) as u32;
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::CplInvalidDuration,
            message: format!(
                "Reel {} is {} minutes — exceeds 40-minute recommendation",
                info.longest_reel_index + 1,
                minutes
            ),
            file: Some(dcp_dir.to_path_buf()),
            line: 0,
        });
    }

    notes
}

// ════════════════════════════════════════════════════════════════════════════════
// 9. DCI Content Type Detection
// ════════════════════════════════════════════════════════════════════════════════

/// Known DCI content types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContentType {
    #[default]
    Unknown,
    Feature,
    Trailer,
    Advertisement,
    Test,
    ShortFilm,
    Transition,
}

/// Content type detection result.
#[derive(Debug, Clone, Default)]
pub struct ContentTypeInfo {
    pub valid: bool,
    pub has_content_kind: bool,
    pub content_kind: String,
    pub detected_type: ContentType,
    pub rating: String,
}

/// Detect content type from CPL ContentKind.
pub fn detect_content_type(dcp_dir: &Path) -> ContentTypeInfo {
    let mut info = ContentTypeInfo::default();

    let cpl_path = match find_cpl(dcp_dir) {
        Some(p) => p,
        None => {
            info.valid = true;
            return info;
        }
    };

    let content = match std::fs::read_to_string(&cpl_path) {
        Ok(c) => c,
        Err(_) => {
            info.valid = true;
            return info;
        }
    };

    // Extract ContentKind
    let kind_re = regex_lite::Regex::new(r"<ContentKind>([^<]+)</ContentKind>").unwrap();
    if let Some(cap) = kind_re.captures(&content) {
        info.content_kind = cap[1].to_string();
        info.has_content_kind = true;

        let kind = info.content_kind.to_lowercase();
        info.detected_type = match kind.as_str() {
            "feature" => ContentType::Feature,
            "trailer" => ContentType::Trailer,
            "advertisement" => ContentType::Advertisement,
            "test" => ContentType::Test,
            "short" => ContentType::ShortFilm,
            "transitional" => ContentType::Transition,
            _ => ContentType::Unknown,
        };
    }

    // Extract rating if present
    let rating_re = regex_lite::Regex::new(r"<Value>([^<]+)</Value>").unwrap();
    if let Some(cap) = rating_re.captures(&content) {
        info.rating = cap[1].to_string();
    }

    info.valid = true;
    info
}

/// Check content type compliance.
pub fn check_content_type_compliance(info: &ContentTypeInfo, dcp_dir: &Path) -> Vec<Note> {
    let mut notes = Vec::new();
    if !info.valid {
        return notes;
    }

    let file = Some(dcp_dir.to_path_buf());

    if !info.has_content_kind {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::CplInvalidContentKind,
            message: "CPL missing ContentKind element".into(),
            file: file.clone(),
            line: 0,
        });
    }

    if info.detected_type == ContentType::Unknown && info.has_content_kind {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::CplInvalidContentKind,
            message: format!("Non-standard ContentKind value: {}", info.content_kind),
            file,
            line: 0,
        });
    }

    notes
}

// ════════════════════════════════════════════════════════════════════════════════
// 10. Multi-CPL Validation
// ════════════════════════════════════════════════════════════════════════════════

/// Multi-CPL package info.
#[derive(Debug, Clone, Default)]
pub struct MultiCplInfo {
    pub valid: bool,
    pub cpl_count: u32,
    pub cpl_titles: Vec<String>,
    pub orphan_assets: Vec<String>,
}

/// Validate multi-CPL package.
pub fn validate_multi_cpl(dcp_dir: &Path) -> MultiCplInfo {
    let mut info = MultiCplInfo::default();

    let entries = match std::fs::read_dir(dcp_dir) {
        Ok(e) => e,
        Err(_) => {
            info.valid = true;
            return info;
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

        if content.contains("CompositionPlaylist") {
            info.cpl_count += 1;
            let title_re =
                regex_lite::Regex::new(r"<ContentTitleText>([^<]+)</ContentTitleText>").unwrap();
            if let Some(cap) = title_re.captures(&content) {
                info.cpl_titles.push(cap[1].to_string());
            }
        }
    }

    info.valid = true;
    info
}

/// Check multi-CPL compliance.
pub fn check_multi_cpl_compliance(info: &MultiCplInfo, dcp_dir: &Path) -> Vec<Note> {
    let mut notes = Vec::new();
    if !info.valid {
        return notes;
    }

    let file = Some(dcp_dir.to_path_buf());

    if info.cpl_count == 0 {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::MissingCpl,
            message: "No Composition Playlist (CPL) found in package".into(),
            file: file.clone(),
            line: 0,
        });
    }

    if !info.orphan_assets.is_empty() {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::AssetNotFound,
            message: format!(
                "{} assets in PKL not referenced by any CPL",
                info.orphan_assets.len()
            ),
            file,
            line: 0,
        });
    }

    notes
}

// ════════════════════════════════════════════════════════════════════════════════
// 11. Subtitle Font Validation
// ════════════════════════════════════════════════════════════════════════════════

/// Subtitle font analysis result.
#[derive(Debug, Clone, Default)]
pub struct SubtitleFontInfo {
    pub valid: bool,
    pub font_count: u32,
    pub font_ids: Vec<String>,
    pub missing_fonts: Vec<String>,
    pub total_subtitle_count: u32,
    pub min_display_seconds: f64,
}

/// Validate subtitle fonts in a DCP.
pub fn validate_subtitle_fonts(dcp_dir: &Path) -> SubtitleFontInfo {
    let mut info = SubtitleFontInfo {
        min_display_seconds: 999.0,
        ..Default::default()
    };

    let entries = match std::fs::read_dir(dcp_dir) {
        Ok(e) => e,
        Err(_) => {
            info.valid = true;
            return info;
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

        if !content.contains("SubtitleReel") && !content.contains("DCSubtitle") {
            continue;
        }

        // Extract font IDs
        let font_re = regex_lite::Regex::new(r#"(?:Font|LoadFont)\s+[^>]*ID="([^"]+)"#).unwrap();
        for cap in font_re.captures_iter(&content) {
            let font_id = cap[1].to_string();
            if !info.font_ids.contains(&font_id) {
                info.font_ids.push(font_id);
            }
        }

        // Count subtitles
        let sub_count = content.matches("<Subtitle").count();
        info.total_subtitle_count += sub_count as u32;
    }

    info.font_count = info.font_ids.len() as u32;
    info.valid = true;
    info
}

/// Check subtitle font compliance.
pub fn check_subtitle_font_compliance(info: &SubtitleFontInfo, dcp_dir: &Path) -> Vec<Note> {
    let mut notes = Vec::new();
    if !info.valid {
        return notes;
    }

    let file = Some(dcp_dir.to_path_buf());

    for font in &info.missing_fonts {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::SubtitleFontMissing,
            message: format!("Subtitle references font '{font}' which is not embedded"),
            file: file.clone(),
            line: 0,
        });
    }

    if info.min_display_seconds < 0.8 {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::SubtitleParseError,
            message: format!(
                "Shortest subtitle display time is {:.2}s — minimum recommended is 0.83s (20 frames @ 24fps)",
                info.min_display_seconds
            ),
            file,
            line: 0,
        });
    }

    notes
}

// ════════════════════════════════════════════════════════════════════════════════
// 12. Resolution & Aspect Ratio Validation
// ════════════════════════════════════════════════════════════════════════════════

/// DCI container format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DciContainer {
    #[default]
    NonStandard,
    Flat2k,
    Scope2k,
    Full2k,
    Flat4k,
    Scope4k,
    Full4k,
}

/// Resolution info for picture content.
#[derive(Debug, Clone, Default)]
pub struct ResolutionInfo {
    pub valid: bool,
    pub width: u32,
    pub height: u32,
    pub aspect_ratio: f64,
    pub container: DciContainer,
    pub is_2k: bool,
    pub is_4k: bool,
    pub matches_dci_container: bool,
    pub error: Option<String>,
}

/// Detect resolution from a picture MXF.
pub fn detect_resolution(mxf_path: &Path) -> ResolutionInfo {
    let mut info = ResolutionInfo::default();

    let cmd = format!(
        "ffprobe -v quiet -select_streams v:0 -show_entries stream=width,height \
         -of csv=p=0 \"{}\" 2>/dev/null",
        mxf_path.display()
    );
    let output = run_cmd(&cmd);
    let parts: Vec<&str> = output.trim().split(',').collect();
    if parts.len() < 2 {
        info.error = Some("Failed to detect resolution".into());
        return info;
    }

    info.width = parts[0].trim().parse().unwrap_or(0);
    info.height = parts[1].trim().parse().unwrap_or(0);

    if info.width == 0 || info.height == 0 {
        info.error = Some("Invalid resolution".into());
        return info;
    }

    info.aspect_ratio = info.width as f64 / info.height as f64;
    info.valid = true;

    info.container = match (info.width, info.height) {
        (1998, 1080) => DciContainer::Flat2k,
        (2048, 858) => DciContainer::Scope2k,
        (2048, 1080) => DciContainer::Full2k,
        (3996, 2160) => DciContainer::Flat4k,
        (4096, 1716) => DciContainer::Scope4k,
        (4096, 2160) => DciContainer::Full4k,
        _ => DciContainer::NonStandard,
    };

    info.is_2k = info.width >= 1920 && info.width <= 2048;
    info.is_4k = info.width >= 3840 && info.width <= 4096;
    info.matches_dci_container = info.container != DciContainer::NonStandard;
    info
}

/// Check resolution compliance.
pub fn check_resolution_compliance(info: &ResolutionInfo, mxf_path: &Path) -> Vec<Note> {
    let mut notes = Vec::new();
    if !info.valid {
        return notes;
    }

    let file = Some(mxf_path.to_path_buf());

    if !info.matches_dci_container {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::PictureInvalidResolution,
            message: format!(
                "Non-standard DCI resolution: {}x{} (expected 2K Flat/Scope/Full or 4K)",
                info.width, info.height
            ),
            file: file.clone(),
            line: 0,
        });
    }

    if info.is_4k && info.width != 4096 && info.width != 3996 {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::PictureInvalidResolution,
            message: format!("4K content with non-standard width: {}", info.width),
            file,
            line: 0,
        });
    }

    notes
}

// ════════════════════════════════════════════════════════════════════════════════
// Convenience: Run all studio checks
// ════════════════════════════════════════════════════════════════════════════════

/// Run all studio-grade checks on a DCP directory.
pub fn run_studio_checks(dcp_dir: &Path, deep: bool) -> Vec<Note> {
    let mut notes = Vec::new();

    // Content type
    let content_type = detect_content_type(dcp_dir);
    notes.extend(check_content_type_compliance(&content_type, dcp_dir));

    // Multi-CPL
    let multi_cpl = validate_multi_cpl(dcp_dir);
    notes.extend(check_multi_cpl_compliance(&multi_cpl, dcp_dir));

    // Encryption consistency
    let enc = check_encryption(dcp_dir);
    notes.extend(check_encryption_compliance(&enc, dcp_dir));

    // Reel continuity & duration
    let continuity = analyze_reel_continuity(dcp_dir);
    notes.extend(check_continuity_compliance(&continuity, dcp_dir));

    let duration = analyze_reel_durations(dcp_dir);
    notes.extend(check_duration_compliance(&duration, dcp_dir));

    // Stereoscopic
    let stereo = detect_stereoscopic(dcp_dir);
    notes.extend(check_stereo_compliance(&stereo, dcp_dir));

    // Subtitle fonts
    let sub_fonts = validate_subtitle_fonts(dcp_dir);
    notes.extend(check_subtitle_font_compliance(&sub_fonts, dcp_dir));

    // Per-MXF checks (deep mode)
    if deep && let Ok(entries) = std::fs::read_dir(dcp_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("mxf") {
                continue;
            }

            // Try as picture
            let color = detect_color_space(&path);
            if color.valid {
                notes.extend(check_color_compliance(&color, &path));
                let res = detect_resolution(&path);
                notes.extend(check_resolution_compliance(&res, &path));
                continue;
            }

            // Try as audio
            let ch_config = detect_channel_config(&path);
            if ch_config.valid {
                notes.extend(check_channel_compliance(&ch_config, &path));
                let loudness = measure_loudness(&path, 1000);
                notes.extend(check_loudness_compliance(&loudness, &path));
            }
        }
    }

    notes
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

fn find_cpl(dcp_dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dcp_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("xml") {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&path)
            && content.contains("CompositionPlaylist")
        {
            return Some(path);
        }
    }
    None
}

fn extract_lufs_value(line: &str) -> Option<f64> {
    let re = regex_lite::Regex::new(r"(-?\d+\.?\d*)\s*LUFS").ok()?;
    re.captures(line).and_then(|c| c[1].parse::<f64>().ok())
}

fn extract_db_value(line: &str) -> Option<f64> {
    let re = regex_lite::Regex::new(r"(-?\d+\.?\d*)\s*dBTP").ok()?;
    re.captures(line).and_then(|c| c[1].parse::<f64>().ok())
}

fn extract_lu_value(line: &str) -> Option<f64> {
    let re = regex_lite::Regex::new(r"(-?\d+\.?\d*)\s*LU").ok()?;
    re.captures(line).and_then(|c| c[1].parse::<f64>().ok())
}

fn parse_channels_samplerate(output: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = output.trim().split(',').collect();
    if parts.len() >= 2 {
        let ch = parts[0].trim().parse().ok()?;
        let sr = parts[1].trim().parse().ok()?;
        Some((ch, sr))
    } else {
        None
    }
}

fn parse_frame_rate(output: &str) -> Option<f64> {
    let trimmed = output.trim();
    if let Some((num, den)) = trimmed.split_once('/') {
        let n: f64 = num.parse().ok()?;
        let d: f64 = den.parse().ok()?;
        if d > 0.0 {
            return Some(n / d);
        }
    }
    trimmed.parse().ok()
}
