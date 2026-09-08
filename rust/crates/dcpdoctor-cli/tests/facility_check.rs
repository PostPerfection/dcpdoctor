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
