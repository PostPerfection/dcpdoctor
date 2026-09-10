mod support;

use assert_cmd::Command;
use tempfile::TempDir;

const NEUTRAL_RGB_CODES: [u16; 3] = [2048, 2048, 2048];
const PKL_CPL_AND_PICTURE_ASSETS: usize = 3;

fn field<'a>(stdout: &'a str, label: &str) -> &'a str {
    stdout
        .lines()
        .find_map(|line| line.trim().strip_prefix(label))
        .unwrap_or_else(|| panic!("no {label} line in {stdout}"))
        .trim()
}

#[test]
fn imp_info_counts_the_assets_cpls_and_pkls_of_an_app_2e_imp() {
    let root = TempDir::new().unwrap();
    let imp = root.path().join("imp");
    std::fs::create_dir_all(&imp).unwrap();
    support::write_app2e_imp(&imp, support::Picture::ImfSolid(NEUTRAL_RGB_CODES));

    let output = Command::cargo_bin("dcpdoctor")
        .unwrap()
        .arg("imp-info")
        .arg(&imp)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "imp-info failed on {}",
        imp.display()
    );
    let stdout = String::from_utf8(output.stdout).unwrap();

    assert!(
        stdout.contains(&format!("IMP Info: {}", imp.display())),
        "the report must name the package it read: {stdout}"
    );
    assert_eq!(field(&stdout, "Standard:"), "SMPTE");
    assert_eq!(
        field(&stdout, "Assets:"),
        PKL_CPL_AND_PICTURE_ASSETS.to_string()
    );
    assert_eq!(field(&stdout, "CPLs:"), "1");
    assert_eq!(field(&stdout, "PKLs:"), "1");
}
