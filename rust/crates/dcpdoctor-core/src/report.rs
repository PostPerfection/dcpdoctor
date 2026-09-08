use std::io::Write;
use std::path::Path;

use crate::VerifyResult;

/// Report output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportFormat {
    Text,
    Json,
    Html,
}

/// Write a verification report.
pub fn write_report<W: Write>(
    result: &VerifyResult,
    dcp_path: &Path,
    writer: &mut W,
    format: ReportFormat,
) -> std::io::Result<()> {
    match format {
        ReportFormat::Text => write_text_report(result, dcp_path, writer),
        ReportFormat::Json => write_json_report(result, writer),
        ReportFormat::Html => write_html_report(result, dcp_path, writer),
    }
}

fn write_text_report<W: Write>(
    result: &VerifyResult,
    dcp_path: &Path,
    writer: &mut W,
) -> std::io::Result<()> {
    writeln!(writer, "=== DcpDoctor Report ===")?;
    writeln!(writer, "DCP: {}", dcp_path.display())?;
    writeln!(writer, "Standard: {}", result.standard)?;
    writeln!(
        writer,
        "Result: {} ({} errors, {} warnings)",
        if result.ok() { "PASS" } else { "FAIL" },
        result.error_count,
        result.warning_count
    )?;
    writeln!(writer)?;

    for note in &result.notes {
        writeln!(writer, "{}", note)?;
    }

    Ok(())
}

fn write_json_report<W: Write>(result: &VerifyResult, writer: &mut W) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(result).map_err(std::io::Error::other)?;
    writer.write_all(json.as_bytes())
}

fn write_html_report<W: Write>(
    result: &VerifyResult,
    dcp_path: &Path,
    writer: &mut W,
) -> std::io::Result<()> {
    writeln!(writer, "<!DOCTYPE html><html><head><meta charset='utf-8'>")?;
    writeln!(writer, "<title>DcpDoctor Report</title>")?;
    writeln!(writer, "<style>")?;
    writeln!(writer, "body{{font-family:sans-serif;margin:2em}}")?;
    writeln!(writer, "table{{border-collapse:collapse;width:100%}}")?;
    writeln!(
        writer,
        "th,td{{border:1px solid #ccc;padding:8px;text-align:left}}"
    )?;
    writeln!(
        writer,
        ".error{{color:#c00}}.warning{{color:#c80}}.info{{color:#08c}}"
    )?;
    writeln!(writer, "</style></head><body>")?;
    writeln!(writer, "<h1>DcpDoctor Report</h1>")?;
    writeln!(writer, "<p>DCP: {}</p>", dcp_path.display())?;
    writeln!(writer, "<p>Standard: {}</p>", result.standard)?;
    writeln!(
        writer,
        "<p>Result: <strong>{}</strong> ({} errors, {} warnings)</p>",
        if result.ok() { "PASS" } else { "FAIL" },
        result.error_count,
        result.warning_count
    )?;
    writeln!(
        writer,
        "<table><tr><th>Severity</th><th>Code</th><th>Message</th><th>File</th></tr>"
    )?;
    for note in &result.notes {
        let class = match note.severity {
            crate::Severity::Error => "error",
            crate::Severity::Warning => "warning",
            crate::Severity::Info => "info",
        };
        let file = note
            .file
            .as_ref()
            .map(|f| f.display().to_string())
            .unwrap_or_default();
        writeln!(
            writer,
            "<tr><td class='{class}'>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            note.severity,
            note.code,
            escape_markup(&note.message),
            escape_markup(&file)
        )?;
    }
    writeln!(writer, "</table></body></html>")?;
    Ok(())
}

/// Text going into an HTML or SVG element. Findings quote element names, so
/// `has no <Hash> in the CPL` reached the browser as an unknown open tag.
pub(crate) fn escape_markup(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Code, Note, VerifyResult};

    fn sample_result() -> VerifyResult {
        let mut r = VerifyResult::default();
        r.add(Note::error(Code::MissingAssetmap, "no ASSETMAP found"));
        r.add(Note::warning(Code::PklHashMismatch, "hash mismatch"));
        r
    }

    #[test]
    fn test_text_report() {
        let r = sample_result();
        let mut buf = Vec::new();
        write_report(&r, Path::new("/test/dcp"), &mut buf, ReportFormat::Text).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(text.contains("FAIL"));
        assert!(text.contains("1 errors"));
        assert!(text.contains("1 warnings"));
    }

    #[test]
    fn test_json_report() {
        let r = sample_result();
        let mut buf = Vec::new();
        write_report(&r, Path::new("/test/dcp"), &mut buf, ReportFormat::Json).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(json["error_count"], 1);
        assert_eq!(json["warning_count"], 1);
    }

    #[test]
    fn test_html_report() {
        let r = sample_result();
        let mut buf = Vec::new();
        write_report(&r, Path::new("/test/dcp"), &mut buf, ReportFormat::Html).unwrap();
        let html = String::from_utf8(buf).unwrap();
        assert!(html.contains("<html>"));
        assert!(html.contains("FAIL"));
        assert!(html.contains("missing_assetmap"));
    }

    #[test]
    fn an_element_name_in_a_finding_survives_the_html_report() {
        let mut r = VerifyResult::default();
        r.add(Note::warning(
            Code::CplMissingHash,
            "Reel 1 picture asset has no <Hash> in the CPL",
        ));
        let mut buf = Vec::new();
        write_report(&r, Path::new("/test/dcp"), &mut buf, ReportFormat::Html).unwrap();
        let html = String::from_utf8(buf).unwrap();
        assert!(html.contains("no &lt;Hash&gt; in the CPL"), "{html}");
    }

    #[test]
    fn test_pass_report() {
        let r = VerifyResult::default();
        let mut buf = Vec::new();
        write_report(&r, Path::new("/test/dcp"), &mut buf, ReportFormat::Text).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(text.contains("PASS"));
    }
}
