//! Threshold-scored frame comparison, a thin wrapper over postkit::frame_compare.
//!
//! postkit runs the ffmpeg PSNR/SSIM/VMAF core; this layer adds dcpdoctor's
//! per-frame threshold scoring (a frame is "significant" when its PSNR falls
//! below the threshold).

use std::path::Path;

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
    pub threshold_psnr: f64,
    pub compute_ssim: bool,
    pub compute_vmaf: bool,
}

impl Default for CompareOptions {
    fn default() -> Self {
        Self {
            threshold_psnr: 40.0,
            compute_ssim: true,
            compute_vmaf: false,
        }
    }
}

/// PSNR postkit reports for a frame whose components carry no error at all:
/// ffmpeg prints `inf` and postkit's stats parser substitutes this. No encode
/// that changes a sample reaches it.
const LOSSLESS_PSNR: f64 = 100.0;

/// How the two files compared.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Every frame decoded to the same samples.
    Identical,
    /// The pictures differ, but no frame fell below the PSNR threshold.
    WithinThreshold,
    /// At least one frame fell below the PSNR threshold.
    #[default]
    Different,
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Verdict::Identical => "IDENTICAL",
            Verdict::WithinThreshold => "WITHIN THRESHOLD",
            Verdict::Different => "DIFFERENT",
        })
    }
}

/// Result of a file comparison.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CompareResult {
    pub success: bool,
    pub error: String,
    pub verdict: Verdict,
    pub frames_compared: u32,
    pub frames_different: u32,
    pub avg_psnr: f64,
    pub min_psnr: f64,
    pub avg_ssim: f64,
    pub min_ssim: f64,
    pub vmaf_score: f64,
    pub diffs: Vec<FrameDiff>,
}

/// Compare two video files frame-by-frame, scoring each frame against the PSNR
/// threshold. SSIM is always computed by the underlying core; VMAF is optional.
pub fn compare_files(file_a: &Path, file_b: &Path, opts: &CompareOptions) -> CompareResult {
    let mut result = CompareResult::default();

    if !file_a.exists() || !file_b.exists() {
        result.error = "One or both files do not exist".into();
        return result;
    }

    let cmp = match postkit::frame_compare::compare_frames(file_a, file_b) {
        Ok(c) => c,
        Err(e) => {
            result.error = e;
            return result;
        }
    };

    result.frames_compared = cmp.frames_compared as u32;
    result.avg_psnr = cmp.avg_psnr;
    result.min_psnr = cmp.min_psnr;
    result.avg_ssim = cmp.avg_ssim;
    result.min_ssim = cmp.min_ssim;

    for m in &cmp.per_frame {
        let significant = m.psnr_avg < opts.threshold_psnr;
        if significant {
            result.frames_different += 1;
            result.diffs.push(FrameDiff {
                frame_number: m.frame as u32,
                psnr: m.psnr_avg,
                ssim: m.ssim_avg,
                significant,
            });
        }
    }

    if opts.compute_vmaf {
        match postkit::frame_compare::compute_vmaf(file_a, file_b) {
            Ok(v) => result.vmaf_score = v.mean,
            Err(e) => {
                result.error = e;
                return result;
            }
        }
    }

    result.verdict = verdict(&cmp.per_frame, result.frames_different);
    result.success = true;
    result
}

/// A comparison is only identical when every frame came back lossless. Reading
/// "no frame below the threshold" as identical called a 62 dB re-encode of the
/// same content an exact match.
fn verdict(per_frame: &[postkit::frame_compare::FrameMetric], frames_different: u32) -> Verdict {
    let lossless = !per_frame.is_empty()
        && per_frame
            .iter()
            .all(|m| m.psnr_avg >= LOSSLESS_PSNR && m.ssim_avg >= 1.0);
    match (lossless, frames_different) {
        (true, _) => Verdict::Identical,
        (false, 0) => Verdict::WithinThreshold,
        (false, _) => Verdict::Different,
    }
}
