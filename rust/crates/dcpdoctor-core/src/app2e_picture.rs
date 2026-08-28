//! Descriptor checks for an IMF App 2E MainImage track file. They read the MXF
//! header only, so they run whether or not the content key is held.

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Severity;
    use crate::codestream_fixtures::{cinema_2k, cinema_2k_bytes, imf_4k, imf_4k_bytes};
    use asdcplib::jp2k::{
        COLOR_PRIMARIES_BT709, CodestreamHeader, PICTURE_ESSENCE_CODING_CINEMA_2K,
        PICTURE_ESSENCE_CODING_IMF_4K_LOSSY, PICTURE_ESSENCE_CODING_IMF_4K_LOSSY_6_3,
        PictureDescriptor, TRANSFER_CHARACTERISTIC_BT709,
    };
    use asdcplib::{LabelSet, Rational, WriterInfo};
    use std::path::PathBuf;

    /// What the AS-02 writer sets for a 12-bit codestream.
    const PIXEL_LAYOUT_RGB_12: [u8; 8] = [b'R', 12, b'G', 12, b'B', 12, 0, 0];
    const PIXEL_LAYOUT_RGB_10: [u8; 8] = [b'R', 10, b'G', 10, b'B', 10, 0, 0];

    const HEADER_BYTES: u32 = 16384;
    const FRAMES: u32 = 2;

    fn bt709() -> asdcplib::jp2k::HdrMetadata {
        asdcplib::jp2k::HdrMetadata {
            color_primaries: Some(COLOR_PRIMARIES_BT709),
            transfer_characteristic: Some(TRANSFER_CHARACTERISTIC_BT709),
            ..Default::default()
        }
    }

    fn write_picture(
        directory: &tempfile::TempDir,
        codestream: CodestreamHeader,
        frame: &[u8],
        hdr: Option<asdcplib::jp2k::HdrMetadata>,
    ) -> PathBuf {
        let path = directory.path().join("picture.mxf");
        let info = WriterInfo {
            asset_uuid: *uuid::Uuid::new_v4().as_bytes(),
            context_id: *uuid::Uuid::new_v4().as_bytes(),
            label_set: LabelSet::Smpte,
            ..Default::default()
        };
        let descriptor = PictureDescriptor {
            edit_rate: Rational::new(24, 1),
            sample_rate: Rational::new(24, 1),
            stored_width: codestream.xsize,
            stored_height: codestream.ysize,
            aspect_ratio: Rational::new(
                i32::try_from(codestream.xsize).unwrap(),
                i32::try_from(codestream.ysize).unwrap(),
            ),
            container_duration: FRAMES,
            codestream,
        };
        let mut writer = asdcplib::as02::jp2k::MxfWriter::new();
        let path_str = path.to_str().unwrap();
        match hdr {
            Some(hdr) => writer
                .open_write_hdr(path_str, &info, &descriptor, &hdr, HEADER_BYTES)
                .unwrap(),
            None => writer
                .open_write(path_str, &info, &descriptor, HEADER_BYTES)
                .unwrap(),
        }
        for _ in 0..FRAMES {
            writer.write_frame(frame, None, None).unwrap();
        }
        writer.finalize().unwrap();
        path
    }

    /// A same-length overwrite keeps every KLV length in the file correct.
    fn patch_bytes(path: &Path, from: &[u8], to: &[u8]) {
        assert_eq!(from.len(), to.len(), "a patch must not change any length");
        let mut bytes = std::fs::read(path).unwrap();
        let occurrences: Vec<usize> = bytes
            .windows(from.len())
            .enumerate()
            .filter(|(_, window)| *window == from)
            .map(|(offset, _)| offset)
            .collect();
        assert_eq!(
            occurrences.len(),
            1,
            "expected one occurrence of {from:02x?}, found {}",
            occurrences.len()
        );
        bytes[occurrences[0]..occurrences[0] + to.len()].copy_from_slice(to);
        std::fs::write(path, bytes).unwrap();
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
        patch_bytes(&path, &PIXEL_LAYOUT_RGB_12, &PIXEL_LAYOUT_RGB_10);

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
}
