//! Hash cache for avoiding redundant checksum computations.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// File-based hash cache (stores path → hash mappings).
pub struct HashCache {
    cache_path: PathBuf,
    entries: HashMap<String, CacheEntry>,
}

struct CacheEntry {
    hash: String,
    mtime: u64,
    size: u64,
}

impl HashCache {
    /// Open or create a hash cache at the given path.
    pub fn new(cache_path: &Path) -> Self {
        let mut cache = Self {
            cache_path: cache_path.to_path_buf(),
            entries: HashMap::new(),
        };
        cache.load();
        cache
    }

    /// Get a cached hash for a file, or None if not cached or stale.
    pub fn get(&self, file: &Path) -> Option<&str> {
        let key = file.to_string_lossy().to_string();
        let entry = self.entries.get(&key)?;

        let meta = std::fs::metadata(file).ok()?;
        let size = meta.len();
        let mtime = meta
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs();

        if entry.mtime == mtime && entry.size == size {
            Some(&entry.hash)
        } else {
            None
        }
    }

    /// Store a hash for a file.
    pub fn put(&mut self, file: &Path, hash: &str) {
        let key = file.to_string_lossy().to_string();
        let meta = std::fs::metadata(file).ok();
        let (mtime, size) = meta
            .map(|m| {
                let s = m.len();
                let t = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                (t, s)
            })
            .unwrap_or((0, 0));

        self.entries.insert(
            key,
            CacheEntry {
                hash: hash.to_string(),
                mtime,
                size,
            },
        );
    }

    /// Save cache to disk.
    pub fn save(&self) {
        let mut lines = Vec::new();
        for (path, entry) in &self.entries {
            lines.push(format!(
                "{}\t{}\t{}\t{}",
                path, entry.hash, entry.mtime, entry.size
            ));
        }
        let _ = std::fs::write(&self.cache_path, lines.join("\n"));
    }

    /// Clear all cached entries.
    pub fn clear(&mut self) {
        self.entries.clear();
        let _ = std::fs::remove_file(&self.cache_path);
    }

    fn load(&mut self) {
        let Ok(content) = std::fs::read_to_string(&self.cache_path) else {
            return;
        };
        for line in content.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() == 4 {
                self.entries.insert(
                    parts[0].to_string(),
                    CacheEntry {
                        hash: parts[1].to_string(),
                        mtime: parts[2].parse().unwrap_or(0),
                        size: parts[3].parse().unwrap_or(0),
                    },
                );
            }
        }
    }
}

impl Drop for HashCache {
    fn drop(&mut self) {
        self.save();
    }
}
