//! deep DCI / SMPTE ST 430-2 certificate-rule compliance for the embedded
//! CPL/PKL signature chain. runs on top of signature.rs's expiry + linkage
//! checks: each cert in the ds:KeyInfo chain must obey the 430-2 profile
//! (sha256 sig, 2048-bit RSA e=65537, correct BasicConstraints/KeyUsage for
//! its role, dnQualifier == public-key thumbprint, consistent Organization).
use crate::signature::extract_certs;
use crate::{Code, Note};
use base64::Engine;
use sha1::{Digest, Sha1};
use std::collections::HashSet;
use std::path::Path;
use x509_parser::der_parser::Oid;
use x509_parser::oid_registry::{
    OID_PKCS1_SHA256WITHRSA, OID_X509_DN_QUALIFIER, OID_X509_ORGANIZATION_NAME,
};
use x509_parser::prelude::*;
use x509_parser::public_key::PublicKey;

/// Apply the per-cert and chain rules to the embedded certificate chain.
/// Parse failures are already reported by signature::verify_cert_chain, so we
/// silently skip anything that does not parse here.
pub fn check_certificates(xml_content: &str, file: &Path) -> Vec<Note> {
    let ders = extract_certs(xml_content);
    if ders.is_empty() {
        return Vec::new();
    }

    let certs: Vec<X509Certificate> = ders
        .iter()
        .filter_map(|der| X509Certificate::from_der(der).ok().map(|(_, c)| c))
        .collect();
    if certs.is_empty() {
        return Vec::new();
    }

    let mut notes = Vec::new();

    // a cert is a CA if it issues some cert in the chain (the self-signed root
    // issues itself, so it is included); the leaf issues nothing.
    let issuers: HashSet<&[u8]> = certs.iter().map(|c| c.issuer().as_raw()).collect();
    let is_ca = |c: &X509Certificate| issuers.contains(c.subject().as_raw());

    for cert in &certs {
        let name = common_name(cert.subject());
        let ca = is_ca(cert);

        // rule 1: signature algorithm must be sha256WithRSAEncryption
        if cert.signature_algorithm.algorithm != OID_PKCS1_SHA256WITHRSA {
            notes.push(err(
                Code::CertificateSignatureAlgorithmInvalid,
                format!("Certificate '{name}' is not signed with sha256WithRSAEncryption"),
                file,
            ));
        }

        // rule 2: RSA 2048-bit modulus, exponent 65537
        if let Some(msg) = rsa_key_problem(cert) {
            notes.push(err(
                Code::CertificateKeySizeInvalid,
                format!("Certificate '{name}' {msg}"),
                file,
            ));
        }

        // rule 3: Basic Constraints present, cA matches the cert's role
        match cert.basic_constraints() {
            Ok(Some(bc)) => {
                if ca && !bc.value.ca {
                    notes.push(err(
                        Code::CertificateBasicConstraintsInvalid,
                        format!("CA certificate '{name}' has Basic Constraints cA=FALSE"),
                        file,
                    ));
                } else if !ca && bc.value.ca {
                    notes.push(err(
                        Code::CertificateBasicConstraintsInvalid,
                        format!("Leaf certificate '{name}' has Basic Constraints cA=TRUE"),
                        file,
                    ));
                }
            }
            _ => notes.push(err(
                Code::CertificateBasicConstraintsInvalid,
                format!("Certificate '{name}' is missing the Basic Constraints extension"),
                file,
            )),
        }

        // rule 4: Key Usage present, asserts the role's required bit
        match cert.key_usage() {
            Ok(Some(ku)) => {
                if ca && !ku.value.key_cert_sign() {
                    notes.push(err(
                        Code::CertificateKeyUsageInvalid,
                        format!("CA certificate '{name}' does not assert keyCertSign"),
                        file,
                    ));
                } else if !ca && !ku.value.digital_signature() {
                    notes.push(err(
                        Code::CertificateKeyUsageInvalid,
                        format!("Leaf certificate '{name}' does not assert digitalSignature"),
                        file,
                    ));
                }
            }
            _ => notes.push(err(
                Code::CertificateKeyUsageInvalid,
                format!("Certificate '{name}' is missing the Key Usage extension"),
                file,
            )),
        }

        // rule 6: dnQualifier == Base64(SHA-1(public-key BIT STRING payload))
        if let Some(dnq) = attr(cert.subject(), &OID_X509_DN_QUALIFIER) {
            let thumb = public_key_thumbprint(cert);
            if dnq != thumb {
                notes.push(err(
                    Code::CertificateThumbprintInvalid,
                    format!(
                        "Certificate '{name}' dnQualifier '{dnq}' does not match its public-key thumbprint '{thumb}'"
                    ),
                    file,
                ));
            }
        }
    }

    // rule 5: the leaf/signer role must be structurally distinct from the CA
    // roles. 430-2 CNs carry a role token before the first '.'; signer certs
    // use a real token (e.g. "CS") while CA/root CNs leave it empty.
    if let Some(leaf) = certs.iter().find(|c| !is_ca(c)) {
        let leaf_role = cn_role(&common_name(leaf.subject()));
        for cert in certs.iter().filter(|c| is_ca(c)) {
            if cn_role(&common_name(cert.subject())) == leaf_role {
                notes.push(err(
                    Code::CertificateRoleInvalid,
                    format!(
                        "Leaf certificate role '{leaf_role}' is not distinct from CA certificate '{}'",
                        common_name(cert.subject())
                    ),
                    file,
                ));
            }
        }
    }

    // rule 7: all certs in one chain must share the same Organization (O)
    let orgs: HashSet<Option<String>> = certs
        .iter()
        .map(|c| attr(c.subject(), &OID_X509_ORGANIZATION_NAME))
        .collect();
    if orgs.len() > 1 {
        notes.push(err(
            Code::CertificateOrganizationInconsistent,
            "Certificates in the chain do not share a single Organization (O)",
            file,
        ));
    }

    notes
}

fn err(code: Code, msg: impl Into<String>, file: &Path) -> Note {
    Note::error(code, msg).with_file(file)
}

fn common_name(name: &X509Name) -> String {
    name.iter_common_name()
        .next()
        .and_then(|a| a.as_str().ok())
        .map(str::to_string)
        .unwrap_or_else(|| name.to_string())
}

/// the role token of a 430-2 CommonName: everything before the first '.'.
fn cn_role(cn: &str) -> String {
    cn.split('.').next().unwrap_or("").to_string()
}

fn attr(name: &X509Name, oid: &Oid) -> Option<String> {
    name.iter_by_oid(oid)
        .next()
        .and_then(|a| a.as_str().ok())
        .map(str::to_string)
}

/// None if the RSA key matches the 430-2 profile, else the reason it doesn't.
fn rsa_key_problem(cert: &X509Certificate) -> Option<String> {
    match cert.public_key().parsed() {
        Ok(PublicKey::RSA(rsa)) => {
            let bits = rsa.key_size();
            if bits != 2048 {
                return Some(format!("has a {bits}-bit RSA key, expected 2048"));
            }
            match rsa.try_exponent() {
                Ok(65537) => None,
                Ok(e) => Some(format!("has RSA public exponent {e}, expected 65537")),
                Err(_) => Some("has an unreadable RSA public exponent".into()),
            }
        }
        Ok(_) => Some("does not use an RSA public key".into()),
        Err(_) => Some("has an unparseable public key".into()),
    }
}

/// Base64(SHA-1(DER of the public-key BIT STRING payload)), the 430-2 thumbprint.
fn public_key_thumbprint(cert: &X509Certificate) -> String {
    let payload = cert.public_key().subject_public_key.data.as_ref();
    base64::engine::general_purpose::STANDARD.encode(Sha1::digest(payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    // real ECL signer leaf cert (valid DCI cert, merely expired).
    const ECL_LEAF: &str = "MIIEXjCCA0agAwIBAgIEB/5beDANBgkqhkiG9w0BAQsFADCBgjEfMB0GA1UEAxMWLkNsaXBzdGVyRENJLlNpZ25hdHVyZTETMBEGA1UEChMKLkRDLkNBLkRWUzEjMCEGA1UECxMaLlNpZ25hdHVyZS5EdnNBRy5EQy5DQS5EVlMxJTAjBgNVBC4THDFIWDJGQzZyKzlGbytNMXJPaUtWOXZ5aGpDQT0wHhcNMTEwMjAyMTMzODUyWhcNMjUwMTAxMDAwMDAwWjCBjjErMCkGA1UEAxMiQ1MuTWFzdGVyaW5nLkNsaXBzdGVyRENJLjEzNDExMDA3MjETMBEGA1UEChMKLkRDLkNBLkRWUzEjMCEGA1UECxMaLlNpZ25hdHVyZS5EdnNBRy5EQy5DQS5EVlMxJTAjBgNVBC4THDc4bWw2WkNJSXlGYnNNRW1zY0ZMQnhyc0JuST0wggEiMA0GCSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQC2nuKwCYwmjX3sAJcutH9KBH9IFwt0xcV13E/XDYdI/mrqP3OwO5MUWkklcEn+4pbwiOEVjmOktVfhCDS8IQKJ7hW5UXnXiT7omu/r4FlOmYdCyD/NzS/QG8NoQK7EmxBnCr2hNtqJSR3cQjtxM7jmTbCLniHp8/HxoqJ762J8Svf875cOWmPMyy5YunMAQ7zVEn2lCuk1X26//azF2Pdi+P3qD6a+ukuuMx9o2B9A7892L1yVEZ7jcJdTYkVmVcyQxxhcjy3GoZY3XCFh/UpkUqMrehr/XaI608wkRxv3JZKIyxFBSv8GIz4CV8f2MTfWDQRw+N3fzyd/w2QKS2o9AgMBAAGjgc0wgcowDAYDVR0TAQH/BAIwADALBgNVHQ8EBAMCBaAwHQYDVR0OBBYEFO/JpemQiCMhW7DBJrHBSwca7AZyMIGNBgNVHSMEgYUwgYKAFNR19hQuq/vRaPjNazoilfb8oYwgoWekZTBjMRAwDgYDVQQDEwcuczQzMC0yMRMwEQYDVQQKEwouREMuQ0EuRFZTMRMwEQYDVQQLEwouREMuQ0EuRFZTMSUwIwYDVQQuExxiK1ZRazg1cUZRVHpVT1NweERoM2xMZWN0QjQ9ggEEMA0GCSqGSIb3DQEBCwUAA4IBAQDFhBUDMt9sFMUS12FM1r+04gSAAvJv13+J3ZqmfgvKG9BRQ8fJhXkVrJI5EjrotB8VR8gskBjucaXudxK5D4jMZCR9pjNIN2OONeQcb1d+4NQrc1MbDyxsNp91F71iITLl1417AyXCMZubTPCp6Z1zjJ3pz62gG0cKArNsFxermM/+8u6Y8AXrc54RbXxNLgNvlc7ENKBMFdoYdOLQ/xScs9bnuf+zYuH6A9aNWenUutsyeqSgVXoUobizpgyLtHPvMjjYVm5mGGU7Owgkt84HxHGMnCzPQ7bv41Sdd/zfQ+bjCgpWb1Ca0GzGUidhCuEcdWLF0+KjfksltqZ9xGj4";

    // self-signed test cert: 1024-bit RSA, O=BadOrg. violates the key-size and
    // (as a leaf-shaped self-signed cert) other rules.
    const BAD_1024: &str = "MIICIjCCAYugAwIBAgIUYVwIYGyTlhtGXQZveG9ixHWqjiowDQYJKoZIhvcNAQELBQAwIzEQMA4GA1UEAwwHQmFkVGVzdDEPMA0GA1UECgwGQmFkT3JnMB4XDTI2MDcyMDIxMjI0N1oXDTM2MDcxNzIxMjI0N1owIzEQMA4GA1UEAwwHQmFkVGVzdDEPMA0GA1UECgwGQmFkT3JnMIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQDnmqM7BI+mvuiPr+ojgFZjCr+GsXiQKiq0uAMSHEOjXJi28gtsDhEUi8OS/JI+pODSWaskGbgIClbW+oHg03ut8CWqDZsfib7Rg7/zmFDZfGA1G0Wyh2l9dU4963Nuy7sU/nnzRY07CXcweTubmW9a94MTaKVSgmi90OXMzWnBWQIDAQABo1MwUTAdBgNVHQ4EFgQU+4+ngcc06J+U49hm+WhVAa3q94YwHwYDVR0jBBgwFoAU+4+ngcc06J+U49hm+WhVAa3q94YwDwYDVR0TAQH/BAUwAwEB/zANBgkqhkiG9w0BAQsFAAOBgQBSL/AKaqpzX/1IiuFonS/bos9mqrUYxMM2FrONVoMeavohv19Q/SVNZcFrBKPrKtCRDYiKp/Tngd5vuqCHxJe6SdsJmPS1i5BJ4CZMemKCR3YBYwT+zuEf//8YeqX8cGqAKrJ3qkMkLHMZVA6UpOAG2o3XDvOpM9RbY2vkQw20KQ==";

    fn der(b64: &str) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap()
    }

    #[test]
    fn valid_ecl_leaf_passes_the_pure_rules() {
        let d = der(ECL_LEAF);
        let (_, cert) = X509Certificate::from_der(&d).unwrap();

        assert_eq!(cert.signature_algorithm.algorithm, OID_PKCS1_SHA256WITHRSA);
        assert_eq!(rsa_key_problem(&cert), None);
        // dnQualifier must equal the computed thumbprint
        let dnq = attr(cert.subject(), &OID_X509_DN_QUALIFIER).unwrap();
        assert_eq!(dnq, public_key_thumbprint(&cert));
        // leaf role token is a real value, distinct from the CA ("") roles
        assert_eq!(cn_role(&common_name(cert.subject())), "CS");
    }

    #[test]
    fn bad_key_size_is_flagged() {
        let d = der(BAD_1024);
        let (_, cert) = X509Certificate::from_der(&d).unwrap();
        assert!(rsa_key_problem(&cert).unwrap().contains("1024"));
    }

    #[test]
    fn key_size_code_fires_via_entry_point() {
        let xml = format!(
            r#"<CompositionPlaylist xmlns="x"><ds:Signature xmlns:ds="http://www.w3.org/2000/09/xmldsig#"><ds:KeyInfo><ds:X509Data><ds:X509Certificate>{BAD_1024}</ds:X509Certificate></ds:X509Data></ds:KeyInfo></ds:Signature></CompositionPlaylist>"#
        );
        let notes = check_certificates(&xml, Path::new("cpl.xml"));
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::CertificateKeySizeInvalid),
            "1024-bit key must be flagged, got: {notes:?}"
        );
    }

    #[test]
    fn mismatched_organization_is_flagged() {
        // ECL leaf (O=.DC.CA.DVS) chained with a foreign self-signed cert (O=BadOrg)
        let xml = format!(
            r#"<CompositionPlaylist xmlns="x"><ds:Signature xmlns:ds="http://www.w3.org/2000/09/xmldsig#"><ds:KeyInfo><ds:X509Data><ds:X509Certificate>{ECL_LEAF}</ds:X509Certificate><ds:X509Certificate>{BAD_1024}</ds:X509Certificate></ds:X509Data></ds:KeyInfo></ds:Signature></CompositionPlaylist>"#
        );
        let notes = check_certificates(&xml, Path::new("cpl.xml"));
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::CertificateOrganizationInconsistent),
            "differing O across the chain must be flagged, got: {notes:?}"
        );
    }
}
