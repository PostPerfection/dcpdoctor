//! Writes the five App 2E fixture IMPs the four picture-descriptor codes are
//! checked against: one clean package and one per code. Each IMP is a directory
//! holding ASSETMAP.xml, PKL.xml, CPL.xml, a one-frame AS-02 picture track file
//! and a one-frame AS-02 sound track file. The codestreams come from
//! tests/fixtures/j2c.
//!
//! cargo run -p dcpdoctor-core --example write_app2e_fixtures -- <output dir>

use asdcplib::jp2k::{
    CodestreamHeader, PICTURE_ESSENCE_CODING_CINEMA_2K, PICTURE_ESSENCE_CODING_IMF_4K_LOSSY_6_3,
};
use asdcplib::pcm::{AudioDescriptor, ChannelFormat};
use asdcplib::{LabelSet, Rational, WriterInfo};
use base64::Engine;
use dcpdoctor_core::app2e_fixtures::{
    PIXEL_LAYOUT_RGB_10, PIXEL_LAYOUT_RGB_12, bt709, patch_bytes, write_picture,
};
use sha1::{Digest, Sha1};
use std::path::{Path, PathBuf};

/// What the fixture's descriptor is made to say, and so which code it fires.
enum Defect {
    None,
    CinemaProfile,
    ColourMissing,
    LabelMismatch,
    LayoutMismatch,
}

struct Fixture {
    directory: &'static str,
    defect: Defect,
    /// The one code `dcpdoctor validate --imf` must report for this IMP.
    code: Option<&'static str>,
}

const FIXTURES: &[Fixture] = &[
    Fixture {
        directory: "clean",
        defect: Defect::None,
        code: None,
    },
    Fixture {
        directory: "cinema_profile",
        defect: Defect::CinemaProfile,
        code: Some("picture_not_imf_profile"),
    },
    Fixture {
        directory: "colour_missing",
        defect: Defect::ColourMissing,
        code: Some("picture_colour_missing"),
    },
    Fixture {
        directory: "label_mismatch",
        defect: Defect::LabelMismatch,
        code: Some("picture_coding_label_mismatch"),
    },
    Fixture {
        directory: "layout_mismatch",
        defect: Defect::LayoutMismatch,
        code: Some("picture_pixel_layout_mismatch"),
    },
];

const FRAMES: u32 = 1;
const EDIT_RATE: &str = "24 1";
const HEADER_BYTES: u32 = 16384;

/// The ST 2067-21 namespace that makes a CPL an App 2E composition.
const APP_2E_NAMESPACE: &str = "http://www.smpte-ra.org/ns/2067-21/2021";

/// Rsize of the DCI Cinema 2K profile, and where a codestream declares it: two
/// bytes past the SOC marker, the SIZ marker and Lsiz.
const CINEMA_2K_RSIZE: u16 = 0x0003;
const RSIZE_OFFSET: usize = 6;

fn codestream_fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/j2c")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The repository's only cinema-profile codestream is 64x64, which App 2E also
/// rejects on resolution, so the fixture would fire two codes. Declaring the
/// cinema profile on the 4K codestream leaves the resolution legal.
fn declare_cinema_profile(codestream: &mut [u8]) {
    codestream[RSIZE_OFFSET..RSIZE_OFFSET + 2].copy_from_slice(&CINEMA_2K_RSIZE.to_be_bytes());
}

/// A one-frame 48 kHz 24-bit stereo sound track file, so an App 2E composition
/// has the MainAudioSequence ST 2067-21 asks for.
fn write_sound(path: &Path) {
    const CHANNELS: u32 = 2;
    const BYTES_PER_SAMPLE: u32 = 3;
    const SAMPLE_RATE: u32 = 48_000;
    const FRAME_RATE: u32 = 24;

    let block_align = CHANNELS * BYTES_PER_SAMPLE;
    let descriptor = AudioDescriptor {
        edit_rate: Rational::new(FRAME_RATE as i32, 1),
        audio_sampling_rate: Rational::new(SAMPLE_RATE as i32, 1),
        locked: true,
        channel_count: CHANNELS,
        quantization_bits: BYTES_PER_SAMPLE * 8,
        block_align,
        avg_bps: SAMPLE_RATE * block_align,
        linked_track_id: 0,
        container_duration: FRAMES,
        channel_format: ChannelFormat::None,
    };
    let info = WriterInfo {
        asset_uuid: *uuid::Uuid::new_v4().as_bytes(),
        context_id: *uuid::Uuid::new_v4().as_bytes(),
        label_set: LabelSet::Smpte,
        ..Default::default()
    };
    let mut writer = asdcplib::as02::pcm::MxfWriter::new();
    writer
        .open_write(path.to_str().unwrap(), &info, &descriptor, HEADER_BYTES)
        .unwrap();
    let silence = vec![0u8; (SAMPLE_RATE / FRAME_RATE * block_align) as usize];
    for _ in 0..FRAMES {
        writer.write_frame(&silence, None, None).unwrap();
    }
    writer.finalize().unwrap();
}

fn sha1_base64(path: &Path) -> String {
    let mut hasher = Sha1::new();
    hasher.update(std::fs::read(path).unwrap());
    base64::engine::general_purpose::STANDARD.encode(hasher.finalize())
}

/// A uuid that only depends on the fixture name and the role, so a rerun writes
/// the same package.
fn fixture_uuid(directory: &str, role: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(directory.as_bytes());
    hasher.update(b"/");
    hasher.update(role.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    uuid::Builder::from_bytes(bytes)
        .with_version(uuid::Version::Sha1)
        .into_uuid()
        .to_string()
}

fn cpl_xml(cpl_id: &str, picture_id: &str, sound_id: &str, title: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/2067-3/2016"
                     xmlns:cc="http://www.smpte-ra.org/schemas/2067-2/2016"
                     xmlns:app="{APP_2E_NAMESPACE}">
  <Id>urn:uuid:{cpl_id}</Id>
  <IssueDate>2026-01-01T00:00:00+00:00</IssueDate>
  <Issuer>dcpdoctor</Issuer>
  <Creator>dcpdoctor write_app2e_fixtures</Creator>
  <ContentTitle>{title}</ContentTitle>
  <ContentKind>feature</ContentKind>
  <EditRate>{EDIT_RATE}</EditRate>
  <SegmentList>
    <Segment>
      <Id>urn:uuid:{segment_id}</Id>
      <SequenceList>
        <MainImageSequence>
          <Id>urn:uuid:{image_sequence_id}</Id>
          <TrackId>urn:uuid:{image_track_id}</TrackId>
          <ResourceList>
            <Resource>
              <Id>urn:uuid:{image_resource_id}</Id>
              <TrackFileId>urn:uuid:{picture_id}</TrackFileId>
              <EditRate>{EDIT_RATE}</EditRate>
              <IntrinsicDuration>{FRAMES}</IntrinsicDuration>
              <EntryPoint>0</EntryPoint>
              <SourceDuration>{FRAMES}</SourceDuration>
            </Resource>
          </ResourceList>
        </MainImageSequence>
        <MainAudioSequence>
          <Id>urn:uuid:{audio_sequence_id}</Id>
          <TrackId>urn:uuid:{audio_track_id}</TrackId>
          <ResourceList>
            <Resource>
              <Id>urn:uuid:{audio_resource_id}</Id>
              <TrackFileId>urn:uuid:{sound_id}</TrackFileId>
              <EditRate>{EDIT_RATE}</EditRate>
              <IntrinsicDuration>{FRAMES}</IntrinsicDuration>
              <EntryPoint>0</EntryPoint>
              <SourceDuration>{FRAMES}</SourceDuration>
            </Resource>
          </ResourceList>
        </MainAudioSequence>
      </SequenceList>
    </Segment>
  </SegmentList>
</CompositionPlaylist>"#,
        segment_id = fixture_uuid(cpl_id, "segment"),
        image_sequence_id = fixture_uuid(cpl_id, "image sequence"),
        image_track_id = fixture_uuid(cpl_id, "image track"),
        image_resource_id = fixture_uuid(cpl_id, "image resource"),
        audio_sequence_id = fixture_uuid(cpl_id, "audio sequence"),
        audio_track_id = fixture_uuid(cpl_id, "audio track"),
        audio_resource_id = fixture_uuid(cpl_id, "audio resource"),
    )
}

struct PackagedAsset {
    id: String,
    file_name: String,
    hash: String,
    size: u64,
    mime_type: &'static str,
}

fn packaged_asset(
    directory: &Path,
    file_name: &str,
    id: &str,
    mime_type: &'static str,
) -> PackagedAsset {
    let path = directory.join(file_name);
    PackagedAsset {
        id: id.to_string(),
        file_name: file_name.to_string(),
        hash: sha1_base64(&path),
        size: std::fs::metadata(&path).unwrap().len(),
        mime_type,
    }
}

fn pkl_xml(pkl_id: &str, assets: &[PackagedAsset]) -> String {
    let entries: String = assets
        .iter()
        .map(|asset| {
            format!(
                r#"
    <Asset>
      <Id>urn:uuid:{id}</Id>
      <Hash>{hash}</Hash>
      <Size>{size}</Size>
      <Type>{mime_type}</Type>
      <OriginalFileName>{file_name}</OriginalFileName>
      <HashAlgorithm Algorithm="http://www.w3.org/2000/09/xmldsig#sha1"/>
    </Asset>"#,
                id = asset.id,
                hash = asset.hash,
                size = asset.size,
                mime_type = asset.mime_type,
                file_name = asset.file_name,
            )
        })
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<PackingList xmlns="http://www.smpte-ra.org/schemas/2067-2/2016/PKL">
  <Id>urn:uuid:{pkl_id}</Id>
  <IssueDate>2026-01-01T00:00:00+00:00</IssueDate>
  <Issuer>dcpdoctor</Issuer>
  <Creator>dcpdoctor write_app2e_fixtures</Creator>
  <AssetList>{entries}
  </AssetList>
</PackingList>"#
    )
}

fn assetmap_xml(assetmap_id: &str, pkl_id: &str, assets: &[PackagedAsset]) -> String {
    let entries: String = assets
        .iter()
        .map(|asset| {
            let packing_list = if asset.id == pkl_id {
                "<PackingList>true</PackingList>"
            } else {
                ""
            };
            format!(
                r#"
    <Asset>
      <Id>urn:uuid:{id}</Id>
      {packing_list}
      <ChunkList><Chunk><Path>{file_name}</Path><VolumeIndex>1</VolumeIndex><Offset>0</Offset><Length>{size}</Length></Chunk></ChunkList>
    </Asset>"#,
                id = asset.id,
                file_name = asset.file_name,
                size = asset.size,
            )
        })
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:{assetmap_id}</Id>
  <Creator>dcpdoctor write_app2e_fixtures</Creator>
  <VolumeCount>1</VolumeCount>
  <IssueDate>2026-01-01T00:00:00+00:00</IssueDate>
  <Issuer>dcpdoctor</Issuer>
  <AssetList>{entries}
  </AssetList>
</AssetMap>"#
    )
}

fn write_fixture(output_root: &Path, fixture: &Fixture) -> PathBuf {
    let directory = output_root.join(fixture.directory);
    std::fs::create_dir_all(&directory).unwrap();

    let mut frame = codestream_fixture("imf4k_black_3840x2160.j2c");
    if matches!(fixture.defect, Defect::CinemaProfile) {
        declare_cinema_profile(&mut frame);
    }
    let codestream = CodestreamHeader::parse(&frame).expect("the fixture codestream parses");
    let colour = match fixture.defect {
        Defect::ColourMissing => None,
        _ => Some(bt709()),
    };

    let picture_id = fixture_uuid(fixture.directory, "picture");
    let picture_name = format!("{picture_id}.mxf");
    let picture_path = directory.join(&picture_name);
    write_picture(&picture_path, codestream, &frame, FRAMES, colour);

    match fixture.defect {
        Defect::LabelMismatch => patch_bytes(
            &picture_path,
            &PICTURE_ESSENCE_CODING_IMF_4K_LOSSY_6_3,
            &PICTURE_ESSENCE_CODING_CINEMA_2K,
        ),
        Defect::LayoutMismatch => {
            patch_bytes(&picture_path, &PIXEL_LAYOUT_RGB_12, &PIXEL_LAYOUT_RGB_10)
        }
        _ => {}
    }

    let sound_id = fixture_uuid(fixture.directory, "sound");
    let sound_name = format!("{sound_id}.mxf");
    write_sound(&directory.join(&sound_name));

    let cpl_id = fixture_uuid(fixture.directory, "cpl");
    std::fs::write(
        directory.join("CPL.xml"),
        cpl_xml(&cpl_id, &picture_id, &sound_id, fixture.directory),
    )
    .unwrap();

    let assets = vec![
        packaged_asset(&directory, "CPL.xml", &cpl_id, "text/xml"),
        packaged_asset(&directory, &picture_name, &picture_id, "application/mxf"),
        packaged_asset(&directory, &sound_name, &sound_id, "application/mxf"),
    ];
    let pkl_id = fixture_uuid(fixture.directory, "pkl");
    std::fs::write(directory.join("PKL.xml"), pkl_xml(&pkl_id, &assets)).unwrap();

    let mut listed = vec![packaged_asset(&directory, "PKL.xml", &pkl_id, "text/xml")];
    listed.extend(assets);
    std::fs::write(
        directory.join("ASSETMAP.xml"),
        assetmap_xml(
            &fixture_uuid(fixture.directory, "assetmap"),
            &pkl_id,
            &listed,
        ),
    )
    .unwrap();

    directory
}

fn main() {
    let Some(output_root) = std::env::args().nth(1).map(PathBuf::from) else {
        eprintln!("usage: write_app2e_fixtures <output dir>");
        std::process::exit(2);
    };
    for fixture in FIXTURES {
        let directory = write_fixture(&output_root, fixture);
        match fixture.code {
            Some(code) => println!("{} expects {code}", directory.display()),
            None => println!("{} expects no finding", directory.display()),
        }
    }
}
