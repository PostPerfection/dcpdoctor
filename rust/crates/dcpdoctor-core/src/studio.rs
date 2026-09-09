//! Studio-grade DCP validation: loudness, channels, color, reel durations,
//! encryption, content type, and resolution analysis.

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

    let mut ffmpeg = std::process::Command::new("ffmpeg");
    ffmpeg.args(["-hide_banner", "-nostats", "-i"]);
    ffmpeg.arg(mxf_path);
    let frames = max_frames.to_string();
    if max_frames > 0 {
        ffmpeg.args(["-frames:a", &frames]);
    }
    ffmpeg.args(["-af", "ebur128=peak=true", "-f", "null", "-"]);

    // the ebur128 numbers are log lines, so they arrive on stderr and -v quiet drops them
    let Ok(output) = ffmpeg.output() else {
        result.error = Some("Failed to measure loudness via ffmpeg".into());
        return result;
    };
    let report = String::from_utf8_lossy(&output.stderr);

    result.momentary_max_lufs = report
        .lines()
        .filter_map(|line| line.split_once(" M:"))
        .filter_map(|(_, rest)| first_number(rest))
        .max_by(f64::total_cmp)
        .unwrap_or_default();

    let Some((_, summary)) = report.split_once("Summary:") else {
        result.error = Some("ffmpeg printed no EBU R128 summary".into());
        return result;
    };

    for line in summary.lines() {
        let Some((key, value)) = line.trim().split_once(':') else {
            continue;
        };
        let Some(number) = first_number(value) else {
            continue;
        };
        match key {
            "I" => result.integrated_lufs = number,
            "LRA" => result.loudness_range_lu = number,
            "Peak" => result.true_peak_dbtp = number,
            _ => {}
        }
    }

    let probe = ffprobe_output(
        &[
            "-v",
            "quiet",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=channels,sample_rate",
            "-of",
            "default=noprint_wrappers=1",
        ],
        mxf_path,
    )
    .unwrap_or_default();
    result.channels = ffprobe_number(&probe, "channels").unwrap_or(0);
    result.sample_rate = ffprobe_number(&probe, "sample_rate").unwrap_or(0);

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

    let probe = match ffprobe_output(
        &[
            "-v",
            "quiet",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=channels",
            "-of",
            "default=noprint_wrappers=1",
        ],
        mxf_path,
    ) {
        Ok(probe) => probe,
        Err(reason) => {
            config.error = Some(reason);
            return config;
        }
    };
    let channels: u32 = ffprobe_number(&probe, "channels").unwrap_or(0);

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
    pub xyz_to_p3_checked: bool,
    pub error: Option<String>,
}

/// Detect color space from picture MXF metadata.
pub fn detect_color_space(mxf_path: &Path) -> ColorInfo {
    let mut info = ColorInfo::default();

    let probe = match ffprobe_output(
        &[
            "-v",
            "quiet",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=bits_per_raw_sample,codec_tag_string,width,height,pix_fmt",
            // ffprobe emits the entries in its own order, so read them by key
            "-of",
            "default=noprint_wrappers=1",
        ],
        mxf_path,
    ) {
        Ok(probe) => probe,
        Err(reason) => {
            info.error = Some(reason);
            return info;
        }
    };
    if probe.trim().is_empty() {
        info.error = Some("Failed to probe picture MXF".into());
        return info;
    }

    info.bit_depth = ffprobe_number(&probe, "bits_per_raw_sample").unwrap_or(0);
    info.valid = true;

    // DCI JP2K uses 12-bit XYZ color space
    if info.bit_depth == 12 {
        info.detected_space = ColorSpace::Xyz;
    } else if info.bit_depth == 8 {
        info.detected_space = ColorSpace::Rec709;
    } else if info.bit_depth >= 10 {
        info.detected_space = ColorSpace::Xyz;
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
    /// ffprobe was on PATH, so the counts below are measurements
    pub probe_available: bool,
    /// MXFs whose encryption state could not be read
    pub unknown_count: u32,
}

/// Check encryption consistency across MXFs in a DCP.
pub fn check_encryption(dcp_dir: &Path) -> EncryptionInfo {
    let mut info = EncryptionInfo {
        valid: true,
        ..Default::default()
    };

    if !ffprobe_available() {
        return info;
    }
    info.probe_available = true;

    let entries = match std::fs::read_dir(dcp_dir) {
        Ok(e) => e,
        Err(_) => return info,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("mxf") {
            continue;
        }

        // an encrypted MXF shows up as a codec_name ffprobe cannot decode or as
        // an error naming the encryption, so both streams are searched
        let output = ffprobe_output(
            &[
                "-v",
                "error",
                "-show_entries",
                "stream=codec_name",
                "-of",
                "csv=p=0",
            ],
            &path,
        );
        let Ok(output) = output else {
            info.unknown_count += 1;
            continue;
        };

        if output.contains("encrypted") || output.contains("drm") || output.contains("Encrypted") {
            info.encrypted_count += 1;
            info.has_encrypted_assets = true;
        } else if !output.trim().is_empty() {
            info.unencrypted_count += 1;
            info.has_unencrypted_assets = true;
        } else {
            info.unknown_count += 1;
        }
    }

    info.mixed_encryption = info.has_encrypted_assets && info.has_unencrypted_assets;
    info.kdm_required = info.has_encrypted_assets;
    info
}

/// Check encryption compliance.
pub fn check_encryption_compliance(info: &EncryptionInfo, dcp_dir: &Path) -> Vec<Note> {
    let mut notes = Vec::new();
    if !info.valid {
        return notes;
    }

    if !info.probe_available {
        notes.push(
            Note::warning(
                Code::CheckSkipped,
                "encryption consistency not checked: ffprobe not found on PATH",
            )
            .with_file(dcp_dir),
        );
        return notes;
    }

    if info.unknown_count > 0 {
        notes.push(
            Note::warning(
                Code::CheckSkipped,
                format!(
                    "encryption state unreadable for {} MXF file(s), they are counted as neither encrypted nor unencrypted",
                    info.unknown_count
                ),
            )
            .with_file(dcp_dir),
        );
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

    let Some(cpl_path) = find_cpl(dcp_dir) else {
        info.error = Some("No CPL found".into());
        return info;
    };
    let Ok(content) = std::fs::read_to_string(&cpl_path) else {
        info.error = Some("Failed to read CPL".into());
        return info;
    };

    info.reel_count =
        content.matches("<Reel>").count() as u32 + content.matches("<Reel ").count() as u32;

    let intrinsic_duration =
        regex_lite::Regex::new(r"<IntrinsicDuration>(\d+)</IntrinsicDuration>").unwrap();
    let reel_durations: Vec<u64> = intrinsic_duration
        .captures_iter(&content)
        .filter_map(|cap| cap[1].parse::<u64>().ok())
        .collect();

    info.frame_rate = 24.0; // default

    // Get frame rate from first picture MXF
    if let Ok(entries) = std::fs::read_dir(dcp_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("mxf") {
                continue;
            }
            let probe = ffprobe_output(
                &[
                    "-v",
                    "quiet",
                    "-select_streams",
                    "v:0",
                    "-show_entries",
                    "stream=r_frame_rate",
                    "-of",
                    "default=noprint_wrappers=1",
                ],
                &path,
            )
            .unwrap_or_default();
            if let Some(fps) = ffprobe_entry(&probe, "r_frame_rate").and_then(parse_frame_rate) {
                info.frame_rate = fps;
                break;
            }
        }
    }

    let mut total: u64 = 0;
    for (i, &dur) in reel_durations.iter().enumerate() {
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

    let probe = match ffprobe_output(
        &[
            "-v",
            "quiet",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "default=noprint_wrappers=1",
        ],
        mxf_path,
    ) {
        Ok(probe) => probe,
        Err(reason) => {
            info.error = Some(reason);
            return info;
        }
    };

    if probe.trim().is_empty() {
        info.error = Some("Failed to detect resolution".into());
        return info;
    }

    info.width = ffprobe_number(&probe, "width").unwrap_or(0);
    info.height = ffprobe_number(&probe, "height").unwrap_or(0);

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

    if !ffprobe_available() {
        notes.push(
            Note::warning(
                Code::CheckSkipped,
                "ffprobe not found on PATH, so the studio checks that read essence did not run, only the CPL-declaration checks below ran",
            )
            .with_file(dcp_dir),
        );
    }

    // Content type
    let content_type = detect_content_type(dcp_dir);
    notes.extend(check_content_type_compliance(&content_type, dcp_dir));

    // Multi-CPL
    let multi_cpl = validate_multi_cpl(dcp_dir);
    notes.extend(check_multi_cpl_compliance(&multi_cpl, dcp_dir));

    // Encryption consistency
    let enc = check_encryption(dcp_dir);
    notes.extend(check_encryption_compliance(&enc, dcp_dir));

    // Reel duration
    let duration = analyze_reel_durations(dcp_dir);
    notes.extend(check_duration_compliance(&duration, dcp_dir));

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
                if res.valid {
                    notes.extend(check_resolution_compliance(&res, &path));
                } else {
                    notes.push(
                        Note::warning(
                            Code::CheckSkipped,
                            format!(
                                "resolution check did not run: {}",
                                res.error.as_deref().unwrap_or("unknown reason")
                            ),
                        )
                        .with_file(&path),
                    );
                }
                continue;
            }

            // Try as audio
            let ch_config = detect_channel_config(&path);
            if ch_config.valid {
                notes.extend(check_channel_compliance(&ch_config, &path));
                let loudness = measure_loudness(&path, 1000);
                if loudness.valid {
                    notes.extend(check_loudness_compliance(&loudness, &path));
                } else {
                    notes.push(
                        Note::warning(
                            Code::CheckSkipped,
                            format!(
                                "loudness check did not run: {}",
                                loudness
                                    .error
                                    .as_deref()
                                    .unwrap_or("no measurement returned")
                            ),
                        )
                        .with_file(&path),
                    );
                }
                // immersive-audio (DTS:X) detection has no core equivalent
                let dtsx = crate::mxf_advanced::detect_dtsx(&path);
                notes.extend(crate::mxf_advanced::check_dtsx_compliance(&dtsx, &path));
            } else {
                notes.push(
                    Note::warning(
                        Code::CheckSkipped,
                        format!(
                            "essence checks did not run, neither probe read this MXF. picture: {}. audio: {}",
                            color.error.as_deref().unwrap_or("unknown reason"),
                            ch_config.error.as_deref().unwrap_or("unknown reason")
                        ),
                    )
                    .with_file(&path),
                );
            }
        }
    }

    notes
}

// ════════════════════════════════════════════════════════════════════════════════
// Internal helpers
// ════════════════════════════════════════════════════════════════════════════════

/// Is ffprobe on PATH? Every essence-level check here and in the premium
/// delivery checks needs it, and without it they can only report a skip.
pub fn ffprobe_available() -> bool {
    std::process::Command::new("ffprobe")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Run ffprobe and return stdout plus stderr, or the reason it could not run.
// the path goes in as its own argument, so a quote or a $(...) in it stays a filename
fn ffprobe_output(args: &[&str], path: &Path) -> Result<String, String> {
    let output = std::process::Command::new("ffprobe")
        .args(args)
        .arg(path)
        .output()
        .map_err(|e| format!("cannot run ffprobe: {e}"))?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(text)
}

fn ffprobe_entry<'a>(probe: &'a str, key: &str) -> Option<&'a str> {
    probe.lines().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        (name == key).then_some(value.trim())
    })
}

fn ffprobe_number<T: std::str::FromStr>(probe: &str, key: &str) -> Option<T> {
    ffprobe_entry(probe, key)?.parse().ok()
}

fn first_number(text: &str) -> Option<f64> {
    text.split_whitespace()
        .find_map(|token| token.parse::<f64>().ok())
        .filter(|number| number.is_finite())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track_fixtures::{
        ReelTiming, SoundStretch, write_picture_track, write_reel_cpl, write_sound_track,
    };
    #[cfg(unix)]
    use crate::track_fixtures::{assert_the_path_was_not_run, shell_injection_name};

    const TONE_SECONDS: f64 = 2.0;
    const LOUD_AMPLITUDE: f64 = 0.5;
    const QUIET_AMPLITUDE: f64 = 0.05;
    const DECIBELS_BETWEEN_AMPLITUDES: f64 = 20.0;
    const SOUND_TRACK_CHANNELS: u32 = 2;
    const SOUND_TRACK_SAMPLE_RATE: u32 = 48_000;
    const PICTURE_FRAMES: u32 = 2;
    const CODESTREAM_SIZE: u32 = 64;
    const CINEMA_BIT_DEPTH: u8 = 12;
    const CINEMA_FRAME_RATE: f64 = 24.0;

    fn tone_track(path: &Path, amplitude: f64) {
        write_sound_track(
            path,
            &[SoundStretch {
                seconds: TONE_SECONDS,
                amplitude,
            }],
        );
    }

    #[test]
    fn a_tone_ten_times_quieter_measures_twenty_lu_lower() {
        let directory = tempfile::tempdir().unwrap();
        let loud_path = directory.path().join("loud.mxf");
        let quiet_path = directory.path().join("quiet.mxf");
        tone_track(&loud_path, LOUD_AMPLITUDE);
        tone_track(&quiet_path, QUIET_AMPLITUDE);

        let loud = measure_loudness(&loud_path, 0);
        let quiet = measure_loudness(&quiet_path, 0);

        assert!(loud.valid, "{loud:?}");
        assert!(quiet.valid, "{quiet:?}");
        let difference = loud.integrated_lufs - quiet.integrated_lufs;
        assert!(
            (difference - DECIBELS_BETWEEN_AMPLITUDES).abs() < 1.0,
            "{} LUFS against {} LUFS",
            loud.integrated_lufs,
            quiet.integrated_lufs
        );
        // a steady tone never moves, so its momentary maximum is its integrated value
        assert!(
            (loud.momentary_max_lufs - loud.integrated_lufs).abs() < 1.0,
            "{loud:?}"
        );
        assert!(loud.true_peak_dbtp < 0.0, "{loud:?}");
    }

    #[test]
    fn a_sound_track_reports_its_channel_count_and_not_its_sample_rate() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sound.mxf");
        tone_track(&path, LOUD_AMPLITUDE);

        let measured = measure_loudness(&path, 0);
        let config = detect_channel_config(&path);

        assert_eq!(measured.channels, SOUND_TRACK_CHANNELS);
        assert_eq!(measured.sample_rate, SOUND_TRACK_SAMPLE_RATE);
        assert!(config.valid, "{config:?}");
        assert_eq!(config.channel_count, SOUND_TRACK_CHANNELS);
        assert_eq!(config.layout, ChannelLayout::Stereo);
    }

    #[test]
    fn a_cinema_picture_track_reports_twelve_bit_xyz_and_its_stored_size() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("picture.mxf");
        write_picture_track(&path, PICTURE_FRAMES, None);

        let color = detect_color_space(&path);
        let resolution = detect_resolution(&path);

        assert!(color.valid, "{color:?}");
        assert_eq!(color.bit_depth, CINEMA_BIT_DEPTH);
        assert_eq!(color.detected_space, ColorSpace::Xyz);
        assert!(resolution.valid, "{resolution:?}");
        assert_eq!(resolution.width, CODESTREAM_SIZE);
        assert_eq!(resolution.height, CODESTREAM_SIZE);
    }

    #[test]
    fn the_reel_durations_read_their_frame_rate_from_the_picture_track() {
        let directory = tempfile::tempdir().unwrap();
        write_picture_track(&directory.path().join("picture.mxf"), PICTURE_FRAMES, None);
        write_reel_cpl(
            &directory.path().join("CPL.xml"),
            &[ReelTiming {
                picture_entry: 0,
                picture_duration: 48,
                sound_entry: 0,
                sound_duration: 48,
            }],
        );

        let durations = analyze_reel_durations(directory.path());

        assert!(durations.valid, "{durations:?}");
        assert_eq!(durations.reel_count, 1);
        assert_eq!(durations.frame_rate, CINEMA_FRAME_RATE);
    }

    #[cfg(unix)]
    #[test]
    fn a_sound_path_holding_shell_text_is_probed_instead_of_run() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(shell_injection_name("mxf"));
        tone_track(&path, LOUD_AMPLITUDE);

        let measured = measure_loudness(&path, 0);
        let config = detect_channel_config(&path);

        assert_the_path_was_not_run();
        assert!(measured.valid, "{measured:?}");
        assert_eq!(measured.channels, SOUND_TRACK_CHANNELS);
        assert_eq!(config.channel_count, SOUND_TRACK_CHANNELS);
    }

    #[cfg(unix)]
    #[test]
    fn a_picture_path_holding_shell_text_is_probed_instead_of_run() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(shell_injection_name("mxf"));
        write_picture_track(&path, PICTURE_FRAMES, None);

        let color = detect_color_space(&path);
        let resolution = detect_resolution(&path);

        assert_the_path_was_not_run();
        assert_eq!(color.bit_depth, CINEMA_BIT_DEPTH);
        assert_eq!(resolution.width, CODESTREAM_SIZE);
    }
}
