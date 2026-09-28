mod support;

use std::path::Path;

use assert_cmd::Command;
use tempfile::TempDir;

const VOLINDEX_FILE: &str = "VOLINDEX.xml";
const VOLINDEX_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<VolumeIndex xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM"><Index>1</Index></VolumeIndex>"#;

const PKL_PRESENT: &str = "DCI-STRUCT-3";
const HASHES_MATCH: &str = "DCI-STRUCT-6";

const GROUPS_AND_TEST_IDS: [(&str, &str); 5] = [
    ("structure_tests", HASHES_MATCH),
    ("cpl_tests", "DCI-CPL-1"),
    ("picture_tests", "DCI-PIC-1"),
    ("audio_tests", "DCI-AUD-1"),
    ("security_tests", "DCI-SEC-1"),
];

fn clean_package() -> TempDir {
    let dir = TempDir::new().unwrap();
    support::write_package(dir.path(), &support::PackageSpec::default());
    std::fs::write(dir.path().join(VOLINDEX_FILE), VOLINDEX_XML).unwrap();
    dir
}

struct Conformance {
    succeeded: bool,
    report: serde_json::Value,
}

fn conformance(dir: &Path) -> Conformance {
    let output = Command::cargo_bin("dcpdoctor")
        .unwrap()
        .args(["--json", "conformance"])
        .arg(dir)
        .output()
        .unwrap();
    Conformance {
        succeeded: output.status.success(),
        report: serde_json::from_slice(&output.stdout).unwrap(),
    }
}

fn test_ids(report: &serde_json::Value, group: &str) -> Vec<String> {
    report[group]
        .as_array()
        .unwrap()
        .iter()
        .map(|test| test["test_id"].as_str().unwrap().to_string())
        .collect()
}

fn failed_test_ids(report: &serde_json::Value) -> Vec<String> {
    GROUPS_AND_TEST_IDS
        .iter()
        .flat_map(|(group, _)| report[*group].as_array().unwrap())
        .filter(|test| test["passed"] == false)
        .map(|test| test["test_id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_clean_package_passes_every_test_group() {
    let dir = clean_package();
    let result = conformance(dir.path());

    assert!(result.succeeded, "{}", result.report);
    assert_eq!(result.report["conformant"], true, "{}", result.report);
    assert_eq!(result.report["tests_failed"], 0, "{}", result.report);
    for (group, test_id) in GROUPS_AND_TEST_IDS {
        assert!(
            test_ids(&result.report, group)
                .iter()
                .any(|id| id == test_id),
            "{group} has no {test_id}: {}",
            result.report
        );
    }
}

#[test]
fn a_corrupted_picture_track_fails_the_hash_test_alone() {
    let dir = clean_package();
    support::corrupt_one_byte(&dir.path().join(support::PICTURE_FILE));
    let result = conformance(dir.path());

    assert!(!result.succeeded, "{}", result.report);
    assert_eq!(result.report["conformant"], false);
    assert_eq!(failed_test_ids(&result.report), [HASHES_MATCH]);
}

#[test]
fn an_assetmap_does_not_stand_in_for_a_missing_packing_list() {
    let dir = clean_package();
    std::fs::remove_file(dir.path().join(support::PKL_FILE)).unwrap();
    let result = conformance(dir.path());

    assert!(!result.succeeded, "{}", result.report);
    assert!(
        failed_test_ids(&result.report)
            .iter()
            .any(|id| id == PKL_PRESENT),
        "{}",
        result.report
    );
}
