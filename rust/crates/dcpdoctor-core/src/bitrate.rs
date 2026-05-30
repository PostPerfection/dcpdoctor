//! Per-frame bitrate analysis of J2K MXF picture tracks.

use std::path::Path;

use serde::Serialize;

use crate::{Code, Note, Severity};

/// Frame-level bitrate statistics for a picture MXF.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FrameBitrateStats {
    pub valid: bool,
    pub error: String,
    pub frame_count: u32,
    pub width: u32,
    pub height: u32,
    pub frame_rate: f64,
    pub total_bytes: u64,
    pub min_frame_bytes: u64,
    pub max_frame_bytes: u64,
    pub max_frame_index: u32,
    pub avg_bitrate_mbps: f64,
    pub min_bitrate_mbps: f64,
    pub max_bitrate_mbps: f64,
}

/// Maximum buffer size for a single J2K frame (16 MB — covers 4K).
const MAX_FRAME_BUF: usize = 16 * 1024 * 1024;

/// Analyze per-frame bitrate of a J2K MXF using asdcplib.
pub fn analyze_picture_bitrate(mxf_path: &Path) -> FrameBitrateStats {
    let mut stats = FrameBitrateStats::default();

    let path_str = match mxf_path.to_str() {
        Some(s) => s,
        None => {
            stats.error = "Invalid UTF-8 in path".into();
            return stats;
        }
    };

    let mut reader = asdcplib::jp2k::MxfReader::new();
    if let Err(e) = reader.open_read(path_str) {
        stats.error = format!("Failed to open MXF: {e}");
        return stats;
    }

    let desc = match reader.picture_descriptor() {
        Ok(d) => d,
        Err(e) => {
            stats.error = format!("Failed to read picture descriptor: {e}");
            return stats;
        }
    };

    stats.frame_count = desc.container_duration;
    stats.width = desc.stored_width;
    stats.height = desc.stored_height;
    stats.frame_rate = desc.edit_rate.numerator as f64 / desc.edit_rate.denominator.max(1) as f64;

    if stats.frame_count == 0 || stats.frame_rate <= 0.0 {
        stats.error = "Invalid frame count or rate".into();
        return stats;
    }

    stats.min_frame_bytes = u64::MAX;
    let mut buf = vec![0u8; MAX_FRAME_BUF];

    for i in 0..stats.frame_count {
        let frame_size = match reader.read_frame(i, &mut buf, None, None) {
            Ok(sz) => sz as u64,
            Err(_) => break,
        };

        stats.total_bytes += frame_size;
        if frame_size > stats.max_frame_bytes {
            stats.max_frame_bytes = frame_size;
            stats.max_frame_index = i;
        }
        if frame_size < stats.min_frame_bytes {
            stats.min_frame_bytes = frame_size;
        }
    }

    if stats.min_frame_bytes == u64::MAX {
        stats.min_frame_bytes = 0;
    }

    let frame_duration_sec = 1.0 / stats.frame_rate;
    stats.avg_bitrate_mbps = (stats.total_bytes as f64 * 8.0)
        / (stats.frame_count as f64 * frame_duration_sec * 1_000_000.0);
    stats.max_bitrate_mbps =
        (stats.max_frame_bytes as f64 * 8.0) / (frame_duration_sec * 1_000_000.0);
    stats.min_bitrate_mbps =
        (stats.min_frame_bytes as f64 * 8.0) / (frame_duration_sec * 1_000_000.0);

    stats.valid = true;
    stats
}

/// Check bitrate stats against DCI limits.
pub fn check_bitrate_compliance(stats: &FrameBitrateStats, mxf_path: &Path) -> Vec<Note> {
    let mut notes = Vec::new();

    if !stats.valid {
        return notes;
    }

    // DCI limits: 250 Mbps for 2K, 500 Mbps for 4K
    let max_allowed: f64 = if stats.width > 2048 { 500.0 } else { 250.0 };

    if stats.max_bitrate_mbps > max_allowed {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::J2kBitrateExceeded,
            message: format!(
                "Peak frame bitrate {} Mbps exceeds DCI limit of {} Mbps (frame #{})",
                stats.max_bitrate_mbps as u32, max_allowed as u32, stats.max_frame_index
            ),
            file: Some(mxf_path.to_path_buf()),
            line: 0,
        });
    } else if stats.max_bitrate_mbps > max_allowed * 0.95 {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::J2kBitrateExceeded,
            message: format!(
                "Peak frame bitrate {} Mbps is near DCI limit of {} Mbps",
                stats.max_bitrate_mbps as u32, max_allowed as u32
            ),
            file: Some(mxf_path.to_path_buf()),
            line: 0,
        });
    }

    if stats.avg_bitrate_mbps > max_allowed {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::J2kBitrateExceeded,
            message: format!(
                "Average bitrate {} Mbps exceeds DCI limit",
                stats.avg_bitrate_mbps as u32
            ),
            file: Some(mxf_path.to_path_buf()),
            line: 0,
        });
    }

    notes
}
