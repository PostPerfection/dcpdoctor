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

/// Verify a DCP at the given path.
pub fn verify_dcp(dcp_dir: &Path, opts: &VerifyOptions) -> VerifyResult {
    if crate::imf::is_imf_package(dcp_dir) {
        return verify_imp(dcp_dir, opts.ov.as_deref(), opts.check_picture_details);
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
        // Verify PKL asset sizes (cheap, so not gated on check_hashes)
        for pkl_asset in &pkl.assets {
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

    // 4b. Validate subtitle assets referenced by CPL reels
    for (_cpl_path, cpl) in &dcp.cpls {
        for reel in &cpl.reels {
            if reel.subtitle.id.is_empty() {
                continue;
            }
            if let Some(&asset_path) = id_to_path.get(reel.subtitle.id.as_str()) {
                let full_path = dcp_dir.join(asset_path);
                if !full_path.exists() {
                    continue;
                }
                // A loose XML asset is read from disk, a SMPTE ST 429-5 asset is
                // unwrapped from its MXF; from there both take the same path, so a
                // wrapped subtitle gets the structural rules and not only glyphs.
                let is_xml = full_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("xml"));

                let (xml, glyph_notes) = if is_xml {
                    // glyph coverage: resolve each LoadFont to a font file and
                    // check every used code point against its cmap.
                    let sub_dir = full_path.parent().unwrap_or(dcp_dir).to_path_buf();
                    let resolve = |decl: &crate::subtitle::FontDecl| -> Option<std::path::PathBuf> {
                        // Interop DCSubtitle: font is a file referenced by URI
                        if let Some(uri) = &decl.uri {
                            let rel = sub_dir.join(uri);
                            if rel.exists() {
                                return Some(rel);
                            }
                            let abs = dcp_dir.join(uri);
                            return abs.exists().then_some(abs);
                        }
                        // SMPTE ST 428-7: font is an asset addressed by urn.
                        // ASSETMAP ids are stored with urn:uuid: stripped.
                        if let Some(urn) = &decl.urn
                            && let Some(&ap) =
                                id_to_path.get(crate::assetmap::strip_urn_uuid(urn).as_str())
                        {
                            let p = dcp_dir.join(ap);
                            return p.exists().then_some(p);
                        }
                        None
                    };
                    (
                        std::fs::read_to_string(&full_path).ok(),
                        crate::subtitle::check_glyph_coverage(&full_path, resolve),
                    )
                } else {
                    // one trip through the MXF feeds the schema, structural and
                    // glyph passes (decrypting with the KDM where one was supplied)
                    match crate::subtitle::read_wrapped_timed_text(&full_path, &content_keys) {
                        Some(asset) => {
                            for note in asset.notes.iter().cloned() {
                                result.add(note);
                            }
                            if asset.is_unreadable() {
                                (None, Vec::new())
                            } else {
                                let glyphs = crate::subtitle::check_glyph_coverage_wrapped(
                                    &asset, &full_path,
                                );
                                (Some(asset.xml), glyphs)
                            }
                        }
                        None => (None, Vec::new()),
                    }
                };

                if let Some(xml) = xml {
                    if let Some(schema_dir) = schema_dir.as_deref() {
                        for note in crate::schema::check_schema_xml(&xml, &full_path, schema_dir) {
                            result.add(note);
                        }
                    }
                    for note in
                        crate::subtitle::validate_subtitle_xml(&xml, &full_path, dcp.standard)
                    {
                        result.add(note);
                    }
                }
                for note in glyph_notes {
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

    for note in crate::validators::check_encryption(dcp_dir, &cpl_paths) {
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
        if !cpl.content_title.is_empty() {
            for note in crate::isdcf::check_isdcf_naming(&cpl.content_title, cpl_path) {
                result.add(note);
            }
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
        for note in
            crate::validators::check_first_subtitle_timing(cpl_path, dcp.standard, &id_to_file)
        {
            result.add(note);
        }
        for note in crate::validators::check_timed_text_content(cpl_path, dcp.standard, &id_to_file)
        {
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
        for note in crate::validators::check_subtitle_frame_rate(cpl_path, &id_to_file) {
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

            // Codestream checks on picture essence: 0xFFFF legacy constraint
            // (SMPTE Cat. 862) and ISO 15444-1 cinema profile constraints.
            if let Some(ref pic) = mxf_info.picture {
                let fps = if pic.frame_rate_num > 0 && pic.frame_rate_den > 0 {
                    pic.frame_rate_num as f64 / pic.frame_rate_den as f64
                } else {
                    24.0
                };
                for note in crate::j2k::check_picture_j2k_mxf(
                    &full_path,
                    fps,
                    &content_keys,
                    opts.scan_every_frame,
                ) {
                    result.add(note);
                }

                let bitrate = crate::bitrate::analyze_picture_bitrate(&full_path);
                for note in crate::bitrate::check_bitrate_compliance(&bitrate, &full_path) {
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
    }

    result
}

/// Build the content keys from `opts.kdm` + `opts.recipient_key`, adding any
/// KDM window / CPL-match / unwrap-failure notes to `result`. Returns
/// `ContentKeys::none()` when no KDM is supplied (or unwrap fails), so encrypted
/// essence keeps skipping instead of producing garbage findings.
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

fn build_content_keys(
    dcp_dir: &Path,
    opts: &VerifyOptions,
    result: &mut VerifyResult,
) -> crate::kdm::ContentKeys {
    let Some(kdm_path) = opts.kdm.as_deref() else {
        return crate::kdm::ContentKeys::none();
    };

    // window (expired / not-yet-valid) + CPL-match checks (extends the validator)
    for note in crate::kdm::validate_kdm(kdm_path, Some(dcp_dir)) {
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

fn verify_imp(imp_dir: &Path, ov_dir: Option<&Path>, check_picture_details: bool) -> VerifyResult {
    let mut result = VerifyResult {
        standard: crate::Standard::Smpte,
        ..Default::default()
    };

    // Native IMF validation works everywhere including WASM.
    for note in crate::imf::validate_imp(imp_dir, ov_dir, check_picture_details) {
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
