use serde::{Deserialize, Serialize};
use std::path::Path;

/// Picture descriptor from MXF.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PictureDescriptor {
    pub width: u32,
    pub height: u32,
    pub frame_rate_num: u32,
    pub frame_rate_den: u32,
    pub bit_depth: u32,
    pub frame_count: u64,
}

/// Sound descriptor from MXF.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SoundDescriptor {
    pub sample_rate: u32,
    pub channels: u32,
    pub bit_depth: u32,
    pub duration: u64,
}

/// MXF file information.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MxfInfo {
    pub valid: bool,
    pub error: String,
    pub essence_type: String,
    pub picture: Option<PictureDescriptor>,
    pub sound: Option<SoundDescriptor>,
    pub file_size_bytes: u64,
}

/// Read MXF file metadata.
///
/// This is a basic implementation that reads KLV header metadata.
/// Full MXF parsing requires asdcplib FFI bindings.
pub fn read_mxf_info(path: &Path) -> MxfInfo {
    // Read the first bytes to check for MXF header
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            return MxfInfo {
                valid: false,
                error: format!("Failed to read file: {e}"),
                ..Default::default()
            };
        }
    };

    // MXF files start with a partition pack key (06 0e 2b 34)
    if data.len() < 16 || data[0..4] != [0x06, 0x0e, 0x2b, 0x34] {
        return MxfInfo {
            valid: false,
            error: "Not a valid MXF file (missing SMPTE UL header)".to_string(),
            ..Default::default()
        };
    }

    // MXF header magic validated — now extract metadata via ffprobe
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
        .output();

    let (picture, sound, essence_type) = match output {
        Ok(o) if o.status.success() => {
            let json: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap_or_default();

            let streams = json["streams"].as_array();

            let mut pic = None;
            let mut snd = None;

            if let Some(streams) = streams {
                for s in streams {
                    match s["codec_type"].as_str() {
                        Some("video") => {
                            let fps_str = s["r_frame_rate"].as_str().unwrap_or("24/1");
                            let (num, den) = parse_fraction(fps_str);
                            pic = Some(PictureDescriptor {
                                width: s["width"].as_u64().unwrap_or(0) as u32,
                                height: s["height"].as_u64().unwrap_or(0) as u32,
                                frame_rate_num: num,
                                frame_rate_den: den,
                                bit_depth: s["bits_per_raw_sample"]
                                    .as_str()
                                    .and_then(|b| b.parse().ok())
                                    .unwrap_or(0),
                                frame_count: s["nb_frames"]
                                    .as_str()
                                    .and_then(|n| n.parse().ok())
                                    .unwrap_or(0),
                            });
                        }
                        Some("audio") => {
                            snd = Some(SoundDescriptor {
                                sample_rate: s["sample_rate"]
                                    .as_str()
                                    .and_then(|r| r.parse().ok())
                                    .unwrap_or(0),
                                channels: s["channels"].as_u64().unwrap_or(0) as u32,
                                bit_depth: s["bits_per_raw_sample"]
                                    .as_str()
                                    .and_then(|b| b.parse().ok())
                                    .unwrap_or(s["bits_per_sample"].as_u64().unwrap_or(0) as u32),
                                duration: s["nb_frames"]
                                    .as_str()
                                    .and_then(|n| n.parse().ok())
                                    .unwrap_or(0),
                            });
                        }
                        _ => {}
                    }
                }
            }

            let etype = if pic.is_some() && snd.is_some() {
                "picture+sound"
            } else if pic.is_some() {
                "picture"
            } else if snd.is_some() {
                "sound"
            } else {
                "unknown"
            };

            (pic, snd, etype.to_string())
        }
        _ => (None, None, "unknown".to_string()),
    };

    let file_size_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    MxfInfo {
        valid: true,
        error: String::new(),
        essence_type,
        picture,
        sound,
        file_size_bytes,
    }
}

fn parse_fraction(s: &str) -> (u32, u32) {
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() == 2 {
        let num: u32 = parts[0].parse().unwrap_or(0);
        let den: u32 = parts[1].parse().unwrap_or(1);
        (num, den)
    } else {
        (s.parse().unwrap_or(0), 1)
    }
}
