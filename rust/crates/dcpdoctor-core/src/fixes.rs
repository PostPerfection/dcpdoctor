//! Fix suggestions and auto-repair for common DCP issues.

use std::path::Path;

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

/// Apply auto-fixable repairs to a DCP directory.
pub fn apply_fixes(dcp_dir: &Path, suggestions: &[FixSuggestion]) -> u32 {
    let mut applied = 0;

    for fix in suggestions {
        if !fix.auto_fixable {
            continue;
        }

        match fix.related_code {
            Code::SmpteNamingViolation if fix.command == "mv ASSETMAP ASSETMAP.xml" => {
                let src = dcp_dir.join("ASSETMAP");
                let dst = dcp_dir.join("ASSETMAP.xml");
                if src.exists() && !dst.exists() && std::fs::rename(&src, &dst).is_ok() {
                    applied += 1;
                }
            }
            Code::SmpteNamespaceWrong => {
                applied += fix_namespaces(dcp_dir);
            }
            Code::PklHashMismatch => {
                applied += fix_pkl_hashes(dcp_dir);
            }
            Code::CplInvalidContentKind => {
                applied += fix_content_kind(dcp_dir);
            }
            _ => {}
        }
    }

    applied
}

/// Replace Interop namespaces with SMPTE in all XML files.
pub fn fix_namespaces(dcp_dir: &Path) -> u32 {
    let mut fixed = 0;
    let interop_cpl_ns = "http://www.digicine.com/PROTO-ASDCP-CPL-20040511#";
    let smpte_cpl_ns = "http://www.smpte-ra.org/schemas/429-7/2006/CPL";

    let Ok(entries) = std::fs::read_dir(dcp_dir) else {
        return 0;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().unwrap_or_default() != "xml" {
            continue;
        }

        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };

        if content.contains(interop_cpl_ns) {
            let updated = content.replace(interop_cpl_ns, smpte_cpl_ns);
            if std::fs::write(&path, &updated).is_ok() {
                fixed += 1;
            }
        }
    }

    fixed
}

/// Recompute PKL hashes from actual files and update the PKL XML.
pub fn fix_pkl_hashes(dcp_dir: &Path) -> u32 {
    let mut fixed = 0;

    let Ok(entries) = std::fs::read_dir(dcp_dir) else {
        return 0;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().unwrap_or_default() != "xml" {
            continue;
        }

        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };

        if !content.contains("PackingList") {
            continue;
        }

        // Find Hash elements and recompute
        let asset_re = regex_lite::Regex::new(
            r"<Asset>[\s\S]*?<OriginalFileName>([^<]+)</OriginalFileName>[\s\S]*?<Hash>([^<]+)</Hash>[\s\S]*?</Asset>",
        ).unwrap();

        let mut updated = content.clone();
        let mut modified = false;

        for cap in asset_re.captures_iter(&content) {
            let filename = &cap[1];
            let old_hash = &cap[2];
            let asset_path = dcp_dir.join(filename);

            if !asset_path.exists() {
                continue;
            }

            match postkit::hash::hash_file(&asset_path, postkit::hash::HashAlgorithm::Sha1) {
                Ok(result) => {
                    if result.base64 != old_hash {
                        updated = updated.replacen(old_hash, &result.base64, 1);
                        modified = true;
                        fixed += 1;
                    }
                }
                Err(_) => continue,
            }
        }

        if modified {
            let _ = std::fs::write(&path, &updated);
        }
    }

    fixed
}

/// Normalize ContentKind values to lowercase SMPTE standard.
pub fn fix_content_kind(dcp_dir: &Path) -> u32 {
    let mut fixed = 0;

    let kind_map: &[(&str, &str)] = &[
        ("Feature", "feature"),
        ("FEATURE", "feature"),
        ("Trailer", "trailer"),
        ("TRAILER", "trailer"),
        ("Test", "test"),
        ("TEST", "test"),
        ("Teaser", "teaser"),
        ("TEASER", "teaser"),
        ("Rating", "rating"),
        ("RATING", "rating"),
        ("Advertisement", "advertisement"),
        ("ADVERTISEMENT", "advertisement"),
        ("Short", "short"),
        ("SHORT", "short"),
        ("Transitional", "transitional"),
        ("PSA", "psa"),
        ("Policy", "policy"),
        ("Episode", "episode"),
    ];

    let Ok(entries) = std::fs::read_dir(dcp_dir) else {
        return 0;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().unwrap_or_default() != "xml" {
            continue;
        }

        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };

        let ck_re = regex_lite::Regex::new(r"<ContentKind>([^<]+)</ContentKind>").unwrap();
        let Some(cap) = ck_re.captures(&content) else {
            continue;
        };

        let original = cap[1].trim();
        let normalized = kind_map
            .iter()
            .find(|(k, _)| *k == original)
            .map(|(_, v)| *v)
            .unwrap_or_else(|| original);

        if normalized == original {
            continue;
        }

        let old_tag = format!("<ContentKind>{}</ContentKind>", &cap[1]);
        let new_tag = format!("<ContentKind>{normalized}</ContentKind>");
        let updated = content.replacen(&old_tag, &new_tag, 1);
        if std::fs::write(&path, &updated).is_ok() {
            fixed += 1;
        }
    }

    fixed
}
