//! HFR (High Frame Rate) validation.

use std::path::Path;

use crate::{Code, Note, Severity};

/// Check HFR compliance for a CPL.
pub fn check_hfr_compliance(cpl_path: &Path) -> Vec<Note> {
    let mut notes = Vec::new();

    let content = match std::fs::read_to_string(cpl_path) {
        Ok(c) => c,
        Err(_) => return notes,
    };

    if !content.contains("CompositionPlaylist") {
        return notes;
    }

    let rate_re = regex_lite::Regex::new(r"<EditRate>(\d+)\s+(\d+)</EditRate>").unwrap();
    let Some(cap) = rate_re.captures(&content) else {
        return notes;
    };

    let num: f64 = cap[1].parse().unwrap_or(0.0);
    let den: f64 = cap[2].parse().unwrap_or(1.0);
    if den <= 0.0 {
        return notes;
    }
    let fps = num / den;
    let path_buf = Some(cpl_path.to_path_buf());

    if fps > 30.0 {
        notes.push(Note {
            severity: Severity::Info,
            code: Code::CplInvalidEditRate,
            message: format!("HFR content detected: {} fps", fps as u32),
            file: path_buf.clone(),
            line: 0,
        });

        if fps > 60.0 {
            notes.push(Note {
                severity: Severity::Warning,
                code: Code::CplInvalidEditRate,
                message: format!(
                    "Frame rate {} fps exceeds DCI maximum (60fps for 2K, 48fps for 4K)",
                    fps as u32
                ),
                file: path_buf.clone(),
                line: 0,
            });
        }

        // 4K + >48fps check
        let width_re = regex_lite::Regex::new(r"<StoredWidth>(\d+)</StoredWidth>").unwrap();
        if let Some(wcap) = width_re.captures(&content) {
            let width: u32 = wcap[1].parse().unwrap_or(0);
            if width > 2048 && fps > 48.0 {
                notes.push(Note {
                    severity: Severity::Error,
                    code: Code::CplInvalidEditRate,
                    message: format!(
                        "4K content at {} fps exceeds DCI 4K HFR limit of 48fps",
                        fps as u32
                    ),
                    file: path_buf.clone(),
                    line: 0,
                });
            }
        }

        // BV2.1 approved rates
        let bv21_rate = fps == 48.0 || fps == 60.0;
        if !bv21_rate {
            notes.push(Note {
                severity: Severity::Warning,
                code: Code::CplInvalidEditRate,
                message: format!(
                    "Frame rate {} fps is HFR but not a BV2.1 approved rate (48 or 60)",
                    fps as u32
                ),
                file: path_buf.clone(),
                line: 0,
            });
        }

        notes.push(Note {
            severity: Severity::Info,
            code: Code::J2kBitrateExceeded,
            message: "HFR content: DCI maximum bitrate is 500 Mbps for all HFR content".into(),
            file: path_buf,
            line: 0,
        });
    }

    notes
}
