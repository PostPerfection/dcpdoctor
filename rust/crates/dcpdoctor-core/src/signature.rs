//! XML digital signature verification for signed CPL/PKL and KDM documents.
//!
//! The enveloped-signature check (digest + RSA over the canonicalized document)
//! is delegated to postkit. A CPL or PKL signs the whole document; a KDM signs
//! its AuthenticatedPublic and AuthenticatedPrivate elements by Id, so the two
//! reach different postkit entry points.
//!
//! A verified signature proves the document has not changed since it was signed.
//! It says nothing about who signed it, since the verifying key is the leaf
//! certificate the document itself carries. Whether that chain is acceptable is
//! checked on top, here and in cert_rules. DCI signatures carry the whole chain
//! (leaf -> intermediate -> root) in ds:KeyInfo, so the trust model is
//! self-contained: each cert must be signed by the next and the chain must
//! terminate in a self-signed root, all within dates.
use crate::{Code, Note};
use base64::Engine;
use std::path::Path;
use x509_parser::prelude::*;

/// The namespace an XML-DSig Signature element is bound to.
const DSIG_NAMESPACE: &[u8] = b"http://www.w3.org/2000/09/xmldsig#";

/// The attribute a KDM's ds:Reference elements name their targets by. A KDM
/// signs AuthenticatedPublic and AuthenticatedPrivate by Id rather than covering
/// the whole document, so it needs the by-Id verifier.
const KDM_REFERENCE_ID_ATTRIBUTE: &str = "Id";

/// True when the document carries an XML-DSig Signature element.
///
/// Resolved by namespace, not by prefix: real packages bind the signature
/// namespace to `ds:` (postkit, asdcplib) and to `dsig:` (DCP-o-matic, the ISDCF
/// reference DCPs), and a prefix nothing recognised read as unsigned, so those
/// documents were never verified. A Signature in no namespace counts too, so a
/// malformed one reaches the verifier rather than passing as unsigned.
pub fn has_signature(content: &str) -> bool {
    use quick_xml::events::Event;
    use quick_xml::name::ResolveResult;

    let mut reader = quick_xml::NsReader::from_str(content);
    loop {
        match reader.read_resolved_event() {
            Ok((ns, Event::Start(e) | Event::Empty(e)))
                if e.local_name().as_ref() == b"Signature" =>
            {
                let is_dsig = match ns {
                    ResolveResult::Bound(ns) => ns.0 == DSIG_NAMESPACE,
                    _ => true,
                };
                if is_dsig {
                    return true;
                }
            }
            Ok((_, Event::Eof)) | Err(_) => return false,
            Ok(_) => {}
        }
    }
}

/// Verify the by-Id enveloped signature on a KDM, over the `xml` already read
/// from `kdm_path`. ST 430-1 requires every KDM to be signed, so an unsigned one
/// is a finding rather than nothing to check.
pub fn verify_kdm_signature(xml: &str, kdm_path: &Path) -> Vec<Note> {
    if !has_signature(xml) {
        return vec![
            Note::error(Code::SignatureInvalid, "KDM carries no signature").with_file(kdm_path),
        ];
    }
    match postkit::xmldsig::verify_enveloped(xml, KDM_REFERENCE_ID_ATTRIBUTE, None) {
        Ok(()) => Vec::new(),
        Err(e) => vec![
            Note::error(
                Code::SignatureInvalid,
                format!("XML signature verification failed: {e}"),
            )
            .with_file(kdm_path),
        ],
    }
}

/// Verify the enveloped XML signature on a signed CPL/PKL.
///
/// Returns an error note if the signature is present but invalid, plus
/// certificate expiry / chain notes; empty if the file carries no signature.
pub fn verify_signature(xml_file: &Path, strict: bool) -> Vec<Note> {
    let content = match std::fs::read_to_string(xml_file) {
        Ok(c) => c,
        Err(e) => {
            return vec![
                Note::error(Code::SignatureInvalid, format!("Failed to read file: {e}"))
                    .with_file(xml_file),
            ];
        }
    };

    // Nothing to verify on an unsigned document.
    if !has_signature(&content) {
        return Vec::new();
    }

    let mut notes = Vec::new();

    if let Err(e) = postkit::xmldsig::verify_document_enveloped(&content, None) {
        notes.push(
            Note::error(
                Code::SignatureInvalid,
                format!("XML signature verification failed: {e}"),
            )
            .with_file(xml_file),
        );
    }

    notes.extend(verify_cert_chain(&content, xml_file, strict));
    notes.extend(crate::cert_rules::check_certificates(&content, xml_file));
    notes
}

/// Extract base64 X509Certificate blobs from ds:KeyInfo, with the count of
/// elements whose base64 would not decode. Without that count a chain with a
/// corrupt certificate in it passes every chain rule.
pub(crate) fn extract_certs(content: &str) -> (Vec<Vec<u8>>, usize) {
    let re =
        regex_lite::Regex::new(r"(?s)<(?:ds:)?X509Certificate>(.*?)</(?:ds:)?X509Certificate>")
            .unwrap();
    let mut out = Vec::new();
    let mut undecodable = 0;
    for cap in re.captures_iter(content) {
        let cleaned: String = cap[1].chars().filter(|c| !c.is_whitespace()).collect();
        match base64::engine::general_purpose::STANDARD.decode(&cleaned) {
            Ok(der) => out.push(der),
            Err(_) => undecodable += 1,
        }
    }
    (out, undecodable)
}

fn common_name<'a>(name: &'a X509Name<'a>) -> String {
    name.iter_common_name()
        .next()
        .and_then(|a| a.as_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| name.to_string())
}

/// Validate the embedded DCI certificate chain: expiry and issuer linkage.
fn verify_cert_chain(content: &str, xml_file: &Path, strict: bool) -> Vec<Note> {
    let (ders, undecodable) = extract_certs(content);
    let mut notes = Vec::new();
    for _ in 0..undecodable {
        notes.push(
            Note::error(
                Code::CertificateChainBroken,
                "embedded certificate is not valid base64",
            )
            .with_file(xml_file),
        );
    }
    if ders.is_empty() {
        return notes;
    }

    let mut certs = Vec::new();
    for der in &ders {
        match X509Certificate::from_der(der) {
            Ok((_, cert)) => certs.push(cert),
            Err(e) => notes.push(
                Note::error(
                    Code::CertificateChainBroken,
                    format!("Failed to parse embedded certificate: {e}"),
                )
                .with_file(xml_file),
            ),
        }
    }

    // Expiry / not-yet-valid per certificate. Projectors don't block playback on an
    // expired CPL/PKL signing cert, so this is a warning by default, error under strict.
    for cert in &certs {
        if !cert.validity().is_valid() {
            let v = cert.validity();
            let msg = format!(
                "Certificate '{}' is outside its validity period ({} to {})",
                common_name(cert.subject()),
                v.not_before,
                v.not_after
            );
            let note = if strict {
                Note::error(Code::CertificateExpired, msg)
            } else {
                Note::warning(Code::CertificateExpired, msg)
            };
            notes.push(note.with_file(xml_file));
        }
    }

    // Chain linkage: each non-root cert must be signed by an issuer present in
    // the set; the chain must terminate in a self-signed root.
    let by_subject: std::collections::HashMap<&[u8], &X509Certificate> =
        certs.iter().map(|c| (c.subject().as_raw(), c)).collect();

    let mut found_root = false;
    for cert in &certs {
        let self_signed = cert.subject().as_raw() == cert.issuer().as_raw();
        if self_signed {
            if cert.verify_signature(None).is_ok() {
                found_root = true;
            } else {
                notes.push(
                    Note::error(
                        Code::CertificateChainBroken,
                        format!(
                            "Self-signed root '{}' has an invalid signature",
                            common_name(cert.subject())
                        ),
                    )
                    .with_file(xml_file),
                );
            }
            continue;
        }

        match by_subject.get(cert.issuer().as_raw()) {
            Some(issuer) => {
                if cert.verify_signature(Some(issuer.public_key())).is_err() {
                    notes.push(
                        Note::error(
                            Code::CertificateChainBroken,
                            format!(
                                "Certificate '{}' is not validly signed by its issuer '{}'",
                                common_name(cert.subject()),
                                common_name(cert.issuer())
                            ),
                        )
                        .with_file(xml_file),
                    );
                }
            }
            None => notes.push(
                Note::error(
                    Code::CertificateChainBroken,
                    format!(
                        "Issuer '{}' of certificate '{}' is not present in the chain",
                        common_name(cert.issuer()),
                        common_name(cert.subject())
                    ),
                )
                .with_file(xml_file),
            ),
        }
    }

    if certs.len() > 1 && !found_root {
        notes.push(
            Note::error(
                Code::CertificateChainBroken,
                "Certificate chain has no self-signed root (DCI requires a self-contained chain)",
            )
            .with_file(xml_file),
        );
    }

    notes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(name: &str, xml: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(name);
        std::fs::File::create(&p)
            .unwrap()
            .write_all(xml.as_bytes())
            .unwrap();
        (dir, p)
    }

    #[test]
    fn unsigned_document_reports_nothing() {
        let (_d, p) = write(
            "cpl.xml",
            "<CompositionPlaylist><Id>x</Id></CompositionPlaylist>",
        );
        assert!(verify_signature(&p, false).is_empty());
    }

    #[test]
    fn unparseable_embedded_certificate_breaks_the_chain() {
        // Valid base64 but not a DER certificate: extraction succeeds, parse fails.
        let bad_cert =
            base64::engine::general_purpose::STANDARD.encode(b"this is not a certificate");
        let xml = format!(
            r#"<CompositionPlaylist xmlns="x"><Id>u</Id><ds:Signature xmlns:ds="http://www.w3.org/2000/09/xmldsig#"><ds:KeyInfo><ds:X509Data><ds:X509Certificate>{bad_cert}</ds:X509Certificate></ds:X509Data></ds:KeyInfo></ds:Signature></CompositionPlaylist>"#
        );
        let (_d, p) = write("cpl.xml", &xml);
        let notes = verify_signature(&p, false);
        assert!(
            notes.iter().any(|n| n.code == Code::CertificateChainBroken),
            "an unparseable embedded cert must break the chain, got: {notes:?}"
        );
    }

    #[test]
    fn undecodable_embedded_certificate_breaks_the_chain() {
        let xml = r#"<CompositionPlaylist xmlns="x"><Id>u</Id><ds:Signature xmlns:ds="http://www.w3.org/2000/09/xmldsig#"><ds:KeyInfo><ds:X509Data><ds:X509Certificate>not base64 !!!</ds:X509Certificate></ds:X509Data></ds:KeyInfo></ds:Signature></CompositionPlaylist>"#;
        let (_d, p) = write("cpl.xml", xml);
        let notes = verify_signature(&p, false);
        assert!(
            notes.iter().any(|n| n.code == Code::CertificateChainBroken
                && n.message.contains("not valid base64")),
            "an undecodable embedded cert must break the chain, got: {notes:?}"
        );
    }

    #[test]
    fn an_unsigned_kdm_is_reported() {
        let notes = verify_kdm_signature(
            "<DCinemaSecurityMessage><AuthenticatedPublic/></DCinemaSecurityMessage>",
            Path::new("kdm.xml"),
        );
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::SignatureInvalid
                    && n.message.contains("carries no signature")),
            "ST 430-1 requires a KDM to be signed, got: {notes:?}"
        );
    }

    #[test]
    fn broken_signature_is_reported_invalid() {
        // Has a ds:Signature but the value is garbage, so verification must fail.
        let xml = r#"<CompositionPlaylist xmlns="x"><Id>u</Id><ds:Signature xmlns:ds="http://www.w3.org/2000/09/xmldsig#"><ds:SignedInfo><ds:Reference URI=""><ds:DigestValue>AAAA</ds:DigestValue></ds:Reference></ds:SignedInfo><ds:SignatureValue>bogus</ds:SignatureValue></ds:Signature></CompositionPlaylist>"#;
        let (_d, p) = write("cpl.xml", xml);
        let notes = verify_signature(&p, false);
        assert!(
            notes.iter().any(|n| n.code == Code::SignatureInvalid),
            "a broken signature must be reported invalid, got: {notes:?}"
        );
    }

    // ─── which documents count as signed ──────────────────────────────────

    fn signature_element(prefix_declaration: &str, open: &str, close: &str) -> String {
        format!(
            "<CompositionPlaylist{prefix_declaration}><Id>u</Id>{open}<x/>{close}</CompositionPlaylist>"
        )
    }

    #[test]
    fn a_signature_is_found_whatever_prefix_carries_the_namespace() {
        let dsig = r#" xmlns:dsig="http://www.w3.org/2000/09/xmldsig#""#;
        for (label, xml) in [
            (
                "ds:",
                signature_element(
                    r#" xmlns:ds="http://www.w3.org/2000/09/xmldsig#""#,
                    "<ds:Signature>",
                    "</ds:Signature>",
                ),
            ),
            (
                "dsig:",
                signature_element(dsig, "<dsig:Signature>", "</dsig:Signature>"),
            ),
            (
                "default namespace",
                signature_element(
                    "",
                    r#"<Signature xmlns="http://www.w3.org/2000/09/xmldsig#">"#,
                    "</Signature>",
                ),
            ),
            (
                "no namespace",
                signature_element("", "<Signature>", "</Signature>"),
            ),
        ] {
            assert!(has_signature(&xml), "{label} must count as signed: {xml}");
        }

        for (label, xml) in [
            (
                "unsigned",
                "<CompositionPlaylist><Id>u</Id></CompositionPlaylist>".to_string(),
            ),
            (
                "a Signature in someone else's namespace",
                signature_element(
                    r#" xmlns:other="http://example.invalid/ns""#,
                    "<other:Signature>",
                    "</other:Signature>",
                ),
            ),
        ] {
            assert!(
                !has_signature(&xml),
                "{label} must not count as signed: {xml}"
            );
        }
    }

    // ─── real signed packages ─────────────────────────────────────────────

    /// The ISDCF/DTB Bv21 reference DCP's CPL and PKL, committed under
    /// tests/fixtures/signature: real signed documents that bind the signature
    /// namespace to `dsig:`, declare the plain comment-free canonicalization,
    /// and carry XML comments inside the signed document. Each of those three
    /// broke verification on its own.
    const ISDCF_PACKAGE: &str = "../../../tests/fixtures/signature";
    const ISDCF_CPL: &str =
        "CPL_SMPTE_TST-1-Bv21_S_EN-EN-CCAP_US_51-HI-VI_2K_ISDCF_20170110_DTB_SMPTE_OV.xml";
    const ISDCF_PKL: &str =
        "PKL_SMPTE_TST-1-Bv21_S_EN-EN-CCAP_US_51-HI-VI_2K_ISDCF_20170110_DTB_SMPTE_OV.xml";
    const PLAIN_C14N: &str = r#"Algorithm="http://www.w3.org/TR/2001/REC-xml-c14n-20010315""#;

    fn isdcf_document(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(ISDCF_PACKAGE)
            .join(name)
    }

    #[test]
    fn a_real_dsig_prefixed_signed_document_verifies() {
        for name in [ISDCF_CPL, ISDCF_PKL] {
            let path = isdcf_document(name);
            let xml = std::fs::read_to_string(&path).expect("the package is committed");
            assert!(xml.contains("<dsig:Signature"), "{name} prefix changed");
            assert!(xml.contains("<!--"), "{name} no longer carries a comment");
            assert!(xml.contains(PLAIN_C14N), "{name} canonicalization changed");
            assert!(has_signature(&xml), "{name} must count as signed");

            let notes = verify_signature(&path, false);
            assert!(
                !notes.iter().any(|n| n.code == Code::SignatureInvalid),
                "{name} is validly signed and must not fire signature_invalid, got: {notes:?}"
            );
        }
    }

    #[test]
    fn one_byte_changed_after_signing_is_reported_invalid() {
        let xml = std::fs::read_to_string(isdcf_document(ISDCF_CPL)).unwrap();
        let tampered = xml.replacen("<Label>G</Label>", "<Label>R</Label>", 1);
        assert_ne!(tampered, xml, "the tamper anchor is gone from the CPL");

        let (_d, path) = write("cpl.xml", &tampered);
        let notes = verify_signature(&path, false);
        let invalid: Vec<&Note> = notes
            .iter()
            .filter(|n| n.code == Code::SignatureInvalid)
            .collect();
        assert_eq!(
            invalid.len(),
            1,
            "a document changed after signing must fire one signature_invalid, got: {notes:?}"
        );
        assert_eq!(invalid[0].severity, crate::Severity::Error);
        assert_eq!(invalid[0].file.as_deref(), Some(path.as_path()));
    }
}
