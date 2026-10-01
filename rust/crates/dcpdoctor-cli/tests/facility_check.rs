//! `dcpdoctor facility-check` on a package built to be ingested, and on one
//! carrying the three defects a facility turns a delivery away for.

mod common;

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use common::{DcpSpec, write_dcp};
use predicates::prelude::*;
use tempfile::TempDir;

/// The ISDCF/DTB Bv2.1 reference CPL committed under tests/fixtures/signature.
/// It is validly signed, and every certificate in its chain below the root
/// expired between 2023 and 2025, which is the expired-signer case.
const SIGNED_CPL: &str =
    "CPL_SMPTE_TST-1-Bv21_S_EN-EN-CCAP_US_51-HI-VI_2K_ISDCF_20170110_DTB_SMPTE_OV.xml";

fn cmd() -> Command {
    Command::cargo_bin("dcpdoctor").unwrap()
}

fn signed_cpl_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/signature")
        .join(SIGNED_CPL)
}

fn clean_package() -> TempDir {
    let dir = TempDir::new().unwrap();
    write_dcp(dir.path(), &DcpSpec::default());
    dir
}

fn facility_check(dir: &TempDir) -> assert_cmd::assert::Assert {
    cmd()
        .args(["facility-check", dir.path().to_str().unwrap()])
        .assert()
}

#[test]
fn a_clean_package_is_ready_for_delivery() {
    facility_check(&clean_package())
        .success()
        .stdout(predicate::str::contains("Ready for delivery: YES"))
        .stdout(predicate::str::contains("[error]").not());
}

#[test]
fn a_missing_asset_a_bad_hash_and_an_expired_certificate_each_fail_by_name() {
    let dir = clean_package();

    std::fs::remove_file(dir.path().join(common::SOUND_FILE)).unwrap();

    // one byte flipped in place: the size still matches, the hash does not
    let picture = dir.path().join(common::PICTURE_FILE);
    let mut bytes = std::fs::read(&picture).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xFF;
    std::fs::write(&picture, &bytes).unwrap();

    std::fs::copy(signed_cpl_fixture(), dir.path().join("CPL_signed.xml")).unwrap();

    facility_check(&dir)
        .failure()
        .stdout(predicate::str::contains("Ready for delivery: NO"))
        .stdout(predicate::str::contains(format!(
            "{} is missing",
            common::SOUND_FILE
        )))
        .stdout(predicate::str::contains(format!(
            "{} hash mismatch",
            common::PICTURE_FILE
        )))
        .stdout(predicate::str::contains("Signing certificates"))
        .stdout(predicate::str::contains("outside its validity period"));
}

/// An ASSETMAP carries a `<PackingList>` element of its own, so a search for
/// that string finds one where the PKL is gone.
#[test]
fn a_package_with_no_packing_list_fails_the_pkl_check() {
    let dir = clean_package();
    std::fs::remove_file(dir.path().join(common::PKL_FILE)).unwrap();

    facility_check(&dir)
        .failure()
        .stdout(predicate::str::contains("PKL present"))
        .stdout(predicate::str::contains("Ready for delivery: NO"));
}

#[test]
fn a_package_with_no_volindex_is_not_ready() {
    let dir = clean_package();
    std::fs::remove_file(dir.path().join("VOLINDEX.xml")).unwrap();

    facility_check(&dir)
        .failure()
        .stdout(predicate::str::contains("VOLINDEX present"));
}

const CHAIN: &str = "<meta:Chain>ALL</meta:Chain>";
const DISTRIBUTOR: &str = "<meta:Distributor>isdcf</meta:Distributor>";
const FACILITY: &str = "<meta:Facility>dtb</meta:Facility>";
const LUMINANCE: &str = r#"<meta:Luminance units="foot-lambert">14</meta:Luminance>"#;

fn package_with_metadata(elements: &[&str]) -> TempDir {
    let dir = TempDir::new().unwrap();
    let composition_metadata_asset = format!(
        r#"
        <meta:CompositionMetadataAsset xmlns:meta="http://www.smpte-ra.org/schemas/429-16/2014/CPL-Metadata">
          <Id>urn:uuid:5532b438-0906-4ff2-953d-994216c66ed4</Id>
          <EditRate>24 1</EditRate>
          <IntrinsicDuration>2</IntrinsicDuration>
          <meta:VersionNumber status="final">1</meta:VersionNumber>
          {}
        </meta:CompositionMetadataAsset>"#,
        elements.join("\n          ")
    );
    write_dcp(
        dir.path(),
        &DcpSpec {
            composition_metadata_asset: Some(composition_metadata_asset),
            ..Default::default()
        },
    );
    dir
}

fn metadata_items(dir: &TempDir) -> Vec<serde_json::Value> {
    let output = cmd()
        .args(["--json", "facility-check", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    result["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["category"] == "metadata")
        .cloned()
        .collect()
}

fn failed(items: &[serde_json::Value]) -> Vec<&serde_json::Value> {
    items
        .iter()
        .filter(|item| item["passed"] == false)
        .collect()
}

#[test]
fn a_cpl_carrying_all_four_facility_metadata_elements_passes_each() {
    let items = metadata_items(&package_with_metadata(&[
        CHAIN,
        DISTRIBUTOR,
        FACILITY,
        LUMINANCE,
    ]));

    assert_eq!(items.len(), 4, "{items:#?}");
    assert!(failed(&items).is_empty(), "{items:#?}");
}

#[test]
fn each_missing_facility_metadata_element_is_a_warning_naming_it() {
    let all = [
        ("Chain", CHAIN),
        ("Distributor", DISTRIBUTOR),
        ("Facility", FACILITY),
        ("Luminance", LUMINANCE),
    ];
    for (missing_name, _) in all {
        let present: Vec<&str> = all
            .iter()
            .filter(|(name, _)| *name != missing_name)
            .map(|(_, element)| *element)
            .collect();
        let items = metadata_items(&package_with_metadata(&present));
        let failures = failed(&items);

        assert_eq!(failures.len(), 1, "{missing_name}: {items:#?}");
        let finding = failures[0];
        assert_eq!(
            finding["check_name"],
            format!("CompositionMetadataAsset {missing_name}")
        );
        assert_eq!(finding["severity"], "warning");
        let detail = finding["detail"].as_str().unwrap();
        assert!(detail.contains(&format!("<{missing_name}>")), "{detail}");
        assert!(detail.contains("QC requires it"), "{detail}");
    }
}

#[test]
fn a_malformed_luminance_is_an_error() {
    for luminance in [
        r#"<meta:Luminance units="nits">14</meta:Luminance>"#,
        "<meta:Luminance>14</meta:Luminance>",
        r#"<meta:Luminance units="foot-lambert">0</meta:Luminance>"#,
        r#"<meta:Luminance units="candela-per-square-metre">bright</meta:Luminance>"#,
    ] {
        let dir = package_with_metadata(&[CHAIN, DISTRIBUTOR, FACILITY, luminance]);
        let items = metadata_items(&dir);
        let failures = failed(&items);

        assert_eq!(failures.len(), 1, "{luminance}: {items:#?}");
        assert_eq!(
            failures[0]["check_name"],
            "CompositionMetadataAsset Luminance"
        );
        assert_eq!(failures[0]["severity"], "error", "{luminance}");
        facility_check(&dir)
            .failure()
            .stdout(predicate::str::contains("Ready for delivery: NO"));
    }
}

#[test]
fn an_empty_chain_is_an_error() {
    let items = metadata_items(&package_with_metadata(&[
        "<meta:Chain/>",
        DISTRIBUTOR,
        FACILITY,
        LUMINANCE,
    ]));
    let failures = failed(&items);

    assert_eq!(failures.len(), 1, "{items:#?}");
    assert_eq!(failures[0]["check_name"], "CompositionMetadataAsset Chain");
    assert_eq!(failures[0]["severity"], "error");
}

#[test]
fn an_interop_cpl_gets_no_composition_metadata_findings() {
    let dir = package_with_metadata(&[CHAIN, DISTRIBUTOR, FACILITY, LUMINANCE]);
    std::fs::write(
        dir.path().join("interop_cpl.xml"),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.digicine.com/PROTO-ASDCP-CPL-20040511#">
  <Id>urn:uuid:9a0e8f56-2d4b-4c1e-8f3a-6b7c5d4e3f21</Id>
  <ContentTitleText>{}</ContentTitleText>
  <ReelList/>
</CompositionPlaylist>"#,
            common::ISDCF_TITLE
        ),
    )
    .unwrap();

    let items = metadata_items(&dir);

    assert_eq!(items.len(), 4, "only the SMPTE CPL is checked: {items:#?}");
    assert!(failed(&items).is_empty(), "{items:#?}");
}
