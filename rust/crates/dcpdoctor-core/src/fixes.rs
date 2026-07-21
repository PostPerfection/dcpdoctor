//! Fix suggestions for common DCP issues.

use serde::Serialize;

use crate::Code;

/// A suggested fix for a validation issue.
#[derive(Debug, Clone, Serialize)]
pub struct FixSuggestion {
    pub related_code: Code,
    pub description: String,
    pub command: String,
    pub auto_fixable: bool,
}

/// Generate fix suggestions from validation notes.
pub fn suggest_fixes(notes: &[crate::Note]) -> Vec<FixSuggestion> {
    let mut suggestions = Vec::new();

    for note in notes {
        let suggestion = match note.code {
            Code::SmpteNamingViolation if note.message.contains("ASSETMAP") => {
                Some(FixSuggestion {
                    related_code: Code::SmpteNamingViolation,
                    description: "Rename ASSETMAP to ASSETMAP.xml for BV2.1 compliance".into(),
                    command: "mv ASSETMAP ASSETMAP.xml".into(),
                    auto_fixable: true,
                })
            }
            Code::SmpteNamespaceWrong => Some(FixSuggestion {
                related_code: Code::SmpteNamespaceWrong,
                description: "Fix namespace: replace Interop namespace with SMPTE namespace".into(),
                command: String::new(),
                auto_fixable: true,
            }),
            Code::PklHashMismatch => Some(FixSuggestion {
                related_code: Code::PklHashMismatch,
                description: "Regenerate PKL hashes from actual file contents".into(),
                command: String::new(),
                auto_fixable: true,
            }),
            Code::MissingRequiredElement if note.message.contains("ContentVersion") => {
                Some(FixSuggestion {
                    related_code: Code::MissingRequiredElement,
                    description: "Add ContentVersion element to CPL (required for BV2.1)".into(),
                    command: String::new(),
                    auto_fixable: false,
                })
            }
            Code::MissingRequiredElement if note.message.contains("MainMarkers") => {
                Some(FixSuggestion {
                    related_code: Code::MissingRequiredElement,
                    description: "Add MainMarkers to first reel (FFOC, LFOC at minimum for BV2.1)"
                        .into(),
                    command: String::new(),
                    auto_fixable: false,
                })
            }
            Code::J2kBitrateExceeded => Some(FixSuggestion {
                related_code: Code::J2kBitrateExceeded,
                description: "Re-encode picture at lower bitrate (DCI: 250 Mbps 2K / 500 Mbps 4K)"
                    .into(),
                command: String::new(),
                auto_fixable: false,
            }),
            Code::IsdcfNamingViolation => Some(FixSuggestion {
                related_code: Code::IsdcfNamingViolation,
                description: "Rename content title to ISDCF convention".into(),
                command: String::new(),
                auto_fixable: false,
            }),
            Code::SoundInvalidChannelCount if note.message.contains("MCA") => Some(FixSuggestion {
                related_code: Code::SoundInvalidChannelCount,
                description: "Add MCA channel labeling metadata to sound MXF".into(),
                command: String::new(),
                auto_fixable: false,
            }),
            Code::EncryptionDetected => Some(FixSuggestion {
                related_code: Code::EncryptionDetected,
                description: "Obtain a valid KDM from the content distributor".into(),
                command: String::new(),
                auto_fixable: false,
            }),
            Code::MarkerMissing => Some(FixSuggestion {
                related_code: Code::MarkerMissing,
                description: "Add required markers to CPL (FFOC, LFOC, FFMC, LFMC)".into(),
                command: String::new(),
                auto_fixable: false,
            }),
            Code::SubtitleInvalidTiming => Some(FixSuggestion {
                related_code: Code::SubtitleInvalidTiming,
                description: "Fix subtitle timing: ensure TimeIn < TimeOut within reel duration"
                    .into(),
                command: String::new(),
                auto_fixable: false,
            }),
            Code::CplInvalidContentKind => Some(FixSuggestion {
                related_code: Code::CplInvalidContentKind,
                description: "Normalize ContentKind to lowercase SMPTE value".into(),
                command: String::new(),
                auto_fixable: true,
            }),
            _ => None,
        };

        if let Some(s) = suggestion {
            // Avoid duplicate suggestions
            if !suggestions.iter().any(|existing: &FixSuggestion| {
                existing.related_code == s.related_code && existing.description == s.description
            }) {
                suggestions.push(s);
            }
        }
    }

    suggestions
}
