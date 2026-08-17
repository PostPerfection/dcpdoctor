//! QC report generation (HTML and PDF via wkhtmltopdf/weasyprint).

use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::Command;

use serde::Serialize;

/// Options for detailed QC report generation.
pub struct DetailedQcOptions {
    pub imp_dir: PathBuf,
    pub output_file: PathBuf,
    pub title: String,
    pub client: String,
    pub include_loudness: bool,
    /// Scan every picture track's codestreams for the forensics section. Costs a
    /// pass over the picture essence, the way loudness costs one over the audio.
    pub include_codestream_forensics: bool,
}

/// Result of QC report generation.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DetailedQcResult {
    pub success: bool,
    pub error: String,
    pub output_file: PathBuf,
    pub pages: u32,
}

/// Track info for the report.
struct TrackInfo {
    track_type: String,
    filename: String,
    uuid: String,
    size: u64,
}

fn current_timestamp() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown".into())
}

/// Generate a detailed HTML (or PDF) QC report for a DCP/IMP.
pub fn generate_detailed_qc(opts: &DetailedQcOptions) -> DetailedQcResult {
    let mut result = DetailedQcResult::default();

    if !opts.imp_dir.exists() {
        result.error = format!("IMP directory not found: {}", opts.imp_dir.display());
        return result;
    }

    // Gather basic info
    let tracks = gather_track_info(&opts.imp_dir);
    let title = if opts.title.is_empty() {
        "DCP/IMP".to_string()
    } else {
        opts.title.clone()
    };

    let want_pdf = opts.output_file.extension().is_some_and(|ext| ext == "pdf");
    let html_path = if want_pdf {
        opts.output_file.with_extension("tmp.html")
    } else {
        opts.output_file.clone()
    };

    // Build HTML
    let mut html = String::with_capacity(4096);
    html.push_str("<!DOCTYPE html>\n<html>\n<head>\n");
    html.push_str("<meta charset=\"utf-8\">\n");
    let _ = writeln!(html, "<title>QC Report — {title}</title>");
    html.push_str("<style>\n");
    html.push_str("body { font-family: -apple-system, sans-serif; max-width: 1000px; margin: 0 auto; padding: 2rem; }\n");
    html.push_str("h1 { border-bottom: 2px solid #333; padding-bottom: 0.5rem; }\n");
    html.push_str("table { width: 100%; border-collapse: collapse; margin: 1rem 0; }\n");
    html.push_str("th, td { border: 1px solid #ddd; padding: 0.5rem; text-align: left; }\n");
    html.push_str("th { background: #f5f5f5; }\n");
    html.push_str(".pass { color: #16a34a; font-weight: bold; }\n");
    html.push_str(".fail { color: #dc2626; font-weight: bold; }\n");
    html.push_str(".meta { color: #666; font-size: 0.9rem; }\n");
    html.push_str("</style>\n</head>\n<body>\n");

    html.push_str("<h1>QC Report</h1>\n");
    let _ = writeln!(
        html,
        "<p class=\"meta\">Generated: {}</p>",
        current_timestamp()
    );
    if !opts.client.is_empty() {
        let _ = writeln!(html, "<p class=\"meta\">Client: {}</p>", opts.client);
    }

    // Package info
    html.push_str("<h2>Package Information</h2>\n");
    html.push_str("<table>\n");
    let _ = writeln!(html, "<tr><th>Title</th><td>{title}</td></tr>");
    let _ = writeln!(
        html,
        "<tr><th>Directory</th><td>{}</td></tr>",
        opts.imp_dir.display()
    );
    html.push_str("</table>\n");

    // Track files
    html.push_str("<h2>Track Files</h2>\n");
    html.push_str("<table>\n");
    html.push_str("<tr><th>Type</th><th>Filename</th><th>UUID</th><th>Size</th></tr>\n");
    for t in &tracks {
        let size_mb = t.size / 1024 / 1024;
        let _ = writeln!(
            html,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{size_mb} MB</td></tr>",
            t.track_type, t.filename, t.uuid
        );
    }
    html.push_str("</table>\n");

    // Codestream forensics per picture essence, if requested
    if opts.include_codestream_forensics {
        let family = if crate::imf::is_imf_package(&opts.imp_dir) {
            crate::j2k::PictureEssenceFamily::Imf
        } else {
            crate::j2k::PictureEssenceFamily::Cinema
        };
        let mut sections = String::new();
        for t in &tracks {
            if t.track_type != "essence" {
                continue;
            }
            let path = opts.imp_dir.join(&t.filename);
            let (_notes, forensics) = crate::j2k::check_picture_j2k_mxf(
                &path,
                &crate::kdm::ContentKeys::none(),
                family,
                true,
            );
            // sound and unreadable essence yield nothing to report
            let Some(forensics) = forensics else {
                continue;
            };
            let _ = writeln!(sections, "<h3>{}</h3>", t.filename);
            sections.push_str(&codestream_forensics_table(&forensics));
        }
        if !sections.is_empty() {
            html.push_str("<h2>Codestream forensics</h2>\n");
            html.push_str(&sections);
        }
    }

    // Loudness (EBU R128) per audio essence, if requested
    if opts.include_loudness {
        let mut rows = String::new();
        for t in &tracks {
            if t.track_type != "essence" {
                continue;
            }
            let path = opts.imp_dir.join(&t.filename);
            if let Ok(m) = crate::audio::measure_loudness(&path) {
                // Leq(m) (ISO 21727) reported alongside the EBU R128 result
                let leq = crate::loudness::measure_leq_m(&path);
                let leq_cell = if leq.success {
                    format!("{:.1} dB", leq.leq_m_db)
                } else {
                    "n/a".to_string()
                };
                let _ = writeln!(
                    rows,
                    "<tr><td>{}</td><td>{:.1} LUFS</td><td>{:.1} dBTP</td><td>{:.1} LU</td><td>{leq_cell}</td></tr>",
                    t.filename, m.integrated_lufs, m.true_peak_dbtp, m.loudness_range_lu
                );
            }
        }
        if !rows.is_empty() {
            html.push_str("<h2>Loudness (EBU R128 and Leq(m))</h2>\n<table>\n");
            html.push_str(
                "<tr><th>File</th><th>Integrated</th><th>True Peak</th><th>Range</th><th>Leq(m)</th></tr>\n",
            );
            html.push_str(&rows);
            html.push_str("</table>\n");
        }
    }

    html.push_str("</body>\n</html>\n");

    if std::fs::write(&html_path, &html).is_err() {
        result.error = format!("Cannot write report to {}", html_path.display());
        return result;
    }

    // PDF conversion if requested
    if want_pdf {
        let html_str = html_path.to_string_lossy();
        let out_str = opts.output_file.to_string_lossy();

        let ok = Command::new("wkhtmltopdf")
            .args(["--quiet", &html_str, &out_str])
            .status()
            .is_ok_and(|s| s.success())
            || Command::new("weasyprint")
                .args([html_str.as_ref(), out_str.as_ref()])
                .status()
                .is_ok_and(|s| s.success());

        let _ = std::fs::remove_file(&html_path);

        if !ok {
            result.error = "PDF conversion failed — install wkhtmltopdf or weasyprint".into();
            return result;
        }
    }

    result.output_file = opts.output_file.clone();
    result.pages = 1;
    result.success = true;
    result
}

/// One picture track's codestream forensics as a parameter/value table.
fn codestream_forensics_table(forensics: &crate::j2k::CodestreamForensics) -> String {
    let reference = &forensics.reference;
    let info = &reference.info;
    let (codeblock_width, codeblock_height) = info.codeblock_size();
    let present = |yes: bool| if yes { "present" } else { "absent" }.to_string();
    let worst_frame = match forensics.dci_frame_byte_cap.zip(forensics.cap_percentage()) {
        Some((cap, percentage)) => format!(
            "{} bytes, {percentage:.0}% of the {cap} byte DCI cap, at frame {} / {}",
            forensics.worst_frame_bytes,
            forensics.worst_frame_index,
            forensics.worst_frame_timecode()
        ),
        None => format!(
            "{} bytes at frame {} / {}",
            forensics.worst_frame_bytes,
            forensics.worst_frame_index,
            forensics.worst_frame_timecode()
        ),
    };
    let mut rows: Vec<(&str, String)> = vec![
        ("Resolution", format!("{}x{}", info.width, info.height)),
        ("Profile", info.profile.clone()),
        (
            "Components",
            format!("{} x {}-bit", info.components, info.bit_depth),
        ),
        (
            "Decomposition levels",
            info.decomposition_levels.to_string(),
        ),
        (
            "Code-block size",
            format!("{codeblock_width}x{codeblock_height}"),
        ),
        (
            "Wavelet transform",
            if info.irreversible_transform {
                "9-7 irreversible".to_string()
            } else {
                "5-3 reversible".to_string()
            },
        ),
        ("Quality layers", info.layers.to_string()),
        ("Progression order", info.progression_order.clone()),
        ("Tiles", reference.tile_count.to_string()),
        (
            "Tile-parts",
            format!(
                "{} (max {} at frame {})",
                reference.tile_part_count,
                forensics.max_tile_part_count,
                forensics.max_tile_part_count_frame
            ),
        ),
        (
            "Multiple component transform",
            if reference.multiple_component_transform {
                "on".to_string()
            } else {
                "off".to_string()
            },
        ),
        ("TLM marker", present(reference.tlm_present)),
        ("POC marker", present(reference.poc_present)),
        (
            "Frames scanned",
            format!(
                "{}{}",
                forensics.frames_scanned,
                if forensics.stereoscopic {
                    " (both eyes)"
                } else {
                    ""
                }
            ),
        ),
        ("Worst frame", worst_frame),
    ];
    // IMF essence has no DCI cap, so the row would only report a zero
    if forensics.dci_frame_byte_cap.is_some() {
        rows.push((
            "Frames over the DCI cap",
            forensics.frames_over_dci_cap.to_string(),
        ));
    }
    rows.push(("Parameters constant", forensics.parameters_constant_text()));

    let mut table = String::from("<table>\n");
    for (name, value) in rows {
        let _ = writeln!(table, "<tr><th>{name}</th><td>{value}</td></tr>");
    }
    table.push_str("</table>\n");
    table
}

fn gather_track_info(dir: &std::path::Path) -> Vec<TrackInfo> {
    let mut tracks = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return tracks;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().unwrap_or_default().to_string_lossy();
        let track_type = match ext.as_ref() {
            "mxf" => "essence",
            "xml" => "metadata",
            _ => continue,
        };
        let filename = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        tracks.push(TrackInfo {
            track_type: track_type.into(),
            filename,
            uuid: String::new(),
            size,
        });
    }
    tracks
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFORMANT_DECOMPOSITION_LEVELS: u8 = 5;
    const PAYLOAD_BYTES: usize = 64;

    #[test]
    fn the_report_carries_a_codestream_forensics_section_per_picture_track() {
        let dir = tempfile::tempdir().unwrap();
        let frames = vec![(CONFORMANT_DECOMPOSITION_LEVELS, PAYLOAD_BYTES); 3];
        crate::j2k::frame_scan_tests::write_picture_mxf(dir.path(), "picture.mxf", &frames);

        let output = dir.path().join("report.html");
        let result = generate_detailed_qc(&DetailedQcOptions {
            imp_dir: dir.path().to_path_buf(),
            output_file: output.clone(),
            title: "Forensics".into(),
            client: String::new(),
            include_loudness: false,
            include_codestream_forensics: true,
        });
        assert!(result.success, "report failed: {}", result.error);

        let html = std::fs::read_to_string(&output).unwrap();
        assert!(html.contains("<h2>Codestream forensics</h2>"));
        assert!(html.contains("<h3>picture.mxf</h3>"));
        for expected in [
            "<td>2048x1080</td>",
            "<td>Cinema 2K</td>",
            "<th>Decomposition levels</th><td>5</td>",
            "<th>Tile-parts</th><td>3 (max 3 at frame 0)</td>",
            "<th>Parameters constant</th><td>yes</td>",
        ] {
            assert!(html.contains(expected), "report must carry {expected:?}");
        }
    }

    /// Resolution of the IMF picture fixture, which is not a DCI one.
    const IMF_WIDTH: u32 = 1920;
    const IMF_HEIGHT: u32 = 1080;

    /// The least that makes a directory read as an IMP: a CPL in the ST 2067-3
    /// namespace next to the track files.
    fn write_imf_composition_playlist(dir: &std::path::Path) {
        std::fs::write(
            dir.join("CPL.xml"),
            r#"<?xml version="1.0"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/2067-3/2016">
  <Id>urn:uuid:1a1a1a1a-0000-0000-0000-000000000000</Id>
</CompositionPlaylist>"#,
        )
        .unwrap();
    }

    #[test]
    fn the_imf_forensics_section_leaves_out_the_dci_cap() {
        let dir = tempfile::tempdir().unwrap();
        write_imf_composition_playlist(dir.path());
        let frames = vec![(CONFORMANT_DECOMPOSITION_LEVELS, PAYLOAD_BYTES); 3];
        crate::j2k::frame_scan_tests::write_as02_picture_mxf(
            dir.path(),
            "picture.mxf",
            IMF_WIDTH,
            IMF_HEIGHT,
            &frames,
        );

        let output = dir.path().join("report.html");
        let result = generate_detailed_qc(&DetailedQcOptions {
            imp_dir: dir.path().to_path_buf(),
            output_file: output.clone(),
            title: "Forensics".into(),
            client: String::new(),
            include_loudness: false,
            include_codestream_forensics: true,
        });
        assert!(result.success, "report failed: {}", result.error);

        let html = std::fs::read_to_string(&output).unwrap();
        assert!(html.contains("<h2>Codestream forensics</h2>"));
        assert!(html.contains("<td>1920x1080</td>"));
        assert!(html.contains("<th>Parameters constant</th><td>yes</td>"));
        assert!(
            !html.contains("DCI cap"),
            "IMF essence is held to no DCI cap: {html}"
        );
    }

    #[test]
    fn the_forensics_section_is_left_out_when_not_requested() {
        let dir = tempfile::tempdir().unwrap();
        let frames = vec![(CONFORMANT_DECOMPOSITION_LEVELS, PAYLOAD_BYTES); 3];
        crate::j2k::frame_scan_tests::write_picture_mxf(dir.path(), "picture.mxf", &frames);

        let output = dir.path().join("report.html");
        let result = generate_detailed_qc(&DetailedQcOptions {
            imp_dir: dir.path().to_path_buf(),
            output_file: output.clone(),
            title: "Forensics".into(),
            client: String::new(),
            include_loudness: false,
            include_codestream_forensics: false,
        });
        assert!(result.success, "report failed: {}", result.error);
        assert!(
            !std::fs::read_to_string(&output)
                .unwrap()
                .contains("Codestream forensics")
        );
    }
}
