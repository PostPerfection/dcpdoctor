//! ISDCF naming convention validation and generation.

use std::path::Path;

use crate::{Code, Note, Severity};

/// Check a content title against ISDCF naming conventions.
pub fn check_isdcf_naming(content_title: &str, cpl_path: &Path) -> Vec<Note> {
    let mut notes = Vec::new();

    if content_title.is_empty() {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::IsdcfNamingViolation,
            message: "Content title is empty".into(),
            file: Some(cpl_path.to_path_buf()),
            line: 0,
        });
        return notes;
    }

    let fields: Vec<&str> = content_title.split('_').collect();

    if fields.len() < 2 {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::IsdcfNamingViolation,
            message: format!(
                "Content title does not follow ISDCF naming convention \
                 (expected underscore-separated fields): {content_title}"
            ),
            file: Some(cpl_path.to_path_buf()),
            line: 0,
        });
        return notes;
    }

    // Field 1: Film title (max 14 chars recommended)
    if fields[0].is_empty() {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::IsdcfNamingViolation,
            message: "Film title field is empty".into(),
            file: Some(cpl_path.to_path_buf()),
            line: 0,
        });
    } else if fields[0].len() > 14 {
        notes.push(Note {
            severity: Severity::Info,
            code: Code::IsdcfNamingViolation,
            message: format!(
                "Film title exceeds 14 character recommendation: {}",
                fields[0]
            ),
            file: Some(cpl_path.to_path_buf()),
            line: 0,
        });
    }

    // Field 2: Content type
    if fields.len() >= 2 {
        const VALID_TYPES: &[&str] = &[
            "FTR", "TLR", "TSR", "PRO", "TST", "RTG", "SHR", "ADV", "XSN", "PSA", "POL", "CLT",
        ];
        if !VALID_TYPES.contains(&fields[1]) {
            notes.push(Note {
                severity: Severity::Warning,
                code: Code::IsdcfNamingViolation,
                message: format!(
                    "Non-standard content type: {} (expected FTR, TLR, TSR, PRO, TST, etc.)",
                    fields[1]
                ),
                file: Some(cpl_path.to_path_buf()),
                line: 0,
            });
        }
    }

    // Field 3: Aspect ratio
    if fields.len() >= 3 {
        const VALID_ASPECTS: &[&str] = &[
            "F", "S", "C", "F-133", "F-137", "F-138", "F-165", "F-166", "F-178", "F-185", "F-190",
            "F-200", "F-220", "F-239", "S-185", "S-239", "C-185", "C-239",
        ];
        if !VALID_ASPECTS.contains(&fields[2]) {
            notes.push(Note {
                severity: Severity::Info,
                code: Code::IsdcfNamingViolation,
                message: format!("Non-standard aspect ratio field: {}", fields[2]),
                file: Some(cpl_path.to_path_buf()),
                line: 0,
            });
        }
    }

    // Field 4: Language (2-3 uppercase letters, optionally hyphenated)
    if fields.len() >= 4 && !fields[3].is_empty() {
        let valid_lang = fields[3].split('-').all(|part| {
            part.len() >= 2 && part.len() <= 3 && part.chars().all(|c| c.is_ascii_uppercase())
        });
        if !valid_lang {
            notes.push(Note {
                severity: Severity::Info,
                code: Code::IsdcfNamingViolation,
                message: format!("Non-standard language field: {}", fields[3]),
                file: Some(cpl_path.to_path_buf()),
                line: 0,
            });
        }
    }

    // Check for resolution field (2K/4K)
    if fields.len() >= 6 && !fields.iter().any(|f| *f == "2K" || *f == "4K") {
        notes.push(Note {
            severity: Severity::Info,
            code: Code::IsdcfNamingViolation,
            message: "No resolution field (2K/4K) found in title".into(),
            file: Some(cpl_path.to_path_buf()),
            line: 0,
        });
    }

    notes
}

/// Parameters for ISDCF name generation.
pub struct IsdcfNameParams {
    pub film_title: String,
    pub content_type: String,
    pub aspect_ratio: String,
    pub is_3d: bool,
    pub language: String,
    pub territory: String,
    pub audio_type: String,
    pub resolution: String,
    pub studio: String,
    pub date: String,
    pub facility: String,
    pub standard: String,
    pub package_type: String,
    pub luminance: String,
    pub frame_rate: String,
}

/// Generate an ISDCF-compliant content title from parameters.
pub fn generate_isdcf_name(params: &IsdcfNameParams) -> String {
    let mut name = String::new();

    // Field 1: Film title (truncate to 14, remove spaces/underscores)
    let title: String = params
        .film_title
        .chars()
        .filter(|c| *c != ' ' && *c != '_')
        .take(14)
        .collect();
    name.push_str(&title);

    // Field 2: Content type
    name.push('_');
    name.push_str(&params.content_type);

    // Field 3: Aspect ratio
    name.push('_');
    name.push_str(&params.aspect_ratio);
    if params.is_3d {
        name.push_str("-3D");
    }

    // Field 4: Language
    name.push('_');
    name.push_str(&params.language);

    // Field 5: Territory
    name.push('_');
    name.push_str(&params.territory);

    // Field 6: Audio type
    name.push('_');
    name.push_str(&params.audio_type);

    // Field 7: Resolution
    name.push('_');
    name.push_str(&params.resolution);

    // Field 8: Studio
    if !params.studio.is_empty() {
        name.push('_');
        name.push_str(&params.studio);
    }

    // Field 9: Date
    name.push('_');
    if params.date.is_empty() {
        let now = time::OffsetDateTime::now_utc();
        name.push_str(&format!(
            "{:04}{:02}{:02}",
            now.year(),
            now.month() as u8,
            now.day()
        ));
    } else {
        name.push_str(&params.date);
    }

    // Field 10: Facility
    if !params.facility.is_empty() {
        name.push('_');
        name.push_str(&params.facility);
    }

    // Field 11: Standard
    name.push('_');
    name.push_str(&params.standard);

    // Field 12: Package type
    name.push('_');
    name.push_str(&params.package_type);

    // Optional: luminance
    if !params.luminance.is_empty() {
        name.push('_');
        name.push_str(&params.luminance);
    }

    // Optional: frame rate
    if !params.frame_rate.is_empty() {
        name.push('_');
        name.push_str(&params.frame_rate);
    }

    name
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_empty_title() {
        let notes = check_isdcf_naming("", &PathBuf::from("CPL.xml"));
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].severity, Severity::Warning);
    }

    #[test]
    fn test_single_field() {
        let notes = check_isdcf_naming("MyMovie", &PathBuf::from("CPL.xml"));
        assert_eq!(notes.len(), 1);
        assert!(notes[0].message.contains("underscore-separated"));
    }

    #[test]
    fn test_valid_isdcf() {
        let notes = check_isdcf_naming(
            "MyMovie_FTR_F_EN_US_51_2K_ST_20230101_FAC_SMPTE_OV",
            &PathBuf::from("CPL.xml"),
        );
        // Should pass with no warnings (only possible infos)
        assert!(notes.iter().all(|n| n.severity != Severity::Error));
    }

    #[test]
    fn test_bad_content_type() {
        let notes = check_isdcf_naming("Movie_XXX_F", &PathBuf::from("CPL.xml"));
        assert!(
            notes
                .iter()
                .any(|n| n.message.contains("Non-standard content type"))
        );
    }

    #[test]
    fn test_long_title() {
        let notes = check_isdcf_naming("AVeryLongMovieTitle_FTR_F", &PathBuf::from("CPL.xml"));
        assert!(notes.iter().any(|n| n.message.contains("14 character")));
    }

    #[test]
    fn test_generate_name() {
        let params = IsdcfNameParams {
            film_title: "My Movie".into(),
            content_type: "FTR".into(),
            aspect_ratio: "F".into(),
            is_3d: false,
            language: "EN".into(),
            territory: "US".into(),
            audio_type: "51".into(),
            resolution: "2K".into(),
            studio: "ST".into(),
            date: "20240101".into(),
            facility: "FAC".into(),
            standard: "SMPTE".into(),
            package_type: "OV".into(),
            luminance: String::new(),
            frame_rate: String::new(),
        };
        let name = generate_isdcf_name(&params);
        assert_eq!(name, "MyMovie_FTR_F_EN_US_51_2K_ST_20240101_FAC_SMPTE_OV");
    }

    #[test]
    fn test_generate_name_3d() {
        let params = IsdcfNameParams {
            film_title: "Movie".into(),
            content_type: "FTR".into(),
            aspect_ratio: "S".into(),
            is_3d: true,
            language: "EN".into(),
            territory: "GB".into(),
            audio_type: "71".into(),
            resolution: "4K".into(),
            studio: String::new(),
            date: "20240601".into(),
            facility: String::new(),
            standard: "SMPTE".into(),
            package_type: "OV".into(),
            luminance: String::new(),
            frame_rate: String::new(),
        };
        let name = generate_isdcf_name(&params);
        assert!(name.contains("S-3D"));
    }
}
