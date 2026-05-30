//! EBU R128 loudness measurement and normalization via ffmpeg.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

/// Result of a loudness measurement.
#[derive(Debug, Clone, Default, Serialize)]
pub struct LoudnessResult {
    pub success: bool,
    pub error: String,
    pub integrated_lufs: f64,
    pub loudness_range_lu: f64,
    pub true_peak_dbtp: f64,
    pub compliant_r128: bool,
    pub compliant_atsc: bool,
}

/// Options for loudness normalization.
pub struct NormalizeOptions {
    pub input_file: PathBuf,
    pub output_file: PathBuf,
    pub target_lufs: f64,
    pub true_peak_limit: f64,
}

/// Result of loudness normalization.
#[derive(Debug, Clone, Default, Serialize)]
pub struct NormalizeResult {
    pub success: bool,
    pub error: String,
    pub output_file: PathBuf,
    pub measured: LoudnessResult,
}

/// Measure integrated loudness of an audio file using ffmpeg ebur128.
pub fn measure_loudness(audio_file: &Path) -> LoudnessResult {
    let mut result = LoudnessResult::default();

    if !audio_file.exists() {
        result.error = "Audio file not found".into();
        return result;
    }

    let output = Command::new("ffmpeg")
        .args([
            "-i",
            &audio_file.to_string_lossy(),
            "-af",
            "ebur128=peak=true",
            "-f",
            "null",
            "-",
        ])
        .output();

    let stderr = match output {
        Ok(o) => String::from_utf8_lossy(&o.stderr).into_owned(),
        Err(e) => {
            result.error = format!("ffmpeg failed: {e}");
            return result;
        }
    };

    let integrated_re = regex_lite::Regex::new(r"I:\s*([-\d.]+)\s*LUFS").unwrap();
    let range_re = regex_lite::Regex::new(r"LRA:\s*([-\d.]+)\s*LU").unwrap();
    let peak_re = regex_lite::Regex::new(r"Peak:\s*([-\d.]+)\s*dBFS").unwrap();

    if let Some(cap) = integrated_re.captures(&stderr) {
        result.integrated_lufs = cap[1].parse().unwrap_or(0.0);
    }
    if let Some(cap) = range_re.captures(&stderr) {
        result.loudness_range_lu = cap[1].parse().unwrap_or(0.0);
    }
    if let Some(cap) = peak_re.captures(&stderr) {
        result.true_peak_dbtp = cap[1].parse().unwrap_or(0.0);
    }

    result.compliant_r128 = result.integrated_lufs >= -24.0 && result.integrated_lufs <= -22.0;
    result.compliant_atsc = result.integrated_lufs >= -26.0 && result.integrated_lufs <= -22.0;

    result.success = true;
    result
}

/// Normalize audio loudness to target LUFS using ffmpeg loudnorm.
pub fn normalize_loudness(opts: &NormalizeOptions) -> NormalizeResult {
    let mut result = NormalizeResult::default();

    let filter = format!(
        "loudnorm=I={}:TP={}:LRA=11",
        opts.target_lufs, opts.true_peak_limit
    );

    let out_str = opts.output_file.to_string_lossy();
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            &opts.input_file.to_string_lossy(),
            "-af",
            &filter,
            out_str.as_ref(),
        ])
        .status();

    match status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            result.error = format!(
                "ffmpeg loudnorm failed with code {}",
                s.code().unwrap_or(-1)
            );
            return result;
        }
        Err(e) => {
            result.error = format!("ffmpeg failed: {e}");
            return result;
        }
    }

    result.output_file = opts.output_file.clone();
    result.measured = measure_loudness(&opts.output_file);
    result.success = true;
    result
}
