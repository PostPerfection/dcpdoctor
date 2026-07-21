//! EBU R128 loudness measurement and normalization.
//!
//! Measurement delegates to postkit::loudness; normalization stays here since
//! postkit has no loudnorm-write helper.

use std::path::{Path, PathBuf};
use std::process::Command;

use postkit::loudness::LoudnessResult;
use serde::Serialize;

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

/// Measure integrated loudness of an audio file (delegates to postkit).
pub fn measure_loudness(audio_file: &Path) -> LoudnessResult {
    if !audio_file.exists() {
        return LoudnessResult {
            error: "Audio file not found".into(),
            ..Default::default()
        };
    }
    postkit::loudness::measure_loudness(audio_file)
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
