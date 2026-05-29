//! JPEG 2000 codestream header parser for DCI profile validation.
//!
//! Parses SIZ (image parameters) and COD (coding style) marker segments
//! from the first J2K frame found in MXF data.
//!
//! Adapted from caliban-rs (pure-Rust J2K decoder).

use serde::{Deserialize, Serialize};

// J2K marker codes
const SOC: u16 = 0xFF4F; // Start of codestream
const SIZ: u16 = 0xFF51; // Image and tile size
const COD: u16 = 0xFF52; // Coding style default

/// Parsed J2K codestream header parameters relevant to DCI validation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct J2kHeader {
    /// Rsiz capability field (profile indicator)
    pub rsiz: u16,
    /// Image width
    pub width: u32,
    /// Image height
    pub height: u32,
    /// Tile width
    pub tile_width: u32,
    /// Tile height
    pub tile_height: u32,
    /// Number of components
    pub num_components: u16,
    /// Bit depth per component
    pub bit_depths: Vec<u8>,
    /// Number of DWT decomposition levels
    pub decomposition_levels: u8,
    /// Code-block width (2^(cb_width_exp + 2))
    pub codeblock_width: u32,
    /// Code-block height (2^(cb_height_exp + 2))
    pub codeblock_height: u32,
    /// Wavelet transform: true = 9-7 irreversible, false = 5-3 reversible
    pub irreversible_wavelet: bool,
    /// Progression order (0=LRCP, 1=RLCP, 2=RPCL, 3=PCRL, 4=CPRL)
    pub progression_order: u8,
    /// Number of quality layers
    pub num_layers: u16,
    /// Multi-component transform
    pub mct: bool,
}

/// Find and parse J2K codestream header from MXF file data.
///
/// Scans for the first SOC marker (0xFF4F) followed by SIZ and COD,
/// then extracts DCI-relevant parameters.
///
/// Pass at least the first 2 MB of the MXF file for best results.
pub fn parse_j2k_from_mxf(data: &[u8]) -> Option<J2kHeader> {
    // Scan for SOC marker (start of J2K codestream)
    // Skip MXF header area (usually first ~20KB), start scanning from offset 4096
    let start = 4096.min(data.len());
    let soc_offset = find_marker(data, start, SOC)?;

    // After SOC, expect SIZ marker immediately
    let siz_offset = soc_offset + 2;
    if siz_offset + 2 > data.len() {
        return None;
    }

    let marker = read_u16_be(&data[siz_offset..]);
    if marker != SIZ {
        return None;
    }

    // Read SIZ segment length
    let siz_len_offset = siz_offset + 2;
    if siz_len_offset + 2 > data.len() {
        return None;
    }
    let siz_len = read_u16_be(&data[siz_len_offset..]) as usize;
    let siz_data_offset = siz_len_offset + 2;
    if siz_data_offset + siz_len - 2 > data.len() {
        return None;
    }
    let siz_data = &data[siz_data_offset..siz_data_offset + siz_len - 2];

    // Parse SIZ
    let siz = parse_siz(siz_data)?;

    // Find COD marker after SIZ
    let after_siz = siz_data_offset + siz_len - 2;
    let cod_offset = find_marker(data, after_siz, COD)?;

    // Read COD segment
    let cod_len_offset = cod_offset + 2;
    if cod_len_offset + 2 > data.len() {
        return Some(siz); // Return SIZ-only if COD not found
    }
    let cod_len = read_u16_be(&data[cod_len_offset..]) as usize;
    let cod_data_offset = cod_len_offset + 2;
    if cod_data_offset + cod_len - 2 > data.len() {
        return Some(siz);
    }
    let cod_data = &data[cod_data_offset..cod_data_offset + cod_len - 2];

    // Parse COD and merge into header
    let mut header = siz;
    parse_cod(cod_data, &mut header);

    Some(header)
}

/// Parse SIZ marker segment data (after marker + length)
fn parse_siz(data: &[u8]) -> Option<J2kHeader> {
    if data.len() < 36 {
        return None;
    }

    let mut pos = 0;
    let rsiz = read_u16_be(&data[pos..]);
    pos += 2;
    let width = read_u32_be(&data[pos..]);
    pos += 4;
    let height = read_u32_be(&data[pos..]);
    pos += 4;
    let _x_origin = read_u32_be(&data[pos..]);
    pos += 4;
    let _y_origin = read_u32_be(&data[pos..]);
    pos += 4;
    let tile_width = read_u32_be(&data[pos..]);
    pos += 4;
    let tile_height = read_u32_be(&data[pos..]);
    pos += 4;
    let _tile_x_origin = read_u32_be(&data[pos..]);
    pos += 4;
    let _tile_y_origin = read_u32_be(&data[pos..]);
    pos += 4;
    let num_components = read_u16_be(&data[pos..]);
    pos += 2;

    let mut bit_depths = Vec::with_capacity(num_components as usize);
    for _ in 0..num_components {
        if pos >= data.len() {
            break;
        }
        let ssiz = data[pos];
        let depth = (ssiz & 0x7F) + 1;
        bit_depths.push(depth);
        pos += 3; // Ssiz + XRsiz + YRsiz
    }

    Some(J2kHeader {
        rsiz,
        width,
        height,
        tile_width,
        tile_height,
        num_components,
        bit_depths,
        ..Default::default()
    })
}

/// Parse COD marker segment data and update header
fn parse_cod(data: &[u8], header: &mut J2kHeader) {
    if data.len() < 9 {
        return;
    }

    let mut pos = 0;
    let _scod = data[pos]; // Coding style flags
    pos += 1;

    // SGcod
    header.progression_order = data[pos];
    pos += 1;
    header.num_layers = read_u16_be(&data[pos..]);
    pos += 2;
    header.mct = data[pos] != 0;
    pos += 1;

    // SPcod
    header.decomposition_levels = data[pos];
    pos += 1;
    let cb_width_exp = data[pos];
    pos += 1;
    let cb_height_exp = data[pos];
    pos += 1;

    header.codeblock_width = 1 << (cb_width_exp + 2);
    header.codeblock_height = 1 << (cb_height_exp + 2);

    // Skip modes byte
    pos += 1;

    // Wavelet transform
    if pos < data.len() {
        header.irreversible_wavelet = data[pos] == 0; // 0 = 9-7 irreversible
    }
}

/// Find a 2-byte marker in data starting from offset
fn find_marker(data: &[u8], start: usize, marker: u16) -> Option<usize> {
    let hi = (marker >> 8) as u8;
    let lo = (marker & 0xFF) as u8;

    (start..data.len().saturating_sub(1)).find(|&i| data[i] == hi && data[i + 1] == lo)
}

fn read_u16_be(data: &[u8]) -> u16 {
    (data[0] as u16) << 8 | data[1] as u16
}

fn read_u32_be(data: &[u8]) -> u32 {
    (data[0] as u32) << 24 | (data[1] as u32) << 16 | (data[2] as u32) << 8 | data[3] as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_parse_j2k_from_real_mxf() {
        let path =
            "/home/aaron/src/dci-ctp/tests/generated/short_2k_24fps/picture_b18b5597-e6f0-4dac-a88a-a44938cc92eb.mxf";
        if !std::path::Path::new(path).exists() {
            eprintln!("Skipping test — MXF fixture not found");
            return;
        }
        let data = fs::read(path).unwrap();
        let header = &data[..std::cmp::min(2 * 1024 * 1024, data.len())];
        let result = parse_j2k_from_mxf(header);
        assert!(
            result.is_some(),
            "Should find J2K codestream in picture MXF"
        );
        let h = result.unwrap();
        eprintln!(
            "J2K header: {}×{}, {} components, bit_depths={:?}, levels={}, cb={}×{}, irrev={}, prog={}",
            h.width, h.height, h.num_components, h.bit_depths, h.decomposition_levels,
            h.codeblock_width, h.codeblock_height, h.irreversible_wavelet, h.progression_order
        );
        assert_eq!(h.width, 2048);
        assert!(h.height > 0, "Height should be non-zero");
        assert_eq!(h.num_components, 3);
        // Validate structure was parsed — actual values depend on source material
        assert!(!h.bit_depths.is_empty());
        assert!(h.decomposition_levels > 0);
        assert!(h.codeblock_width > 0);
        assert!(h.codeblock_height > 0);
    }
}
