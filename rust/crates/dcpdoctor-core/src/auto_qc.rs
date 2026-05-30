//! Automated QC — detect black frames, freeze frames, silence, and clipping.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

/// Type of QC issue detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QcIssueType {
    BlackFrame,
    FreezeFrame,
    AudioSilence,
    AudioClipping,
}

/// A single QC detection result.
#[derive(Debug, Clone, Serialize)]
pub struct QcIssue {
    pub issue_type: QcIssueType,
    pub start_sec: f64,
    pub end_sec: f64,
    pub duration_sec: f64,
    pub severity: &'static str,
    pub description: String,
}

/// Options for automatic QC analysis.
pub struct AutoQcOptions {
    pub video_path: PathBuf,
    pub audio_path: PathBuf,
    pub black_threshold: f64,
    pub black_duration_min: f64,
    pub freeze_threshold: f64,
    pub freeze_duration_min: f64,
    pub silence_threshold: f64,
    pub silence_duration_min: f64,
    pub clipping_threshold: f64,
}

impl Default for AutoQcOptions {
    fn default() -> Self {
        Self {
            video_path: PathBuf::new(),
            audio_path: PathBuf::new(),
            black_threshold: 0.98,
            black_duration_min: 2.0,
            freeze_threshold: -60.0,
            freeze_duration_min: 3.0,
            silence_threshold: -60.0,
            silence_duration_min: 3.0,
            clipping_threshold: -0.5,
        }
    }
}

/// Result of an automatic QC scan.
#[derive(Debug, Clone, Serialize)]
pub struct AutoQcResult {
    pub success: bool,
    pub error: String,
    pub issues: Vec<QcIssue>,
}

fn run_ffmpeg(args: &[&str]) -> String {
    let output = Command::new("ffmpeg").args(args).output();
    match output {
        Ok(o) => String::from_utf8_lossy(&o.stderr).into_owned(),
        Err(_) => String::new(),
    }
}

fn detect_black_frames(video: &Path, threshold: f64, min_duration: f64) -> Vec<QcIssue> {
    let filter = format!("blackdetect=d={min_duration}:pic_th={threshold}");
    let path_str = video.to_string_lossy();
    let output = run_ffmpeg(&["-i", &path_str, "-vf", &filter, "-an", "-f", "null", "-"]);

    let re = regex_lite::Regex::new(
        r"black_start:([\d.]+)\s+black_end:([\d.]+)\s+black_duration:([\d.]+)",
    )
    .unwrap();

    re.captures_iter(&output)
        .filter_map(|cap| {
            let start: f64 = cap[1].parse().ok()?;
            let end: f64 = cap[2].parse().ok()?;
            let dur: f64 = cap[3].parse().ok()?;
            Some(QcIssue {
                issue_type: QcIssueType::BlackFrame,
                start_sec: start,
                end_sec: end,
                duration_sec: dur,
                severity: if dur > 5.0 { "warning" } else { "info" },
                description: format!("Black frames: {dur:.1}s at {start:.1}s"),
            })
        })
        .collect()
}

fn detect_freeze_frames(video: &Path, threshold: f64, min_duration: f64) -> Vec<QcIssue> {
    let filter = format!("freezedetect=n={threshold}:d={min_duration}");
    let path_str = video.to_string_lossy();
    let output = run_ffmpeg(&["-i", &path_str, "-vf", &filter, "-an", "-f", "null", "-"]);

    let start_re = regex_lite::Regex::new(r"freeze_start:\s*([\d.]+)").unwrap();
    let end_re =
        regex_lite::Regex::new(r"freeze_end:\s*([\d.]+)\s*\|\s*freeze_duration:\s*([\d.]+)")
            .unwrap();

    let starts: Vec<f64> = start_re
        .captures_iter(&output)
        .filter_map(|cap| cap[1].parse().ok())
        .collect();

    end_re
        .captures_iter(&output)
        .enumerate()
        .filter_map(|(idx, cap)| {
            let end: f64 = cap[1].parse().ok()?;
            let dur: f64 = cap[2].parse().ok()?;
            let start = starts.get(idx).copied().unwrap_or(end - dur);
            Some(QcIssue {
                issue_type: QcIssueType::FreezeFrame,
                start_sec: start,
                end_sec: end,
                duration_sec: dur,
                severity: if dur > 10.0 { "warning" } else { "info" },
                description: format!("Freeze frame: {dur:.1}s at {start:.1}s"),
            })
        })
        .collect()
}

fn detect_silence(audio: &Path, threshold_db: f64, min_duration: f64) -> Vec<QcIssue> {
    let filter = format!("silencedetect=noise={threshold_db}dB:d={min_duration}");
    let path_str = audio.to_string_lossy();
    let output = run_ffmpeg(&["-i", &path_str, "-af", &filter, "-vn", "-f", "null", "-"]);

    let start_re = regex_lite::Regex::new(r"silence_start:\s*([\d.]+)").unwrap();
    let end_re =
        regex_lite::Regex::new(r"silence_end:\s*([\d.]+)\s*\|\s*silence_duration:\s*([\d.]+)")
            .unwrap();

    let starts: Vec<f64> = start_re
        .captures_iter(&output)
        .filter_map(|cap| cap[1].parse().ok())
        .collect();

    end_re
        .captures_iter(&output)
        .enumerate()
        .filter_map(|(idx, cap)| {
            let end: f64 = cap[1].parse().ok()?;
            let dur: f64 = cap[2].parse().ok()?;
            let start = starts.get(idx).copied().unwrap_or(end - dur);
            Some(QcIssue {
                issue_type: QcIssueType::AudioSilence,
                start_sec: start,
                end_sec: end,
                duration_sec: dur,
                severity: if dur > 5.0 { "warning" } else { "info" },
                description: format!("Audio silence: {dur:.1}s at {start:.1}s"),
            })
        })
        .collect()
}

fn detect_clipping(audio: &Path, threshold_dbfs: f64) -> Vec<QcIssue> {
    let path_str = audio.to_string_lossy();
    let output = run_ffmpeg(&[
        "-i",
        &path_str,
        "-af",
        "volumedetect",
        "-vn",
        "-f",
        "null",
        "-",
    ]);

    let re = regex_lite::Regex::new(r"max_volume:\s*([-\d.]+)\s*dB").unwrap();
    if let Some(cap) = re.captures(&output)
        && let Ok(max_vol) = cap[1].parse::<f64>()
        && max_vol > threshold_dbfs
    {
        return vec![QcIssue {
            issue_type: QcIssueType::AudioClipping,
            start_sec: 0.0,
            end_sec: 0.0,
            duration_sec: 0.0,
            severity: "error",
            description: format!(
                "Audio clipping detected: max volume {max_vol:.1} dBFS \
                 (threshold: {threshold_dbfs:.1} dBFS)"
            ),
        }];
    }
    Vec::new()
}

/// Run automatic QC analysis on video/audio files.
pub fn run_auto_qc(opts: &AutoQcOptions) -> AutoQcResult {
    let video_exists = opts.video_path.exists();
    let audio_exists = !opts.audio_path.as_os_str().is_empty() && opts.audio_path.exists();

    if !video_exists && !audio_exists {
        return AutoQcResult {
            success: false,
            error: "No valid input files specified".into(),
            issues: Vec::new(),
        };
    }

    let mut issues = Vec::new();

    if video_exists {
        issues.extend(detect_black_frames(
            &opts.video_path,
            opts.black_threshold,
            opts.black_duration_min,
        ));
        issues.extend(detect_freeze_frames(
            &opts.video_path,
            opts.freeze_threshold,
            opts.freeze_duration_min,
        ));

        if !audio_exists {
            issues.extend(detect_silence(
                &opts.video_path,
                opts.silence_threshold,
                opts.silence_duration_min,
            ));
            issues.extend(detect_clipping(&opts.video_path, opts.clipping_threshold));
        }
    }

    if audio_exists {
        issues.extend(detect_silence(
            &opts.audio_path,
            opts.silence_threshold,
            opts.silence_duration_min,
        ));
        issues.extend(detect_clipping(&opts.audio_path, opts.clipping_threshold));
    }

    AutoQcResult {
        success: true,
        error: String::new(),
        issues,
    }
}
