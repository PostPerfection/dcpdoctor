/// Theater compatibility profiles for DCP validation.
use serde::{Deserialize, Serialize};

/// A theater server profile with supported format constraints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TheaterProfile {
    pub name: String,
    pub vendor: String,
    pub max_resolution: (u32, u32),
    pub max_frame_rate: u32,
    pub max_bitrate_mbps: u32,
    pub supports_hfr: bool,
    pub supports_4k: bool,
    pub supports_atmos: bool,
    pub supports_stereo3d: bool,
    pub max_channels: u32,
    pub notes: String,
}

/// Get all built-in theater profiles.
pub fn all_profiles() -> Vec<TheaterProfile> {
    vec![
        TheaterProfile {
            name: "Dolby IMS3000".into(),
            vendor: "Dolby".into(),
            max_resolution: (4096, 2160),
            max_frame_rate: 120,
            max_bitrate_mbps: 500,
            supports_hfr: true,
            supports_4k: true,
            supports_atmos: true,
            supports_stereo3d: true,
            max_channels: 128,
            notes: "Dolby Cinema premium format; supports all current DCI features".into(),
        },
        TheaterProfile {
            name: "Dolby IMS2000".into(),
            vendor: "Dolby".into(),
            max_resolution: (4096, 2160),
            max_frame_rate: 60,
            max_bitrate_mbps: 500,
            supports_hfr: true,
            supports_4k: true,
            supports_atmos: true,
            supports_stereo3d: true,
            max_channels: 64,
            notes: "Standard Dolby server; wide deployment".into(),
        },
        TheaterProfile {
            name: "Dolby Cinema (Premium)".into(),
            vendor: "Dolby".into(),
            max_resolution: (4096, 2160),
            max_frame_rate: 120,
            max_bitrate_mbps: 500,
            supports_hfr: true,
            supports_4k: true,
            supports_atmos: true,
            supports_stereo3d: true,
            max_channels: 128,
            notes: "Full Dolby Cinema auditorium (Vision + Atmos)".into(),
        },
        TheaterProfile {
            name: "Barco SP4K".into(),
            vendor: "Barco".into(),
            max_resolution: (4096, 2160),
            max_frame_rate: 60,
            max_bitrate_mbps: 500,
            supports_hfr: true,
            supports_4k: true,
            supports_atmos: false,
            supports_stereo3d: true,
            max_channels: 16,
            notes: "Barco Series 4 projector with integrated media block".into(),
        },
        TheaterProfile {
            name: "Barco SP2K".into(),
            vendor: "Barco".into(),
            max_resolution: (2048, 1080),
            max_frame_rate: 48,
            max_bitrate_mbps: 250,
            supports_hfr: true,
            supports_4k: false,
            supports_atmos: false,
            supports_stereo3d: true,
            max_channels: 16,
            notes: "Barco Series 2 projector; 2K-only".into(),
        },
        TheaterProfile {
            name: "Christie CP4440-RGB".into(),
            vendor: "Christie".into(),
            max_resolution: (4096, 2160),
            max_frame_rate: 120,
            max_bitrate_mbps: 500,
            supports_hfr: true,
            supports_4k: true,
            supports_atmos: false,
            supports_stereo3d: true,
            max_channels: 16,
            notes: "Christie CineLife+ RGB laser; premium large format".into(),
        },
        TheaterProfile {
            name: "Christie CP2230".into(),
            vendor: "Christie".into(),
            max_resolution: (2048, 1080),
            max_frame_rate: 48,
            max_bitrate_mbps: 250,
            supports_hfr: true,
            supports_4k: false,
            supports_atmos: false,
            supports_stereo3d: true,
            max_channels: 16,
            notes: "Christie compact 2K projector; common in mid-size screens".into(),
        },
        TheaterProfile {
            name: "GDC SX-4000".into(),
            vendor: "GDC Technology".into(),
            max_resolution: (4096, 2160),
            max_frame_rate: 60,
            max_bitrate_mbps: 500,
            supports_hfr: true,
            supports_4k: true,
            supports_atmos: true,
            supports_stereo3d: true,
            max_channels: 64,
            notes: "GDC flagship server; supports Atmos via AES67".into(),
        },
        TheaterProfile {
            name: "GDC SR-1000".into(),
            vendor: "GDC Technology".into(),
            max_resolution: (2048, 1080),
            max_frame_rate: 30,
            max_bitrate_mbps: 250,
            supports_hfr: false,
            supports_4k: false,
            supports_atmos: false,
            supports_stereo3d: true,
            max_channels: 8,
            notes: "GDC entry-level server; 2K/24-30fps only".into(),
        },
        TheaterProfile {
            name: "IMAX Digital".into(),
            vendor: "IMAX".into(),
            max_resolution: (4096, 2160),
            max_frame_rate: 60,
            max_bitrate_mbps: 500,
            supports_hfr: true,
            supports_4k: true,
            supports_atmos: false,
            supports_stereo3d: true,
            max_channels: 12,
            notes: "IMAX digital projection; 12-channel immersive audio; specific aspect ratios"
                .into(),
        },
        TheaterProfile {
            name: "NEC NC3541L".into(),
            vendor: "NEC/Sharp".into(),
            max_resolution: (4096, 2160),
            max_frame_rate: 60,
            max_bitrate_mbps: 500,
            supports_hfr: true,
            supports_4k: true,
            supports_atmos: false,
            supports_stereo3d: true,
            max_channels: 16,
            notes: "NEC laser phosphor 4K projector".into(),
        },
    ]
}

/// Find a profile by name (case-insensitive partial match).
pub fn find_profile(name: &str) -> Option<TheaterProfile> {
    let lower = name.to_lowercase();
    all_profiles()
        .into_iter()
        .find(|p| p.name.to_lowercase().contains(&lower))
}

/// Check DCP compatibility against a theater profile.
/// Returns a list of compatibility warnings/errors.
pub fn check_compatibility(
    profile: &TheaterProfile,
    resolution: (u32, u32),
    frame_rate: u32,
    channel_count: u32,
    has_atmos: bool,
    is_stereo3d: bool,
) -> Vec<String> {
    let mut issues = Vec::new();

    if resolution.0 > profile.max_resolution.0 || resolution.1 > profile.max_resolution.1 {
        issues.push(format!(
            "Resolution {}x{} exceeds {} maximum ({}x{})",
            resolution.0,
            resolution.1,
            profile.name,
            profile.max_resolution.0,
            profile.max_resolution.1
        ));
    }

    if frame_rate > profile.max_frame_rate {
        issues.push(format!(
            "Frame rate {} fps exceeds {} maximum ({} fps)",
            frame_rate, profile.name, profile.max_frame_rate
        ));
    }

    if !profile.supports_hfr && frame_rate > 30 {
        issues.push(format!(
            "{} does not support HFR ({} fps requested)",
            profile.name, frame_rate
        ));
    }

    if !profile.supports_4k && resolution.0 > 2048 {
        issues.push(format!("{} does not support 4K", profile.name));
    }

    if has_atmos && !profile.supports_atmos {
        issues.push(format!("{} does not support Dolby Atmos/IAB", profile.name));
    }

    if is_stereo3d && !profile.supports_stereo3d {
        issues.push(format!("{} does not support stereoscopic 3D", profile.name));
    }

    if channel_count > profile.max_channels {
        issues.push(format!(
            "Channel count {} exceeds {} maximum ({})",
            channel_count, profile.name, profile.max_channels
        ));
    }

    issues
}
