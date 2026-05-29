//! DCI JPEG 2000 profile validation.
//!
//! Checks J2K codestream parameters against SMPTE 428-1 / DCI specification requirements.

use crate::j2k::J2kHeader;
use crate::{Note, Severity};

/// Validate J2K codestream header against DCI requirements.
pub fn validate_j2k(path: &str, header: &J2kHeader) -> Vec<Note> {
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

    notes
}
