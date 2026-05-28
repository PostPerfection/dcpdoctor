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
        .stdout(predicate::str::contains("info"));
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
