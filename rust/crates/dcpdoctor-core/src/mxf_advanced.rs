//! Advanced MXF analysis: partition validation, DTS:X detection.

use std::path::Path;

use serde::Serialize;

use crate::{Code, Note, Severity};

/// MXF partition pack key prefix (SMPTE 377-1).
const PARTITION_PACK_KEY: [u8; 13] = [
    0x06, 0x0e, 0x2b, 0x34, 0x02, 0x05, 0x01, 0x01, 0x0d, 0x01, 0x02, 0x01, 0x01,
];

/// Information about MXF partition structure.
#[derive(Debug, Clone, Default, Serialize)]
pub struct MxfPartitionInfo {
    pub valid: bool,
    pub error: String,
    pub has_header_partition: bool,
    pub has_body_partition: bool,
    pub has_footer_partition: bool,
    pub closed_complete: bool,
    pub body_partition_count: u32,
    pub header_size: u64,
    pub footer_offset: i64,
}

/// DTS:X detection result.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DtsxInfo {
    pub detected: bool,
    pub immersive: bool,
    pub channel_count: u32,
    pub version: String,
    /// Why the channel count could not be read, empty when it was. Without it a
    /// probe that never ran looks the same as a track carrying no DTS:X.
    pub error: String,
}

/// Validate MXF file partition structure (header, body, footer).
pub fn validate_mxf_partitions(mxf_path: &Path) -> MxfPartitionInfo {
    let mut info = MxfPartitionInfo::default();

    let data = match std::fs::read(mxf_path) {
        Ok(d) => d,
        Err(_) => {
            info.error = "Cannot open MXF file".into();
            return info;
        }
    };

    if data.len() < 16 {
        info.error = "File too small for MXF".into();
        return info;
    }

    // Check header partition (first 16 bytes)
    if data[..13] == PARTITION_PACK_KEY {
        info.has_header_partition = true;
        info.closed_complete = data[14] >= 0x04;
    }

    info.header_size = data.len() as u64;

    // Scan last portion for footer/body partitions
    let scan_start = data.len().saturating_sub(65536);
    let tail = &data[scan_start..];

    for i in 0..tail.len().saturating_sub(16) {
        if tail[i..i + 13] == PARTITION_PACK_KEY {
            let partition_type = tail[i + 13];
            if partition_type == 0x04 {
                info.has_footer_partition = true;
                info.footer_offset = (scan_start + i) as i64;
            } else if partition_type == 0x03 {
                info.has_body_partition = true;
                info.body_partition_count += 1;
            }
        }
    }

    info.valid = true;
    info
}

/// Generate validation notes from MXF partition info.
pub fn check_mxf_partitions(info: &MxfPartitionInfo, mxf_path: &Path) -> Vec<Note> {
    let mut notes = Vec::new();
    if !info.valid {
        return notes;
    }

    let path_buf = Some(mxf_path.to_path_buf());

    if !info.has_header_partition {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::MxfInvalidStructure,
            message: "MXF missing header partition".into(),
            file: path_buf.clone(),
            line: 0,
        });
    }

    if !info.has_footer_partition {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::MxfInvalidStructure,
            message: "MXF missing footer partition (may cause playback issues on some servers)"
                .into(),
            file: path_buf.clone(),
            line: 0,
        });
    }

    if !info.closed_complete {
        notes.push(Note {
            severity: Severity::Info,
            code: Code::MxfInvalidStructure,
            message: "MXF header partition not Closed & Complete".into(),
            file: path_buf,
            line: 0,
        });
    }

    notes
}

/// Detect DTS:X immersive audio in an MXF file.
pub fn detect_dtsx(mxf_path: &Path) -> DtsxInfo {
    let mut info = DtsxInfo::default();

    // Use ffprobe to check channel count and metadata
    let output = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=channels,codec_name",
            "-of",
            "csv=p=0",
            &mxf_path.to_string_lossy(),
        ])
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout);
            let parts: Vec<&str> = s.trim().split(',').collect();
            match parts.get(1).map(|channels| channels.parse::<u32>()) {
                Some(Ok(channels)) => {
                    if channels > 8 {
                        info.channel_count = channels;
                        info.detected = true;
                        info.immersive = true;
                    }
                }
                Some(Err(e)) => info.error = format!("ffprobe reported no channel count: {e}"),
                None => info.error = "ffprobe reported no audio stream".into(),
            }
        }
        Ok(o) => info.error = format!("ffprobe exited with {}", o.status),
        Err(e) => info.error = format!("ffprobe would not run: {e}"),
    }

    info
}

/// Generate compliance notes for DTS:X content.
pub fn check_dtsx_compliance(info: &DtsxInfo, mxf_path: &Path) -> Vec<Note> {
    let mut notes = Vec::new();
    if !info.error.is_empty() {
        notes.push(
            Note::warning(
                Code::CheckSkipped,
                format!("the DTS:X check did not run: {}", info.error),
            )
            .with_file(mxf_path),
        );
        return notes;
    }
    if !info.detected {
        return notes;
    }

    let path_buf = Some(mxf_path.to_path_buf());

    notes.push(Note {
        severity: Severity::Info,
        code: Code::SoundInvalidChannelCount,
        message: format!(
            "DTS:X Immersive Audio detected ({} channels)",
            info.channel_count
        ),
        file: path_buf.clone(),
        line: 0,
    });

    if info.channel_count < 12 {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::SoundInvalidChannelCount,
            message: "DTS:X typically requires 12+ channels for full immersive experience".into(),
            file: path_buf,
            line: 0,
        });
    }

    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_probe_says_the_dtsx_check_did_not_run() {
        let info = DtsxInfo {
            error: "ffprobe would not run: No such file or directory".into(),
            ..Default::default()
        };
        let notes = check_dtsx_compliance(&info, Path::new("sound.mxf"));
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert_eq!(notes[0].code, Code::CheckSkipped);
        assert_eq!(notes[0].severity, Severity::Warning);
        assert!(notes[0].message.contains("ffprobe would not run"));
    }

    #[test]
    fn a_track_without_dtsx_stays_silent() {
        assert!(check_dtsx_compliance(&DtsxInfo::default(), Path::new("sound.mxf")).is_empty());
    }

    #[test]
    fn a_probe_that_read_no_channel_count_is_an_error_not_an_absence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("picture.mxf");
        std::fs::write(&path, b"not an MXF").unwrap();

        let info = detect_dtsx(&path);
        assert!(!info.detected);
        assert!(
            !info.error.is_empty(),
            "a file ffprobe reports no audio stream for must carry a reason"
        );
    }
}
