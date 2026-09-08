//! `dcpdoctor validate --netflix` on an IMP: the offline rules Netflix's own
//! validator applies to the package XML.

use std::path::Path;

use assert_cmd::Command;
use tempfile::TempDir;

const APP_2E_2020: &str = "http://www.smpte-ra.org/ns/2067-21/2020";
const IMF_CPL_NAMESPACE: &str = "http://www.smpte-ra.org/schemas/2067-3/2016";
const DCP_CPL_NAMESPACE: &str = "http://www.smpte-ra.org/schemas/429-7/2006/CPL";

fn write_imp(dir: &Path, assetmap_name: &str, cpl_namespace: &str, application: Option<&str>) {
    std::fs::write(
        dir.join(assetmap_name),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:2fd93ab2-dab7-481d-bb43-5779ba62384d</Id>
</AssetMap>"#,
    )
    .unwrap();

    let application = match application {
        Some(id) => format!("<ApplicationIdentification>{id}</ApplicationIdentification>"),
        None => String::new(),
    };
    std::fs::write(
        dir.join("CPL.xml"),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="{cpl_namespace}">
  <Id>urn:uuid:394080ca-5471-40e9-9827-e6e577753400</Id>
  <ContentTitle>Netflix delivery</ContentTitle>
  <EditRate>24 1</EditRate>
  {application}
</CompositionPlaylist>"#
        ),
    )
    .unwrap();
}

fn validate_stdout(dir: &TempDir) -> String {
    let output = Command::cargo_bin("dcpdoctor")
        .unwrap()
        .args(["validate", dir.path().to_str().unwrap(), "--netflix"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn an_app_2e_imp_passes_the_netflix_rules() {
    let dir = TempDir::new().unwrap();
    write_imp(
        dir.path(),
        "ASSETMAP.xml",
        IMF_CPL_NAMESPACE,
        Some(APP_2E_2020),
    );

    let stdout = validate_stdout(&dir);
    assert!(
        stdout.contains("Netflix delivery spec: PASS"),
        "got: {stdout}"
    );
    assert!(
        !stdout.contains("netflix_delivery_violation: Netflix delivery spec: the package"),
        "got: {stdout}"
    );
}

#[test]
fn an_interop_named_assetmap_a_missing_field_and_a_non_app_2e_cpl_each_fail() {
    let dir = TempDir::new().unwrap();
    write_imp(dir.path(), "ASSETMAP", DCP_CPL_NAMESPACE, None);

    let stdout = validate_stdout(&dir);
    for expected in [
        "0 files named ASSETMAP.xml",
        "carries no ApplicationIdentification",
        "CompositionPlaylist in http://www.smpte-ra.org/schemas/429-7/2006/CPL",
    ] {
        assert!(
            stdout.contains(expected),
            "expected a violation naming '{expected}', got: {stdout}"
        );
    }
    // every rule cites where it comes from
    assert!(
        stdout.contains("https://github.com/Netflix/photon/blob/master/"),
        "got: {stdout}"
    );
}
