//! HFR (High Frame Rate) and stereoscopic 3D validation.

use std::path::Path;

use serde::Serialize;

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

/// Multi-CPL entry info.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CplEntry {
    pub id: String,
    pub content_title: String,
    pub edit_rate: String,
    pub reel_count: u32,
    pub cpl_type: String,
}

/// Multi-CPL analysis result.
#[derive(Debug, Clone, Default, Serialize)]
pub struct MultiCplInfo {
    pub cpls: Vec<CplEntry>,
    pub consistent_frame_rate: bool,
}

/// Analyze multiple CPLs in a DCP directory.
pub fn analyze_multi_cpl(dcp_dir: &Path) -> MultiCplInfo {
    let mut info = MultiCplInfo::default();
    let mut frame_rates = std::collections::HashSet::new();

    let Ok(entries) = std::fs::read_dir(dcp_dir) else {
        return info;
    };

    let id_re = regex_lite::Regex::new(r"<Id>([^<]+)</Id>").unwrap();
    let title_re = regex_lite::Regex::new(r"<ContentTitleText>([^<]+)</ContentTitleText>").unwrap();
    let rate_re = regex_lite::Regex::new(r"<EditRate>(\d+\s+\d+)</EditRate>").unwrap();
    let reel_re = regex_lite::Regex::new(r"<Reel>").unwrap();
    let seg_re = regex_lite::Regex::new(r"<Segment>").unwrap();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().is_none_or(|ext| ext != "xml") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        if !content.contains("CompositionPlaylist") {
            continue;
        }

        let mut cpl = CplEntry::default();
        if let Some(cap) = id_re.captures(&content) {
            cpl.id = cap[1].to_string();
        }
        if let Some(cap) = title_re.captures(&content) {
            cpl.content_title = cap[1].to_string();
        }
        if let Some(cap) = rate_re.captures(&content) {
            cpl.edit_rate = cap[1].to_string();
            let parts: Vec<&str> = cap[1].split_whitespace().collect();
            if parts.len() == 2 {
                let num: f64 = parts[0].parse().unwrap_or(0.0);
                let den: f64 = parts[1].parse().unwrap_or(1.0);
                if den > 0.0 {
                    frame_rates.insert(((num / den) * 1000.0) as u64);
                }
            }
        }

        cpl.reel_count =
            reel_re.find_iter(&content).count() as u32 + seg_re.find_iter(&content).count() as u32;

        let title_lower = cpl.content_title.to_lowercase();
        cpl.cpl_type = if title_lower.contains("trailer") {
            "trailer".into()
        } else if title_lower.contains("advert") {
            "advertisement".into()
        } else if title_lower.contains("test") {
            "test".into()
        } else {
            "main".into()
        };

        info.cpls.push(cpl);
    }

    info.consistent_frame_rate = frame_rates.len() <= 1;
    info
}

/// Stereoscopic 3D analysis result.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Stereo3dInfo {
    pub is_stereo: bool,
    pub has_stereo_metadata: bool,
    pub stereo_type: String,
    pub left_duration: u64,
    pub right_duration: u64,
    pub eyes_aligned: bool,
    pub eye_offset: i64,
}

/// Analyze stereo 3D content in a CPL.
pub fn analyze_stereo3d(cpl_path: &Path) -> Stereo3dInfo {
    let mut info = Stereo3dInfo::default();

    let content = match std::fs::read_to_string(cpl_path) {
        Ok(c) => c,
        Err(_) => return info,
    };

    if content.contains("MainStereoscopicPicture") {
        info.is_stereo = true;
        info.has_stereo_metadata = true;
        info.stereo_type = "frame-sequential".into();

        let dur_re =
            regex_lite::Regex::new(r"<MainStereoscopicPicture>[\s\S]*?<Duration>(\d+)</Duration>")
                .unwrap();
        for cap in dur_re.captures_iter(&content) {
            let d: u64 = cap[1].parse().unwrap_or(0);
            info.left_duration += d / 2;
            info.right_duration += d / 2;
        }

        info.eye_offset = info.left_duration as i64 - info.right_duration as i64;
        info.eyes_aligned = info.eye_offset == 0;
    }

    info
}

/// Check stereo 3D compliance.
pub fn check_stereo3d_compliance(info: &Stereo3dInfo, cpl_path: &Path) -> Vec<Note> {
    let mut notes = Vec::new();
    if !info.is_stereo {
        return notes;
    }

    let path_buf = Some(cpl_path.to_path_buf());

    if !info.eyes_aligned {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::StereoMismatch,
            message: format!(
                "3D eye alignment error: {} frame offset between left and right eyes",
                info.eye_offset
            ),
            file: path_buf.clone(),
            line: 0,
        });
    }

    if !info.has_stereo_metadata {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::StereoMismatch,
            message: "3D content missing stereoscopic metadata".into(),
            file: path_buf.clone(),
            line: 0,
        });
    }

    notes.push(Note {
        severity: Severity::Info,
        code: Code::StereoMismatch,
        message: format!(
            "3D stereoscopic content ({}): L={} R={} frames",
            info.stereo_type, info.left_duration, info.right_duration
        ),
        file: path_buf,
        line: 0,
    });

    notes
}

/// CPL version chain entry.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CplChainEntry {
    pub cpl_id: String,
    pub content_title: String,
    pub content_version_id: String,
    pub content_version_label: String,
    pub is_supplemental: bool,
    pub original_cpl_id: String,
}

/// Trace CPL version chain in a DCP directory.
pub fn trace_cpl_chain(dcp_dir: &Path) -> Vec<CplChainEntry> {
    let mut chain = Vec::new();

    let Ok(entries) = std::fs::read_dir(dcp_dir) else {
        return chain;
    };

    let id_re = regex_lite::Regex::new(r"<Id>([^<]+)</Id>").unwrap();
    let title_re = regex_lite::Regex::new(r"<ContentTitleText>([^<]+)</ContentTitleText>").unwrap();
    let ver_re = regex_lite::Regex::new(r"<VersionNumber>([^<]+)</VersionNumber>").unwrap();
    let label_re = regex_lite::Regex::new(r"<LabelText>([^<]+)</LabelText>").unwrap();
    let opl_re =
        regex_lite::Regex::new(r"<OriginalPackageList>([^<]+)</OriginalPackageList>").unwrap();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().is_none_or(|ext| ext != "xml") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        if !content.contains("CompositionPlaylist") {
            continue;
        }

        let mut cpl = CplChainEntry::default();
        if let Some(cap) = id_re.captures(&content) {
            cpl.cpl_id = cap[1].to_string();
        }
        if let Some(cap) = title_re.captures(&content) {
            cpl.content_title = cap[1].to_string();
        }
        if let Some(cap) = ver_re.captures(&content) {
            cpl.content_version_id = cap[1].to_string();
        }
        if let Some(cap) = label_re.captures(&content) {
            cpl.content_version_label = cap[1].to_string();
        }
        if let Some(cap) = opl_re.captures(&content) {
            cpl.is_supplemental = true;
            cpl.original_cpl_id = cap[1].to_string();
        }

        chain.push(cpl);
    }

    chain
}

/// Check CPL chain for issues.
pub fn check_cpl_chain(chain: &[CplChainEntry], dcp_dir: &Path) -> Vec<Note> {
    let mut notes = Vec::new();

    let supplemental_count = chain.iter().filter(|e| e.is_supplemental).count();

    if supplemental_count > 0 {
        let path_buf = Some(dcp_dir.to_path_buf());

        notes.push(Note {
            severity: Severity::Info,
            code: Code::SupplementalOplMissing,
            message: format!(
                "DCP contains {supplemental_count} supplemental CPL(s) in version chain"
            ),
            file: path_buf.clone(),
            line: 0,
        });

        let known_ids: std::collections::HashSet<&str> =
            chain.iter().map(|e| e.cpl_id.as_str()).collect();

        for entry in chain {
            if entry.is_supplemental
                && !entry.original_cpl_id.is_empty()
                && !known_ids.contains(entry.original_cpl_id.as_str())
            {
                notes.push(Note {
                    severity: Severity::Warning,
                    code: Code::SupplementalOplMissing,
                    message: format!(
                        "Supplemental CPL references original {} not found in this package",
                        entry.original_cpl_id
                    ),
                    file: path_buf.clone(),
                    line: 0,
                });
            }
        }
    }

    notes
}
