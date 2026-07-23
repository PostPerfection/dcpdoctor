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

    result.identical = result.frames_different == 0;
    result.success = true;
    result
}
