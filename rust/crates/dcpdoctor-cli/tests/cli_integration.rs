use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn cmd() -> Command {
    Command::cargo_bin("dcpdoctor").unwrap()
}

#[test]
fn version_flag() {
    cmd()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("dcpdoctor"));
}

#[test]
fn help_flag() {
    cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"))
        .stdout(predicate::str::contains("validate"))
        .stdout(predicate::str::contains("diff"))
        .stdout(predicate::str::contains("info"))
        .stdout(predicate::str::contains("kdm"))
        .stdout(predicate::str::contains("mxf-extract"))
        .stdout(predicate::str::contains("qc-report"))
        .stdout(predicate::str::contains("imp-info"));
}

#[test]
fn validate_missing_directory() {
    cmd()
        .args(["validate", "/nonexistent/path"])
        .assert()
        .failure();
}

#[test]
fn validate_empty_directory() {
    let dir = TempDir::new().unwrap();
    cmd()
        .args(["validate", dir.path().to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn validate_with_json_output() {
    let dir = TempDir::new().unwrap();
    cmd()
        .args(["validate", dir.path().to_str().unwrap(), "--json"])
        .assert()
        .failure();
}

#[test]
fn validate_help() {
    cmd()
        .args(["validate", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--no-hashes"))
        .stdout(predicate::str::contains("--no-signatures"))
        .stdout(predicate::str::contains("--check-mxf"))
        .stdout(predicate::str::contains("--strict"));
}

#[test]
fn info_missing_directory() {
    cmd().args(["info", "/nonexistent/path"]).assert().failure();
}

#[test]
fn shorthand_positional_arg() {
    let dir = TempDir::new().unwrap();
    // Positional arg without subcommand should act like `validate`
    cmd().arg(dir.path().to_str().unwrap()).assert().failure();
}

#[test]
fn rust_command_surface_help_smoke() {
    let subcommands = [
        "watch",
        "serve",
        "fix",
        "kdm",
        "profiles",
        "checksum-verify",
        "loudness",
        "frame-qc",
        "auto-qc",
        "imf-compliance",
        "mxf-extract",
        "schema-validate",
        "qc-report",
        "av-sync",
        "hdr-validate",
        "frame-compare",
        "imp-info",
    ];

    for subcommand in subcommands {
        cmd().args([subcommand, "--help"]).assert().success();
    }
}

#[test]
fn kdm_missing_file_fails() {
    cmd()
        .args(["kdm", "/nonexistent/file.kdm.xml"])
        .assert()
        .failure();
}

#[test]
fn checksum_verify_missing_directory_fails() {
    cmd()
        .args(["checksum-verify", "/nonexistent/path"])
        .assert()
        .failure();
}

#[test]
fn imf_compliance_rejects_unknown_target() {
    let dir = TempDir::new().unwrap();
    cmd()
        .args([
            "imf-compliance",
            dir.path().to_str().unwrap(),
            "--target",
            "bogus",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown target"));
}

#[test]
fn validate_accepts_imf_flag() {
    // --imf is a real global flag now; a plain path with --imf must parse (not exit 2)
    let dir = TempDir::new().unwrap();
    cmd()
        .args([dir.path().to_str().unwrap(), "--imf"])
        .assert()
        .code(1);
}

#[test]
fn diff_help_lists_fingerprint_flag() {
    cmd()
        .args(["diff", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--fingerprint"));
}

#[test]
fn imf_compliance_help_lists_target_flag() {
    cmd()
        .args(["imf-compliance", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--target"));
}

#[test]
fn schema_validate_rejects_malformed_package_xml_without_schemas() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("CPL_broken.xml"), "<CompositionPlaylist>").unwrap();

    cmd()
        .args(["schema-validate", dir.path().to_str().unwrap()])
        .assert()
        .failure()
        .stdout(predicate::str::contains("XML parse error"));
}

#[test]
fn schema_validate_uses_supplied_xsd_for_each_package_xml() {
    if std::process::Command::new("xmllint")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }

    let package_dir = TempDir::new().unwrap();
    let schema_dir = TempDir::new().unwrap();
    std::fs::write(
        package_dir.path().join("CPL_test.xml"),
        r#"<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL"><Id>test</Id></CompositionPlaylist>"#,
    )
    .unwrap();
    std::fs::write(
        schema_dir.path().join("SMPTE-429-7-2006-CPL.xsd"),
        r#"<?xml version="1.0"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
           targetNamespace="http://www.smpte-ra.org/schemas/429-7/2006/CPL"
           xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL"
           elementFormDefault="qualified">
  <xs:element name="CompositionPlaylist">
    <xs:complexType>
      <xs:sequence>
        <xs:element name="Required" type="xs:string"/>
      </xs:sequence>
    </xs:complexType>
  </xs:element>
</xs:schema>"#,
    )
    .unwrap();

    cmd()
        .args([
            "schema-validate",
            package_dir.path().to_str().unwrap(),
            "--schema-dir",
            schema_dir.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains("Required"));
}
