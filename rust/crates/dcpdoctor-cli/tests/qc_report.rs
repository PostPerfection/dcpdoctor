//! `dcpdoctor qc-report` over a real DCP: the HTML parses, carries the package
//! and track summary, a loudness figure that matches what the loudness command
//! measures on the same file, and a JPEG 2000 forensics table.

mod support;

use std::collections::HashMap;
use std::path::Path;

use assert_cmd::Command;
use scraper::{ElementRef, Html, Selector};
use tempfile::TempDir;

fn cmd() -> Command {
    Command::cargo_bin("dcpdoctor").unwrap()
}

fn package() -> TempDir {
    let dir = TempDir::new().unwrap();
    support::write_package(dir.path(), &support::PackageSpec::default());
    dir
}

fn write_report(dir: &Path, output: &Path) {
    cmd()
        .args(["qc-report"])
        .arg(dir)
        .arg("-o")
        .arg(output)
        .args(["--title", "Fixture Feature", "--client", "Acme Cinemas"])
        .assert()
        .success();
}

/// Every table in the report, keyed by the heading above it. The report writes
/// one table per section and one per picture track, each under its own heading.
fn tables_by_heading(document: &Html) -> HashMap<String, Vec<Vec<String>>> {
    let body = Selector::parse("body").unwrap();
    let row = Selector::parse("tr").unwrap();
    let cell = Selector::parse("th, td").unwrap();

    let mut tables = HashMap::new();
    let mut heading = String::new();
    let Some(body) = document.select(&body).next() else {
        return tables;
    };
    for element in body.children().filter_map(ElementRef::wrap) {
        match element.value().name() {
            "h2" | "h3" => heading = element.text().collect::<String>().trim().to_string(),
            "table" => {
                let rows = element
                    .select(&row)
                    .map(|r| {
                        r.select(&cell)
                            .map(|c| c.text().collect::<String>().trim().to_string())
                            .collect()
                    })
                    .collect();
                tables.insert(heading.clone(), rows);
            }
            _ => {}
        }
    }
    tables
}

fn row_starting_with<'a>(rows: &'a [Vec<String>], first_cell: &str) -> &'a Vec<String> {
    rows.iter()
        .find(|r| r.first().is_some_and(|c| c == first_cell))
        .unwrap_or_else(|| panic!("no row for {first_cell:?} in {rows:?}"))
}

/// Integrated loudness as the `loudness` subcommand reports it for the same
/// file, rendered the way the report's cell renders it.
fn measured_integrated_lufs(sound: &Path) -> String {
    let output = cmd()
        .args(["--json", "loudness"])
        .arg(sound)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let measured: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let lufs = measured["integrated_lufs"].as_f64().unwrap();
    format!("{lufs:.1} LUFS")
}

#[test]
fn the_html_report_carries_the_package_and_track_summary() {
    let dir = package();
    let output = dir.path().join("report.html");
    write_report(dir.path(), &output);

    let html = std::fs::read_to_string(&output).unwrap();
    let document = Html::parse_document(&html);
    assert!(
        document.errors.is_empty(),
        "the report must parse as HTML: {:?}",
        document.errors
    );

    let tables = tables_by_heading(&document);
    let package = &tables["Package Information"];
    assert_eq!(row_starting_with(package, "Title")[1], "Fixture Feature");
    assert_eq!(row_starting_with(package, "Tracks")[1], "5");
    assert!(
        html.contains("Acme Cinemas"),
        "the client name belongs on the report"
    );

    let tracks = &tables["Track Files"];
    // one header row plus one row per file in the package
    assert_eq!(tracks.len(), 6, "{tracks:?}");
    assert_eq!(
        row_starting_with(tracks, "picture")[1..3],
        [
            support::PICTURE_FILE.to_string(),
            support::PICTURE_ID.into()
        ]
    );
    assert_eq!(
        row_starting_with(tracks, "sound")[1..3],
        [support::SOUND_FILE.to_string(), support::SOUND_ID.into()]
    );
    // a sound track under a megabyte used to round to "0 MB"
    assert!(
        row_starting_with(tracks, "sound")[3].ends_with(" KB"),
        "{tracks:?}"
    );
}

#[test]
fn every_sound_track_reports_the_loudness_the_loudness_command_measures() {
    let dir = package();
    let output = dir.path().join("report.html");
    write_report(dir.path(), &output);

    let document = Html::parse_document(&std::fs::read_to_string(&output).unwrap());
    let tables = tables_by_heading(&document);
    let loudness = &tables["Loudness (EBU R128 and Leq(m))"];
    // one header row plus the package's single sound track
    assert_eq!(loudness.len(), 2, "{loudness:?}");

    let row = row_starting_with(loudness, support::SOUND_FILE);
    assert_eq!(
        row[1],
        measured_integrated_lufs(&dir.path().join(support::SOUND_FILE))
    );
    assert!(row[2].ends_with(" dBTP"), "{row:?}");
    assert!(row[4].ends_with(" dB"), "the Leq(m) cell: {row:?}");
}

#[test]
fn the_picture_track_gets_a_codestream_forensics_table() {
    let dir = package();
    let output = dir.path().join("report.html");
    write_report(dir.path(), &output);

    let document = Html::parse_document(&std::fs::read_to_string(&output).unwrap());
    let tables = tables_by_heading(&document);
    let forensics = &tables[support::PICTURE_FILE];

    let resolution = format!("{}x{}", support::PICTURE_WIDTH, support::PICTURE_HEIGHT);
    assert_eq!(row_starting_with(forensics, "Resolution")[1], resolution);
    assert_eq!(
        row_starting_with(forensics, "Frames scanned")[1],
        support::FRAMES.to_string()
    );
    assert_eq!(
        row_starting_with(forensics, "Parameters constant")[1],
        "yes"
    );
    for parameter in [
        "Components",
        "Decomposition levels",
        "Code-block size",
        "Wavelet transform",
        "Quality layers",
        "Progression order",
        "Tiles",
        "Tile-parts",
        "Multiple component transform",
        "TLM marker",
        "POC marker",
        "Worst frame",
    ] {
        assert!(
            !row_starting_with(forensics, parameter)[1].is_empty(),
            "{parameter} has no value: {forensics:?}"
        );
    }
}

/// The conversion shells out to weasyprint or wkhtmltopdf. Both need a GTK
/// runtime the Windows runner has no clean install for, so the assertion is
/// compiled out there rather than skipped at run time.
#[test]
#[cfg(not(target_os = "windows"))]
fn a_pdf_report_is_a_pdf() {
    let dir = package();
    let output = dir.path().join("report.pdf");
    write_report(dir.path(), &output);

    let pdf = std::fs::read(&output).unwrap();
    assert!(
        pdf.starts_with(b"%PDF-"),
        "conversion produced {} bytes that are no PDF",
        pdf.len()
    );
    assert!(pdf.len() > b"%PDF-".len());
    assert!(
        !output.with_extension("tmp.html").exists(),
        "the intermediate HTML must be cleaned up"
    );
}
