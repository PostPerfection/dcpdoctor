/// ISDCF naming convention validation.
/// Checks if a content title follows the ISDCF Digital Cinema Naming Convention.
use crate::{Note, Severity};

/// Validate ISDCF naming convention for a content title.
pub fn check_naming(title: &str) -> Vec<Note> {
    let mut notes = Vec::new();

    if title.is_empty() {
        notes.push(Note {
            severity: Severity::Warning,
            code: "isdcf_empty_title".to_string(),
            message: "ContentTitleText is empty".to_string(),
            file: None,
        });
        return notes;
    }

    // ISDCF names use underscores as separators
    let parts: Vec<&str> = title.split('_').collect();

    if parts.len() < 5 {
        notes.push(Note {
            severity: Severity::Warning,
            code: "isdcf_few_fields".to_string(),
            message: format!(
                "Content title '{}' has only {} fields (expected 7+, ISDCF uses _ separators)",
                title,
                parts.len()
            ),
            file: None,
        });
    }

    // Check for spaces (not allowed in ISDCF)
    if title.contains(' ') {
        notes.push(Note {
            severity: Severity::Warning,
            code: "isdcf_spaces".to_string(),
            message: "Content title contains spaces (ISDCF uses underscores)".to_string(),
            file: None,
        });
    }

    // Validate content type field (2nd field) if present
    if parts.len() >= 2 {
        let valid_types = [
            "FTR", "TLR", "TSR", "PRO", "TST", "RTG", "SHR", "ADV", "XSN", "PSA", "POL",
        ];
        if !valid_types.contains(&parts[1]) && parts[1].len() == 3 {
            notes.push(Note {
                severity: Severity::Info,
                code: "isdcf_unknown_type".to_string(),
                message: format!(
                    "Content type '{}' is not a standard ISDCF type (FTR, TLR, TSR, etc.)",
                    parts[1]
                ),
                file: None,
            });
        }
    }

    notes
}
