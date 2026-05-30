//! Frame-level QC analysis for J2K image sequences and quality metrics.

use std::path::PathBuf;

use serde::Serialize;

/// Per-frame QC entry.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FrameQcEntry {
    pub frame_number: u32,
    pub size_bytes: u64,
    pub bitrate_mbps: f64,
    pub over_budget: bool,
    pub under_budget: bool,
}

/// Options for frame QC analysis.
pub struct FrameQcOptions {
    pub j2k_dir: PathBuf,
    pub fps_num: u32,
    pub fps_den: u32,
    pub max_bitrate_mbps: f64,
    pub min_bitrate_mbps: f64,
}

/// Result of frame QC analysis.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FrameQcResult {
    pub success: bool,
    pub error: String,
    pub total_frames: u32,
    pub total_bytes: u64,
    pub average_bitrate_mbps: f64,
    pub peak_bitrate_mbps: f64,
    pub min_bitrate_mbps: f64,
    pub over_budget_count: u32,
    pub under_budget_count: u32,
    pub frames: Vec<FrameQcEntry>,
}

/// Analyze a directory of J2K frames for bitrate budget compliance.
pub fn analyze_frame_qc(opts: &FrameQcOptions) -> FrameQcResult {
    let mut result = FrameQcResult::default();

    if !opts.j2k_dir.exists() {
        result.error = "J2K directory not found".into();
        return result;
    }

    let fps = opts.fps_num as f64 / opts.fps_den as f64;

    let mut frames: Vec<PathBuf> = std::fs::read_dir(&opts.j2k_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.is_file()
                && p.extension()
                    .is_some_and(|ext| ext == "j2c" || ext == "j2k")
            {
                Some(p)
            } else {
                None
            }
        })
        .collect();

    frames.sort();

    for (i, frame_path) in frames.iter().enumerate() {
        let size = std::fs::metadata(frame_path).map(|m| m.len()).unwrap_or(0);
        let bitrate = (size as f64 * 8.0 * fps) / 1e6;

        let over_budget = bitrate > opts.max_bitrate_mbps;
        let under_budget = bitrate < opts.min_bitrate_mbps;

        if over_budget {
            result.over_budget_count += 1;
        }
        if under_budget {
            result.under_budget_count += 1;
        }

        result.total_bytes += size;
        result.frames.push(FrameQcEntry {
            frame_number: i as u32,
            size_bytes: size,
            bitrate_mbps: bitrate,
            over_budget,
            under_budget,
        });
    }

    result.total_frames = frames.len() as u32;
    if result.total_frames > 0 {
        result.average_bitrate_mbps =
            (result.total_bytes as f64 * 8.0 * fps) / (result.total_frames as f64 * 1e6);
        result.peak_bitrate_mbps = result
            .frames
            .iter()
            .map(|f| f.bitrate_mbps)
            .fold(0.0_f64, f64::max);
        result.min_bitrate_mbps = result
            .frames
            .iter()
            .map(|f| f.bitrate_mbps)
            .fold(f64::INFINITY, f64::min);
    }

    result.success = true;
    result
}
