//! `validate --manifest` against a reference manifest JSON.

mod support;

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use tempfile::TempDir;

struct Package {
    _root: TempDir,
    dcp: PathBuf,
}

fn package() -> Package {
    let root = TempDir::new().unwrap();
    let dcp = root.path().join("dcp");
    std::fs::create_dir_all(&dcp).unwrap();
    support::write_package(&dcp, &support::PackageSpec::default());
    Package { _root: root, dcp }
}

fn size_of(dcp: &Path, file: &str) -> u64 {
    std::fs::metadata(dcp.join(file)).unwrap().len()
}

fn write_manifest(dcp: &Path, assets: serde_json::Value) -> PathBuf {
    let path = dcp.parent().unwrap().join("manifest.json");
    std::fs::write(&path, serde_json::json!({ "assets": assets }).to_string()).unwrap();
    path
}

fn validate_against(dcp: &Path, manifest: &Path) -> serde_json::Value {
    let stdout = Command::cargo_bin("dcpdoctor")
        .unwrap()
        .args(["--json", "validate", "--manifest"])
        .arg(manifest)
        .arg(dcp)
        .output()
        .unwrap()
        .stdout;
    serde_json::from_slice(&stdout).unwrap()
}

fn manifest_notes(report: &serde_json::Value) -> Vec<&serde_json::Value> {
    report["notes"]
        .as_array()
        .unwrap_or_else(|| panic!("no notes in {report:#}"))
        .iter()
        .filter(|note| {
            matches!(
                note["code"].as_str(),
                Some("AssetNotFound" | "ManifestSizeMismatch")
            )
        })
        .collect()
}

#[test]
fn a_manifest_matching_the_package_raises_nothing() {
    let package = package();
    let manifest = write_manifest(
        &package.dcp,
        serde_json::json!([
            { "filename": support::PICTURE_FILE, "size": size_of(&package.dcp, support::PICTURE_FILE) },
            { "filename": support::SOUND_FILE, "size": size_of(&package.dcp, support::SOUND_FILE) },
        ]),
    );

    let report = validate_against(&package.dcp, &manifest);

    assert!(
        manifest_notes(&report).is_empty(),
        "a manifest that matches must raise nothing: {report:#}"
    );
}

#[test]
fn a_wrong_size_and_a_missing_asset_each_fail_naming_the_asset() {
    let package = package();
    let manifest = write_manifest(
        &package.dcp,
        serde_json::json!([
            { "filename": support::PICTURE_FILE, "size": size_of(&package.dcp, support::PICTURE_FILE) + 1 },
            { "filename": "reel_2_sound.mxf", "size": 4096 },
        ]),
    );

    let report = validate_against(&package.dcp, &manifest);

    let mismatch = manifest_notes(&report)
        .into_iter()
        .find(|note| note["code"] == "ManifestSizeMismatch")
        .unwrap_or_else(|| panic!("no size mismatch in {report:#}"));
    assert_eq!(mismatch["severity"], "error");
    assert!(
        mismatch["message"]
            .as_str()
            .unwrap()
            .contains(support::PICTURE_FILE),
        "the mismatch must name the asset: {mismatch:#}"
    );

    let missing = manifest_notes(&report)
        .into_iter()
        .find(|note| note["code"] == "AssetNotFound")
        .unwrap_or_else(|| panic!("no missing-asset note in {report:#}"));
    assert_eq!(missing["severity"], "error");
    assert!(
        missing["message"]
            .as_str()
            .unwrap()
            .contains("reel_2_sound.mxf"),
        "the missing note must name the asset: {missing:#}"
    );

    assert!(
        report["error_count"].as_u64().unwrap() >= 2,
        "both failures must count as errors: {report:#}"
    );
}

#[test]
fn a_manifest_with_no_assets_array_says_nothing_was_compared() {
    let package = package();
    let manifest = package.dcp.parent().unwrap().join("manifest.json");
    std::fs::write(&manifest, r#"{"files":[]}"#).unwrap();

    let report = validate_against(&package.dcp, &manifest);

    let note = report["notes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|note| {
            note["message"]
                .as_str()
                .unwrap()
                .contains("nothing in the package was compared")
        })
        .unwrap_or_else(|| panic!("a manifest naming no assets must say so: {report:#}"));
    assert_eq!(note["severity"], "error");
}
