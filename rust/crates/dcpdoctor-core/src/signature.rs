//! XML digital signature verification for signed CPL/PKL, delegated to postkit.
use crate::{Code, Note};
use std::path::Path;

/// Verify the enveloped XML signature on a signed CPL/PKL.
///
/// Uses postkit's XML-DSig verifier: recomputes the reference digest over the
/// canonicalized document (ds:Signature removed) and checks the RSA-SHA256
/// signature against the embedded signing certificate. Returns an error note if
/// the signature is present but invalid; empty if the file carries no signature.
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

    match postkit::xmldsig::verify_document_enveloped(&content, None) {
        Ok(()) => Vec::new(),
        Err(e) => vec![
            Note::error(
                Code::SignatureInvalid,
                format!("XML signature verification failed: {e}"),
            )
            .with_file(xml_file),
        ],
    }
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
