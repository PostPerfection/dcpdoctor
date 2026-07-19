use std::path::{Path, PathBuf};

use crate::assetmap::{AssetMap, ParseXmlFile};
use crate::cpl::Cpl;
use crate::pkl::Pkl;
use crate::{Code, Note, Severity, Standard};

/// Detect DCP standard from directory contents.
pub fn detect_standard(dcp_dir: &Path) -> Standard {
    if dcp_dir.join("ASSETMAP.xml").exists() {
        Standard::Smpte
    } else if dcp_dir.join("ASSETMAP").exists() {
        Standard::Interop
    } else {
        Standard::Unknown
    }
}

/// Find the ASSETMAP file in a DCP directory.
pub fn find_assetmap(dcp_dir: &Path) -> Option<PathBuf> {
    let smpte = dcp_dir.join("ASSETMAP.xml");
    if smpte.exists() {
        return Some(smpte);
    }
    let interop = dcp_dir.join("ASSETMAP");
    if interop.exists() {
        return Some(interop);
    }
    None
}

/// Parsed DCP structure.
pub struct Dcp {
    pub standard: Standard,
    pub assetmap: AssetMap,
    pub assetmap_path: PathBuf,
    pub pkls: Vec<(PathBuf, Pkl)>,
    pub cpls: Vec<(PathBuf, Cpl)>,
}

/// Open and parse a DCP directory.
pub fn open_dcp(dcp_dir: &Path) -> Result<Dcp, Vec<Note>> {
    let mut errors = Vec::new();

    if !dcp_dir.is_dir() {
        errors.push(Note {
            severity: Severity::Error,
            code: Code::MissingAssetmap,
            message: format!("Path is not a directory: {}", dcp_dir.display()),
            file: Some(dcp_dir.to_path_buf()),
            line: 0,
        });
        return Err(errors);
    }

    let standard = detect_standard(dcp_dir);

    let assetmap_path = match find_assetmap(dcp_dir) {
        Some(p) => p,
        None => {
            errors.push(Note {
                severity: Severity::Error,
                code: Code::MissingAssetmap,
                message: "No ASSETMAP or ASSETMAP.xml found".to_string(),
                file: Some(dcp_dir.to_path_buf()),
                line: 0,
            });
            return Err(errors);
        }
    };

    let assetmap = match AssetMap::parse(&assetmap_path) {
        Some(am) => am,
        None => {
            errors.push(Note {
                severity: Severity::Error,
                code: Code::XmlParseError,
                message: "Failed to parse ASSETMAP".to_string(),
                file: Some(assetmap_path),
                line: 0,
            });
            return Err(errors);
        }
    };

    // Find PKLs and CPLs among the assets
    let mut pkls = Vec::new();
    let mut cpls = Vec::new();

    for asset in &assetmap.assets {
        let full_path = dcp_dir.join(&asset.path);
        if !full_path.exists() {
            continue;
        }

        // Only try XML files or files named PKL/CPL
        let ext = full_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let fname = full_path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        if ext != "xml" && !fname.contains("PKL") && !fname.contains("pkl") {
            // Try CPL
            if (ext == "xml" || fname.contains("CPL") || fname.contains("cpl"))
                && let Some(cpl) = Cpl::parse(&full_path)
            {
                cpls.push((full_path, cpl));
            }
            continue;
        }

        // Try as PKL first
        if let Some(pkl) = Pkl::parse(&full_path) {
            pkls.push((full_path, pkl));
            continue;
        }

        // Try as CPL
        if let Some(cpl) = Cpl::parse(&full_path) {
            cpls.push((full_path, cpl));
        }
    }

    Ok(Dcp {
        standard,
        assetmap,
        assetmap_path,
        pkls,
        cpls,
    })
}
