//! SMPTE DCP packages built from real essence, for the checks that read a
//! package rather than its XML alone. The picture size and the channel count
//! live only in the MXF descriptors, so a stub file cannot stand in.

use std::path::Path;

use asdcplib::jp2k::{
    CodestreamHeader, MxfWriter, PictureDescriptor, StereoMxfWriter, StereoscopicPhase,
};
use asdcplib::pcm::{AudioDescriptor, ChannelFormat};
use asdcplib::{LabelSet, Rational, WriterInfo};

pub const CPL_ID: &str = "5c3d4b1b-19cc-43de-896b-b22b3f39b766";
pub const PKL_ID: &str = "dfb16c73-27c3-4afa-915f-1d4dc616cd7a";
pub const PICTURE_ID: &str = "148971a4-abc6-44ae-bf59-34026d0faf17";
pub const SOUND_ID: &str = "3878f6e9-eeec-4d0f-b2b3-4fb96b7759e1";

pub const PICTURE_FILE: &str = "picture.mxf";
pub const SOUND_FILE: &str = "sound.mxf";
pub const CPL_FILE: &str = "cpl.xml";
pub const PKL_FILE: &str = "pkl.xml";

/// A valid ISDCF content title: 2K, 24 fps, 5.1 with HI and VI, SMPTE OV.
pub const ISDCF_TITLE: &str =
    "TestFilm_FTR_F-178_EN-XX_US-13_51-HI-VI_2K_STU_20260101_FAC_SMPTE_OV";

const FRAMES: u32 = 2;
const SOUND_SAMPLE_RATE: u32 = 48_000;
const BYTES_PER_SAMPLE: u32 = 3;

/// What the package declares and what its essence carries.
pub struct DcpSpec {
    pub content_title: String,
    pub width: u32,
    pub height: u32,
    pub edit_rate: (u32, u32),
    pub channels: u32,
    pub stereo3d: bool,
}

impl Default for DcpSpec {
    /// The package every profile accepts: 2K, 24 fps, 8 channels, 2D.
    fn default() -> Self {
        Self {
            content_title: ISDCF_TITLE.into(),
            width: 2048,
            height: 1080,
            edit_rate: (24, 1),
            channels: 8,
            stereo3d: false,
        }
    }
}

/// Write a complete SMPTE DCP: picture and sound essence, CPL, PKL with the
/// files' real sizes and SHA-1s, ASSETMAP and VOLINDEX.
pub fn write_dcp(dir: &Path, spec: &DcpSpec) {
    write_picture(&dir.join(PICTURE_FILE), spec);
    write_sound(&dir.join(SOUND_FILE), spec);
    write_cpl(dir, spec);
    write_packing_list(dir);
    write_assetmap(dir);
    std::fs::write(
        dir.join("VOLINDEX.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<VolumeIndex xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM"><Index>1</Index></VolumeIndex>"#,
    )
    .unwrap();
}

fn writer_info(asset_id: &str) -> WriterInfo {
    WriterInfo {
        asset_uuid: uuid_bytes(asset_id),
        context_id: uuid_bytes(CPL_ID),
        label_set: LabelSet::Smpte,
        ..Default::default()
    }
}

/// The DCI cinema 2K codestream committed under tests/fixtures/j2c. Its own
/// grid is 64x64; the descriptor carries the stored size the package declares.
fn codestream() -> CodestreamHeader {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/j2c/cinema2k_64x64.j2c");
    let bytes = std::fs::read(&path).expect("the cinema 2K codestream fixture is committed");
    CodestreamHeader::parse(&bytes).expect("parse the cinema 2K codestream fixture")
}

fn write_picture(path: &Path, spec: &DcpSpec) {
    let (numerator, denominator) = (spec.edit_rate.0 as i32, spec.edit_rate.1 as i32);
    let descriptor = PictureDescriptor {
        edit_rate: Rational::new(numerator, denominator),
        // ST 429-10: a stereoscopic track samples both eyes per edit unit
        sample_rate: Rational::new(
            if spec.stereo3d {
                numerator * 2
            } else {
                numerator
            },
            denominator,
        ),
        stored_width: spec.width,
        stored_height: spec.height,
        aspect_ratio: Rational::new(spec.width as i32, spec.height as i32),
        container_duration: FRAMES,
        codestream: codestream(),
    };
    let info = writer_info(PICTURE_ID);
    let frame = [0xFF, 0x4F, 0xFF, 0x93, 0, 0, 0, 0];

    if spec.stereo3d {
        let mut writer = StereoMxfWriter::new();
        writer
            .open_write(path.to_str().unwrap(), &info, &descriptor, 16_384)
            .unwrap();
        for _ in 0..FRAMES {
            for eye in [StereoscopicPhase::Left, StereoscopicPhase::Right] {
                writer.write_frame(&frame, eye, None, None).unwrap();
            }
        }
        writer.finalize().unwrap();
        return;
    }

    let mut writer = MxfWriter::new();
    writer
        .open_write(path.to_str().unwrap(), &info, &descriptor, 16_384)
        .unwrap();
    for _ in 0..FRAMES {
        writer.write_frame(&frame, None, None).unwrap();
    }
    writer.finalize().unwrap();
}

fn write_sound(path: &Path, spec: &DcpSpec) {
    let block_align = spec.channels * BYTES_PER_SAMPLE;
    let samples_per_frame = SOUND_SAMPLE_RATE * spec.edit_rate.1 / spec.edit_rate.0;
    let descriptor = AudioDescriptor {
        edit_rate: Rational::new(spec.edit_rate.0 as i32, spec.edit_rate.1 as i32),
        audio_sampling_rate: Rational::new(SOUND_SAMPLE_RATE as i32, 1),
        locked: true,
        channel_count: spec.channels,
        quantization_bits: BYTES_PER_SAMPLE * 8,
        block_align,
        avg_bps: SOUND_SAMPLE_RATE * block_align,
        linked_track_id: 0,
        container_duration: FRAMES,
        channel_format: ChannelFormat::None,
    };
    let mut writer = asdcplib::pcm::MxfWriter::new();
    writer
        .open_write(
            path.to_str().unwrap(),
            &writer_info(SOUND_ID),
            &descriptor,
            16_384,
        )
        .unwrap();
    let frame = vec![0u8; (block_align * samples_per_frame) as usize];
    for _ in 0..FRAMES {
        writer.write_frame(&frame, None, None).unwrap();
    }
    writer.finalize().unwrap();
}

fn write_cpl(dir: &Path, spec: &DcpSpec) {
    let (numerator, denominator) = spec.edit_rate;
    let edit_rate = format!("{numerator} {denominator}");
    let picture_element = if spec.stereo3d {
        "MainStereoscopicPicture"
    } else {
        "MainPicture"
    };
    let frame_rate = if spec.stereo3d {
        format!("{} {denominator}", numerator * 2)
    } else {
        edit_rate.clone()
    };
    let title = &spec.content_title;

    std::fs::write(
        dir.join(CPL_FILE),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL">
  <Id>urn:uuid:{CPL_ID}</Id>
  <AnnotationText>{title}</AnnotationText>
  <IssueDate>2026-01-01T00:00:00+00:00</IssueDate>
  <Issuer>dcpdoctor tests</Issuer>
  <Creator>dcpdoctor tests</Creator>
  <ContentTitleText>{title}</ContentTitleText>
  <ContentKind>feature</ContentKind>
  <ContentVersion>
    <Id>urn:uuid:0d1c4a1a-0f0d-4a53-8f0a-5a5f2a4d9c11</Id>
    <LabelText>{title}</LabelText>
  </ContentVersion>
  <ReelList>
    <Reel>
      <Id>urn:uuid:35e485f4-d779-4391-a9a3-21adc074bbc4</Id>
      <AssetList>
        <{picture_element}>
          <Id>urn:uuid:{PICTURE_ID}</Id>
          <EditRate>{edit_rate}</EditRate>
          <IntrinsicDuration>{FRAMES}</IntrinsicDuration>
          <Duration>{FRAMES}</Duration>
          <EntryPoint>0</EntryPoint>
          <FrameRate>{frame_rate}</FrameRate>
          <ScreenAspectRatio>{width} {height}</ScreenAspectRatio>
        </{picture_element}>
        <MainSound>
          <Id>urn:uuid:{SOUND_ID}</Id>
          <EditRate>{edit_rate}</EditRate>
          <IntrinsicDuration>{FRAMES}</IntrinsicDuration>
          <Duration>{FRAMES}</Duration>
          <EntryPoint>0</EntryPoint>
        </MainSound>
      </AssetList>
    </Reel>
  </ReelList>
</CompositionPlaylist>"#,
            width = spec.width,
            height = spec.height,
        ),
    )
    .unwrap();
}

fn write_packing_list(dir: &Path) {
    let mut assets = String::new();
    for (id, file) in [
        (PICTURE_ID, PICTURE_FILE),
        (SOUND_ID, SOUND_FILE),
        (CPL_ID, CPL_FILE),
    ] {
        let path = dir.join(file);
        let mime = if file.ends_with(".mxf") {
            "application/mxf"
        } else {
            "text/xml"
        };
        assets.push_str(&format!(
            r#"
    <Asset>
      <Id>urn:uuid:{id}</Id>
      <Hash>{hash}</Hash>
      <Size>{size}</Size>
      <Type>{mime}</Type>
      <OriginalFileName>{file}</OriginalFileName>
    </Asset>"#,
            hash = dcpdoctor_core::hash::sha1_base64(&path).unwrap(),
            size = std::fs::metadata(&path).unwrap().len(),
        ));
    }

    std::fs::write(
        dir.join(PKL_FILE),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<PackingList xmlns="http://www.smpte-ra.org/schemas/429-8/2007/PKL">
  <Id>urn:uuid:{PKL_ID}</Id>
  <IssueDate>2026-01-01T00:00:00+00:00</IssueDate>
  <Issuer>dcpdoctor tests</Issuer>
  <Creator>dcpdoctor tests</Creator>
  <AssetList>{assets}
  </AssetList>
</PackingList>"#
        ),
    )
    .unwrap();
}

fn write_assetmap(dir: &Path) {
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
      <ChunkList><Chunk><Path>{file}</Path><Length>{length}</Length></Chunk></ChunkList>
    </Asset>"#,
            length = std::fs::metadata(dir.join(file)).unwrap().len(),
        ));
    }

    std::fs::write(
        dir.join("ASSETMAP.xml"),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:01cfce20-0df4-4393-87a6-bd8b0251e5bd</Id>
  <Creator>dcpdoctor tests</Creator>
  <VolumeCount>1</VolumeCount>
  <IssueDate>2026-01-01T00:00:00+00:00</IssueDate>
  <Issuer>dcpdoctor tests</Issuer>
  <AssetList>{assets}
  </AssetList>
</AssetMap>"#
        ),
    )
    .unwrap();
}

/// The 16 bytes of a hyphenated UUID string.
fn uuid_bytes(uuid: &str) -> [u8; 16] {
    let hex: String = uuid.chars().filter(|c| *c != '-').collect();
    let mut bytes = [0u8; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).unwrap();
    }
    bytes
}
