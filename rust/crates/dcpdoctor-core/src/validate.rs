use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::dcp;
use crate::hash::sha1_base64;
use crate::{Code, Note, Severity, VerifyOptions, VerifyResult};

/// Picture sizes a DCP may carry, as (width, height). The coded sizes are the
/// DCI/ST 428-1 flat and scope images (what libdcp's verify accepts); the full
/// containers are the 2K and 4K frames themselves, which a picture asset is
/// allowed to fill.
const STANDARD_PICTURE_SIZES: &[(u32, u32)] = &[
    (1998, 1080), // 2K flat, coded
    (2048, 858),  // 2K scope, coded
    (2048, 1080), // 2K full container
    (3996, 2160), // 4K flat, coded
    (4096, 1716), // 4K scope, coded
    (4096, 2160), // 4K full container
];

fn is_standard_picture_size(width: u32, height: u32) -> bool {
    STANDARD_PICTURE_SIZES.contains(&(width, height))
}

/// Extension of the essence files ST 429-3 / ST 429-2 wrap picture and sound in.
/// libdcp requires a CPL `Hash` on exactly these (its "encryptable" assets), so a
/// loose Interop subtitle XML with no hash is not a finding.
const ESSENCE_EXTENSION: &str = "mxf";

/// A reel's essence references, each with the label findings name it by.
fn reel_file_assets(
    reel: &crate::cpl::Reel,
) -> [(&'static str, &crate::cpl::ReelAsset); REEL_ASSET_KINDS] {
    [
        ("picture", &reel.picture),
        ("sound", &reel.sound),
        ("subtitle", &reel.subtitle),
    ]
}

/// Picture, sound and subtitle: the reel asset classes `Reel` carries.
const REEL_ASSET_KINDS: usize = 3;

/// The CPL carries its own `Hash` per reel asset alongside the PKL's, and some
/// servers hash-check against the CPL rather than the PKL, so the two disagreeing
/// is a rejection even when the PKL hash matches the bytes on disk. Covers
/// libdcp's MISSING_HASH plus MISMATCHED_PICTURE_HASHES / MISMATCHED_SOUND_HASHES
/// (one code here, since the message names the asset class).
fn check_cpl_asset_hashes(
    cpl_path: &Path,
    cpl: &crate::cpl::Cpl,
    pkl_hashes: &HashMap<&str, &str>,
    id_to_path: &HashMap<&str, &str>,
) -> Vec<Note> {
    let mut notes = Vec::new();

    for (index, reel) in cpl.reels.iter().enumerate() {
        for (kind, asset) in reel_file_assets(reel) {
            if asset.id.is_empty() {
                continue;
            }
            let is_essence = id_to_path.get(asset.id.as_str()).is_some_and(|path| {
                Path::new(path)
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case(ESSENCE_EXTENSION))
            });

            if asset.hash.is_empty() {
                if is_essence {
                    notes.push(
                        Note::warning(
                            Code::CplMissingHash,
                            format!(
                                "Reel {} {kind} asset {} has no <Hash> in the CPL",
                                index + 1,
                                asset.id
                            ),
                        )
                        .with_file(cpl_path),
                    );
                }
                continue;
            }

            if let Some(&pkl_hash) = pkl_hashes.get(asset.id.as_str())
                && pkl_hash != asset.hash
            {
                notes.push(
                    Note::error(
                        Code::CplPklHashMismatch,
                        format!(
                            "Reel {} {kind} asset {} hash differs between CPL ({}) and PKL ({pkl_hash})",
                            index + 1,
                            asset.id,
                            asset.hash
                        ),
                    )
                    .with_file(cpl_path),
                );
            }
        }
    }

    notes
}

/// Bv2.1 §8.1 requires a SMPTE CPL's `AnnotationText` to be present and equal to
/// its `ContentTitleText`, so the human-readable label a projectionist sees on
/// the server matches the title the package is filed under. Interop CPLs carry
/// no such rule. libdcp MISSING_CPL_ANNOTATION_TEXT / MISMATCHED_CPL_ANNOTATION_TEXT,
/// which it grades as a Bv2.1 error and a warning respectively.
fn check_cpl_annotation_text(
    cpl_path: &Path,
    cpl: &crate::cpl::Cpl,
    standard: crate::Standard,
) -> Vec<Note> {
    if standard != crate::Standard::Smpte {
        return Vec::new();
    }
    if cpl.annotation.trim().is_empty() {
        return vec![
            Note::warning(
                Code::MissingRequiredElement,
                "SMPTE CPL has no <AnnotationText>",
            )
            .with_file(cpl_path),
        ];
    }
    if cpl.annotation != cpl.content_title {
        return vec![
            Note::warning(
                Code::CplAnnotationTextMismatch,
                format!(
                    "CPL <AnnotationText> '{}' differs from its <ContentTitleText> '{}'",
                    cpl.annotation, cpl.content_title
                ),
            )
            .with_file(cpl_path),
        ];
    }
    Vec::new()
}

/// A PKL that packages exactly one CPL must repeat that CPL's `ContentTitleText`
/// as its own `AnnotationText` (libdcp MISMATCHED_PKL_ANNOTATION_TEXT_WITH_CPL).
/// With more than one CPL in the PKL there is no single title to require, so the
/// rule does not apply.
fn check_pkl_annotation_text(
    pkl_path: &Path,
    pkl: &crate::pkl::Pkl,
    cpls: &[(std::path::PathBuf, crate::cpl::Cpl)],
    standard: crate::Standard,
) -> Vec<Note> {
    if standard != crate::Standard::Smpte {
        return Vec::new();
    }
    let mut packaged = pkl
        .assets
        .iter()
        .filter_map(|asset| cpls.iter().find(|(_, cpl)| cpl.id == asset.id));
    let (Some((_, cpl)), None) = (packaged.next(), packaged.next()) else {
        return Vec::new();
    };
    if pkl.annotation == cpl.content_title {
        return Vec::new();
    }
    vec![
        Note::warning(
            Code::PklAnnotationTextMismatch,
            format!(
                "PKL <AnnotationText> '{}' differs from its only CPL's <ContentTitleText> '{}'",
                pkl.annotation, cpl.content_title
            ),
        )
        .with_file(pkl_path),
    ]
}

/// libdcp DUPLICATE_ASSET_ID_IN_PKL, the PKL counterpart of the asset map rule:
/// one asset id must not be listed twice.
fn check_pkl_duplicate_asset_ids(pkl_path: &Path, pkl: &crate::pkl::Pkl) -> Vec<Note> {
    let mut seen = HashSet::new();
    pkl.assets
        .iter()
        .filter(|asset| !seen.insert(asset.id.as_str()))
        .map(|asset| {
            Note::error(
                Code::DuplicateAssetId,
                format!("PKL lists asset ID more than once: {}", asset.id),
            )
            .with_file(pkl_path)
        })
        .collect()
}

/// Bv2.1 §7.2.1 caps a timed-text track file at 115 MB, and the fonts inside it
/// at 10 MB. libdcp reads the font limit as the total across every font in the
/// asset rather than each one alone, and notes the wording is ambiguous.
const MAX_TIMED_TEXT_ASSET_BYTES: u64 = 115 * 1024 * 1024;
const MAX_TIMED_TEXT_FONT_BYTES: usize = 10 * 1024 * 1024;

/// No standard sets this, but a font past it is very likely to cause playback
/// problems, which is the threshold DCP-o-matic warns authors at.
const MAX_SINGLE_FONT_BYTES: usize = 640 * 1024;

/// Bv2.1 §7.2.3 caps a closed-caption XML document at 256 kB. Subtitles have no
/// equivalent cap, so this applies to caption tracks only.
const MAX_CLOSED_CAPTION_XML_BYTES: usize = 256 * 1024;

/// Which timed-text track class an asset is, since several limits differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimedTextKind {
    Subtitle,
    ClosedCaption,
}

impl TimedTextKind {
    fn label(self) -> &'static str {
        match self {
            TimedTextKind::Subtitle => "subtitle",
            TimedTextKind::ClosedCaption => "closed caption",
        }
    }
}

/// Shared inputs for the per-asset timed-text checks.
struct TimedTextContext<'a> {
    dcp_dir: &'a Path,
    id_to_path: &'a HashMap<&'a str, &'a str>,
    schema_dir: Option<&'a Path>,
    keys: &'a crate::kdm::ContentKeys,
    standard: crate::Standard,
}

impl TimedTextContext<'_> {
    /// Every rule that applies to one timed-text asset. Reads the essence once:
    /// a loose XML asset from disk, a wrapped one out of its MXF.
    fn check_asset(&self, asset: &crate::cpl::ReelAsset, kind: TimedTextKind) -> Vec<Note> {
        let mut notes = Vec::new();
        if asset.id.is_empty() {
            return notes;
        }
        let Some(&asset_path) = self.id_to_path.get(asset.id.as_str()) else {
            return notes;
        };
        let path = self.dcp_dir.join(asset_path);
        if !path.exists() {
            return notes;
        }

        // Bv2.1 §8.3.2 wants an EntryPoint of 0 on a timed-text asset, and text
        // that is no integer is neither 0 nor absent
        if asset.entry_point_unparseable {
            notes.push(
                Note::error(
                    Code::XmlParseError,
                    format!(
                        "the {} reel's <EntryPoint> is not an integer, so the entry-point check did not run",
                        kind.label()
                    ),
                )
                .with_file(&path),
            );
        }

        let is_xml = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("xml"));

        // Bv2.1 caps the track file itself, whichever form it takes
        if let Ok(size) = std::fs::metadata(&path).map(|m| m.len())
            && size > MAX_TIMED_TEXT_ASSET_BYTES
        {
            notes.push(
                Note::error(
                    Code::TimedTextSizeExceeded,
                    format!(
                        "{} asset is {size} bytes, over the Bv2.1 maximum {MAX_TIMED_TEXT_ASSET_BYTES}",
                        kind.label()
                    ),
                )
                .with_file(&path),
            );
        }

        let (xml, glyph_notes) = if is_xml {
            let xml = match std::fs::read_to_string(&path) {
                Ok(xml) => Some(xml),
                Err(e) => {
                    notes.push(
                        Note::error(
                            Code::SubtitleParseError,
                            format!("failed to read: {e}, so no subtitle rule ran on it"),
                        )
                        .with_file(&path),
                    );
                    None
                }
            };
            (
                xml,
                crate::subtitle::check_glyph_coverage(&path, |decl| self.resolve_font(&path, decl)),
            )
        } else {
            match crate::subtitle::read_wrapped_timed_text(
                &path,
                self.keys,
                crate::subtitle::FontData::Include,
            ) {
                Some(wrapped) => {
                    notes.extend(wrapped.notes.iter().cloned());
                    if wrapped.is_unreadable() {
                        (None, Vec::new())
                    } else {
                        notes.extend(self.wrapped_asset_notes(&wrapped, asset, kind, &path));
                        let glyphs = crate::subtitle::check_glyph_coverage_wrapped(&wrapped, &path);
                        (Some(wrapped.xml), glyphs)
                    }
                }
                None => {
                    notes.push(
                        Note::error(
                            Code::MxfUnreadable,
                            "timed-text MXF could not be read, so none of its subtitle checks ran",
                        )
                        .with_file(&path),
                    );
                    (None, Vec::new())
                }
            }
        };

        if let Some(xml) = xml {
            if kind == TimedTextKind::ClosedCaption && xml.len() > MAX_CLOSED_CAPTION_XML_BYTES {
                notes.push(
                    Note::error(
                        Code::TimedTextSizeExceeded,
                        format!(
                            "closed caption XML is {} bytes, over the Bv2.1 maximum {MAX_CLOSED_CAPTION_XML_BYTES}",
                            xml.len()
                        ),
                    )
                    .with_file(&path),
                );
            }
            if let Some(schema_dir) = self.schema_dir {
                notes.extend(crate::schema::check_schema_xml(&xml, &path, schema_dir));
            }
            notes.extend(crate::subtitle::validate_subtitle_xml(
                &xml,
                &path,
                self.standard,
            ));
        }
        notes.extend(glyph_notes);
        notes
    }

    /// Rules that need the MXF wrapper rather than the document: the font size
    /// cap and the ST 429-5 identity and duration relationships.
    fn wrapped_asset_notes(
        &self,
        wrapped: &crate::subtitle::WrappedTimedText,
        asset: &crate::cpl::ReelAsset,
        kind: TimedTextKind,
        path: &Path,
    ) -> Vec<Note> {
        let mut notes = Vec::new();

        if wrapped.font_bytes > MAX_TIMED_TEXT_FONT_BYTES {
            notes.push(
                Note::error(
                    Code::TimedTextSizeExceeded,
                    format!(
                        "embedded fonts total {} bytes, over the Bv2.1 maximum {MAX_TIMED_TEXT_FONT_BYTES}",
                        wrapped.font_bytes
                    ),
                )
                .with_file(path),
            );
        }

        // the aggregate cap above is the conformance limit; a single font this
        // big is under it and still stalls playback on real players, so it is a
        // warning rather than an error
        if let Some(largest) = wrapped.fonts.values().map(Vec::len).max()
            && largest > MAX_SINGLE_FONT_BYTES
        {
            notes.push(
                Note::warning(
                    Code::SubtitleFontTooLarge,
                    format!(
                        "an embedded font is {largest} bytes, over the {MAX_SINGLE_FONT_BYTES} that plays back reliably"
                    ),
                )
                .with_file(path),
            );
        }

        // ST 429-5: the descriptor's ResourceID names the document inside, and
        // the AssetUUID names the track file. Reusing one as the other leaves two
        // different things sharing an id, which is what libdcp rejects.
        let document_id = crate::subtitle::document_id(&wrapped.xml);
        if let Some(document_id) = document_id
            && wrapped.resource_id != document_id
        {
            notes.push(
                Note::error(
                    Code::TimedTextIdMismatch,
                    format!(
                        "MXF ResourceID {} does not match the document's Id {}",
                        format_uuid(&wrapped.resource_id),
                        format_uuid(&document_id)
                    ),
                )
                .with_file(path),
            );
        }
        if wrapped.asset_id == wrapped.resource_id
            || document_id.is_some_and(|id| wrapped.asset_id == id)
        {
            notes.push(
                Note::error(
                    Code::TimedTextIdMismatch,
                    format!(
                        "MXF AssetID {} is reused as the ResourceID or the document Id; ST 429-5 wants three distinct ids",
                        format_uuid(&wrapped.asset_id)
                    ),
                )
                .with_file(path),
            );
        }

        // ST 429-2 §9.4: the reel's Duration is what the essence actually carries
        if asset.duration_unparseable {
            notes.push(
                Note::error(
                    Code::XmlParseError,
                    format!(
                        "the {} reel's <Duration> or <IntrinsicDuration> is not an integer, so it was not compared with the essence",
                        kind.label()
                    ),
                )
                .with_file(path),
            );
        }
        if asset.duration > 0 && i64::from(wrapped.container_duration) != asset.duration {
            notes.push(
                Note::error(
                    Code::CplMismatchedDurations,
                    format!(
                        "{} essence carries {} frames but the reel declares Duration {}",
                        kind.label(),
                        wrapped.container_duration,
                        asset.duration
                    ),
                )
                .with_file(path),
            );
        }

        notes
    }

    /// Resolve a LoadFont declaration to a font file in the package. Interop
    /// names a file by URI; SMPTE ST 428-7 names an asset by urn.
    fn resolve_font(
        &self,
        subtitle_path: &Path,
        decl: &crate::subtitle::FontDecl,
    ) -> Option<std::path::PathBuf> {
        if let Some(uri) = &decl.uri {
            let beside = subtitle_path.parent().unwrap_or(self.dcp_dir).join(uri);
            if beside.exists() {
                return Some(beside);
            }
            let at_root = self.dcp_dir.join(uri);
            return at_root.exists().then_some(at_root);
        }
        // ASSETMAP ids are stored with urn:uuid: stripped
        let urn = decl.urn.as_ref()?;
        let &path = self
            .id_to_path
            .get(crate::assetmap::strip_urn_uuid(urn).as_str())?;
        let path = self.dcp_dir.join(path);
        path.exists().then_some(path)
    }
}

/// Render raw uuid bytes the way findings and CPLs write them.
fn format_uuid(bytes: &[u8; 16]) -> String {
    uuid::Uuid::from_bytes(*bytes).to_string()
}

/// Verify a DCP at the given path.
pub fn verify_dcp(dcp_dir: &Path, opts: &VerifyOptions) -> VerifyResult {
    if crate::imf::is_imf_package(dcp_dir) {
        return verify_imp(dcp_dir, opts);
    }

    let mut result = VerifyResult::default();

    let dcp = match dcp::open_dcp(dcp_dir) {
        Ok(d) => d,
        Err(notes) => {
            for note in notes {
                result.add(note);
            }
            return result;
        }
    };

    result.standard = dcp.standard;

    // KDM: when supplied, run the window/CPL-match checks and unwrap the content
    // keys so the encrypted-essence checks below can decrypt. A wrong recipient
    // key or malformed KDM fails loud here as a clear error rather than garbage.
    let content_keys = build_content_keys(dcp_dir, opts, &mut result);

    // 0. XSD schema validation, when a schema dir is available (schema-path
    // driven, see schema::locate_schema_dir). Validates every CPL/PKL/ASSETMAP
    // against the SMPTE/Interop XSDs, emitting xml_schema_violation. Subtitle
    // documents get the same treatment further down, where the reels resolve
    // them. When the pass cannot run at all it says so rather than passing
    // silently with no XSD coverage.
    let schema_dir = crate::schema::locate_schema_dir();
    if let Some(reason) = crate::schema::schema_pass_unavailable(schema_dir.as_deref()) {
        result.add(Note::warning(Code::SchemaValidationSkipped, reason).with_file(dcp_dir));
    }
    if let Some(schema_dir) = schema_dir.as_deref() {
        for note in crate::schema::check_schema(&dcp.assetmap_path, schema_dir) {
            result.add(note);
        }
        for (pkl_path, _) in &dcp.pkls {
            for note in crate::schema::check_schema(pkl_path, schema_dir) {
                result.add(note);
            }
        }
        for (cpl_path, _) in &dcp.cpls {
            for note in crate::schema::check_schema(cpl_path, schema_dir) {
                result.add(note);
            }
        }
    }

    // 1. Check for duplicate asset IDs
    let mut seen_ids = HashSet::new();
    for asset in &dcp.assetmap.assets {
        if !seen_ids.insert(&asset.id) {
            result.add(Note {
                severity: Severity::Error,
                code: Code::DuplicateAssetId,
                message: format!("Duplicate asset ID: {}", asset.id),
                file: Some(dcp.assetmap_path.clone()),
                line: 0,
            });
        }
    }

    // 2. Verify all referenced files exist
    for asset in &dcp.assetmap.assets {
        let full_path = dcp_dir.join(&asset.path);
        if !full_path.exists() {
            result.add(Note {
                severity: Severity::Error,
                code: Code::AssetNotFound,
                message: format!("Asset file not found: {}", asset.path),
                file: Some(dcp.assetmap_path.clone()),
                line: 0,
            });
        }
    }

    // 2b. ASSETMAP's own file name and its declared chunk lengths (ClairMeta
    // check_am_name / check_assets_am_size).
    for note in crate::validators::check_assetmap_name(&dcp.assetmap_path, dcp.standard) {
        result.add(note);
    }
    for note in
        crate::validators::check_assetmap_chunk_size(dcp_dir, &dcp.assetmap_path, &dcp.assetmap)
    {
        result.add(note);
    }

    // 3. Validate PKLs
    if dcp.pkls.is_empty() {
        result.add(Note {
            severity: Severity::Error,
            code: Code::MissingPkl,
            message: "No valid PKL found in DCP".to_string(),
            file: Some(dcp_dir.to_path_buf()),
            line: 0,
        });
    }

    // Build ID→path map
    let id_to_path: HashMap<&str, &str> = dcp
        .assetmap
        .assets
        .iter()
        .map(|a| (a.id.as_str(), a.path.as_str()))
        .collect();

    for (pkl_path, pkl) in &dcp.pkls {
        if opts.check_signatures {
            for note in crate::signature::verify_signature(pkl_path, opts.strict_smpte) {
                result.add(note);
            }
        }
        for note in check_pkl_duplicate_asset_ids(pkl_path, pkl) {
            result.add(note);
        }
        for note in check_pkl_annotation_text(pkl_path, pkl, &dcp.cpls, dcp.standard) {
            result.add(note);
        }
        if pkl.assets_without_id > 0 {
            result.add(
                Note::error(
                    Code::MissingRequiredElement,
                    format!(
                        "PKL lists {} asset(s) with no readable <Id>, so nothing was checked against them",
                        pkl.assets_without_id
                    ),
                )
                .with_file(pkl_path),
            );
        }
        // Verify PKL asset sizes (cheap, so not gated on check_hashes)
        for pkl_asset in &pkl.assets {
            if pkl_asset.size_unparseable {
                result.add(
                    Note::error(
                        Code::XmlParseError,
                        format!(
                            "PKL <Size> for asset {} is not an integer, so the size check did not run",
                            pkl_asset.id
                        ),
                    )
                    .with_file(pkl_path),
                );
                continue;
            }
            if let Some(&asset_path) = id_to_path.get(pkl_asset.id.as_str()) {
                let full_path = dcp_dir.join(asset_path);
                if pkl_asset.size > 0
                    && let Ok(meta) = std::fs::metadata(&full_path)
                    && meta.len() != pkl_asset.size as u64
                {
                    result.add(Note {
                        severity: Severity::Error,
                        code: Code::PklSizeMismatch,
                        message: format!(
                            "Size mismatch for {} (PKL says {}, file is {})",
                            asset_path,
                            pkl_asset.size,
                            meta.len()
                        ),
                        file: Some(full_path),
                        line: 0,
                    });
                }
            }
        }
        // Verify PKL asset hashes
        if opts.check_hashes {
            for pkl_asset in &pkl.assets {
                if let Some(&asset_path) = id_to_path.get(pkl_asset.id.as_str()) {
                    let full_path = dcp_dir.join(asset_path);
                    if full_path.exists() && !pkl_asset.hash.is_empty() {
                        match sha1_base64(&full_path) {
                            Ok(computed) if computed != pkl_asset.hash => {
                                result.add(Note {
                                    severity: Severity::Error,
                                    code: Code::PklHashMismatch,
                                    message: format!(
                                        "Hash mismatch for {} (expected {}, got {})",
                                        asset_path, pkl_asset.hash, computed
                                    ),
                                    file: Some(full_path),
                                    line: 0,
                                });
                            }
                            Err(e) => {
                                tracing::warn!("Failed to hash {}: {}", asset_path, e);
                            }
                            _ => {}
                        }
                    }
                } else {
                    result.add(Note {
                        severity: Severity::Warning,
                        code: Code::PklMissingAssetReference,
                        message: format!("PKL references unknown asset: {}", pkl_asset.id),
                        file: Some(pkl_path.clone()),
                        line: 0,
                    });
                }
            }
        }
    }

    // 4. Validate CPLs
    if dcp.cpls.is_empty() {
        result.add(Note {
            severity: Severity::Error,
            code: Code::MissingCpl,
            message: "No valid CPL found in DCP".to_string(),
            file: Some(dcp_dir.to_path_buf()),
            line: 0,
        });
    }

    // asset id -> the hash the PKLs record for it, for the CPL/PKL comparison
    let pkl_hashes: HashMap<&str, &str> = dcp
        .pkls
        .iter()
        .flat_map(|(_, pkl)| &pkl.assets)
        .filter(|a| !a.hash.is_empty())
        .map(|a| (a.id.as_str(), a.hash.as_str()))
        .collect();

    for (cpl_path, cpl) in &dcp.cpls {
        if opts.check_signatures {
            for note in crate::signature::verify_signature(cpl_path, opts.strict_smpte) {
                result.add(note);
            }
        }
        for note in check_cpl_asset_hashes(cpl_path, cpl, &pkl_hashes, &id_to_path) {
            result.add(note);
        }
        for note in check_cpl_annotation_text(cpl_path, cpl, dcp.standard) {
            result.add(note);
        }
        if cpl.reels.is_empty() {
            result.add(Note {
                severity: Severity::Error,
                code: Code::CplMissingReel,
                message: "CPL has no reels".to_string(),
                file: Some(cpl_path.clone()),
                line: 0,
            });
        }

        // ContentKind validation (strict mode)
        if opts.strict_smpte && !cpl.content_kind.is_empty() {
            const VALID_CONTENT_KINDS: &[&str] = &[
                "feature",
                "trailer",
                "test",
                "teaser",
                "rating",
                "advertisement",
                "short",
                "transitional",
                "psa",
                "policy",
                "episode",
            ];
            if !VALID_CONTENT_KINDS.contains(&cpl.content_kind.to_lowercase().as_str()) {
                result.add(Note {
                    severity: Severity::Error,
                    code: Code::CplInvalidContentKind,
                    message: format!(
                        "Invalid ContentKind '{}' (expected one of: {})",
                        cpl.content_kind,
                        VALID_CONTENT_KINDS.join(", ")
                    ),
                    file: Some(cpl_path.clone()),
                    line: 0,
                });
            }
        }

        for reel in &cpl.reels {
            for (track, asset) in [("picture", &reel.picture), ("sound", &reel.sound)] {
                if asset.duration_unparseable {
                    result.add(
                        Note::error(
                            Code::XmlParseError,
                            format!(
                                "Reel {} {track} <Duration> or <IntrinsicDuration> is not an integer, so the duration checks did not run on it",
                                reel.id
                            ),
                        )
                        .with_file(cpl_path),
                    );
                }
            }
            if reel.picture.duration <= 0 {
                result.add(Note {
                    severity: Severity::Error,
                    code: Code::CplInvalidDuration,
                    message: "Reel has invalid picture duration".to_string(),
                    file: Some(cpl_path.clone()),
                    line: 0,
                });
            }
            // ST 429-2 §9.4: all Duration elements in a reel (except timed text)
            // shall be equal, so a sound/picture mismatch is a hard violation.
            if reel.sound.duration > 0 && reel.sound.duration != reel.picture.duration {
                result.add(Note {
                    severity: Severity::Error,
                    code: Code::CplMismatchedDurations,
                    message: format!(
                        "Sound duration differs from picture duration in reel {}",
                        reel.id
                    ),
                    file: Some(cpl_path.clone()),
                    line: 0,
                });
            }

            // ST 429-2:2020 §8.1: "The composition shall have an Edit Rate of
            // 24/1, 25/1, 30/1, 48/1, 50/1 or 60/1." Read off the picture, which
            // §9.6.1 makes identical across every picture asset.
            if !reel.picture.edit_rate.is_empty()
                && !edit_rate_is_permitted(&reel.picture.edit_rate)
            {
                result.add(Note {
                    severity: Severity::Error,
                    code: Code::CplInvalidEditRate,
                    message: format!(
                        "Non-DCI edit rate '{}' (allowed: 24, 25, 30, 48, 50, 60 fps)",
                        reel.picture.edit_rate
                    ),
                    file: Some(cpl_path.clone()),
                    line: 0,
                });
            }
        }
    }

    // 4b. Every timed-text asset a CPL reel references: the XSD pass, the
    // structural rules, glyph coverage, the Bv2.1 size caps and the ST 429-5
    // identity rules, all from one read of each asset.
    let timed_text = TimedTextContext {
        dcp_dir,
        id_to_path: &id_to_path,
        schema_dir: schema_dir.as_deref(),
        keys: &content_keys,
        standard: dcp.standard,
    };
    for (_cpl_path, cpl) in &dcp.cpls {
        for reel in &cpl.reels {
            let tracks = std::iter::once((TimedTextKind::Subtitle, &reel.subtitle)).chain(
                reel.closed_captions
                    .iter()
                    .map(|caption| (TimedTextKind::ClosedCaption, caption)),
            );
            for (kind, asset) in tracks {
                for note in timed_text.check_asset(asset, kind) {
                    result.add(note);
                }
            }
        }
    }

    // 4c. Structural CPL checks (markers, MCA labeling, cross-references,
    // supplemental/OPL, reel continuity, stereo). Lightweight XML checks that
    // run in the core path; --studio adds deeper ffprobe-based analysis.
    let cpl_paths: Vec<std::path::PathBuf> = dcp.cpls.iter().map(|(p, _)| p.clone()).collect();
    let known_asset_ids: Vec<String> = dcp.assetmap.assets.iter().map(|a| a.id.clone()).collect();

    // stripped-id -> on-disk essence path, for validators that probe the MXF
    let id_to_file: HashMap<String, std::path::PathBuf> = dcp
        .assetmap
        .assets
        .iter()
        .map(|a| {
            let id =
                a.id.strip_prefix("urn:uuid:")
                    .unwrap_or(&a.id)
                    .to_lowercase();
            (id, dcp_dir.join(&a.path))
        })
        .collect();

    // OV-aware cross-ref for supplemental DCPs: resolve refs across this package
    // and the OV DCP when --ov is given.
    let ov_asset_ids: Option<HashSet<String>> = opts.ov.as_deref().map(dcp_asset_ids);

    for note in crate::validators::check_encryption(dcp_dir, &cpl_paths, opts.kdm.is_some()) {
        result.add(note);
    }
    // Encrypted packages must have a signed CPL and PKL (ClairMeta check_dcp_signed).
    if opts.check_signatures {
        let pkl_paths: Vec<std::path::PathBuf> = dcp.pkls.iter().map(|(p, _)| p.clone()).collect();
        for note in crate::validators::check_dcp_signed(&cpl_paths, &pkl_paths) {
            result.add(note);
        }
    }
    for note in crate::validators::check_cross_references(
        &known_asset_ids,
        ov_asset_ids.as_ref(),
        &cpl_paths,
    ) {
        result.add(note);
    }
    for note in crate::validators::check_supplemental(&cpl_paths) {
        result.add(note);
    }
    for note in crate::compliance::check_uuids(dcp_dir) {
        result.add(note);
    }
    // Package hygiene: unreferenced/zero-byte files in the package directory.
    let referenced_paths: Vec<String> =
        dcp.assetmap.assets.iter().map(|a| a.path.clone()).collect();
    for note in crate::validators::check_package_files(dcp_dir, &referenced_paths) {
        result.add(note);
    }
    // Non-ASCII characters in folder/file names (portability warning).
    for note in crate::validators::check_non_ascii_names(dcp_dir) {
        result.add(note);
    }
    for (cpl_path, cpl) in &dcp.cpls {
        // an empty ContentTitleText is check_isdcf_naming's first finding, so it
        // runs on one too
        for note in crate::isdcf::check_isdcf_naming(&cpl.content_title, cpl_path) {
            result.add(note);
        }
        for note in crate::validators::check_cpl_metadata(cpl_path, dcp.standard) {
            result.add(note);
        }
    }
    for cpl_path in &cpl_paths {
        for note in crate::validators::check_markers(cpl_path, opts.strict_smpte) {
            result.add(note);
        }
        for note in crate::validators::check_audio_channels(cpl_path, &id_to_file) {
            result.add(note);
        }
        let sound_channels =
            crate::validators::first_sound_channel_count_of_cpl(cpl_path, &id_to_file);
        for note in crate::validators::check_main_sound_configuration(
            cpl_path,
            dcp.standard,
            sound_channels,
        ) {
            result.add(note);
        }
        for note in crate::validators::check_first_subtitle_timing(
            cpl_path,
            dcp.standard,
            &id_to_file,
            &content_keys,
        ) {
            result.add(note);
        }
        for note in crate::validators::check_timed_text_content(
            cpl_path,
            dcp.standard,
            &id_to_file,
            &content_keys,
        ) {
            result.add(note);
        }
        for note in crate::validators::check_timed_text_reels(
            cpl_path,
            dcp.standard,
            &id_to_file,
            &content_keys,
        ) {
            result.add(note);
        }
        for note in crate::validators::check_playback_compatibility(cpl_path, &id_to_file) {
            result.add(note);
        }
        for note in crate::validators::check_partial_encryption(cpl_path) {
            result.add(note);
        }
        for note in crate::validators::check_reel_continuity(cpl_path) {
            result.add(note);
        }
        for note in crate::validators::check_reel_coherence(cpl_path) {
            result.add(note);
        }
        for note in crate::validators::check_reel_edit_rates(cpl_path) {
            result.add(note);
        }
        for note in crate::validators::check_composition_metadata_asset(cpl_path) {
            result.add(note);
        }
        for note in crate::validators::check_cpl_metadata_asset(cpl_path, dcp.standard) {
            result.add(note);
        }
        for note in crate::validators::check_main_picture_active_area(cpl_path, &id_to_file) {
            result.add(note);
        }
        for note in crate::validators::check_cpl_languages(cpl_path, dcp.standard) {
            result.add(note);
        }
        for note in crate::validators::check_reel_duration(cpl_path) {
            result.add(note);
        }
        for note in crate::validators::check_sound_channel_configuration(
            cpl_path,
            dcp.standard,
            &id_to_file,
        ) {
            result.add(note);
        }
        for note in
            crate::validators::check_subtitle_frame_rate(cpl_path, &id_to_file, &content_keys)
        {
            result.add(note);
        }
        for note in crate::validators::check_stereo(cpl_path, &id_to_file) {
            result.add(note);
        }
        for note in crate::validators::check_asset_id_matches_essence(cpl_path, &id_to_file) {
            result.add(note);
        }
        for note in crate::validators::check_aux_data(cpl_path, &id_to_file) {
            result.add(note);
        }
        for note in crate::hfr_stereo::check_hfr_compliance(cpl_path) {
            result.add(note);
        }
    }

    // 5. MXF validation (if picture details requested)
    if opts.check_picture_details {
        // track files ffprobe described no stream of, and the first reason it
        // gave. Everything under `mxf_info.picture` is gated on a descriptor that
        // probe was the only source of.
        let mut unprobed_files: Vec<String> = Vec::new();
        let mut probe_error = String::new();
        for asset in &dcp.assetmap.assets {
            let full_path = dcp_dir.join(&asset.path);
            let ext = full_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if ext != "mxf" || !full_path.exists() {
                continue;
            }

            // MXF partition structure (SMPTE 377-1): header/footer/closed-complete.
            let partitions = crate::mxf_advanced::validate_mxf_partitions(&full_path);
            for note in crate::mxf_advanced::check_mxf_partitions(&partitions, &full_path) {
                result.add(note);
            }

            let mxf_info = crate::mxf::read_mxf_info(&full_path);
            if !mxf_info.valid {
                result.add(Note {
                    severity: Severity::Error,
                    code: Code::MxfUnreadable,
                    message: format!("Invalid MXF file: {}", mxf_info.error),
                    file: Some(full_path),
                    line: 0,
                });
                continue;
            }

            if mxf_info.picture.is_none()
                && mxf_info.sound.is_none()
                && !mxf_info.stream_probe_error.is_empty()
                && !crate::mxf::known_non_picture_essence(&full_path)
            {
                unprobed_files.push(asset.path.clone());
                if probe_error.is_empty() {
                    probe_error = mxf_info.stream_probe_error.clone();
                }
            }

            // Codestream checks on picture essence: 0xFFFF legacy constraint
            // (SMPTE Cat. 862) and ISO 15444-1 cinema profile constraints.
            if mxf_info.picture.is_some() {
                let (codestream_notes, _forensics) = crate::j2k::check_picture_j2k_mxf(
                    &full_path,
                    &content_keys,
                    crate::j2k::PictureEssenceFamily::Cinema,
                    opts.scan_every_frame,
                );
                for note in codestream_notes {
                    result.add(note);
                }

                let bitrate = crate::bitrate::analyze_picture_bitrate(&full_path, &content_keys);
                for note in crate::bitrate::check_bitrate_compliance(&bitrate, &full_path) {
                    result.add(note);
                }
                if let Some(note) = crate::bitrate::skipped_measurement_note(&bitrate, &full_path) {
                    result.add(note);
                }

                for note in crate::mxf::check_picture_frame_rate_mxf(&full_path) {
                    result.add(note);
                }
            }

            // Encrypted sound essence: ffprobe can't read it, so read the
            // descriptor + verify frame integrity via asdcplib with the KDM.
            // Cleartext sound is handled by the ffprobe path below.
            for note in crate::mxf::check_sound_essence_mxf(&full_path, &content_keys) {
                result.add(note);
            }

            // Picture validation
            if let Some(ref pic) = mxf_info.picture
                && pic.width > 0
                && pic.height > 0
                && opts.strict_smpte
                && !is_standard_picture_size(pic.width, pic.height)
            {
                result.add(Note {
                    severity: Severity::Warning,
                    code: Code::PictureInvalidResolution,
                    message: format!(
                        "Non-standard picture resolution: {}x{}",
                        pic.width, pic.height
                    ),
                    file: Some(full_path.clone()),
                    line: 0,
                });
            }

            // J2K bitrate validation
            if let Some(ref pic) = mxf_info.picture
                && pic.frame_count > 0
                && pic.frame_rate_num > 0
                && pic.frame_rate_den > 0
                && mxf_info.file_size_bytes > 0
            {
                let duration_secs =
                    pic.frame_count as f64 * pic.frame_rate_den as f64 / pic.frame_rate_num as f64;
                let bitrate_mbps =
                    (mxf_info.file_size_bytes as f64 * 8.0) / (duration_secs * 1_000_000.0);

                let max_bitrate = postkit::j2k::DCI_MAX_BITRATE_MBPS;

                if bitrate_mbps > max_bitrate {
                    result.add(Note {
                        severity: Severity::Error,
                        code: Code::J2kBitrateExceeded,
                        message: format!(
                            "J2K bitrate {:.1} Mbps exceeds maximum {:.0} Mbps",
                            bitrate_mbps, max_bitrate
                        ),
                        file: Some(full_path.clone()),
                        line: 0,
                    });
                }
            }

            // Sound validation
            if let Some(ref snd) = mxf_info.sound {
                if snd.sample_rate > 0 && snd.sample_rate != 48000 && snd.sample_rate != 96000 {
                    result.add(Note {
                        severity: Severity::Warning,
                        code: Code::SoundInvalidSampleRate,
                        message: format!("Non-standard audio sample rate: {} Hz", snd.sample_rate),
                        file: Some(full_path.clone()),
                        line: 0,
                    });
                }
                // 24-bit PCM / block-align (SMPTE ST 429-2)
                for note in crate::mxf::check_sound_descriptor(snd, &full_path) {
                    result.add(note);
                }
            }
        }

        if !unprobed_files.is_empty() {
            result.add(
                Note::warning(
                    Code::CheckSkipped,
                    format!(
                        "the picture essence checks (resolution, frame rate, bitrate, codestream, and the note for encrypted picture without a KDM) did not run on {}: {probe_error}",
                        unprobed_files.join(", ")
                    ),
                )
                .with_file(dcp_dir),
            );
        }
    }

    result
}

/// True when an `EditRate` is one of the six ST 429-2 §8.1 composition rates.
///
/// Compares the value, not the text: a CPL may write 48 2 for 24 fps, and
/// rejecting that would be a false positive on a conformant package.
fn edit_rate_is_permitted(edit_rate: &str) -> bool {
    const PERMITTED_RATES: &[u32] = &[24, 25, 30, 48, 50, 60];

    let mut parts = edit_rate.split_whitespace();
    let (Some(numerator), Some(denominator), None) = (parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    let (Ok(numerator), Ok(denominator)) = (numerator.parse::<u32>(), denominator.parse::<u32>())
    else {
        return false;
    };
    denominator != 0
        && numerator % denominator == 0
        && PERMITTED_RATES.contains(&(numerator / denominator))
}

/// Build the content keys from `opts.kdm` + `opts.recipient_key`, adding any
/// KDM window / CPL-match / unwrap-failure notes to `result`. Returns
/// `ContentKeys::none()` when no KDM is supplied (or unwrap fails), so encrypted
/// essence keeps skipping instead of producing garbage findings. Serves a DCP and
/// an IMP alike: both are a directory of XML the KDM's CPL id is looked for in.
fn build_content_keys(
    package_dir: &Path,
    opts: &VerifyOptions,
    result: &mut VerifyResult,
) -> crate::kdm::ContentKeys {
    let Some(kdm_path) = opts.kdm.as_deref() else {
        return crate::kdm::ContentKeys::none();
    };

    // window (expired / not-yet-valid) + CPL-match checks (extends the validator)
    for note in crate::kdm::validate_kdm(kdm_path, Some(package_dir)) {
        result.add(note);
    }

    let Some(key_path) = opts.recipient_key.as_deref() else {
        result.add(
            Note::warning(
                Code::KdmRequired,
                "--kdm supplied without --recipient-key; cannot decrypt essence",
            )
            .with_file(kdm_path),
        );
        return crate::kdm::ContentKeys::none();
    };

    match crate::kdm::ContentKeys::from_kdm(kdm_path, key_path) {
        Ok(keys) => keys,
        Err(e) => {
            result.add(
                Note::error(Code::KdmRequired, format!("failed to unwrap KDM: {e}"))
                    .with_file(kdm_path),
            );
            crate::kdm::ContentKeys::none()
        }
    }
}

/// Collect a DCP's ASSETMAP asset ids (urn:uuid: prefix stripped) for OV
/// cross-referencing. Returns empty if the OV can't be opened.
fn dcp_asset_ids(dir: &Path) -> HashSet<String> {
    match dcp::open_dcp(dir) {
        Ok(dcp) => dcp
            .assetmap
            .assets
            .iter()
            .map(|a| a.id.strip_prefix("urn:uuid:").unwrap_or(&a.id).to_string())
            .collect(),
        Err(_) => HashSet::new(),
    }
}

fn verify_imp(imp_dir: &Path, opts: &VerifyOptions) -> VerifyResult {
    let mut result = VerifyResult {
        standard: crate::Standard::Smpte,
        ..Default::default()
    };

    // same KDM handling as the DCP path: the window and CPL-match checks, then
    // the content keys the encrypted-essence passes read AS-02 track files with.
    let content_keys = build_content_keys(imp_dir, opts, &mut result);

    // Native IMF validation works everywhere including WASM.
    for note in crate::imf::validate_imp(
        imp_dir,
        opts.ov.as_deref(),
        opts.check_picture_details,
        opts.scan_every_frame,
        &content_keys,
    ) {
        result.add(note);
    }

    // Photon adds deep IMF conformance checks.
    match crate::photon::run_photon(imp_dir) {
        Ok(photon_notes) => {
            for note in photon_notes {
                result.add(note);
            }
        }
        Err(error) => result.add(crate::photon::unavailable_note(&error)),
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(relative_path: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/dcps/synthetic/valid")
            .join(relative_path)
    }

    // ─── CPL asset hashes ─────────────────────────────────────────────────

    /// A committed SMPTE package whose PKL records the real SHA-1 of its own
    /// files, so a hash case starts from genuine digests instead of invented ones.
    const SMPTE_PACKAGE: &str = "../../../tests/fixtures/valid_smpte";

    /// Anchors the hash edits attach to; both are unique in that package's CPL.
    const AFTER_PICTURE_FIELDS: &str = "<ScreenAspectRatio>1998 1080</ScreenAspectRatio>";
    const SOUND_CLOSE_TAG: &str = "</MainSound>";

    fn smpte_package_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(SMPTE_PACKAGE)
    }

    /// Files inside the committed package that the cases below mutate.
    const CPL_FILE: &str = "cpl.xml";
    const PKL_FILE: &str = "pkl.xml";

    /// Copy the committed package into a temp dir with `edits` applied to `file`
    /// in order, so each case is one deliberate deviation from a real package.
    fn mutated_package(file: &str, edits: &[(&str, String)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for entry in std::fs::read_dir(smpte_package_dir()).unwrap().flatten() {
            std::fs::copy(entry.path(), dir.path().join(entry.file_name())).unwrap();
        }
        let target = dir.path().join(file);
        let mut xml = std::fs::read_to_string(&target).unwrap();
        for (from, to) in edits {
            assert!(
                xml.contains(from),
                "the package's {file} no longer has {from:?}"
            );
            xml = xml.replace(from, to);
        }
        std::fs::write(&target, xml).unwrap();
        dir
    }

    /// The hash the committed package's PKL records for one of its files.
    fn pkl_hash_of(original_filename: &str) -> String {
        use crate::assetmap::ParseXmlFile;
        crate::pkl::Pkl::parse(&smpte_package_dir().join("pkl.xml"))
            .expect("the package's PKL parses")
            .assets
            .into_iter()
            .find(|a| a.original_filename == original_filename)
            .expect("the package's PKL lists the file")
            .hash
    }

    /// Edits putting the PKL's own hashes into the CPL's picture and sound assets.
    fn agreeing_hashes() -> Vec<(&'static str, String)> {
        vec![
            (
                AFTER_PICTURE_FIELDS,
                format!(
                    "{AFTER_PICTURE_FIELDS}<Hash>{}</Hash>",
                    pkl_hash_of("picture.mxf")
                ),
            ),
            (
                SOUND_CLOSE_TAG,
                format!("<Hash>{}</Hash>{SOUND_CLOSE_TAG}", pkl_hash_of("sound.mxf")),
            ),
        ]
    }

    #[test]
    fn cpl_hashes_agreeing_with_the_pkl_are_silent() {
        let dir = mutated_package(CPL_FILE, &agreeing_hashes());
        let result = verify_dcp(dir.path(), &VerifyOptions::default());
        assert!(
            !result
                .notes
                .iter()
                .any(|n| n.code == Code::CplMissingHash || n.code == Code::CplPklHashMismatch),
            "CPL hashes equal to the PKL's must draw nothing, got: {:?}",
            result.notes
        );
    }

    #[test]
    fn cpl_essence_asset_without_a_hash_fires() {
        // the package's CPL carries no <Hash> at all, which is the finding
        let dir = mutated_package(CPL_FILE, &[]);
        let result = verify_dcp(dir.path(), &VerifyOptions::default());
        for kind in ["picture", "sound"] {
            assert!(
                result
                    .notes
                    .iter()
                    .any(|n| n.code == Code::CplMissingHash && n.message.contains(kind)),
                "an MXF-backed {kind} asset with no <Hash> must fire, got: {:?}",
                result.notes
            );
        }
    }

    #[test]
    fn cpl_hash_disagreeing_with_the_pkl_fires() {
        // the sound asset's real hash, put on the picture asset: a valid base64
        // SHA-1 that is simply the wrong one, which is what a botched re-wrap makes
        let mut edits = agreeing_hashes();
        edits[0] = (
            AFTER_PICTURE_FIELDS,
            format!(
                "{AFTER_PICTURE_FIELDS}<Hash>{}</Hash>",
                pkl_hash_of("sound.mxf")
            ),
        );
        let dir = mutated_package(CPL_FILE, &edits);
        let result = verify_dcp(dir.path(), &VerifyOptions::default());
        assert!(
            result
                .notes
                .iter()
                .any(|n| n.code == Code::CplPklHashMismatch && n.message.contains("picture")),
            "a CPL picture hash unequal to the PKL's must fire, got: {:?}",
            result.notes
        );
        assert!(
            !result
                .notes
                .iter()
                .any(|n| n.code == Code::CplPklHashMismatch && n.message.contains("sound")),
            "the untouched sound asset must stay silent, got: {:?}",
            result.notes
        );
    }

    #[test]
    fn smpte_dcp_skips_imf_validation() {
        let result = verify_dcp(&fixture("minimal_smpte_2k"), &VerifyOptions::default());

        assert!(!result.notes.iter().any(|note| {
            note.message.contains("IMF Composition Playlist") || note.message.contains("[Photon]")
        }));
    }

    #[test]
    fn interop_dcp_skips_imf_validation() {
        let result = verify_dcp(&fixture("minimal_interop"), &VerifyOptions::default());

        assert!(!result.notes.iter().any(|note| {
            note.message.contains("IMF Composition Playlist") || note.message.contains("[Photon]")
        }));
    }

    // ─── CPL and PKL identity (Bv2.1 §8.1) ────────────────────────────────

    /// The title the committed package's CPL carries, which its AnnotationText
    /// and its PKL's AnnotationText must both repeat.
    const PACKAGE_TITLE: &str = "Test DCP";

    /// Where an AnnotationText is inserted: it precedes ContentTitleText in the
    /// CPL and the Creator element in the PKL.
    const CPL_TITLE_ELEMENT: &str = "<ContentTitleText>Test DCP</ContentTitleText>";
    const PKL_CREATOR_ELEMENT: &str = "<Creator>";

    fn cpl_with_annotation(text: &str) -> Vec<(&'static str, String)> {
        vec![(
            CPL_TITLE_ELEMENT,
            format!("<AnnotationText>{text}</AnnotationText>{CPL_TITLE_ELEMENT}"),
        )]
    }

    fn notes_of(dir: &Path) -> Vec<Note> {
        verify_dcp(dir, &VerifyOptions::default()).notes
    }

    #[test]
    fn cpl_annotation_text_must_be_present_and_equal_the_content_title() {
        // the committed package's CPL has no AnnotationText at all
        let missing = notes_of(mutated_package(CPL_FILE, &[]).path());
        assert!(
            missing
                .iter()
                .any(|n| n.code == Code::MissingRequiredElement
                    && n.message.contains("AnnotationText")),
            "a SMPTE CPL with no AnnotationText must fire, got: {missing:?}"
        );

        let matching =
            notes_of(mutated_package(CPL_FILE, &cpl_with_annotation(PACKAGE_TITLE)).path());
        assert!(
            !matching
                .iter()
                .any(|n| n.code == Code::CplAnnotationTextMismatch
                    || (n.code == Code::MissingRequiredElement
                        && n.message.contains("AnnotationText"))),
            "an AnnotationText equal to the ContentTitleText must stay silent, got: {matching:?}"
        );

        let differing =
            notes_of(mutated_package(CPL_FILE, &cpl_with_annotation("Some Other Title")).path());
        assert!(
            differing
                .iter()
                .any(|n| n.code == Code::CplAnnotationTextMismatch),
            "an AnnotationText differing from the ContentTitleText must fire, got: {differing:?}"
        );
    }

    #[test]
    fn a_pkl_with_one_cpl_must_repeat_its_content_title() {
        // the committed package's PKL has no AnnotationText, and lists one CPL
        let missing = notes_of(mutated_package(PKL_FILE, &[]).path());
        assert!(
            missing
                .iter()
                .any(|n| n.code == Code::PklAnnotationTextMismatch),
            "a PKL whose AnnotationText is not its only CPL's title must fire, got: {missing:?}"
        );

        let matching = notes_of(
            mutated_package(
                PKL_FILE,
                &[(
                    PKL_CREATOR_ELEMENT,
                    format!(
                        "<AnnotationText>{PACKAGE_TITLE}</AnnotationText>{PKL_CREATOR_ELEMENT}"
                    ),
                )],
            )
            .path(),
        );
        assert!(
            !matching
                .iter()
                .any(|n| n.code == Code::PklAnnotationTextMismatch),
            "a PKL repeating its CPL's title must stay silent, got: {matching:?}"
        );
    }

    #[test]
    fn a_pkl_listing_one_asset_twice_fires() {
        let clean = notes_of(mutated_package(PKL_FILE, &[]).path());
        assert!(
            !clean.iter().any(|n| n.code == Code::DuplicateAssetId),
            "the committed package lists each asset once, got: {clean:?}"
        );

        let duplicated_asset = r#"<Asset>
      <Id>urn:uuid:148971a4-abc6-44ae-bf59-34026d0faf17</Id>"#;
        let notes = notes_of(
            mutated_package(
                PKL_FILE,
                &[(
                    duplicated_asset,
                    format!("{duplicated_asset}{}", "</Asset>\n    <Asset>\n      <Id>urn:uuid:148971a4-abc6-44ae-bf59-34026d0faf17</Id>"),
                )],
            )
            .path(),
        );
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::DuplicateAssetId && n.message.contains("PKL")),
            "a PKL listing one id twice must fire, got: {notes:?}"
        );
    }

    /// Write a package whose asset map sits at `am_name` and declares
    /// `length_element` for a 4-byte CPL, so the pipeline sees both ST 429-9
    /// asset map checks.
    fn assetmap_pipeline_package(am_name: &str, length_element: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("cpl.xml"), "abcd").unwrap();
        std::fs::write(
            dir.path().join(am_name),
            format!(
                r#"<?xml version="1.0"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee</Id>
  <AssetList>
    <Asset><Id>urn:uuid:cccccccc-0000-0000-0000-000000000000</Id>
      <ChunkList><Chunk><Path>cpl.xml</Path>{length_element}</Chunk></ChunkList></Asset>
  </AssetList>
</AssetMap>"#
            ),
        )
        .unwrap();
        dir
    }

    #[test]
    fn assetmap_name_and_chunk_length_reach_the_pipeline() {
        let dir = assetmap_pipeline_package("ASSETMAP", "<Length>99</Length>");
        let result = verify_dcp(dir.path(), &VerifyOptions::default());
        assert!(
            result
                .notes
                .iter()
                .any(|n| n.code == Code::AssetmapInvalidName),
            "SMPTE asset map named ASSETMAP must fire from verify_dcp, got: {:?}",
            result.notes
        );
        assert!(
            result
                .notes
                .iter()
                .any(|n| n.code == Code::AssetmapSizeMismatch),
            "Length 99 against a 4-byte file must fire from verify_dcp, got: {:?}",
            result.notes
        );

        let clean = assetmap_pipeline_package("ASSETMAP.xml", "<Length>4</Length>");
        let result = verify_dcp(clean.path(), &VerifyOptions::default());
        assert!(
            !result.notes.iter().any(
                |n| n.code == Code::AssetmapInvalidName || n.code == Code::AssetmapSizeMismatch
            ),
            "correctly named asset map with a matching Length must stay silent, got: {:?}",
            result.notes
        );
    }

    #[test]
    fn an_assetmap_length_that_is_no_integer_fires() {
        let dir = assetmap_pipeline_package("ASSETMAP.xml", "<Length>four</Length>");
        let result = verify_dcp(dir.path(), &VerifyOptions::default());
        assert!(
            result
                .notes
                .iter()
                .any(|n| n.code == Code::XmlParseError && n.message.contains("<Length>")),
            "a Length that is no integer must fire rather than read as absent, got: {:?}",
            result.notes
        );
    }

    // ─── parse-level failure signals ──────────────────────────────────────

    /// Elements the cases below break in the committed package, each unique in
    /// the file it sits in.
    const PKL_PICTURE_ID: &str = "<Id>urn:uuid:148971a4-abc6-44ae-bf59-34026d0faf17</Id>";
    const PKL_PICTURE_SIZE: &str = "<Size>86</Size>";
    const CPL_DURATION: &str = "<Duration>100</Duration>";

    #[test]
    fn a_pkl_asset_with_no_id_is_reported_rather_than_dropped() {
        let dir = mutated_package(PKL_FILE, &[(PKL_PICTURE_ID, String::new())]);
        let notes = notes_of(dir.path());
        assert!(
            notes.iter().any(|n| n.code == Code::MissingRequiredElement
                && n.message.contains("no readable <Id>")),
            "an asset the PKL parse dropped must be reported, got: {notes:?}"
        );
    }

    #[test]
    fn a_pkl_size_that_is_no_integer_fires() {
        let dir = mutated_package(
            PKL_FILE,
            &[(PKL_PICTURE_SIZE, "<Size>eighty-six</Size>".to_string())],
        );
        let notes = notes_of(dir.path());
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::XmlParseError && n.message.contains("<Size>")),
            "a Size that is no integer must fire rather than skip like a 0, got: {notes:?}"
        );
    }

    #[test]
    fn a_reel_duration_that_is_no_integer_fires() {
        let dir = mutated_package(
            CPL_FILE,
            &[(CPL_DURATION, "<Duration>lots</Duration>".to_string())],
        );
        let notes = notes_of(dir.path());
        for track in ["picture", "sound"] {
            assert!(
                notes.iter().any(|n| n.code == Code::XmlParseError
                    && n.message.contains(track)
                    && n.message.contains("<Duration>")),
                "the {track} Duration must be reported as unreadable, got: {notes:?}"
            );
        }
    }

    // an empty title is the one input the first naming rule is about
    #[test]
    fn an_empty_content_title_reaches_the_isdcf_rules() {
        let dir = mutated_package(
            CPL_FILE,
            &[(
                CPL_TITLE_ELEMENT,
                "<ContentTitleText></ContentTitleText>".to_string(),
            )],
        );
        let notes = notes_of(dir.path());
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::IsdcfNamingViolation && n.message.contains("empty")),
            "an empty ContentTitleText must fire, got: {notes:?}"
        );
    }

    /// ST 429-2 §8.1 is a plain "shall", so it must not need --strict.
    #[test]
    fn non_dci_composition_edit_rate_fires_without_strict() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ASSETMAP.xml"),
            r#"<?xml version="1.0"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee</Id>
  <AssetList>
    <Asset><Id>urn:uuid:cccccccc-0000-0000-0000-000000000000</Id>
      <ChunkList><Chunk><Path>cpl.xml</Path></Chunk></ChunkList></Asset>
  </AssetList>
</AssetMap>"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("cpl.xml"),
            r#"<?xml version="1.0"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL">
  <Id>urn:uuid:96558952-39b8-42d3-825e-9ddd31298219</Id>
  <ContentTitleText>t</ContentTitleText>
  <ReelList><Reel><Id>urn:uuid:b353da2a-703e-4d3f-8fcd-659930713ece</Id>
    <AssetList>
      <MainPicture><Id>urn:uuid:f76deec8-ab85-4f05-973d-089b67a55e5f</Id><EditRate>13 1</EditRate><Duration>48</Duration></MainPicture>
    </AssetList>
  </Reel></ReelList>
</CompositionPlaylist>"#,
        )
        .unwrap();

        let opts = VerifyOptions::default();
        assert!(!opts.strict_smpte, "this test is about the default path");
        let result = verify_dcp(dir.path(), &opts);
        assert!(
            result
                .notes
                .iter()
                .any(|n| n.code == Code::CplInvalidEditRate && n.severity == Severity::Error),
            "13 1 is not one of the six permitted composition edit rates, got: {:?}",
            result.notes
        );
    }

    #[test]
    fn permitted_edit_rates_are_compared_by_value_not_by_text() {
        for rate in [
            "24 1", "25 1", "30 1", "48 1", "50 1", "60 1", "48 2", "120 5",
        ] {
            assert!(edit_rate_is_permitted(rate), "{rate} reduces to a DCI rate");
        }
        for rate in ["13 1", "24000 1001", "23 1", "24 0", "24", "", "x 1"] {
            assert!(!edit_rate_is_permitted(rate), "{rate} is not a DCI rate");
        }
    }

    #[test]
    fn marker_edit_rate_mismatch_reaches_the_pipeline() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ASSETMAP.xml"),
            r#"<?xml version="1.0"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee</Id>
  <AssetList>
    <Asset><Id>urn:uuid:cccccccc-0000-0000-0000-000000000000</Id>
      <ChunkList><Chunk><Path>cpl.xml</Path></Chunk></ChunkList></Asset>
  </AssetList>
</AssetMap>"#,
        )
        .unwrap();
        // DCP-o-matic writes MainMarkers ahead of MainPicture, so keep that order
        std::fs::write(
            dir.path().join("cpl.xml"),
            r#"<?xml version="1.0"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL">
  <Id>urn:uuid:96558952-39b8-42d3-825e-9ddd31298219</Id>
  <ContentTitleText>t</ContentTitleText>
  <ReelList><Reel><Id>urn:uuid:b353da2a-703e-4d3f-8fcd-659930713ece</Id>
    <AssetList>
      <MainMarkers><Id>urn:uuid:f76deec8-ab85-4f05-973d-089b67a55e50</Id><EditRate>13 1</EditRate><IntrinsicDuration>48</IntrinsicDuration></MainMarkers>
      <MainPicture><Id>urn:uuid:f76deec8-ab85-4f05-973d-089b67a55e5f</Id><EditRate>24 1</EditRate><Duration>48</Duration></MainPicture>
    </AssetList>
  </Reel></ReelList>
</CompositionPlaylist>"#,
        )
        .unwrap();

        let result = verify_dcp(dir.path(), &VerifyOptions::default());
        assert!(
            result
                .notes
                .iter()
                .any(|n| n.code == Code::ReelEditRateMismatch),
            "a 13 1 marker asset against a 24 1 picture must fire from verify_dcp, got: {:?}",
            result.notes
        );
    }

    #[test]
    fn subtitle_asset_referenced_by_cpl_is_validated() {
        let dir = tempfile::tempdir().unwrap();
        let sub_id = "11111111-2222-3333-4444-555555555555";

        std::fs::write(
            dir.path().join("ASSETMAP.xml"),
            format!(
                r#"<?xml version="1.0"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee</Id>
  <AssetList>
    <Asset><Id>urn:uuid:cccccccc-0000-0000-0000-000000000000</Id>
      <ChunkList><Chunk><Path>cpl.xml</Path></Chunk></ChunkList></Asset>
    <Asset><Id>urn:uuid:{sub_id}</Id>
      <ChunkList><Chunk><Path>sub.xml</Path></Chunk></ChunkList></Asset>
  </AssetList>
</AssetMap>"#
            ),
        )
        .unwrap();

        std::fs::write(
            dir.path().join("cpl.xml"),
            format!(
                r#"<?xml version="1.0"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL">
  <Id>urn:uuid:96558952-39b8-42d3-825e-9ddd31298219</Id>
  <ContentTitleText>t</ContentTitleText>
  <ReelList><Reel><Id>urn:uuid:b353da2a-703e-4d3f-8fcd-659930713ece</Id>
    <AssetList>
      <MainPicture><Id>urn:uuid:f76deec8-ab85-4f05-973d-089b67a55e5f</Id><Duration>48</Duration></MainPicture>
      <MainSubtitle><Id>urn:uuid:{sub_id}</Id><Duration>48</Duration></MainSubtitle>
    </AssetList>
  </Reel></ReelList>
</CompositionPlaylist>"#
            ),
        )
        .unwrap();

        // TimeIn after TimeOut -> must surface as a SubtitleInvalidTiming error
        std::fs::write(
            dir.path().join("sub.xml"),
            r#"<?xml version="1.0"?>
<dcst:SubtitleReel xmlns:dcst="http://www.smpte-ra.org/schemas/428-7/2010/DCST">
  <dcst:Id>urn:uuid:22222222-2222-3333-4444-555555555555</dcst:Id>
  <dcst:ReelNumber>1</dcst:ReelNumber>
  <dcst:Language>en</dcst:Language>
  <dcst:LoadFont ID="f">urn:uuid:aaaa</dcst:LoadFont>
  <dcst:SubtitleList>
    <dcst:Subtitle SpotNumber="1" TimeIn="00:00:07:000" TimeOut="00:00:05:000"/>
  </dcst:SubtitleList>
</dcst:SubtitleReel>"#,
        )
        .unwrap();

        let result = verify_dcp(dir.path(), &VerifyOptions::default());
        assert!(
            result
                .notes
                .iter()
                .any(|n| n.code == Code::SubtitleInvalidTiming),
            "expected subtitle timing note from pipeline, got: {:?}",
            result.notes
        );
    }

    /// Write a package whose single reel references `sub.xml`, and return it.
    fn package_with_subtitle(subtitle_xml: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let sub_id = "11111111-2222-3333-4444-555555555555";
        std::fs::write(
            dir.path().join("ASSETMAP.xml"),
            format!(
                r#"<?xml version="1.0"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee</Id>
  <AssetList>
    <Asset><Id>urn:uuid:cccccccc-0000-0000-0000-000000000000</Id>
      <ChunkList><Chunk><Path>cpl.xml</Path></Chunk></ChunkList></Asset>
    <Asset><Id>urn:uuid:{sub_id}</Id>
      <ChunkList><Chunk><Path>sub.xml</Path></Chunk></ChunkList></Asset>
  </AssetList>
</AssetMap>"#
            ),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("cpl.xml"),
            format!(
                r#"<?xml version="1.0"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL">
  <Id>urn:uuid:cccccccc-0000-0000-0000-000000000000</Id>
  <ContentTitleText>t</ContentTitleText>
  <ReelList><Reel><Id>urn:uuid:b353da2a-703e-4d3f-8fcd-659930713ece</Id>
    <AssetList>
      <MainPicture><Id>urn:uuid:f76deec8-ab85-4f05-973d-089b67a55e5f</Id><Duration>48</Duration></MainPicture>
      <MainSubtitle><Id>urn:uuid:{sub_id}</Id><Duration>48</Duration></MainSubtitle>
    </AssetList>
  </Reel></ReelList>
</CompositionPlaylist>"#
            ),
        )
        .unwrap();
        std::fs::write(dir.path().join("sub.xml"), subtitle_xml).unwrap();
        dir
    }

    /// The XSD the conformant document below routes to.
    const SUBTITLE_SCHEMA: &str = "DCDMSubtitle-2010.xsd";

    /// A conformant ST 428-7:2010 timed-text document.
    const CONFORMANT_SUBTITLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<dcst:SubtitleReel xmlns:dcst="http://www.smpte-ra.org/schemas/428-7/2010/DCST">
  <dcst:Id>urn:uuid:22222222-2222-3333-4444-555555555555</dcst:Id>
  <dcst:ContentTitleText>Test</dcst:ContentTitleText>
  <dcst:IssueDate>2024-01-01T00:00:00.000-00:00</dcst:IssueDate>
  <dcst:ReelNumber>1</dcst:ReelNumber>
  <dcst:Language>en</dcst:Language>
  <dcst:EditRate>24 1</dcst:EditRate>
  <dcst:TimeCodeRate>24</dcst:TimeCodeRate>
  <dcst:StartTime>00:00:00:000</dcst:StartTime>
  <dcst:LoadFont ID="f">urn:uuid:33333333-2222-3333-4444-555555555555</dcst:LoadFont>
  <dcst:SubtitleList>
    <dcst:Font ID="f">
      <dcst:Subtitle SpotNumber="1" TimeIn="00:00:05:000" TimeOut="00:00:07:000">
        <dcst:Text>Hi</dcst:Text>
      </dcst:Subtitle>
    </dcst:Font>
  </dcst:SubtitleList>
</dcst:SubtitleReel>"#;

    // subtitle documents used to reach no XSD at all, so a schema-invalid one
    // passed here and failed at the cinema.
    #[test]
    fn subtitle_asset_is_schema_validated_by_the_pipeline() {
        if !crate::schema::xmllint_available() {
            return;
        }
        // this minimal package's own ASSETMAP and CPL are not schema-conformant,
        // so only findings against the subtitle file itself count here
        let subtitle_violations = |dir: &Path| {
            let subtitle = dir.join("sub.xml");
            verify_dcp(dir, &VerifyOptions::default())
                .notes
                .into_iter()
                .filter(|n| {
                    n.code == Code::XmlSchemaViolation && n.file.as_deref() == Some(&*subtitle)
                })
                .collect::<Vec<_>>()
        };

        let clean = package_with_subtitle(CONFORMANT_SUBTITLE);
        assert!(
            subtitle_violations(clean.path()).is_empty(),
            "a conformant subtitle must not draw a schema violation"
        );

        let broken = package_with_subtitle(&CONFORMANT_SUBTITLE.replace(
            "urn:uuid:22222222-2222-3333-4444-555555555555",
            "not-a-uuid",
        ));
        let notes = subtitle_violations(broken.path());
        assert!(
            notes.iter().any(|n| n.message.contains(SUBTITLE_SCHEMA)),
            "a schema-invalid subtitle must fire against its own XSD, got: {notes:?}"
        );
    }

    // the XSD pass silently skipping itself turned a machine without xmllint into
    // a clean run; it now says so, and must stay quiet when it really did run.
    #[test]
    fn a_complete_schema_environment_draws_no_skip_note() {
        if !crate::schema::xmllint_available() {
            return;
        }
        let dir = package_with_subtitle(CONFORMANT_SUBTITLE);
        let result = verify_dcp(dir.path(), &VerifyOptions::default());
        assert!(
            !result
                .notes
                .iter()
                .any(|n| n.code == Code::SchemaValidationSkipped),
            "xmllint and the schemas are both present here, got: {:?}",
            result.notes
        );
    }

    // A SMPTE package's subtitles are MXF-wrapped, and the structural rules used
    // to run only on loose .xml assets, so none of them fired on a real package.
    #[test]
    fn a_wrapped_subtitle_reaches_the_structural_rules_through_the_pipeline() {
        let dir = tempfile::tempdir().unwrap();
        let sub_id = "11111111-2222-3333-4444-555555555555";
        let font_uuid = [0xCD; 16];

        // this document has an Id and a font but no ReelNumber and no Language
        let document = r#"<?xml version="1.0"?>
<dcst:SubtitleReel xmlns:dcst="http://www.smpte-ra.org/schemas/428-7/2010/DCST">
  <dcst:Id>urn:uuid:22222222-2222-3333-4444-555555555555</dcst:Id>
  <dcst:LoadFont ID="f1">urn:uuid:cdcdcdcd-cdcd-cdcd-cdcd-cdcdcdcdcdcd</dcst:LoadFont>
  <dcst:SubtitleList>
    <dcst:Font ID="f1">
      <dcst:Subtitle SpotNumber="1" TimeIn="00:00:05:000" TimeOut="00:00:07:000">
        <dcst:Text>Hi</dcst:Text>
      </dcst:Subtitle>
    </dcst:Font>
  </dcst:SubtitleList>
</dcst:SubtitleReel>"#;
        let mxf = crate::subtitle::tests::write_mxf(
            dir.path(),
            document,
            Some((&crate::subtitle::tests::make_font(&['H', 'i']), font_uuid)),
        );
        let mxf_name = mxf.file_name().unwrap().to_string_lossy().to_string();

        std::fs::write(
            dir.path().join("ASSETMAP.xml"),
            format!(
                r#"<?xml version="1.0"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee</Id>
  <AssetList>
    <Asset><Id>urn:uuid:cccccccc-0000-0000-0000-000000000000</Id>
      <ChunkList><Chunk><Path>cpl.xml</Path></Chunk></ChunkList></Asset>
    <Asset><Id>urn:uuid:{sub_id}</Id>
      <ChunkList><Chunk><Path>{mxf_name}</Path></Chunk></ChunkList></Asset>
  </AssetList>
</AssetMap>"#
            ),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("cpl.xml"),
            format!(
                r#"<?xml version="1.0"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL">
  <Id>urn:uuid:cccccccc-0000-0000-0000-000000000000</Id>
  <ContentTitleText>t</ContentTitleText>
  <ReelList><Reel><Id>urn:uuid:b353da2a-703e-4d3f-8fcd-659930713ece</Id>
    <AssetList>
      <MainPicture><Id>urn:uuid:f76deec8-ab85-4f05-973d-089b67a55e5f</Id><Duration>48</Duration></MainPicture>
      <MainSubtitle><Id>urn:uuid:{sub_id}</Id><Duration>48</Duration></MainSubtitle>
    </AssetList>
  </Reel></ReelList>
</CompositionPlaylist>"#
            ),
        )
        .unwrap();

        let notes = verify_dcp(dir.path(), &VerifyOptions::default()).notes;
        for missing in ["ReelNumber", "Language"] {
            assert!(
                notes.iter().any(|n| n.code == Code::MissingRequiredElement
                    && n.message.contains(missing)
                    && n.file.as_deref() == Some(&*mxf)),
                "a wrapped subtitle missing {missing} must fire from verify_dcp, got: {notes:?}"
            );
        }
    }

    // ─── timed-text sizes and ST 429-5 identity ───────────────────────────

    /// The per-asset rules below look only at one asset, so the package around it
    /// can be empty. Owns the borrows the context needs.
    struct LoneAsset {
        paths: HashMap<&'static str, &'static str>,
        keys: crate::kdm::ContentKeys,
    }

    impl LoneAsset {
        fn new() -> Self {
            Self {
                paths: HashMap::new(),
                keys: crate::kdm::ContentKeys::none(),
            }
        }

        fn context<'a>(&'a self, dir: &'a Path) -> TimedTextContext<'a> {
            TimedTextContext {
                dcp_dir: dir,
                id_to_path: &self.paths,
                schema_dir: None,
                keys: &self.keys,
                standard: crate::Standard::Smpte,
            }
        }
    }

    fn read_fixture(path: &Path) -> crate::subtitle::WrappedTimedText {
        crate::subtitle::read_wrapped_timed_text(
            path,
            &crate::kdm::ContentKeys::none(),
            crate::subtitle::FontData::Include,
        )
        .expect("fixture is readable")
    }

    /// The document the fixtures wrap, whose Id the ResourceID must repeat.
    const FIXTURE_DOCUMENT: &str = r#"<?xml version="1.0"?>
<dcst:SubtitleReel xmlns:dcst="http://www.smpte-ra.org/schemas/428-7/2010/DCST">
  <dcst:Id>urn:uuid:11111111-2222-3333-4444-555555555555</dcst:Id>
  <dcst:ReelNumber>1</dcst:ReelNumber>
  <dcst:Language>en</dcst:Language>
  <dcst:LoadFont ID="f1">urn:uuid:abababab-abab-abab-abab-abababababab</dcst:LoadFont>
  <dcst:SubtitleList>
    <dcst:Font ID="f1">
      <dcst:Subtitle SpotNumber="1" TimeIn="00:00:05:000" TimeOut="00:00:07:000">
        <dcst:Text>Hi</dcst:Text>
      </dcst:Subtitle>
    </dcst:Font>
  </dcst:SubtitleList>
</dcst:SubtitleReel>"#;

    /// A reel asset declaring the duration the fixture essence actually carries.
    fn matching_reel_asset() -> crate::cpl::ReelAsset {
        crate::cpl::ReelAsset {
            duration: i64::from(crate::subtitle::tests::FIXTURE_CONTAINER_DURATION),
            ..Default::default()
        }
    }

    fn wrapped_notes(path: &Path, dir: &Path, asset: &crate::cpl::ReelAsset) -> Vec<Note> {
        let package = LoneAsset::new();
        package.context(dir).wrapped_asset_notes(
            &read_fixture(path),
            asset,
            TimedTextKind::Subtitle,
            path,
        )
    }

    #[test]
    fn a_conformant_wrapped_asset_draws_no_identity_or_duration_note() {
        let dir = tempfile::tempdir().unwrap();
        let path = crate::subtitle::tests::write_mxf(dir.path(), FIXTURE_DOCUMENT, None);
        let notes = wrapped_notes(&path, dir.path(), &matching_reel_asset());
        assert!(
            notes.is_empty(),
            "distinct ids, matching ResourceID and matching duration must be clean, got: {notes:?}"
        );
    }

    #[test]
    fn a_resource_id_that_is_not_the_document_id_fires() {
        let dir = tempfile::tempdir().unwrap();
        let path = crate::subtitle::tests::write_mxf_with_ids(
            dir.path(),
            FIXTURE_DOCUMENT,
            None,
            crate::subtitle::tests::FIXTURE_ASSET_UUID,
            [0x99; 16], // a ResourceID naming nothing in the document
        );
        let notes = wrapped_notes(&path, dir.path(), &matching_reel_asset());
        assert!(
            notes.iter().any(|n| n.code == Code::TimedTextIdMismatch
                && n.message.contains("does not match the document")),
            "a ResourceID unequal to the document Id must fire, got: {notes:?}"
        );
    }

    #[test]
    fn reusing_the_asset_id_as_the_resource_id_fires() {
        let dir = tempfile::tempdir().unwrap();
        let shared = [0x77; 16];
        let path = crate::subtitle::tests::write_mxf_with_ids(
            dir.path(),
            FIXTURE_DOCUMENT,
            None,
            shared,
            shared,
        );
        let notes = wrapped_notes(&path, dir.path(), &matching_reel_asset());
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::TimedTextIdMismatch && n.message.contains("reused")),
            "one id serving as both AssetID and ResourceID must fire, got: {notes:?}"
        );
    }

    #[test]
    fn essence_duration_disagreeing_with_the_reel_fires() {
        let dir = tempfile::tempdir().unwrap();
        let path = crate::subtitle::tests::write_mxf(dir.path(), FIXTURE_DOCUMENT, None);

        let mut wrong = matching_reel_asset();
        wrong.duration += 1;
        let notes = wrapped_notes(&path, dir.path(), &wrong);
        assert!(
            notes.iter().any(|n| n.code == Code::CplMismatchedDurations),
            "a reel Duration unequal to the essence duration must fire, got: {notes:?}"
        );

        // a reel that declares no Duration has nothing to disagree with
        let mut absent = matching_reel_asset();
        absent.duration = 0;
        assert!(
            !wrapped_notes(&path, dir.path(), &absent)
                .iter()
                .any(|n| n.code == Code::CplMismatchedDurations),
            "an undeclared reel Duration must stay silent"
        );
    }

    /// Ids and file names the size cases register in the package map.
    const TIMED_TEXT_ASSET_ID: &str = "11111111-2222-3333-4444-555555555555";
    const OVERSIZE_ASSET_FILE: &str = "sub.mxf";
    const CAPTION_FILE: &str = "ccap.xml";

    /// A package holding exactly one timed-text asset at `file`.
    struct OneAssetPackage {
        paths: HashMap<&'static str, &'static str>,
        keys: crate::kdm::ContentKeys,
    }

    impl OneAssetPackage {
        fn new(file: &'static str) -> Self {
            Self {
                paths: HashMap::from([(TIMED_TEXT_ASSET_ID, file)]),
                keys: crate::kdm::ContentKeys::none(),
            }
        }

        fn context<'a>(&'a self, dir: &'a Path) -> TimedTextContext<'a> {
            TimedTextContext {
                dcp_dir: dir,
                id_to_path: &self.paths,
                schema_dir: None,
                keys: &self.keys,
                standard: crate::Standard::Smpte,
            }
        }

        fn notes(&self, dir: &Path, kind: TimedTextKind) -> Vec<Note> {
            self.notes_for(dir, kind, &registered_asset())
        }

        fn notes_for(
            &self,
            dir: &Path,
            kind: TimedTextKind,
            asset: &crate::cpl::ReelAsset,
        ) -> Vec<Note> {
            self.context(dir).check_asset(asset, kind)
        }
    }

    /// The reel asset the one-asset package's map resolves.
    fn registered_asset() -> crate::cpl::ReelAsset {
        crate::cpl::ReelAsset {
            id: TIMED_TEXT_ASSET_ID.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn a_timed_text_entry_point_that_is_no_integer_fires() {
        let dir = tempfile::tempdir().unwrap();
        let package = OneAssetPackage::new(CAPTION_FILE);
        std::fs::write(dir.path().join(CAPTION_FILE), CONFORMANT_SUBTITLE).unwrap();

        let asset = crate::cpl::ReelAsset {
            entry_point_unparseable: true,
            ..registered_asset()
        };
        let notes = package.notes_for(dir.path(), TimedTextKind::ClosedCaption, &asset);
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::XmlParseError && n.message.contains("<EntryPoint>")),
            "an EntryPoint that is no integer must fire, got: {notes:?}"
        );
        assert!(
            !package
                .notes(dir.path(), TimedTextKind::ClosedCaption)
                .iter()
                .any(|n| n.code == Code::XmlParseError),
            "a readable EntryPoint draws nothing"
        );
    }

    #[test]
    fn a_timed_text_duration_that_is_no_integer_fires() {
        let dir = tempfile::tempdir().unwrap();
        let path = crate::subtitle::tests::write_mxf(dir.path(), FIXTURE_DOCUMENT, None);

        let asset = crate::cpl::ReelAsset {
            duration: 0,
            duration_unparseable: true,
            ..Default::default()
        };
        let notes = wrapped_notes(&path, dir.path(), &asset);
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::XmlParseError && n.message.contains("<Duration>")),
            "a Duration that is no integer must fire rather than skip the comparison, got: {notes:?}"
        );
    }

    // the caller notes for essence that could not be read at all
    #[test]
    fn a_loose_xml_asset_that_will_not_read_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let package = OneAssetPackage::new(CAPTION_FILE);
        // a directory in the asset's place: it exists, and reading it fails
        std::fs::create_dir(dir.path().join(CAPTION_FILE)).unwrap();

        let notes = package.notes(dir.path(), TimedTextKind::ClosedCaption);
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::SubtitleParseError && n.message.contains("failed to read")),
            "an unreadable XML asset must say no rule ran on it, got: {notes:?}"
        );
    }

    #[test]
    fn a_wrapped_asset_that_will_not_open_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let package = OneAssetPackage::new(OVERSIZE_ASSET_FILE);
        std::fs::write(dir.path().join(OVERSIZE_ASSET_FILE), b"not an MXF").unwrap();

        let notes = package.notes(dir.path(), TimedTextKind::Subtitle);
        assert!(
            notes.iter().any(|n| n.code == Code::MxfUnreadable),
            "an unopenable timed-text MXF must say no rule ran on it, got: {notes:?}"
        );
    }

    // a track file over the Bv2.1 ceiling is rejected before anyone tries to
    // play it, so the size is checked whether or not the essence parses.
    #[test]
    fn a_track_file_over_the_bv21_size_limit_fires() {
        let dir = tempfile::tempdir().unwrap();
        let package = OneAssetPackage::new(OVERSIZE_ASSET_FILE);
        let path = dir.path().join(OVERSIZE_ASSET_FILE);

        // sparse, so the case costs no disk: the rule reads the declared length
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_TIMED_TEXT_ASSET_BYTES + 1).unwrap();
        drop(file);
        assert!(
            package
                .notes(dir.path(), TimedTextKind::Subtitle)
                .iter()
                .any(|n| n.code == Code::TimedTextSizeExceeded),
            "a track file over 115 MB must fire"
        );

        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_TIMED_TEXT_ASSET_BYTES).unwrap();
        drop(file);
        assert!(
            !package
                .notes(dir.path(), TimedTextKind::Subtitle)
                .iter()
                .any(|n| n.code == Code::TimedTextSizeExceeded),
            "a track file exactly at the limit is within it"
        );
    }

    // the 256 kB cap is a caption rule; subtitles have no equivalent, so applying
    // it to both would reject conformant subtitle assets.
    #[test]
    fn an_oversize_caption_document_fires_but_the_same_subtitle_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let package = OneAssetPackage::new(CAPTION_FILE);
        let path = dir.path().join(CAPTION_FILE);

        let padding = "x".repeat(MAX_CLOSED_CAPTION_XML_BYTES);
        let document = CONFORMANT_SUBTITLE.replace(
            "<dcst:Text>Hi</dcst:Text>",
            &format!("<dcst:Text>{padding}</dcst:Text>"),
        );
        assert!(document.len() > MAX_CLOSED_CAPTION_XML_BYTES);
        std::fs::write(&path, &document).unwrap();

        assert!(
            package
                .notes(dir.path(), TimedTextKind::ClosedCaption)
                .iter()
                .any(|n| n.code == Code::TimedTextSizeExceeded),
            "a caption document over 256 kB must fire"
        );
        assert!(
            !package
                .notes(dir.path(), TimedTextKind::Subtitle)
                .iter()
                .any(|n| n.code == Code::TimedTextSizeExceeded),
            "the same size as a subtitle is not a finding: the cap is caption-only"
        );
    }

    #[test]
    fn embedded_fonts_over_the_bv21_limit_fire() {
        let dir = tempfile::tempdir().unwrap();
        let path = crate::subtitle::tests::write_mxf(dir.path(), FIXTURE_DOCUMENT, None);
        let wrapped = read_fixture(&path);
        let package = LoneAsset::new();
        let context = package.context(dir.path());
        let asset = matching_reel_asset();

        let over = crate::subtitle::WrappedTimedText {
            font_bytes: MAX_TIMED_TEXT_FONT_BYTES + 1,
            xml: wrapped.xml.clone(),
            resource_id: wrapped.resource_id,
            asset_id: wrapped.asset_id,
            container_duration: wrapped.container_duration,
            ..Default::default()
        };
        assert!(
            context
                .wrapped_asset_notes(&over, &asset, TimedTextKind::Subtitle, &path)
                .iter()
                .any(|n| n.code == Code::TimedTextSizeExceeded),
            "fonts over 10 MB must fire"
        );

        let at_limit = crate::subtitle::WrappedTimedText {
            font_bytes: MAX_TIMED_TEXT_FONT_BYTES,
            ..over
        };
        assert!(
            !context
                .wrapped_asset_notes(&at_limit, &asset, TimedTextKind::Subtitle, &path)
                .iter()
                .any(|n| n.code == Code::TimedTextSizeExceeded),
            "fonts exactly at the limit are within it"
        );
    }

    // SMPTE ST 428-7 carries the font asset id as the LoadFont element text, and
    // the ASSETMAP ids it resolves against are stored with urn:uuid: stripped.
    #[test]
    fn smpte_subtitle_font_urn_resolves_for_glyph_coverage() {
        let dir = tempfile::tempdir().unwrap();
        let sub_id = "11111111-2222-3333-4444-555555555555";
        let font_id = "22222222-3333-4444-5555-666666666666";

        std::fs::write(
            dir.path().join("ASSETMAP.xml"),
            format!(
                r#"<?xml version="1.0"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee</Id>
  <AssetList>
    <Asset><Id>urn:uuid:cccccccc-0000-0000-0000-000000000000</Id>
      <ChunkList><Chunk><Path>cpl.xml</Path></Chunk></ChunkList></Asset>
    <Asset><Id>urn:uuid:{sub_id}</Id>
      <ChunkList><Chunk><Path>sub.xml</Path></Chunk></ChunkList></Asset>
    <Asset><Id>urn:uuid:{font_id}</Id>
      <ChunkList><Chunk><Path>font.ttf</Path></Chunk></ChunkList></Asset>
  </AssetList>
</AssetMap>"#
            ),
        )
        .unwrap();

        std::fs::write(
            dir.path().join("cpl.xml"),
            format!(
                r#"<?xml version="1.0"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL">
  <Id>urn:uuid:cccccccc-0000-0000-0000-000000000000</Id>
  <ContentTitleText>t</ContentTitleText>
  <ReelList><Reel><Id>urn:uuid:b353da2a-703e-4d3f-8fcd-659930713ece</Id>
    <AssetList>
      <MainPicture><Id>urn:uuid:f76deec8-ab85-4f05-973d-089b67a55e5f</Id><Duration>48</Duration></MainPicture>
      <MainSubtitle><Id>urn:uuid:{sub_id}</Id><Duration>48</Duration></MainSubtitle>
    </AssetList>
  </Reel></ReelList>
</CompositionPlaylist>"#
            ),
        )
        .unwrap();

        // the font covers only 'H' and 'i', so the accented character has no glyph
        std::fs::write(
            dir.path().join("font.ttf"),
            crate::subtitle::tests::make_font(&['H', 'i']),
        )
        .unwrap();

        std::fs::write(
            dir.path().join("sub.xml"),
            format!(
                r#"<?xml version="1.0"?>
<dcst:SubtitleReel xmlns:dcst="http://www.smpte-ra.org/schemas/428-7/2010/DCST">
  <dcst:Id>urn:uuid:22222222-2222-3333-4444-555555555555</dcst:Id>
  <dcst:ReelNumber>1</dcst:ReelNumber>
  <dcst:Language>en</dcst:Language>
  <dcst:LoadFont ID="f">urn:uuid:{font_id}</dcst:LoadFont>
  <dcst:SubtitleList>
    <dcst:Font ID="f">
      <dcst:Subtitle SpotNumber="1" TimeIn="00:00:05:000" TimeOut="00:00:07:000">
        <dcst:Text>Hé</dcst:Text>
      </dcst:Subtitle>
    </dcst:Font>
  </dcst:SubtitleList>
</dcst:SubtitleReel>"#
            ),
        )
        .unwrap();

        let result = verify_dcp(dir.path(), &VerifyOptions::default());
        assert!(
            result
                .notes
                .iter()
                .any(|n| n.code == Code::SubtitleGlyphMissing && n.message.contains("U+00E9")),
            "expected a glyph-coverage note from the SMPTE LoadFont urn, got: {:?}",
            result.notes
        );
    }

    // Without --ov an unresolved ref is treated as a VF referencing an external
    // OV (matching ClairMeta): a warning, not a hard error. The broken-with-ov
    // case is covered by supplemental_dcp_with_ov_still_catches_genuinely_broken_ref.
    #[test]
    fn cpl_reference_to_unknown_asset_warns_without_ov() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ASSETMAP.xml"),
            r#"<?xml version="1.0"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee</Id>
  <AssetList>
    <Asset><Id>urn:uuid:cccccccc-0000-0000-0000-000000000000</Id>
      <ChunkList><Chunk><Path>cpl.xml</Path></Chunk></ChunkList></Asset>
  </AssetList>
</AssetMap>"#,
        )
        .unwrap();

        // MainPicture references an id that is not present in the ASSETMAP
        std::fs::write(
            dir.path().join("cpl.xml"),
            r#"<?xml version="1.0"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL">
  <Id>urn:uuid:cccccccc-0000-0000-0000-000000000000</Id>
  <ContentTitleText>t</ContentTitleText>
  <ReelList><Reel><Id>urn:uuid:b353da2a-703e-4d3f-8fcd-659930713ece</Id>
    <AssetList>
      <MainPicture><Id>urn:uuid:deadbeef-0000-0000-0000-000000000000</Id><Duration>48</Duration></MainPicture>
    </AssetList>
  </Reel></ReelList>
</CompositionPlaylist>"#,
        )
        .unwrap();

        let result = verify_dcp(dir.path(), &VerifyOptions::default());
        assert!(
            !result.notes.iter().any(|n| n.code == Code::CrossRefBroken),
            "without --ov an unresolved ref must not hard-fail, got: {:?}",
            result.notes
        );
        assert!(
            result
                .notes
                .iter()
                .any(|n| n.code == Code::SupplementalOvNotProvided),
            "expected SupplementalOvNotProvided warning, got: {:?}",
            result.notes
        );
    }

    // reel_too_short and non_ascii_filename must surface through the full
    // verify_dcp pipeline, not just their unit tests.
    #[test]
    fn short_reel_and_non_ascii_surface_from_verify() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ASSETMAP.xml"),
            r#"<?xml version="1.0"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee</Id>
  <AssetList>
    <Asset><Id>urn:uuid:cccccccc-0000-0000-0000-000000000000</Id>
      <ChunkList><Chunk><Path>cpl.xml</Path></Chunk></ChunkList></Asset>
  </AssetList>
</AssetMap>"#,
        )
        .unwrap();
        // 6 frames at 24 fps = 0.25 s
        std::fs::write(
            dir.path().join("cpl.xml"),
            r#"<?xml version="1.0"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL">
  <Id>urn:uuid:cccccccc-0000-0000-0000-000000000000</Id>
  <ContentTitleText>t</ContentTitleText>
  <ReelList><Reel><Id>urn:uuid:b353da2a-703e-4d3f-8fcd-659930713ece</Id>
    <AssetList>
      <MainPicture><Id>urn:uuid:f76deec8-ab85-4f05-973d-089b67a55e5f</Id><EditRate>24 1</EditRate><Duration>6</Duration></MainPicture>
    </AssetList>
  </Reel></ReelList>
</CompositionPlaylist>"#,
        )
        .unwrap();
        // a non-ASCII file name in the package
        std::fs::write(dir.path().join("naïve.txt"), b"x").unwrap();

        let result = verify_dcp(dir.path(), &VerifyOptions::default());
        assert!(
            result.notes.iter().any(|n| n.code == Code::ReelTooShort),
            "expected ReelTooShort, got: {:?}",
            result.notes
        );
        assert!(
            result
                .notes
                .iter()
                .any(|n| n.code == Code::NonAsciiFilename),
            "expected NonAsciiFilename, got: {:?}",
            result.notes
        );
    }

    #[test]
    fn imf_package_is_detected_from_its_cpl_namespace() {
        assert!(crate::imf::is_imf_package(&fixture("minimal_imf")));
    }

    // compliance::check_uuids is wired into verify_dcp: a malformed urn:uuid token
    // anywhere in the package XML must surface as invalid_uuid from `validate`.
    #[test]
    fn malformed_uuid_is_flagged_by_validate() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ASSETMAP.xml"),
            r#"<?xml version="1.0"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee</Id>
  <AnnotationText>urn:uuid:not-a-valid-uuid</AnnotationText>
  <AssetList>
    <Asset><Id>urn:uuid:cccccccc-0000-0000-0000-000000000000</Id>
      <ChunkList><Chunk><Path>cpl.xml</Path></Chunk></ChunkList></Asset>
  </AssetList>
</AssetMap>"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("cpl.xml"),
            r#"<?xml version="1.0"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL">
  <Id>urn:uuid:cccccccc-0000-0000-0000-000000000000</Id>
  <ContentTitleText>t</ContentTitleText>
  <ReelList><Reel><Id>urn:uuid:b353da2a-703e-4d3f-8fcd-659930713ece</Id>
    <AssetList>
      <MainPicture><Id>urn:uuid:f76deec8-ab85-4f05-973d-089b67a55e5f</Id><Duration>48</Duration></MainPicture>
    </AssetList>
  </Reel></ReelList>
</CompositionPlaylist>"#,
        )
        .unwrap();

        let result = verify_dcp(dir.path(), &VerifyOptions::default());
        assert!(
            result
                .notes
                .iter()
                .any(|n| n.code == Code::InvalidUuid && n.message.contains("not-a-valid-uuid")),
            "expected InvalidUuid from validate path, got: {:?}",
            result.notes
        );
    }

    // ─── OV-aware supplemental DCP cross-reference ─────────────────────────

    const PIC_ID: &str = "aaaaaaaa-1111-1111-1111-aaaaaaaaaaaa";
    const SND_ID: &str = "bbbbbbbb-2222-2222-2222-bbbbbbbbbbbb";

    /// Write a minimal SMPTE DCP dir: an ASSETMAP listing `asset_ids` plus a CPL
    /// referencing `pic_id`/`snd_id`. When `supplemental`, the CPL carries an OPL
    /// marker so it is detected as a version-file package.
    fn write_dcp(
        dir: &Path,
        cpl_id: &str,
        asset_ids: &[&str],
        pic_id: &str,
        snd_id: &str,
        supplemental: bool,
    ) {
        let mut asset_entries = format!(
            r#"<Asset><Id>urn:uuid:{cpl_id}</Id><ChunkList><Chunk><Path>cpl.xml</Path></Chunk></ChunkList></Asset>"#
        );
        for id in asset_ids {
            asset_entries.push_str(&format!(
                r#"<Asset><Id>urn:uuid:{id}</Id><ChunkList><Chunk><Path>{id}.mxf</Path></Chunk></ChunkList></Asset>"#
            ));
        }
        std::fs::write(
            dir.join("ASSETMAP.xml"),
            format!(
                r#"<?xml version="1.0"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:cccccccc-0000-0000-0000-000000000000</Id>
  <AssetList>{asset_entries}</AssetList>
</AssetMap>"#
            ),
        )
        .unwrap();

        let opl = if supplemental {
            "<OriginalPackagingList>ov</OriginalPackagingList>"
        } else {
            ""
        };
        std::fs::write(
            dir.join("cpl.xml"),
            format!(
                r#"<?xml version="1.0"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL">
  <Id>urn:uuid:{cpl_id}</Id>
  <ContentTitleText>t</ContentTitleText>
  {opl}
  <ReelList><Reel><Id>urn:uuid:b353da2a-703e-4d3f-8fcd-659930713ece</Id>
    <AssetList>
      <MainPicture><Id>urn:uuid:{pic_id}</Id><Duration>48</Duration></MainPicture>
      <MainSound><Id>urn:uuid:{snd_id}</Id><Duration>48</Duration></MainSound>
    </AssetList>
  </Reel></ReelList>
</CompositionPlaylist>"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn supplemental_dcp_with_ov_resolves_cross_package_refs() {
        let ov = tempfile::tempdir().unwrap();
        write_dcp(
            ov.path(),
            "0f0f0f0f-0000-0000-0000-000000000000",
            &[PIC_ID],
            PIC_ID,
            PIC_ID,
            false,
        );
        let supp = tempfile::tempdir().unwrap();
        // supp physically holds only SND_ID; its CPL references OV's PIC_ID + local SND_ID
        write_dcp(
            supp.path(),
            "1a1a1a1a-0000-0000-0000-000000000000",
            &[SND_ID],
            PIC_ID,
            SND_ID,
            true,
        );

        let opts = VerifyOptions {
            ov: Some(ov.path().to_path_buf()),
            ..VerifyOptions::default()
        };
        let result = verify_dcp(supp.path(), &opts);
        assert!(
            !result.notes.iter().any(|n| n.code == Code::CrossRefBroken),
            "OV must satisfy the picture ref, got: {:?}",
            result.notes
        );
        assert!(
            !result
                .notes
                .iter()
                .any(|n| n.code == Code::SupplementalOvNotProvided),
            "everything resolves, no OV-missing note expected"
        );
    }

    #[test]
    fn supplemental_dcp_alone_reports_missing_ov_not_broken() {
        let supp = tempfile::tempdir().unwrap();
        write_dcp(
            supp.path(),
            "1a1a1a1a-0000-0000-0000-000000000000",
            &[SND_ID],
            PIC_ID,
            SND_ID,
            true,
        );

        let result = verify_dcp(supp.path(), &VerifyOptions::default());
        assert!(
            !result.notes.iter().any(|n| n.code == Code::CrossRefBroken),
            "supplemental with no OV must not hard-fail as broken, got: {:?}",
            result.notes
        );
        assert!(
            result
                .notes
                .iter()
                .any(|n| n.code == Code::SupplementalOvNotProvided),
            "expected SupplementalOvNotProvided diagnostic, got: {:?}",
            result.notes
        );
    }

    #[test]
    fn supplemental_dcp_with_ov_still_catches_genuinely_broken_ref() {
        let ov = tempfile::tempdir().unwrap();
        write_dcp(
            ov.path(),
            "0f0f0f0f-0000-0000-0000-000000000000",
            &[PIC_ID],
            PIC_ID,
            PIC_ID,
            false,
        );
        let supp = tempfile::tempdir().unwrap();
        // picture ref points at an id present in neither OV nor supp
        write_dcp(
            supp.path(),
            "1a1a1a1a-0000-0000-0000-000000000000",
            &[SND_ID],
            "deadbeef-0000-0000-0000-000000000000",
            SND_ID,
            true,
        );

        let opts = VerifyOptions {
            ov: Some(ov.path().to_path_buf()),
            ..VerifyOptions::default()
        };
        let result = verify_dcp(supp.path(), &opts);
        assert!(
            result.notes.iter().any(|n| n.code == Code::CrossRefBroken),
            "a ref in neither package is a real break even with --ov, got: {:?}",
            result.notes
        );
    }

    // every check under check_picture_details is gated on a descriptor ffprobe is
    // the only source of
    #[test]
    fn a_track_file_no_stream_was_probed_from_says_the_picture_checks_did_not_run() {
        let dir = tempfile::tempdir().unwrap();
        write_dcp(
            dir.path(),
            "0f0f0f0f-0000-0000-0000-000000000000",
            &[PIC_ID],
            PIC_ID,
            SND_ID,
            false,
        );
        // a SMPTE partition-pack header and nothing ffprobe can make a stream of
        let mut essence = vec![0x06, 0x0e, 0x2b, 0x34];
        essence.resize(64, 0);
        std::fs::write(dir.path().join(format!("{PIC_ID}.mxf")), &essence).unwrap();

        let opts = VerifyOptions {
            check_picture_details: true,
            ..VerifyOptions::default()
        };
        let result = verify_dcp(dir.path(), &opts);
        let skipped = result
            .notes
            .iter()
            .find(|n| n.code == Code::CheckSkipped && n.message.contains("picture essence checks"))
            .unwrap_or_else(|| {
                panic!(
                    "the skipped picture checks must be reported: {:?}",
                    result.notes
                )
            });
        assert_eq!(skipped.severity, Severity::Warning);
        assert!(
            skipped.message.contains(PIC_ID),
            "the note must name the track file, got: {}",
            skipped.message
        );
    }

    // A normal scope DCP codes 2048x858 (2K) or 4096x1716 (4K), not the full
    // container, so those sizes must not draw picture_invalid_resolution.
    #[test]
    fn scope_coded_sizes_are_standard() {
        assert!(is_standard_picture_size(2048, 858));
        assert!(is_standard_picture_size(4096, 1716));
    }

    #[test]
    fn container_and_flat_sizes_stay_standard() {
        assert!(is_standard_picture_size(2048, 1080));
        assert!(is_standard_picture_size(1998, 1080));
        assert!(is_standard_picture_size(4096, 2160));
        assert!(is_standard_picture_size(3996, 2160));
    }

    #[test]
    fn a_broadcast_size_is_not_standard() {
        assert!(!is_standard_picture_size(1920, 1080));
    }
}
