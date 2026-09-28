mod support;

use assert_cmd::Command;
use tempfile::TempDir;

const TRUE_PEAK_CODE: &str = "SoundTruePeakExceeded";
const SILENCE_AMPLITUDE: f64 = 0.0;
// -0.09 dBFS, a tone whose true peak sits above the -1 dBTP limit
const HOT_TONE_AMPLITUDE: f64 = 0.99;

fn sound_notes(sound_amplitude: f64) -> Vec<serde_json::Value> {
    let root = TempDir::new().unwrap();
    let dcp = root.path().join("dcp");
    std::fs::create_dir_all(&dcp).unwrap();
    support::write_package(
        &dcp,
        &support::PackageSpec {
            sound_amplitude,
            ..support::PackageSpec::default()
        },
    );

    let stdout = Command::cargo_bin("dcpdoctor")
        .unwrap()
        .args(["--json", "validate"])
        .arg(&dcp)
        .output()
        .unwrap()
        .stdout;
    let report: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
    let notes = report["notes"]
        .as_array()
        .unwrap_or_else(|| panic!("no notes in {report:#}"));
    let loudness_skipped = notes.iter().any(|note| {
        note["code"] == "CheckSkipped"
            && note["message"]
                .as_str()
                .is_some_and(|message| message.starts_with("loudness check did not run"))
    });
    assert!(!loudness_skipped, "the loudness check must run: {notes:#?}");
    notes
        .iter()
        .filter(|note| {
            note["file"]
                .as_str()
                .is_some_and(|file| file.ends_with(support::SOUND_FILE))
        })
        .cloned()
        .collect()
}

#[test]
fn a_silent_sound_track_raises_no_true_peak_error() {
    let notes = sound_notes(SILENCE_AMPLITUDE);

    let true_peak_notes: Vec<_> = notes
        .iter()
        .filter(|note| {
            note["code"] == TRUE_PEAK_CODE
                || note["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("True peak"))
        })
        .collect();
    assert!(true_peak_notes.is_empty(), "got: {true_peak_notes:#?}");
}

#[test]
fn a_tone_near_full_scale_raises_a_true_peak_error() {
    let notes = sound_notes(HOT_TONE_AMPLITUDE);

    assert!(
        notes
            .iter()
            .any(|note| note["code"] == TRUE_PEAK_CODE && note["severity"] == "error"),
        "got: {notes:#?}"
    );
}
