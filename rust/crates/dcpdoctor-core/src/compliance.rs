//! Standard-agnostic compliance checks for DCP packages.

use std::path::Path;

use crate::{Code, Note, Severity};

const UUID_PATTERN: &str =
    r"^urn:uuid:[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$";

/// Validate that every `urn:uuid:` identifier in the DCP's XML documents is a
/// well-formed RFC 4122 UUID. Cheap and standard-agnostic; wired into the
/// default validate path (ClairMeta: check_cpl_id_rfc4122 / check_assets_*_uuid).
/// Only tokens that already carry the `urn:uuid:` prefix are checked, so bare
/// non-UUID identifiers in other schemas are not falsely flagged.
pub fn check_uuids(dcp_dir: &Path) -> Vec<Note> {
    let mut notes = Vec::new();
    let uuid_re = regex_lite::Regex::new(UUID_PATTERN).unwrap();
    let token_re = regex_lite::Regex::new(r"urn:uuid:[0-9A-Za-z._-]+").unwrap();

    let Ok(entries) = std::fs::read_dir(dcp_dir) else {
        return notes;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let is_xml = name.ends_with(".xml") || name.ends_with(".XML");
        if !is_xml && name != "ASSETMAP" && name != "VOLINDEX" {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let mut seen = std::collections::HashSet::new();
        for m in token_re.find_iter(&content) {
            let tok = m.as_str();
            if seen.insert(tok) && !uuid_re.is_match(tok) {
                notes.push(Note {
                    severity: Severity::Warning,
                    code: Code::InvalidUuid,
                    message: format!("Malformed UUID: {tok}"),
                    file: Some(path.clone()),
                    line: 0,
                });
            }
        }
    }
    notes
}
