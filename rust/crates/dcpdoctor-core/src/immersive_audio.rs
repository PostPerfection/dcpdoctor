use std::path::Path;

use postkit::iab::qc::{
    ConformanceState, ConstraintSet, Finding, PackageKind, Severity as FindingSeverity,
    StreamChecker, StreamReport, StreamSummary,
};
use postkit::iab::{AtmosReader, FrameRate};

use crate::kdm::{ContentKeys, EssenceKey};
use crate::{Code, Note};

pub(crate) fn iab_package_kind(path: &Path) -> Option<PackageKind> {
    match asdcplib::essence_type(&path.to_string_lossy()) {
        Ok(asdcplib::EssenceType::DcDataDolbyAtmos) => Some(PackageKind::Dcp),
        Ok(asdcplib::EssenceType::As02Iab) => Some(PackageKind::Imp),
        _ => None,
    }
}

pub fn check_immersive_audio_mxf(path: &Path, keys: &ContentKeys) -> Vec<Note> {
    let Some(package) = iab_package_kind(path) else {
        return Vec::new();
    };
    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    let skipped = |reason: String| {
        vec![
            Note::warning(
                Code::CheckSkipped,
                format!("IAB checks did not run: {reason}"),
            )
            .with_file(path),
        ]
    };
    let info = match postkit::iab::atmos_writer_info(path) {
        Ok(info) => info,
        Err(error) => return skipped(error),
    };
    let content_key = match keys.resolve(&info) {
        EssenceKey::Cleartext => None,
        EssenceKey::Available { key, .. } => Some(key),
        EssenceKey::Missing { had_kdm: false, .. } => {
            return vec![
                Note::info(
                    Code::KdmRequired,
                    format!(
                        "IAB checks did not run on the encrypted immersive audio in {file_name}: no KDM and recipient key were supplied"
                    ),
                )
                .with_file(path),
            ];
        }
        missing @ EssenceKey::Missing { had_kdm: true, .. } => {
            return missing.skip_note(path).into_iter().collect();
        }
    };
    let mut reader = match AtmosReader::open(path, content_key.as_ref()) {
        Ok(reader) => reader,
        Err(error) => return skipped(error),
    };

    let mut checker = StreamChecker::new(package);
    for frame_index in 0..reader.frame_count() {
        let frame = match reader.read_frame(frame_index) {
            Ok(frame) => frame,
            Err(error) => {
                return vec![
                    Note::error(
                        Code::MxfUnreadable,
                        format!(
                            "IAB checks did not finish on {file_name}: frame {frame_index} did not read: {error}"
                        ),
                    )
                    .with_file(path),
                ];
            }
        };
        if checker.push_frame(frame).is_err() {
            break;
        }
    }
    let report = checker.finish();

    let mut notes = vec![
        Note::info(
            Code::IabStreamSummary,
            summary_message(&file_name, reader.frame_count(), &report),
        )
        .with_file(path),
    ];
    if let Some(failure) = &report.parse_failure {
        notes.push(
            Note::error(
                Code::IabParseError,
                format!(
                    "IAB stream in {file_name}: frame {} does not parse, so every constraint set is Invalid: {}",
                    failure.frame_index, failure.error
                ),
            )
            .with_file(path),
        );
    }
    for set_result in &report.sets {
        for finding in &set_result.findings {
            notes.push(violation_note(&file_name, set_result.set, finding).with_file(path));
        }
    }
    notes
}

fn summary_message(file_name: &str, track_frame_count: u32, report: &StreamReport) -> String {
    let StreamSummary {
        frame_count,
        first_frame_format,
        largest_max_rendered,
        largest_bed_count,
        largest_bed_channel_count,
        largest_object_count,
        largest_frame_bytes,
    } = &report.summary;
    let mut details = vec![format!(
        "{frame_count} of {track_frame_count} frames parsed"
    )];
    if let Some(format) = first_frame_format {
        details.push(format!(
            "{} kHz, {}-bit, {} fps, MaxRendered up to {largest_max_rendered}, at most {largest_bed_count} beds, {largest_bed_channel_count} bed channels and {largest_object_count} objects in a frame, largest frame {largest_frame_bytes} bytes",
            format.sample_rate.hertz() / 1000,
            format.bit_depth.bits(),
            frame_rate_name(format.frame_rate),
        ));
    }
    let set_states: Vec<String> = report
        .sets
        .iter()
        .map(|set_result| {
            format!(
                "{}: {}",
                constraint_set_name(set_result.set),
                state_name(set_result.state)
            )
        })
        .collect();
    format!(
        "IAB stream in {file_name}: {}. {}",
        details.join(", "),
        set_states.join(", ")
    )
}

fn violation_note(file_name: &str, set: ConstraintSet, finding: &Finding) -> Note {
    let message = format!(
        "IAB stream in {file_name}, {}: {} ({}), {} occurrences in {} frames, first at frame {}",
        constraint_set_name(set),
        finding.message,
        finding.clause,
        finding.occurrence_count,
        finding.frame_count,
        finding.first_frame_index
    );
    match finding.severity {
        FindingSeverity::Error => Note::error(Code::IabConstraintViolation, message),
        FindingSeverity::Warning => Note::warning(Code::IabConstraintViolation, message),
    }
}

fn constraint_set_name(set: ConstraintSet) -> &'static str {
    match set {
        ConstraintSet::CinemaSt2098_2 => "ST 2098-2 (cinema)",
        ConstraintSet::CinemaSt429_18 => "ST 429-18",
        ConstraintSet::Rdd57Profile1 => "RDD 57 Profile 1",
        ConstraintSet::ImfSt2098_2 => "ST 2098-2 (IMF)",
        ConstraintSet::ImfSt2067_201 => "ST 2067-201",
    }
}

fn state_name(state: ConformanceState) -> &'static str {
    match state {
        ConformanceState::Valid => "Valid",
        ConformanceState::ValidWithWarning => "Valid with warnings",
        ConformanceState::Invalid => "Invalid",
    }
}

fn frame_rate_name(frame_rate: FrameRate) -> &'static str {
    match frame_rate {
        FrameRate::Fps24 => "24",
        FrameRate::Fps25 => "25",
        FrameRate::Fps30 => "30",
        FrameRate::Fps48 => "48",
        FrameRate::Fps50 => "50",
        FrameRate::Fps60 => "60",
        FrameRate::Fps96 => "96",
        FrameRate::Fps100 => "100",
        FrameRate::Fps120 => "120",
        FrameRate::Fps24000Over1001 => "23.976",
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::Severity;
    use crate::track_fixtures::wrap_iab_frames;
    use postkit::iab::qc::check_stream;
    use postkit::mxf_wrap::{MxfEncryption, MxfStandard};
    use std::io::Read;

    pub(crate) fn validation_stream_frames(number: u32) -> Vec<Vec<u8>> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures/iab")
            .join(format!("IABValidationStream_{number}.iab.zip"));
        let file = std::fs::File::open(&path)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut stream = Vec::new();
        archive
            .by_index(0)
            .unwrap()
            .read_to_end(&mut stream)
            .unwrap();
        postkit::iab::split_frames(&stream)
            .unwrap()
            .into_iter()
            .map(<[u8]>::to_vec)
            .collect()
    }

    fn checked_track<F: AsRef<[u8]>>(
        frames: &[F],
        standard: MxfStandard,
    ) -> (tempfile::TempDir, Vec<Note>) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ATMOS.mxf");
        wrap_iab_frames(frames, &path, standard, None);
        let notes = check_immersive_audio_mxf(&path, &ContentKeys::none());
        (directory, notes)
    }

    fn the_one_note_of(notes: &[Note], code: Code) -> &Note {
        let matching: Vec<&Note> = notes.iter().filter(|note| note.code == code).collect();
        assert_eq!(matching.len(), 1, "expected one {code:?}, got: {notes:?}");
        matching[0]
    }

    fn assert_contains_all(message: &str, expected: &[&str]) {
        for part in expected {
            assert!(message.contains(part), "{part:?} is not in {message:?}");
        }
    }

    #[test]
    fn a_conforming_dcp_atmos_track_draws_its_summary_alone() {
        let (directory, notes) = checked_track(&validation_stream_frames(1), MxfStandard::AsDcp);

        assert_eq!(notes.len(), 1, "{notes:?}");
        let summary = &notes[0];
        assert_eq!(summary.code, Code::IabStreamSummary);
        assert_eq!(summary.severity, Severity::Info);
        assert_eq!(
            summary.file.as_deref(),
            Some(directory.path().join("ATMOS.mxf").as_path())
        );
        assert_contains_all(
            &summary.message,
            &[
                "IAB stream in ATMOS.mxf",
                "48 of 48 frames parsed",
                "48 kHz, 24-bit, 24 fps",
                "MaxRendered up to 15",
                "ST 2098-2 (cinema): Valid, ST 429-18: Valid, RDD 57 Profile 1: Valid",
            ],
        );
    }

    #[test]
    fn every_finding_on_a_nonconforming_dcp_track_is_one_note() {
        let frames = validation_stream_frames(129);
        let (_directory, notes) = checked_track(&frames, MxfStandard::AsDcp);

        let findings: Vec<_> = check_stream(&frames, PackageKind::Dcp)
            .sets
            .into_iter()
            .flat_map(|set_result| set_result.findings)
            .collect();
        let violations: Vec<&Note> = notes
            .iter()
            .filter(|note| note.code == Code::IabConstraintViolation)
            .collect();
        assert_eq!(violations.len(), findings.len(), "{notes:?}");
        for (note, finding) in violations.iter().zip(&findings) {
            let severity = match finding.severity {
                FindingSeverity::Error => Severity::Error,
                FindingSeverity::Warning => Severity::Warning,
            };
            assert_eq!(note.severity, severity, "{}", note.message);
            assert!(note.message.contains(finding.message), "{}", note.message);
        }

        // Stream129_cA.json: 48 issues at the root set and 720 more under ST 429-18
        let frame_rate = violations
            .iter()
            .find(|note| {
                note.message
                    .contains("ST 2098-2 (cinema): Unsupported frame rate")
            })
            .unwrap_or_else(|| panic!("no frame rate finding in {notes:?}"));
        assert_eq!(frame_rate.severity, Severity::Error);
        assert!(
            frame_rate
                .message
                .contains("48 occurrences in 48 frames, first at frame 0"),
            "{}",
            frame_rate.message
        );
        assert!(
            violations
                .iter()
                .any(|note| note.severity == Severity::Error
                    && note
                        .message
                        .contains("ST 429-18: AudioDataPCM is not allowed")
                    && note.message.contains("720 occurrences in 48 frames")),
            "{notes:?}"
        );
        assert!(
            the_one_note_of(&notes, Code::IabStreamSummary)
                .message
                .contains(
                    "ST 2098-2 (cinema): Invalid, ST 429-18: Invalid, RDD 57 Profile 1: Invalid"
                )
        );
    }

    #[test]
    fn an_imf_iab_track_is_held_to_the_imf_sets() {
        let (_directory, notes) = checked_track(&validation_stream_frames(1), MxfStandard::As02);

        let dlc = the_one_note_of(&notes, Code::IabConstraintViolation);
        assert_eq!(dlc.severity, Severity::Error);
        assert_contains_all(
            &dlc.message,
            &[
                "ST 2067-201: AudioDataDLC is not allowed",
                "720 occurrences in 48 frames",
            ],
        );
        assert!(
            the_one_note_of(&notes, Code::IabStreamSummary)
                .message
                .contains("ST 2098-2 (IMF): Valid, ST 2067-201: Invalid")
        );
    }

    fn comparable(notes: &[Note]) -> Vec<(Severity, Code, String)> {
        notes
            .iter()
            .map(|note| (note.severity, note.code, note.message.clone()))
            .collect()
    }

    // the KDM names a CPL, which the check never reads
    const ENCRYPTED_TRACK_CPL_ID: &str = "3c3c3c3c-0000-0000-0000-000000000000";

    #[test]
    fn an_encrypted_dcp_atmos_track_is_checked_only_with_its_key() {
        let frames = validation_stream_frames(129);
        let (_clear_directory, clear_notes) = checked_track(&frames, MxfStandard::AsDcp);

        let key_id = uuid::Uuid::new_v4();
        let content_key = [0x5a; 16];
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ATMOS.mxf");
        wrap_iab_frames(
            &frames,
            &path,
            MxfStandard::AsDcp,
            Some(MxfEncryption {
                content_key,
                key_id: *key_id.as_bytes(),
            }),
        );

        let (kdm, recipient_key, _wrong_key, _certificates) =
            crate::kdm::decrypt_tests::make_kdm(key_id, content_key, ENCRYPTED_TRACK_CPL_ID);
        let keys = ContentKeys::from_kdm(&kdm, &recipient_key).unwrap();
        assert_eq!(
            comparable(&check_immersive_audio_mxf(&path, &keys)),
            comparable(&clear_notes)
        );

        let (other_kdm, other_recipient_key, _other_wrong_key, _other_certificates) =
            crate::kdm::decrypt_tests::make_kdm(
                uuid::Uuid::new_v4(),
                content_key,
                ENCRYPTED_TRACK_CPL_ID,
            );
        let other_keys = ContentKeys::from_kdm(&other_kdm, &other_recipient_key).unwrap();
        let without_the_key = check_immersive_audio_mxf(&path, &other_keys);
        assert_eq!(without_the_key.len(), 1, "{without_the_key:?}");
        assert_eq!(without_the_key[0].code, Code::KdmRequired);
        assert_eq!(without_the_key[0].severity, Severity::Warning);
        assert!(
            without_the_key[0].message.contains(&key_id.to_string()),
            "{}",
            without_the_key[0].message
        );

        let without_a_kdm = check_immersive_audio_mxf(&path, &ContentKeys::none());
        assert_eq!(without_a_kdm.len(), 1, "{without_a_kdm:?}");
        assert_eq!(without_a_kdm[0].code, Code::KdmRequired);
        assert_eq!(without_a_kdm[0].severity, Severity::Info);
        assert_contains_all(
            &without_a_kdm[0].message,
            &["ATMOS.mxf", "no KDM and recipient key were supplied"],
        );
    }

    #[test]
    fn a_frame_that_does_not_parse_invalidates_every_set() {
        let (_directory, notes) = checked_track(&[b"not an IAB frame"], MxfStandard::AsDcp);

        let parse_error = the_one_note_of(&notes, Code::IabParseError);
        assert_eq!(parse_error.severity, Severity::Error);
        assert_contains_all(&parse_error.message, &["frame 0", "PreambleTag"]);
        assert_contains_all(
            &the_one_note_of(&notes, Code::IabStreamSummary).message,
            &[
                "0 of 1 frames parsed",
                "ST 2098-2 (cinema): Invalid, ST 429-18: Invalid, RDD 57 Profile 1: Invalid",
            ],
        );
    }

    // ST 429-18 Table 1 at 24 fps
    const LARGEST_DCP_FRAME_BYTES: usize = 781_250;

    #[test]
    fn a_frame_that_does_not_read_is_named() {
        let mut frames = validation_stream_frames(1)[..2].to_vec();
        frames.push(vec![0; LARGEST_DCP_FRAME_BYTES + 1]);
        let (_directory, notes) = checked_track(&frames, MxfStandard::AsDcp);

        assert_eq!(notes.len(), 1, "{notes:?}");
        assert_eq!(notes[0].code, Code::MxfUnreadable);
        assert_eq!(notes[0].severity, Severity::Error);
        assert_contains_all(
            &notes[0].message,
            &["ATMOS.mxf", "frame 2 did not read", "781250"],
        );
    }
}
