use serde::{Deserialize, Serialize};
use std::path::Path;

/// Difference between two DCPs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiffResult {
    pub differences: Vec<DiffEntry>,
    pub identical: bool,
}

/// A single difference entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffEntry {
    pub category: String,
    pub description: String,
    pub value_a: String,
    pub value_b: String,
}

/// Compare two DCP directories.
pub fn diff_dcps(dcp_a: &Path, dcp_b: &Path, compare_hashes: bool) -> DiffResult {
    let mut result = DiffResult::default();

    let dcp_a = match crate::dcp::open_dcp(dcp_a) {
        Ok(d) => d,
        Err(_) => {
            result.differences.push(DiffEntry {
                category: "structure".to_string(),
                description: "Failed to open DCP A".to_string(),
                value_a: String::new(),
                value_b: String::new(),
            });
            return result;
        }
    };

    let dcp_b = match crate::dcp::open_dcp(dcp_b) {
        Ok(d) => d,
        Err(_) => {
            result.differences.push(DiffEntry {
                category: "structure".to_string(),
                description: "Failed to open DCP B".to_string(),
                value_a: String::new(),
                value_b: String::new(),
            });
            return result;
        }
    };

    // Compare standards
    if dcp_a.standard != dcp_b.standard {
        result.differences.push(DiffEntry {
            category: "standard".to_string(),
            description: "Different DCP standard".to_string(),
            value_a: format!("{}", dcp_a.standard),
            value_b: format!("{}", dcp_b.standard),
        });
    }

    // Compare asset counts
    if dcp_a.assetmap.assets.len() != dcp_b.assetmap.assets.len() {
        result.differences.push(DiffEntry {
            category: "structure".to_string(),
            description: "Different number of assets".to_string(),
            value_a: format!("{}", dcp_a.assetmap.assets.len()),
            value_b: format!("{}", dcp_b.assetmap.assets.len()),
        });
    }

    // Compare CPL count
    if dcp_a.cpls.len() != dcp_b.cpls.len() {
        result.differences.push(DiffEntry {
            category: "structure".to_string(),
            description: "Different number of CPLs".to_string(),
            value_a: format!("{}", dcp_a.cpls.len()),
            value_b: format!("{}", dcp_b.cpls.len()),
        });
    }

    // Compare CPL titles
    for (i, ((_, cpl_a), (_, cpl_b))) in dcp_a.cpls.iter().zip(dcp_b.cpls.iter()).enumerate() {
        if cpl_a.content_title != cpl_b.content_title {
            result.differences.push(DiffEntry {
                category: "content".to_string(),
                description: format!("CPL {} title differs", i + 1),
                value_a: cpl_a.content_title.clone(),
                value_b: cpl_b.content_title.clone(),
            });
        }
    }

    // Compare asset hashes if requested
    if compare_hashes {
        use std::collections::HashMap;

        let dir_a = dcp_a.assetmap_path.parent().unwrap_or(Path::new("."));
        let dir_b = dcp_b.assetmap_path.parent().unwrap_or(Path::new("."));

        // Build maps of asset ID → path for both DCPs
        let assets_a: HashMap<&str, &str> = dcp_a
            .assetmap
            .assets
            .iter()
            .map(|a| (a.id.as_str(), a.path.as_str()))
            .collect();

        let assets_b: HashMap<&str, &str> = dcp_b
            .assetmap
            .assets
            .iter()
            .map(|a| (a.id.as_str(), a.path.as_str()))
            .collect();

        // Compare hashes for matching asset IDs
        for (id, path_a) in &assets_a {
            if let Some(path_b) = assets_b.get(id) {
                let full_a = dir_a.join(path_a);
                let full_b = dir_b.join(path_b);

                if !full_a.exists() || !full_b.exists() {
                    result.differences.push(DiffEntry {
                        category: "hash".to_string(),
                        description: format!("Asset {id} not compared, file missing"),
                        value_a: presence(&full_a),
                        value_b: presence(&full_b),
                    });
                    continue;
                }

                match (
                    crate::hash::sha1_base64(&full_a),
                    crate::hash::sha1_base64(&full_b),
                ) {
                    (Ok(a), Ok(b)) if a == b => {}
                    (Ok(a), Ok(b)) => {
                        result.differences.push(DiffEntry {
                            category: "hash".to_string(),
                            description: format!("Asset {id} has different content"),
                            value_a: a,
                            value_b: b,
                        });
                    }
                    (a, b) => {
                        result.differences.push(DiffEntry {
                            category: "hash".to_string(),
                            description: format!("Asset {id} not compared, hashing failed"),
                            value_a: hash_outcome(&a),
                            value_b: hash_outcome(&b),
                        });
                    }
                }
            } else {
                result.differences.push(DiffEntry {
                    category: "structure".to_string(),
                    description: format!("Asset {id} only in DCP A"),
                    value_a: path_a.to_string(),
                    value_b: String::new(),
                });
            }
        }

        // Check for assets only in B
        for (id, path_b) in &assets_b {
            if !assets_a.contains_key(id) {
                result.differences.push(DiffEntry {
                    category: "structure".to_string(),
                    description: format!("Asset {id} only in DCP B"),
                    value_a: String::new(),
                    value_b: path_b.to_string(),
                });
            }
        }
    }

    result.identical = result.differences.is_empty();
    result
}

fn presence(path: &Path) -> String {
    if path.exists() {
        "present".to_string()
    } else {
        format!("missing: {}", path.display())
    }
}

fn hash_outcome(hash: &std::io::Result<String>) -> String {
    match hash {
        Ok(h) => h.clone(),
        Err(e) => format!("hash failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package_copy() -> tempfile::TempDir {
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/valid_smpte");
        let dir = tempfile::tempdir().unwrap();
        for entry in std::fs::read_dir(source).unwrap().flatten() {
            std::fs::copy(entry.path(), dir.path().join(entry.file_name())).unwrap();
        }
        dir
    }

    #[test]
    fn an_asset_missing_on_one_side_is_not_reported_as_identical() {
        let a = package_copy();
        let b = package_copy();
        std::fs::remove_file(b.path().join("picture.mxf")).unwrap();

        let result = diff_dcps(a.path(), b.path(), true);
        assert!(
            !result.identical,
            "an asset that was never compared cannot make the DCPs identical"
        );
        assert!(
            result
                .differences
                .iter()
                .any(|d| d.description.contains("not compared")),
            "differences: {:?}",
            result.differences
        );
    }
}
