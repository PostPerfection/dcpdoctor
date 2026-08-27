//! Real JPEG 2000 codestreams for tests that build an
//! `asdcplib::jp2k::PictureDescriptor`. `CodestreamHeader::parse` reads a SIZ,
//! COD and QCD out of the bytes, so a synthetic stub will not build one.

use asdcplib::jp2k::CodestreamHeader;
use std::path::{Path, PathBuf};

const FIXTURE_DIR: &str = "../../../tests/fixtures/j2c";

/// Rsize and reference grid of `cinema2k_64x64.j2c`.
pub(crate) const CINEMA_2K_RSIZE: u16 = 0x0003;
pub(crate) const CINEMA_2K_WIDTH: u32 = 64;
pub(crate) const CINEMA_2K_HEIGHT: u32 = 64;

/// Rsize and reference grid of `imf4k_black_3840x2160.j2c`.
pub(crate) const IMF_4K_RSIZE: u16 = 0x0536;
pub(crate) const IMF_4K_WIDTH: u32 = 3840;
pub(crate) const IMF_4K_HEIGHT: u32 = 2160;

/// Both fixtures are 12-bit 4:4:4.
pub(crate) const FIXTURE_BIT_DEPTH: u8 = 12;

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(FIXTURE_DIR)
        .join(name)
}

pub(crate) fn cinema_2k_bytes() -> Vec<u8> {
    std::fs::read(fixture_path("cinema2k_64x64.j2c")).expect("read the cinema 2K fixture")
}

pub(crate) fn imf_4k_bytes() -> Vec<u8> {
    std::fs::read(fixture_path("imf4k_black_3840x2160.j2c")).expect("read the IMF 4K fixture")
}

/// DCI Cinema 2K, 64x64, 12-bit X'Y'Z'.
pub(crate) fn cinema_2k() -> CodestreamHeader {
    CodestreamHeader::parse(&cinema_2k_bytes()).expect("parse the cinema 2K fixture")
}

/// IMF 4K mainlevel 6 sublevel 3, 3840x2160, 12-bit RGB.
pub(crate) fn imf_4k() -> CodestreamHeader {
    CodestreamHeader::parse(&imf_4k_bytes()).expect("parse the IMF 4K fixture")
}

#[test]
fn the_fixtures_are_the_codestreams_the_readme_describes() {
    let cinema = cinema_2k();
    assert_eq!(cinema.rsize, CINEMA_2K_RSIZE);
    assert_eq!(
        (cinema.xsize, cinema.ysize),
        (CINEMA_2K_WIDTH, CINEMA_2K_HEIGHT)
    );
    assert_eq!(cinema.components.len(), 3);
    assert_eq!(cinema.components[0].bit_depth(), FIXTURE_BIT_DEPTH);

    let imf = imf_4k();
    assert_eq!(imf.rsize, IMF_4K_RSIZE);
    assert_eq!((imf.xsize, imf.ysize), (IMF_4K_WIDTH, IMF_4K_HEIGHT));
    assert_eq!(imf.components.len(), 3);
    assert_eq!(imf.components[0].bit_depth(), FIXTURE_BIT_DEPTH);
}
