//! `dcpdoctor profiles --check` against real packages, and the playback
//! warnings `validate` raises alongside it.

mod common;

use assert_cmd::Command;
use common::{DcpSpec, write_dcp};
use predicates::prelude::*;
use tempfile::TempDir;

/// One profile per vendor the site names.
const VENDOR_PROFILES: [&str; 5] = [
    "Dolby IMS2000",
    "Barco SP2K",
    "Christie CP2230",
    "GDC SR-1000",
    "IMAX Digital",
];

fn cmd() -> Command {
    Command::cargo_bin("dcpdoctor").unwrap()
}

fn package(spec: DcpSpec) -> TempDir {
    let dir = TempDir::new().unwrap();
    write_dcp(dir.path(), &spec);
    dir
}

fn check(dir: &TempDir, profile: &str) -> assert_cmd::assert::Assert {
    cmd()
        .args([
            "profiles",
            "--check",
            profile,
            "--dcp",
            dir.path().to_str().unwrap(),
        ])
        .assert()
}

#[test]
fn a_plain_2k_package_is_compatible_with_every_vendor_profile() {
    let dir = package(DcpSpec::default());

    for profile in VENDOR_PROFILES {
        check(&dir, profile)
            .success()
            .stdout(predicate::str::contains(format!(
                "PASS: DCP is compatible with {profile}"
            )));
    }
}

#[test]
fn a_4k_package_fails_the_2k_only_profiles_by_name() {
    let dir = package(DcpSpec {
        width: 4096,
        height: 2160,
        ..Default::default()
    });

    check(&dir, "Barco SP2K")
        .failure()
        .stdout(predicate::str::contains(
            "Resolution 4096x2160 exceeds Barco SP2K maximum (2048x1080)",
        ))
        .stdout(predicate::str::contains("Barco SP2K does not support 4K"));

    check(&dir, "Dolby IMS2000").success();
}

#[test]
fn a_96_fps_package_fails_the_frame_rate_maximum_by_name() {
    let dir = package(DcpSpec {
        edit_rate: (96, 1),
        ..Default::default()
    });

    check(&dir, "Dolby IMS2000")
        .failure()
        .stdout(predicate::str::contains(
            "Frame rate 96 fps exceeds Dolby IMS2000 maximum (60 fps)",
        ));

    check(&dir, "Dolby IMS3000").success();
}

#[test]
fn a_sixteen_channel_track_fails_the_channel_maximum_by_name() {
    let dir = package(DcpSpec {
        channels: 16,
        ..Default::default()
    });

    check(&dir, "IMAX Digital")
        .failure()
        .stdout(predicate::str::contains(
            "Channel count 16 exceeds IMAX Digital maximum (12)",
        ));
    check(&dir, "GDC SR-1000")
        .failure()
        .stdout(predicate::str::contains(
            "Channel count 16 exceeds GDC SR-1000 maximum (8)",
        ));

    check(&dir, "Dolby IMS2000").success();
}

#[test]
fn a_4k_3d_package_fails_a_2k_profile_on_both_counts() {
    let dir = package(DcpSpec {
        width: 4096,
        height: 2160,
        stereo3d: true,
        ..Default::default()
    });

    check(&dir, "Christie CP2230")
        .failure()
        .stdout(predicate::str::contains(
            "Resolution 4096x2160 exceeds Christie CP2230 maximum (2048x1080)",
        ));
}

/// A package whose essence will not read must not come back compatible: the
/// limits were never compared against anything.
#[test]
fn essence_that_will_not_read_is_reported_not_checked() {
    let dir = package(DcpSpec::default());
    std::fs::write(dir.path().join(common::PICTURE_FILE), b"not an MXF").unwrap();
    std::fs::write(dir.path().join(common::SOUND_FILE), b"not an MXF").unwrap();

    check(&dir, "Barco SP2K")
        .failure()
        .stdout(predicate::str::contains("INCOMPLETE"))
        .stdout(predicate::str::contains("2048x1080 maximum"))
        .stdout(predicate::str::contains("16 channel maximum"))
        .stdout(predicate::str::contains("PASS").not());
}

// ─── playback warnings ────────────────────────────────────────────────────────

fn playback_warnings(spec: DcpSpec) -> String {
    let dir = package(spec);
    let output = cmd()
        .args(["validate", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn a_plain_package_raises_none_of_the_playback_warnings() {
    let stdout = playback_warnings(DcpSpec::default());
    for code in [
        "projector_frame_rate_support",
        "projector_4k_stereo_support",
        "distributor_audio_channel_count",
    ] {
        assert!(
            !stdout.contains(code),
            "24 fps 2K 2D 8-channel must not raise {code}, got: {stdout}"
        );
    }
}

#[test]
fn an_unusual_frame_rate_raises_the_projector_warning() {
    for (numerator, rate) in [(25, "25"), (30, "30"), (48, "48"), (50, "50"), (60, "60")] {
        let stdout = playback_warnings(DcpSpec {
            edit_rate: (numerator, 1),
            ..Default::default()
        });
        assert!(
            stdout.contains("projector_frame_rate_support")
                && stdout.contains(&format!("DCP is {rate} fps")),
            "{rate} fps must raise the projector warning, got: {stdout}"
        );
    }
}

#[test]
fn a_4k_3d_package_raises_the_4k_stereo_warning() {
    let stdout = playback_warnings(DcpSpec {
        width: 4096,
        height: 2160,
        stereo3d: true,
        ..Default::default()
    });
    assert!(
        stdout.contains("projector_4k_stereo_support"),
        "4K 3D must raise the projector warning, got: {stdout}"
    );
}

#[test]
fn a_six_channel_track_raises_the_distributor_warning() {
    let stdout = playback_warnings(DcpSpec {
        channels: 6,
        ..Default::default()
    });
    assert!(
        stdout.contains("distributor_audio_channel_count")
            && stdout.contains("sound has 6 channels"),
        "6 channels must raise the distributor warning, got: {stdout}"
    );
}
