//! HDR metadata validation — detect and cross-check HDR10/HLG/PQ metadata.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

/// Transfer function (EOTF).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub enum TransferFunction {
    #[default]
    SdrBt1886,
    Pq,
    Hlg,
    Linear,
}

/// Color primaries / gamut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub enum Colorimetry {
    #[default]
    Bt709,
    Bt2020,
    P3D65,
    P3Dci,
    Aces,
}

/// Content Light Level metadata.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ContentLightLevel {
    pub max_cll: u16,
    pub max_fall: u16,
}

/// Mastering display metadata.
#[derive(Debug, Clone, Default, Serialize)]
pub struct MasteringDisplay {
    pub min_luminance: u32,
    pub max_luminance: u32,
}

/// Detected HDR metadata.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HdrMetadata {
    pub transfer: TransferFunction,
    pub colorimetry: Colorimetry,
    pub bit_depth: u16,
    pub content_light: Option<ContentLightLevel>,
    pub mastering_display: Option<MasteringDisplay>,
}

/// A single HDR validation issue.
#[derive(Debug, Clone, Serialize)]
pub struct HdrIssue {
    pub field: String,
    pub expected: String,
    pub actual: String,
    pub severity: String,
    pub description: String,
}

/// Options for HDR validation.
pub struct HdrValidateOptions {
    pub video_path: PathBuf,
    pub expected_transfer: TransferFunction,
    pub expected_colorimetry: Colorimetry,
    pub expected_bit_depth: u16,
    pub expected_max_cll: u16,
    pub expected_max_fall: u16,
    pub expected_max_luminance: u32,
}

/// Result of HDR validation.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HdrValidateResult {
    pub success: bool,
    pub error: String,
    pub valid: bool,
    pub detected: HdrMetadata,
    pub issues: Vec<HdrIssue>,
}

fn transfer_str(tf: TransferFunction) -> &'static str {
    match tf {
        TransferFunction::Pq => "PQ (ST 2084)",
        TransferFunction::Hlg => "HLG (ARIB STD-B67)",
        TransferFunction::SdrBt1886 => "SDR (BT.1886)",
        TransferFunction::Linear => "Linear",
    }
}

fn colorimetry_str(c: Colorimetry) -> &'static str {
    match c {
        Colorimetry::Bt709 => "BT.709",
        Colorimetry::Bt2020 => "BT.2020",
        Colorimetry::P3D65 => "P3-D65",
        Colorimetry::P3Dci => "P3-DCI",
        Colorimetry::Aces => "ACES",
    }
}

/// Run ffprobe, or return why it could not run. An empty probe output decodes
/// as SDR/BT.709, so a failure must never come back as a string.
fn run_ffprobe(args: &[&str]) -> Result<String, String> {
    let output = Command::new("ffprobe")
        .args(args)
        .output()
        .map_err(|e| format!("cannot run ffprobe: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail = stderr.trim().lines().last().unwrap_or("unknown error");
        return Err(format!("ffprobe exited with an error: {tail}"));
    }

    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(text)
}

/// Validate HDR metadata of a video file.
pub fn validate_hdr_metadata(opts: &HdrValidateOptions) -> HdrValidateResult {
    let mut result = HdrValidateResult::default();

    if !opts.video_path.exists() {
        result.error = "Video file not found".into();
        return result;
    }

    let path_str = opts.video_path.to_string_lossy().to_string();

    // Probe stream-level metadata
    let stream_output = match run_ffprobe(&[
        "-v",
        "error",
        "-select_streams",
        "v:0",
        "-show_entries",
        "stream=color_space,color_primaries,color_transfer,bits_per_raw_sample",
        "-of",
        "json",
        &path_str,
    ]) {
        Ok(o) => o,
        Err(e) => {
            result.error = format!("HDR metadata not read: {e}");
            return result;
        }
    };

    // Probe frame-level side data (CLL, mastering display)
    let frame_output = match run_ffprobe(&[
        "-v",
        "error",
        "-select_streams",
        "v:0",
        "-show_frames",
        "-read_intervals",
        "%+1",
        "-show_entries",
        "frame=side_data_list",
        "-of",
        "json",
        &path_str,
    ]) {
        Ok(o) => o,
        Err(e) => {
            result.error = format!("HDR side data not read: {e}");
            return result;
        }
    };

    // Detect transfer function
    let detected_tf =
        if stream_output.contains("smpte2084") || stream_output.contains("smpte-st-2084") {
            TransferFunction::Pq
        } else if stream_output.contains("arib-std-b67") {
            TransferFunction::Hlg
        } else {
            TransferFunction::SdrBt1886
        };
    result.detected.transfer = detected_tf;

    // Detect color primaries
    let detected_color = if stream_output.contains("bt2020") {
        Colorimetry::Bt2020
    } else if stream_output.contains("smpte432") || stream_output.contains("p3") {
        Colorimetry::P3D65
    } else {
        Colorimetry::Bt709
    };
    result.detected.colorimetry = detected_color;

    // Detect bit depth
    let bits_re = regex_lite::Regex::new(r#""bits_per_raw_sample"\s*:\s*"(\d+)""#).unwrap();
    if let Some(cap) = bits_re.captures(&stream_output) {
        result.detected.bit_depth = cap[1].parse().unwrap_or(0);
    }

    // Detect MaxCLL/MaxFALL
    let cll_re = regex_lite::Regex::new(r"max_content\s*:\s*(\d+)").unwrap();
    let fall_re = regex_lite::Regex::new(r"max_average\s*:\s*(\d+)").unwrap();
    if let Some(cap) = cll_re.captures(&frame_output) {
        let max_fall = fall_re
            .captures(&frame_output)
            .and_then(|fc| fc[1].parse().ok())
            .unwrap_or(0);
        let cll = ContentLightLevel {
            max_cll: cap[1].parse().unwrap_or(0),
            max_fall,
        };
        result.detected.content_light = Some(cll);
    }

    // Detect mastering display
    let master_re = regex_lite::Regex::new(r"min_luminance=(\d+).*?max_luminance=(\d+)").unwrap();
    if let Some(cap) = master_re.captures(&frame_output) {
        result.detected.mastering_display = Some(MasteringDisplay {
            min_luminance: cap[1].parse().unwrap_or(0),
            max_luminance: cap[2].parse().unwrap_or(0),
        });
    }

    // Validate
    result.valid = true;

    if detected_tf != opts.expected_transfer {
        result.issues.push(HdrIssue {
            field: "Transfer Function".into(),
            expected: transfer_str(opts.expected_transfer).into(),
            actual: transfer_str(detected_tf).into(),
            severity: "error".into(),
            description: "Transfer function mismatch".into(),
        });
        result.valid = false;
    }

    if detected_color != opts.expected_colorimetry {
        result.issues.push(HdrIssue {
            field: "Color Primaries".into(),
            expected: colorimetry_str(opts.expected_colorimetry).into(),
            actual: colorimetry_str(detected_color).into(),
            severity: "error".into(),
            description: "Color primaries mismatch".into(),
        });
        result.valid = false;
    }

    if result.detected.bit_depth > 0 && result.detected.bit_depth < opts.expected_bit_depth {
        result.issues.push(HdrIssue {
            field: "Bit Depth".into(),
            expected: opts.expected_bit_depth.to_string(),
            actual: result.detected.bit_depth.to_string(),
            severity: "error".into(),
            description: "Bit depth below requirement".into(),
        });
        result.valid = false;
    }

    if opts.expected_max_cll > 0
        && let Some(ref cll) = result.detected.content_light
        && cll.max_cll > opts.expected_max_cll
    {
        result.issues.push(HdrIssue {
            field: "MaxCLL".into(),
            expected: format!("≤ {} nits", opts.expected_max_cll),
            actual: format!("{} nits", cll.max_cll),
            severity: "warning".into(),
            description: "MaxCLL exceeds expected limit".into(),
        });
    }

    if opts.expected_max_fall > 0
        && let Some(ref cll) = result.detected.content_light
        && cll.max_fall > opts.expected_max_fall
    {
        result.issues.push(HdrIssue {
            field: "MaxFALL".into(),
            expected: format!("≤ {} nits", opts.expected_max_fall),
            actual: format!("{} nits", cll.max_fall),
            severity: "warning".into(),
            description: "MaxFALL exceeds expected limit".into(),
        });
    }

    if opts.expected_max_luminance > 0
        && let Some(ref md) = result.detected.mastering_display
        && md.max_luminance != opts.expected_max_luminance
    {
        result.issues.push(HdrIssue {
            field: "Mastering Display Max Luminance".into(),
            expected: format!("{} nits", opts.expected_max_luminance),
            actual: format!("{} nits", md.max_luminance),
            severity: "warning".into(),
            description: "Mastering display luminance does not match expected value".into(),
        });
    }

    result.success = true;
    result
}

/// Cross-validate CPL HDR declarations against actual MXF video metadata.
pub fn validate_cpl_hdr(cpl_path: &Path, video_path: &Path) -> HdrValidateResult {
    let mut result = HdrValidateResult::default();

    if !cpl_path.exists() {
        result.error = format!("CPL file not found: {}", cpl_path.display());
        return result;
    }
    if !video_path.exists() {
        result.error = format!("Video MXF not found: {}", video_path.display());
        return result;
    }

    let cpl_content = match std::fs::read_to_string(cpl_path) {
        Ok(c) => c,
        Err(e) => {
            result.error = format!("Cannot read CPL: {e}");
            return result;
        }
    };

    // Detect CPL-declared transfer function
    let cpl_transfer = if cpl_content.contains("SMPTE-ST-2084") || cpl_content.contains("ST2084") {
        TransferFunction::Pq
    } else if cpl_content.contains("ARIB-STD-B67") || cpl_content.contains("HLG") {
        TransferFunction::Hlg
    } else {
        TransferFunction::SdrBt1886
    };

    // Detect CPL-declared colorimetry
    let cpl_color = if cpl_content.contains("ITU-R-BT.2020") || cpl_content.contains("BT.2020") {
        Colorimetry::Bt2020
    } else if cpl_content.contains("P3-D65") || cpl_content.contains("SMPTE-RP-431") {
        Colorimetry::P3D65
    } else if cpl_content.contains("P3-DCI") {
        Colorimetry::P3Dci
    } else {
        Colorimetry::Bt709
    };

    // Validate the actual video against CPL declarations
    let opts = HdrValidateOptions {
        video_path: video_path.to_path_buf(),
        expected_transfer: cpl_transfer,
        expected_colorimetry: cpl_color,
        expected_bit_depth: 0,
        expected_max_cll: 0,
        expected_max_fall: 0,
        expected_max_luminance: 0,
    };
    let video_result = validate_hdr_metadata(&opts);
    if !video_result.success {
        result.error = video_result.error;
        return result;
    }

    result.detected = video_result.detected;
    result.issues = video_result.issues;
    result.valid = video_result.valid;
    result.success = video_result.success;

    // Cross-check: CPL says SDR but video has HDR
    if cpl_transfer == TransferFunction::SdrBt1886
        && result.detected.transfer != TransferFunction::SdrBt1886
    {
        result.issues.push(HdrIssue {
            field: "CPL Transfer Function".into(),
            expected: "SDR (no HDR metadata in CPL)".into(),
            actual: format!(
                "{} (detected in MXF)",
                transfer_str(result.detected.transfer)
            ),
            severity: "warning".into(),
            description: "Video contains HDR metadata but CPL does not declare it".into(),
        });
    }

    // Cross-check: CPL says HDR but video is SDR
    if cpl_transfer != TransferFunction::SdrBt1886
        && result.detected.transfer == TransferFunction::SdrBt1886
    {
        result.issues.push(HdrIssue {
            field: "CPL Transfer Function".into(),
            expected: format!("{} (declared in CPL)", transfer_str(cpl_transfer)),
            actual: "SDR (no HDR in MXF)".into(),
            severity: "error".into(),
            description: "CPL declares HDR but video does not contain HDR metadata".into(),
        });
        result.valid = false;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_ffprobe_cannot_read_does_not_come_back_as_sdr() {
        let dir = tempfile::tempdir().unwrap();
        let not_video = dir.path().join("picture.mxf");
        std::fs::write(&not_video, b"this is not an MXF").unwrap();

        let result = validate_hdr_metadata(&HdrValidateOptions {
            video_path: not_video,
            expected_transfer: TransferFunction::Pq,
            expected_colorimetry: Colorimetry::Bt2020,
            expected_bit_depth: 0,
            expected_max_cll: 0,
            expected_max_fall: 0,
            expected_max_luminance: 0,
        });

        assert!(
            !result.success,
            "an unreadable file must not report a measurement"
        );
        assert!(!result.error.is_empty(), "the reason must be reported");
    }
}
