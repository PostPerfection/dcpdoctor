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
