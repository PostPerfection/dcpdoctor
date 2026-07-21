use postkit::hash::{HashAlgorithm, hash_file};
use std::path::Path;

/// Compute SHA-1 hash of a file, returning base64-encoded digest.
pub fn sha1_base64(path: &Path) -> std::io::Result<String> {
    Ok(hash_file(path, HashAlgorithm::Sha1)?.base64)
}

/// Compute SHA-1 hash of a file, returning hex-encoded digest.
pub fn sha1_hex(path: &Path) -> std::io::Result<String> {
    Ok(hash_file(path, HashAlgorithm::Sha1)?.hex)
}
