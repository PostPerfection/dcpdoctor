//! Automatic repair of common DCP issues flagged by validation.

use std::path::{Path, PathBuf};

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

/// Whether `fix` may rewrite a CPL or PKL that carries an XML signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignedDocuments {
    /// Leave the package as it is when a repair would rewrite a signed document.
    Refuse,
    /// Write the repairs and report every signature they invalidate.
    Break,
}

/// Result of a fix operation.
#[derive(Debug, Default)]
pub struct FixResult {
    pub repairs: Vec<Repair>,
    pub skipped: Vec<Note>,
    pub signature_notes: Vec<Note>,
}

impl FixResult {
    pub fn repair_count(&self) -> usize {
        self.repairs.len()
    }
}

// the CPL and PKL text as the repairs leave it, so a dry run hashes what a real run writes
#[derive(Default)]
struct DocumentEdits {
    edited: std::collections::BTreeMap<PathBuf, String>,
}

impl DocumentEdits {
    fn text(&self, path: &Path) -> std::io::Result<String> {
        match self.edited.get(path) {
            Some(text) => Ok(text.clone()),
            None => std::fs::read_to_string(path),
        }
    }

    fn set(&mut self, path: &Path, text: String) {
        self.edited.insert(path.to_path_buf(), text);
    }

    fn hash_and_size(&self, path: &Path) -> std::io::Result<(String, u64)> {
        match self.edited.get(path) {
            Some(text) => Ok((sha1_base64_of(text.as_bytes()), text.len() as u64)),
            None => Ok((sha1_base64(path)?, std::fs::metadata(path)?.len())),
        }
    }
}

fn sha1_base64_of(bytes: &[u8]) -> String {
    use base64::Engine as _;
    use sha1::Digest as _;
    base64::engine::general_purpose::STANDARD.encode(sha1::Sha1::digest(bytes))
}

type DocumentRewrite = fn(&str, Standard) -> Option<String>;

/// Fix all auto-repairable issues in the given DCP directory, or under
/// `FixMode::DryRun` report the same repairs without writing a byte.
pub fn fix_dcp(dcp_dir: &Path, mode: FixMode, signed_documents: SignedDocuments) -> FixResult {
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
    let mut edits = DocumentEdits::default();

    // PKL hash and size mismatches are repaired in the batch pass below. a note
    // that reaches neither list would vanish from the report.
    let mut pending_pkl_mismatches: Vec<Note> = Vec::new();

    for note in &verify_result.notes {
        let (rewrite, element): (DocumentRewrite, &str) = match note.code {
            Code::PklHashMismatch | Code::PklSizeMismatch => {
                pending_pkl_mismatches.push(note.clone());
                continue;
            }
            Code::SmpteNamespaceWrong | Code::InteropNamespaceWrong => (fix_namespace, "Namespace"),
            Code::CplInvalidContentKind => (|xml, _| fix_content_kind(xml), "ContentKind"),
            _ => {
                // Not auto-fixable
                result.skipped.push(note.clone());
                continue;
            }
        };
        let rewritten = note.file.as_ref().and_then(|file| {
            let text = edits.text(file).ok()?;
            Some((file, rewrite(&text, dcp.standard)?))
        });
        let Some((file, text)) = rewritten else {
            result.skipped.push(note.clone());
            continue;
        };
        edits.set(file, text);
        result.repairs.push(Repair {
            code: note.code,
            description: format!(
                "{element} in {}",
                file.file_name().unwrap_or_default().to_string_lossy()
            ),
            file: file.clone(),
        });
    }

    let id_to_path: std::collections::HashMap<&str, &str> = dcp
        .assetmap
        .assets
        .iter()
        .map(|a| (a.id.as_str(), a.path.as_str()))
        .collect();

    // the CPL goes first: rewriting its hashes changes the CPL's own PKL entry
    for (cpl_path, cpl) in &dcp.cpls {
        repair_cpl_hashes(cpl_path, cpl, dcp_dir, &id_to_path, &mut edits, &mut result);
    }

    // Batch fix: recompute PKL hashes

    let mut repaired_assets: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    // an earlier repair can put a file back to the bytes its PKL entry records
    let mut already_matching_assets: std::collections::HashSet<PathBuf> =
        std::collections::HashSet::new();
    let mut failure_reasons: std::collections::HashMap<PathBuf, String> =
        std::collections::HashMap::new();

    for (pkl_path, pkl) in &dcp.pkls {
        let mut pkl_modified = false;
        let mut xml = match edits.text(pkl_path) {
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
            let (computed_hash, computed_size) = match edits.hash_and_size(&full_path) {
                Ok(measured) => measured,
                Err(e) => {
                    failure_reasons
                        .insert(full_path.clone(), format!("could not hash the asset: {e}"));
                    continue;
                }
            };
            let mut asset_rewritten = false;
            let mut hash_matches = false;

            if pkl_asset.hash.is_empty() {
                failure_reasons.insert(full_path.clone(), "the PKL records no hash".into());
            } else if computed_hash == pkl_asset.hash {
                hash_matches = true;
            } else {
                match replace_asset_element(
                    &xml,
                    &pkl_asset.id,
                    "Hash",
                    &pkl_asset.hash,
                    &computed_hash,
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

            // a repaired hash without its size leaves the package failing on
            // pkl_size_mismatch instead
            let recorded_size = pkl_asset.size.to_string();
            let size_matches = computed_size.to_string() == recorded_size;
            if !size_matches {
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

            if hash_matches && size_matches {
                already_matching_assets.insert(full_path.clone());
            }
            if asset_rewritten {
                pkl_modified = true;
                repaired_assets.insert(full_path);
            }
        }

        if pkl_modified {
            edits.set(pkl_path, xml);
        }
    }

    let signed_paths: Vec<PathBuf> = edits
        .edited
        .iter()
        .filter(|(_, text)| carries_signature(text))
        .map(|(path, _)| path.clone())
        .collect();
    for path in &signed_paths {
        result
            .signature_notes
            .push(signature_note(path, signed_documents));
    }
    let refused = !signed_paths.is_empty() && signed_documents == SignedDocuments::Refuse;
    if refused {
        result.repairs.clear();
        edits = DocumentEdits::default();
        repaired_assets.clear();
        already_matching_assets.clear();
    }

    if mode.writes() {
        let pkl_paths: Vec<&PathBuf> = dcp.pkls.iter().map(|(path, _)| path).collect();
        write_edits(&mut edits, &pkl_paths, &mut result);
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

    // compared on the text the repairs leave, which a dry run never writes
    result
        .skipped
        .retain(|note| note.code != Code::CplPklHashMismatch);
    result
        .skipped
        .extend(cpl_pkl_hash_mismatches(&dcp, &edits, &id_to_path));

    for note in pending_pkl_mismatches {
        if refused {
            result.skipped.push(note);
            continue;
        }
        let repaired = note
            .file
            .as_ref()
            .is_some_and(|f| repaired_assets.contains(f) || already_matching_assets.contains(f));
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

// every CPL is written before any PKL, since the PKL records the CPL's hash
fn write_edits(edits: &mut DocumentEdits, pkl_paths: &[&PathBuf], result: &mut FixResult) {
    let (pkls, others): (Vec<PathBuf>, Vec<PathBuf>) = edits
        .edited
        .keys()
        .cloned()
        .partition(|path| pkl_paths.contains(&path));
    let mut failed_document: Option<String> = None;
    for path in others.iter().chain(&pkls) {
        let outcome = match &failed_document {
            Some(document) => Err(format!("{document} could not be written")),
            None => std::fs::write(path, &edits.edited[path]).map_err(|e| e.to_string()),
        };
        let Err(reason) = outcome else {
            continue;
        };
        if failed_document.is_none() && others.contains(path) {
            failed_document = Some(path.display().to_string());
        }
        edits.edited.remove(path);
        let (unwritten, kept): (Vec<Repair>, Vec<Repair>) =
            result.repairs.drain(..).partition(|r| &r.file == path);
        result.repairs = kept;
        for repair in unwritten {
            result.skipped.push(
                Note::error(
                    repair.code,
                    format!("{} not repaired: {reason}", repair.description),
                )
                .with_file(path),
            );
        }
    }
}

fn cpl_pkl_hash_mismatches(
    dcp: &dcp::Dcp,
    edits: &DocumentEdits,
    id_to_path: &std::collections::HashMap<&str, &str>,
) -> Vec<Note> {
    let pkls: Vec<crate::pkl::Pkl> = dcp
        .pkls
        .iter()
        .filter_map(|(path, _)| dcpdoctor_parse::parse_pkl(&edits.text(path).ok()?))
        .collect();
    let pkl_hashes: std::collections::HashMap<&str, &str> = pkls
        .iter()
        .flat_map(|pkl| &pkl.assets)
        .filter(|asset| !asset.hash.is_empty())
        .map(|asset| (asset.id.as_str(), asset.hash.as_str()))
        .collect();
    dcp.cpls
        .iter()
        .filter_map(|(path, _)| {
            let cpl = dcpdoctor_parse::parse_cpl(&edits.text(path).ok()?)?;
            Some(crate::validate::check_cpl_asset_hashes(
                path,
                &cpl,
                &pkl_hashes,
                id_to_path,
            ))
        })
        .flatten()
        .filter(|note| note.code == Code::CplPklHashMismatch)
        .collect()
}

fn carries_signature(xml: &str) -> bool {
    xml.match_indices('<')
        .any(|(open, _)| local_name_at(xml, open) == Some(("Signature", false)))
}

fn signature_note(path: &Path, signed_documents: SignedDocuments) -> Note {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    match signed_documents {
        SignedDocuments::Refuse => Note::error(
            Code::SignatureInvalid,
            format!(
                "{name} carries an XML signature the repairs would invalidate, so fix leaves the package as it is. Pass --break-signatures to write the repairs, then re-sign the package"
            ),
        ),
        SignedDocuments::Break => Note::warning(
            Code::SignatureInvalid,
            format!(
                "{name} is rewritten by the repairs, so its signature no longer verifies and the package needs re-signing"
            ),
        ),
    }
    .with_file(path)
}

fn repair_cpl_hashes(
    cpl_path: &Path,
    cpl: &crate::cpl::Cpl,
    dcp_dir: &Path,
    id_to_path: &std::collections::HashMap<&str, &str>,
    edits: &mut DocumentEdits,
    result: &mut FixResult,
) {
    let hashed_assets = cpl.reels.iter().flat_map(|reel| {
        [&reel.picture, &reel.sound, &reel.subtitle]
            .into_iter()
            .chain(&reel.closed_captions)
            .filter(|asset| !asset.hash.is_empty())
    });
    for asset in hashed_assets {
        let Some(&asset_rel) = id_to_path.get(asset.id.as_str()) else {
            continue;
        };
        let Ok((computed, _)) = edits.hash_and_size(&dcp_dir.join(asset_rel)) else {
            continue;
        };
        if computed == asset.hash {
            continue;
        }
        let text = match edits.text(cpl_path) {
            Ok(text) => text,
            Err(e) => {
                result.skipped.push(cpl_hash_not_repaired(
                    cpl_path,
                    asset_rel,
                    &format!("the CPL could not be read: {e}"),
                ));
                return;
            }
        };
        let rewritten = element_with_id_range(&text, &asset.id).and_then(|element| {
            replace_element_text(&text, element, "Hash", &asset.hash, &computed)
        });
        match rewritten {
            Some(rewritten) => {
                edits.set(cpl_path, rewritten);
                result.repairs.push(Repair {
                    code: Code::CplPklHashMismatch,
                    description: format!("Hash for {asset_rel} in CPL"),
                    file: cpl_path.to_path_buf(),
                });
            }
            None => result.skipped.push(cpl_hash_not_repaired(
                cpl_path,
                asset_rel,
                "the hash the CPL records was not found in its text",
            )),
        }
    }
}

fn cpl_hash_not_repaired(cpl_path: &Path, asset: &str, reason: &str) -> Note {
    Note::error(
        Code::CplPklHashMismatch,
        format!("CPL hash for {asset} differs from the file, not repaired: {reason}"),
    )
    .with_file(cpl_path)
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
    replace_element_text(xml, asset, local, recorded, computed)
}

fn replace_element_text(
    xml: &str,
    region: std::ops::Range<usize>,
    local: &str,
    recorded: &str,
    computed: &str,
) -> Option<String> {
    let element_text = element_text_range(xml, region, local)?;
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

// a CPL reel asset opens with its <Id>, so the tag before it is the asset's own
fn element_with_id_range(xml: &str, asset_id: &str) -> Option<std::ops::Range<usize>> {
    for (open, _) in xml.match_indices('<') {
        if local_name_at(xml, open) != Some(("Id", false)) {
            continue;
        }
        let Some(id_text) = element_text_range(xml, open..xml.len(), "Id") else {
            continue;
        };
        if dcpdoctor_parse::strip_urn_uuid(xml[id_text].trim()) != asset_id {
            continue;
        }
        let parent_open = xml[..open].rfind('<')?;
        let (parent, closing) = local_name_at(xml, parent_open)?;
        if closing {
            return None;
        }
        let close = find_close_tag(xml, open..xml.len(), parent)?;
        return Some(parent_open..close);
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
fn fix_namespace(xml: &str, standard: Standard) -> Option<String> {
    let (wrong, correct) = match standard {
        Standard::Smpte => (
            "http://www.digicine.com/PROTO-ASDCP-CPL-20040511#",
            "http://www.smpte-ra.org/schemas/429-7/2006/CPL",
        ),
        Standard::Interop => (
            "http://www.smpte-ra.org/schemas/429-7/2006/CPL",
            "http://www.digicine.com/PROTO-ASDCP-CPL-20040511#",
        ),
        Standard::Unknown => return None,
    };
    xml.contains(wrong).then(|| xml.replace(wrong, correct))
}

/// Normalize ContentKind to lowercase canonical form.
fn fix_content_kind(xml: &str) -> Option<String> {
    // Find <ContentKind>...</ContentKind> and normalize the value
    let start_tag = "<ContentKind>";
    let end_tag = "</ContentKind>";
    let start = xml.find(start_tag)? + start_tag.len();
    let end = start + xml[start..].find(end_tag)?;

    let original = &xml[start..end];
    let normalized = normalize_content_kind(original.trim());
    if normalized == original.trim() {
        return None;
    }
    Some(format!("{}{}{}", &xml[..start], normalized, &xml[end..]))
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

    fn fixture_package() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/valid_smpte")
    }

    /// Copy the committed SMPTE package into a temp dir with `edits` applied to
    /// `file`, so each case is one deliberate deviation from a real package.
    fn mutated_package(file: &str, edits: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for entry in fs::read_dir(fixture_package()).unwrap().flatten() {
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
                "<ContentKind>feature</ContentKind>",
                r#"<ContentKind scope="http://www.smpte-ra.org/schemas/429-7/2006/CPL#standard-content">nonsense</ContentKind>"#,
            )],
        );

        let result = fix_dcp(dir.path(), FixMode::Apply, SignedDocuments::Refuse);

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

    const STALE_HASH_PREFIX: &str = "AAAA";

    // the package's PKL names no files, so the asset map ties each file to its id
    fn pkl_asset_of(file_name: &str) -> crate::pkl::PklAsset {
        use crate::assetmap::ParseXmlFile;
        let id = crate::assetmap::AssetMap::parse(&fixture_package().join("ASSETMAP.xml"))
            .expect("the package's asset map parses")
            .assets
            .into_iter()
            .find(|a| a.path == file_name)
            .expect("the package's asset map lists the file")
            .id;
        crate::pkl::Pkl::parse(&fixture_package().join("pkl.xml"))
            .expect("the package's PKL parses")
            .assets
            .into_iter()
            .find(|a| a.id == id)
            .expect("the package's PKL lists the file")
    }

    fn pkl_hash_of(file_name: &str) -> String {
        pkl_asset_of(file_name).hash
    }

    fn picture_id_element() -> String {
        format!("<Id>urn:uuid:{}</Id>", pkl_asset_of("picture.mxf").id)
    }

    // still valid base64 of 20 bytes, just not the file's digest
    fn stale(hash: &str) -> String {
        format!("{STALE_HASH_PREFIX}{}", &hash[STALE_HASH_PREFIX.len()..])
    }

    // the picture asset precedes the sound one in the PKL
    fn planted_in_picture_asset(text: &str) -> String {
        format!(
            "{}\n      <AnnotationText>{text}</AnnotationText>",
            picture_id_element()
        )
    }

    #[test]
    fn a_stale_hash_that_also_appears_earlier_is_rewritten_only_in_its_own_asset() {
        let sound_hash = pkl_hash_of("sound.mxf");
        let stale_sound_hash = stale(&sound_hash);
        let dir = mutated_package(
            "pkl.xml",
            &[
                (
                    &format!("<Hash>{sound_hash}</Hash>"),
                    &format!("<Hash>{stale_sound_hash}</Hash>"),
                ),
                (
                    &picture_id_element(),
                    &planted_in_picture_asset(&stale_sound_hash),
                ),
            ],
        );

        let result = fix_dcp(dir.path(), FixMode::Apply, SignedDocuments::Refuse);

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
                "<AnnotationText>{stale_sound_hash}</AnnotationText>"
            )),
            "the earlier occurrence must survive byte for byte: {xml}"
        );
        assert_eq!(
            xml.matches(&stale_sound_hash).count(),
            1,
            "only the sound asset's Hash may be rewritten: {xml}"
        );
        assert!(
            xml.contains(&format!("<Hash>{sound_hash}</Hash>")),
            "the sound asset must carry its computed hash: {xml}"
        );
        assert!(
            xml.contains(&format!("<Hash>{}</Hash>", pkl_hash_of("picture.mxf"))),
            "the picture asset's own hash must be untouched: {xml}"
        );
    }

    #[test]
    fn a_hash_the_asset_element_does_not_carry_leaves_the_rest_of_the_pkl_alone() {
        // the numeric reference parses to the same hash the file does not spell
        // out, so the recorded hash is nowhere in this asset's text
        let sound_hash = pkl_hash_of("sound.mxf");
        let stale_sound_hash = stale(&sound_hash);
        let unpadded_stale_sound_hash = stale_sound_hash
            .strip_suffix('=')
            .expect("a base64 SHA-1 ends in one pad character");
        let dir = mutated_package(
            "pkl.xml",
            &[
                (
                    &format!("<Hash>{sound_hash}</Hash>"),
                    &format!("<Hash>{unpadded_stale_sound_hash}&#61;</Hash>"),
                ),
                (
                    &picture_id_element(),
                    &planted_in_picture_asset(&stale_sound_hash),
                ),
            ],
        );
        let pkl = dir.path().join("pkl.xml");
        let before = fs::read_to_string(&pkl).unwrap();

        let result = fix_dcp(dir.path(), FixMode::Apply, SignedDocuments::Refuse);

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

    const CHANGED_ESSENCE_BYTE_OFFSET: u64 = 40;

    fn flip_byte(path: &Path, offset: u64) {
        use std::io::{Read, Seek, SeekFrom, Write};
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        let mut byte = [0u8; 1];
        file.seek(SeekFrom::Start(offset)).unwrap();
        file.read_exact(&mut byte).unwrap();
        file.seek(SeekFrom::Start(offset)).unwrap();
        file.write_all(&[byte[0] ^ 0xff]).unwrap();
    }

    #[test]
    fn a_changed_essence_file_has_its_cpl_hash_and_then_its_pkl_entries_repaired() {
        let dir = mutated_package("cpl.xml", &[]);
        let sound = dir.path().join("sound.mxf");
        flip_byte(&sound, CHANGED_ESSENCE_BYTE_OFFSET);
        let new_sound_hash = sha1_base64(&sound).unwrap();

        let result = fix_dcp(dir.path(), FixMode::Apply, SignedDocuments::Refuse);

        assert!(
            result
                .repairs
                .iter()
                .any(|r| r.code == Code::CplPklHashMismatch && r.description.contains("sound.mxf")),
            "the CPL's sound hash must be repaired: {:?}",
            result.repairs
        );
        let cpl = fs::read_to_string(dir.path().join("cpl.xml")).unwrap();
        assert!(
            cpl.contains(&format!("<Hash>{new_sound_hash}</Hash>")),
            "{cpl}"
        );
        let after = crate::validate::verify_dcp(dir.path(), &VerifyOptions::default());
        let hash_codes = [
            Code::CplPklHashMismatch,
            Code::PklHashMismatch,
            Code::PklSizeMismatch,
        ];
        assert!(
            !after.notes.iter().any(|n| hash_codes.contains(&n.code)),
            "the repaired package must agree with its files: {:?}",
            after.notes
        );
    }

    #[test]
    fn a_dry_run_does_not_report_a_cpl_hash_it_would_repair_as_remaining() {
        let sound_hash = pkl_hash_of("sound.mxf");
        let dir = mutated_package(
            "cpl.xml",
            &[(
                &format!("<Hash>{sound_hash}</Hash>"),
                &format!("<Hash>{}</Hash>", stale(&sound_hash)),
            )],
        );

        let result = fix_dcp(dir.path(), FixMode::DryRun, SignedDocuments::Refuse);

        assert!(
            result
                .repairs
                .iter()
                .any(|r| r.code == Code::CplPklHashMismatch && r.description.contains("sound.mxf")),
            "{:?}",
            result.repairs
        );
        assert!(
            !result
                .skipped
                .iter()
                .any(|n| n.code == Code::CplPklHashMismatch),
            "a hash the run would repair cannot also remain: {:?}",
            result.skipped
        );
    }

    fn repair_list(result: &FixResult) -> Vec<(Code, String)> {
        result
            .repairs
            .iter()
            .map(|r| (r.code, r.description.clone()))
            .collect()
    }

    #[test]
    fn a_dry_run_lists_the_repairs_the_real_run_makes() {
        let changed_essence = || {
            let dir = mutated_package("cpl.xml", &[]);
            flip_byte(&dir.path().join("sound.mxf"), CHANGED_ESSENCE_BYTE_OFFSET);
            dir
        };
        let wrong_namespace = || {
            mutated_package(
                "cpl.xml",
                &[(
                    "http://www.smpte-ra.org/schemas/429-7/2006/CPL",
                    "http://www.digicine.com/PROTO-ASDCP-CPL-20040511#",
                )],
            )
        };
        let cases: [(&str, &dyn Fn() -> TempDir); 2] = [
            ("changed essence", &changed_essence),
            ("wrong namespace", &wrong_namespace),
        ];
        for (case, package) in cases {
            let previewed = package();
            let applied = package();

            let preview = fix_dcp(previewed.path(), FixMode::DryRun, SignedDocuments::Refuse);
            let applied = fix_dcp(applied.path(), FixMode::Apply, SignedDocuments::Refuse);

            assert_eq!(repair_list(&preview), repair_list(&applied), "{case}");
            assert!(!applied.repairs.is_empty(), "{case}");
        }
    }

    const SIGNED_PACKAGE_DIR: &str = "../../../tests/fixtures/signature";
    const SIGNED_CPL_FILE: &str =
        "CPL_SMPTE_TST-1-Bv21_S_EN-EN-CCAP_US_51-HI-VI_2K_ISDCF_20170110_DTB_SMPTE_OV.xml";
    const SIGNED_PKL_FILE: &str =
        "PKL_SMPTE_TST-1-Bv21_S_EN-EN-CCAP_US_51-HI-VI_2K_ISDCF_20170110_DTB_SMPTE_OV.xml";
    const SIGNED_CPL_ID: &str = "f5f10f5e-80db-42cf-9955-e622c9c99e90";
    const SIGNED_PKL_ID: &str = "1f4db139-fcde-498c-a028-cb0c54792c99";

    // the signed ISDCF CPL and PKL, with a ContentKind whose repair changes the CPL
    fn signed_package_with_a_repair() -> TempDir {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(SIGNED_PACKAGE_DIR);
        let dir = TempDir::new().unwrap();
        fs::copy(
            source.join(SIGNED_PKL_FILE),
            dir.path().join(SIGNED_PKL_FILE),
        )
        .unwrap();
        let cpl = fs::read_to_string(source.join(SIGNED_CPL_FILE)).unwrap();
        fs::write(
            dir.path().join(SIGNED_CPL_FILE),
            cpl.replace(
                "<ContentKind>test</ContentKind>",
                "<ContentKind>Feature Film</ContentKind>",
            ),
        )
        .unwrap();
        fs::write(
            dir.path().join("ASSETMAP.xml"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:0e5f4d4a-6c55-4b9b-9d0e-3f2f1b6c7a10</Id>
  <AssetList>
    <Asset><Id>urn:uuid:{SIGNED_PKL_ID}</Id><PackingList>true</PackingList><ChunkList><Chunk><Path>{SIGNED_PKL_FILE}</Path></Chunk></ChunkList></Asset>
    <Asset><Id>urn:uuid:{SIGNED_CPL_ID}</Id><ChunkList><Chunk><Path>{SIGNED_CPL_FILE}</Path></Chunk></ChunkList></Asset>
  </AssetList>
</AssetMap>"#
            ),
        )
        .unwrap();
        dir
    }

    fn signature_note_files(result: &FixResult, severity: Severity) -> Vec<String> {
        let mut files: Vec<String> = result
            .signature_notes
            .iter()
            .filter(|n| n.code == Code::SignatureInvalid && n.severity == severity)
            .filter_map(|n| n.file.as_ref()?.file_name()?.to_str().map(str::to_string))
            .collect();
        files.sort();
        files
    }

    #[test]
    fn fix_refuses_to_rewrite_a_signed_package_without_leave_to_break_it() {
        let dir = signed_package_with_a_repair();
        let before: Vec<String> = [SIGNED_CPL_FILE, SIGNED_PKL_FILE]
            .iter()
            .map(|file| fs::read_to_string(dir.path().join(file)).unwrap())
            .collect();

        let result = fix_dcp(dir.path(), FixMode::Apply, SignedDocuments::Refuse);

        assert_eq!(
            signature_note_files(&result, Severity::Error),
            [SIGNED_CPL_FILE, SIGNED_PKL_FILE],
            "{:?}",
            result.signature_notes
        );
        assert!(result.repairs.is_empty(), "{:?}", result.repairs);
        let after: Vec<String> = [SIGNED_CPL_FILE, SIGNED_PKL_FILE]
            .iter()
            .map(|file| fs::read_to_string(dir.path().join(file)).unwrap())
            .collect();
        assert_eq!(after, before, "a refused fix must write nothing");
    }

    #[test]
    fn fix_with_leave_to_break_signatures_writes_and_names_each_broken_signature() {
        let dir = signed_package_with_a_repair();

        let result = fix_dcp(dir.path(), FixMode::Apply, SignedDocuments::Break);

        assert_eq!(
            signature_note_files(&result, Severity::Warning),
            [SIGNED_CPL_FILE, SIGNED_PKL_FILE],
            "{:?}",
            result.signature_notes
        );
        assert!(
            result
                .repairs
                .iter()
                .any(|r| r.code == Code::CplInvalidContentKind),
            "{:?}",
            result.repairs
        );
        let cpl = fs::read_to_string(dir.path().join(SIGNED_CPL_FILE)).unwrap();
        assert!(cpl.contains("<ContentKind>feature</ContentKind>"), "{cpl}");
    }

    #[test]
    fn a_cpl_its_own_repair_puts_back_is_not_reported_against_the_pkl() {
        let dir = mutated_package(
            "cpl.xml",
            &[(
                "http://www.smpte-ra.org/schemas/429-7/2006/CPL",
                "http://www.digicine.com/PROTO-ASDCP-CPL-20040511#",
            )],
        );

        let result = fix_dcp(dir.path(), FixMode::Apply, SignedDocuments::Refuse);

        assert!(
            result
                .repairs
                .iter()
                .any(|r| r.code == Code::SmpteNamespaceWrong),
            "the namespace must be repaired: {:?}",
            result.repairs
        );
        assert!(
            !result
                .skipped
                .iter()
                .any(|n| matches!(n.code, Code::PklHashMismatch | Code::PklSizeMismatch)),
            "the repaired CPL matches its PKL entry again: {:?}",
            result.skipped
        );
    }

    #[test]
    fn a_hash_mismatch_the_repair_cannot_write_is_reported_as_skipped() {
        let picture_hash = pkl_hash_of("picture.mxf");
        let dir = mutated_package(
            "pkl.xml",
            &[(
                &format!("<Hash>{picture_hash}</Hash>"),
                &format!("<Hash>{}</Hash>", stale(&picture_hash)),
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

        let result = fix_dcp(dir.path(), FixMode::Apply, SignedDocuments::Refuse);

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
        let result = fix_content_kind(
            r#"<?xml version="1.0"?><CompositionPlaylist><ContentKind>TRAILER</ContentKind></CompositionPlaylist>"#,
        )
        .unwrap();
        assert!(result.contains("<ContentKind>trailer</ContentKind>"));
    }

    #[test]
    fn test_fix_content_kind_already_correct() {
        assert!(
            fix_content_kind(
                r#"<?xml version="1.0"?><CompositionPlaylist><ContentKind>feature</ContentKind></CompositionPlaylist>"#,
            )
            .is_none()
        );
    }

    #[test]
    fn test_fix_namespace_smpte() {
        let result = fix_namespace(
            r#"<CompositionPlaylist xmlns="http://www.digicine.com/PROTO-ASDCP-CPL-20040511#"><Id>urn:uuid:test</Id></CompositionPlaylist>"#,
            Standard::Smpte,
        )
        .unwrap();
        assert!(result.contains("http://www.smpte-ra.org/schemas/429-7/2006/CPL"));
    }
}
