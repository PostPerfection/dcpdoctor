// writers for the synthetic DCP track files the QC tests drive, panicking on
// failure because they only ever build fixtures

use std::path::Path;

use asdcplib::{LabelSet, Rational, WriterInfo};

const HEADER_BYTES: u32 = 16384;
const SAMPLE_RATE: u32 = 48_000;
const FRAME_RATE: u32 = 24;
const BYTES_PER_SAMPLE: u32 = 3;
const FULL_SCALE_24_BIT: f64 = 8_388_607.0;

// asdcplib MDD.cpp ImmersiveAudioCoding, the DataEssenceCoding of an Atmos track
pub const IMMERSIVE_AUDIO_CODING: [u8; 16] = [
    0x06, 0x0e, 0x2b, 0x34, 0x04, 0x01, 0x01, 0x05, 0x0e, 0x09, 0x06, 0x04, 0x00, 0x00, 0x00, 0x00,
];

// asdcplib MDD.cpp WaveAudioDescriptor, the set key an AS-02 PCM track carries
const WAVE_AUDIO_DESCRIPTOR_UL: [u8; 16] = [
    0x06, 0x0e, 0x2b, 0x34, 0x02, 0x53, 0x01, 0x01, 0x0d, 0x01, 0x01, 0x01, 0x01, 0x01, 0x48, 0x00,
];

// asdcplib MDD.cpp IABEssenceDescriptor, the set key IAB detection looks for
const IAB_ESSENCE_DESCRIPTOR_UL: [u8; 16] = [
    0x06, 0x0e, 0x2b, 0x34, 0x02, 0x53, 0x01, 0x01, 0x0d, 0x01, 0x01, 0x01, 0x01, 0x01, 0x7b, 0x00,
];

// the SDR colour an App 2E picture track carries, defined once for both fixture modules
pub use crate::app2e_fixtures::bt709;

// PQ over BT.2020 with an ST 2086 display mastered to 1000 nits
pub fn pq_bt2020() -> asdcplib::jp2k::HdrMetadata {
    asdcplib::jp2k::HdrMetadata {
        transfer_characteristic: Some(asdcplib::jp2k::TRANSFER_CHARACTERISTIC_ST2084),
        color_primaries: Some(asdcplib::jp2k::COLOR_PRIMARIES_BT2020),
        mastering_display_max_luminance: Some(10_000_000),
        mastering_display_min_luminance: Some(50),
        ..Default::default()
    }
}

pub fn hlg_bt2020() -> asdcplib::jp2k::HdrMetadata {
    asdcplib::jp2k::HdrMetadata {
        transfer_characteristic: Some(crate::premium::TRANSFER_CHARACTERISTIC_HLG),
        color_primaries: Some(asdcplib::jp2k::COLOR_PRIMARIES_BT2020),
        ..Default::default()
    }
}

pub struct ReelTiming {
    pub picture_entry: i64,
    pub picture_duration: i64,
    pub sound_entry: i64,
    pub sound_duration: i64,
}

// a ST 429-7 CPL of picture and sound reels, the timing a server plays them at
pub fn write_reel_cpl(path: &Path, reels: &[ReelTiming]) {
    let mut reel_xml = String::new();
    for reel in reels {
        reel_xml.push_str(&format!(
            r#"
    <Reel>
      <Id>urn:uuid:{reel_id}</Id>
      <AssetList>
        <MainPicture>
          <Id>urn:uuid:{picture_id}</Id>
          <EditRate>{FRAME_RATE} 1</EditRate>
          <IntrinsicDuration>{picture_intrinsic}</IntrinsicDuration>
          <EntryPoint>{picture_entry}</EntryPoint>
          <Duration>{picture_duration}</Duration>
        </MainPicture>
        <MainSound>
          <Id>urn:uuid:{sound_id}</Id>
          <EditRate>{FRAME_RATE} 1</EditRate>
          <IntrinsicDuration>{sound_intrinsic}</IntrinsicDuration>
          <EntryPoint>{sound_entry}</EntryPoint>
          <Duration>{sound_duration}</Duration>
        </MainSound>
      </AssetList>
    </Reel>"#,
            reel_id = uuid::Uuid::new_v4(),
            picture_id = uuid::Uuid::new_v4(),
            sound_id = uuid::Uuid::new_v4(),
            picture_intrinsic = reel.picture_entry + reel.picture_duration,
            sound_intrinsic = reel.sound_entry + reel.sound_duration,
            picture_entry = reel.picture_entry,
            picture_duration = reel.picture_duration,
            sound_entry = reel.sound_entry,
            sound_duration = reel.sound_duration,
        ));
    }

    std::fs::write(
        path,
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL">
  <Id>urn:uuid:{cpl_id}</Id>
  <IssueDate>2026-01-01T00:00:00+00:00</IssueDate>
  <Issuer>dcpdoctor tests</Issuer>
  <ContentTitleText>Sync fixture</ContentTitleText>
  <ContentKind>feature</ContentKind>
  <EditRate>{FRAME_RATE} 1</EditRate>
  <ReelList>{reel_xml}
  </ReelList>
</CompositionPlaylist>"#,
            cpl_id = uuid::Uuid::new_v4(),
        ),
    )
    .unwrap();
}

fn writer_info() -> WriterInfo {
    WriterInfo {
        asset_uuid: *uuid::Uuid::new_v4().as_bytes(),
        context_id: *uuid::Uuid::new_v4().as_bytes(),
        label_set: LabelSet::Smpte,
        ..Default::default()
    }
}

pub fn cinema_2k_codestream() -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/j2c/cinema2k_64x64.j2c");
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

pub fn write_picture_track(
    path: &Path,
    frames: u32,
    hdr: Option<asdcplib::jp2k::HdrMetadata>,
) -> Vec<u8> {
    let frame = cinema_2k_codestream();
    let codestream = asdcplib::jp2k::CodestreamHeader::parse(&frame).unwrap();
    let descriptor = asdcplib::jp2k::PictureDescriptor {
        edit_rate: Rational::new(FRAME_RATE as i32, 1),
        sample_rate: Rational::new(FRAME_RATE as i32, 1),
        stored_width: codestream.xsize,
        stored_height: codestream.ysize,
        aspect_ratio: Rational::new(
            i32::try_from(codestream.xsize).unwrap(),
            i32::try_from(codestream.ysize).unwrap(),
        ),
        container_duration: frames,
        codestream,
    };

    let info = writer_info();
    let mut writer = asdcplib::jp2k::MxfWriter::new();
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
        writer.write_frame(&frame, None, None).unwrap();
    }
    writer.finalize().unwrap();
    frame
}

pub struct SoundStretch {
    pub seconds: f64,
    // fraction of full scale, a 1 kHz tone unless it is zero
    pub amplitude: f64,
}

// 48 kHz 24-bit stereo at 24 fps, sample exact so a reported second can be checked
pub fn write_sound_track(path: &Path, stretches: &[SoundStretch]) {
    const CHANNELS: u32 = 2;
    const SAMPLES_PER_FRAME: u32 = SAMPLE_RATE / FRAME_RATE;

    let total_seconds: f64 = stretches.iter().map(|s| s.seconds).sum();
    let frames = (total_seconds * FRAME_RATE as f64).round() as u32;
    let block_align = CHANNELS * BYTES_PER_SAMPLE;

    let descriptor = asdcplib::pcm::AudioDescriptor {
        edit_rate: Rational::new(FRAME_RATE as i32, 1),
        audio_sampling_rate: Rational::new(SAMPLE_RATE as i32, 1),
        locked: true,
        channel_count: CHANNELS,
        quantization_bits: BYTES_PER_SAMPLE * 8,
        block_align,
        avg_bps: SAMPLE_RATE * block_align,
        linked_track_id: 0,
        container_duration: frames,
        channel_format: asdcplib::pcm::ChannelFormat::None,
    };

    let mut writer = asdcplib::pcm::MxfWriter::new();
    writer
        .open_write(
            path.to_str().unwrap(),
            &writer_info(),
            &descriptor,
            HEADER_BYTES,
        )
        .unwrap();

    for frame_index in 0..frames {
        let mut frame = Vec::with_capacity((SAMPLES_PER_FRAME * block_align) as usize);
        for sample_index in 0..SAMPLES_PER_FRAME {
            let seconds =
                (frame_index * SAMPLES_PER_FRAME + sample_index) as f64 / SAMPLE_RATE as f64;
            let value = (sample_amplitude(stretches, seconds) * FULL_SCALE_24_BIT) as i32;
            for _ in 0..CHANNELS {
                frame.extend_from_slice(&value.to_le_bytes()[..BYTES_PER_SAMPLE as usize]);
            }
        }
        writer.write_frame(&frame, None, None).unwrap();
    }
    writer.finalize().unwrap();
}

fn sample_amplitude(stretches: &[SoundStretch], seconds: f64) -> f64 {
    let mut start = 0.0;
    for stretch in stretches {
        if seconds < start + stretch.seconds {
            if stretch.amplitude == 0.0 {
                return 0.0;
            }
            return stretch.amplitude * (2.0 * std::f64::consts::PI * 1000.0 * seconds).sin();
        }
        start += stretch.seconds;
    }
    0.0
}

pub fn write_atmos_track(path: &Path, frames: u32, max_object_count: u16) {
    let info = writer_info();
    let descriptor = asdcplib::atmos::AtmosDescriptor {
        edit_rate: Rational::new(FRAME_RATE as i32, 1),
        container_duration: frames,
        asset_id: info.asset_uuid,
        data_essence_coding: IMMERSIVE_AUDIO_CODING,
        first_frame: 0,
        max_channel_count: 10,
        max_object_count,
        atmos_id: *uuid::Uuid::new_v4().as_bytes(),
        atmos_version: 1,
    };

    let mut writer = asdcplib::atmos::MxfWriter::new();
    writer
        .open_write(path.to_str().unwrap(), &info, &descriptor, HEADER_BYTES)
        .unwrap();
    let frame = vec![0u8; 2048];
    for _ in 0..frames {
        writer.write_frame(&frame, None, None).unwrap();
    }
    writer.finalize().unwrap();
}

// asdcplib has no IAB writer, so an AS-02 PCM file gets its descriptor set key
// overwritten with the IAB one, same length so every KLV length stays right
pub fn write_iab_track(path: &Path, frames: u32) {
    const CHANNELS: u32 = 2;
    let block_align = CHANNELS * BYTES_PER_SAMPLE;

    let descriptor = asdcplib::pcm::AudioDescriptor {
        edit_rate: Rational::new(FRAME_RATE as i32, 1),
        audio_sampling_rate: Rational::new(SAMPLE_RATE as i32, 1),
        locked: true,
        channel_count: CHANNELS,
        quantization_bits: BYTES_PER_SAMPLE * 8,
        block_align,
        avg_bps: SAMPLE_RATE * block_align,
        linked_track_id: 0,
        container_duration: frames,
        channel_format: asdcplib::pcm::ChannelFormat::None,
    };

    let mut writer = asdcplib::as02::pcm::MxfWriter::new();
    writer
        .open_write(
            path.to_str().unwrap(),
            &writer_info(),
            &descriptor,
            HEADER_BYTES,
        )
        .unwrap();
    let frame = vec![0u8; (SAMPLE_RATE / FRAME_RATE * block_align) as usize];
    for _ in 0..frames {
        writer.write_frame(&frame, None, None).unwrap();
    }
    writer.finalize().unwrap();

    let mut bytes = std::fs::read(path).unwrap();
    let offsets: Vec<usize> = bytes
        .windows(WAVE_AUDIO_DESCRIPTOR_UL.len())
        .enumerate()
        .filter(|(_, window)| *window == WAVE_AUDIO_DESCRIPTOR_UL)
        .map(|(offset, _)| offset)
        .collect();
    assert_eq!(
        offsets.len(),
        1,
        "expected one WaveAudioDescriptor, found {}",
        offsets.len()
    );
    for offset in offsets {
        bytes[offset..offset + IAB_ESSENCE_DESCRIPTOR_UL.len()]
            .copy_from_slice(&IAB_ESSENCE_DESCRIPTOR_UL);
    }
    std::fs::write(path, bytes).unwrap();
}
