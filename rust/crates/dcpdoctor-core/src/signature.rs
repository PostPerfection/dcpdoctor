//! XML digital signature verification for signed CPL/PKL.
//!
//! The enveloped-signature check (digest + RSA over the canonicalized document)
//! is delegated to postkit. On top of that we validate the embedded certificate
//! chain: DCI signatures carry the whole chain (leaf -> intermediate -> root) in
//! ds:KeyInfo, so the trust model is self-contained: each cert must be signed by
//! the next and the chain must terminate in a self-signed root, all within dates.
use crate::{Code, Note};
use base64::Engine;
use std::path::Path;
use x509_parser::prelude::*;

/// Verify the enveloped XML signature on a signed CPL/PKL.
///
/// Returns an error note if the signature is present but invalid, plus
/// certificate expiry / chain notes; empty if the file carries no signature.
pub fn verify_signature(xml_file: &Path) -> Vec<Note> {
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
    if !content.contains("<Signature") && !content.contains("<ds:Signature") {
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

    notes.extend(verify_cert_chain(&content, xml_file));
    notes
}

/// Extract base64 X509Certificate blobs from ds:KeyInfo.
fn extract_certs(content: &str) -> Vec<Vec<u8>> {
    let re =
        regex_lite::Regex::new(r"(?s)<(?:ds:)?X509Certificate>(.*?)</(?:ds:)?X509Certificate>")
            .unwrap();
    let mut out = Vec::new();
    for cap in re.captures_iter(content) {
        let cleaned: String = cap[1].chars().filter(|c| !c.is_whitespace()).collect();
        if let Ok(der) = base64::engine::general_purpose::STANDARD.decode(&cleaned) {
            out.push(der);
        }
    }
    out
}

fn common_name<'a>(name: &'a X509Name<'a>) -> String {
    name.iter_common_name()
        .next()
        .and_then(|a| a.as_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| name.to_string())
}

/// Validate the embedded DCI certificate chain: expiry and issuer linkage.
fn verify_cert_chain(content: &str, xml_file: &Path) -> Vec<Note> {
    let ders = extract_certs(content);
    if ders.is_empty() {
        return Vec::new();
    }

    let mut certs = Vec::new();
    let mut notes = Vec::new();
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

    // Expiry / not-yet-valid per certificate.
    for cert in &certs {
        if !cert.validity().is_valid() {
            let v = cert.validity();
            notes.push(
                Note::error(
                    Code::CertificateExpired,
                    format!(
                        "Certificate '{}' is outside its validity period ({} to {})",
                        common_name(cert.subject()),
                        v.not_before,
                        v.not_after
                    ),
                )
                .with_file(xml_file),
            );
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
        assert!(verify_signature(&p).is_empty());
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
        let notes = verify_signature(&p);
        assert!(
            notes.iter().any(|n| n.code == Code::CertificateChainBroken),
            "an unparseable embedded cert must break the chain, got: {notes:?}"
        );
    }

    #[test]
    fn broken_signature_is_reported_invalid() {
        // Has a ds:Signature but the value is garbage, so verification must fail.
        let xml = r#"<CompositionPlaylist xmlns="x"><Id>u</Id><ds:Signature xmlns:ds="http://www.w3.org/2000/09/xmldsig#"><ds:SignedInfo><ds:Reference URI=""><ds:DigestValue>AAAA</ds:DigestValue></ds:Reference></ds:SignedInfo><ds:SignatureValue>bogus</ds:SignatureValue></ds:Signature></CompositionPlaylist>"#;
        let (_d, p) = write("cpl.xml", xml);
        let notes = verify_signature(&p);
        assert!(
            notes.iter().any(|n| n.code == Code::SignatureInvalid),
            "a broken signature must be reported invalid, got: {notes:?}"
        );
    }
}
