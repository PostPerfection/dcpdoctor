mod support;

use assert_cmd::Command;
use tempfile::TempDir;

// asdcplib logs to stderr whenever a reader opens a track file of another kind
fn validate_stderr(flags: &[&str]) -> String {
    let dir = TempDir::new().unwrap();
    support::write_package(dir.path(), &support::PackageSpec::default());

    let output = Command::cargo_bin("dcpdoctor")
        .unwrap()
        .arg("validate")
        .args(flags)
        .arg(dir.path())
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn validating_a_package_logs_nothing_to_stderr() {
    let stderr = validate_stderr(&[]);
    assert!(stderr.is_empty(), "{stderr}");
}

#[test]
fn the_hdr_checks_log_nothing_to_stderr() {
    let stderr = validate_stderr(&["--hdr"]);
    assert!(stderr.is_empty(), "{stderr}");
}
