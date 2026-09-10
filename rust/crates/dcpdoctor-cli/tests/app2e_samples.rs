mod support;

use std::path::Path;

use assert_cmd::Command;
use tempfile::TempDir;

const XYZ_WHITE_CODES: [u16; 3] = [3883, 3960, 4092];
const RGB_GREY_CODES: [u16; 3] = [2048, 2048, 2048];
const BLACK_CODES: [u16; 3] = [0, 0, 0];

const SAMPLES_CODE: &str = "PictureSamplesNotRgb";
const SKIPPED_CODE: &str = "CheckSkipped";
const SKIPPED_SAMPLES_MESSAGE: &str = "picture sample decoding";

fn imp_notes(codes: [u16; 3]) -> Vec<serde_json::Value> {
    let root = TempDir::new().unwrap();
    let imp = root.path().join("imp");
    std::fs::create_dir_all(&imp).unwrap();
    support::write_app2e_imp(&imp, support::Picture::ImfSolid(codes));

    let stdout = Command::cargo_bin("dcpdoctor")
        .unwrap()
        .args(["--json", "validate", "--imf"])
        .arg(&imp)
        .output()
        .unwrap()
        .stdout;
    let report: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
    report["notes"]
        .as_array()
        .unwrap_or_else(|| panic!("no notes in {report:#}"))
        .clone()
}

fn find<'a>(notes: &'a [serde_json::Value], code: &str) -> Option<&'a serde_json::Value> {
    notes.iter().find(|note| note["code"] == code)
}

fn skipped_samples_note(notes: &[serde_json::Value]) -> Option<&serde_json::Value> {
    notes.iter().find(|note| {
        note["code"] == SKIPPED_CODE
            && note["message"]
                .as_str()
                .is_some_and(|message| message.contains(SKIPPED_SAMPLES_MESSAGE))
    })
}

#[test]
fn a_track_whose_samples_are_xyz_white_fails_though_its_header_says_imf() {
    let notes = imp_notes(XYZ_WHITE_CODES);

    let note = find(&notes, SAMPLES_CODE)
        .unwrap_or_else(|| panic!("no X'Y'Z' sample finding in {notes:#?}"));
    assert_eq!(note["severity"], "error");
    let message = note["message"].as_str().unwrap();
    assert!(
        message.contains("X'Y'Z' white ratio"),
        "the message must say what the pixels sit at: {message}"
    );
    assert!(
        message.contains(&format!("{} decoded frames", support::IMP_FRAMES)),
        "the message must say how many frames it decoded: {message}"
    );
    assert!(
        Path::new(note["file"].as_str().unwrap()).ends_with(support::imp_picture_file()),
        "the finding must name the track file: {note:#}"
    );
}

#[test]
fn a_track_of_neutral_rgb_passes_with_nothing_skipped() {
    let notes = imp_notes(RGB_GREY_CODES);

    assert!(
        find(&notes, SAMPLES_CODE).is_none(),
        "grey RGB samples must not read as X'Y'Z': {notes:#?}"
    );
    assert!(
        skipped_samples_note(&notes).is_none(),
        "grey RGB samples decide the check: {notes:#?}"
    );
}

#[test]
fn a_black_track_says_the_samples_decided_nothing() {
    let notes = imp_notes(BLACK_CODES);

    assert!(
        find(&notes, SAMPLES_CODE).is_none(),
        "black says nothing about the encoding: {notes:#?}"
    );
    let note = skipped_samples_note(&notes)
        .unwrap_or_else(|| panic!("black must report an undecided check: {notes:#?}"));
    assert_eq!(note["severity"], "warning");
    assert!(
        note["message"]
            .as_str()
            .unwrap()
            .contains("decided nothing"),
        "{note:#}"
    );
}
