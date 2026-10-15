use std::path::PathBuf;

use assert_cmd::Command;

const ERROR_LINE_PREFIX: &str = "[ERROR]";

fn fixture_package() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/valid_smpte")
}

#[test]
fn the_valid_smpte_fixture_passes_a_full_validate() {
    let output = Command::cargo_bin("dcpdoctor")
        .unwrap()
        .arg("validate")
        .arg(fixture_package())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(stdout.contains("Result: PASS"), "{stdout}");
    assert!(
        !stdout
            .lines()
            .any(|line| line.starts_with(ERROR_LINE_PREFIX)),
        "{stdout}"
    );
}

const CPL_ID: &str = "56807594-d778-47ca-860a-24191018db3b";
const PKL_ID: &str = "f7c8f048-bcd4-4c53-b630-8b151630c086";

#[test]
fn info_lists_the_cpls_and_pkls_and_the_json_lists_the_assetmap() {
    let output = Command::cargo_bin("dcpdoctor")
        .unwrap()
        .arg("info")
        .arg(fixture_package())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(
        stdout.contains(&format!("CPL: urn:uuid:{CPL_ID} Test DCP (clear)\n")),
        "{stdout}"
    );
    assert!(
        stdout.contains(&format!("PKL: urn:uuid:{PKL_ID}\n")),
        "{stdout}"
    );
    assert!(!stdout.contains("picture.mxf"), "{stdout}");

    let output = Command::cargo_bin("dcpdoctor")
        .unwrap()
        .args(["--json", "info"])
        .arg(fixture_package())
        .output()
        .unwrap();
    let info: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        info["cpls"],
        serde_json::json!([{ "id": CPL_ID, "title": "Test DCP", "encrypted": false }])
    );
    assert_eq!(info["pkl_ids"], serde_json::json!([PKL_ID]));
    let paths: Vec<&str> = info["assetmap_entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["pkl.xml", "cpl.xml", "picture.mxf", "sound.mxf"]);
}
