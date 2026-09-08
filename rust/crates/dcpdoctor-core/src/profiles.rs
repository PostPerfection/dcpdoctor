//! Theater compatibility profiles: what a package is, read from the essence and
//! the CPL, against the format limits of the servers it will play on.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Stored width above which ST 429-2 Table 1 calls a picture format 4K.
const TWO_K_MAX_WIDTH: u32 = 2048;

/// Above this rate a server needs the high frame rate option.
const HIGH_FRAME_RATE_THRESHOLD: u32 = 30;

/// A theater server profile with supported format constraints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TheaterProfile {
    pub name: String,
    pub vendor: String,
    pub max_resolution: (u32, u32),
    pub max_frame_rate: u32,
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

/// The picture and sound parameters a profile is checked against, read from the
/// package itself. Each value is `Err` with the reason it could not be read, so
/// a limit nothing was compared against is reported rather than passed.
#[derive(Debug, Clone)]
pub struct PackageFormat {
    pub resolution: Result<(u32, u32), String>,
    pub frame_rate: Result<u32, String>,
    pub channel_count: Result<u32, String>,
    pub has_atmos: Result<bool, String>,
    pub is_stereo3d: bool,
}

/// What a profile check found: limits the package breaks, and limits nothing was
/// compared against.
#[derive(Debug, Clone, Default)]
pub struct CompatibilityReport {
    pub issues: Vec<String>,
    pub not_checked: Vec<String>,
}

impl CompatibilityReport {
    pub fn is_compatible(&self) -> bool {
        self.issues.is_empty() && self.not_checked.is_empty()
    }
}

/// Read the picture size, frame rate, channel count, Atmos presence and 3D flag
/// of a DCP. The picture size and channel count come from the essence
/// descriptors through asdcplib, the rate and the 3D flag from the CPL.
pub fn read_package_format(dcp_dir: &Path) -> Result<PackageFormat, String> {
    let dcp = crate::dcp::open_dcp(dcp_dir).map_err(|notes| {
        notes
            .first()
            .map(|n| n.message.clone())
            .unwrap_or_else(|| format!("{} would not open as a DCP", dcp_dir.display()))
    })?;
    let Some((cpl_path, cpl)) = dcp.cpls.first() else {
        return Err(format!("no CPL in {}", dcp_dir.display()));
    };

    let id_to_file: HashMap<String, PathBuf> = dcp
        .assetmap
        .assets
        .iter()
        .map(|a| {
            let id =
                a.id.strip_prefix("urn:uuid:")
                    .unwrap_or(&a.id)
                    .to_lowercase();
            (id, dcp_dir.join(&a.path))
        })
        .collect();

    let picture = cpl.reels.first().map(|reel| &reel.picture);
    Ok(PackageFormat {
        resolution: picture
            .ok_or_else(|| "the CPL carries no reel".to_string())
            .and_then(|picture| picture_resolution(picture, &id_to_file)),
        frame_rate: picture
            .ok_or_else(|| "the CPL carries no reel".to_string())
            .and_then(|picture| picture_frame_rate(&picture.edit_rate)),
        channel_count: crate::validators::first_sound_channel_count_of_cpl(cpl_path, &id_to_file)
            .ok_or_else(|| "no sound essence the CPL references would open as PCM".to_string()),
        has_atmos: package_has_atmos(&id_to_file),
        is_stereo3d: cpl.reels.iter().any(|reel| reel.stereoscopic),
    })
}

/// Stored size of the reel's picture essence. The CPL schema has no element for
/// the picture size, so the essence is the only source.
fn picture_resolution(
    picture: &crate::cpl::ReelAsset,
    id_to_file: &HashMap<String, PathBuf>,
) -> Result<(u32, u32), String> {
    let id = picture.id.strip_prefix("urn:uuid:").unwrap_or(&picture.id);
    let path = id_to_file
        .get(&id.to_lowercase())
        .ok_or_else(|| format!("no ASSETMAP asset matches the picture id {id}"))?;
    let path_str = path
        .to_str()
        .ok_or_else(|| format!("non-UTF-8 path {}", path.display()))?;
    let mut reader =
        crate::j2k::PictureEssenceReader::open(path_str, crate::j2k::PictureEssenceFamily::Cinema)
            .ok_or_else(|| {
                format!(
                    "{} would not open as picture essence",
                    path.file_name().unwrap_or_default().to_string_lossy()
                )
            })?;
    let descriptor = reader
        .picture_descriptor()
        .map_err(|e| format!("the picture descriptor would not read: {e}"))?;
    Ok((descriptor.stored_width, descriptor.stored_height))
}

/// The composition's frame rate, rounded from the picture EditRate the way a
/// server reads it: 24000/1001 runs at 24 fps.
fn picture_frame_rate(edit_rate: &str) -> Result<u32, String> {
    let mut parts = edit_rate.split_whitespace();
    let numerator: f64 = parts
        .next()
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| format!("the picture EditRate '{edit_rate}' is no rate"))?;
    let denominator: f64 = match parts.next() {
        Some(d) => d
            .parse()
            .map_err(|_| format!("the picture EditRate '{edit_rate}' is no rate"))?,
        None => 1.0,
    };
    if denominator <= 0.0 || numerator <= 0.0 {
        return Err(format!("the picture EditRate '{edit_rate}' is no rate"));
    }
    Ok((numerator / denominator).round() as u32)
}

/// Whether the package carries a Dolby Atmos track, from the essence type
/// asdcplib reads out of each MXF. A file whose type will not read leaves the
/// answer unknown rather than reading as no Atmos.
fn package_has_atmos(id_to_file: &HashMap<String, PathBuf>) -> Result<bool, String> {
    let mut unreadable = Vec::new();
    for path in id_to_file.values() {
        if path.extension().is_none_or(|ext| ext != "mxf") {
            continue;
        }
        let Some(path_str) = path.to_str() else {
            continue;
        };
        match asdcplib::essence_type(path_str) {
            Ok(asdcplib::EssenceType::DcDataDolbyAtmos) => return Ok(true),
            Ok(_) => {}
            Err(_) => unreadable.push(
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            ),
        }
    }
    if unreadable.is_empty() {
        return Ok(false);
    }
    unreadable.sort();
    Err(format!(
        "the essence type of {} would not read",
        unreadable.join(", ")
    ))
}

/// Check a package's format against a theater profile.
pub fn check_compatibility(
    profile: &TheaterProfile,
    format: &PackageFormat,
) -> CompatibilityReport {
    let mut report = CompatibilityReport::default();

    match &format.resolution {
        Ok((width, height)) => {
            if *width > profile.max_resolution.0 || *height > profile.max_resolution.1 {
                report.issues.push(format!(
                    "Resolution {}x{} exceeds {} maximum ({}x{})",
                    width, height, profile.name, profile.max_resolution.0, profile.max_resolution.1
                ));
            }
            if !profile.supports_4k && *width > TWO_K_MAX_WIDTH {
                report
                    .issues
                    .push(format!("{} does not support 4K", profile.name));
            }
        }
        Err(reason) => report.not_checked.push(format!(
            "the {}x{} maximum: {reason}",
            profile.max_resolution.0, profile.max_resolution.1
        )),
    }

    match &format.frame_rate {
        Ok(rate) => {
            if *rate > profile.max_frame_rate {
                report.issues.push(format!(
                    "Frame rate {} fps exceeds {} maximum ({} fps)",
                    rate, profile.name, profile.max_frame_rate
                ));
            }
            if !profile.supports_hfr && *rate > HIGH_FRAME_RATE_THRESHOLD {
                report.issues.push(format!(
                    "{} does not support HFR ({rate} fps requested)",
                    profile.name
                ));
            }
        }
        Err(reason) => report.not_checked.push(format!(
            "the {} fps maximum: {reason}",
            profile.max_frame_rate
        )),
    }

    match &format.channel_count {
        Ok(channels) => {
            if *channels > profile.max_channels {
                report.issues.push(format!(
                    "Channel count {} exceeds {} maximum ({})",
                    channels, profile.name, profile.max_channels
                ));
            }
        }
        Err(reason) => report.not_checked.push(format!(
            "the {} channel maximum: {reason}",
            profile.max_channels
        )),
    }

    match &format.has_atmos {
        Ok(true) if !profile.supports_atmos => report
            .issues
            .push(format!("{} does not support Dolby Atmos/IAB", profile.name)),
        Ok(_) => {}
        Err(reason) if !profile.supports_atmos => report
            .not_checked
            .push(format!("Dolby Atmos/IAB support: {reason}")),
        Err(_) => {}
    }

    if format.is_stereo3d && !profile.supports_stereo3d {
        report
            .issues
            .push(format!("{} does not support stereoscopic 3D", profile.name));
    }

    report
}
