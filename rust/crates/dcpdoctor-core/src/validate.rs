use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::dcp;
use crate::hash::sha1_base64;
use crate::{Code, Note, Severity, VerifyOptions, VerifyResult};

/// Verify a DCP at the given path.
pub fn verify_dcp(dcp_dir: &Path, opts: &VerifyOptions) -> VerifyResult {
    if crate::imf::is_imf_package(dcp_dir) {
        return verify_imp(dcp_dir, opts.ov.as_deref());
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

    // 0. XSD schema validation, when a schema dir is available (schema-path
    // driven, see schema::locate_schema_dir). Validates every CPL/PKL/ASSETMAP
    // against the SMPTE/Interop XSDs, emitting xml_schema_violation.
    if let Some(schema_dir) = crate::schema::locate_schema_dir() {
        for note in crate::schema::check_schema(&dcp.assetmap_path, &schema_dir) {
            result.add(note);
        }
        for (pkl_path, _) in &dcp.pkls {
            for note in crate::schema::check_schema(pkl_path, &schema_dir) {
                result.add(note);
            }
        }
        for (cpl_path, _) in &dcp.cpls {
            for note in crate::schema::check_schema(cpl_path, &schema_dir) {
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

    for (cpl_path, cpl) in &dcp.cpls {
        if opts.check_signatures {
            for note in crate::signature::verify_signature(cpl_path, opts.strict_smpte) {
                result.add(note);
            }
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
            if reel.sound.duration > 0 && reel.sound.duration != reel.picture.duration {
                result.add(Note {
                    severity: Severity::Warning,
                    code: Code::CplMismatchedDurations,
                    message: format!(
                        "Sound duration differs from picture duration in reel {}",
                        reel.id
                    ),
                    file: Some(cpl_path.clone()),
                    line: 0,
                });
            }

            // EditRate validation (strict mode)
            if opts.strict_smpte && !reel.picture.edit_rate.is_empty() {
                const VALID_EDIT_RATES: &[&str] = &["24 1", "25 1", "30 1", "48 1", "50 1", "60 1"];
                if !VALID_EDIT_RATES.contains(&reel.picture.edit_rate.as_str()) {
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
    }

    // 4b. Validate subtitle assets referenced by CPL reels
    for (_cpl_path, cpl) in &dcp.cpls {
        for reel in &cpl.reels {
            if reel.subtitle.id.is_empty() {
                continue;
            }
            if let Some(&asset_path) = id_to_path.get(reel.subtitle.id.as_str()) {
                let full_path = dcp_dir.join(asset_path);
                // SMPTE subtitles are usually MXF-wrapped; only the plain-XML form is inspectable here
                let is_xml = full_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("xml"));
                if is_xml && full_path.exists() {
                    for note in crate::subtitle::validate_subtitle(&full_path, dcp.standard) {
                        result.add(note);
                    }
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
                        // SMPTE ST 428-7: font is an asset addressed by urn
                        if let Some(urn) = &decl.urn
                            && let Some(&ap) = id_to_path.get(urn.as_str())
                        {
                            let p = dcp_dir.join(ap);
                            return p.exists().then_some(p);
                        }
                        None
                    };
                    for note in crate::subtitle::check_glyph_coverage(&full_path, resolve) {
                        result.add(note);
                    }
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
                for note in crate::j2k::check_picture_j2k_mxf(&full_path, fps) {
                    result.add(note);
                }
            }

            // Picture validation
            if let Some(ref pic) = mxf_info.picture
                && pic.width > 0
                && pic.height > 0
                && opts.strict_smpte
            {
                let valid_res = matches!(
                    (pic.width, pic.height),
                    (2048, 1080) | (1998, 1080) | (4096, 2160) | (3996, 2160)
                );
                if !valid_res {
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

                // SMPTE ST 429-4: 250 Mbps for 2K, 500 Mbps for 4K
                let max_bitrate = if pic.width > 2048 { 500.0 } else { 250.0 };

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

fn verify_imp(imp_dir: &Path, ov_dir: Option<&Path>) -> VerifyResult {
    let mut result = VerifyResult {
        standard: crate::Standard::Smpte,
        ..Default::default()
    };

    // Native IMF validation works everywhere including WASM.
    for note in crate::imf::validate_imp(imp_dir, ov_dir) {
        result.add(note);
    }

    // Photon adds deep IMF conformance checks.
    match crate::photon::run_photon(imp_dir) {
        Ok(photon_notes) => {
            for note in photon_notes {
                result.add(note);
            }
        }
        Err(error) => {
            result.add(Note {
                severity: Severity::Warning,
                code: Code::MissingRequiredElement,
                message: format!("[Photon] {error}"),
                file: None,
                line: 0,
            });
        }
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
}
