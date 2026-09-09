//! `validate --imf --deep-j2k` over an IMP's AS-02 picture track: the same
//! per-frame codestream forensics the DCP path reports.

use std::path::Path;

use assert_cmd::Command;
use tempfile::TempDir;

const CPL_ID: &str = "1a1a1a1a-0000-0000-0000-000000000000";
const PKL_ID: &str = "dddddddd-0000-0000-0000-000000000000";
const PICTURE_ID: &str = "aaaa0000-0000-0000-0000-000000000000";
const SOUND_ID: &str = "bbbb0000-0000-0000-0000-000000000000";

const PICTURE_FRAMES: u32 = 6;
const PICTURE_WIDTH: u32 = 3840;
const PICTURE_HEIGHT: u32 = 2160;

fn imf_4k_codestream() -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/j2c/imf4k_black_3840x2160.j2c");
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn write_imp(dir: &Path) {
    let frame = imf_4k_codestream();
    dcpdoctor_core::app2e_fixtures::write_picture(
        &dir.join(format!("{PICTURE_ID}.mxf")),
        asdcplib::jp2k::CodestreamHeader::parse(&frame).unwrap(),
        &frame,
        PICTURE_FRAMES,
        Some(dcpdoctor_core::app2e_fixtures::bt709()),
    );

    std::fs::write(
        dir.join("ASSETMAP.xml"),
        format!(
            r#"<?xml version="1.0"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:cccccccc-0000-0000-0000-000000000000</Id>
  <AssetList>
    <Asset><Id>urn:uuid:{PKL_ID}</Id><PackingList>true</PackingList><ChunkList><Chunk><Path>PKL.xml</Path></Chunk></ChunkList></Asset>
    <Asset><Id>urn:uuid:{CPL_ID}</Id><ChunkList><Chunk><Path>CPL.xml</Path></Chunk></ChunkList></Asset>
    <Asset><Id>urn:uuid:{PICTURE_ID}</Id><ChunkList><Chunk><Path>{PICTURE_ID}.mxf</Path></Chunk></ChunkList></Asset>
    <Asset><Id>urn:uuid:{SOUND_ID}</Id><ChunkList><Chunk><Path>{SOUND_ID}.mxf</Path></Chunk></ChunkList></Asset>
  </AssetList>
</AssetMap>"#
        ),
    )
    .unwrap();

    std::fs::write(
        dir.join("PKL.xml"),
        format!(
            r#"<?xml version="1.0"?>
<PackingList xmlns="http://www.smpte-ra.org/schemas/2067-2/2016/PKL">
  <Id>urn:uuid:{PKL_ID}</Id>
  <AssetList><Asset><Id>urn:uuid:{CPL_ID}</Id><Type>text/xml</Type></Asset></AssetList>
</PackingList>"#
        ),
    )
    .unwrap();

    std::fs::write(
        dir.join("CPL.xml"),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/2067-3/2016"
                     xmlns:cc="http://www.smpte-ra.org/schemas/2067-2/2016">
  <Id>urn:uuid:{CPL_ID}</Id>
  <ContentTitle>Forensics fixture</ContentTitle>
  <EditRate>24 1</EditRate>
  <SegmentList>
    <Segment>
      <MainImageSequence>
        <Id>urn:uuid:eeeeeeee-0000-0000-0000-000000000000</Id>
        <ResourceList><Resource>
          <Id>urn:uuid:11110000-0000-0000-0000-000000000000</Id>
          <TrackFileId>urn:uuid:{PICTURE_ID}</TrackFileId>
          <EditRate>24 1</EditRate><IntrinsicDuration>{PICTURE_FRAMES}</IntrinsicDuration>
          <EntryPoint>0</EntryPoint><SourceDuration>{PICTURE_FRAMES}</SourceDuration>
        </Resource></ResourceList>
      </MainImageSequence>
      <MainAudioSequence>
        <Id>urn:uuid:ffffffff-0000-0000-0000-000000000000</Id>
        <ResourceList><Resource>
          <Id>urn:uuid:22220000-0000-0000-0000-000000000000</Id>
          <TrackFileId>urn:uuid:{SOUND_ID}</TrackFileId>
          <EditRate>24 1</EditRate><IntrinsicDuration>{PICTURE_FRAMES}</IntrinsicDuration>
          <EntryPoint>0</EntryPoint><SourceDuration>{PICTURE_FRAMES}</SourceDuration>
        </Resource></ResourceList>
      </MainAudioSequence>
    </Segment>
  </SegmentList>
</CompositionPlaylist>"#
        ),
    )
    .unwrap();
}

#[test]
fn an_imp_picture_track_reports_the_same_codestream_forensics_a_dcp_does() {
    let imp = TempDir::new().unwrap();
    write_imp(imp.path());

    let stdout = Command::cargo_bin("dcpdoctor")
        .unwrap()
        .args(["--json", "validate", "--imf", "--deep-j2k"])
        .arg(imp.path())
        .output()
        .unwrap()
        .stdout;
    let report: serde_json::Value = serde_json::from_slice(&stdout).unwrap();

    let summary = report["notes"]
        .as_array()
        .unwrap_or_else(|| panic!("no notes in {report:#}"))
        .iter()
        .find(|note| note["code"] == "J2kCodestreamSummary")
        .unwrap_or_else(|| panic!("no codestream forensics for the AS-02 track: {report:#}"));

    assert_eq!(summary["severity"], "info");
    let message = summary["message"].as_str().unwrap();
    for expected in [
        &format!("{PICTURE_WIDTH}x{PICTURE_HEIGHT}"),
        &format!("parameters identical across {PICTURE_FRAMES} frames"),
        &"decomposition levels".to_string(),
        &"code-blocks".to_string(),
        &"tile-parts".to_string(),
    ] {
        assert!(
            message.contains(expected.as_str()),
            "the forensics must report {expected}: {message}"
        );
    }
    assert!(
        summary["file"]
            .as_str()
            .unwrap()
            .ends_with(&format!("{PICTURE_ID}.mxf")),
        "the forensics must name the track file: {summary:#}"
    );
}
