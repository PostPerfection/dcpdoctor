//! DCI JPEG 2000 profile validation.
//!
//! Checks J2K codestream parameters against SMPTE 428-1 / DCI specification requirements.

use crate::j2k::J2kHeader;
use crate::{Note, Severity};

/// POC markers the DCI profiles allow in a codestream's main header: none for
/// 2K, exactly one for 4K, where the marker signals the switch from the 2K
/// portion to the 4K portion. This file used to reject a POC marker outright,
/// which is a false positive on every conformant 4K DCP.
const POC_MARKERS_IN_MAIN_HEADER_2K: usize = 0;
const POC_MARKERS_IN_MAIN_HEADER_4K: usize = 1;

/// Stored width above which a codestream is a 4K one.
const MAX_2K_WIDTH: u32 = 2048;

/// Validate J2K codestream header against DCI requirements. `mxf_data` is the
/// buffer `header` was parsed from, so the marker walk can reach the codestream.
pub fn validate_j2k(path: &str, header: &J2kHeader, mxf_data: &[u8]) -> Vec<Note> {
    let mut notes = Vec::new();

    // DCI requires 3 components (XYZ color)
    if header.num_components != 3 {
        notes.push(Note {
            severity: Severity::Warning,
            code: "j2k_component_count".to_string(),
            message: format!(
                "J2K has {} components (DCI requires 3 for XYZ)",
                header.num_components
            ),
            file: Some(path.to_string()),
        });
    }

    // DCI requires 12-bit per component
    for (i, &depth) in header.bit_depths.iter().enumerate() {
        if depth != 12 {
            notes.push(Note {
                severity: Severity::Info,
                code: "j2k_bit_depth".to_string(),
                message: format!(
                    "J2K component {} has {}-bit depth (DCI requires 12-bit)",
                    i, depth
                ),
                file: Some(path.to_string()),
            });
            break; // Only report once
        }
    }

    // DCI requires 9-7 irreversible wavelet transform
    if !header.irreversible_wavelet && header.decomposition_levels > 0 {
        notes.push(Note {
            severity: Severity::Warning,
            code: "j2k_wavelet".to_string(),
            message: "J2K uses reversible 5-3 wavelet (DCI requires 9-7 irreversible)".to_string(),
            file: Some(path.to_string()),
        });
    }

    // DCI decomposition levels: typically 5 for 2K, 6 for 4K
    if header.decomposition_levels > 0 {
        let expected_levels = if header.width > 2048 { 6 } else { 5 };
        if header.decomposition_levels != expected_levels {
            notes.push(Note {
                severity: Severity::Info,
                code: "j2k_decomposition_levels".to_string(),
                message: format!(
                    "J2K has {} decomposition levels (typical for {}K is {})",
                    header.decomposition_levels,
                    if header.width > 2048 { "4" } else { "2" },
                    expected_levels
                ),
                file: Some(path.to_string()),
            });
        }
    }

    // DCI code-block size must be 32×32 (SMPTE 428-1)
    if header.codeblock_width > 0
        && header.codeblock_height > 0
        && (header.codeblock_width != 32 || header.codeblock_height != 32)
    {
        notes.push(Note {
            severity: Severity::Warning,
            code: "j2k_codeblock_size".to_string(),
            message: format!(
                "J2K code-block size {}×{} (DCI requires 32×32)",
                header.codeblock_width, header.codeblock_height
            ),
            file: Some(path.to_string()),
        });
    }

    // Tile size should equal image size (single tile) for DCI
    if header.tile_width > 0
        && header.tile_height > 0
        && (header.tile_width != header.width || header.tile_height != header.height)
    {
        notes.push(Note {
            severity: Severity::Info,
            code: "j2k_multi_tile".to_string(),
            message: format!(
                "J2K uses tiles {}×{} (DCI typically uses single tile = image size)",
                header.tile_width, header.tile_height
            ),
            file: Some(path.to_string()),
        });
    }

    // Progression order: DCI requires CPRL for 2K, CPRL for 4K
    if header.progression_order != 4 && header.decomposition_levels > 0 {
        let order_name = match header.progression_order {
            0 => "LRCP",
            1 => "RLCP",
            2 => "RPCL",
            3 => "PCRL",
            4 => "CPRL",
            _ => "unknown",
        };
        notes.push(Note {
            severity: Severity::Info,
            code: "j2k_progression".to_string(),
            message: format!(
                "J2K progression order: {} (DCI recommends CPRL)",
                order_name
            ),
            file: Some(path.to_string()),
        });
    }

    // Rsiz profile check
    // DCI profile: rsiz should include Cinema 2K (0x0003) or Cinema 4K (0x0004)
    if header.rsiz > 0 {
        let is_cinema_profile = (header.rsiz & 0x0003) != 0 // Cinema 2K
            || (header.rsiz & 0x0004) != 0 // Cinema 4K
            || header.rsiz == 3 || header.rsiz == 4;

        if !is_cinema_profile {
            notes.push(Note {
                severity: Severity::Info,
                code: "j2k_profile".to_string(),
                message: format!(
                    "J2K Rsiz=0x{:04X} (not a Cinema 2K/4K profile indicator)",
                    header.rsiz
                ),
                file: Some(path.to_string()),
            });
        }
    }

    // TLM presence and POC placement need the whole codestream, not just the
    // main-header fields above (SMPTE 428-1, matching libdcp's verify_j2k).
    let codestream = &mxf_data[header.codestream_offset.min(mxf_data.len())..];
    let scan = dcpdoctor_parse::j2k::scan_markers(codestream);
    if !scan.tlm_present {
        notes.push(Note {
            severity: Severity::Error,
            code: "j2k_missing_tlm".to_string(),
            message: "J2K codestream missing required TLM (tile-part length) marker".to_string(),
            file: Some(path.to_string()),
        });
    }

    let expected_poc = if header.width > MAX_2K_WIDTH {
        POC_MARKERS_IN_MAIN_HEADER_4K
    } else {
        POC_MARKERS_IN_MAIN_HEADER_2K
    };
    if scan.poc_in_main_header != expected_poc {
        notes.push(Note {
            severity: Severity::Error,
            code: "j2k_poc_invalid".to_string(),
            message: format!(
                "J2K main header has {} POC marker(s); DCI requires {expected_poc}",
                scan.poc_in_main_header
            ),
            file: Some(path.to_string()),
        });
    }
    if scan.poc_after_main_header > 0 {
        notes.push(Note {
            severity: Severity::Error,
            code: "j2k_poc_invalid".to_string(),
            message: format!(
                "{} POC marker(s) sit in a tile-part header, where no DCI profile permits one",
                scan.poc_after_main_header
            ),
            file: Some(path.to_string()),
        });
    }
    for mismatch in &scan.poc_field_mismatches {
        notes.push(Note {
            severity: Severity::Error,
            code: "j2k_poc_invalid".to_string(),
            message: format!(
                "POC {} is {}, expected {}",
                mismatch.field, mismatch.found, mismatch.expected
            ),
            file: Some(path.to_string()),
        });
    }

    // Number of tile-parts: DCI requires exactly 1 for single-tile (or 3 for 4K with 3 tiles)
    if header.tile_part_count > 0 {
        let expected = if header.tile_width > 0 && header.tile_width < header.width {
            // Multi-tile: tile count × layers
            let num_tiles_x = header.width.div_ceil(header.tile_width);
            let num_tiles_y = header.height.div_ceil(header.tile_height);
            num_tiles_x * num_tiles_y
        } else {
            1
        };
        if header.tile_part_count != expected {
            notes.push(Note {
                severity: Severity::Warning,
                code: "j2k_tile_part_count".to_string(),
                message: format!(
                    "J2K has {} tile-part(s) (expected {})",
                    header.tile_part_count, expected
                ),
                file: Some(path.to_string()),
            });
        }
    }

    notes
}
