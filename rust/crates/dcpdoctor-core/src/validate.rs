use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::dcp;
use crate::hash::sha1_base64;
use crate::{Code, Note, Severity, VerifyOptions, VerifyResult};

/// Verify a DCP at the given path.
pub fn verify_dcp(dcp_dir: &Path, opts: &VerifyOptions) -> VerifyResult {
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
            if let Some(ref snd) = mxf_info.sound
                && snd.sample_rate > 0
                && snd.sample_rate != 48000
                && snd.sample_rate != 96000
            {
                result.add(Note {
                    severity: Severity::Warning,
                    code: Code::SoundInvalidSampleRate,
                    message: format!("Non-standard audio sample rate: {} Hz", snd.sample_rate),
                    file: Some(full_path.clone()),
                    line: 0,
                });
            }
        }
    }

    result
}
