/// J2K codestream deep analysis.
///
/// Parses JPEG 2000 codestream headers to extract:
/// - Profile (RSIZ / capabilities)
/// - Decomposition levels
/// - Code-block size
/// - Wavelet transform type
/// - Component count and bit depth
/// - Per-frame bitrate
use crate::{Code, Note};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// J2K codestream parameters extracted from SIZ and COD markers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct J2kCodestreamInfo {
    /// RSIZ capabilities field (profile indicator)
    pub rsiz: u16,
    /// Profile name
    pub profile: String,
    /// Image width
    pub width: u32,
    /// Image height
    pub height: u32,
    /// Number of components
    pub components: u16,
    /// Bits per component (from first component)
    pub bit_depth: u8,
    /// Number of decomposition levels
    pub decomposition_levels: u8,
    /// Code-block width exponent (actual size = 2^(exp+2))
    pub codeblock_width_exp: u8,
    /// Code-block height exponent (actual size = 2^(exp+2))
    pub codeblock_height_exp: u8,
    /// Wavelet transform: true = irreversible (9-7), false = reversible (5-3)
    pub irreversible_transform: bool,
    /// Number of quality layers
    pub layers: u16,
    /// Progression order
    pub progression_order: String,
    /// Frame size in bytes
    pub frame_bytes: u64,
}

/// Analyze a JPEG 2000 codestream file or extract from MXF.
///
/// For direct .j2c files, reads the codestream header.
/// For MXF files, uses ffprobe to extract J2K parameters.
pub fn analyze_j2k(path: &Path) -> Result<J2kCodestreamInfo, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "j2c" | "j2k" | "jp2" => parse_j2k_codestream(path),
        "mxf" => analyze_j2k_from_mxf(path),
        _ => Err(format!("Unsupported file type: .{ext}")),
    }
}

/// Parse J2K codestream directly from a .j2c/.j2k file.
///
/// The SIZ-derived fields, profile (RSIZ), decomposition levels, layers and
/// progression order come from postkit's header parser. Code-block size and the
/// wavelet transform type aren't in postkit's J2kHeader, so we read those extra
/// COD fields locally.
fn parse_j2k_codestream(path: &Path) -> Result<J2kCodestreamInfo, String> {
    let data = std::fs::read(path).map_err(|e| format!("Failed to read file: {e}"))?;

    if data.len() < 4 {
        return Err("File too small to be a J2K codestream".into());
    }

    let hdr = postkit::j2k::parse_j2k_header(&data)
        .ok_or("Missing SOC marker (not a valid J2K codestream)")?;

    let mut info = J2kCodestreamInfo {
        rsiz: hdr.profile,
        profile: rsiz_to_profile(hdr.profile),
        width: hdr.width,
        height: hdr.height,
        components: hdr.num_components,
        bit_depth: hdr.bit_depth,
        decomposition_levels: hdr.num_decomp_levels,
        layers: hdr.num_layers,
        progression_order: progression_order_name(hdr.progression_order),
        frame_bytes: data.len() as u64,
        ..Default::default()
    };

    parse_cod_extras(&data, &mut info);

    Ok(info)
}

/// Read the COD fields postkit's header parser doesn't expose: code-block size
/// exponents and the wavelet transform type. Marker walk mirrors postkit's.
fn parse_cod_extras(data: &[u8], info: &mut J2kCodestreamInfo) {
    const SOT: u16 = 0xFF90;
    const SOD: u16 = 0xFF93;
    const COD: u16 = 0xFF52;
    const EOC: u16 = 0xFFD9;

    let mut pos = 2; // skip SOC
    while pos + 2 < data.len() {
        let marker = u16::from_be_bytes([data[pos], data[pos + 1]]);
        pos += 2;

        if marker == SOD || marker == EOC || marker == SOT {
            break;
        }
        if pos + 2 > data.len() {
            break;
        }
        let seg_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;
        if seg_len < 2 || pos + seg_len - 2 > data.len() {
            break;
        }
        let seg = &data[pos..pos + seg_len - 2];

        if marker == COD {
            if seg.len() > 6 {
                info.codeblock_width_exp = seg[6];
            }
            if seg.len() > 7 {
                info.codeblock_height_exp = seg[7];
            }
            if seg.len() > 9 {
                // 0 = 9-7 irreversible, 1 = 5-3 reversible
                info.irreversible_transform = seg[9] == 0;
            }
        }

        pos += seg_len - 2;
    }
}

fn progression_order_name(po: u8) -> String {
    match po {
        0 => "LRCP".into(),
        1 => "RLCP".into(),
        2 => "RPCL".into(),
        3 => "PCRL".into(),
        4 => "CPRL".into(),
        n => format!("Unknown({n})"),
    }
}

fn rsiz_to_profile(rsiz: u16) -> String {
    match rsiz {
        0 => "No profile (unrestricted)".into(),
        1 => "Profile 0 (DCI 2K)".into(),
        2 => "Profile 1 (DCI 4K)".into(),
        3 => "Cinema 2K".into(),
        4 => "Cinema 4K".into(),
        0x0100..=0x01FF => format!("Broadcast Profile (RSIZ=0x{rsiz:04X})"),
        _ => format!("Unknown (RSIZ=0x{rsiz:04X})"),
    }
}

/// Analyze J2K parameters from an MXF file via ffprobe.
fn analyze_j2k_from_mxf(path: &Path) -> Result<J2kCodestreamInfo, String> {
    let output = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_streams",
            "-show_format",
        ])
        .arg(path)
        .output()
        .map_err(|e| format!("Failed to run ffprobe: {e}"))?;

    if !output.status.success() {
        return Err("ffprobe failed to read MXF file".into());
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Failed to parse ffprobe output: {e}"))?;

    let stream = json["streams"]
        .as_array()
        .and_then(|s| {
            s.iter()
                .find(|s| s["codec_name"].as_str() == Some("jpeg2000"))
        })
        .or_else(|| {
            json["streams"]
                .as_array()
                .and_then(|s| s.iter().find(|s| s["codec_type"].as_str() == Some("video")))
        });

    let stream = match stream {
        Some(s) => s,
        None => return Err("No video/J2K stream found in MXF".into()),
    };

    let width = stream["width"].as_u64().unwrap_or(0) as u32;
    let height = stream["height"].as_u64().unwrap_or(0) as u32;
    let file_size = json["format"]["size"]
        .as_str()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let frame_count = stream["nb_frames"]
        .as_str()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(1);

    let frame_bytes = file_size.checked_div(frame_count).unwrap_or(0);

    let bit_depth = stream["bits_per_raw_sample"]
        .as_str()
        .and_then(|s| s.parse::<u8>().ok())
        .unwrap_or(12);

    Ok(J2kCodestreamInfo {
        width,
        height,
        components: 3,
        bit_depth,
        frame_bytes,
        profile: if width > 2048 {
            "Cinema 4K (from MXF)".into()
        } else {
            "Cinema 2K (from MXF)".into()
        },
        irreversible_transform: true, // DCI requires 9-7
        ..Default::default()
    })
}

/// Validate J2K codestream against DCI requirements.
pub fn validate_j2k_dci(info: &J2kCodestreamInfo) -> Vec<Note> {
    let mut notes = Vec::new();

    // DCI requires 9-7 irreversible wavelet
    if !info.irreversible_transform && info.decomposition_levels > 0 {
        notes.push(Note::error(
            Code::J2kInvalidProfile,
            "DCI requires irreversible (9-7) wavelet transform; found reversible (5-3)",
        ));
    }

    // DCI requires 5 or 6 decomposition levels
    if info.decomposition_levels > 0 && !(5..=6).contains(&info.decomposition_levels) {
        notes.push(Note::warning(
            Code::J2kInvalidProfile,
            format!(
                "DCI typically requires 5-6 decomposition levels; found {}",
                info.decomposition_levels
            ),
        ));
    }

    // DCI requires code-block size 32x32 or 64x64 (exp = 3 or 4 → size = 2^(exp+2))
    if info.codeblock_width_exp > 0 {
        let cb_w = 1u32 << (info.codeblock_width_exp + 2);
        let cb_h = 1u32 << (info.codeblock_height_exp + 2);
        if !matches!((cb_w, cb_h), (32, 32) | (64, 64)) {
            notes.push(Note::warning(
                Code::J2kInvalidProfile,
                format!("Non-standard code-block size: {cb_w}x{cb_h} (DCI uses 32x32)"),
            ));
        }
    }

    // Component count: DCI requires 3 (XYZ)
    if info.components > 0 && info.components != 3 {
        notes.push(Note::error(
            Code::J2kInvalidComponentCount,
            format!("DCI requires 3 components (XYZ); found {}", info.components),
        ));
    }

    // Bit depth: DCI is 12 bits per component
    if info.bit_depth > 0 && info.bit_depth != 12 {
        notes.push(Note::warning(
            Code::J2kInvalidProfile,
            format!(
                "DCI standard uses 12 bits per component; found {}",
                info.bit_depth
            ),
        ));
    }

    // Bitrate check
    if info.frame_bytes > 0 && info.width > 0 {
        let max_bitrate_mbps = postkit::j2k::dci_max_bitrate_mbps(info.width);
        let max_bitrate_bps = (max_bitrate_mbps * 1_000_000.0) as u64;
        // At 24fps, one frame max = max_bitrate / 24 / 8 bytes
        let max_frame_bytes_24 = max_bitrate_bps / 24 / 8;
        if info.frame_bytes > max_frame_bytes_24 {
            let actual_bitrate_mbps = (info.frame_bytes * 24 * 8) as f64 / 1_000_000.0;
            notes.push(Note::error(
                Code::J2kBitrateExceeded,
                format!(
                    "Frame size {} bytes implies {:.1} Mbps at 24fps (max {} Mbps)",
                    info.frame_bytes, actual_bitrate_mbps, max_bitrate_mbps as u64
                ),
            ));
        }
    }

    // RSIZ profile validation
    if info.rsiz > 0 {
        match info.rsiz {
            1 | 3 => {
                // Profile 0 / Cinema 2K
                if info.width > 2048 || info.height > 1080 {
                    notes.push(Note::error(
                        Code::J2kInvalidProfile,
                        format!(
                            "Profile claims Cinema 2K but resolution is {}x{}",
                            info.width, info.height
                        ),
                    ));
                }
            }
            2 | 4 if info.width > 4096 || info.height > 2160 => {
                // Profile 1 / Cinema 4K
                notes.push(Note::error(
                    Code::J2kInvalidProfile,
                    format!(
                        "Resolution {}x{} exceeds Cinema 4K maximum (4096x2160)",
                        info.width, info.height
                    ),
                ));
            }
            _ => {}
        }
    }

    notes
}

/// Analyze per-frame bitrate for a directory of J2K codestream files.
pub fn analyze_frame_bitrates(dir: &Path, fps: f64) -> Result<BitrateStats, String> {
    let mut frames: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("Failed to read directory: {e}"))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| matches!(e, "j2c" | "j2k" | "jp2"))
        })
        .collect();

    if frames.is_empty() {
        return Err("No J2K frame files found".into());
    }

    frames.sort();

    let mut sizes: Vec<u64> = Vec::with_capacity(frames.len());
    for f in &frames {
        let size = std::fs::metadata(f).map(|m| m.len()).unwrap_or(0);
        sizes.push(size);
    }

    let total: u64 = sizes.iter().sum();
    let count = sizes.len() as f64;
    let avg = total as f64 / count;
    let min = *sizes.iter().min().unwrap_or(&0);
    let max = *sizes.iter().max().unwrap_or(&0);

    let avg_bitrate = avg * fps * 8.0 / 1_000_000.0;
    let max_bitrate = max as f64 * fps * 8.0 / 1_000_000.0;
    let min_bitrate = min as f64 * fps * 8.0 / 1_000_000.0;

    Ok(BitrateStats {
        frame_count: sizes.len() as u64,
        avg_frame_bytes: avg as u64,
        min_frame_bytes: min,
        max_frame_bytes: max,
        avg_bitrate_mbps: avg_bitrate,
        min_bitrate_mbps: min_bitrate,
        max_bitrate_mbps: max_bitrate,
        fps,
    })
}

use std::path::PathBuf;

/// Per-frame bitrate statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BitrateStats {
    pub frame_count: u64,
    pub avg_frame_bytes: u64,
    pub min_frame_bytes: u64,
    pub max_frame_bytes: u64,
    pub avg_bitrate_mbps: f64,
    pub min_bitrate_mbps: f64,
    pub max_bitrate_mbps: f64,
    pub fps: f64,
}
