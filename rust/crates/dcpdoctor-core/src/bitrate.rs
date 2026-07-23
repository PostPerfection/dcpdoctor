//! Per-frame bitrate analysis of J2K MXF picture tracks.
//!
//! The per-frame reader lives in postkit (`j2k::analyse_mxf_bitrate`); the
//! Note-producing compliance check stays here.

use std::path::Path;

use crate::{Code, Note, Severity};

/// Frame-level bitrate statistics for a picture MXF (postkit's reader output).
pub type FrameBitrateStats = postkit::j2k::MxfBitrateStats;

/// Analyze per-frame bitrate of a J2K MXF using asdcplib.
pub fn analyze_picture_bitrate(mxf_path: &Path) -> FrameBitrateStats {
    postkit::j2k::analyse_mxf_bitrate(mxf_path)
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
