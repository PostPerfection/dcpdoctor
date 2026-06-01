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
fn imf_compliance_help_lists_target_flag() {
    cmd()
        .args(["imf-compliance", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--target"));
}
