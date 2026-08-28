//! Writers for the AS-02 picture track files the App 2E descriptor checks read.
//! The unit tests in `app2e_picture` and the `write_app2e_fixtures` example both
//! go through here, so a fixture shipped to another repository and one a test
//! asserts on come out of the same writer. Every function panics on failure:
//! these build fixtures, they never run on a user's file.

use asdcplib::jp2k::{
    COLOR_PRIMARIES_BT709, CodestreamHeader, HdrMetadata, PictureDescriptor,
    TRANSFER_CHARACTERISTIC_BT709,
};
use asdcplib::{LabelSet, Rational, WriterInfo};
use std::path::Path;

/// The pixel layout the AS-02 writer sets for a 12-bit codestream, and the
/// 10-bit one a fixture is patched to so the layout disagrees with it.
pub const PIXEL_LAYOUT_RGB_12: [u8; 8] = [b'R', 12, b'G', 12, b'B', 12, 0, 0];
pub const PIXEL_LAYOUT_RGB_10: [u8; 8] = [b'R', 10, b'G', 10, b'B', 10, 0, 0];

const HEADER_BYTES: u32 = 16384;

/// The colour properties an App 2E track is required to carry.
pub fn bt709() -> HdrMetadata {
    HdrMetadata {
        color_primaries: Some(COLOR_PRIMARIES_BT709),
        transfer_characteristic: Some(TRANSFER_CHARACTERISTIC_BT709),
        ..Default::default()
    }
}

/// Write an AS-02 picture track file of `frames` copies of `frame`, described by
/// `codestream`. `hdr` of `None` leaves the colour properties out of the RGBA
/// descriptor, which is what `picture_colour_missing` fires on.
pub fn write_picture(
    path: &Path,
    codestream: CodestreamHeader,
    frame: &[u8],
    frames: u32,
    hdr: Option<HdrMetadata>,
) {
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
        container_duration: frames,
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
    for _ in 0..frames {
        writer.write_frame(frame, None, None).unwrap();
    }
    writer.finalize().unwrap();
}

/// A same-length overwrite keeps every KLV length in the file correct. Panics
/// unless `from` occurs exactly once.
pub fn patch_bytes(path: &Path, from: &[u8], to: &[u8]) {
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
