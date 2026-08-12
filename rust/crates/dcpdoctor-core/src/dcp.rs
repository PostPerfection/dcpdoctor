use std::path::{Path, PathBuf};

use crate::assetmap::{AssetMap, ParseXmlFile};
use crate::cpl::Cpl;
use crate::pkl::Pkl;
use crate::{Code, Note, Severity, Standard};

/// Detect DCP standard from the asset map's root namespace. Never from the file
/// name: that is the thing `assetmap_invalid_name` exists to catch, so a package
/// under the wrong name would otherwise be validated as the wrong standard.
pub fn detect_standard(dcp_dir: &Path) -> Standard {
    let Some(am_path) = find_assetmap(dcp_dir) else {
        return Standard::Unknown;
    };
    let Ok(content) = std::fs::read_to_string(&am_path) else {
        return Standard::Unknown;
    };
    standard_of_assetmap(&content)
}

/// The standard an asset map document declares. Matches on the namespace
/// authority rather than one exact URI, the same test `schema_file_for` uses, so
/// the IMF asset map namespace still reads as SMPTE.
pub fn standard_of_assetmap(xml: &str) -> Standard {
    let Some(namespace) = crate::schema::root_namespace(xml) else {
        return Standard::Unknown;
    };
    if namespace.contains("digicine.com") {
        Standard::Interop
    } else if namespace.contains("smpte-ra.org") {
        Standard::Smpte
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

#[cfg(test)]
mod tests {
    use super::*;

    fn package_with(am_name: &str, namespace: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(am_name),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<AssetMap xmlns="{namespace}">
  <Id>urn:uuid:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee</Id>
  <AssetList/>
</AssetMap>"#
            ),
        )
        .unwrap();
        dir
    }

    const SMPTE_AM: &str = "http://www.smpte-ra.org/schemas/429-9/2007/AM";
    const INTEROP_AM: &str = "http://www.digicine.com/PROTO-ASDCP-AM-20040311#";

    #[test]
    fn namespace_beats_the_filename_in_both_directions() {
        // the two signals disagree on purpose: this is what the filename
        // derivation used to get wrong
        let smpte_under_interop_name = package_with("ASSETMAP", SMPTE_AM);
        assert_eq!(
            detect_standard(smpte_under_interop_name.path()),
            Standard::Smpte,
            "a SMPTE-namespace asset map named ASSETMAP is still SMPTE"
        );

        let interop_under_smpte_name = package_with("ASSETMAP.xml", INTEROP_AM);
        assert_eq!(
            detect_standard(interop_under_smpte_name.path()),
            Standard::Interop,
            "an Interop-namespace asset map named ASSETMAP.xml is still Interop"
        );
    }

    #[test]
    fn agreeing_signals_still_resolve() {
        assert_eq!(
            detect_standard(package_with("ASSETMAP.xml", SMPTE_AM).path()),
            Standard::Smpte
        );
        assert_eq!(
            detect_standard(package_with("ASSETMAP", INTEROP_AM).path()),
            Standard::Interop
        );
    }

    #[test]
    fn imf_assetmap_namespace_still_reads_as_smpte() {
        let dir = package_with(
            "ASSETMAP.xml",
            "http://www.smpte-ra.org/schemas/2067-2/2016/AM",
        );
        assert_eq!(detect_standard(dir.path()), Standard::Smpte);
    }

    #[test]
    fn missing_or_unrecognised_assetmap_is_unknown() {
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(detect_standard(empty.path()), Standard::Unknown);
        let odd = package_with("ASSETMAP.xml", "http://example.invalid/AM");
        assert_eq!(detect_standard(odd.path()), Standard::Unknown);
    }
}
