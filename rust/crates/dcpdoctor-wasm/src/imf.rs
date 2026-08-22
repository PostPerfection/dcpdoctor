//! Native IMF validation for the browser (WASM).
//!
//! Uses `dcpdoctor_imf` for shared parsing and pure validation logic.
//! Converts results into the WASM crate's Note type for JavaScript.

use std::collections::HashSet;

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

    notes.extend(integer_parse_failure_note(&cpl, cpl_path));

    // Run all pure validators from the shared crate
    let imf_notes = dcpdoctor_imf::validate_imf_cpl_pure(&cpl);
    for n in &imf_notes {
        notes.push(convert_note(n, cpl_path));
    }

    // Track file cross-references (needs AssetMap data)
    if let Some(am_xml) = assetmap_xml {
        match parse_assetmap_ids(am_xml) {
            Ok(asset_ids) => {
                let ref_notes = dcpdoctor_imf::validate_track_refs(&cpl, &asset_ids);
                for n in &ref_notes {
                    notes.push(convert_note(n, cpl_path));
                }
            }
            Err(e) => notes.push(assetmap_unreadable_note(&e, cpl_path)),
        }
    }

    notes
}

/// The note for an ASSETMAP whose asset ids cannot be read, which leaves the
/// track file references unchecked.
fn assetmap_unreadable_note(error: &str, cpl_path: &str) -> Note {
    Note {
        severity: Severity::Error,
        code: "xml_parse_error".to_string(),
        message: format!(
            "Cannot read the asset ids of the ASSETMAP, so the track file references in this CPL were not checked: {error}"
        ),
        file: Some(cpl_path.to_string()),
    }
}

/// The note for CPL elements whose text is no integer: the duration and offset
/// checks read 0 for them.
fn integer_parse_failure_note(cpl: &dcpdoctor_imf::ImfCpl, cpl_path: &str) -> Option<Note> {
    if cpl.integer_parse_failures.is_empty() {
        return None;
    }
    Some(Note {
        severity: Severity::Error,
        code: "check_skipped".to_string(),
        message: format!(
            "CPL element(s) {} do not hold an integer, so the duration, offset and alignment checks that read them ran against 0",
            cpl.integer_parse_failures.join(", ")
        ),
        file: Some(cpl_path.to_string()),
    })
}

/// OV-aware IMF supplemental validation for the browser.
///
/// The browser has no filesystem, so the OV package cannot be a path. The
/// caller instead passes the OV's available asset ids (from its ASSETMAP). A
/// supplemental ref that resolves in the OV passes; a ref in neither the
/// supplemental nor the OV is a real `cross_ref_broken`. Pure validators still
/// run against the supplemental CPL.
pub fn validate_imf_supplemental(
    cpl_xml: &str,
    assetmap_xml: &str,
    ov_asset_ids: &HashSet<String>,
    cpl_path: &str,
) -> Vec<Note> {
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

    notes.extend(integer_parse_failure_note(&cpl, cpl_path));

    let imf_notes = dcpdoctor_imf::validate_imf_cpl_pure(&cpl);
    for n in &imf_notes {
        notes.push(convert_note(n, cpl_path));
    }

    let local_ids = match parse_assetmap_ids(assetmap_xml) {
        Ok(ids) => ids,
        Err(e) => {
            notes.push(assetmap_unreadable_note(&e, cpl_path));
            return notes;
        }
    };
    let ref_notes = dcpdoctor_imf::validate_track_refs_ov(&cpl, &local_ids, ov_asset_ids, true);
    for n in &ref_notes {
        notes.push(convert_note(n, cpl_path));
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

#[cfg(test)]
mod tests {
    use super::*;

    const PIC_ID: &str = "cccccccc-3333-3333-3333-cccccccccccc";

    // supplemental CPL references PIC_ID; the local ASSETMAP does not list it.
    const SUPP_CPL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/2067-3/2016"
                     xmlns:cc="http://www.smpte-ra.org/schemas/2067-2/2016">
  <Id>urn:uuid:12345678-1234-1234-1234-123456789abc</Id>
  <ContentTitle>Supp</ContentTitle>
  <EditRate>24 1</EditRate>
  <SegmentList><Segment>
    <MainImageSequence>
      <Id>urn:uuid:aaaaaaaa-1111-1111-1111-aaaaaaaaaaaa</Id>
      <ResourceList><Resource>
        <Id>urn:uuid:bbbbbbbb-2222-2222-2222-bbbbbbbbbbbb</Id>
        <TrackFileId>urn:uuid:cccccccc-3333-3333-3333-cccccccccccc</TrackFileId>
        <EditRate>24 1</EditRate><IntrinsicDuration>240</IntrinsicDuration>
        <EntryPoint>0</EntryPoint><SourceDuration>240</SourceDuration>
      </Resource></ResourceList>
    </MainImageSequence>
  </Segment></SegmentList>
</CompositionPlaylist>"#;

    const EMPTY_ASSETMAP: &str = r#"<?xml version="1.0"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:dddddddd-0000-0000-0000-000000000000</Id>
  <AssetList></AssetList>
</AssetMap>"#;

    #[test]
    fn ov_asset_ids_resolve_an_otherwise_broken_supplemental_ref() {
        let ov: HashSet<String> = [PIC_ID.to_string()].into();
        let notes = validate_imf_supplemental(SUPP_CPL, EMPTY_ASSETMAP, &ov, "cpl.xml");
        assert!(
            !notes.iter().any(|n| n.code == "cross_ref_broken"),
            "OV must satisfy the picture ref, got: {notes:?}"
        );
    }

    #[test]
    fn missing_ov_id_still_breaks_the_ref() {
        let ov: HashSet<String> = HashSet::new();
        let notes = validate_imf_supplemental(SUPP_CPL, EMPTY_ASSETMAP, &ov, "cpl.xml");
        assert!(
            notes.iter().any(|n| n.code == "cross_ref_broken"),
            "ref in neither package is a real break, got: {notes:?}"
        );
    }

    #[test]
    fn an_unreadable_assetmap_is_reported_instead_of_breaking_every_ref() {
        let broken = EMPTY_ASSETMAP.replace("</AssetList>", "</AssetLisst>");
        let notes = validate_imf_cpl(SUPP_CPL, Some(&broken), "cpl.xml");
        assert!(
            notes
                .iter()
                .any(|n| n.code == "xml_parse_error" && n.message.contains("not checked")),
            "an ASSETMAP that cannot be read must be reported, got: {notes:?}"
        );
        assert!(
            !notes.iter().any(|n| n.code == "cross_ref_broken"),
            "ids that were never read are no evidence of a broken ref, got: {notes:?}"
        );
    }

    #[test]
    fn a_duration_that_is_no_integer_is_reported_as_a_skipped_check() {
        let cpl = SUPP_CPL.replace(
            "<IntrinsicDuration>240</IntrinsicDuration>",
            "<IntrinsicDuration>two forty</IntrinsicDuration>",
        );
        let notes = validate_imf_cpl(&cpl, None, "cpl.xml");
        let skipped = notes
            .iter()
            .find(|n| n.code == "check_skipped")
            .unwrap_or_else(|| panic!("a coerced duration must be reported: {notes:?}"));
        assert!(
            skipped.message.contains("IntrinsicDuration"),
            "{}",
            skipped.message
        );
    }
}
