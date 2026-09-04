//! Advanced MXF analysis: partition validation, DTS:X detection.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde::Serialize;

use crate::{Code, Note, Severity};

/// MXF partition pack key prefix (SMPTE 377-1).
const PARTITION_PACK_KEY: [u8; 13] = [
    0x06, 0x0e, 0x2b, 0x34, 0x02, 0x05, 0x01, 0x01, 0x0d, 0x01, 0x02, 0x01, 0x01,
];

const PARTITION_PACK_BYTES: usize = 16;
const FOOTER_PARTITION_KIND: u8 = 0x04;
const BODY_PARTITION_KIND: u8 = 0x03;

// the last pack in the file
const RIP_KEY: [u8; 16] = [
    0x06, 0x0e, 0x2b, 0x34, 0x02, 0x05, 0x01, 0x01, 0x0d, 0x01, 0x02, 0x01, 0x01, 0x11, 0x01, 0x00,
];
const RIP_ENTRY_BYTES: usize = 12;
const RIP_LENGTH_FIELD_BYTES: u64 = 4;
// a RIP is 12 bytes a partition, a longer length field is not a RIP
const RIP_MAX_BYTES: u64 = 1 << 20;

const FOOTER_SCAN_BYTES: u64 = 65536;

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
    let opened = std::fs::File::open(mxf_path).and_then(|file| Ok((file.metadata()?.len(), file)));
    let (len, file) = match opened {
        Ok(opened) => opened,
        Err(_) => {
            return MxfPartitionInfo {
                error: "Cannot open MXF file".into(),
                ..Default::default()
            };
        }
    };
    match read_partitions(file, len) {
        Ok(info) => info,
        Err(e) => MxfPartitionInfo {
            error: format!("Cannot read MXF file: {e}"),
            ..Default::default()
        },
    }
}

// picture track files run to tens of GB
pub(crate) fn read_partitions(
    mut essence: impl Read + Seek,
    len: u64,
) -> std::io::Result<MxfPartitionInfo> {
    let mut info = MxfPartitionInfo::default();
    if len < PARTITION_PACK_BYTES as u64 {
        info.error = "File too small for MXF".into();
        return Ok(info);
    }

    let mut head = [0u8; PARTITION_PACK_BYTES];
    essence.read_exact(&mut head)?;
    if head[..13] == PARTITION_PACK_KEY {
        info.has_header_partition = true;
        info.closed_complete = head[14] >= 0x04;
    }

    info.header_size = len;

    if let Some(offset) = footer_offset_from_rip(&mut essence, len)? {
        info.has_footer_partition = true;
        info.footer_offset = offset as i64;
        info.valid = true;
        return Ok(info);
    }

    // the footer's index table alone can be longer than this window
    let scan_start = len.saturating_sub(FOOTER_SCAN_BYTES);
    essence.seek(SeekFrom::Start(scan_start))?;
    let mut tail = Vec::new();
    essence
        .by_ref()
        .take(FOOTER_SCAN_BYTES)
        .read_to_end(&mut tail)?;

    for i in 0..tail.len().saturating_sub(PARTITION_PACK_BYTES) {
        if tail[i..i + 13] == PARTITION_PACK_KEY {
            let partition_type = tail[i + 13];
            if partition_type == FOOTER_PARTITION_KIND {
                info.has_footer_partition = true;
                info.footer_offset = (scan_start + i as u64) as i64;
            } else if partition_type == BODY_PARTITION_KIND {
                info.has_body_partition = true;
                info.body_partition_count += 1;
            }
        }
    }

    info.valid = true;
    Ok(info)
}

fn footer_offset_from_rip(
    essence: &mut (impl Read + Seek),
    len: u64,
) -> std::io::Result<Option<u64>> {
    if len < RIP_LENGTH_FIELD_BYTES {
        return Ok(None);
    }
    essence.seek(SeekFrom::Start(len - RIP_LENGTH_FIELD_BYTES))?;
    let mut rip_len_bytes = [0u8; RIP_LENGTH_FIELD_BYTES as usize];
    essence.read_exact(&mut rip_len_bytes)?;
    let rip_len = u64::from(u32::from_be_bytes(rip_len_bytes));
    let smallest_rip = RIP_KEY.len() as u64 + 1 + RIP_LENGTH_FIELD_BYTES;
    if rip_len < smallest_rip || rip_len > len || rip_len > RIP_MAX_BYTES {
        return Ok(None);
    }

    essence.seek(SeekFrom::Start(len - rip_len))?;
    let mut rip = vec![0u8; rip_len as usize];
    essence.read_exact(&mut rip)?;
    if rip[..RIP_KEY.len()] != RIP_KEY {
        return Ok(None);
    }
    // BER length, 0x80 | n means n length bytes follow
    let length_byte = rip[RIP_KEY.len()];
    let entries_start = match length_byte {
        short if short < 0x80 => RIP_KEY.len() + 1,
        long => RIP_KEY.len() + 1 + usize::from(long & 0x7f),
    };
    let entries_end = rip.len() - RIP_LENGTH_FIELD_BYTES as usize;
    let Some(entries) = rip.get(entries_start..entries_end) else {
        return Ok(None);
    };
    let (entries, _partial_entry) = entries.as_chunks::<RIP_ENTRY_BYTES>();
    let Some(last_entry) = entries.last() else {
        return Ok(None);
    };
    let offset = u64::from_be_bytes(last_entry[4..12].try_into().expect("8 byte offset"));
    if offset + PARTITION_PACK_BYTES as u64 > len {
        return Ok(None);
    }

    essence.seek(SeekFrom::Start(offset))?;
    let mut pack = [0u8; PARTITION_PACK_BYTES];
    essence.read_exact(&mut pack)?;
    let is_footer = pack[..13] == PARTITION_PACK_KEY && pack[13] == FOOTER_PARTITION_KIND;
    Ok(is_footer.then_some(offset))
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
    use std::io::Cursor;

    struct CountingReader<R> {
        inner: R,
        bytes_read: u64,
    }

    impl<R: Read> Read for CountingReader<R> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.inner.read(buf)?;
            self.bytes_read += n as u64;
            Ok(n)
        }
    }

    impl<R: Seek> Seek for CountingReader<R> {
        fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
            self.inner.seek(pos)
        }
    }

    const BODY_PARTITION_AT: usize = 16384;

    fn track_file_bytes(len: usize, footer_at: usize, with_rip: bool) -> Vec<u8> {
        let mut bytes = vec![0u8; len];
        bytes[..13].copy_from_slice(&PARTITION_PACK_KEY);
        bytes[13] = 0x02;
        bytes[14] = 0x04;
        bytes[BODY_PARTITION_AT..BODY_PARTITION_AT + 13].copy_from_slice(&PARTITION_PACK_KEY);
        bytes[BODY_PARTITION_AT + 13] = BODY_PARTITION_KIND;
        bytes[footer_at..footer_at + 13].copy_from_slice(&PARTITION_PACK_KEY);
        bytes[footer_at + 13] = FOOTER_PARTITION_KIND;
        if with_rip {
            let mut rip = RIP_KEY.to_vec();
            let entries = [
                (0u32, 0u64),
                (1, BODY_PARTITION_AT as u64),
                (0, footer_at as u64),
            ];
            let entries_len = entries.len() * RIP_ENTRY_BYTES + RIP_LENGTH_FIELD_BYTES as usize;
            rip.push(0x83);
            rip.extend_from_slice(&(entries_len as u32).to_be_bytes()[1..]);
            for (sid, offset) in entries {
                rip.extend_from_slice(&sid.to_be_bytes());
                rip.extend_from_slice(&offset.to_be_bytes());
            }
            rip.extend_from_slice(&(rip.len() as u32 + 4).to_be_bytes());
            let rip_at = len - rip.len();
            bytes[rip_at..].copy_from_slice(&rip);
        }
        bytes
    }

    fn counted_partitions(bytes: &[u8]) -> (MxfPartitionInfo, u64) {
        let mut reader = CountingReader {
            inner: Cursor::new(bytes),
            bytes_read: 0,
        };
        let info = read_partitions(&mut reader, bytes.len() as u64).unwrap();
        (info, reader.bytes_read)
    }

    #[test]
    fn the_rip_finds_a_footer_whose_index_table_is_longer_than_the_scan_window() {
        let len = 8 << 20;
        let footer_at = len - 300_000;
        let (info, bytes_read) = counted_partitions(&track_file_bytes(len, footer_at, true));

        assert!(info.valid);
        assert!(info.has_header_partition);
        assert!(info.closed_complete);
        assert!(info.has_footer_partition);
        assert_eq!(info.footer_offset, footer_at as i64);
        assert_eq!(info.header_size, len as u64);
        assert!(
            bytes_read < 1024,
            "read {bytes_read} bytes of a {len} byte track file"
        );
    }

    #[test]
    fn without_a_rip_the_tail_scan_reads_the_head_and_the_tail_not_the_essence() {
        let len = 8 << 20;
        let footer_at = len - 1000;
        let (info, bytes_read) = counted_partitions(&track_file_bytes(len, footer_at, false));

        assert!(info.valid);
        assert!(info.has_footer_partition);
        assert_eq!(info.footer_offset, footer_at as i64);
        assert!(
            bytes_read <= 2 * PARTITION_PACK_BYTES as u64 + FOOTER_SCAN_BYTES,
            "read {bytes_read} bytes of a {len} byte track file"
        );
    }

    #[test]
    fn a_track_file_on_disk_reports_its_partitions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("picture.mxf");
        let len = 200_000;
        let footer_at = len - 500;
        std::fs::write(&path, track_file_bytes(len, footer_at, true)).unwrap();

        let info = validate_mxf_partitions(&path);

        assert!(info.valid, "{}", info.error);
        assert!(info.has_header_partition);
        assert!(info.has_footer_partition);
        assert_eq!(info.footer_offset, footer_at as i64);
    }

    #[test]
    fn a_file_shorter_than_a_partition_pack_is_too_small() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("picture.mxf");
        std::fs::write(&path, b"short").unwrap();

        let info = validate_mxf_partitions(&path);

        assert!(!info.valid);
        assert_eq!(info.error, "File too small for MXF");
    }

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
