use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::{Code, Note};

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
    /// WaveAudioDescriptor BlockAlign (bytes per sample frame). 0 when the prober
    /// did not report it, in which case the block-align check is skipped.
    pub block_align: u32,
    pub duration: u64,
}

/// DCI sound-essence checks (SMPTE ST 429-2 / 382M): 24-bit PCM, and a block
/// alignment of channels * bytes-per-sample. bit_depth/block_align of 0 mean the
/// prober did not report the field, so that check is skipped rather than firing.
pub fn check_sound_descriptor(snd: &SoundDescriptor, file: &Path) -> Vec<Note> {
    let mut notes = Vec::new();

    if snd.bit_depth != 0 && snd.bit_depth != 24 {
        notes.push(
            Note::error(
                Code::SoundInvalidQuantization,
                format!(
                    "Audio quantization is {}-bit, DCI requires 24-bit PCM",
                    snd.bit_depth
                ),
            )
            .with_file(file),
        );
    }

    if snd.block_align != 0 && snd.bit_depth != 0 && snd.channels != 0 {
        let expected = snd.channels * (snd.bit_depth / 8);
        if snd.block_align != expected {
            notes.push(
                Note::error(
                    Code::SoundInvalidBlockAlign,
                    format!(
                        "Audio block align is {} bytes, expected {} ({} channels x {}-bit)",
                        snd.block_align, expected, snd.channels, snd.bit_depth
                    ),
                )
                .with_file(file),
            );
        }
    }

    notes
}

/// Sound-descriptor + integrity checks for an encrypted sound MXF, read straight
/// from asdcplib (ffprobe can't see encrypted essence). Runs only on encrypted
/// PCM essence; cleartext sound is already covered by the ffprobe path in
/// `verify_dcp`, and non-PCM MXFs open-fail and yield nothing. With a covering
/// key the descriptor check runs and frame 0's HMAC/MIC is verified; without one
/// it skips (a note only when a KDM was supplied).
pub fn check_sound_essence_mxf(path: &Path, keys: &crate::kdm::ContentKeys) -> Vec<Note> {
    let mut notes = Vec::new();
    let Some(s) = path.to_str() else {
        return notes;
    };
    let mut reader = asdcplib::pcm::MxfReader::new();
    if reader.open_read(s).is_err() {
        return notes; // not a PCM MXF
    }
    let info = match reader.writer_info() {
        Ok(info) => info,
        Err(e) => {
            notes.push(
                Note::error(
                    Code::MxfUnreadable,
                    format!(
                        "sound essence checks did not run: the MXF writer info would not read: {e}"
                    ),
                )
                .with_file(path),
            );
            return notes;
        }
    };
    if !info.encrypted_essence {
        return notes; // cleartext sound handled by the ffprobe path
    }
    let essence = keys.resolve(&info);
    if essence.is_missing() {
        notes.extend(essence.skip_note(path));
        return notes;
    }
    let desc = match reader.audio_descriptor() {
        Ok(desc) => desc,
        Err(e) => {
            notes.push(
                Note::error(
                    Code::MxfUnreadable,
                    format!(
                        "the descriptor and frame 0 integrity checks did not run: the audio descriptor would not read: {e}"
                    ),
                )
                .with_file(path),
            );
            return notes;
        }
    };
    let snd = SoundDescriptor {
        sample_rate: (desc.audio_sampling_rate.quotient()).round() as u32,
        channels: desc.channel_count,
        bit_depth: desc.quantization_bits,
        block_align: desc.block_align,
        duration: desc.container_duration as u64,
    };
    notes.extend(check_sound_descriptor(&snd, path));

    // integrity: decrypt frame 0 and let asdcplib verify the MIC.
    let mut ctx = match essence.contexts() {
        Ok(c) => c,
        Err(e) => {
            notes.push(Note::error(Code::MxfUnreadable, e).with_file(path));
            return notes;
        }
    };
    let (dec, hmac) = match ctx.as_mut() {
        Some(c) => (Some(&mut c.dec), Some(&mut c.hmac)),
        None => (None, None),
    };
    let mut buf = vec![0u8; desc.block_align.max(1) as usize * 2048 + 8192];
    if let Err(e) = reader.read_frame(0, &mut buf, dec, hmac) {
        notes.push(
            Note::error(
                Code::MxfHashMismatch,
                format!("frame 0 integrity check (HMAC/MIC) failed: {e}"),
            )
            .with_file(path),
        );
    }
    notes
}

/// Stored width above which ST 429-2 Table 1 calls a picture format 4K.
const TWO_K_MAX_STORED_WIDTH: u32 = 2048;

/// Frame rates Table 1 allows for the 4K formats, as numerator over 1.
const FOUR_K_FRAME_RATES: [i32; 3] = [24, 25, 30];

/// ST 429-2 §8.2 Table 1: the 4K picture formats carry a frame rate of 24/1,
/// 25/1 or 30/1, where the 2K formats also allow 48/1, 50/1 and 60/1. §8.2 has a
/// monoscopic picture track's frame rate equal its edit rate, so the descriptor's
/// edit rate is the rate Table 1 constrains. The requirement is monoscopic only,
/// which this reader enforces by construction: 3D essence needs asdcplib's
/// stereoscopic reader and fails to open here.
pub fn check_picture_frame_rate_mxf(path: &Path) -> Vec<Note> {
    let mut notes = Vec::new();
    let Some(path_str) = path.to_str() else {
        return notes;
    };
    let mut reader = asdcplib::jp2k::MxfReader::new();
    if reader.open_read(path_str).is_err() {
        return notes; // not monoscopic JP2K picture essence
    }
    let desc = match reader.picture_descriptor() {
        Ok(desc) => desc,
        Err(e) => {
            notes.push(
                Note::error(
                    Code::MxfUnreadable,
                    format!(
                        "the Table 1 frame-rate check did not run: the picture descriptor would not read: {e}"
                    ),
                )
                .with_file(path),
            );
            return notes;
        }
    };

    let (numerator, denominator) = (desc.edit_rate.numerator, desc.edit_rate.denominator);
    if desc.stored_width <= TWO_K_MAX_STORED_WIDTH {
        return notes; // Table 1 allows every composition rate at 2K
    }
    if numerator <= 0 || denominator <= 0 {
        notes.push(
            Note::error(
                Code::PictureInvalidFrameRate,
                format!(
                    "4K picture essence ({}x{}) declares an edit rate of {numerator}/{denominator}, which is no frame rate",
                    desc.stored_width, desc.stored_height
                ),
            )
            .with_file(path),
        );
        return notes;
    }
    if denominator == 1 && FOUR_K_FRAME_RATES.contains(&numerator) {
        return notes;
    }

    notes.push(
        Note::error(
            Code::PictureInvalidFrameRate,
            format!(
                "4K picture essence ({}x{}) at {numerator}/{denominator} fps: ST 429-2 Table 1 allows 24/1, 25/1 or 30/1 above 2K",
                desc.stored_width, desc.stored_height
            ),
        )
        .with_file(path),
    );
    notes
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
    /// Why ffprobe listed no streams for this file, empty when it listed some.
    /// The descriptors are what the picture checks are gated on, so a probe that
    /// produced nothing has to be told apart from essence that carries nothing.
    #[serde(default)]
    pub stream_probe_error: String,
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

    let mut stream_probe_error = String::new();
    let (picture, sound) = match output {
        Ok(o) if o.status.success() => {
            let json: serde_json::Value = match serde_json::from_slice(&o.stdout) {
                Ok(json) => json,
                Err(e) => {
                    stream_probe_error = format!("ffprobe output is not JSON: {e}");
                    serde_json::Value::default()
                }
            };

            let streams = json["streams"].as_array();
            if streams.is_none_or(|streams| streams.is_empty()) && stream_probe_error.is_empty() {
                stream_probe_error = "ffprobe listed no streams".to_string();
            }

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
                                block_align: s["block_align"]
                                    .as_str()
                                    .and_then(|b| b.parse().ok())
                                    .unwrap_or(s["block_align"].as_u64().unwrap_or(0) as u32),
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

            (pic, snd)
        }
        Ok(o) => {
            stream_probe_error = format!("ffprobe exited with {}", o.status);
            (None, None)
        }
        Err(e) => {
            stream_probe_error = format!("ffprobe would not run: {e}");
            (None, None)
        }
    };

    let file_size_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    // ffprobe reports only some WaveAudioDescriptor fields for an MXF and the
    // rest arrive as 0, which makes the check for that field skip itself
    let sound = wave_sound_descriptor(path).or(sound);

    let essence_type = match (picture.is_some(), sound.is_some()) {
        (true, true) => "picture+sound",
        (true, false) => "picture",
        (false, true) => "sound",
        (false, false) => "unknown",
    }
    .to_string();

    MxfInfo {
        valid: true,
        error: String::new(),
        essence_type,
        picture,
        sound,
        file_size_bytes,
        stream_probe_error,
    }
}

/// The WaveAudioDescriptor of a cleartext PCM MXF, read through asdcplib.
/// Returns None for anything that is not one: a picture or aux-data MXF fails to
/// open, an IMF audio track file is AS-02 and this reader is OP-Atom, and encrypted
/// essence is `check_sound_essence_mxf`'s job, so answering here too would report
/// the same defect twice. Each of those falls back to ffprobe.
/// true when asdcplib opens the file as PCM sound essence, AS-DCP or AS-02. Tells
/// a sound track file from a picture one without ffprobe, so a report can leave a
/// sound track out of the picture sections and a picture track out of the sound
/// ones instead of relying on each check failing quietly.
pub fn is_pcm_sound_essence(path: &Path) -> bool {
    use asdcplib::EssenceType;
    let Some(path) = path.to_str() else {
        return false;
    };
    matches!(
        asdcplib::essence_type(path),
        Ok(EssenceType::Pcm24b48k
            | EssenceType::Pcm24b96k
            | EssenceType::As02Pcm24b48k
            | EssenceType::As02Pcm24b96k)
    )
}

/// True when asdcplib identifies the essence as a type the picture checks never
/// apply to, so ffprobe failing on it does not mean a check was skipped.
pub fn known_non_picture_essence(path: &Path) -> bool {
    use asdcplib::EssenceType;
    let Some(path) = path.to_str() else {
        return false;
    };
    matches!(
        asdcplib::essence_type(path),
        Ok(EssenceType::Pcm24b48k
            | EssenceType::Pcm24b96k
            | EssenceType::As02Pcm24b48k
            | EssenceType::As02Pcm24b96k
            | EssenceType::TimedText
            | EssenceType::As02TimedText
            | EssenceType::DcDataUnknown
            | EssenceType::DcDataDolbyAtmos
            | EssenceType::As02Isxd
            | EssenceType::As02Iab)
    )
}

fn wave_sound_descriptor(path: &Path) -> Option<SoundDescriptor> {
    let mut reader = asdcplib::pcm::MxfReader::new();
    reader.open_read(path.to_str()?).ok()?;
    if reader.writer_info().ok()?.encrypted_essence {
        return None;
    }
    let desc = reader.audio_descriptor().ok()?;
    Some(SoundDescriptor {
        sample_rate: desc.audio_sampling_rate.quotient().round() as u32,
        channels: desc.channel_count,
        bit_depth: desc.quantization_bits,
        block_align: desc.block_align,
        duration: desc.container_duration as u64,
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Code;
    use std::path::Path;

    #[test]
    fn twenty_four_bit_pcm_passes() {
        let snd = SoundDescriptor {
            sample_rate: 48000,
            channels: 6,
            bit_depth: 24,
            block_align: 18, // 6 channels x 3 bytes
            duration: 0,
        };
        assert!(check_sound_descriptor(&snd, Path::new("a.mxf")).is_empty());
    }

    #[test]
    fn sixteen_bit_pcm_flags_quantization() {
        let snd = SoundDescriptor {
            sample_rate: 48000,
            channels: 6,
            bit_depth: 16,
            block_align: 0,
            duration: 0,
        };
        let notes = check_sound_descriptor(&snd, Path::new("a.mxf"));
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::SoundInvalidQuantization)
        );
    }

    #[test]
    fn wrong_block_align_flags_when_reported() {
        let snd = SoundDescriptor {
            sample_rate: 48000,
            channels: 6,
            bit_depth: 24,
            block_align: 12, // should be 18
            duration: 0,
        };
        let notes = check_sound_descriptor(&snd, Path::new("a.mxf"));
        assert!(notes.iter().any(|n| n.code == Code::SoundInvalidBlockAlign));
    }

    #[test]
    fn unknown_fields_skip_checks() {
        let snd = SoundDescriptor::default(); // all zero
        assert!(check_sound_descriptor(&snd, Path::new("a.mxf")).is_empty());
    }

    /// Write a one-frame monoscopic picture MXF of the given format and rate.
    fn write_picture_mxf(path: &Path, width: u32, height: u32, frame_rate: i32) {
        use asdcplib::jp2k::{MxfWriter, PictureDescriptor};
        use asdcplib::{LabelSet, Rational, WriterInfo};

        let info = WriterInfo {
            asset_uuid: *uuid::Uuid::new_v4().as_bytes(),
            context_id: *uuid::Uuid::new_v4().as_bytes(),
            label_set: LabelSet::Smpte,
            ..Default::default()
        };
        let descriptor = PictureDescriptor {
            edit_rate: Rational::new(frame_rate, 1),
            sample_rate: Rational::new(frame_rate, 1),
            stored_width: width,
            stored_height: height,
            aspect_ratio: Rational::new(width as i32, height as i32),
            container_duration: 1,
            component_count: 3,
        };
        let mut writer = MxfWriter::new();
        writer
            .open_write(path.to_str().unwrap(), &info, &descriptor, 16384)
            .unwrap();
        writer.write_frame(&vec![0u8; 1024], None, None).unwrap();
        writer.finalize().unwrap();
    }

    #[test]
    fn four_k_above_thirty_fps_violates_table_1() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("picture_4k_48.mxf");
        write_picture_mxf(&path, 4096, 2160, 48);

        let notes = check_picture_frame_rate_mxf(&path);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert_eq!(notes[0].code, Code::PictureInvalidFrameRate);
        assert_eq!(notes[0].severity, crate::Severity::Error);
        assert!(
            notes[0].message.contains("48/1") && notes[0].message.contains("4096x2160"),
            "{}",
            notes[0].message
        );
    }

    #[test]
    fn four_k_at_twenty_four_fps_is_clean() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("picture_4k_24.mxf");
        write_picture_mxf(&path, 4096, 2160, 24);

        assert!(check_picture_frame_rate_mxf(&path).is_empty());
    }

    #[test]
    fn two_k_at_forty_eight_fps_is_clean() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("picture_2k_48.mxf");
        write_picture_mxf(&path, 2048, 1080, 48);

        assert!(check_picture_frame_rate_mxf(&path).is_empty());
    }

    // Table 1 exempts the 2K formats, not a 4K descriptor whose rate will not read
    #[test]
    fn four_k_with_no_frame_rate_fires_instead_of_being_exempted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("picture_4k_no_rate.mxf");
        write_picture_mxf(&path, 4096, 2160, 0);

        let notes = check_picture_frame_rate_mxf(&path);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert_eq!(notes[0].code, Code::PictureInvalidFrameRate);
        assert_eq!(notes[0].severity, crate::Severity::Error);
        assert!(
            notes[0].message.contains("0/1"),
            "the note must name the rate it read, got: {}",
            notes[0].message
        );
    }

    #[test]
    fn two_k_with_no_frame_rate_stays_silent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("picture_2k_no_rate.mxf");
        write_picture_mxf(&path, 2048, 1080, 0);

        assert!(
            check_picture_frame_rate_mxf(&path).is_empty(),
            "Table 1 constrains the 4K formats, so 2K has nothing to violate"
        );
    }

    #[test]
    fn picture_essence_is_not_pcm_sound() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("picture.mxf");
        write_picture_mxf(&path, 2048, 1080, 24);

        assert!(!is_pcm_sound_essence(&path));
        assert!(!is_pcm_sound_essence(&dir.path().join("absent.mxf")));
    }
}
