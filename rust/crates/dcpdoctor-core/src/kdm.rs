/// KDM (Key Delivery Message) parsing and validation.
use crate::{Code, Note};
use asdcplib::WriterInfo;
use asdcplib::crypto::{AesDecContext, HmacContext};
use base64::Engine;
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
    let xml = read_kdm(kdm_path)?;
    parse_kdm_xml(&xml)
}

fn read_kdm(kdm_path: &Path) -> Result<String, String> {
    std::fs::read_to_string(kdm_path).map_err(|e| format!("Failed to read KDM file: {e}"))
}

fn parse_kdm_xml(xml: &str) -> Result<KdmInfo, String> {
    if !xml.contains("KeyDeliveryMessage") && !xml.contains("KDM") {
        return Err("File does not appear to be a KDM".into());
    }

    let cpl_id = extract_element(xml, "CompositionPlaylistId")
        .unwrap_or_default()
        .replace("urn:uuid:", "");

    let content_title = extract_element(xml, "ContentTitleText").unwrap_or_default();

    let not_valid_before = extract_element(xml, "ContentKeysNotValidBefore")
        .or_else(|| extract_element(xml, "NotValidBefore"))
        .unwrap_or_default();
    let not_valid_after = extract_element(xml, "ContentKeysNotValidAfter")
        .or_else(|| extract_element(xml, "NotValidAfter"))
        .unwrap_or_default();

    let recipient_cn = extract_element(xml, "X509SubjectName")
        .and_then(|s| {
            s.split(',')
                .find(|part| part.trim().starts_with("CN="))
                .map(|cn| cn.trim().trim_start_matches("CN=").to_string())
        })
        .unwrap_or_default();

    let issuer = extract_element(xml, "X509IssuerName")
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

    // ST 430-1 Annex B makes several elements required that a hand-rolled writer
    // can omit and every other check here would still pass, so the XSD runs
    // first. Skips itself when the schema dir or xmllint is absent.
    if let Some(schema_dir) = crate::schema::locate_schema_dir() {
        notes.extend(crate::schema::check_schema(kdm_path, &schema_dir));
    }

    let xml = match read_kdm(kdm_path) {
        Ok(x) => x,
        Err(e) => {
            notes.push(Note::error(Code::XmlParseError, e).with_file(kdm_path));
            return notes;
        }
    };

    let info = match parse_kdm_xml(&xml) {
        Ok(i) => i,
        Err(e) => {
            notes.push(Note::error(Code::XmlParseError, e).with_file(kdm_path));
            return notes;
        }
    };

    notes.extend(check_digests(&xml, kdm_path));

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

/// ST 430-1:2006 element names carrying the digests checked below.
const DEVICE_LIST_TAG: &str = "DeviceList";
const CERTIFICATE_THUMBPRINT_TAG: &str = "CertificateThumbprint";
const CONTENT_AUTHENTICATOR_TAG: &str = "ContentAuthenticator";

/// A ST 430-2:2006 thumbprint is a SHA-1 digest, so 20 bytes once decoded. The
/// KDM schema types these elements `base64Binary` / `ds:DigestValueType`, which
/// fixes the encoding but not the length, so the XSD pass cannot catch this.
const SHA1_DIGEST_BYTES: usize = 20;

/// DCI DCSS 9.4.3.5: the base64 SHA-1 of empty input, used as the DeviceList
/// entry meaning the trusted-device requirement is already met.
const ASSUME_TRUST_THUMBPRINT: &str = "2jmj7l5rSw0yVb/vlWAYkK/YBwk=";

/// The base64 digests a KDM carries outside its signature.
#[derive(Debug, Default)]
struct KdmDigests {
    device_thumbprints: Vec<String>,
    content_authenticator: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum DigestElement {
    CertificateThumbprint,
    ContentAuthenticator,
}

/// Collect every `<CertificateThumbprint>` in the DeviceList plus the optional
/// `<ContentAuthenticator>`. Element names are matched on their local name, so
/// a namespace-prefixed document reads the same as a bare one.
fn extract_digests(xml: &str) -> KdmDigests {
    use quick_xml::Reader;
    use quick_xml::events::Event;

    let local_name = |name: quick_xml::name::QName| {
        String::from_utf8_lossy(name.local_name().as_ref()).into_owned()
    };

    let mut reader = Reader::from_str(xml);
    let mut digests = KdmDigests::default();
    let mut device_list_depth: u32 = 0;
    let mut collecting: Option<DigestElement> = None;
    let mut value = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match local_name(e.name()).as_str() {
                DEVICE_LIST_TAG => device_list_depth += 1,
                CERTIFICATE_THUMBPRINT_TAG if device_list_depth > 0 => {
                    collecting = Some(DigestElement::CertificateThumbprint);
                    value.clear();
                }
                CONTENT_AUTHENTICATOR_TAG => {
                    collecting = Some(DigestElement::ContentAuthenticator);
                    value.clear();
                }
                _ => {}
            },
            // a self-closed element carries no digest at all, which the length
            // rule below has to see rather than skip
            Ok(Event::Empty(e)) => match local_name(e.name()).as_str() {
                CERTIFICATE_THUMBPRINT_TAG if device_list_depth > 0 => {
                    digests.device_thumbprints.push(String::new())
                }
                CONTENT_AUTHENTICATOR_TAG => digests.content_authenticator = Some(String::new()),
                _ => {}
            },
            Ok(Event::Text(e)) => {
                if collecting.is_some() {
                    value.push_str(&dcpdoctor_parse::text_of(&e));
                }
            }
            Ok(Event::End(e)) => match local_name(e.name()).as_str() {
                DEVICE_LIST_TAG => device_list_depth = device_list_depth.saturating_sub(1),
                CERTIFICATE_THUMBPRINT_TAG | CONTENT_AUTHENTICATOR_TAG => {
                    // base64 may be wrapped across lines, so the value is the
                    // text with every run of whitespace removed.
                    let digest: String = value.split_whitespace().collect();
                    match collecting.take() {
                        Some(DigestElement::CertificateThumbprint) => {
                            digests.device_thumbprints.push(digest)
                        }
                        Some(DigestElement::ContentAuthenticator) => {
                            digests.content_authenticator = Some(digest)
                        }
                        None => {}
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    digests
}

fn is_sha1_digest(base64_digest: &str) -> bool {
    base64::engine::general_purpose::STANDARD
        .decode(base64_digest)
        .is_ok_and(|bytes| bytes.len() == SHA1_DIGEST_BYTES)
}

/// Digest rules the KDM schema cannot express: thumbprint and content
/// authenticator length, and the exclusivity of the DCI assume-trust marker.
fn check_digests(xml: &str, kdm_path: &Path) -> Vec<Note> {
    let digests = extract_digests(xml);
    let mut notes = Vec::new();

    for thumbprint in &digests.device_thumbprints {
        if !is_sha1_digest(thumbprint) {
            notes.push(
                Note::error(
                    Code::KdmThumbprintInvalid,
                    format!(
                        "DeviceList thumbprint '{thumbprint}' does not decode to a {SHA1_DIGEST_BYTES}-byte SHA-1 digest"
                    ),
                )
                .with_file(kdm_path),
            );
        }
    }

    if let Some(authenticator) = &digests.content_authenticator
        && !is_sha1_digest(authenticator)
    {
        notes.push(
            Note::error(
                Code::KdmContentAuthenticatorInvalid,
                format!(
                    "ContentAuthenticator '{authenticator}' does not decode to a {SHA1_DIGEST_BYTES}-byte SHA-1 digest"
                ),
            )
            .with_file(kdm_path),
        );
    }

    let named_devices = digests
        .device_thumbprints
        .iter()
        .filter(|t| *t != ASSUME_TRUST_THUMBPRINT)
        .count();
    let assumes_trust = digests
        .device_thumbprints
        .iter()
        .any(|t| t == ASSUME_TRUST_THUMBPRINT);
    if assumes_trust && named_devices > 0 {
        notes.push(
            Note::error(
                Code::KdmAssumeTrustConflict,
                format!(
                    "DeviceList carries the DCI assume-trust thumbprint '{ASSUME_TRUST_THUMBPRINT}' alongside {named_devices} device thumbprint(s); the list either restricts playback to named devices or it does not"
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

// The digest rules, run against KDMs another implementation wrote. Each rule has
// a test that it fires on a mutant and one that it stays silent on the real
// files, matching the per-code corpus shape the dci-ctp suite uses.
#[cfg(test)]
mod digest_tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    // Generated with DCP-o-matic 2.18.39 (libdcp), one file per ISDCF
    // formulation, by
    //   dcpomatic2_kdm_cli -F <formulation> [-T certs/intermediate.pem] -C certs/leaf.pem
    // so every digest in them is another implementation's output, not a value we
    // invented. certs/ is the chain they were signed with; the dci-* files'
    // ContentAuthenticator is the ST 430-2 thumbprint of certs/leaf.pem.
    const FIXTURE_DIR: &str = "../../../tests/fixtures/kdm";

    /// assume-trust marker in the DeviceList, no ContentAuthenticator.
    const MODIFIED_TRANSITIONAL_1: &str = "kdm-modified-transitional-1.xml";
    /// a real device thumbprint, no ContentAuthenticator.
    const MULTIPLE_MODIFIED_TRANSITIONAL_1: &str = "kdm-multiple-modified-transitional-1.xml";
    /// assume-trust marker plus a ContentAuthenticator.
    const DCI_ANY: &str = "kdm-dci-any.xml";
    /// a real device thumbprint plus a ContentAuthenticator.
    const DCI_SPECIFIC: &str = "kdm-dci-specific.xml";

    const ALL_FIXTURES: &[&str] = &[
        MODIFIED_TRANSITIONAL_1,
        MULTIPLE_MODIFIED_TRANSITIONAL_1,
        DCI_ANY,
        DCI_SPECIFIC,
    ];

    const DIGEST_CODES: &[Code] = &[
        Code::KdmThumbprintInvalid,
        Code::KdmContentAuthenticatorInvalid,
        Code::KdmAssumeTrustConflict,
    ];

    fn fixture_path(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(FIXTURE_DIR)
            .join(name)
    }

    fn fixture_xml(name: &str) -> String {
        std::fs::read_to_string(fixture_path(name)).expect("fixture is present")
    }

    /// Write a fixture out with one substring swapped, so a negative case starts
    /// from a real KDM instead of a hand-written one.
    fn mutated_fixture(name: &str, from: &str, to: &str) -> tempfile::NamedTempFile {
        let xml = fixture_xml(name);
        assert!(xml.contains(from), "{name} does not contain '{from}'");
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(f, "{}", xml.replace(from, to)).unwrap();
        f
    }

    /// The same digest one byte short: still base64, no longer SHA-1 sized.
    fn truncated(base64_digest: &str) -> String {
        let engine = base64::engine::general_purpose::STANDARD;
        let mut bytes = engine
            .decode(base64_digest)
            .expect("fixture digest is base64");
        bytes.pop();
        engine.encode(bytes)
    }

    fn only_thumbprint(name: &str) -> String {
        let digests = extract_digests(&fixture_xml(name));
        assert_eq!(digests.device_thumbprints.len(), 1, "{name} DeviceList");
        digests.device_thumbprints[0].clone()
    }

    fn thumbprint_element(digest: &str) -> String {
        format!("<{CERTIFICATE_THUMBPRINT_TAG}>{digest}</{CERTIFICATE_THUMBPRINT_TAG}>")
    }

    #[test]
    fn the_fixtures_cover_both_content_authenticator_cases() {
        for name in [MODIFIED_TRANSITIONAL_1, MULTIPLE_MODIFIED_TRANSITIONAL_1] {
            assert!(
                extract_digests(&fixture_xml(name))
                    .content_authenticator
                    .is_none(),
                "{name} must have no ContentAuthenticator"
            );
        }
        for name in [DCI_ANY, DCI_SPECIFIC] {
            assert!(
                extract_digests(&fixture_xml(name))
                    .content_authenticator
                    .is_some(),
                "{name} must carry a ContentAuthenticator"
            );
        }
    }

    #[test]
    fn real_kdms_draw_no_digest_note() {
        for name in ALL_FIXTURES {
            let notes = validate_kdm(&fixture_path(name), None);
            assert!(
                !notes.iter().any(|n| DIGEST_CODES.contains(&n.code)),
                "{name} is a valid KDM, got: {notes:?}"
            );
            assert!(
                !notes.iter().any(|n| n.code == Code::XmlSchemaViolation),
                "{name} must satisfy the vendored ST 430-1 / 430-3 schemas, got: {notes:?}"
            );
        }
    }

    #[test]
    fn a_short_thumbprint_fires() {
        let real = only_thumbprint(MULTIPLE_MODIFIED_TRANSITIONAL_1);
        let f = mutated_fixture(MULTIPLE_MODIFIED_TRANSITIONAL_1, &real, &truncated(&real));
        let notes = validate_kdm(f.path(), None);
        assert!(
            notes.iter().any(|n| n.code == Code::KdmThumbprintInvalid),
            "a 19-byte DeviceList thumbprint must error, got: {notes:?}"
        );
    }

    #[test]
    fn an_empty_thumbprint_element_fires() {
        let real = only_thumbprint(MULTIPLE_MODIFIED_TRANSITIONAL_1);
        let f = mutated_fixture(
            MULTIPLE_MODIFIED_TRANSITIONAL_1,
            &thumbprint_element(&real),
            &format!("<{CERTIFICATE_THUMBPRINT_TAG}/>"),
        );
        let notes = validate_kdm(f.path(), None);
        assert!(
            notes.iter().any(|n| n.code == Code::KdmThumbprintInvalid),
            "a self-closed thumbprint carries no digest, got: {notes:?}"
        );
    }

    #[test]
    fn a_short_content_authenticator_fires() {
        let real = extract_digests(&fixture_xml(DCI_ANY))
            .content_authenticator
            .expect("dci-any carries a ContentAuthenticator");
        let f = mutated_fixture(DCI_ANY, &real, &truncated(&real));
        let notes = validate_kdm(f.path(), None);
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::KdmContentAuthenticatorInvalid),
            "a 19-byte ContentAuthenticator must error, got: {notes:?}"
        );
    }

    #[test]
    fn the_assume_trust_marker_beside_a_real_device_fires() {
        let real = only_thumbprint(DCI_SPECIFIC);
        let both = format!(
            "{}\n            {}",
            thumbprint_element(&real),
            thumbprint_element(ASSUME_TRUST_THUMBPRINT)
        );
        let f = mutated_fixture(DCI_SPECIFIC, &thumbprint_element(&real), &both);
        let notes = validate_kdm(f.path(), None);
        assert!(
            notes.iter().any(|n| n.code == Code::KdmAssumeTrustConflict),
            "the assume-trust marker must not share a DeviceList, got: {notes:?}"
        );
        assert!(
            !notes.iter().any(|n| n.code == Code::KdmThumbprintInvalid),
            "both thumbprints are SHA-1 sized, got: {notes:?}"
        );
    }
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

    // ─── KDM schema validation (ST 430-1 / ST 430-3) ───────────────────────

    const AUTHORIZED_DEVICE_INFO: &str = r#"<AuthorizedDeviceInfo>
          <DeviceListIdentifier>urn:uuid:bbbbbbbb-cccc-dddd-eeee-ffffffffffff</DeviceListIdentifier>
          <DeviceList>
            <CertificateThumbprint>oQjE4GVsXTeawQOL//tMJ3HAMzk=</CertificateThumbprint>
          </DeviceList>
        </AuthorizedDeviceInfo>"#;

    /// A structurally complete SMPTE KDM. `authorized_device_info` is spliced in
    /// so the same document can be built with and without that one element.
    fn smpte_kdm(authorized_device_info: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(
            f,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<DCinemaSecurityMessage xmlns="http://www.smpte-ra.org/schemas/430-3/2006/ETM" xmlns:ds="http://www.w3.org/2000/09/xmldsig#" xmlns:enc="http://www.w3.org/2001/04/xmlenc#">
  <AuthenticatedPublic Id="ID_AuthenticatedPublic">
    <MessageId>urn:uuid:11111111-2222-3333-4444-555555555555</MessageId>
    <MessageType>http://www.smpte-ra.org/430-1/2006/KDM#kdm-key-type</MessageType>
    <IssueDate>2026-01-01T00:00:00+00:00</IssueDate>
    <Signer>
      <ds:X509IssuerName>dnQualifier=abc,CN=test,OU=test,O=test</ds:X509IssuerName>
      <ds:X509SerialNumber>1</ds:X509SerialNumber>
    </Signer>
    <RequiredExtensions>
      <KDMRequiredExtensions xmlns="http://www.smpte-ra.org/schemas/430-1/2006/KDM">
        <Recipient>
          <X509IssuerSerial>
            <ds:X509IssuerName>dnQualifier=abc,CN=test,OU=test,O=test</ds:X509IssuerName>
            <ds:X509SerialNumber>2</ds:X509SerialNumber>
          </X509IssuerSerial>
          <X509SubjectName>dnQualifier=xyz,CN=recipient,OU=test,O=test</X509SubjectName>
        </Recipient>
        <CompositionPlaylistId>urn:uuid:66666666-7777-8888-9999-aaaaaaaaaaaa</CompositionPlaylistId>
        <ContentTitleText>test</ContentTitleText>
        <ContentKeysNotValidBefore>2026-01-01T00:00:00+00:00</ContentKeysNotValidBefore>
        <ContentKeysNotValidAfter>2099-01-01T00:00:00+00:00</ContentKeysNotValidAfter>
        {authorized_device_info}
        <KeyIdList>
          <TypedKeyId>
            <KeyType>MDIK</KeyType>
            <KeyId>urn:uuid:cccccccc-dddd-eeee-ffff-000000000000</KeyId>
          </TypedKeyId>
        </KeyIdList>
      </KDMRequiredExtensions>
    </RequiredExtensions>
    <NonCriticalExtensions/>
  </AuthenticatedPublic>
  <AuthenticatedPrivate Id="ID_AuthenticatedPrivate">
    <enc:EncryptedKey>
      <enc:EncryptionMethod Algorithm="http://www.w3.org/2001/04/xmlenc#rsa-oaep-mgf1p"/>
      <enc:CipherData><enc:CipherValue>YWJj</enc:CipherValue></enc:CipherData>
    </enc:EncryptedKey>
  </AuthenticatedPrivate>
  <ds:Signature>
    <ds:SignedInfo>
      <ds:CanonicalizationMethod Algorithm="http://www.w3.org/TR/2001/REC-xml-c14n-20010315#"/>
      <ds:SignatureMethod Algorithm="http://www.w3.org/2001/04/xmldsig-more#rsa-sha256"/>
      <ds:Reference URI="">
        <ds:DigestMethod Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/>
        <ds:DigestValue>oQjE4GVsXTeawQOL//tMJ3HAMzk=</ds:DigestValue>
      </ds:Reference>
    </ds:SignedInfo>
    <ds:SignatureValue>YWJj</ds:SignatureValue>
  </ds:Signature>
</DCinemaSecurityMessage>"#
        )
        .unwrap();
        f
    }

    /// The schema pass shells out to xmllint against the vendored XSDs, so it
    /// cannot run without either.
    fn schema_validation_available() -> bool {
        std::process::Command::new("xmllint")
            .arg("--version")
            .output()
            .is_ok()
            && crate::schema::locate_schema_dir().is_some()
    }

    #[test]
    fn kdm_without_authorized_device_info_violates_the_schema() {
        if !schema_validation_available() {
            return;
        }
        let f = smpte_kdm("");
        let notes = validate_kdm(f.path(), None);
        assert!(
            notes.iter().any(|n| n.code == Code::XmlSchemaViolation
                && n.message.contains("AuthorizedDeviceInfo")),
            "a KDM missing AuthorizedDeviceInfo must fail the ST 430-1 schema, got: {notes:?}"
        );
    }

    #[test]
    fn complete_kdm_passes_the_schema() {
        if !schema_validation_available() {
            return;
        }
        let f = smpte_kdm(AUTHORIZED_DEVICE_INFO);
        let notes = validate_kdm(f.path(), None);
        assert!(
            !notes.iter().any(|n| n.code == Code::XmlSchemaViolation),
            "a complete KDM must draw no schema violation, got: {notes:?}"
        );
    }
}

// End-to-end KDM decryption: build an encrypted picture MXF in-test, generate a
// KDM for it, and prove the encrypted-essence checks skip without a KDM, fire on
// decrypted essence with the right KDM, and fail loud on a wrong key / bad MIC.
#[cfg(test)]
mod decrypt_tests {
    use super::*;
    use postkit::certificate::{
        KdmConfig, KdmContentKey, KdmFormulation, build_kdm, generate_chain,
    };
    use std::path::PathBuf;

    // A 2K picture codestream with `guard_bits` in its QCD: SOC, QCD, SOT. Enough
    // for the guard-bit rule (width comes from the descriptor, guard bits from the
    // QCD). 2K expects 1 guard bit, so gb=0 is a planted violation.
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

    /// An ST 430-1 timestamp one day out. The chain is minted now, and a signer
    /// may not start on or after the day its KDM's window starts, so a window
    /// starting today would be refused the way libdcp refuses it.
    fn tomorrow() -> String {
        let t = time::OffsetDateTime::now_utc() + time::Duration::days(1);
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}+00:00",
            t.year(),
            u8::from(t.month()),
            t.day(),
            t.hour(),
            t.minute(),
            t.second()
        )
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
            valid_from: tomorrow(),
            valid_to: "7 days".to_string(),
            formulation: KdmFormulation::DciAny,
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

    /// A subtitle document that breaks two structural rules: no ReelNumber and no
    /// Language. Wrapped and encrypted below, it proves the rules only see it
    /// when a key is available.
    const NONCONFORMANT_SUBTITLE: &str = r#"<?xml version="1.0"?>
<dcst:SubtitleReel xmlns:dcst="http://www.smpte-ra.org/schemas/428-7/2010/DCST">
  <dcst:Id>urn:uuid:11111111-2222-3333-4444-555555555555</dcst:Id>
  <dcst:LoadFont ID="f1">urn:uuid:abababab-abab-abab-abab-abababababab</dcst:LoadFont>
  <dcst:SubtitleList>
    <dcst:Font ID="f1">
      <dcst:Subtitle SpotNumber="1" TimeIn="00:00:05:000" TimeOut="00:00:07:000">
        <dcst:Text>Hi</dcst:Text>
      </dcst:Subtitle>
    </dcst:Font>
  </dcst:SubtitleList>
</dcst:SubtitleReel>"#;

    /// Write an encrypted timed-text MXF carrying `doc`, HMAC-guarded the way
    /// asdcplib wraps a real subtitle asset.
    fn write_encrypted_subtitle_mxf(path: &Path, key_id: uuid::Uuid, content_key: [u8; 16]) {
        use asdcplib::crypto::{AesEncContext, HmacContext};
        use asdcplib::timed_text::{MxfWriter, TimedTextDescriptor};
        use asdcplib::{EDIT_RATE_24, LabelSet, WriterInfo};

        let info = WriterInfo {
            asset_uuid: *uuid::Uuid::new_v4().as_bytes(),
            context_id: *uuid::Uuid::new_v4().as_bytes(),
            cryptographic_key_id: *key_id.as_bytes(),
            encrypted_essence: true,
            uses_hmac: true,
            label_set: LabelSet::Smpte,
            ..Default::default()
        };
        let desc = TimedTextDescriptor {
            edit_rate: EDIT_RATE_24,
            container_duration: 96,
            asset_id: [6; 16],
        };
        let mut enc = AesEncContext::new();
        enc.init_key(&content_key).unwrap();
        let mut hmac = HmacContext::new();
        hmac.init_key(&content_key, LabelSet::Smpte).unwrap();

        let mut w = MxfWriter::new();
        w.open_write(path.to_str().unwrap(), &info, &desc, 16_384)
            .unwrap();
        w.write_timed_text_resource(NONCONFORMANT_SUBTITLE, Some(&mut enc), Some(&mut hmac))
            .unwrap();
        w.finalize().unwrap();
    }

    // KDM-aware subtitle validation is the thing this tool does that a validator
    // without keys cannot: holding the key and still skipping the rules would be
    // a silent pass on content nobody checked.
    #[test]
    fn encrypted_subtitles_are_checked_when_the_kdm_covers_them() {
        use crate::subtitle::{FontData, read_wrapped_timed_text, validate_subtitle_xml};

        let key_id = uuid::Uuid::new_v4();
        let content_key = [0x44; 16];
        let cpl_id = "8a2b1c3d-4e5f-6071-8293-a4b5c6d7e8f9";
        let (kdm_path, recipient_key, _wrong, _certs) = make_kdm(key_id, content_key, cpl_id);

        let dir = tempfile::tempdir().unwrap();
        let mxf = dir.path().join("sub.mxf");
        write_encrypted_subtitle_mxf(&mxf, key_id, content_key);

        // without keys the document is unreadable, so the rules must not pretend
        let without = read_wrapped_timed_text(&mxf, &ContentKeys::none(), FontData::Omit)
            .expect("the MXF header is readable even when the essence is not");
        assert!(
            without.is_unreadable(),
            "encrypted essence must not come back as a document without a key"
        );

        // with the KDM the document decrypts and the structural rules apply
        let keys = ContentKeys::from_kdm(&kdm_path, &recipient_key).expect("unwrap kdm");
        let with = read_wrapped_timed_text(&mxf, &keys, FontData::Omit)
            .expect("the asset must be readable with its key");
        assert!(
            !with.is_unreadable(),
            "a covered KeyId must yield the document"
        );
        let notes = validate_subtitle_xml(&with.xml, &mxf, crate::Standard::Smpte);
        for missing in ["ReelNumber", "Language"] {
            assert!(
                notes
                    .iter()
                    .any(|n| n.code == Code::MissingRequiredElement && n.message.contains(missing)),
                "the decrypted document must reach the structural rules ({missing}), got: {notes:?}"
            );
        }
    }

    /// A one-reel SMPTE CPL whose MainSubtitle points at `sub.mxf`, plus the
    /// id -> file map the reel-level rules resolve assets through.
    fn package_with_encrypted_subtitle(
        dir: &Path,
        key_id: uuid::Uuid,
        content_key: [u8; 16],
    ) -> (PathBuf, std::collections::HashMap<String, PathBuf>) {
        const SUBTITLE_ID: &str = "11111111-2222-3333-4444-555555555555";

        let mxf = dir.join("sub.mxf");
        write_encrypted_subtitle_mxf(&mxf, key_id, content_key);

        let cpl = dir.join("cpl.xml");
        std::fs::write(
            &cpl,
            format!(
                r#"<?xml version="1.0"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL">
  <Id>urn:uuid:cccccccc-0000-0000-0000-000000000000</Id>
  <ContentTitleText>t</ContentTitleText>
  <ReelList><Reel><Id>urn:uuid:b353da2a-703e-4d3f-8fcd-659930713ece</Id>
    <AssetList>
      <MainPicture><Id>urn:uuid:f76deec8-ab85-4f05-973d-089b67a55e5f</Id><EditRate>24 1</EditRate><Duration>96</Duration></MainPicture>
      <MainSubtitle><Id>urn:uuid:{SUBTITLE_ID}</Id><EditRate>24 1</EditRate><Duration>96</Duration></MainSubtitle>
    </AssetList>
  </Reel></ReelList>
</CompositionPlaylist>"#
            ),
        )
        .unwrap();

        let map = std::collections::HashMap::from([(SUBTITLE_ID.to_string(), mxf)]);
        (cpl, map)
    }

    // an encrypted package validated without its keys used to skip the timed-text
    // rules in silence, which reads as a pass on content nobody examined.
    #[test]
    fn skipped_encrypted_timed_text_is_reported_not_silent() {
        let key_id = uuid::Uuid::new_v4();
        let content_key = [0x55; 16];
        let cpl_id = "8a2b1c3d-4e5f-6071-8293-a4b5c6d7e8f9";
        let (kdm_path, recipient_key, _wrong, _certs) = make_kdm(key_id, content_key, cpl_id);

        let dir = tempfile::tempdir().unwrap();
        let (cpl, map) = package_with_encrypted_subtitle(dir.path(), key_id, content_key);

        // no KDM: the rules cannot run, and the report must say so
        let notes = crate::validators::check_timed_text_content(
            &cpl,
            crate::Standard::Smpte,
            &map,
            &ContentKeys::none(),
        );
        let skip = notes
            .iter()
            .find(|n| n.code == Code::KdmRequired)
            .expect("a skipped encrypted asset must be reported");
        assert!(
            skip.message.contains("timed-text rules did not run"),
            "got: {}",
            skip.message
        );
        assert_eq!(
            skip.severity,
            crate::Severity::Info,
            "validating an encrypted package without its KDM is a normal thing to do"
        );

        // with the KDM the rules run, so nothing is skipped
        let keys = ContentKeys::from_kdm(&kdm_path, &recipient_key).expect("unwrap kdm");
        let notes =
            crate::validators::check_timed_text_content(&cpl, crate::Standard::Smpte, &map, &keys);
        assert!(
            !notes.iter().any(|n| n.code == Code::KdmRequired),
            "nothing is skipped once the key is available, got: {notes:?}"
        );
    }

    #[test]
    fn encrypted_essence_skips_without_a_kdm() {
        let dir = tempfile::tempdir().unwrap();
        let mxf = dir.path().join("pic.mxf");
        write_encrypted_mxf(&mxf, uuid::Uuid::new_v4(), [0x11; 16], 0);

        let (notes, _forensics) =
            crate::j2k::check_picture_j2k_mxf(&mxf, &ContentKeys::none(), true);
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
        let (notes, _forensics) = crate::j2k::check_picture_j2k_mxf(&mxf, &keys, true);
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
        let (notes, _forensics) = crate::j2k::check_picture_j2k_mxf(&mxf, &keys, true);
        assert!(
            notes.iter().any(|n| n.code == Code::MxfHashMismatch),
            "a mismatched content key must fail the MIC, got: {notes:?}"
        );
    }
}
