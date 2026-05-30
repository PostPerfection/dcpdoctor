/// KDM (Key Delivery Message) parsing and validation.
use crate::{Code, Note};
use serde::{Deserialize, Serialize};
use std::path::Path;
use time::OffsetDateTime;

/// Parsed KDM information.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KdmInfo {
    pub cpl_id: String,
    pub content_title: String,
    pub not_valid_before: String,
    pub not_valid_after: String,
    pub recipient_cn: String,
    pub issuer: String,
    pub key_count: usize,
}

/// Parse a KDM XML file and extract key information.
pub fn parse_kdm(kdm_path: &Path) -> Result<KdmInfo, String> {
    let xml =
        std::fs::read_to_string(kdm_path).map_err(|e| format!("Failed to read KDM file: {e}"))?;

    if !xml.contains("KeyDeliveryMessage") && !xml.contains("KDM") {
        return Err("File does not appear to be a KDM".into());
    }

    let cpl_id = extract_element(&xml, "CompositionPlaylistId")
        .unwrap_or_default()
        .replace("urn:uuid:", "");

    let content_title = extract_element(&xml, "ContentTitleText").unwrap_or_default();

    let not_valid_before = extract_element(&xml, "ContentKeysNotValidBefore")
        .or_else(|| extract_element(&xml, "NotValidBefore"))
        .unwrap_or_default();
    let not_valid_after = extract_element(&xml, "ContentKeysNotValidAfter")
        .or_else(|| extract_element(&xml, "NotValidAfter"))
        .unwrap_or_default();

    let recipient_cn = extract_element(&xml, "X509SubjectName")
        .and_then(|s| {
            s.split(',')
                .find(|part| part.trim().starts_with("CN="))
                .map(|cn| cn.trim().trim_start_matches("CN=").to_string())
        })
        .unwrap_or_default();

    let issuer = extract_element(&xml, "X509IssuerName")
        .and_then(|s| {
            s.split(',')
                .find(|part| part.trim().starts_with("O="))
                .map(|o| o.trim().trim_start_matches("O=").to_string())
        })
        .unwrap_or_default();

    let mut key_count =
        xml.matches("<KeyId>").count() + xml.matches("<enc:CipherValue>").count().min(1);
    if key_count == 0 {
        key_count = xml.matches("CipherValue").count();
    }

    Ok(KdmInfo {
        cpl_id,
        content_title,
        not_valid_before,
        not_valid_after,
        recipient_cn,
        issuer,
        key_count,
    })
}

/// Validate a KDM file, optionally against a DCP.
pub fn validate_kdm(kdm_path: &Path, dcp_dir: Option<&Path>) -> Vec<Note> {
    let mut notes = Vec::new();

    let info = match parse_kdm(kdm_path) {
        Ok(i) => i,
        Err(e) => {
            notes.push(Note::error(Code::XmlParseError, e).with_file(kdm_path));
            return notes;
        }
    };

    // Check validity period
    if !info.not_valid_before.is_empty() && !info.not_valid_after.is_empty() {
        match (
            parse_iso_datetime(&info.not_valid_before),
            parse_iso_datetime(&info.not_valid_after),
        ) {
            (Some(start), Some(end)) => {
                let now = OffsetDateTime::now_utc();
                if now < start {
                    notes.push(
                        Note::warning(
                            Code::KdmRequired,
                            format!("KDM is not yet valid (starts {})", info.not_valid_before),
                        )
                        .with_file(kdm_path),
                    );
                }
                if now > end {
                    notes.push(
                        Note::error(
                            Code::KdmRequired,
                            format!("KDM has expired (ended {})", info.not_valid_after),
                        )
                        .with_file(kdm_path),
                    );
                }
                if end <= start {
                    notes.push(
                        Note::error(
                            Code::KdmRequired,
                            "KDM validity period is invalid (end <= start)".to_string(),
                        )
                        .with_file(kdm_path),
                    );
                }
            }
            _ => {
                notes.push(
                    Note::warning(
                        Code::KdmRequired,
                        "Could not parse KDM validity dates".to_string(),
                    )
                    .with_file(kdm_path),
                );
            }
        }
    }

    // Check CPL ID is present
    if info.cpl_id.is_empty() {
        notes.push(
            Note::warning(
                Code::KdmRequired,
                "KDM does not reference a CompositionPlaylistId".to_string(),
            )
            .with_file(kdm_path),
        );
    }

    // Cross-validate against DCP if provided
    if let Some(dcp_path) = dcp_dir
        && !info.cpl_id.is_empty()
    {
        // Check that the DCP contains a CPL with the referenced ID
        let cpl_found = find_cpl_id_in_dcp(dcp_path, &info.cpl_id);
        if !cpl_found {
            notes.push(
                Note::error(
                    Code::CrossRefBroken,
                    format!(
                        "KDM references CPL {} which is not present in the DCP",
                        info.cpl_id
                    ),
                )
                .with_file(kdm_path),
            );
        }
    }

    if notes.is_empty() {
        notes.push(
            Note::info(
                Code::EncryptionDetected,
                format!(
                    "KDM valid for '{}' (CPL {}, {} key(s), valid {} → {})",
                    info.content_title,
                    if info.cpl_id.is_empty() {
                        "unknown"
                    } else {
                        &info.cpl_id
                    },
                    info.key_count,
                    info.not_valid_before,
                    info.not_valid_after
                ),
            )
            .with_file(kdm_path),
        );
    }

    notes
}

fn extract_element(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    // Also try with namespace prefix
    let open_ns = format!(":{tag}>");
    let close_ns = format!(":/{tag}>");

    if let Some(start) = xml.find(&open) {
        let after = start + open.len();
        if let Some(end) = xml[after..].find(&close) {
            return Some(xml[after..after + end].trim().to_string());
        }
    }

    // Try namespace-prefixed version
    for line in xml.lines() {
        let trimmed = line.trim();
        if trimmed.contains(&open_ns) || trimmed.contains(&format!("<{tag}>")) {
            // Extract content between > and <
            if let Some(gt) = trimmed.find('>') {
                let rest = &trimmed[gt + 1..];
                if let Some(lt) = rest.find('<') {
                    let val = rest[..lt].trim();
                    if !val.is_empty() {
                        return Some(val.to_string());
                    }
                }
            }
        }
    }

    // Try without namespace
    let _ = (open_ns, close_ns);
    None
}

fn parse_iso_datetime(s: &str) -> Option<OffsetDateTime> {
    // Try RFC 3339 format directly
    if let Ok(dt) = OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339) {
        return Some(dt);
    }

    // Try without fractional seconds: 2024-01-15T00:00:00+00:00
    let cleaned = if s.contains('T') && !s.contains('+') && !s.contains('Z') {
        format!("{s}+00:00")
    } else if s.ends_with('Z') {
        s.replace('Z', "+00:00")
    } else {
        s.to_string()
    };

    OffsetDateTime::parse(&cleaned, &time::format_description::well_known::Rfc3339).ok()
}

fn find_cpl_id_in_dcp(dcp_dir: &Path, cpl_id: &str) -> bool {
    let target = cpl_id.to_lowercase();
    // Search all XML files for the CPL ID
    let entries = match std::fs::read_dir(dcp_dir) {
        Ok(e) => e,
        Err(_) => return false,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("xml")
            && let Ok(content) = std::fs::read_to_string(&path)
            && content.contains("CompositionPlaylist")
            && content.to_lowercase().contains(&target)
        {
            return true;
        }
    }
    false
}
