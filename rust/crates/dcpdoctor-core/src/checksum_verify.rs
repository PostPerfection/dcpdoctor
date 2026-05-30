//! Full package checksum verification — verify all hashes in PKL against files.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// Options for checksum verification.
pub struct ChecksumVerifyOptions {
    pub package_dir: PathBuf,
    pub verify_hashes: bool,
    pub verify_sizes: bool,
    pub stop_on_first_error: bool,
}

impl Default for ChecksumVerifyOptions {
    fn default() -> Self {
        Self {
            package_dir: PathBuf::new(),
            verify_hashes: true,
            verify_sizes: true,
            stop_on_first_error: false,
        }
    }
}

/// Status of a single asset checksum check.
#[derive(Debug, Clone, Serialize)]
pub struct ChecksumEntry {
    pub asset_id: String,
    pub filename: String,
    pub file_exists: bool,
    pub expected_hash: String,
    pub computed_hash: String,
    pub hash_match: bool,
    pub expected_size: i64,
    pub actual_size: i64,
    pub size_match: bool,
}

/// Result of verifying all checksums in a package.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ChecksumVerifyResult {
    pub success: bool,
    pub all_valid: bool,
    pub error: String,
    pub total_assets: u32,
    pub verified_ok: u32,
    pub hash_mismatches: u32,
    pub size_mismatches: u32,
    pub missing_files: u32,
    pub entries: Vec<ChecksumEntry>,
}

/// Verify all asset checksums in a DCP/IMF package.
pub fn verify_package_checksums(opts: &ChecksumVerifyOptions) -> ChecksumVerifyResult {
    let mut result = ChecksumVerifyResult::default();

    if !opts.package_dir.exists() {
        result.error = format!(
            "Package directory does not exist: {}",
            opts.package_dir.display()
        );
        return result;
    }

    // Find PKL file(s)
    let pkl_files = find_pkl_files(&opts.package_dir);
    if pkl_files.is_empty() {
        result.error = format!("No PKL found in {}", opts.package_dir.display());
        return result;
    }

    // Build asset-id to path mapping from ASSETMAP
    let asset_paths = parse_assetmap(&opts.package_dir);

    // Process each PKL
    for pkl_path in &pkl_files {
        let assets = parse_pkl_assets(pkl_path);

        for asset in &assets {
            let mut entry = ChecksumEntry {
                asset_id: asset.id.clone(),
                filename: String::new(),
                file_exists: false,
                expected_hash: asset.hash.clone(),
                computed_hash: String::new(),
                hash_match: true,
                expected_size: asset.size,
                actual_size: 0,
                size_match: true,
            };

            // Resolve file path
            let file_path = resolve_asset_path(
                &opts.package_dir,
                &asset.id,
                &asset.original_filename,
                &asset_paths,
            );

            entry.filename = file_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            entry.file_exists = file_path.exists();

            if !entry.file_exists {
                result.missing_files += 1;
                result.entries.push(entry);
                result.total_assets += 1;
                if opts.stop_on_first_error {
                    result.all_valid = false;
                    result.success = true;
                    return result;
                }
                continue;
            }

            // Size check
            if opts.verify_sizes && entry.expected_size > 0 {
                entry.actual_size = std::fs::metadata(&file_path)
                    .map(|m| m.len() as i64)
                    .unwrap_or(0);
                entry.size_match = entry.actual_size == entry.expected_size;
                if !entry.size_match {
                    result.size_mismatches += 1;
                }
            }

            // Hash check
            if opts.verify_hashes && !entry.expected_hash.is_empty() {
                match postkit::hash::hash_file(&file_path, postkit::hash::HashAlgorithm::Sha1) {
                    Ok(hash_result) => {
                        entry.computed_hash = hash_result.base64.clone();
                        entry.hash_match = entry.computed_hash == entry.expected_hash;
                        if !entry.hash_match {
                            result.hash_mismatches += 1;
                        }
                    }
                    Err(_) => {
                        entry.hash_match = false;
                        result.hash_mismatches += 1;
                    }
                }
            }

            if entry.hash_match && entry.size_match && entry.file_exists {
                result.verified_ok += 1;
            }

            result.total_assets += 1;
            let failed = !entry.hash_match || !entry.size_match;
            result.entries.push(entry);

            if opts.stop_on_first_error && failed {
                result.all_valid = false;
                result.success = true;
                return result;
            }
        }
    }

    result.all_valid =
        result.hash_mismatches == 0 && result.size_mismatches == 0 && result.missing_files == 0;
    result.success = true;
    result
}

// --- Internal helpers ---

struct PklAsset {
    id: String,
    hash: String,
    size: i64,
    original_filename: String,
}

fn find_pkl_files(dir: &Path) -> Vec<PathBuf> {
    let mut pkls = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return pkls;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if name.contains("PKL") || name.contains("pkl") {
            pkls.push(path);
            continue;
        }
        if path.extension().is_some_and(|e| e == "xml")
            && let Ok(content) = std::fs::read_to_string(&path)
            && content[..content.len().min(512)].contains("PackingList")
        {
            pkls.push(path);
        }
    }
    pkls
}

fn parse_assetmap(dir: &Path) -> Vec<(String, String)> {
    let am_path = if dir.join("ASSETMAP.xml").exists() {
        dir.join("ASSETMAP.xml")
    } else if dir.join("ASSETMAP").exists() {
        dir.join("ASSETMAP")
    } else {
        return Vec::new();
    };

    let Ok(content) = std::fs::read_to_string(&am_path) else {
        return Vec::new();
    };

    let re = regex_lite::Regex::new(
        r"<Asset>[\s\S]*?<Id>urn:uuid:([^<]+)</Id>[\s\S]*?<Path>([^<]+)</Path>[\s\S]*?</Asset>",
    )
    .unwrap();

    re.captures_iter(&content)
        .map(|cap| (cap[1].to_string(), cap[2].to_string()))
        .collect()
}

fn parse_pkl_assets(pkl_path: &Path) -> Vec<PklAsset> {
    let Ok(content) = std::fs::read_to_string(pkl_path) else {
        return Vec::new();
    };

    let asset_re = regex_lite::Regex::new(r"<Asset>[\s\S]*?</Asset>").unwrap();
    let id_re = regex_lite::Regex::new(r"<Id>urn:uuid:([^<]+)</Id>").unwrap();
    let hash_re = regex_lite::Regex::new(r"<Hash>([^<]+)</Hash>").unwrap();
    let size_re = regex_lite::Regex::new(r"<Size>([^<]+)</Size>").unwrap();
    let filename_re =
        regex_lite::Regex::new(r"<OriginalFileName>([^<]+)</OriginalFileName>").unwrap();

    asset_re
        .find_iter(&content)
        .map(|m| {
            let block = m.as_str();
            PklAsset {
                id: id_re
                    .captures(block)
                    .map(|c| c[1].to_string())
                    .unwrap_or_default(),
                hash: hash_re
                    .captures(block)
                    .map(|c| c[1].to_string())
                    .unwrap_or_default(),
                size: size_re
                    .captures(block)
                    .and_then(|c| c[1].parse().ok())
                    .unwrap_or(0),
                original_filename: filename_re
                    .captures(block)
                    .map(|c| c[1].to_string())
                    .unwrap_or_default(),
            }
        })
        .collect()
}

fn resolve_asset_path(
    pkg_dir: &Path,
    asset_id: &str,
    original_filename: &str,
    asset_map: &[(String, String)],
) -> PathBuf {
    if !original_filename.is_empty() {
        return pkg_dir.join(original_filename);
    }

    if let Some((_, rel_path)) = asset_map.iter().find(|(id, _)| id == asset_id) {
        return pkg_dir.join(rel_path);
    }

    // Fallback: find file containing the UUID in its name
    if let Ok(entries) = std::fs::read_dir(pkg_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.contains(asset_id) {
                return entry.path();
            }
        }
    }

    pkg_dir.join(asset_id)
}
