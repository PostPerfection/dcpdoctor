//! `dcpdoctor frame-compare` over real DCP picture essence, which decodes to
//! RGB rather than the luma ffmpeg's stats name for YUV video.

mod support;

use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

/// What postkit reports for a frame with no error, ffmpeg's `inf`.
const LOSSLESS_PSNR: f64 = 100.0;

fn compare(a: &Path, b: &Path) -> serde_json::Value {
    let output = Command::cargo_bin("dcpdoctor")
        .unwrap()
        .args(["--json", "frame-compare", "--imp-a"])
        .arg(a)
        .arg("--imp-b")
        .arg(b)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&output).unwrap()
}

#[test]
fn a_picture_matches_itself_and_a_re_encode_of_it_does_not() {
    let root = TempDir::new().unwrap();
    let bars = root.path().join("bars");
    let rebars = root.path().join("rebars");
    std::fs::create_dir_all(&bars).unwrap();
    std::fs::create_dir_all(&rebars).unwrap();
    support::write_package(&bars, &support::PackageSpec::default());
    support::write_package(
        &rebars,
        &support::PackageSpec {
            // the same frames at a coarser quantizer
            picture_quality: 40,
            ..Default::default()
        },
    );

    let same = compare(&bars, &bars);
    assert_eq!(same["frames_compared"], support::FRAMES);
    assert_eq!(same["avg_psnr"].as_f64().unwrap(), LOSSLESS_PSNR);
    assert_eq!(same["min_psnr"].as_f64().unwrap(), LOSSLESS_PSNR);
    assert_eq!(same["avg_ssim"].as_f64().unwrap(), 1.0);
    assert_eq!(same["min_ssim"].as_f64().unwrap(), 1.0);
    assert_eq!(same["verdict"], "identical");

    let degraded = compare(&bars, &rebars);
    assert_eq!(degraded["frames_compared"], support::FRAMES);
    let psnr = degraded["avg_psnr"].as_f64().unwrap();
    let ssim = degraded["avg_ssim"].as_f64().unwrap();
    assert!(
        psnr.is_finite() && psnr < LOSSLESS_PSNR,
        "a re-encode must measure a finite PSNR, got {psnr}"
    );
    assert!(
        ssim < 1.0,
        "a re-encode must measure an SSIM below 1: {ssim}"
    );
    assert_ne!(
        degraded["verdict"], "identical",
        "a {psnr:.2} dB re-encode is not an exact match"
    );
}

#[test]
fn the_verdict_line_names_the_threshold_it_was_measured_against() {
    let dir = TempDir::new().unwrap();
    support::write_package(dir.path(), &support::PackageSpec::default());
    let picture = dir.path().join(support::PICTURE_FILE);

    Command::cargo_bin("dcpdoctor")
        .unwrap()
        .args(["frame-compare", "--file-a"])
        .arg(&picture)
        .arg("--file-b")
        .arg(&picture)
        .assert()
        .success()
        .stdout(predicates::str::contains("Result:          IDENTICAL"))
        .stdout(predicates::str::contains("Frames below").not());
}
