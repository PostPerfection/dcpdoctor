use crate::{Code, Note};
use asdcplib::as02::jp2k::RgbaDescriptor;
use asdcplib::jp2k::{HdrMetadata, ImageComponent};
use postkit::j2k::J2kProfile;
use std::path::Path;

/// SMPTE 377 RGBALayout is (component code, depth) pairs, a zero code ending
/// the list.
const PIXEL_LAYOUT_PAIR_BYTES: usize = 2;
const PIXEL_LAYOUT_TERMINATOR: u8 = 0;

/// The last byte of a PictureEssenceCoding label names the level pair, 0x00
/// being the profile family's generic label.
const LABEL_LEVEL_BYTE_INDEX: usize = 15;
const LABEL_GENERIC_LEVEL_BYTE: u8 = 0x00;

struct Descriptors {
    rsize: u16,
    components: Vec<ImageComponent>,
    rgba: RgbaDescriptor,
    hdr: HdrMetadata,
}

/// Every App 2E picture rule that the MXF essence descriptor alone settles: the
/// codestream profile, the colour metadata, the coding label and the pixel
/// layout.
pub fn check_descriptor(mxf_path: &Path) -> Vec<Note> {
    let descriptors = match read_descriptors(mxf_path) {
        Ok(descriptors) => descriptors,
        Err(reason) => {
            return vec![
                Note::warning(
                    Code::CheckSkipped,
                    format!("App 2E picture descriptor checks did not run: {reason}"),
                )
                .with_file(mxf_path),
            ];
        }
    };

    let mut notes = Vec::new();
    let profile_note = check_profile(descriptors.rsize);
    let profile_is_imf = profile_note.is_none();
    notes.extend(profile_note);
    notes.extend(check_colour(&descriptors.hdr));
    if profile_is_imf {
        notes.extend(check_coding_label(&descriptors.rgba, descriptors.rsize));
    }
    notes.extend(check_pixel_layout(
        &descriptors.rgba.pixel_layout,
        &descriptors.components,
    ));

    notes
        .into_iter()
        .map(|note| note.with_file(mxf_path))
        .collect()
}

fn read_descriptors(mxf_path: &Path) -> Result<Descriptors, String> {
    let path_str = mxf_path
        .to_str()
        .ok_or_else(|| "the path is not valid UTF-8".to_string())?;
    let mut reader = asdcplib::as02::jp2k::MxfReader::new();
    reader.open_read(path_str).map_err(|e| e.to_string())?;
    let picture = reader.picture_descriptor().map_err(|e| e.to_string())?;
    let rgba = reader.rgba_descriptor().map_err(|e| e.to_string())?;
    let hdr = reader.hdr_metadata().map_err(|e| e.to_string())?;
    reader.close().map_err(|e| e.to_string())?;
    Ok(Descriptors {
        rsize: picture.codestream.rsize,
        components: picture.codestream.components,
        rgba,
        hdr,
    })
}

fn check_profile(rsize: u16) -> Option<Note> {
    let profile = J2kProfile::from(rsize);
    if profile == J2kProfile::Imf {
        return None;
    }
    let description = if profile.is_dci_cinema() {
        "a DCI cinema profile, so its samples are X'Y'Z' rather than the RGB an App 2E track carries"
    } else {
        match profile {
            J2kProfile::None => "an unrestricted codestream declaring no profile",
            J2kProfile::Broadcast => "a broadcast contribution profile",
            _ => "not an IMF profile",
        }
    };
    Some(Note::error(
        Code::PictureNotImfProfile,
        format!("App 2E: picture Rsiz {rsize:#06x} is {description}"),
    ))
}

fn check_colour(hdr: &HdrMetadata) -> Option<Note> {
    let missing = match (
        hdr.color_primaries.is_some(),
        hdr.transfer_characteristic.is_some(),
    ) {
        (true, true) => return None,
        (false, true) => "ColorPrimaries",
        (true, false) => "TransferCharacteristic",
        (false, false) => "ColorPrimaries and TransferCharacteristic",
    };
    Some(Note::error(
        Code::PictureColourMissing,
        format!("App 2E: the picture essence descriptor carries no {missing}"),
    ))
}

fn check_coding_label(rgba: &RgbaDescriptor, rsize: u16) -> Option<Note> {
    let expected = asdcplib::jp2k::picture_essence_coding_for_rsize(rsize);
    let mut generic = expected;
    generic[LABEL_LEVEL_BYTE_INDEX] = LABEL_GENERIC_LEVEL_BYTE;

    let Some(found) = rgba.picture_essence_coding else {
        return Some(Note::error(
            Code::PictureCodingLabelMismatch,
            format!(
                "App 2E: the picture essence descriptor carries no PictureEssenceCoding, Rsiz {rsize:#06x} calls for {expected:02x?}"
            ),
        ));
    };
    if found == expected || found == generic {
        return None;
    }
    Some(Note::error(
        Code::PictureCodingLabelMismatch,
        format!(
            "App 2E: PictureEssenceCoding {found:02x?} does not match Rsiz {rsize:#06x}, which calls for {expected:02x?}"
        ),
    ))
}

fn check_pixel_layout(pixel_layout: &[u8; 16], components: &[ImageComponent]) -> Option<Note> {
    let layout_depths: Vec<u8> = pixel_layout
        .as_chunks::<PIXEL_LAYOUT_PAIR_BYTES>()
        .0
        .iter()
        .take_while(|[code, _depth]| *code != PIXEL_LAYOUT_TERMINATOR)
        .map(|[_code, depth]| *depth)
        .collect();
    let codestream_depths: Vec<u8> = components.iter().map(|c| c.bit_depth()).collect();
    if layout_depths == codestream_depths {
        return None;
    }
    Some(Note::error(
        Code::PicturePixelLayoutMismatch,
        format!(
            "App 2E: pixel layout depths {layout_depths:?} do not match the codestream component depths {codestream_depths:?}"
        ),
    ))
}

const RGB_PIXEL_FORMAT: &str = "rgb48le";

const DROPPED_WAVELET_LEVELS: u8 = 2;

const BYTES_PER_PIXEL: usize = 6;
// the 12-bit code sits in the high bits of the 16-bit word
const SAMPLE_SHIFT: u32 = 4;
const BYTES_PER_SAMPLE: usize = 2;

const BRIGHT_CODE: u16 = 2047;
// X'Y'Z' of a Rec.709 colour keeps its smallest channel above about three tenths of its largest
const SATURATED_HIGH_CODE: u16 = 3276;
const SATURATED_LOW_CODE: u16 = 256;
const RGB_NEUTRAL_SPREAD_CODES: u16 = 32;
const D65_WHITE_RATIO: (f64, f64) = (3883.0 / 3960.0, 4092.0 / 3960.0);
// the DCI white, x 0.314 y 0.351 through the ST 428-1 gamma
const DCI_WHITE_RATIO: (f64, f64) = (0.9581, 0.9822);
const WHITE_RATIO_TOLERANCE: f64 = 0.005;
const BRIGHT_PER_XYZ_NEUTRAL: u64 = 100;
const XYZ_NEUTRAL_OVER_RGB_NEUTRAL: u64 = 10;

#[derive(Debug, Default, PartialEq, Eq)]
struct SampleCounts {
    bright: u64,
    saturated: u64,
    rgb_neutral: u64,
    xyz_neutral: u64,
}

impl SampleCounts {
    fn add_frame(&mut self, frame: &[u8]) {
        for pixel in frame.as_chunks::<BYTES_PER_PIXEL>().0 {
            let mut codes = [0u16; 3];
            for (code, word) in codes
                .iter_mut()
                .zip(pixel.as_chunks::<BYTES_PER_SAMPLE>().0)
            {
                *code = u16::from_le_bytes(*word) >> SAMPLE_SHIFT;
            }
            self.add_pixel(codes);
        }
    }

    fn add_pixel(&mut self, codes: [u16; 3]) {
        let high = codes.iter().copied().max().unwrap_or(0);
        let low = codes.iter().copied().min().unwrap_or(0);
        if high > SATURATED_HIGH_CODE && low < SATURATED_LOW_CODE {
            self.saturated += 1;
        }
        if high <= BRIGHT_CODE {
            return;
        }
        self.bright += 1;
        if high - low <= RGB_NEUTRAL_SPREAD_CODES {
            self.rgb_neutral += 1;
        }
        if is_xyz_white(codes) {
            self.xyz_neutral += 1;
        }
    }
}

fn is_xyz_white(codes: [u16; 3]) -> bool {
    let [first, middle, last] = codes.map(f64::from);
    if middle == 0.0 {
        return false;
    }
    [D65_WHITE_RATIO, DCI_WHITE_RATIO].iter().any(|white| {
        (first / middle - white.0).abs() <= white.0 * WHITE_RATIO_TOLERANCE
            && (last / middle - white.1).abs() <= white.1 * WHITE_RATIO_TOLERANCE
    })
}

fn sample_note(counts: &SampleCounts, frames: u32) -> Option<Note> {
    if counts.saturated > 0 || counts.rgb_neutral > counts.xyz_neutral {
        return None;
    }
    if counts.xyz_neutral > 0
        && counts.xyz_neutral >= counts.bright / BRIGHT_PER_XYZ_NEUTRAL
        && counts.rgb_neutral * XYZ_NEUTRAL_OVER_RGB_NEUTRAL <= counts.xyz_neutral
    {
        return Some(Note::error(
            Code::PictureSamplesNotRgb,
            format!(
                "App 2E: {} of the {} bright pixels over {frames} decoded frames sit at the X'Y'Z' white ratio and {} read as neutral RGB, so the samples are X'Y'Z' rather than the RGB an App 2E track carries",
                counts.xyz_neutral, counts.bright, counts.rgb_neutral
            ),
        ));
    }
    Some(Note::warning(
        Code::CheckSkipped,
        format!(
            "App 2E: picture sample decoding decided nothing: over {frames} frames, {} bright pixels, {} saturated, {} neutral RGB and {} at the X'Y'Z' white ratio",
            counts.bright, counts.saturated, counts.rgb_neutral, counts.xyz_neutral
        ),
    ))
}

pub fn check_samples(mxf_path: &Path) -> Vec<Note> {
    let mut counts = SampleCounts::default();
    // every codestream of one track file carries the same profile
    let mut probed_format: Option<&'static str> = None;
    let walk = crate::studio::walk_sampled_frames(
        mxf_path,
        crate::j2k::PictureEssenceFamily::Imf,
        |index, frame_path| {
            let pixel_format = match probed_format {
                Some(pixel_format) => pixel_format,
                None => *probed_format.insert(
                    read_back_format(frame_path)
                        .map_err(|e| format!("frame {index} would not probe: {e}"))?,
                ),
            };
            let decoded =
                crate::studio::decode_frame(frame_path, pixel_format, DROPPED_WAVELET_LEVELS)
                    .map_err(|e| format!("frame {index} would not decode: {e}"))?;
            counts.add_frame(&decoded);
            Ok(())
        },
    );
    match walk {
        Ok(frames) => sample_note(&counts, frames)
            .into_iter()
            .map(|note| note.with_file(mxf_path))
            .collect(),
        Err(reason) => vec![
            Note::warning(
                Code::CheckSkipped,
                format!("App 2E: picture sample decoding did not run: {reason}"),
            )
            .with_file(mxf_path),
        ],
    }
}

fn read_back_format(frame_path: &Path) -> Result<&'static str, String> {
    let probed = crate::studio::probe_pixel_format(frame_path)?;
    Ok(if probed == crate::studio::XYZ_PIXEL_FORMAT {
        crate::studio::XYZ_PIXEL_FORMAT
    } else {
        RGB_PIXEL_FORMAT
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Severity;
    use crate::app2e_fixtures::{
        PIXEL_LAYOUT_RGB_10, PIXEL_LAYOUT_RGB_12, bt709, patch_bytes, patch_pixel_layout,
        write_picture as write_mxf,
    };
    use crate::codestream_fixtures::{cinema_2k, cinema_2k_bytes, imf_4k, imf_4k_bytes};
    use asdcplib::jp2k::{
        CodestreamHeader, PICTURE_ESSENCE_CODING_CINEMA_2K, PICTURE_ESSENCE_CODING_IMF_4K_LOSSY,
        PICTURE_ESSENCE_CODING_IMF_4K_LOSSY_6_3,
    };
    use std::path::PathBuf;

    const FRAMES: u32 = 2;

    fn write_picture(
        directory: &tempfile::TempDir,
        codestream: CodestreamHeader,
        frame: &[u8],
        hdr: Option<asdcplib::jp2k::HdrMetadata>,
    ) -> PathBuf {
        let path = directory.path().join("picture.mxf");
        write_mxf(&path, codestream, frame, FRAMES, hdr);
        path
    }

    fn only_note(notes: &[Note]) -> &Note {
        assert_eq!(notes.len(), 1, "expected exactly one note, got: {notes:?}");
        &notes[0]
    }

    #[test]
    fn an_imf_track_with_colour_passes_every_descriptor_check() {
        let directory = tempfile::tempdir().unwrap();
        let path = write_picture(&directory, imf_4k(), &imf_4k_bytes(), Some(bt709()));

        let notes = check_descriptor(&path);
        assert!(notes.is_empty(), "expected no notes, got: {notes:?}");
    }

    #[test]
    fn a_cinema_profile_codestream_in_an_app_2e_track_is_an_error() {
        let directory = tempfile::tempdir().unwrap();
        let path = write_picture(&directory, cinema_2k(), &cinema_2k_bytes(), Some(bt709()));

        let notes = check_descriptor(&path);
        let note = only_note(&notes);
        assert_eq!(note.code, Code::PictureNotImfProfile);
        assert_eq!(note.severity, Severity::Error);
        assert!(note.message.contains("0x0003"), "{}", note.message);
    }

    #[test]
    fn a_track_without_colour_metadata_names_both_missing_properties() {
        let directory = tempfile::tempdir().unwrap();
        let path = write_picture(&directory, imf_4k(), &imf_4k_bytes(), None);

        let notes = check_descriptor(&path);
        let note = only_note(&notes);
        assert_eq!(note.code, Code::PictureColourMissing);
        assert_eq!(note.severity, Severity::Error);
        assert!(note.message.contains("ColorPrimaries"), "{}", note.message);
        assert!(
            note.message.contains("TransferCharacteristic"),
            "{}",
            note.message
        );
    }

    #[test]
    fn a_coding_label_from_another_profile_is_an_error() {
        let directory = tempfile::tempdir().unwrap();
        let path = write_picture(&directory, imf_4k(), &imf_4k_bytes(), Some(bt709()));
        patch_bytes(
            &path,
            &PICTURE_ESSENCE_CODING_IMF_4K_LOSSY_6_3,
            &PICTURE_ESSENCE_CODING_CINEMA_2K,
        );

        let notes = check_descriptor(&path);
        let note = only_note(&notes);
        assert_eq!(note.code, Code::PictureCodingLabelMismatch);
        assert_eq!(note.severity, Severity::Error);
    }

    #[test]
    fn the_generic_family_coding_label_is_accepted() {
        let directory = tempfile::tempdir().unwrap();
        let path = write_picture(&directory, imf_4k(), &imf_4k_bytes(), Some(bt709()));
        patch_bytes(
            &path,
            &PICTURE_ESSENCE_CODING_IMF_4K_LOSSY_6_3,
            &PICTURE_ESSENCE_CODING_IMF_4K_LOSSY,
        );

        let notes = check_descriptor(&path);
        assert!(notes.is_empty(), "expected no notes, got: {notes:?}");
    }

    #[test]
    fn a_pixel_layout_depth_that_the_codestream_does_not_carry_is_an_error() {
        let directory = tempfile::tempdir().unwrap();
        let path = write_picture(&directory, imf_4k(), &imf_4k_bytes(), Some(bt709()));
        patch_pixel_layout(&path, &PIXEL_LAYOUT_RGB_12, &PIXEL_LAYOUT_RGB_10);

        let notes = check_descriptor(&path);
        let note = only_note(&notes);
        assert_eq!(note.code, Code::PicturePixelLayoutMismatch);
        assert_eq!(note.severity, Severity::Error);
        assert!(note.message.contains("10"), "{}", note.message);
        assert!(note.message.contains("12"), "{}", note.message);
    }

    #[test]
    fn a_file_that_is_not_an_mxf_says_the_checks_did_not_run() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("notes.txt");
        std::fs::write(&path, b"this is not an MXF file").unwrap();

        let notes = check_descriptor(&path);
        let note = only_note(&notes);
        assert_eq!(note.code, Code::CheckSkipped);
        assert_eq!(note.severity, Severity::Warning);
        assert_eq!(note.file.as_deref(), Some(path.as_path()));
    }

    const D65_WHITE_CODES: [u16; 3] = [3883, 3960, 4092];
    const DCI_WHITE_CODES: [u16; 3] = [2874, 3000, 2947];
    const RED_CODES: [u16; 3] = [4095, 120, 60];
    const GREY_RGB_CODES: [u16; 3] = [2048, 2048, 2048];

    const FRAMES_SAMPLED: u32 = 12;

    fn counts_of(pixels: &[([u16; 3], u64)]) -> SampleCounts {
        let mut counts = SampleCounts::default();
        for (codes, repeats) in pixels {
            for _ in 0..*repeats {
                counts.add_pixel(*codes);
            }
        }
        counts
    }

    #[test]
    fn a_red_pixel_is_saturated_and_neither_kind_of_neutral() {
        let counts = counts_of(&[(RED_CODES, 1)]);
        assert_eq!(
            counts,
            SampleCounts {
                bright: 1,
                saturated: 1,
                rgb_neutral: 0,
                xyz_neutral: 0,
            }
        );
    }

    #[test]
    fn a_grey_rgb_pixel_is_neutral_rgb_and_sits_at_no_xyz_white() {
        let counts = counts_of(&[(GREY_RGB_CODES, 1)]);
        assert_eq!(
            counts,
            SampleCounts {
                bright: 1,
                saturated: 0,
                rgb_neutral: 1,
                xyz_neutral: 0,
            }
        );
    }

    #[test]
    fn both_xyz_whites_are_xyz_neutral_and_neither_is_neutral_rgb() {
        for codes in [D65_WHITE_CODES, DCI_WHITE_CODES] {
            let counts = counts_of(&[(codes, 1)]);
            assert_eq!(
                counts,
                SampleCounts {
                    bright: 1,
                    saturated: 0,
                    rgb_neutral: 0,
                    xyz_neutral: 1,
                },
                "{codes:?}"
            );
        }
    }

    #[test]
    fn a_black_pixel_says_nothing_at_all() {
        assert_eq!(counts_of(&[([0, 0, 0], 1)]), SampleCounts::default());
    }

    #[test]
    fn one_xyz_white_in_a_hundred_bright_pixels_is_an_error() {
        let counts = counts_of(&[(D65_WHITE_CODES, 1), ([3000, 2400, 2200], 99)]);

        let note = sample_note(&counts, FRAMES_SAMPLED).expect("a note");
        assert_eq!(note.code, Code::PictureSamplesNotRgb);
        assert_eq!(note.severity, Severity::Error);
        assert!(
            note.message.contains("12 decoded frames"),
            "{}",
            note.message
        );
    }

    #[test]
    fn one_saturated_pixel_settles_the_samples_as_rgb() {
        let counts = counts_of(&[(RED_CODES, 1), (D65_WHITE_CODES, 500)]);

        assert!(sample_note(&counts, FRAMES_SAMPLED).is_none());
    }

    #[test]
    fn more_neutral_rgb_than_xyz_white_settles_the_samples_as_rgb() {
        let counts = counts_of(&[(GREY_RGB_CODES, 2), (D65_WHITE_CODES, 1)]);

        assert!(sample_note(&counts, FRAMES_SAMPLED).is_none());
    }

    #[test]
    fn a_picture_with_nothing_bright_in_it_is_undecided() {
        let counts = counts_of(&[([0, 0, 0], 100)]);

        let note = sample_note(&counts, FRAMES_SAMPLED).expect("a note");
        assert_eq!(note.code, Code::CheckSkipped);
        assert_eq!(note.severity, Severity::Warning);
        assert!(note.message.contains("decided nothing"), "{}", note.message);
    }

    #[test]
    fn too_few_xyz_whites_among_the_bright_pixels_are_undecided() {
        let counts = counts_of(&[(D65_WHITE_CODES, 1), ([3000, 2400, 2200], 500)]);

        let note = sample_note(&counts, FRAMES_SAMPLED).expect("a note");
        assert_eq!(note.code, Code::CheckSkipped);
    }
}
