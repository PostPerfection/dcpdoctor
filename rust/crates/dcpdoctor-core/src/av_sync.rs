//! Audio/video sync detection and repair via ffprobe/ffmpeg.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

/// Options for A/V sync detection.
pub struct AvSyncOptions {
    pub video_file: PathBuf,
    pub audio_file: PathBuf,
    pub fps_num: u32,
    pub fps_den: u32,
    pub sample_rate: u32,
}

/// Result of A/V sync detection.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AvSyncResult {
    pub success: bool,
    pub error: String,
    pub drift_ms: f64,
    pub drift_samples: i32,
    pub drift_frames: f64,
    pub in_sync: bool,
    pub recommendation: String,
}

/// Options for fixing A/V sync.
pub struct AvSyncFixOptions {
    pub audio_file: PathBuf,
    pub output_file: PathBuf,
    pub trim_samples: i32,
    pub sample_rate: u32,
    pub bit_depth: u32,
}

/// Result of A/V sync fix.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AvSyncFixResult {
    pub success: bool,
    pub error: String,
    pub output_file: PathBuf,
    pub samples_adjusted: i32,
}

fn probe_duration(file: &Path) -> Option<f64> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
            &file.to_string_lossy(),
        ])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&output.stdout);
    s.trim().parse().ok()
}

/// Detect A/V sync drift between video and audio files.
pub fn detect_av_sync(opts: &AvSyncOptions) -> AvSyncResult {
    let mut result = AvSyncResult::default();

    if !opts.video_file.exists() {
        result.error = format!("Video file not found: {}", opts.video_file.display());
        return result;
    }
    if !opts.audio_file.exists() {
        result.error = format!("Audio file not found: {}", opts.audio_file.display());
        return result;
    }

    let video_duration = match probe_duration(&opts.video_file) {
        Some(d) if d > 0.0 => d,
        _ => {
            result.error = "Failed to determine video duration".into();
            return result;
        }
    };

    let audio_duration = match probe_duration(&opts.audio_file) {
        Some(d) if d > 0.0 => d,
        _ => {
            result.error = "Failed to determine audio duration".into();
            return result;
        }
    };

    let drift_seconds = audio_duration - video_duration;
    result.drift_ms = drift_seconds * 1000.0;
    result.drift_samples = (drift_seconds * opts.sample_rate as f64) as i32;

    let frame_duration = opts.fps_den as f64 / opts.fps_num as f64;
    result.drift_frames = drift_seconds / frame_duration;

    // Within ±1 frame = in sync
    result.in_sync = result.drift_frames.abs() < 1.0;

    result.recommendation = if result.drift_samples > 0 {
        format!(
            "Audio is {} samples longer than video. Trim {} samples from audio tail.",
            result.drift_samples.unsigned_abs(),
            result.drift_samples.unsigned_abs()
        )
    } else if result.drift_samples < 0 {
        format!(
            "Audio is {} samples shorter than video. Pad {} samples of silence at audio tail.",
            result.drift_samples.unsigned_abs(),
            result.drift_samples.unsigned_abs()
        )
    } else {
        "Audio and video are perfectly in sync.".into()
    };

    result.success = true;
    result
}

/// Fix A/V sync by trimming or padding audio.
pub fn fix_av_sync(opts: &AvSyncFixOptions) -> AvSyncFixResult {
    let mut result = AvSyncFixResult::default();

    if !opts.audio_file.exists() {
        result.error = format!("Audio file not found: {}", opts.audio_file.display());
        return result;
    }

    if opts.trim_samples == 0 {
        // No adjustment needed — just copy
        if let Err(e) = std::fs::copy(&opts.audio_file, &opts.output_file) {
            result.error = format!("Copy failed: {e}");
            return result;
        }
        result.output_file = opts.output_file.clone();
        result.samples_adjusted = 0;
        result.success = true;
        return result;
    }

    let out_str = opts.output_file.to_string_lossy();
    let status = if opts.trim_samples > 0 {
        let trim_seconds = opts.trim_samples as f64 / opts.sample_rate as f64;
        Command::new("ffmpeg")
            .args([
                "-y",
                "-i",
                &opts.audio_file.to_string_lossy(),
                "-ss",
                &format!("{trim_seconds}"),
                "-c:a",
                &format!("pcm_s{}le", opts.bit_depth),
                "-ar",
                &opts.sample_rate.to_string(),
                out_str.as_ref(),
            ])
            .status()
    } else {
        let pad_seconds = (-opts.trim_samples) as f64 / opts.sample_rate as f64;
        Command::new("ffmpeg")
            .args([
                "-y",
                "-f",
                "lavfi",
                "-t",
                &format!("{pad_seconds}"),
                "-i",
                &format!("anullsrc=r={}", opts.sample_rate),
                "-i",
                &opts.audio_file.to_string_lossy(),
                "-filter_complex",
                "[0:a][1:a]concat=n=2:v=0:a=1",
                "-c:a",
                &format!("pcm_s{}le", opts.bit_depth),
                out_str.as_ref(),
            ])
            .status()
    };

    match status {
        Ok(s) if s.success() => {
            result.output_file = opts.output_file.clone();
            result.samples_adjusted = opts.trim_samples;
            result.success = true;
        }
        _ => {
            result.error = "ffmpeg A/V sync fix failed".into();
        }
    }

    result
}
