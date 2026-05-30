//! Advanced KDM/DKDM parsing, validation, and trusted device list management.

use std::path::Path;

use serde::Serialize;

use crate::{Code, Note, Severity};

/// Parsed DKDM information.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DkdmInfo {
    pub valid: bool,
    pub error: String,
    pub cpl_id: String,
    pub content_title: String,
    pub issuer: String,
    pub recipient: String,
    pub not_valid_before: String,
    pub not_valid_after: String,
    pub is_dkdm: bool,
}

/// A trusted device list entry.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TdlEntry {
    pub thumbprint: String,
    pub common_name: String,
    pub organization: String,
}

/// Parsed KDM annotation fields.
#[derive(Debug, Clone, Default, Serialize)]
pub struct KdmAnnotation {
    pub facility_name: String,
    pub screen_name: String,
    pub content_title: String,
    pub valid_from: String,
    pub valid_to: String,
    pub valid_format: bool,
}

/// Parse a DKDM/KDM XML file.
pub fn parse_dkdm(dkdm_path: &Path) -> DkdmInfo {
    let mut info = DkdmInfo::default();

    let content = match std::fs::read_to_string(dkdm_path) {
        Ok(c) => c,
        Err(_) => {
            info.error = "Failed to read DKDM file".into();
            return info;
        }
    };

    if !content.contains("DCinemaSecurityMessage") {
        info.error = "Not a KDM/DKDM file".into();
        return info;
    }

    info.cpl_id = extract_tag(&content, "CompositionPlaylistId").unwrap_or_default();
    info.content_title = extract_tag(&content, "ContentTitleText").unwrap_or_default();
    info.issuer = extract_tag(&content, "X509IssuerName").unwrap_or_default();
    info.recipient = extract_tag(&content, "X509SerialNumber").unwrap_or_default();
    info.not_valid_before = extract_tag(&content, "ContentKeysNotValidBefore").unwrap_or_default();
    info.not_valid_after = extract_tag(&content, "ContentKeysNotValidAfter").unwrap_or_default();

    // DKDMs don't have DeviceListIdentifier
    info.is_dkdm = !content.contains("DeviceListIdentifier");

    info.valid = true;
    info
}

/// Validate a DKDM for structural correctness and expiration.
pub fn validate_dkdm(dkdm_path: &Path) -> Vec<Note> {
    let mut notes = Vec::new();

    let info = parse_dkdm(dkdm_path);
    if !info.valid {
        notes.push(Note {
            severity: Severity::Error,
            code: Code::EncryptionDetected,
            message: format!("Invalid DKDM: {}", info.error),
            file: Some(dkdm_path.to_path_buf()),
            line: 0,
        });
        return notes;
    }

    if !info.is_dkdm {
        notes.push(Note {
            severity: Severity::Info,
            code: Code::EncryptionDetected,
            message: "File is a KDM (not a DKDM) — has DeviceListIdentifier".into(),
            file: Some(dkdm_path.to_path_buf()),
            line: 0,
        });
    }

    // Check expiration using simple string comparison (ISO 8601 sorts lexicographically)
    if !info.not_valid_after.is_empty() {
        let now = {
            let t = time::OffsetDateTime::now_utc();
            format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
                t.year(),
                t.month() as u8,
                t.day(),
                t.hour(),
                t.minute(),
                t.second()
            )
        };
        if now > info.not_valid_after {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::KdmExpired,
                message: "DKDM has expired".into(),
                file: Some(dkdm_path.to_path_buf()),
                line: 0,
            });
        }
    }

    notes
}

/// Load a trusted device list from XML.
pub fn load_trusted_device_list(tdl_path: &Path) -> Vec<TdlEntry> {
    let content = match std::fs::read_to_string(tdl_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut entries = Vec::new();
    let device_re =
        regex_lite::Regex::new(r"<(?:TrustedDevice|Device)>([\s\S]*?)</(?:TrustedDevice|Device)>")
            .unwrap();

    for cap in device_re.captures_iter(&content) {
        let block = cap.get(1).unwrap().as_str();
        let thumbprint = extract_tag(block, "CertificateThumbprint")
            .or_else(|| extract_tag(block, "Thumbprint"))
            .unwrap_or_default();

        if thumbprint.is_empty() {
            continue;
        }

        entries.push(TdlEntry {
            thumbprint,
            common_name: extract_tag(block, "CommonName")
                .or_else(|| extract_tag(block, "DeviceName"))
                .unwrap_or_default(),
            organization: extract_tag(block, "Organization").unwrap_or_default(),
        });
    }

    entries
}

/// Validate a KDM against a trusted device list.
pub fn validate_kdm_against_tdl(kdm_path: &Path, tdl: &[TdlEntry]) -> Vec<Note> {
    let mut notes = Vec::new();

    if tdl.is_empty() {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::EncryptionDetected,
            message: "Trusted Device List is empty".into(),
            file: Some(kdm_path.to_path_buf()),
            line: 0,
        });
        return notes;
    }

    let content = match std::fs::read_to_string(kdm_path) {
        Ok(c) => c,
        Err(_) => {
            notes.push(Note {
                severity: Severity::Error,
                code: Code::EncryptionDetected,
                message: "Cannot read KDM for TDL validation".into(),
                file: Some(kdm_path.to_path_buf()),
                line: 0,
            });
            return notes;
        }
    };

    let recipient_serial = extract_tag(&content, "X509SerialNumber").unwrap_or_default();
    if recipient_serial.is_empty() {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::EncryptionDetected,
            message: "KDM has no recipient certificate serial number".into(),
            file: Some(kdm_path.to_path_buf()),
            line: 0,
        });
        return notes;
    }

    let found = tdl
        .iter()
        .any(|entry| entry.thumbprint == recipient_serial || entry.common_name == recipient_serial);

    if !found {
        notes.push(Note {
            severity: Severity::Warning,
            code: Code::EncryptionDetected,
            message: "KDM recipient not found in Trusted Device List".into(),
            file: Some(kdm_path.to_path_buf()),
            line: 0,
        });
    }

    notes
}

/// Parse a KDM annotation text into structured fields.
pub fn parse_kdm_annotation(annotation_text: &str) -> KdmAnnotation {
    let parts: Vec<&str> = annotation_text
        .split(['_', '.'])
        .filter(|s| !s.is_empty())
        .collect();

    if parts.len() >= 4 {
        KdmAnnotation {
            facility_name: parts[0].to_string(),
            screen_name: parts[1].to_string(),
            content_title: parts[2].to_string(),
            valid_from: parts.get(3).unwrap_or(&"").to_string(),
            valid_to: parts.get(4).unwrap_or(&"").to_string(),
            valid_format: true,
        }
    } else if parts.len() >= 2 {
        KdmAnnotation {
            content_title: parts[0].to_string(),
            facility_name: parts.get(1).unwrap_or(&"").to_string(),
            valid_format: false,
            ..Default::default()
        }
    } else {
        KdmAnnotation::default()
    }
}

fn extract_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].trim().to_string())
}
