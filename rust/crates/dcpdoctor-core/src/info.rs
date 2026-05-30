use serde::{Deserialize, Serialize};
use std::path::Path;

/// DCP information summary.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DcpInfo {
    pub standard: String,
    pub title: String,
    pub content_kind: String,
    pub asset_count: usize,
    pub cpl_count: usize,
    pub pkl_count: usize,
    pub reel_count: usize,
    pub total_duration_frames: i64,
    pub picture_width: u32,
    pub picture_height: u32,
    pub frame_rate: u32,
    pub audio_channels: u32,
    pub has_atmos: bool,
    pub is_stereo3d: bool,
}

/// Get summary information about a DCP.
pub fn get_dcp_info(dcp_dir: &Path) -> Option<DcpInfo> {
    let dcp = crate::dcp::open_dcp(dcp_dir).ok()?;

    let mut info = DcpInfo {
        standard: format!("{}", dcp.standard),
        asset_count: dcp.assetmap.assets.len(),
        cpl_count: dcp.cpls.len(),
        pkl_count: dcp.pkls.len(),
        ..Default::default()
    };

    if let Some((_, cpl)) = dcp.cpls.first() {
        info.title = cpl.content_title.clone();
        info.content_kind = cpl.content_kind.clone();
        info.reel_count = cpl.reels.len();
        info.total_duration_frames = cpl.reels.iter().map(|r| r.picture.duration).sum();

        // Extract picture/sound info from first reel
        if let Some(reel) = cpl.reels.first() {
            // Parse edit rate for frame rate
            if !reel.picture.edit_rate.is_empty() {
                let parts: Vec<&str> = reel.picture.edit_rate.split_whitespace().collect();
                if let Some(fps) = parts.first().and_then(|s| s.parse::<u32>().ok()) {
                    info.frame_rate = fps;
                }
            }
        }

        // Check for stereoscopic content
        info.is_stereo3d = cpl.reels.iter().any(|r| r.stereoscopic);
    }

    // Probe MXF files for resolution and audio channel info
    for asset in &dcp.assetmap.assets {
        let full_path = dcp_dir.join(&asset.path);
        let ext = full_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if ext != "mxf" || !full_path.exists() {
            continue;
        }

        let mxf_info = crate::mxf::read_mxf_info(&full_path);
        if !mxf_info.valid {
            continue;
        }

        if let Some(ref pic) = mxf_info.picture
            && pic.width > 0
            && info.picture_width == 0
        {
            info.picture_width = pic.width;
            info.picture_height = pic.height;
            if info.frame_rate == 0 && pic.frame_rate_den > 0 {
                info.frame_rate = pic.frame_rate_num / pic.frame_rate_den;
            }
        }
        if let Some(ref snd) = mxf_info.sound
            && snd.channels > info.audio_channels
        {
            info.audio_channels = snd.channels;
        }

        // Detect Atmos (auxiliary data / IAB)
        if mxf_info.essence_type.contains("data") || mxf_info.essence_type.contains("unknown") {
            // Check if it's an Atmos asset by examining file size patterns
            // Atmos MXFs tend to be large auxiliary data tracks
            if mxf_info.file_size_bytes > 1024 * 1024
                && mxf_info.picture.is_none()
                && mxf_info.sound.is_none()
            {
                info.has_atmos = true;
            }
        }
    }

    Some(info)
}
