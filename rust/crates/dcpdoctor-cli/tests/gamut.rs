//! `--studio --deep` decodes a spread of frames and reports how much of the
//! picture falls outside DCI-P3.

mod support;

use std::path::Path;

use assert_cmd::Command;
use tempfile::TempDir;

/// X', Y', Z' for linear DCI-P3 (0.2, 0.2, 0.2), a grey well inside the gamut.
const INSIDE_P3_CODES: [u16; 3] = [2113, 2205, 2166];
/// A strong X' with no Y' or Z' at all, which matrixes to a negative G and a
/// red past full scale.
const OUTSIDE_P3_CODES: [u16; 3] = [3500, 0, 0];

const GAMUT_CODE: &str = "PictureOutOfGamut";

fn studio_notes(picture: support::Picture) -> Vec<serde_json::Value> {
    let root = TempDir::new().unwrap();
    let dcp = root.path().join("dcp");
    std::fs::create_dir_all(&dcp).unwrap();
    support::write_package(
        &dcp,
        &support::PackageSpec {
            picture,
            ..support::PackageSpec::default()
        },
    );

    let stdout = Command::cargo_bin("dcpdoctor")
        .unwrap()
        .args(["--json", "--studio", "--deep"])
        .arg(&dcp)
        .output()
        .unwrap()
        .stdout;
    let report: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
    report["notes"]
        .as_array()
        .unwrap_or_else(|| panic!("no notes in {report:#}"))
        .iter()
        .filter(|note| note["code"] == GAMUT_CODE)
        .cloned()
        .collect()
}

fn summary(notes: &[serde_json::Value]) -> &str {
    notes
        .iter()
        .find(|note| note["severity"] == "info")
        .unwrap_or_else(|| panic!("no out-of-gamut summary in {notes:#?}"))["message"]
        .as_str()
        .unwrap()
}

#[test]
fn a_picture_inside_p3_reports_no_out_of_gamut_samples() {
    let notes = studio_notes(support::Picture::Xyz(INSIDE_P3_CODES));

    let summary = summary(&notes);
    assert!(
        summary.starts_with("out-of-gamut sampling: 0.000% of "),
        "an in-gamut grey must sample clean: {summary}"
    );
    assert!(
        summary.contains("over 12 frames"),
        "the summary must say how many frames it decoded: {summary}"
    );
    assert!(
        !notes.iter().any(|note| note["severity"] == "warning"),
        "an in-gamut picture must not warn: {notes:#?}"
    );
}

#[test]
fn a_picture_outside_p3_reports_its_share_and_warns() {
    let notes = studio_notes(support::Picture::Xyz(OUTSIDE_P3_CODES));

    let summary = summary(&notes);
    assert!(
        summary.starts_with("out-of-gamut sampling: 100.000% of "),
        "every sample of this picture is outside P3: {summary}"
    );

    let warning = notes
        .iter()
        .find(|note| note["severity"] == "warning")
        .unwrap_or_else(|| panic!("no out-of-gamut warning in {notes:#?}"));
    assert_eq!(
        warning["message"],
        "100.000% of the sampled picture is outside DCI-P3, over the 1.000% this warns at"
    );
    assert!(
        Path::new(warning["file"].as_str().unwrap()).ends_with(support::PICTURE_FILE),
        "the warning must name the track file: {warning:#}"
    );
}
