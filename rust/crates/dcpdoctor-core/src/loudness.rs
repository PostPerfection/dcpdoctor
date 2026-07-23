//! EBU R128 loudness measurement and normalization, plus Leq(m) (ISO 21727).
//!
//! Measurement (R128) and Leq(m) both delegate to postkit::loudness;
//! normalization stays here since postkit has no loudnorm-write helper.

use std::path::{Path, PathBuf};
use std::process::Command;

use postkit::loudness::LoudnessResult;
use serde::Serialize;

// Leq(m) (ISO 21727, CCIR 468-weighted) now lives in postkit::loudness.
pub use postkit::loudness::{LeqMResult, leq_m_from_samples, measure_leq_m};

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    // Integration assertion over the re-exported postkit Leq(m): a full-scale
    // 1 kHz sine is -3.01 dBFS RMS, weighting is 0 dB at 1 kHz, +105 dB B-chain
    // offset -> 101.99 dB. Guards that the CLI's leq_m_db stays on the right
    // reference across the postkit switch.
    #[test]
    fn full_scale_1khz_sine_matches_derived_leq_m() {
        let sr = 48000u32;
        let n = sr as usize; // 1 second
        let samples: Vec<f32> = (0..n)
            .map(|i| (2.0 * PI * 1000.0 * i as f32 / sr as f32).sin())
            .collect();
        let leq = leq_m_from_samples(&samples, sr);
        assert!(
            (leq - 101.99).abs() < 0.3,
            "Leq(m) was {leq}, expected ~101.99 dB"
        );
    }
}
