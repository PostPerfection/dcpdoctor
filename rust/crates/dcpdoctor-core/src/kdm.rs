/// KDM (Key Delivery Message) parsing and validation.
use crate::{Code, Note};
use asdcplib::WriterInfo;
use asdcplib::crypto::{AesDecContext, HmacContext};
use postkit::certificate::{UnwrappedKdm, unwrap_kdm};
use serde::{Deserialize, Serialize};
use std::path::Path;
use time::OffsetDateTime;

/// Content keys unwrapped from a KDM, used to decrypt essence during verify.
/// Holds secret key material (zeroed on drop by the inner `UnwrappedKdm`); never
/// log it. `none()` means no KDM was supplied, so encrypted essence stays skipped.
pub struct ContentKeys {
    kdm: Option<UnwrappedKdm>,
}

impl ContentKeys {
    pub fn none() -> Self {
        Self { kdm: None }
    }

    /// Unwrap a KDM with the recipient private key. Fails loud on a wrong key or
    /// malformed KDM so the caller surfaces a clear error, not garbage findings.
    pub fn from_kdm(kdm_file: &Path, recipient_key: &Path) -> Result<Self, String> {
        let xml = std::fs::read_to_string(kdm_file)
            .map_err(|e| format!("cannot read KDM {}: {e}", kdm_file.display()))?;
        let kdm = unwrap_kdm(&xml, recipient_key)?;
        Ok(Self { kdm: Some(kdm) })
    }

    pub fn has_kdm(&self) -> bool {
        self.kdm.is_some()
    }

    fn key(&self, key_id: &uuid::Uuid) -> Option<[u8; 16]> {
        self.kdm
            .as_ref()
            .and_then(|k| k.content_key(key_id))
            .copied()
    }

    /// Resolve one essence's encryption state against the available keys.
    pub fn resolve(&self, info: &WriterInfo) -> EssenceKey {
        if !info.encrypted_essence {
            return EssenceKey::Cleartext;
        }
        let key_id = uuid::Uuid::from_bytes(info.cryptographic_key_id);
        match self.key(&key_id) {
            Some(key) => EssenceKey::Available {
                key,
                label_set: info.label_set,
            },
            None => EssenceKey::Missing {
                key_id,
                had_kdm: self.has_kdm(),
            },
        }
    }
}

/// How a single essence's encryption resolves against the content keys.
pub enum EssenceKey {
    /// cleartext essence: read with no crypto context
    Cleartext,
    /// encrypted essence whose content key we hold
    Available {
        key: [u8; 16],
        label_set: asdcplib::LabelSet,
    },
    /// encrypted essence with no usable key (no KDM, or the KDM lacks this KeyId)
    Missing { key_id: uuid::Uuid, had_kdm: bool },
}

/// AES + HMAC contexts for decrypting and integrity-checking one essence.
pub struct DecryptContexts {
    pub dec: AesDecContext,
    pub hmac: HmacContext,
}

impl EssenceKey {
    /// Build the crypto contexts for a decryptable essence. `Cleartext` and
    /// `Missing` yield `None` (read without contexts, or skip).
    pub fn contexts(&self) -> Result<Option<DecryptContexts>, String> {
        match self {
            EssenceKey::Available { key, label_set } => {
                let mut dec = AesDecContext::new();
                dec.init_key(key)
                    .map_err(|e| format!("AES key init failed: {e}"))?;
                let mut hmac = HmacContext::new();
                hmac.init_key(key, *label_set)
                    .map_err(|e| format!("HMAC key init failed: {e}"))?;
                Ok(Some(DecryptContexts { dec, hmac }))
            }
            _ => Ok(None),
        }
    }

    /// true for encrypted essence we can't read (no key).
    pub fn is_missing(&self) -> bool {
        matches!(self, EssenceKey::Missing { .. })
    }

    /// A skip note when a KDM was supplied but doesn't cover this essence's
    /// KeyId. Without a KDM, encrypted essence skips silently (returns `None`),
    /// keeping the pre-KDM behavior.
    pub fn skip_note(&self, file: &Path) -> Option<Note> {
        match self {
            EssenceKey::Missing {
                key_id,
                had_kdm: true,
            } => Some(
                Note::warning(
                    Code::KdmRequired,
                    format!(
                        "KDM does not carry the content key for KeyId {key_id}; encrypted essence checks skipped"
                    ),
                )
                .with_file(file),
            ),
            _ => None,
        }
    }
}

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
                            Code::KdmNotYetValid,
                            format!("KDM is not yet valid (starts {})", info.not_valid_before),
                        )
                        .with_file(kdm_path),
                    );
                }
                if now > end {
                    notes.push(
                        Note::error(
                            Code::KdmExpired,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_kdm(before: &str, after: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(
            f,
            r#"<?xml version="1.0"?>
<KeyDeliveryMessage>
  <CompositionPlaylistId>urn:uuid:96558952-39b8-42d3-825e-9ddd31298219</CompositionPlaylistId>
  <ContentKeysNotValidBefore>{before}</ContentKeysNotValidBefore>
  <ContentKeysNotValidAfter>{after}</ContentKeysNotValidAfter>
</KeyDeliveryMessage>"#
        )
        .unwrap();
        f
    }

    #[test]
    fn future_window_emits_not_yet_valid() {
        let f = write_kdm("2099-01-01T00:00:00+00:00", "2100-01-01T00:00:00+00:00");
        let notes = validate_kdm(f.path(), None);
        assert!(
            notes.iter().any(|n| n.code == Code::KdmNotYetValid),
            "got: {notes:?}"
        );
        assert!(!notes.iter().any(|n| n.code == Code::KdmExpired));
    }

    #[test]
    fn past_window_emits_expired() {
        let f = write_kdm("2000-01-01T00:00:00+00:00", "2001-01-01T00:00:00+00:00");
        let notes = validate_kdm(f.path(), None);
        assert!(
            notes.iter().any(|n| n.code == Code::KdmExpired),
            "got: {notes:?}"
        );
        assert!(!notes.iter().any(|n| n.code == Code::KdmNotYetValid));
    }
}

// End-to-end KDM decryption: build an encrypted picture MXF in-test, generate a
// KDM for it, and prove the encrypted-essence checks skip without a KDM, fire on
// decrypted essence with the right KDM, and fail loud on a wrong key / bad MIC.
#[cfg(test)]
mod decrypt_tests {
    use super::*;
    use postkit::certificate::{KdmConfig, KdmContentKey, build_kdm, generate_chain};
    use std::path::PathBuf;

    // A 2K picture codestream with `guard_bits` in its QCD: SOC, QCD, SOT. Enough
    // for check_guard_bits_mxf (which reads width from the descriptor, guard bits
    // from the QCD). 2K expects 1 guard bit, so gb=0 is a planted violation.
    fn codestream(guard_bits: u8) -> Vec<u8> {
        let mut d = vec![0xFF, 0x4F]; // SOC
        d.extend_from_slice(&[0xFF, 0x5C]); // QCD
        d.extend_from_slice(&[0x00, 0x04]); // Lqcd
        d.push(guard_bits << 5); // Sqcd: guard bits in top 3 bits
        d.push(0x00); // one SPqcd byte
        d.extend_from_slice(&[0xFF, 0x90]); // SOT
        // pad so the AES-CBC essence packet has real body bytes to encrypt; the
        // guard-bit walk stops at SOT so trailing zeros are ignored.
        d.resize(4096, 0);
        d
    }

    // Write a single-frame encrypted 2K picture MXF with the given content key and
    // key id, guarding it with an HMAC (MIC) the same way asdcplib wraps a DCP.
    fn write_encrypted_mxf(path: &Path, key_id: uuid::Uuid, content_key: [u8; 16], gb: u8) {
        use asdcplib::crypto::{AesEncContext, HmacContext};
        use asdcplib::jp2k::{MxfWriter, PictureDescriptor};
        use asdcplib::{LabelSet, Rational, WriterInfo};

        let info = WriterInfo {
            asset_uuid: *uuid::Uuid::new_v4().as_bytes(),
            context_id: *uuid::Uuid::new_v4().as_bytes(),
            cryptographic_key_id: *key_id.as_bytes(),
            encrypted_essence: true,
            uses_hmac: true,
            label_set: LabelSet::Smpte,
            ..Default::default()
        };
        let desc = PictureDescriptor {
            edit_rate: Rational::new(24, 1),
            sample_rate: Rational::new(24, 1),
            stored_width: 2048,
            stored_height: 1080,
            aspect_ratio: Rational::new(2048, 1080),
            container_duration: 1,
            component_count: 3,
        };
        let mut enc = AesEncContext::new();
        enc.init_key(&content_key).unwrap();
        let mut hmac = HmacContext::new();
        hmac.init_key(&content_key, LabelSet::Smpte).unwrap();

        let mut w = MxfWriter::new();
        w.open_write(path.to_str().unwrap(), &info, &desc, 16_384)
            .unwrap();
        w.write_frame(&codestream(gb), Some(&mut enc), Some(&mut hmac))
            .unwrap();
        w.finalize().unwrap();
    }

    // Generate a cert chain and a KDM carrying `content_key` for `key_id`, bound to
    // the given (fake) CPL id. Returns (kdm_path, recipient_key, wrong_key, dir).
    fn make_kdm(
        key_id: uuid::Uuid,
        content_key: [u8; 16],
        cpl_id: &str,
    ) -> (PathBuf, PathBuf, PathBuf, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(generate_chain("Test", dir.path()), 0, "chain generation");
        let signer = dir.path().join("signer.pem");
        let signer_key = dir.path().join("signer.key");
        let root = dir.path().join("root.pem");
        let root_key = dir.path().join("root.key");

        let config = KdmConfig {
            cpl_id: cpl_id.to_string(),
            content_title: "Test".to_string(),
            recipient_cert_file: signer.clone(),
            signer_cert_file: root.clone(),
            signer_key_file: root_key.clone(),
            signer_chain_files: vec![],
            output_file: PathBuf::from("unused"),
            valid_from: "now".to_string(),
            valid_to: "7 days".to_string(),
            formulation: "dci-any".to_string(),
            content_keys: vec![KdmContentKey {
                key_type: *b"MDIK",
                key_id,
                content_key,
            }],
            ..Default::default()
        };
        let kdm = build_kdm(&config).expect("build kdm");
        let kdm_path = dir.path().join("kdm.xml");
        std::fs::write(&kdm_path, &kdm.xml).unwrap();
        // signer_key decrypts (recipient == signer); root_key is the wrong key.
        (kdm_path, signer_key, root_key, dir)
    }

    #[test]
    fn encrypted_essence_skips_without_a_kdm() {
        let dir = tempfile::tempdir().unwrap();
        let mxf = dir.path().join("pic.mxf");
        write_encrypted_mxf(&mxf, uuid::Uuid::new_v4(), [0x11; 16], 0);

        let notes = crate::j2k::check_guard_bits_mxf(&mxf, &ContentKeys::none());
        assert!(
            notes.is_empty(),
            "encrypted essence must skip without a KDM, got: {notes:?}"
        );
    }

    #[test]
    fn right_kdm_runs_checks_and_planted_violation_fires() {
        let key_id = uuid::Uuid::new_v4();
        let content_key = [0x22; 16];
        let cpl_id = "8a2b1c3d-4e5f-6071-8293-a4b5c6d7e8f9";
        let (kdm_path, recipient_key, _wrong, _certs) = make_kdm(key_id, content_key, cpl_id);

        let dir = tempfile::tempdir().unwrap();
        let mxf = dir.path().join("pic.mxf");
        // gb=0 for a 2K frame violates the RDD 52 guard-bit rule (expects 1).
        write_encrypted_mxf(&mxf, key_id, content_key, 0);

        let keys = ContentKeys::from_kdm(&kdm_path, &recipient_key).expect("unwrap kdm");
        let notes = crate::j2k::check_guard_bits_mxf(&mxf, &keys);
        assert!(
            notes.iter().any(|n| n.code == Code::J2kGuardBits),
            "planted guard-bit violation must fire on decrypted essence, got: {notes:?}"
        );
    }

    #[test]
    fn wrong_recipient_key_fails_loud() {
        let key_id = uuid::Uuid::new_v4();
        let content_key = [0x33; 16];
        let cpl_id = "8a2b1c3d-4e5f-6071-8293-a4b5c6d7e8f9";
        let (kdm_path, _recipient_key, wrong_key, _certs) = make_kdm(key_id, content_key, cpl_id);

        match ContentKeys::from_kdm(&kdm_path, &wrong_key) {
            Ok(_) => panic!("wrong recipient key must fail loud"),
            Err(e) => assert!(!e.is_empty(), "error must carry a message"),
        }
    }

    #[test]
    fn mismatched_content_key_fails_the_mic() {
        // right recipient key, but the KDM carries a content key that did not
        // encrypt this MXF (a KDM for a different DCP): decrypt yields garbage and
        // the HMAC/MIC check must fire rather than produce a bogus finding.
        let key_id = uuid::Uuid::new_v4();
        let cpl_id = "8a2b1c3d-4e5f-6071-8293-a4b5c6d7e8f9";
        let (kdm_path, recipient_key, _wrong, _certs) = make_kdm(key_id, [0xAB; 16], cpl_id);

        let dir = tempfile::tempdir().unwrap();
        let mxf = dir.path().join("pic.mxf");
        // MXF encrypted with a different content key than the KDM carries.
        write_encrypted_mxf(&mxf, key_id, [0xCD; 16], 1);

        let keys = ContentKeys::from_kdm(&kdm_path, &recipient_key).expect("unwrap kdm");
        let notes = crate::j2k::check_guard_bits_mxf(&mxf, &keys);
        assert!(
            notes.iter().any(|n| n.code == Code::MxfHashMismatch),
            "a mismatched content key must fail the MIC, got: {notes:?}"
        );
    }
}
