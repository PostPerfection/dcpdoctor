use std::path::Path;

use crate::hash::sha1_base64;

pub(crate) const CPL_ID: &str = "394080ca-5471-40e9-9827-e6e577753400";
pub(crate) const PKL_ID: &str = "d74e5590-b9fd-4482-8fd3-ddb7fe496e64";
pub(crate) const PICTURE_ID: &str = "c7d75d7b-7cec-4974-a665-b91536bec4cd";
pub(crate) const DESCRIPTOR_ID: &str = "5e7fd9bc-dec0-4334-a452-197f4376d5aa";

pub(crate) const CPL_FILE: &str = "CPL.xml";
pub(crate) const PKL_FILE: &str = "PKL.xml";
pub(crate) const PICTURE_FILE: &str = "VIDEO.mxf";

// AS-02 is the wrapping an IMP uses for picture essence
pub(crate) fn write_as02_picture(
    path: &Path,
    asset_uuid: uuid::Uuid,
    codestream: asdcplib::jp2k::CodestreamHeader,
    frames: u32,
    frame_bytes: usize,
) {
    use asdcplib::jp2k::{
        COLOR_PRIMARIES_BT709, HdrMetadata, PictureDescriptor, TRANSFER_CHARACTERISTIC_BT709,
    };
    use asdcplib::{LabelSet, Rational, WriterInfo};

    let info = WriterInfo {
        asset_uuid: *asset_uuid.as_bytes(),
        context_id: *uuid::Uuid::new_v4().as_bytes(),
        label_set: LabelSet::Smpte,
        ..Default::default()
    };
    let descriptor = PictureDescriptor {
        edit_rate: Rational::new(24, 1),
        sample_rate: Rational::new(24, 1),
        stored_width: 2048,
        stored_height: 1080,
        aspect_ratio: Rational::new(2048, 1080),
        container_duration: frames,
        codestream,
    };
    let hdr = HdrMetadata {
        color_primaries: Some(COLOR_PRIMARIES_BT709),
        transfer_characteristic: Some(TRANSFER_CHARACTERISTIC_BT709),
        ..Default::default()
    };
    let mut writer = asdcplib::as02::jp2k::MxfWriter::new();
    writer
        .open_write_hdr(path.to_str().unwrap(), &info, &descriptor, &hdr, 16384)
        .unwrap();
    let frame = vec![0u8; frame_bytes];
    for _ in 0..frames {
        writer.write_frame(&frame, None, None).unwrap();
    }
    writer.finalize().unwrap();
}

// the PKL carries the real size and SHA-1 of the files, so a mutation shows up
pub(crate) fn write_imp(dir: &Path) {
    write_as02_picture(
        &dir.join(PICTURE_FILE),
        PICTURE_ID.parse().unwrap(),
        crate::codestream_fixtures::imf_4k(),
        2,
        4096,
    );

    std::fs::write(
        dir.join(CPL_FILE),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/2067-3/2016" xmlns:cc="http://www.smpte-ra.org/ns/2067-2/2020" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <Id>urn:uuid:{CPL_ID}</Id>
  <IssueDate>2026-01-01T00:00:00+00:00</IssueDate>
  <Issuer>dcpdoctor tests</Issuer>
  <Creator>dcpdoctor tests</Creator>
  <ContentTitle>Mutation fixture</ContentTitle>
  <ContentKind>feature</ContentKind>
  <EssenceDescriptorList>
    <EssenceDescriptor>
      <Id>urn:uuid:{DESCRIPTOR_ID}</Id>
    </EssenceDescriptor>
  </EssenceDescriptorList>
  <EditRate>24 1</EditRate>
  <SegmentList>
    <Segment>
      <Id>urn:uuid:a3dfa541-9c0a-4b4b-9a43-59de39b5f0d2</Id>
      <SequenceList>
        <cc:MainImageSequence>
          <Id>urn:uuid:186bf940-2770-4046-a7dd-b5b90bd3c85e</Id>
          <TrackId>urn:uuid:123437fd-f375-4ddb-aa33-e78e094dce50</TrackId>
          <ResourceList>
            <Resource xsi:type="TrackFileResourceType">
              <Id>urn:uuid:d8c4801a-5bdb-47f0-8867-102b88af8815</Id>
              <EditRate>24 1</EditRate>
              <IntrinsicDuration>2</IntrinsicDuration>
              <SourceDuration>2</SourceDuration>
              <SourceEncoding>urn:uuid:{DESCRIPTOR_ID}</SourceEncoding>
              <TrackFileId>urn:uuid:{PICTURE_ID}</TrackFileId>
            </Resource>
          </ResourceList>
        </cc:MainImageSequence>
      </SequenceList>
    </Segment>
  </SegmentList>
</CompositionPlaylist>"#
        ),
    )
    .unwrap();

    let mut pkl_assets = String::new();
    for (id, file) in [(CPL_ID, CPL_FILE), (PICTURE_ID, PICTURE_FILE)] {
        let path = dir.join(file);
        let mime = if file.ends_with(".mxf") {
            "application/mxf"
        } else {
            "text/xml"
        };
        pkl_assets.push_str(&format!(
            r#"
    <Asset>
      <Id>urn:uuid:{id}</Id>
      <Hash>{hash}</Hash>
      <Size>{size}</Size>
      <Type>{mime}</Type>
      <HashAlgorithm Algorithm="http://www.w3.org/2000/09/xmldsig#sha1"/>
    </Asset>"#,
            hash = sha1_base64(&path).unwrap(),
            size = std::fs::metadata(&path).unwrap().len(),
        ));
    }
    std::fs::write(
        dir.join(PKL_FILE),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<PackingList xmlns="http://www.smpte-ra.org/schemas/2067-2/2016/PKL">
  <Id>urn:uuid:{PKL_ID}</Id>
  <IssueDate>2026-01-01T00:00:00+00:00</IssueDate>
  <Issuer>dcpdoctor tests</Issuer>
  <Creator>dcpdoctor tests</Creator>
  <AssetList>{pkl_assets}
  </AssetList>
</PackingList>"#
        ),
    )
    .unwrap();

    std::fs::write(
        dir.join("ASSETMAP.xml"),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:2fd93ab2-dab7-481d-bb43-5779ba62384d</Id>
  <Creator>dcpdoctor tests</Creator>
  <VolumeCount>1</VolumeCount>
  <IssueDate>2026-01-01T00:00:00+00:00</IssueDate>
  <Issuer>dcpdoctor tests</Issuer>
  <AssetList>
    <Asset>
      <Id>urn:uuid:{PKL_ID}</Id>
      <PackingList>true</PackingList>
      <ChunkList><Chunk><Path>{PKL_FILE}</Path></Chunk></ChunkList>
    </Asset>
    <Asset>
      <Id>urn:uuid:{CPL_ID}</Id>
      <ChunkList><Chunk><Path>{CPL_FILE}</Path></Chunk></ChunkList>
    </Asset>
    <Asset>
      <Id>urn:uuid:{PICTURE_ID}</Id>
      <ChunkList><Chunk><Path>{PICTURE_FILE}</Path></Chunk></ChunkList>
    </Asset>
  </AssetList>
</AssetMap>"#
        ),
    )
    .unwrap();
}
