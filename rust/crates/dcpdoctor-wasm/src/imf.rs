//! Native IMF validation for the browser (WASM).
//!
//! Uses `dcpdoctor_imf` for shared parsing and pure validation logic.
//! Converts results into the WASM crate's Note type for JavaScript.

use dcpdoctor_imf::{parse_assetmap_ids, ImfSeverity};

use crate::{Note, Severity};

/// Validate an IMF CPL from XML, optionally cross-referencing an AssetMap.
pub fn validate_imf_cpl(cpl_xml: &str, assetmap_xml: Option<&str>, cpl_path: &str) -> Vec<Note> {
    let mut notes = Vec::new();

    let cpl = match dcpdoctor_imf::parse_imf_cpl(cpl_xml) {
        Ok(c) => c,
        Err(e) => {
            notes.push(Note {
                severity: Severity::Error,
                code: "xml_parse_error".to_string(),
                message: format!("Failed to parse IMF CPL: {e}"),
                file: Some(cpl_path.to_string()),
            });
            return notes;
        }
    };

    // Run all pure validators from the shared crate
    let imf_notes = dcpdoctor_imf::validate_imf_cpl_pure(&cpl);
    for n in &imf_notes {
        notes.push(convert_note(n, cpl_path));
    }

    // Track file cross-references (needs AssetMap data)
    if let Some(am_xml) = assetmap_xml {
        let asset_ids = parse_assetmap_ids(am_xml);
        let ref_notes = dcpdoctor_imf::validate_track_refs(&cpl, &asset_ids);
        for n in &ref_notes {
            notes.push(convert_note(n, cpl_path));
        }
    }

    notes
}

/// Convert a shared `ImfNote` into the WASM crate's `Note` type.
fn convert_note(n: &dcpdoctor_imf::ImfNote, file: &str) -> Note {
    let severity = match n.severity {
        ImfSeverity::Error => Severity::Error,
        ImfSeverity::Warning => Severity::Warning,
        ImfSeverity::Info => Severity::Info,
    };
    Note {
        severity,
        code: n.code.to_string(),
        message: n.message.clone(),
        file: Some(file.to_string()),
    }
}
