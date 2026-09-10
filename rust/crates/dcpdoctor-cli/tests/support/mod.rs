//! Real SMPTE DCPs for the CLI tests: ffmpeg-encoded JPEG 2000 picture essence
//! and a PCM tone, wrapped by asdcplib, with a CPL, PKL and ASSETMAP whose
//! hashes and sizes match the files on disk.

// every test binary compiles the whole module and uses part of it
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use base64::Engine as _;
use sha1::{Digest, Sha1};

pub const PICTURE_FILE: &str = "picture.mxf";
pub const SOUND_FILE: &str = "sound.mxf";
pub const CPL_FILE: &str = "cpl.xml";
pub const PKL_FILE: &str = "pkl.xml";
pub const ASSETMAP_FILE: &str = "ASSETMAP.xml";

pub const EDIT_RATE: u32 = 24;
pub const PICTURE_WIDTH: u32 = 256;
pub const PICTURE_HEIGHT: u32 = 144;

/// Two seconds, so EBU R128 gating has enough 400 ms blocks to report an
/// integrated loudness rather than -inf.
pub const FRAMES: u32 = 48;

pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: u32 = 2;
pub const BYTES_PER_SAMPLE: u32 = 3;

/// -20 dBFS, well inside the R128 gate and clear of clipping.
const TONE_AMPLITUDE: f64 = 0.1;
const TONE_HZ: f64 = 1000.0;

/// Bytes asdcplib reserves for the header partition.
const HEADER_BYTES: u32 = 16_384;

/// What the picture essence shows. Bars carry detail in every 8x8 block of a
/// perceptual hash; Flat carries none, so the two fingerprint differently.
#[derive(Clone, Copy)]
pub enum Picture {
    Bars,
    Flat,
    /// A solid X', Y', Z' code triple, wrapped in a DCI cinema codestream so the
    /// picture really carries those codes rather than a conversion of them.
    Xyz([u16; 3]),
    ImfSolid([u16; 3]),
}

impl Picture {
    fn lavfi_source(self) -> Option<String> {
        match self {
            Picture::Bars => Some(format!(
                "testsrc=size={PICTURE_WIDTH}x{PICTURE_HEIGHT}:rate={EDIT_RATE}:duration=4"
            )),
            Picture::Flat => Some(format!(
                "color=c=gray:size={PICTURE_WIDTH}x{PICTURE_HEIGHT}:rate={EDIT_RATE}:duration=4"
            )),
            Picture::Xyz(_) | Picture::ImfSolid(_) => None,
        }
    }

    fn solid_codes(self) -> Option<[u16; 3]> {
        match self {
            Picture::Xyz(codes) | Picture::ImfSolid(codes) => Some(codes),
            Picture::Bars | Picture::Flat => None,
        }
    }

    fn rsiz(self) -> Option<u16> {
        match self {
            Picture::Xyz(_) => Some(CINEMA_2K_RSIZ),
            Picture::ImfSolid(_) => Some(IMF_2K_RSIZ),
            Picture::Bars | Picture::Flat => None,
        }
    }
}

/// SIZ carries Rsiz two bytes in, and 3 is the DCI Cinema 2K profile ffmpeg
/// reads a codestream as X'Y'Z' on the strength of.
const CINEMA_2K_RSIZ: u16 = 3;
const IMF_2K_RSIZ: u16 = 0x0436;
const RSIZ_OFFSET: usize = 6;

/// One frame of gbrp12le, whose planes ffmpeg's JPEG 2000 encoder writes as
/// codestream components in R, G, B order.
fn solid_gbrp12le_frame(xyz: [u16; 3]) -> Vec<u8> {
    let pixels = (PICTURE_WIDTH * PICTURE_HEIGHT) as usize;
    let [x, y, z] = xyz;
    let mut frame = Vec::with_capacity(pixels * 6);
    for code in [y, z, x] {
        frame.extend(std::iter::repeat_n(code.to_le_bytes(), pixels).flatten());
    }
    frame
}

fn set_profile(codestream: &mut [u8], rsiz: u16) {
    assert_eq!(&codestream[0..2], b"\xff\x4f", "no SOC marker");
    assert_eq!(&codestream[2..4], b"\xff\x51", "no SIZ marker");
    codestream[RSIZ_OFFSET..RSIZ_OFFSET + 2].copy_from_slice(&rsiz.to_be_bytes());
}

/// A package to write: the picture content, the JPEG 2000 quantizer step ffmpeg
/// encodes it at (a second value re-encodes the same frames at another
/// bitrate), the CPL title and the reel durations in frames.
pub struct PackageSpec {
    pub title: String,
    pub picture: Picture,
    pub picture_quality: u32,
    pub reel_durations: Vec<i64>,
}

impl Default for PackageSpec {
    fn default() -> Self {
        Self {
            title: "Fixture DCP".into(),
            picture: Picture::Bars,
            picture_quality: 2,
            reel_durations: vec![FRAMES as i64],
        }
    }
}

/// Identifiers of one package. A second package written with different
/// identifiers still diffs as the same assets, so every package shares these.
const CPL_ID: &str = "5c3d4b1b-19cc-43de-896b-b22b3f39b766";
const PKL_ID: &str = "dfb16c73-27c3-4afa-915f-1d4dc616cd7a";
const ASSETMAP_ID: &str = "01cfce20-0df4-4393-87a6-bd8b0251e5bd";
pub const PICTURE_ID: &str = "148971a4-abc6-44ae-bf59-34026d0faf17";
pub const SOUND_ID: &str = "3878f6e9-eeec-4d0f-b2b3-4fb96b7759e1";

/// Write a complete SMPTE DCP into `dir`.
pub fn write_package(dir: &Path, spec: &PackageSpec) {
    let frames = encode_j2c_frames(dir, spec.picture, spec.picture_quality);
    write_picture_mxf(&dir.join(PICTURE_FILE), &frames);
    write_sound_mxf(&dir.join(SOUND_FILE));
    write_xml(dir, spec);
}

pub const IMP_FRAMES: u32 = 12;

const IMP_CPL_ID: &str = "1a1a1a1a-1111-1111-1111-111111111111";
const IMP_PKL_ID: &str = "dddddddd-1111-1111-1111-111111111111";
const IMP_ASSETMAP_ID: &str = "cccccccc-1111-1111-1111-111111111111";

const APP_2E_NAMESPACE: &str = "http://www.smpte-ra.org/ns/2067-21/2021";

pub fn imp_picture_file() -> String {
    format!("{PICTURE_ID}.mxf")
}

pub fn imp_sound_file() -> String {
    format!("{SOUND_ID}.mxf")
}

pub fn write_app2e_imp(dir: &Path, picture: Picture) {
    let frames = encode_j2c_frames(dir, picture, 2);
    let codestream =
        asdcplib::jp2k::CodestreamHeader::parse(&frames[0]).expect("parse the ffmpeg codestream");
    dcpdoctor_core::app2e_fixtures::write_picture(
        &dir.join(imp_picture_file()),
        codestream,
        &frames[0],
        IMP_FRAMES,
        Some(dcpdoctor_core::app2e_fixtures::bt709()),
    );
    write_as02_sound_mxf(&dir.join(imp_sound_file()));

    let picture_file = imp_picture_file();
    let sound_file = imp_sound_file();
    std::fs::write(
        dir.join("ASSETMAP.xml"),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:{IMP_ASSETMAP_ID}</Id>
  <Creator>dcpdoctor tests</Creator>
  <VolumeCount>1</VolumeCount>
  <IssueDate>2026-01-01T00:00:00+00:00</IssueDate>
  <Issuer>dcpdoctor tests</Issuer>
  <AssetList>
    <Asset><Id>urn:uuid:{IMP_PKL_ID}</Id><PackingList>true</PackingList><ChunkList><Chunk><Path>PKL.xml</Path></Chunk></ChunkList></Asset>
    <Asset><Id>urn:uuid:{IMP_CPL_ID}</Id><ChunkList><Chunk><Path>CPL.xml</Path></Chunk></ChunkList></Asset>
    <Asset><Id>urn:uuid:{PICTURE_ID}</Id><ChunkList><Chunk><Path>{picture_file}</Path></Chunk></ChunkList></Asset>
    <Asset><Id>urn:uuid:{SOUND_ID}</Id><ChunkList><Chunk><Path>{sound_file}</Path></Chunk></ChunkList></Asset>
  </AssetList>
</AssetMap>"#
        ),
    )
    .unwrap();

    std::fs::write(
        dir.join("PKL.xml"),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<PackingList xmlns="http://www.smpte-ra.org/schemas/2067-2/2016/PKL">
  <Id>urn:uuid:{IMP_PKL_ID}</Id>
  <AssetList>
    <Asset><Id>urn:uuid:{IMP_CPL_ID}</Id><Type>text/xml</Type></Asset>
    <Asset><Id>urn:uuid:{PICTURE_ID}</Id><Type>application/mxf</Type></Asset>
    <Asset><Id>urn:uuid:{SOUND_ID}</Id><Type>application/mxf</Type></Asset>
  </AssetList>
</PackingList>"#
        ),
    )
    .unwrap();

    std::fs::write(
        dir.join("CPL.xml"),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/2067-3/2016"
                     xmlns:cc="http://www.smpte-ra.org/schemas/2067-2/2016"
                     xmlns:app="{APP_2E_NAMESPACE}">
  <Id>urn:uuid:{IMP_CPL_ID}</Id>
  <ContentTitle>App 2E sample fixture</ContentTitle>
  <EditRate>{EDIT_RATE} 1</EditRate>
  <SegmentList>
    <Segment>
      <MainImageSequence>
        <Id>urn:uuid:eeeeeeee-1111-1111-1111-111111111111</Id>
        <ResourceList><Resource>
          <Id>urn:uuid:11110000-1111-1111-1111-111111111111</Id>
          <TrackFileId>urn:uuid:{PICTURE_ID}</TrackFileId>
          <EditRate>{EDIT_RATE} 1</EditRate><IntrinsicDuration>{IMP_FRAMES}</IntrinsicDuration>
          <EntryPoint>0</EntryPoint><SourceDuration>{IMP_FRAMES}</SourceDuration>
        </Resource></ResourceList>
      </MainImageSequence>
      <MainAudioSequence>
        <Id>urn:uuid:eeeeeeee-2222-2222-2222-222222222222</Id>
        <ResourceList><Resource>
          <Id>urn:uuid:22220000-1111-1111-1111-111111111111</Id>
          <TrackFileId>urn:uuid:{SOUND_ID}</TrackFileId>
          <EditRate>{EDIT_RATE} 1</EditRate><IntrinsicDuration>{IMP_FRAMES}</IntrinsicDuration>
          <EntryPoint>0</EntryPoint><SourceDuration>{IMP_FRAMES}</SourceDuration>
        </Resource></ResourceList>
      </MainAudioSequence>
    </Segment>
  </SegmentList>
</CompositionPlaylist>"#
        ),
    )
    .unwrap();
}

/// Copy a written package, so the two differ only where a test changes them.
/// Wrapping the same essence twice would not do: asdcplib stamps a fresh
/// context id and modification date into every MXF header.
pub fn copy_package(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
    }
}

/// Flip one byte in the middle of a file, past any header.
pub fn corrupt_one_byte(path: &Path) {
    let mut bytes = std::fs::read(path).unwrap();
    let at = bytes.len() / 2;
    bytes[at] ^= 0xff;
    std::fs::write(path, bytes).unwrap();
}

/// Rewrite the CPL, PKL and ASSETMAP against whatever essence is on disk. Used
/// after a test mutates an asset so only the field under test differs.
pub fn write_xml(dir: &Path, spec: &PackageSpec) {
    std::fs::write(dir.join(CPL_FILE), cpl_xml(spec)).unwrap();
    std::fs::write(dir.join(PKL_FILE), pkl_xml(dir, &spec.title)).unwrap();
    std::fs::write(dir.join(ASSETMAP_FILE), assetmap_xml()).unwrap();
}

/// One JPEG 2000 codestream per frame, encoded by ffmpeg so the essence really
/// decodes. `quality` is ffmpeg's `-q:v`, the knob that moves the bitrate.
fn encode_j2c_frames(dir: &Path, picture: Picture, quality: u32) -> Vec<Vec<u8>> {
    let scratch = dir.join("j2c");
    std::fs::create_dir_all(&scratch).unwrap();
    let pattern = scratch.join("frame%04d.j2k");
    let raw = dir.join("frames.raw");
    let mut ffmpeg = Command::new("ffmpeg");
    ffmpeg.args(["-v", "error", "-y"]);
    match picture.lavfi_source() {
        Some(source) => {
            ffmpeg.args(["-f", "lavfi", "-i"]).arg(source);
        }
        None => {
            let codes = picture
                .solid_codes()
                .expect("only a solid picture has no lavfi source");
            let frame = solid_gbrp12le_frame(codes);
            std::fs::write(&raw, frame.repeat(FRAMES as usize)).unwrap();
            ffmpeg
                .args([
                    "-f",
                    "rawvideo",
                    "-pix_fmt",
                    "gbrp12le",
                    "-s",
                    &format!("{PICTURE_WIDTH}x{PICTURE_HEIGHT}"),
                    "-framerate",
                    &EDIT_RATE.to_string(),
                    "-i",
                ])
                .arg(&raw);
        }
    }
    let status = ffmpeg
        .args([
            "-c:v",
            "jpeg2000",
            // the 5/3 wavelet, so a solid colour comes back as the codes it went in as
            "-pred",
            "1",
            // the encoder writes a JP2 container by default, and asdcplib wraps
            // a bare codestream
            "-format",
            "j2k",
            "-pix_fmt",
            "gbrp12le",
            "-q:v",
            &quality.to_string(),
            "-frames:v",
            &FRAMES.to_string(),
            "-f",
            "image2",
        ])
        .arg(&pattern)
        .status()
        .expect("run ffmpeg");
    assert!(status.success(), "ffmpeg could not encode the fixture j2c");

    let mut paths: Vec<PathBuf> = std::fs::read_dir(&scratch)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    paths.sort();
    assert_eq!(paths.len(), FRAMES as usize, "ffmpeg wrote {paths:?}");
    let mut frames: Vec<Vec<u8>> = paths.iter().map(|p| std::fs::read(p).unwrap()).collect();
    if let Some(rsiz) = picture.rsiz() {
        let _ = std::fs::remove_file(&raw);
        for frame in &mut frames {
            set_profile(frame, rsiz);
        }
    }
    std::fs::remove_dir_all(&scratch).unwrap();
    frames
}

fn write_picture_mxf(path: &Path, frames: &[Vec<u8>]) {
    use asdcplib::jp2k::{CodestreamHeader, MxfWriter, PictureDescriptor};
    use asdcplib::{LabelSet, Rational, WriterInfo};

    let info = WriterInfo {
        asset_uuid: *uuid(PICTURE_ID).as_bytes(),
        label_set: LabelSet::Smpte,
        ..Default::default()
    };
    let descriptor = PictureDescriptor {
        edit_rate: Rational::new(EDIT_RATE as i32, 1),
        sample_rate: Rational::new(EDIT_RATE as i32, 1),
        stored_width: PICTURE_WIDTH,
        stored_height: PICTURE_HEIGHT,
        aspect_ratio: Rational::new(PICTURE_WIDTH as i32, PICTURE_HEIGHT as i32),
        container_duration: frames.len() as u32,
        codestream: CodestreamHeader::parse(&frames[0]).expect("parse the ffmpeg codestream"),
    };
    let mut writer = MxfWriter::new();
    writer
        .open_write(path.to_str().unwrap(), &info, &descriptor, HEADER_BYTES)
        .unwrap();
    for frame in frames {
        writer.write_frame(frame, None, None).unwrap();
    }
    writer.finalize().unwrap();
}

fn write_sound_mxf(path: &Path) {
    use asdcplib::pcm::{AudioDescriptor, ChannelFormat, MxfWriter};
    use asdcplib::{LabelSet, Rational, WriterInfo};

    let block_align = CHANNELS * BYTES_PER_SAMPLE;
    let descriptor = AudioDescriptor {
        edit_rate: Rational::new(EDIT_RATE as i32, 1),
        audio_sampling_rate: Rational::new(SAMPLE_RATE as i32, 1),
        locked: true,
        channel_count: CHANNELS,
        quantization_bits: BYTES_PER_SAMPLE * 8,
        block_align,
        avg_bps: SAMPLE_RATE * block_align,
        linked_track_id: 0,
        container_duration: FRAMES,
        channel_format: ChannelFormat::Cfg4,
    };
    let info = WriterInfo {
        asset_uuid: *uuid(SOUND_ID).as_bytes(),
        label_set: LabelSet::Smpte,
        ..Default::default()
    };
    let mut writer = MxfWriter::new();
    writer
        .open_write(path.to_str().unwrap(), &info, &descriptor, HEADER_BYTES)
        .unwrap();
    for edit_unit in 0..FRAMES {
        writer
            .write_frame(&tone_frame(edit_unit), None, None)
            .unwrap();
    }
    writer.finalize().unwrap();
}

// the same tone, in the AS-02 clip wrapping an IMP sound track file uses
fn write_as02_sound_mxf(path: &Path) {
    use asdcplib::pcm::{AudioDescriptor, ChannelFormat};
    use asdcplib::{LabelSet, Rational, WriterInfo};

    let block_align = CHANNELS * BYTES_PER_SAMPLE;
    let descriptor = AudioDescriptor {
        edit_rate: Rational::new(EDIT_RATE as i32, 1),
        audio_sampling_rate: Rational::new(SAMPLE_RATE as i32, 1),
        locked: true,
        channel_count: CHANNELS,
        quantization_bits: BYTES_PER_SAMPLE * 8,
        block_align,
        avg_bps: SAMPLE_RATE * block_align,
        linked_track_id: 0,
        container_duration: IMP_FRAMES,
        channel_format: ChannelFormat::None,
    };
    let info = WriterInfo {
        asset_uuid: *uuid(SOUND_ID).as_bytes(),
        label_set: LabelSet::Smpte,
        ..Default::default()
    };
    let mut writer = asdcplib::as02::pcm::MxfWriter::new();
    writer
        .open_write(path.to_str().unwrap(), &info, &descriptor, HEADER_BYTES)
        .unwrap();
    for edit_unit in 0..IMP_FRAMES {
        writer
            .write_frame(&tone_frame(edit_unit), None, None)
            .unwrap();
    }
    writer.finalize().unwrap();
}

fn tone_frame(edit_unit: u32) -> Vec<u8> {
    let block_align = CHANNELS * BYTES_PER_SAMPLE;
    let samples_per_edit_unit = SAMPLE_RATE / EDIT_RATE;
    let mut frame = Vec::with_capacity((samples_per_edit_unit * block_align) as usize);
    for offset in 0..samples_per_edit_unit {
        let sample_index = edit_unit * samples_per_edit_unit + offset;
        let value = TONE_AMPLITUDE
            * (std::f64::consts::TAU * TONE_HZ * sample_index as f64 / SAMPLE_RATE as f64).sin();
        let quantized = (value * 8_388_607.0) as i32;
        for _ in 0..CHANNELS {
            frame.extend_from_slice(&quantized.to_le_bytes()[..BYTES_PER_SAMPLE as usize]);
        }
    }
    frame
}

fn uuid(text: &str) -> uuid::Uuid {
    text.parse().unwrap()
}

fn sha1_base64(path: &Path) -> String {
    let mut hasher = Sha1::new();
    hasher.update(std::fs::read(path).unwrap());
    base64::engine::general_purpose::STANDARD.encode(hasher.finalize())
}

fn cpl_xml(spec: &PackageSpec) -> String {
    let mut reels = String::new();
    let mut entry_point = 0i64;
    for (index, duration) in spec.reel_durations.iter().enumerate() {
        reels.push_str(&format!(
            r#"
    <Reel>
      <Id>urn:uuid:35e485f4-d779-4391-a9a3-21adc074bb{index:02x}</Id>
      <AssetList>
        <MainPicture>
          <Id>urn:uuid:{PICTURE_ID}</Id>
          <EditRate>{EDIT_RATE} 1</EditRate>
          <IntrinsicDuration>{FRAMES}</IntrinsicDuration>
          <Duration>{duration}</Duration>
          <EntryPoint>{entry_point}</EntryPoint>
          <FrameRate>{EDIT_RATE} 1</FrameRate>
          <ScreenAspectRatio>{PICTURE_WIDTH} {PICTURE_HEIGHT}</ScreenAspectRatio>
        </MainPicture>
        <MainSound>
          <Id>urn:uuid:{SOUND_ID}</Id>
          <EditRate>{EDIT_RATE} 1</EditRate>
          <IntrinsicDuration>{FRAMES}</IntrinsicDuration>
          <Duration>{duration}</Duration>
          <EntryPoint>{entry_point}</EntryPoint>
        </MainSound>
      </AssetList>
    </Reel>"#
        ));
        entry_point += duration;
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL">
  <Id>urn:uuid:{CPL_ID}</Id>
  <AnnotationText>{title}</AnnotationText>
  <IssueDate>2026-01-01T00:00:00+00:00</IssueDate>
  <Issuer>dcpdoctor tests</Issuer>
  <Creator>dcpdoctor tests</Creator>
  <ContentTitleText>{title}</ContentTitleText>
  <ContentKind>test</ContentKind>
  <ContentVersion>
    <Id>urn:uuid:6dbd2c3a-0a41-4f1f-8b2e-1c4d5e6f7a80</Id>
    <LabelText>{title}</LabelText>
  </ContentVersion>
  <ReelList>{reels}
  </ReelList>
</CompositionPlaylist>
"#,
        title = spec.title
    )
}

fn pkl_xml(dir: &Path, title: &str) -> String {
    let mut assets = String::new();
    for (id, file, mime) in [
        (PICTURE_ID, PICTURE_FILE, "application/mxf"),
        (SOUND_ID, SOUND_FILE, "application/mxf"),
        (CPL_ID, CPL_FILE, "text/xml"),
    ] {
        let path = dir.join(file);
        assets.push_str(&format!(
            r#"
    <Asset>
      <Id>urn:uuid:{id}</Id>
      <Hash>{hash}</Hash>
      <Size>{size}</Size>
      <Type>{mime}</Type>
      <OriginalFileName>{file}</OriginalFileName>
    </Asset>"#,
            hash = sha1_base64(&path),
            size = std::fs::metadata(&path).unwrap().len(),
        ));
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<PackingList xmlns="http://www.smpte-ra.org/schemas/429-8/2007/PKL">
  <Id>urn:uuid:{PKL_ID}</Id>
  <AnnotationText>{title}</AnnotationText>
  <IssueDate>2026-01-01T00:00:00+00:00</IssueDate>
  <Issuer>dcpdoctor tests</Issuer>
  <Creator>dcpdoctor tests</Creator>
  <AssetList>{assets}
  </AssetList>
</PackingList>
"#
    )
}

fn assetmap_xml() -> String {
    let mut assets = format!(
        r#"
    <Asset>
      <Id>urn:uuid:{PKL_ID}</Id>
      <PackingList>true</PackingList>
      <ChunkList><Chunk><Path>{PKL_FILE}</Path></Chunk></ChunkList>
    </Asset>"#
    );
    for (id, file) in [
        (CPL_ID, CPL_FILE),
        (PICTURE_ID, PICTURE_FILE),
        (SOUND_ID, SOUND_FILE),
    ] {
        assets.push_str(&format!(
            r#"
    <Asset>
      <Id>urn:uuid:{id}</Id>
      <ChunkList><Chunk><Path>{file}</Path></Chunk></ChunkList>
    </Asset>"#
        ));
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:{ASSETMAP_ID}</Id>
  <Creator>dcpdoctor tests</Creator>
  <VolumeCount>1</VolumeCount>
  <IssueDate>2026-01-01T00:00:00+00:00</IssueDate>
  <Issuer>dcpdoctor tests</Issuer>
  <AssetList>{assets}
  </AssetList>
</AssetMap>
"#
    )
}
