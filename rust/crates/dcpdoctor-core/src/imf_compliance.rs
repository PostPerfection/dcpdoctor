//! IMF platform compliance checking (Netflix, Disney+, Amazon, Apple TV+, etc.).

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

/// Target platform for compliance checking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ImfComplianceTarget {
    Netflix,
    Disney,
    Amazon,
    Apple,
    Cinema2K,
    Cinema4K,
    BroadcastHd,
    BroadcastUhd,
}

/// A single compliance check result.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ImfComplianceCheck {
    pub rule: String,
    pub description: String,
    pub expected_value: String,
    pub actual_value: String,
    pub passed: bool,
}

/// Options for IMF compliance check.
pub struct ImfComplianceOptions {
    pub imp_dir: PathBuf,
    pub target: ImfComplianceTarget,
}

/// Result of IMF compliance check.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ImfComplianceResult {
    pub success: bool,
    pub error: String,
    pub target: Option<ImfComplianceTarget>,
    pub compliant: bool,
    pub passed: u32,
    pub failed: u32,
    pub checks: Vec<ImfComplianceCheck>,
}

/// Return the display name for a compliance target.
pub fn target_name(target: ImfComplianceTarget) -> &'static str {
    match target {
        ImfComplianceTarget::Netflix => "Netflix",
        ImfComplianceTarget::Disney => "Disney+",
        ImfComplianceTarget::Amazon => "Amazon",
        ImfComplianceTarget::Apple => "Apple TV+",
        ImfComplianceTarget::Cinema2K => "Cinema 2K",
        ImfComplianceTarget::Cinema4K => "Cinema 4K",
        ImfComplianceTarget::BroadcastHd => "Broadcast HD",
        ImfComplianceTarget::BroadcastUhd => "Broadcast UHD",
    }
}

/// Run IMF compliance checks against a target platform spec.
pub fn check_imf_compliance(opts: &ImfComplianceOptions) -> ImfComplianceResult {
    let mut result = ImfComplianceResult {
        target: Some(opts.target),
        ..Default::default()
    };

    if !opts.imp_dir.exists() {
        result.error = format!("IMP directory not found: {}", opts.imp_dir.display());
        return result;
    }

    // Find a video track file
    let video_file = find_track_file(&opts.imp_dir, "mxf");

    match opts.target {
        ImfComplianceTarget::Netflix => {
            check_resolution(&mut result, video_file.as_deref(), 3840, 2160);
            check_framerate(
                &mut result,
                video_file.as_deref(),
                &[23.976, 24.0, 25.0, 29.97, 30.0, 50.0, 59.94, 60.0],
            );
        }
        ImfComplianceTarget::Disney => {
            check_resolution(&mut result, video_file.as_deref(), 3840, 2160);
            check_framerate(&mut result, video_file.as_deref(), &[23.976, 24.0, 25.0]);
        }
        ImfComplianceTarget::Amazon => {
            check_resolution(&mut result, video_file.as_deref(), 3840, 2160);
            check_framerate(
                &mut result,
                video_file.as_deref(),
                &[23.976, 24.0, 25.0, 29.97],
            );
        }
        ImfComplianceTarget::Apple => {
            check_resolution(&mut result, video_file.as_deref(), 3840, 2160);
            check_framerate(
                &mut result,
                video_file.as_deref(),
                &[23.976, 24.0, 25.0, 29.97],
            );
        }
        ImfComplianceTarget::Cinema2K => {
            check_resolution(&mut result, video_file.as_deref(), 2048, 1080);
            check_framerate(&mut result, video_file.as_deref(), &[24.0, 48.0]);
        }
        ImfComplianceTarget::Cinema4K => {
            check_resolution(&mut result, video_file.as_deref(), 4096, 2160);
            check_framerate(&mut result, video_file.as_deref(), &[24.0, 48.0]);
        }
        ImfComplianceTarget::BroadcastHd => {
            check_resolution(&mut result, video_file.as_deref(), 1920, 1080);
            check_framerate(
                &mut result,
                video_file.as_deref(),
                &[25.0, 29.97, 50.0, 59.94],
            );
        }
        ImfComplianceTarget::BroadcastUhd => {
            check_resolution(&mut result, video_file.as_deref(), 3840, 2160);
            check_framerate(&mut result, video_file.as_deref(), &[50.0, 59.94]);
        }
    }

    for c in &result.checks {
        if c.passed {
            result.passed += 1;
        } else {
            result.failed += 1;
        }
    }

    result.compliant = result.failed == 0;
    result.success = true;
    result
}

fn find_track_file(dir: &Path, ext: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().find_map(|e| {
        let p = e.path();
        if p.is_file() && p.extension().is_some_and(|x| x == ext) {
            Some(p)
        } else {
            None
        }
    })
}

fn check_resolution(
    result: &mut ImfComplianceResult,
    video: Option<&Path>,
    max_w: u32,
    max_h: u32,
) {
    let mut check = ImfComplianceCheck {
        rule: "resolution".into(),
        description: "Video resolution within platform limits".into(),
        expected_value: format!("{max_w}x{max_h}"),
        ..Default::default()
    };

    if let Some(video_path) = video {
        let output = Command::new("ffprobe")
            .args([
                "-v",
                "quiet",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=width,height",
                "-of",
                "csv=p=0",
                &video_path.to_string_lossy(),
            ])
            .output();

        if let Ok(o) = output {
            let s = String::from_utf8_lossy(&o.stdout);
            let parts: Vec<&str> = s.trim().split(',').collect();
            if parts.len() == 2 {
                let w: u32 = parts[0].parse().unwrap_or(0);
                let h: u32 = parts[1].parse().unwrap_or(0);
                check.actual_value = format!("{w}x{h}");
                check.passed = w <= max_w && h <= max_h;
            }
        }
    }

    result.checks.push(check);
}

fn check_framerate(result: &mut ImfComplianceResult, video: Option<&Path>, allowed: &[f64]) {
    let mut check = ImfComplianceCheck {
        rule: "framerate".into(),
        description: "Frame rate is in allowed set".into(),
        expected_value: allowed
            .iter()
            .map(|f| format!("{f}"))
            .collect::<Vec<_>>()
            .join(", "),
        ..Default::default()
    };

    if let Some(video_path) = video {
        let output = Command::new("ffprobe")
            .args([
                "-v",
                "quiet",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=r_frame_rate",
                "-of",
                "csv=p=0",
                &video_path.to_string_lossy(),
            ])
            .output();

        if let Ok(o) = output {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let parts: Vec<&str> = s.split('/').collect();
            if parts.len() == 2 {
                let num: f64 = parts[0].parse().unwrap_or(0.0);
                let den: f64 = parts[1].parse().unwrap_or(1.0);
                if den > 0.0 {
                    let fps = num / den;
                    check.actual_value = format!("{fps:.3} fps");
                    check.passed = allowed.iter().any(|a| (fps - a).abs() < 0.01);
                }
            }
        }
    }

    result.checks.push(check);
}
