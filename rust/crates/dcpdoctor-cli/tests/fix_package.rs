use assert_cmd::Command;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const INTEROP_CPL_NAMESPACE: &str = "http://www.digicine.com/PROTO-ASDCP-CPL-20040511#";
const SMPTE_CPL_NAMESPACE: &str = "http://www.smpte-ra.org/schemas/429-7/2006/CPL";

fn fixture_package() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/valid_smpte")
}

/// The committed package copied into a temp dir, then broken three ways at
/// once: a stale PKL hash, an Interop namespace on a SMPTE CPL, and a
/// ContentKind that is not one of the SMPTE kinds.
fn broken_package() -> TempDir {
    let package = TempDir::new().unwrap();
    for entry in std::fs::read_dir(fixture_package()).unwrap().flatten() {
        std::fs::copy(entry.path(), package.path().join(entry.file_name())).unwrap();
    }

    let sound = package.path().join("sound.mxf");
    let mut bytes = std::fs::read(&sound).unwrap();
    bytes[40] ^= 0xff;
    std::fs::write(&sound, bytes).unwrap();

    let cpl_path = package.path().join("cpl.xml");
    let cpl = std::fs::read_to_string(&cpl_path).unwrap();
    assert!(cpl.contains(SMPTE_CPL_NAMESPACE) && cpl.contains("<ContentKind>test</ContentKind>"));
    std::fs::write(
        &cpl_path,
        cpl.replace(SMPTE_CPL_NAMESPACE, INTEROP_CPL_NAMESPACE)
            .replace(
                "<ContentKind>test</ContentKind>",
                "<ContentKind>Feature Film</ContentKind>",
            ),
    )
    .unwrap();

    package
}

fn tree_bytes(directory: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(directory)
        .unwrap()
        .flatten()
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                std::fs::read(entry.path()).unwrap(),
            )
        })
        .collect()
}

fn dcpdoctor() -> Command {
    Command::cargo_bin("dcpdoctor").unwrap()
}

#[test]
fn dry_run_reports_the_three_repairs_and_writes_nothing() {
    let package = broken_package();
    let before = tree_bytes(package.path());

    let output = dcpdoctor()
        .args(["fix", package.path().to_str().unwrap(), "--dry-run"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let report = String::from_utf8(output).unwrap();

    assert!(report.contains("Would fix"), "{report}");
    for repair in [
        "pkl_hash_mismatch",
        "smpte_namespace_wrong",
        "cpl_invalid_content_kind",
    ] {
        assert!(
            report.contains(repair),
            "the preview does not name {repair}: {report}"
        );
    }
    assert_eq!(
        tree_bytes(package.path()),
        before,
        "a dry run must leave every file byte for byte as it was"
    );
}

#[test]
fn fix_repairs_the_package_so_validate_passes() {
    let package = broken_package();

    dcpdoctor()
        .args(["validate", package.path().to_str().unwrap()])
        .assert()
        .failure();

    let output = dcpdoctor()
        .args(["fix", package.path().to_str().unwrap()])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let report = String::from_utf8(output).unwrap();
    assert!(report.contains("Fixed"), "{report}");

    let cpl = std::fs::read_to_string(package.path().join("cpl.xml")).unwrap();
    assert!(
        cpl.contains(SMPTE_CPL_NAMESPACE) && !cpl.contains(INTEROP_CPL_NAMESPACE),
        "the CPL namespace was not put back: {cpl}"
    );
    assert!(
        cpl.contains("<ContentKind>feature</ContentKind>"),
        "the ContentKind was not normalized: {cpl}"
    );

    dcpdoctor()
        .args(["validate", package.path().to_str().unwrap()])
        .assert()
        .success();
}
