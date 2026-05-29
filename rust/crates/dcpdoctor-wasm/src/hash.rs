use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use sha1::{Digest, Sha1};

/// Compute SHA-1 of raw bytes, return as base64.
pub fn compute_sha1_base64(data: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(data);
    let result = hasher.finalize();
    BASE64.encode(result)
}
