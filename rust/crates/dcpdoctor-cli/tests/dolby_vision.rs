//! `dcpdoctor --dolby-vision` on a JPEG 2000 DCP, whose track files carry no
//! RPU, so the report says that instead of staying silent.

mod support;

use assert_cmd::Command;
use tempfile::TempDir;

#[test]
fn a_jpeg_2000_package_is_told_it_has_no_dolby_vision_metadata() {
    let root = TempDir::new().unwrap();
    let dcp = root.path().join("dcp");
    std::fs::create_dir_all(&dcp).unwrap();
    support::write_package(&dcp, &support::PackageSpec::default());

    let stdout = Command::cargo_bin("dcpdoctor")
        .unwrap()
        .args(["--json", "--dolby-vision"])
        .arg(&dcp)
        .output()
        .unwrap()
        .stdout;
    let report: serde_json::Value = serde_json::from_slice(&stdout).unwrap();

    let note = report["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|note| {
            note["message"]
                .as_str()
                .unwrap()
                .contains("no Dolby Vision RPU")
        })
        .unwrap_or_else(|| panic!("no Dolby Vision note in {report:#}"));

    assert_eq!(note["severity"], "info");
    assert_eq!(note["code"], "HdrMetadataSummary");
    assert_eq!(
        note["message"],
        "JPEG 2000 track files carry no Dolby Vision RPU, so this package has no Dolby Vision metadata to check"
    );
    assert_eq!(note["file"], dcp.to_str().unwrap());
}
