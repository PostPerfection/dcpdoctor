//! The ISDCF naming rules as `dcpdoctor validate` reports them, on a whole DCP
//! rather than on a title string.

mod common;

use assert_cmd::Command;
use common::{DcpSpec, ISDCF_TITLE, write_dcp};
use tempfile::TempDir;

/// The fields of the convention this test rewrites, by index.
const CONTENT_TYPE: usize = 1;
const LANGUAGE: usize = 3;
const AUDIO: usize = 5;
const RESOLUTION: usize = 6;

fn validate_stdout(content_title: &str) -> String {
    let dir = TempDir::new().unwrap();
    write_dcp(
        dir.path(),
        &DcpSpec {
            content_title: content_title.into(),
            ..Default::default()
        },
    );
    let output = Command::cargo_bin("dcpdoctor")
        .unwrap()
        .args(["validate", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn title_with(field: usize, value: &str) -> String {
    let mut fields: Vec<&str> = ISDCF_TITLE.split('_').collect();
    fields[field] = value;
    fields.join("_")
}

#[test]
fn a_valid_isdcf_title_draws_no_naming_note() {
    let stdout = validate_stdout(ISDCF_TITLE);
    assert!(
        !stdout.contains("isdcf_naming_violation"),
        "{ISDCF_TITLE} follows the convention, got: {stdout}"
    );
}

#[test]
fn a_bad_token_in_any_checked_field_is_named() {
    for (field, value) in [
        (CONTENT_TYPE, "XXX"),
        (LANGUAGE, "english"),
        (AUDIO, "91"),
        (RESOLUTION, "3K"),
    ] {
        let stdout = validate_stdout(&title_with(field, value));
        assert!(
            stdout.contains("isdcf_naming_violation") && stdout.contains(value),
            "field {field} = {value} must draw a naming note that names it, got: {stdout}"
        );
    }
}
