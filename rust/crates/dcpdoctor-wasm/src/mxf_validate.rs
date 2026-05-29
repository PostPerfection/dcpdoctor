//! MXF validation logic — checks extracted MXF metadata against DCI specs.

use crate::mxf::{EssenceType, MxfMetadata};
use crate::{Note, Severity};

/// Validate MXF metadata against DCI specifications.
/// Returns a list of validation notes.
pub fn validate_mxf(path: &str, meta: &MxfMetadata) -> Vec<Note> {
    let mut notes = Vec::new();

    if !meta.valid {
        if let Some(err) = &meta.error {
            notes.push(Note {
                severity: Severity::Error,
                code: "mxf_invalid".to_string(),
                message: format!("Invalid MXF: {err}"),
                file: Some(path.to_string()),
            });
        }
        return notes;
    }

    // Partition structure checks
    if !meta.partitions.has_footer {
        notes.push(Note {
            severity: Severity::Warning,
            code: "mxf_no_footer".to_string(),
            message: "MXF missing footer partition (may cause playback issues)".to_string(),
            file: Some(path.to_string()),
        });
    }

    if !meta.partitions.closed_complete {
        notes.push(Note {
            severity: Severity::Info,
            code: "mxf_not_closed".to_string(),
            message: "MXF header partition not Closed & Complete".to_string(),
            file: Some(path.to_string()),
        });
    }

    // Picture validation
    if let Some(pic) = &meta.picture {
        validate_picture(path, pic, &mut notes);
    }

    // Sound validation
    if let Some(snd) = &meta.sound {
        validate_sound(path, snd, &mut notes);
    }

    // Writer info
    if let Some(writer) = &meta.writer_info {
        if writer.encrypted {
            notes.push(Note {
                severity: Severity::Info,
                code: "mxf_encrypted".to_string(),
                message: "MXF contains encrypted essence".to_string(),
                file: Some(path.to_string()),
            });
        }
    }

    notes
}

fn validate_picture(path: &str, pic: &crate::mxf::PictureDescriptor, notes: &mut Vec<Note>) {
    // DCI resolution check
    let valid_resolutions = [
        (2048, 1080), // 2K Scope
        (1998, 1080), // 2K Flat
        (4096, 2160), // 4K Scope
        (3996, 2160), // 4K Flat
    ];

    if pic.width > 0 && pic.height > 0 {
        let is_valid_res = valid_resolutions
            .iter()
            .any(|(w, h)| *w == pic.width && *h == pic.height);

        if !is_valid_res {
            notes.push(Note {
                severity: Severity::Warning,
                code: "mxf_unusual_resolution".to_string(),
                message: format!(
                    "Non-standard DCI resolution: {}×{} (expected 2K or 4K scope/flat)",
                    pic.width, pic.height
                ),
                file: Some(path.to_string()),
            });
        }
    }

    // Frame rate check
    if pic.frame_rate_num > 0 && pic.frame_rate_den > 0 {
        let fps = pic.frame_rate_num as f64 / pic.frame_rate_den as f64;
        let valid_fps = [24.0, 25.0, 30.0, 48.0, 60.0];
        let is_valid_fps = valid_fps.iter().any(|f| (fps - f).abs() < 0.01);

        if !is_valid_fps {
            notes.push(Note {
                severity: Severity::Warning,
                code: "mxf_unusual_framerate".to_string(),
                message: format!(
                    "Non-standard frame rate: {}/{} ({:.2} fps)",
                    pic.frame_rate_num, pic.frame_rate_den, fps
                ),
                file: Some(path.to_string()),
            });
        }
    }

    // Bit depth check (DCI requires 12-bit for JPEG 2000)
    if pic.bit_depth > 0 && pic.bit_depth != 12 {
        notes.push(Note {
            severity: Severity::Info,
            code: "mxf_bit_depth".to_string(),
            message: format!(
                "Picture bit depth {} (DCI standard is 12-bit)",
                pic.bit_depth
            ),
            file: Some(path.to_string()),
        });
    }

    // Essence type info
    if let Some(etype) = &pic.essence_type {
        if *etype != EssenceType::Jpeg2000 {
            notes.push(Note {
                severity: Severity::Info,
                code: "mxf_non_j2k".to_string(),
                message: format!("Picture essence type: {} (not JPEG 2000)", etype.as_str()),
                file: Some(path.to_string()),
            });
        }
    }
}

fn validate_sound(path: &str, snd: &crate::mxf::SoundDescriptor, notes: &mut Vec<Note>) {
    // Sample rate check (DCI requires 48kHz or 96kHz)
    if snd.sample_rate > 0 && snd.sample_rate != 48000 && snd.sample_rate != 96000 {
        notes.push(Note {
            severity: Severity::Warning,
            code: "mxf_unusual_sample_rate".to_string(),
            message: format!(
                "Non-standard audio sample rate: {} Hz (DCI requires 48000 or 96000)",
                snd.sample_rate
            ),
            file: Some(path.to_string()),
        });
    }

    // Channel count info
    if snd.channels > 0 && snd.channels < 6 {
        notes.push(Note {
            severity: Severity::Info,
            code: "mxf_low_channel_count".to_string(),
            message: format!(
                "Audio has {} channels (theatrical typically requires 5.1 or more)",
                snd.channels
            ),
            file: Some(path.to_string()),
        });
    }

    // Bit depth check (DCI requires 24-bit)
    if snd.bit_depth > 0 && snd.bit_depth != 24 {
        notes.push(Note {
            severity: Severity::Info,
            code: "mxf_audio_bit_depth".to_string(),
            message: format!("Audio bit depth {} (DCI standard is 24-bit)", snd.bit_depth),
            file: Some(path.to_string()),
        });
    }
}
