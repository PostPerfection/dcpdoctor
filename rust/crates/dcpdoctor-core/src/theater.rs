//! Theater/server profile compatibility checking.

use std::path::Path;

use serde::Serialize;

use crate::{Code, Note, Severity, Standard};

/// Theater/server hardware profile.
#[derive(Debug, Clone, Serialize)]
pub struct TheaterProfile {
    pub name: String,
    pub vendor: String,
    pub requires_bv21: bool,
    pub supports_interop: bool,
    pub supports_hfr: bool,
    pub supports_4k: bool,
    pub supports_atmos: bool,
    pub max_channels: u32,
    pub max_bitrate_mbps: u32,
    pub known_issues: Vec<String>,
}

/// Get known theater profiles.
pub fn get_theater_profiles() -> Vec<TheaterProfile> {
    vec![
        TheaterProfile {
            name: "Dolby IMS3000".into(),
            vendor: "Dolby".into(),
            requires_bv21: true,
            supports_interop: true,
            supports_hfr: true,
            supports_4k: true,
            supports_atmos: true,
            max_channels: 64,
            max_bitrate_mbps: 500,
            known_issues: vec![
                "Requires BV2.1 for Atmos content".into(),
                "May reject DCPs with non-standard MCA labels".into(),
                "Requires ASSETMAP.xml naming".into(),
            ],
        },
        TheaterProfile {
            name: "Dolby DSS/IMS2000".into(),
            vendor: "Dolby".into(),
            requires_bv21: false,
            supports_interop: true,
            supports_hfr: false,
            supports_4k: false,
            supports_atmos: true,
            max_channels: 16,
            max_bitrate_mbps: 250,
            known_issues: vec![
                "Limited Atmos support (bed channels only on older firmware)".into(),
                "May have issues with 48fps content".into(),
            ],
        },
        TheaterProfile {
            name: "Barco SP4K".into(),
            vendor: "Barco".into(),
            requires_bv21: true,
            supports_interop: true,
            supports_hfr: true,
            supports_4k: true,
            supports_atmos: false,
            max_channels: 16,
            max_bitrate_mbps: 500,
            known_issues: vec![
                "Requires SMPTE standard for 4K".into(),
                "No native Dolby Atmos (requires external processor)".into(),
                "Some firmware versions have subtitle timing issues".into(),
            ],
        },
        TheaterProfile {
            name: "Barco SP2K".into(),
            vendor: "Barco".into(),
            requires_bv21: false,
            supports_interop: true,
            supports_hfr: false,
            supports_4k: false,
            supports_atmos: false,
            max_channels: 8,
            max_bitrate_mbps: 250,
            known_issues: vec![
                "Limited to 2K projection".into(),
                "May not support all SMPTE subtitle features".into(),
            ],
        },
        TheaterProfile {
            name: "Christie CP4440-RGB".into(),
            vendor: "Christie".into(),
            requires_bv21: true,
            supports_interop: true,
            supports_hfr: true,
            supports_4k: true,
            supports_atmos: false,
            max_channels: 16,
            max_bitrate_mbps: 500,
            known_issues: vec![
                "No native Atmos support".into(),
                "Requires external IMB for content decryption".into(),
                "4K RGB laser - ensure correct color profile".into(),
            ],
        },
        TheaterProfile {
            name: "Christie CP2230".into(),
            vendor: "Christie".into(),
            requires_bv21: false,
            supports_interop: true,
            supports_hfr: false,
            supports_4k: false,
            supports_atmos: false,
            max_channels: 8,
            max_bitrate_mbps: 250,
            known_issues: vec![
                "Legacy 2K DLP projection".into(),
                "Limited subtitle font support".into(),
                "No HFR capability".into(),
            ],
        },
        TheaterProfile {
            name: "GDC SX-4000".into(),
            vendor: "GDC".into(),
            requires_bv21: true,
            supports_interop: true,
            supports_hfr: true,
            supports_4k: true,
            supports_atmos: true,
            max_channels: 64,
            max_bitrate_mbps: 500,
            known_issues: vec![
                "Full Atmos support with integrated processor".into(),
                "Strict BV2.1 enforcement on latest firmware".into(),
                "Known issue: rejects PKLs without .xml extension".into(),
            ],
        },
        TheaterProfile {
            name: "GDC SR-1000".into(),
            vendor: "GDC".into(),
            requires_bv21: false,
            supports_interop: true,
            supports_hfr: false,
            supports_4k: false,
            supports_atmos: false,
            max_channels: 8,
            max_bitrate_mbps: 250,
            known_issues: vec![
                "Legacy server - limited feature support".into(),
                "May have issues with large CPLs (>50 reels)".into(),
                "No subtitle font embedding support".into(),
            ],
        },
        TheaterProfile {
            name: "Dolby Cinema (Premium)".into(),
            vendor: "Dolby".into(),
            requires_bv21: true,
            supports_interop: false,
            supports_hfr: true,
            supports_4k: true,
            supports_atmos: true,
            max_channels: 128,
            max_bitrate_mbps: 500,
            known_issues: vec![
                "SMPTE-only (rejects Interop)".into(),
                "Requires BV2.1 compliance".into(),
                "Dolby Vision HDR metadata required for premium experience".into(),
                "Strict audio channel labeling (MCA) required".into(),
            ],
        },
        TheaterProfile {
            name: "IMAX Digital".into(),
            vendor: "IMAX".into(),
            requires_bv21: true,
            supports_interop: false,
            supports_hfr: true,
            supports_4k: true,
            supports_atmos: false,
            max_channels: 12,
            max_bitrate_mbps: 500,
            known_issues: vec![
                "SMPTE-only".into(),
                "Requires IMAX-specific audio channel layout".into(),
                "12-channel audio configuration mandatory".into(),
                "Higher frame rate requirements for IMAX Enhanced".into(),
            ],
        },
    ]
}

/// Find a theater profile by name or vendor (case-insensitive search).
pub fn find_profile(query: &str) -> Option<TheaterProfile> {
    let profiles = get_theater_profiles();
    let lower_query = query.to_lowercase();

    profiles.into_iter().find(|p| {
        p.name.to_lowercase().contains(&lower_query) || p.vendor.to_lowercase() == lower_query
    })
}

/// Check DCP compatibility with a specific theater profile.
pub fn check_theater_compatibility(
    dcp_dir: &Path,
    standard: Standard,
    profile: &TheaterProfile,
) -> Vec<Note> {
    let mut notes = Vec::new();
    let path_buf = Some(dcp_dir.to_path_buf());

    if !profile.supports_interop && standard == Standard::Interop {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::SmpteNamespaceWrong,
            message: format!("{} does not support Interop standard", profile.name),
            file: path_buf.clone(),
            line: 0,
        });
    }

    if profile.requires_bv21
        && standard == Standard::Smpte
        && !dcp_dir.join("ASSETMAP.xml").exists()
    {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::SmpteNamingViolation,
            message: format!("{} requires BV2.1 (ASSETMAP.xml naming)", profile.name),
            file: path_buf.clone(),
            line: 0,
        });
    }

    for issue in &profile.known_issues {
        notes.push(Note {
            severity: Severity::Info,
            code: Code::SmpteNamespaceWrong,
            message: format!("{} note: {issue}", profile.name),
            file: path_buf.clone(),
            line: 0,
        });
    }

    notes
}
