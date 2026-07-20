//! Frame-by-frame quality comparison using PSNR, SSIM, and VMAF via ffmpeg.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

/// A single frame difference measurement.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FrameDiff {
    pub frame_number: u32,
    pub psnr: f64,
    pub ssim: f64,
    pub significant: bool,
}

/// Options for file comparison.
pub struct CompareOptions {
    pub start_frame: u32,
    pub end_frame: u32,
    pub threshold_psnr: f64,
    pub compute_ssim: bool,
    pub compute_vmaf: bool,
}

impl Default for CompareOptions {
    fn default() -> Self {
        Self {
            start_frame: 0,
            end_frame: 0,
            threshold_psnr: 40.0,
            compute_ssim: true,
            compute_vmaf: false,
        }
    }
}

/// Result of a file comparison.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CompareResult {
    pub success: bool,
    pub error: String,
    pub identical: bool,
    pub frames_compared: u32,
    pub frames_different: u32,
    pub avg_psnr: f64,
    pub min_psnr: f64,
    pub avg_ssim: f64,
    pub min_ssim: f64,
    pub vmaf_score: f64,
    pub diffs: Vec<FrameDiff>,
}

/// Quality metrics from reference-based comparison.
#[derive(Debug, Clone, Default, Serialize)]
pub struct QualityMetrics {
    pub success: bool,
    pub error: String,
    pub vmaf_score: f64,
    pub psnr_avg: f64,
    pub ssim: f64,
}

/// Options for quality metric computation.
pub struct QualityOptions {
    pub reference: PathBuf,
    pub distorted: PathBuf,
    pub compute_vmaf: bool,
    pub compute_psnr: bool,
    pub compute_ssim: bool,
}

fn run_cmd(cmd: &str, args: &[&str]) -> String {
    Command::new(cmd)
        .args(args)
        .output()
        .map(|o| {
            let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
            s.push_str(&String::from_utf8_lossy(&o.stderr));
            s
        })
        .unwrap_or_default()
}

/// Compare two video files frame-by-frame using PSNR (and optionally SSIM/VMAF).
pub fn compare_files(file_a: &Path, file_b: &Path, opts: &CompareOptions) -> CompareResult {
    let mut result = CompareResult::default();

    if !file_a.exists() || !file_b.exists() {
        result.error = "One or both files do not exist".into();
        return result;
    }

    let a_str = file_a.to_string_lossy();
    let b_str = file_b.to_string_lossy();

    let stats_file = std::env::temp_dir().join("dcpdoctor_psnr.txt");

    // Build PSNR command
    let mut psnr_args = Vec::new();
    let seek_str = opts.start_frame.to_string();
    let frames_str = (opts.end_frame - opts.start_frame).to_string();

    if opts.start_frame > 0 {
        psnr_args.extend(["-ss", seek_str.as_str()]);
    }
    psnr_args.extend(["-i", &a_str]);
    if opts.start_frame > 0 {
        psnr_args.extend(["-ss", seek_str.as_str()]);
    }
    psnr_args.extend(["-i", &b_str]);

    if opts.end_frame > opts.start_frame {
        psnr_args.extend(["-frames:v", frames_str.as_str()]);
    }

    let stats_path_str = stats_file.to_string_lossy().to_string();
    let filter = format!("psnr=stats_file={stats_path_str}");
    psnr_args.extend(["-lavfi", &filter, "-f", "null", "-"]);

    let output = run_cmd("ffmpeg", &psnr_args.to_vec());

    // Parse average PSNR from output
    if let Some(pos) = output.find("average:") {
        let rest = &output[pos + 8..];
        if let Some(end) = rest.find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-') {
            result.avg_psnr = rest[..end].parse().unwrap_or(0.0);
        }
    }

    // Parse per-frame stats
    if let Ok(stats_content) = std::fs::read_to_string(&stats_file) {
        let psnr_re = regex_lite::Regex::new(r"n:(\d+)\s+.*?psnr_avg:([\d.inf]+)").unwrap();
        for cap in psnr_re.captures_iter(&stats_content) {
            let frame_num: u32 = cap[1].parse().unwrap_or(0);
            let psnr_str = &cap[2];
            let psnr = if psnr_str == "inf" {
                100.0
            } else {
                psnr_str.parse().unwrap_or(100.0)
            };

            let significant = psnr < opts.threshold_psnr;
            let diff = FrameDiff {
                frame_number: frame_num,
                psnr,
                ssim: 0.0,
                significant,
            };

            result.frames_compared += 1;
            if significant {
                result.frames_different += 1;
                result.diffs.push(diff);
            }
            if psnr < result.min_psnr || result.min_psnr == 0.0 {
                result.min_psnr = psnr;
            }
        }
    }
    let _ = std::fs::remove_file(&stats_file);

    // SSIM
    if opts.compute_ssim {
        let ssim_file = std::env::temp_dir().join("dcpdoctor_ssim.txt");
        let ssim_path_str = ssim_file.to_string_lossy().to_string();
        let ssim_filter = format!("ssim=stats_file={ssim_path_str}");

        let mut ssim_args = Vec::new();
        if opts.start_frame > 0 {
            ssim_args.extend(["-ss", seek_str.as_str()]);
        }
        ssim_args.extend(["-i", &a_str]);
        if opts.start_frame > 0 {
            ssim_args.extend(["-ss", seek_str.as_str()]);
        }
        ssim_args.extend(["-i", &b_str]);
        if opts.end_frame > opts.start_frame {
            ssim_args.extend(["-frames:v", frames_str.as_str()]);
        }
        ssim_args.extend(["-lavfi", &ssim_filter, "-f", "null", "-"]);

        let ssim_out = run_cmd("ffmpeg", &ssim_args.to_vec());

        let ssim_avg_re = regex_lite::Regex::new(r"All:([\d.]+)").unwrap();
        if let Some(cap) = ssim_avg_re.captures(&ssim_out) {
            result.avg_ssim = cap[1].parse().unwrap_or(0.0);
        }

        result.min_ssim = 1.0;
        if let Ok(ssim_content) = std::fs::read_to_string(&ssim_file) {
            let ssim_line_re = regex_lite::Regex::new(r"n:(\d+)\s+.*?All:([\d.]+)").unwrap();
            for cap in ssim_line_re.captures_iter(&ssim_content) {
                let val: f64 = cap[2].parse().unwrap_or(1.0);
                if val < result.min_ssim {
                    result.min_ssim = val;
                }
            }
        }
        let _ = std::fs::remove_file(&ssim_file);
    }

    // VMAF
    if opts.compute_vmaf {
        let mut vmaf_args = Vec::new();
        if opts.start_frame > 0 {
            vmaf_args.extend(["-ss", seek_str.as_str()]);
        }
        vmaf_args.extend(["-i", &a_str]);
        if opts.start_frame > 0 {
            vmaf_args.extend(["-ss", seek_str.as_str()]);
        }
        vmaf_args.extend(["-i", &b_str]);
        if opts.end_frame > opts.start_frame {
            vmaf_args.extend(["-frames:v", frames_str.as_str()]);
        }
        vmaf_args.extend(["-lavfi", "libvmaf", "-f", "null", "-"]);

        let vmaf_out = run_cmd("ffmpeg", &vmaf_args.to_vec());
        let vmaf_re = regex_lite::Regex::new(r"VMAF score:\s*([\d.]+)").unwrap();
        match vmaf_re.captures(&vmaf_out) {
            Some(cap) => result.vmaf_score = cap[1].parse().unwrap_or(0.0),
            None => {
                // no score means ffmpeg lacks libvmaf or the run failed; don't pass silently
                result.error =
                    "VMAF requested but ffmpeg produced no score (libvmaf missing or ffmpeg run failed)"
                        .into();
                return result;
            }
        }
    }

    // ffmpeg produced no per-frame PSNR: inputs unreadable or the run failed.
    // Never report "0 frames, IDENTICAL".
    if result.frames_compared == 0 {
        let tail = output.trim().lines().last().unwrap_or("no output");
        result.error = format!("no frames compared (ffmpeg produced no PSNR data): {tail}");
        return result;
    }

    result.identical = result.frames_different == 0;
    result.success = true;
    result
}

/// Compute quality metrics (VMAF, PSNR, SSIM) between reference and distorted.
pub fn compute_quality(opts: &QualityOptions) -> QualityMetrics {
    let mut result = QualityMetrics::default();

    let ref_str = opts.reference.to_string_lossy().to_string();
    let dist_str = opts.distorted.to_string_lossy().to_string();

    let mut parts = Vec::new();
    if opts.compute_vmaf {
        parts.push("libvmaf");
    }
    if opts.compute_psnr {
        parts.push("psnr");
    }
    if opts.compute_ssim {
        parts.push("ssim");
    }

    if parts.is_empty() {
        result.error = "No metrics selected".into();
        return result;
    }

    let filter = format!("[0:v][1:v]{}", parts.join(";[0:v][1:v]"));
    let output = run_cmd(
        "ffmpeg",
        &[
            "-i", &dist_str, "-i", &ref_str, "-lavfi", &filter, "-f", "null", "-",
        ],
    );

    if output.is_empty() {
        result.error = "ffmpeg quality analysis failed".into();
        return result;
    }

    let vmaf_re = regex_lite::Regex::new(r"VMAF score:\s*([\d.]+)").unwrap();
    let psnr_re = regex_lite::Regex::new(r"average:([\d.]+)").unwrap();
    let ssim_re = regex_lite::Regex::new(r"All:([\d.]+)").unwrap();

    if let Some(cap) = vmaf_re.captures(&output) {
        result.vmaf_score = cap[1].parse().unwrap_or(0.0);
    }
    if let Some(cap) = psnr_re.captures(&output) {
        result.psnr_avg = cap[1].parse().unwrap_or(0.0);
    }
    if let Some(cap) = ssim_re.captures(&output) {
        result.ssim = cap[1].parse().unwrap_or(0.0);
    }

    result.success = true;
    result
}
