//! Automatic repair of common DCP issues flagged by validation.

use std::path::Path;

use crate::dcp;
use crate::hash::sha1_base64;
use crate::{Code, Note, Severity, Standard, VerifyOptions};

/// A single repair action that was applied.
#[derive(Debug, Clone)]
pub struct Repair {
    pub code: Code,
    pub description: String,
    pub file: std::path::PathBuf,
}

/// Whether a run rewrites the package or only reports what it would rewrite.
/// One code path answers both, so a preview cannot drift from what `fix` applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixMode {
    Apply,
    DryRun,
}

impl FixMode {
    fn writes(self) -> bool {
        self == FixMode::Apply
    }
}

/// Result of a fix operation.
#[derive(Debug, Default)]
pub struct FixResult {
    pub repairs: Vec<Repair>,
    pub skipped: Vec<Note>,
}

impl FixResult {
    pub fn repair_count(&self) -> usize {
        self.repairs.len()
    }
}

/// Fix all auto-repairable issues in the given DCP directory, or under
/// `FixMode::DryRun` report the same repairs without writing a byte.
pub fn fix_dcp(dcp_dir: &Path, mode: FixMode) -> FixResult {
    let mut result = FixResult::default();

    let dcp = match dcp::open_dcp(dcp_dir) {
        Ok(d) => d,
        Err(notes) => {
            // Can't even parse the DCP — nothing to fix
            result.skipped = notes;
            return result;
        }
    };

    // First validate to find issues (use strict mode so all fixable issues surface)
    let opts = VerifyOptions {
        check_hashes: true,
        check_signatures: false,
        check_picture_details: false,
        strict_smpte: true,
        ..Default::default()
    };
    let verify_result = crate::validate::verify_dcp(dcp_dir, &opts);

    // PKL hash and size mismatches are repaired in the batch pass below. a note
    // that reaches neither list would vanish from the report.
    let mut pending_pkl_mismatches: Vec<Note> = Vec::new();

    for note in &verify_result.notes {
        match note.code {
            Code::PklHashMismatch | Code::PklSizeMismatch => {
                pending_pkl_mismatches.push(note.clone());
            }
            Code::SmpteNamespaceWrong | Code::InteropNamespaceWrong => {
                let fixed = note
                    .file
                    .as_ref()
                    .is_some_and(|file| fix_namespace(file, dcp.standard, mode));
                match (fixed, &note.file) {
                    (true, Some(file)) => result.repairs.push(Repair {
                        code: note.code,
                        description: format!(
                            "Namespace in {}",
                            file.file_name().unwrap_or_default().to_string_lossy()
                        ),
                        file: file.clone(),
                    }),
                    _ => result.skipped.push(note.clone()),
                }
            }
            Code::CplInvalidContentKind => {
                let fixed = note
                    .file
                    .as_ref()
                    .is_some_and(|file| fix_content_kind(file, mode));
                match (fixed, &note.file) {
                    (true, Some(file)) => result.repairs.push(Repair {
                        code: note.code,
                        description: format!(
                            "ContentKind in {}",
                            file.file_name().unwrap_or_default().to_string_lossy()
                        ),
                        file: file.clone(),
                    }),
                    _ => result.skipped.push(note.clone()),
                }
            }
            _ => {
                // Not auto-fixable
                result.skipped.push(note.clone());
            }
        }
    }

    // Batch fix: recompute PKL hashes
    let id_to_path: std::collections::HashMap<&str, &str> = dcp
        .assetmap
        .assets
        .iter()
        .map(|a| (a.id.as_str(), a.path.as_str()))
        .collect();

    let mut repaired_assets: std::collections::HashSet<std::path::PathBuf> =
        std::collections::HashSet::new();
    let mut failure_reasons: std::collections::HashMap<std::path::PathBuf, String> =
        std::collections::HashMap::new();

    for (pkl_path, pkl) in &dcp.pkls {
        let mut pkl_modified = false;
        let repairs_before_this_pkl = result.repairs.len();
        let mut rewritten_in_this_pkl: Vec<std::path::PathBuf> = Vec::new();
        let mut xml = match std::fs::read_to_string(pkl_path) {
            Ok(s) => s,
            Err(e) => {
                result.skipped.push(Note {
                    severity: Severity::Error,
                    code: Code::PklHashMismatch,
                    message: format!("Cannot read PKL to rewrite hashes: {e}"),
                    file: Some(pkl_path.clone()),
                    line: 0,
                });
                continue;
            }
        };

        for pkl_asset in &pkl.assets {
            let Some(&asset_rel) = id_to_path.get(pkl_asset.id.as_str()) else {
                continue;
            };
            let full_path = dcp_dir.join(asset_rel);
            if !full_path.exists() {
                failure_reasons.insert(full_path, "asset file not found".into());
                continue;
            }
            let mut asset_rewritten = false;

            if pkl_asset.hash.is_empty() {
                failure_reasons.insert(full_path.clone(), "the PKL records no hash".into());
            } else {
                match sha1_base64(&full_path) {
                    Ok(computed) if computed != pkl_asset.hash => {
                        match replace_asset_element(
                            &xml,
                            &pkl_asset.id,
                            "Hash",
                            &pkl_asset.hash,
                            &computed,
                        ) {
                            Some(rewritten) => {
                                xml = rewritten;
                                asset_rewritten = true;
                                result.repairs.push(Repair {
                                    code: Code::PklHashMismatch,
                                    description: format!("Hash for {asset_rel} in PKL"),
                                    file: pkl_path.clone(),
                                });
                            }
                            None => {
                                failure_reasons.insert(
                                    full_path.clone(),
                                    "the hash the PKL records was not found in its text".into(),
                                );
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        failure_reasons
                            .insert(full_path.clone(), format!("could not hash the asset: {e}"));
                    }
                }
            }

            // a repaired hash without its size leaves the package failing on
            // pkl_size_mismatch instead
            let recorded_size = pkl_asset.size.to_string();
            let computed_size = std::fs::metadata(&full_path).map(|metadata| metadata.len());
            match computed_size {
                Ok(computed_size) if computed_size.to_string() != recorded_size => {
                    match replace_asset_element(
                        &xml,
                        &pkl_asset.id,
                        "Size",
                        &recorded_size,
                        &computed_size.to_string(),
                    ) {
                        Some(rewritten) => {
                            xml = rewritten;
                            asset_rewritten = true;
                            result.repairs.push(Repair {
                                code: Code::PklSizeMismatch,
                                description: format!("Size for {asset_rel} in PKL"),
                                file: pkl_path.clone(),
                            });
                        }
                        None => {
                            failure_reasons.insert(
                                full_path.clone(),
                                "the size the PKL records was not found in its text".into(),
                            );
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    failure_reasons.insert(
                        full_path.clone(),
                        format!("could not measure the asset: {e}"),
                    );
                }
            }

            if asset_rewritten {
                pkl_modified = true;
                repaired_assets.insert(full_path.clone());
                rewritten_in_this_pkl.push(full_path);
            }
        }

        if pkl_modified
            && mode.writes()
            && let Err(e) = std::fs::write(pkl_path, &xml)
        {
            result.skipped.push(Note {
                severity: Severity::Error,
                code: Code::PklHashMismatch,
                message: format!("Failed to write updated PKL: {}", e),
                file: Some(pkl_path.clone()),
                line: 0,
            });
            // the new hashes never reached the file, so take the repairs back
            result.repairs.truncate(repairs_before_this_pkl);
            for asset in rewritten_in_this_pkl {
                repaired_assets.remove(&asset);
                failure_reasons.insert(asset, format!("the updated PKL could not be written: {e}"));
            }
        }
    }

    // a repair can clear a finding this run already collected: putting the CPL
    // namespace back settles the schema violation the wrong one caused. re-read
    // the package without re-hashing it and keep only what still stands. the
    // PKL mismatches are settled below instead, from the repairs themselves.
    if mode.writes() && !result.repairs.is_empty() {
        let after = crate::validate::verify_dcp(
            dcp_dir,
            &VerifyOptions {
                check_hashes: false,
                ..opts.clone()
            },
        );
        result.skipped.retain(|note| {
            matches!(note.code, Code::PklHashMismatch | Code::PklSizeMismatch)
                || after
                    .notes
                    .iter()
                    .any(|remaining| remaining.code == note.code && remaining.file == note.file)
        });
    }

    for note in pending_pkl_mismatches {
        let repaired = note
            .file
            .as_ref()
            .is_some_and(|f| repaired_assets.contains(f));
        if repaired {
            continue;
        }
        let reason = note
            .file
            .as_ref()
            .and_then(|f| failure_reasons.get(f))
            .cloned()
            .unwrap_or_else(|| "no PKL entry matched this asset".into());
        result.skipped.push(Note {
            message: format!("{}, not repaired: {reason}", note.message),
            ..note
        });
    }

    result
}

/// Rewrite the text of `local` inside the `<Asset>` whose `<Id>` is `asset_id`,
/// so the same string elsewhere in the PKL is left alone. Returns None when no
/// asset carries that id or when the element's text is not `recorded`.
fn replace_asset_element(
    xml: &str,
    asset_id: &str,
    local: &str,
    recorded: &str,
    computed: &str,
) -> Option<String> {
    let asset = asset_element_range(xml, asset_id)?;
    let element_text = element_text_range(xml, asset, local)?;
    let text = &xml[element_text.clone()];
    if text.trim() != recorded {
        return None;
    }
    let start = element_text.start + (text.len() - text.trim_start().len());
    let end = start + recorded.len();
    Some(format!("{}{computed}{}", &xml[..start], &xml[end..]))
}

/// Byte range from the `<Asset>` open tag to its `</Asset>` close tag, for the
/// asset whose `<Id>` is `asset_id`.
fn asset_element_range(xml: &str, asset_id: &str) -> Option<std::ops::Range<usize>> {
    for (open, _) in xml.match_indices('<') {
        if local_name_at(xml, open) != Some(("Asset", false)) {
            continue;
        }
        let Some(close) = find_close_tag(xml, open + 1..xml.len(), "Asset") else {
            continue;
        };
        let asset = open..close;
        let Some(id_text) = element_text_range(xml, asset.clone(), "Id") else {
            continue;
        };
        if dcpdoctor_parse::strip_urn_uuid(xml[id_text].trim()) == asset_id {
            return Some(asset);
        }
    }
    None
}

/// Byte range of the text inside the first element named `local` within
/// `region`. Any namespace prefix is accepted.
fn element_text_range(
    xml: &str,
    region: std::ops::Range<usize>,
    local: &str,
) -> Option<std::ops::Range<usize>> {
    for (offset, _) in xml.get(region.clone())?.match_indices('<') {
        let open = region.start + offset;
        if local_name_at(xml, open) != Some((local, false)) {
            continue;
        }
        let text_start = open + xml[open..region.end].find('>')? + 1;
        let close = find_close_tag(xml, text_start..region.end, local)?;
        return Some(text_start..close);
    }
    None
}

/// Offset of the `<` of the first closing tag named `local` within `region`.
fn find_close_tag(xml: &str, region: std::ops::Range<usize>, local: &str) -> Option<usize> {
    xml.get(region.clone())?
        .match_indices('<')
        .map(|(offset, _)| region.start + offset)
        .find(|&open| local_name_at(xml, open) == Some((local, true)))
}

/// Local name of the tag starting at `open`, plus whether it is a closing tag.
/// None for comments, declarations and anything unparseable.
fn local_name_at(xml: &str, open: usize) -> Option<(&str, bool)> {
    let rest = xml.get(open + 1..)?;
    let (closing, rest) = match rest.strip_prefix('/') {
        Some(rest) => (true, rest),
        None => (false, rest),
    };
    if rest.starts_with(['?', '!']) {
        return None;
    }
    let name_end = rest.find(|c: char| c.is_whitespace() || c == '>' || c == '/')?;
    let name = &rest[..name_end];
    Some((name.rsplit(':').next()?, closing))
}

/// Fix XML namespace to match detected standard.
fn fix_namespace(file: &Path, standard: Standard, mode: FixMode) -> bool {
    let xml = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(_) => return false,
    };

    let (wrong, correct) = match standard {
        Standard::Smpte => (
            "http://www.digicine.com/PROTO-ASDCP-CPL-20040511#",
            "http://www.smpte-ra.org/schemas/429-7/2006/CPL",
        ),
        Standard::Interop => (
            "http://www.smpte-ra.org/schemas/429-7/2006/CPL",
            "http://www.digicine.com/PROTO-ASDCP-CPL-20040511#",
        ),
        Standard::Unknown => return false,
    };

    if !xml.contains(wrong) {
        return false;
    }
    if !mode.writes() {
        return true;
    }

    let fixed = xml.replace(wrong, correct);
    std::fs::write(file, &fixed).is_ok()
}

/// Normalize ContentKind to lowercase canonical form.
fn fix_content_kind(file: &Path, mode: FixMode) -> bool {
    let xml = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(_) => return false,
    };

    // Find <ContentKind>...</ContentKind> and normalize the value
    let start_tag = "<ContentKind>";
    let end_tag = "</ContentKind>";
    let start = match xml.find(start_tag) {
        Some(i) => i + start_tag.len(),
        None => return false,
    };
    let end = match xml[start..].find(end_tag) {
        Some(i) => start + i,
        None => return false,
    };

    let original = &xml[start..end];
    let normalized = normalize_content_kind(original.trim());

    if normalized == original.trim() {
        return false;
    }
    if !mode.writes() {
        return true;
    }

    let fixed = format!("{}{}{}", &xml[..start], normalized, &xml[end..]);
    std::fs::write(file, &fixed).is_ok()
}

/// Map common misspellings/variants to the canonical SMPTE content kinds.
fn normalize_content_kind(kind: &str) -> &'static str {
    match kind.to_lowercase().as_str() {
        "feature" | "features" | "feature film" => "feature",
        "trailer" | "trailers" => "trailer",
        "test" | "testing" => "test",
        "teaser" | "teasers" => "teaser",
        "rating" | "ratings" | "rating card" => "rating",
        "advertisement" | "ad" | "advert" | "advertising" => "advertisement",
        "short" | "shorts" | "short film" => "short",
        "transitional" | "transition" => "transitional",
        "psa" | "public service" => "psa",
        "policy" | "policies" | "policy trailer" => "policy",
        "episode" | "episodes" => "episode",
        _ => "feature", // Default fallback for unknown kinds
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Copy the committed SMPTE package into a temp dir with `edits` applied to
    /// `file`, so each case is one deliberate deviation from a real package.
    fn mutated_package(file: &str, edits: &[(&str, &str)]) -> TempDir {
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/valid_smpte");
        let dir = TempDir::new().unwrap();
        for entry in fs::read_dir(source).unwrap().flatten() {
            fs::copy(entry.path(), dir.path().join(entry.file_name())).unwrap();
        }
        let target = dir.path().join(file);
        let mut xml = fs::read_to_string(&target).unwrap();
        for (from, to) in edits {
            assert!(xml.contains(from), "the package's {file} has no {from:?}");
            xml = xml.replace(from, to);
        }
        fs::write(&target, xml).unwrap();
        dir
    }

    #[test]
    fn a_content_kind_the_repair_cannot_reach_is_reported_as_skipped() {
        // a scope attribute is legal in SMPTE CPLs and hides the element from
        // the repair, which searches for the bare "<ContentKind>" tag
        let dir = mutated_package(
            "cpl.xml",
            &[(
                "<ContentKind>test</ContentKind>",
                r#"<ContentKind scope="http://www.smpte-ra.org/schemas/429-7/2006/CPL#standard-content">nonsense</ContentKind>"#,
            )],
        );

        let result = fix_dcp(dir.path(), FixMode::Apply);

        assert!(
            !result
                .repairs
                .iter()
                .any(|r| r.code == Code::CplInvalidContentKind),
            "nothing was rewritten, so no repair may be claimed: {:?}",
            result.repairs
        );
        assert!(
            result
                .skipped
                .iter()
                .any(|n| n.code == Code::CplInvalidContentKind),
            "the unrepaired ContentKind must reach the report: {:?}",
            result.skipped
        );
    }

    /// The stale hash of sound.mxf, planted a second time in the picture asset
    /// that precedes it.
    const STALE_SOUND_HASH: &str = "AAAAOYm1qkf6RkMLA7PgTU/KY7s=";

    #[test]
    fn a_stale_hash_that_also_appears_earlier_is_rewritten_only_in_its_own_asset() {
        let dir = mutated_package(
            "pkl.xml",
            &[
                (
                    "<Hash>DvGtOYm1qkf6RkMLA7PgTU/KY7s=</Hash>",
                    "<Hash>AAAAOYm1qkf6RkMLA7PgTU/KY7s=</Hash>",
                ),
                (
                    "<Id>urn:uuid:148971a4-abc6-44ae-bf59-34026d0faf17</Id>",
                    "<Id>urn:uuid:148971a4-abc6-44ae-bf59-34026d0faf17</Id>\n      <AnnotationText>AAAAOYm1qkf6RkMLA7PgTU/KY7s=</AnnotationText>",
                ),
            ],
        );

        let result = fix_dcp(dir.path(), FixMode::Apply);

        assert!(
            result
                .repairs
                .iter()
                .any(|r| r.code == Code::PklHashMismatch && r.description.contains("sound.mxf")),
            "the sound hash must be repaired: {:?}",
            result.repairs
        );
        let xml = fs::read_to_string(dir.path().join("pkl.xml")).unwrap();
        assert!(
            xml.contains(&format!(
                "<AnnotationText>{STALE_SOUND_HASH}</AnnotationText>"
            )),
            "the earlier occurrence must survive byte for byte: {xml}"
        );
        assert_eq!(
            xml.matches(STALE_SOUND_HASH).count(),
            1,
            "only the sound asset's Hash may be rewritten: {xml}"
        );
        assert!(
            xml.contains("<Hash>DvGtOYm1qkf6RkMLA7PgTU/KY7s=</Hash>"),
            "the sound asset must carry its computed hash: {xml}"
        );
        assert!(
            xml.contains("<Hash>pDjIK8UaYOLZLpbbBBI0hFVQbXE=</Hash>"),
            "the picture asset's own hash must be untouched: {xml}"
        );
    }

    #[test]
    fn a_hash_the_asset_element_does_not_carry_leaves_the_rest_of_the_pkl_alone() {
        // the numeric reference parses to the same hash the file does not spell
        // out, so the recorded hash is nowhere in this asset's text
        let dir = mutated_package(
            "pkl.xml",
            &[
                (
                    "<Hash>DvGtOYm1qkf6RkMLA7PgTU/KY7s=</Hash>",
                    "<Hash>AAAAOYm1qkf6RkMLA7PgTU/KY7s&#61;</Hash>",
                ),
                (
                    "<Id>urn:uuid:148971a4-abc6-44ae-bf59-34026d0faf17</Id>",
                    "<Id>urn:uuid:148971a4-abc6-44ae-bf59-34026d0faf17</Id>\n      <AnnotationText>AAAAOYm1qkf6RkMLA7PgTU/KY7s=</AnnotationText>",
                ),
            ],
        );
        let pkl = dir.path().join("pkl.xml");
        let before = fs::read_to_string(&pkl).unwrap();

        let result = fix_dcp(dir.path(), FixMode::Apply);

        assert!(
            !result
                .repairs
                .iter()
                .any(|r| r.code == Code::PklHashMismatch),
            "no hash was found to rewrite, so no repair may be claimed: {:?}",
            result.repairs
        );
        assert!(
            result
                .skipped
                .iter()
                .any(|n| n.code == Code::PklHashMismatch
                    && n.message.contains("was not found in its text")),
            "the unrepaired hash mismatch must reach the report: {:?}",
            result.skipped
        );
        assert_eq!(
            fs::read_to_string(&pkl).unwrap(),
            before,
            "the PKL must be left byte for byte as it was"
        );
    }

    #[test]
    fn a_hash_mismatch_the_repair_cannot_write_is_reported_as_skipped() {
        let dir = mutated_package(
            "pkl.xml",
            &[(
                "<Hash>pDjIK8UaYOLZLpbbBBI0hFVQbXE=</Hash>",
                "<Hash>AAAAK8UaYOLZLpbbBBI0hFVQbXE=</Hash>",
            )],
        );
        let pkl = dir.path().join("pkl.xml");
        let mut permissions = fs::metadata(&pkl).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&pkl, permissions).unwrap();
        if fs::OpenOptions::new().write(true).open(&pkl).is_ok() {
            // running as root, where read-only says nothing about writability
            return;
        }

        let result = fix_dcp(dir.path(), FixMode::Apply);

        assert!(
            !result
                .repairs
                .iter()
                .any(|r| r.code == Code::PklHashMismatch),
            "the PKL was never written, so no repair may be claimed: {:?}",
            result.repairs
        );
        assert!(
            result
                .skipped
                .iter()
                .any(|n| n.code == Code::PklHashMismatch && n.message.contains("not repaired")),
            "the unrepaired hash mismatch must reach the report: {:?}",
            result.skipped
        );
    }

    #[test]
    fn test_normalize_content_kind_variants() {
        assert_eq!(normalize_content_kind("Feature"), "feature");
        assert_eq!(normalize_content_kind("TRAILER"), "trailer");
        assert_eq!(normalize_content_kind("ad"), "advertisement");
        assert_eq!(normalize_content_kind("short film"), "short");
        assert_eq!(normalize_content_kind("PSA"), "psa");
    }

    #[test]
    fn test_fix_content_kind_in_xml() {
        let dir = TempDir::new().unwrap();
        let cpl = dir.path().join("cpl.xml");
        fs::write(
            &cpl,
            r#"<?xml version="1.0"?><CompositionPlaylist><ContentKind>TRAILER</ContentKind></CompositionPlaylist>"#,
        )
        .unwrap();

        assert!(fix_content_kind(&cpl, FixMode::Apply));
        let result = fs::read_to_string(&cpl).unwrap();
        assert!(result.contains("<ContentKind>trailer</ContentKind>"));
    }

    #[test]
    fn test_fix_content_kind_already_correct() {
        let dir = TempDir::new().unwrap();
        let cpl = dir.path().join("cpl.xml");
        fs::write(
            &cpl,
            r#"<?xml version="1.0"?><CompositionPlaylist><ContentKind>feature</ContentKind></CompositionPlaylist>"#,
        )
        .unwrap();

        assert!(!fix_content_kind(&cpl, FixMode::Apply));
    }

    #[test]
    fn test_fix_namespace_smpte() {
        let dir = TempDir::new().unwrap();
        let cpl = dir.path().join("cpl.xml");
        fs::write(
            &cpl,
            r#"<CompositionPlaylist xmlns="http://www.digicine.com/PROTO-ASDCP-CPL-20040511#"><Id>urn:uuid:test</Id></CompositionPlaylist>"#,
        )
        .unwrap();

        assert!(fix_namespace(&cpl, Standard::Smpte, FixMode::Apply));
        let result = fs::read_to_string(&cpl).unwrap();
        assert!(result.contains("http://www.smpte-ra.org/schemas/429-7/2006/CPL"));
    }
}
