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
