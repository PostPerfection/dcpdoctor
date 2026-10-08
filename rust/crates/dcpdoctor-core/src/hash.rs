use base64::Engine as _;
use postkit::hash::{HashAlgorithm, hash_file};
use sha1::{Digest as _, Sha1};
use std::io::Read as _;
use std::path::Path;

const HASH_READ_CHUNK_BYTES: usize = 1 << 20;

/// Compute SHA-1 hash of a file, returning base64-encoded digest.
pub fn sha1_base64(path: &Path) -> std::io::Result<String> {
    Ok(hash_file(path, HashAlgorithm::Sha1)?.base64)
}

pub fn sha1_base64_with_progress(
    path: &Path,
    on_read: &mut dyn FnMut(u64),
) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut buffer = vec![0u8; HASH_READ_CHUNK_BYTES];
    let mut hasher = Sha1::new();
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        on_read(read as u64);
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(hasher.finalize()))
}

/// Compute SHA-1 hash of a file, returning hex-encoded digest.
pub fn sha1_hex(path: &Path) -> std::io::Result<String> {
    Ok(hash_file(path, HashAlgorithm::Sha1)?.hex)
}
