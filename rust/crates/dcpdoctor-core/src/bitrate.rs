//! Per-frame bitrate analysis of J2K MXF picture tracks.
//!
//! Every frame is read through the picture reader in `j2k`, which covers AS-DCP
//! monoscopic, AS-DCP stereoscopic and AS-02 wrapping and takes the decryption
//! contexts encrypted essence needs. postkit's own bitrate readers take none, so
//! they cannot measure an encrypted track. One edit unit of stereoscopic essence
//! carries a left and a right codestream that share the frame's time slot, so
//! the pair counts as one frame. The Note-producing compliance check stays here.

use std::path::Path;

use crate::j2k::{PictureEssenceFamily, PictureEssenceReader};
use crate::kdm::ContentKeys;
use crate::{Code, Note};

/// DCI caps a frame near 1.3 MB (2K) / 2.6 MB (4K); 16 MiB is safe headroom.
const FRAME_BUFFER_BYTES: usize = 16 * 1024 * 1024;

/// Peak bitrate above this fraction of the limit is reported as a warning.
const NEAR_LIMIT_FRACTION: f64 = 0.95;

/// The `error` a track whose content key the run does not hold reports. The
/// callers that know about the KDM report that case themselves, so the skipped
/// measurement note leaves it alone.
pub const NO_CONTENT_KEY_ERROR: &str = "Encrypted essence with no content key";

/// Frame-level bitrate statistics for a picture MXF (postkit's reader output).
pub type FrameBitrateStats = postkit::j2k::MxfBitrateStats;

/// Measure per-frame bitrate of a picture MXF by reading every frame, decrypting
/// with `keys` where the essence is encrypted. Encrypted essence with no covering
/// key measures nothing: its ciphertext frames are longer than the codestreams
/// they carry, so a rate read off them is not the asset's rate.
///
/// AS-02 is tried after the AS-DCP wrappings because the AS-02 reader also opens
/// stereoscopic AS-DCP essence and sees one eye per frame.
pub fn analyze_picture_bitrate(mxf_path: &Path, keys: &ContentKeys) -> FrameBitrateStats {
    let as_dcp = measure_frames(mxf_path, keys, PictureEssenceFamily::Cinema);
    if as_dcp.valid {
        return as_dcp;
    }
    let as02 = measure_frames(mxf_path, keys, PictureEssenceFamily::Imf);
    if as02.valid { as02 } else { as_dcp }
}

/// Read every edit unit of one picture track and turn the codestream sizes into
/// bitrate statistics. An unopenable file, an essence we hold no key for, or a
/// track whose first frame will not read all come back invalid, which is how the
/// caller falls through to the other wrapping and how the reporting stays quiet.
fn measure_frames(
    mxf_path: &Path,
    keys: &ContentKeys,
    family: PictureEssenceFamily,
) -> FrameBitrateStats {
    let mut stats = FrameBitrateStats::default();

    let Some(path) = mxf_path.to_str() else {
        stats.error = "non-UTF-8 MXF path".into();
        return stats;
    };
    let Some(mut reader) = PictureEssenceReader::open(path, family) else {
        stats.error = "Failed to open picture essence".into();
        return stats;
    };
    let Ok(descriptor) = reader.picture_descriptor() else {
        stats.error = "Failed to read picture descriptor".into();
        return stats;
    };
    let Ok(info) = reader.writer_info() else {
        stats.error = "Failed to read the MXF writer info".into();
        return stats;
    };

    let essence = keys.resolve(&info);
    if essence.is_missing() {
        stats.error = NO_CONTENT_KEY_ERROR.into();
        return stats;
    }
    let mut contexts = match essence.contexts() {
        Ok(contexts) => contexts,
        Err(e) => {
            stats.error = e;
            return stats;
        }
    };

    stats.width = descriptor.stored_width;
    stats.height = descriptor.stored_height;
    stats.frame_rate =
        descriptor.edit_rate.numerator as f64 / descriptor.edit_rate.denominator.max(1) as f64;
    if descriptor.container_duration == 0 || stats.frame_rate <= 0.0 {
        stats.error = "Invalid frame count or rate".into();
        return stats;
    }

    let eyes = reader.eyes();
    let mut buffer = vec![0u8; FRAME_BUFFER_BYTES];
    let mut min_frame_bytes = u64::MAX;
    let mut frames_read = 0u32;

    'edit_units: for index in 0..descriptor.container_duration {
        let mut frame_bytes = 0u64;
        for &eye in eyes {
            let (dec, hmac) = match contexts.as_mut() {
                Some(contexts) => (Some(&mut contexts.dec), Some(&mut contexts.hmac)),
                None => (None, None),
            };
            let Ok(read) = reader.read_frame(index, eye, &mut buffer, dec, hmac) else {
                break 'edit_units;
            };
            frame_bytes += read as u64;
        }
        stats.total_bytes += frame_bytes;
        if frame_bytes > stats.max_frame_bytes {
            stats.max_frame_bytes = frame_bytes;
            stats.max_frame_index = index;
        }
        min_frame_bytes = min_frame_bytes.min(frame_bytes);
        frames_read += 1;
    }

    if frames_read == 0 {
        stats.error = "No frames could be read".into();
        return stats;
    }

    let megabits_per_second = |bytes: f64| bytes * 8.0 * stats.frame_rate / 1_000_000.0;
    stats.frame_count = frames_read;
    stats.min_frame_bytes = min_frame_bytes;
    stats.avg_bitrate_mbps = megabits_per_second(stats.total_bytes as f64 / frames_read as f64);
    stats.min_bitrate_mbps = megabits_per_second(min_frame_bytes as f64);
    stats.max_bitrate_mbps = megabits_per_second(stats.max_frame_bytes as f64);
    stats.valid = true;
    stats
}

/// Check measured bitrate against the DCI peak limit. One limit at every
/// resolution: DCSS 4.3.3 caps a 4K frame at the same bytes as a 24 fps 2K one.
pub fn check_bitrate_compliance(stats: &FrameBitrateStats, mxf_path: &Path) -> Vec<Note> {
    let mut notes = Vec::new();

    if !stats.valid {
        return notes;
    }

    let max_allowed = postkit::j2k::DCI_MAX_BITRATE_MBPS;

    if stats.max_bitrate_mbps > max_allowed {
        notes.push(
            Note::error(
                Code::J2kBitrateExceeded,
                format!(
                    "Peak frame bitrate {:.1} Mbps exceeds the DCI limit of {} Mbps (frame {} of {})",
                    stats.max_bitrate_mbps,
                    max_allowed as u32,
                    stats.max_frame_index,
                    stats.frame_count
                ),
            )
            .with_file(mxf_path),
        );
    } else if stats.max_bitrate_mbps > max_allowed * NEAR_LIMIT_FRACTION {
        notes.push(
            Note::warning(
                Code::J2kBitrateExceeded,
                format!(
                    "Peak frame bitrate {:.1} Mbps is near the DCI limit of {} Mbps (frame {})",
                    stats.max_bitrate_mbps, max_allowed as u32, stats.max_frame_index
                ),
            )
            .with_file(mxf_path),
        );
    }

    notes
}

/// The note for a measurement that did not happen, so a dropped peak-bitrate
/// check is never read as a clean one. `None` once the measurement succeeded, and
/// for the missing content key, which the KDM-aware caller reports itself.
pub fn skipped_measurement_note(stats: &FrameBitrateStats, mxf_path: &Path) -> Option<Note> {
    if stats.valid || stats.error == NO_CONTENT_KEY_ERROR {
        return None;
    }
    Some(
        Note::warning(
            Code::CheckSkipped,
            format!(
                "the peak bitrate check did not run: the essence could not be measured: {}",
                stats.error
            ),
        )
        .with_file(mxf_path),
    )
}

/// Report the measured peak for IMF picture essence as INFO. The DCI limit is a
/// DCP rule and no IMF specification sets a peak bitrate for App 2E picture
/// essence, so there is nothing here to pass or fail against.
pub fn report_measured_bitrate(stats: &FrameBitrateStats, mxf_path: &Path) -> Vec<Note> {
    if !stats.valid {
        return Vec::new();
    }

    vec![
        Note::info(
            Code::PictureBitrateMeasured,
            format!(
                "Peak picture bitrate {:.1} Mbps, average {:.1} Mbps over {} frames",
                stats.max_bitrate_mbps, stats.avg_bitrate_mbps, stats.frame_count
            ),
        )
        .with_file(mxf_path),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use asdcplib::jp2k::{MxfWriter, PictureDescriptor, StereoMxfWriter, StereoscopicPhase};
    use asdcplib::{LabelSet, Rational, WriterInfo};

    const FRAMES: u32 = 3;
    const EDIT_RATE: i32 = 24;

    fn writer_info() -> WriterInfo {
        WriterInfo {
            asset_uuid: *uuid::Uuid::new_v4().as_bytes(),
            context_id: *uuid::Uuid::new_v4().as_bytes(),
            label_set: LabelSet::Smpte,
            ..Default::default()
        }
    }

    fn descriptor(width: u32, height: u32) -> PictureDescriptor {
        PictureDescriptor {
            edit_rate: Rational::new(EDIT_RATE, 1),
            sample_rate: Rational::new(EDIT_RATE, 1),
            stored_width: width,
            stored_height: height,
            aspect_ratio: Rational::new(width as i32, height as i32),
            container_duration: FRAMES,
            component_count: 3,
        }
    }

    /// Write a picture MXF whose every frame is exactly `frame_bytes` long, so the
    /// peak bitrate the reader should report is `frame_bytes * 8 * 24 / 1e6`.
    /// asdcp-info reports the same rate for every file these tests write.
    fn write_mono_mxf(path: &Path, width: u32, height: u32, frame_bytes: usize) {
        let mut writer = MxfWriter::new();
        writer
            .open_write(
                path.to_str().unwrap(),
                &writer_info(),
                &descriptor(width, height),
                16384,
            )
            .unwrap();
        let frame = vec![0u8; frame_bytes];
        for _ in 0..FRAMES {
            writer.write_frame(&frame, None, None).unwrap();
        }
        writer.finalize().unwrap();
    }

    /// Same, AS-02 wrapped: what an IMP carries instead of OP-Atom.
    fn write_as02_mxf(path: &Path, width: u32, height: u32, frame_bytes: usize) {
        let mut writer = asdcplib::as02::jp2k::MxfWriter::new();
        writer
            .open_write(
                path.to_str().unwrap(),
                &writer_info(),
                &descriptor(width, height),
                16384,
            )
            .unwrap();
        let frame = vec![0u8; frame_bytes];
        for _ in 0..FRAMES {
            writer.write_frame(&frame, None, None).unwrap();
        }
        writer.finalize().unwrap();
    }

    /// Same, stereoscopic: each edit unit holds two eyes of `eye_bytes` each.
    fn write_stereo_mxf(path: &Path, eye_bytes: usize) {
        let mut descriptor = descriptor(2048, 1080);
        descriptor.sample_rate = Rational::new(EDIT_RATE * 2, 1);
        let mut writer = StereoMxfWriter::new();
        writer
            .open_write(path.to_str().unwrap(), &writer_info(), &descriptor, 16384)
            .unwrap();
        let frame = vec![0u8; eye_bytes];
        for _ in 0..FRAMES {
            writer
                .write_frame(&frame, StereoscopicPhase::Left, None, None)
                .unwrap();
            writer
                .write_frame(&frame, StereoscopicPhase::Right, None, None)
                .unwrap();
        }
        writer.finalize().unwrap();
    }

    fn measure(path: &Path) -> FrameBitrateStats {
        let stats = analyze_picture_bitrate(path, &ContentKeys::none());
        assert!(stats.valid, "analysis failed: {}", stats.error);
        stats
    }

    // 1_000_000 bytes per frame at 24 fps is 192.0 Mb/s, under the 2K limit.
    #[test]
    fn peak_under_the_2k_limit_draws_no_note() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("picture.mxf");
        write_mono_mxf(&path, 2048, 1080, 1_000_000);

        let stats = measure(&path);
        assert!((stats.max_bitrate_mbps - 192.0).abs() < 0.05);
        assert!(check_bitrate_compliance(&stats, &path).is_empty());
    }

    // 1_400_000 bytes per frame at 24 fps is 268.8 Mb/s.
    #[test]
    fn peak_over_the_2k_limit_reports_the_measured_rate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("picture.mxf");
        write_mono_mxf(&path, 2048, 1080, 1_400_000);

        let stats = measure(&path);
        assert!((stats.max_bitrate_mbps - 268.8).abs() < 0.05);

        let notes = check_bitrate_compliance(&stats, &path);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].severity, crate::Severity::Error);
        assert_eq!(notes[0].code, Code::J2kBitrateExceeded);
        assert!(
            notes[0].message.contains("268.8") && notes[0].message.contains("250"),
            "{}",
            notes[0].message
        );
    }

    // A 3D edit unit is one frame carrying two eyes, so 700_000 bytes per eye at
    // 24 fps is 268.8 Mb/s for the pair, not 134.4 per eye.
    #[test]
    fn stereoscopic_eyes_count_as_one_frame() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("picture_3d.mxf");
        write_stereo_mxf(&path, 700_000);

        let stats = measure(&path);
        assert_eq!(stats.frame_count, FRAMES);
        assert!((stats.max_bitrate_mbps - 268.8).abs() < 0.05);

        let notes = check_bitrate_compliance(&stats, &path);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].severity, crate::Severity::Error);
        assert!(notes[0].message.contains("268.8"), "{}", notes[0].message);
    }

    // 1_600_000 bytes per frame at 24 fps is 307.2 Mb/s. DCSS 4.3.3 caps a 4K
    // frame at the same bytes as a 24 fps 2K one, so the container resolution
    // buys nothing and this fails exactly as the 2K case would.
    #[test]
    fn four_k_container_uses_the_same_limit_as_2k() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("picture_4k.mxf");
        write_mono_mxf(&path, 4096, 2160, 1_600_000);

        let stats = measure(&path);
        assert!((stats.max_bitrate_mbps - 307.2).abs() < 0.05);

        let notes = check_bitrate_compliance(&stats, &path);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert_eq!(notes[0].severity, crate::Severity::Error);
        assert_eq!(notes[0].code, Code::J2kBitrateExceeded);
        assert!(
            notes[0].message.contains("307.2") && notes[0].message.contains("250"),
            "{}",
            notes[0].message
        );
    }

    // 1_100_000 bytes per frame at 24 fps is 211.2 Mb/s. An IMP wraps picture in
    // AS-02, which the OP-Atom reader cannot open.
    #[test]
    fn as02_essence_reports_the_measured_peak() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("picture_as02.mxf");
        write_as02_mxf(&path, 2048, 1080, 1_100_000);

        let stats = measure(&path);
        assert_eq!(stats.frame_count, FRAMES);
        assert_eq!(stats.max_frame_bytes, 1_100_000);
        assert!((stats.max_bitrate_mbps - 211.2).abs() < 0.05);

        let notes = report_measured_bitrate(&stats, &path);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].severity, crate::Severity::Info);
        assert_eq!(notes[0].code, Code::PictureBitrateMeasured);
        assert!(notes[0].message.contains("211.2"), "{}", notes[0].message);
    }

    // An AS-02 peak above the DCI limit is still only a measurement: the DCI
    // limit is a DCP rule, so the IMF report never fails against it.
    #[test]
    fn as02_essence_over_the_dci_limit_is_still_a_measurement() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("picture_as02_fat.mxf");
        write_as02_mxf(&path, 4096, 2160, 1_600_000);

        let stats = measure(&path);
        assert!((stats.max_bitrate_mbps - 307.2).abs() < 0.05);

        let notes = report_measured_bitrate(&stats, &path);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].severity, crate::Severity::Info);
    }

    /// Codestream shape of the encrypted fixture's frames, and the CPL its KDM is
    /// issued for, which nothing here validates the package against.
    const ENCRYPTED_DECOMPOSITION_LEVELS: u8 = 5;
    const ENCRYPTED_PAYLOAD_BYTES: usize = 1_100_000;
    const ENCRYPTED_CPL_ID: &str = "3c3c3c3c-0000-0000-0000-000000000000";

    // an AES frame is longer than the codestream inside it, so reading ciphertext
    // measures the wrapping rather than the essence.
    #[test]
    fn encrypted_as02_essence_measures_the_codestreams_not_the_ciphertext() {
        let key_id = uuid::Uuid::new_v4();
        let content_key = [0x9A; 16];
        let (kdm, recipient_key, _wrong, _certs) =
            crate::kdm::decrypt_tests::make_kdm(key_id, content_key, ENCRYPTED_CPL_ID);

        let dir = tempfile::tempdir().unwrap();
        let frames =
            vec![(ENCRYPTED_DECOMPOSITION_LEVELS, ENCRYPTED_PAYLOAD_BYTES); FRAMES as usize];
        let path = crate::j2k::frame_scan_tests::write_encrypted_as02_picture_mxf(
            dir.path(),
            "picture_as02_encrypted.mxf",
            2048,
            1080,
            &frames,
            key_id,
            content_key,
        );
        let frame_bytes = crate::j2k::frame_scan_tests::as02_frame_bytes(
            2048,
            1080,
            ENCRYPTED_DECOMPOSITION_LEVELS,
            ENCRYPTED_PAYLOAD_BYTES,
        ) as u64;

        let keys = ContentKeys::from_kdm(&kdm, &recipient_key).expect("unwrap kdm");
        let stats = analyze_picture_bitrate(&path, &keys);
        assert!(stats.valid, "measurement failed: {}", stats.error);
        assert_eq!(stats.frame_count, FRAMES);
        assert_eq!(stats.max_frame_bytes, frame_bytes);

        let without_key = analyze_picture_bitrate(&path, &ContentKeys::none());
        assert!(
            !without_key.valid,
            "ciphertext frames must not be reported as a rate, got {} Mbps peak",
            without_key.max_bitrate_mbps
        );
        assert!(report_measured_bitrate(&without_key, &path).is_empty());
        assert!(check_bitrate_compliance(&without_key, &path).is_empty());
        assert!(
            skipped_measurement_note(&without_key, &path).is_none(),
            "the KDM-aware caller reports the missing key itself"
        );
    }

    #[test]
    fn an_unmeasurable_track_says_the_peak_check_did_not_run() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("junk.mxf");
        std::fs::write(&path, b"not an MXF").unwrap();

        let stats = analyze_picture_bitrate(&path, &ContentKeys::none());
        assert!(!stats.valid);
        let note = skipped_measurement_note(&stats, &path).expect("a skipped measurement says so");
        assert_eq!(note.code, Code::CheckSkipped);
        assert_eq!(note.severity, crate::Severity::Warning);
        assert!(
            note.message.contains(&stats.error),
            "the note must carry the reason, got: {}",
            note.message
        );

        let readable = dir.path().join("picture.mxf");
        write_mono_mxf(&readable, 2048, 1080, 1024);
        let measured = measure(&readable);
        assert!(measured.valid, "measurement failed: {}", measured.error);
        assert!(skipped_measurement_note(&measured, &readable).is_none());
    }

    // 1_000_000 bytes per frame at 24 fps is 192 Mb/s, under the limit at any
    // resolution, so the 4K path is not simply failing everything.
    #[test]
    fn four_k_container_under_the_limit_is_clean() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("picture_4k_ok.mxf");
        write_mono_mxf(&path, 4096, 2160, 1_000_000);

        let stats = measure(&path);
        assert!((stats.max_bitrate_mbps - 192.0).abs() < 0.05);
        assert!(check_bitrate_compliance(&stats, &path).is_empty());
    }
}
