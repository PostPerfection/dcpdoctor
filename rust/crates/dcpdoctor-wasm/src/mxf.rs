//! Pure-Rust MXF header parser for WASM.
//!
//! Parses MXF partition packs, KLV triplets, and local sets from raw bytes
//! to extract picture/sound descriptors and writer info without any native deps.

use serde::{Deserialize, Serialize};

// SMPTE 336M Universal Label prefix
const UL_PREFIX: [u8; 4] = [0x06, 0x0e, 0x2b, 0x34];

// Partition pack identification (bytes 4-7 after UL prefix)
const PARTITION_PACK_PREFIX: [u8; 3] = [0x02, 0x05, 0x01];

// Local set tags for Picture Essence Descriptor (SMPTE 377M)
const TAG_STORED_WIDTH: u16 = 0x3203;
const TAG_STORED_HEIGHT: u16 = 0x3202;
const TAG_SAMPLE_RATE: u16 = 0x3001; // Edit rate (rational)
const TAG_CONTAINER_DURATION: u16 = 0x3002;
const TAG_COMPONENT_DEPTH: u16 = 0x3301;

// Local set tags for Sound Essence Descriptor
const TAG_AUDIO_SAMPLING_RATE: u16 = 0x3D01;
const TAG_CHANNEL_COUNT: u16 = 0x3D07;
const TAG_AUDIO_QUANT_BITS: u16 = 0x3D03;

// Preface / Identification tags
const TAG_IDENT_PRODUCT_NAME: u16 = 0x3C04;

// Essence container labels — detect type from bytes 12-13 of the EC UL
fn is_jpeg2000_container(ul: &[u8; 16]) -> bool {
    ul[12] == 0x02 && ul[13] == 0x0c
}

fn is_pcm_container(ul: &[u8; 16]) -> bool {
    ul[12] == 0x02 && ul[13] == 0x06
}

fn is_timed_text_container(ul: &[u8; 16]) -> bool {
    ul[12] == 0x02 && ul[13] == 0x0d
}

fn is_dolby_atmos_container(ul: &[u8; 16]) -> bool {
    ul[12] == 0x02 && (ul[13] == 0x1e || ul[13] == 0x0e)
}

/// Essence type detected from MXF container label
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EssenceType {
    Jpeg2000,
    PcmAudio,
    TimedText,
    DolbyAtmos,
    #[default]
    Unknown,
}

impl EssenceType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Jpeg2000 => "JPEG 2000",
            Self::PcmAudio => "PCM Audio",
            Self::TimedText => "Timed Text",
            Self::DolbyAtmos => "Dolby Atmos (IAB)",
            Self::Unknown => "Unknown",
        }
    }
}

/// Picture essence descriptor extracted from MXF header metadata
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PictureDescriptor {
    pub width: u32,
    pub height: u32,
    pub frame_rate_num: u32,
    pub frame_rate_den: u32,
    pub bit_depth: u32,
    pub frame_count: u64,
    pub essence_type: Option<EssenceType>,
}

/// Sound essence descriptor extracted from MXF header metadata
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SoundDescriptor {
    pub sample_rate: u32,
    pub channels: u32,
    pub bit_depth: u32,
    pub duration: u64,
}

/// Writer info / product identification from MXF header
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WriterInfo {
    pub product_name: String,
    pub encrypted: bool,
}

/// MXF partition structure info
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PartitionInfo {
    pub has_header: bool,
    pub has_footer: bool,
    pub closed_complete: bool,
    pub body_partitions: u32,
}

/// Complete MXF metadata extracted from file header
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MxfMetadata {
    pub valid: bool,
    pub essence_type: EssenceType,
    pub picture: Option<PictureDescriptor>,
    pub sound: Option<SoundDescriptor>,
    pub writer_info: Option<WriterInfo>,
    pub partitions: PartitionInfo,
    pub error: Option<String>,
}

/// Parse MXF metadata from raw bytes (first ~1 MB of the file).
///
/// This extracts partition info, picture/sound descriptors, and writer identification
/// by parsing KLV triplets and local sets per SMPTE 377M.
pub fn parse_mxf(data: &[u8]) -> MxfMetadata {
    let mut meta = MxfMetadata::default();

    if data.len() < 20 {
        meta.error = Some("File too small for MXF".into());
        return meta;
    }

    // Verify UL prefix
    if data[0..4] != UL_PREFIX {
        meta.error = Some("Not an MXF file (bad UL prefix)".into());
        return meta;
    }

    // Verify partition pack key (bytes 4-6)
    if data[4..7] != PARTITION_PACK_PREFIX {
        meta.error = Some("Not an MXF file (not a partition pack)".into());
        return meta;
    }

    meta.valid = true;
    meta.partitions.has_header = true;

    // Check partition status (byte 14)
    if data.len() > 14 {
        meta.partitions.closed_complete = data[14] >= 0x04;
    }

    // Read partition pack BER length
    let (pack_len, header_end) = match read_ber_length(data, 16) {
        Some(v) => v,
        None => {
            meta.error = Some("Invalid partition pack BER length".into());
            return meta;
        }
    };

    let pack_end = header_end + pack_len as usize;
    if pack_end > data.len() {
        meta.error = Some("Truncated partition pack".into());
        return meta;
    }

    // Parse essence container label from partition pack
    let pack_data = &data[header_end..pack_end];
    if pack_data.len() >= 100 {
        // EC labels batch starts around offset 84 in the partition pack value
        let ec_offset = 84;
        if ec_offset + 8 <= pack_data.len() {
            let ec_count = read_u32_be(&pack_data[ec_offset..]);
            let ec_item_len = read_u32_be(&pack_data[ec_offset + 4..]);
            if ec_item_len == 16 && ec_count > 0 && ec_offset + 8 + 16 <= pack_data.len() {
                let mut ec_label = [0u8; 16];
                ec_label.copy_from_slice(&pack_data[ec_offset + 8..ec_offset + 24]);
                meta.essence_type = detect_essence_type(&ec_label);
            }
        }
    }

    // Scan KLV triplets in header metadata (limit to available data)
    let max_scan = data.len().min(1024 * 1024); // Cap at 1 MB
    let mut pos = pack_end;

    while pos + 20 <= max_scan {
        // Read 16-byte key
        let key: [u8; 16] = match data[pos..pos + 16].try_into() {
            Ok(k) => k,
            Err(_) => break,
        };
        pos += 16;

        // Read BER length
        let (klv_len, value_start) = match read_ber_length(data, pos) {
            Some(v) => v,
            None => break,
        };

        if klv_len == 0 {
            break;
        }

        let value_end = value_start + klv_len as usize;
        if value_end > max_scan {
            break;
        }

        let value = &data[value_start..value_end];

        // Check for picture descriptor
        if is_picture_descriptor_ul(&key) {
            let pic = parse_picture_descriptor(value, &meta.essence_type);
            meta.picture = Some(pic);
        }
        // Check for sound descriptor
        else if is_sound_descriptor_ul(&key) {
            let snd = parse_sound_descriptor(value);
            meta.sound = Some(snd);
        }
        // Check for Identification set (contains WriterInfo)
        else if is_identification_ul(&key) {
            let info = parse_writer_info(value);
            if !info.product_name.is_empty() {
                meta.writer_info = Some(info);
            }
        }

        pos = value_end;
    }

    // Scan for footer partition in the last 64K of data (if we have it)
    if data.len() > 65536 {
        scan_for_partitions(data, &mut meta.partitions);
    }

    meta
}

/// Check if this UL is a CDCI or RGBA Picture Essence Descriptor
fn is_picture_descriptor_ul(ul: &[u8; 16]) -> bool {
    ul[0..4] == UL_PREFIX && ul[4] == 0x02 && ul[5] == 0x53 && (ul[13] == 0x28 || ul[13] == 0x29)
}

/// Check if this UL is a Generic Sound or Wave PCM Descriptor
fn is_sound_descriptor_ul(ul: &[u8; 16]) -> bool {
    ul[0..4] == UL_PREFIX && ul[4] == 0x02 && ul[5] == 0x53 && (ul[13] == 0x42 || ul[13] == 0x48)
}

/// Check if this UL is an Identification set
/// Identification: 06.0e.2b.34.02.53.01.01.0d.01.01.01.01.01.30.00
fn is_identification_ul(ul: &[u8; 16]) -> bool {
    ul[0..4] == UL_PREFIX && ul[4] == 0x02 && ul[5] == 0x53 && ul[13] == 0x30
}

fn detect_essence_type(ec_label: &[u8; 16]) -> EssenceType {
    if is_jpeg2000_container(ec_label) {
        EssenceType::Jpeg2000
    } else if is_pcm_container(ec_label) {
        EssenceType::PcmAudio
    } else if is_timed_text_container(ec_label) {
        EssenceType::TimedText
    } else if is_dolby_atmos_container(ec_label) {
        EssenceType::DolbyAtmos
    } else {
        EssenceType::Unknown
    }
}

fn parse_picture_descriptor(data: &[u8], essence_type: &EssenceType) -> PictureDescriptor {
    let etype = match essence_type {
        EssenceType::Unknown => EssenceType::Jpeg2000,
        other => *other,
    };
    let mut pic = PictureDescriptor {
        essence_type: Some(etype),
        ..Default::default()
    };

    for (tag, value) in parse_local_set(data) {
        match tag {
            TAG_STORED_WIDTH if value.len() >= 4 => {
                pic.width = read_u32_be(value);
            }
            TAG_STORED_HEIGHT if value.len() >= 4 => {
                pic.height = read_u32_be(value);
            }
            TAG_SAMPLE_RATE if value.len() >= 8 => {
                pic.frame_rate_num = read_u32_be(value);
                pic.frame_rate_den = read_u32_be(&value[4..]);
            }
            TAG_CONTAINER_DURATION if value.len() >= 8 => {
                pic.frame_count = read_u64_be(value);
            }
            TAG_COMPONENT_DEPTH if value.len() >= 4 => {
                pic.bit_depth = read_u32_be(value);
            }
            _ => {}
        }
    }

    pic
}

fn parse_sound_descriptor(data: &[u8]) -> SoundDescriptor {
    let mut snd = SoundDescriptor::default();

    for (tag, value) in parse_local_set(data) {
        match tag {
            TAG_AUDIO_SAMPLING_RATE if value.len() >= 8 => {
                // Rational: numerator/denominator
                snd.sample_rate = read_u32_be(value);
            }
            TAG_CHANNEL_COUNT if value.len() >= 4 => {
                snd.channels = read_u32_be(value);
            }
            TAG_AUDIO_QUANT_BITS if value.len() >= 4 => {
                snd.bit_depth = read_u32_be(value);
            }
            TAG_CONTAINER_DURATION if value.len() >= 8 => {
                snd.duration = read_u64_be(value);
            }
            _ => {}
        }
    }

    snd
}

fn parse_writer_info(data: &[u8]) -> WriterInfo {
    let mut info = WriterInfo::default();

    for (tag, value) in parse_local_set(data) {
        if tag == TAG_IDENT_PRODUCT_NAME && !value.is_empty() {
            // UTF-16BE encoded string
            info.product_name = decode_utf16be(value);
        }
    }

    info
}

/// Scan for footer and body partitions in file data
fn scan_for_partitions(data: &[u8], partitions: &mut PartitionInfo) {
    // Partition pack key: 06.0e.2b.34.02.05.01.01.0d.01.02.01.01.XX.YY.00
    // XX = partition type: 02=header, 03=body, 04=footer
    // YY = open/closed/complete status
    let tail_start = data.len().saturating_sub(65536);
    let tail = &data[tail_start..];

    for i in 0..tail.len().saturating_sub(16) {
        if tail[i..i + 4] == UL_PREFIX
            && tail[i + 4..i + 7] == PARTITION_PACK_PREFIX
            && tail[i + 8] == 0x0d
            && tail[i + 9] == 0x01
            && tail[i + 10] == 0x02
            && tail[i + 11] == 0x01
            && tail[i + 12] == 0x01
        {
            match tail[i + 13] {
                0x04 => partitions.has_footer = true,
                0x03 => {
                    partitions.body_partitions += 1;
                }
                _ => {}
            }
        }
    }
}

// === Binary helpers ===

/// Read BER-encoded length from buffer at given offset.
/// Returns (length_value, offset_after_ber).
fn read_ber_length(data: &[u8], offset: usize) -> Option<(u64, usize)> {
    if offset >= data.len() {
        return None;
    }

    let first = data[offset];
    if first < 0x80 {
        return Some((first as u64, offset + 1));
    }

    let num_bytes = (first & 0x7f) as usize;
    if num_bytes > 8 || offset + 1 + num_bytes > data.len() {
        return None;
    }

    let mut len: u64 = 0;
    for i in 0..num_bytes {
        len = (len << 8) | data[offset + 1 + i] as u64;
    }

    Some((len, offset + 1 + num_bytes))
}

fn read_u16_be(data: &[u8]) -> u16 {
    (data[0] as u16) << 8 | data[1] as u16
}

fn read_u32_be(data: &[u8]) -> u32 {
    (data[0] as u32) << 24 | (data[1] as u32) << 16 | (data[2] as u32) << 8 | data[3] as u32
}

fn read_u64_be(data: &[u8]) -> u64 {
    (read_u32_be(data) as u64) << 32 | read_u32_be(&data[4..]) as u64
}

/// Parse SMPTE 377M local set: sequence of (tag:u16, len:u16, value) entries
fn parse_local_set(data: &[u8]) -> Vec<(u16, &[u8])> {
    let mut tags = Vec::new();
    let mut pos = 0;

    while pos + 4 <= data.len() {
        let tag = read_u16_be(&data[pos..]);
        let vlen = read_u16_be(&data[pos + 2..]) as usize;
        pos += 4;
        if pos + vlen > data.len() {
            break;
        }
        tags.push((tag, &data[pos..pos + vlen]));
        pos += vlen;
    }

    tags
}

/// Decode UTF-16BE bytes to a String
fn decode_utf16be(data: &[u8]) -> String {
    let chars: Vec<u16> = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_be_bytes(*c))
        .collect();
    String::from_utf16_lossy(&chars)
        .trim_end_matches('\0')
        .to_string()
}
