//! `dcpdoctor diff` over real DCPs: what changed between two packages, and the
//! perceptual picture fingerprint that tells a re-encode of the same content
//! apart from different content.

mod support;

use std::path::Path;

use assert_cmd::Command;
use dcpdoctor_core::premium::SAME_PICTURE_DISTANCE;
use tempfile::TempDir;

fn cmd() -> Command {
    Command::cargo_bin("dcpdoctor").unwrap()
}

fn diff_json(a: &Path, b: &Path) -> serde_json::Value {
    let output = cmd()
        .args(["--json", "diff"])
        .arg(a)
        .arg(b)
        .arg("--hashes")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&output).unwrap()
}

fn descriptions(diff: &serde_json::Value, category: &str) -> Vec<String> {
    diff["differences"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["category"] == category)
        .map(|d| d["description"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_changed_asset_and_a_changed_cpl_field_are_both_named() {
    let root = TempDir::new().unwrap();
    let a = root.path().join("a");
    let b = root.path().join("b");
    std::fs::create_dir_all(&a).unwrap();
    let spec = support::PackageSpec::default();
    support::write_package(&a, &spec);
    support::copy_package(&a, &b);

    support::corrupt_one_byte(&b.join(support::PICTURE_FILE));
    support::write_xml(
        &b,
        &support::PackageSpec {
            title: "Changed title".into(),
            ..support::PackageSpec::default()
        },
    );

    let diff = diff_json(&a, &b);
    assert_eq!(diff["identical"], false);

    let content = descriptions(&diff, "content");
    assert_eq!(content, ["CPL 1 title differs"], "{diff}");
    let titles: Vec<&str> = diff["differences"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["category"] == "content")
        .flat_map(|d| {
            [
                d["value_a"].as_str().unwrap(),
                d["value_b"].as_str().unwrap(),
            ]
        })
        .collect();
    assert_eq!(titles, [spec.title.as_str(), "Changed title"]);

    let hashes = descriptions(&diff, "hash");
    assert!(
        hashes.contains(&format!(
            "Asset {} has different content",
            support::PICTURE_ID
        )),
        "the corrupted picture asset must be named: {hashes:?}"
    );
    assert!(
        !hashes
            .iter()
            .any(|d| d.contains(support::SOUND_ID) || d.contains("not compared")),
        "the untouched sound asset must not be reported: {hashes:?}"
    );
}

#[test]
fn identical_packages_diff_clean() {
    let root = TempDir::new().unwrap();
    let a = root.path().join("a");
    let b = root.path().join("b");
    std::fs::create_dir_all(&a).unwrap();
    support::write_package(&a, &support::PackageSpec::default());
    support::copy_package(&a, &b);

    let diff = diff_json(&a, &b);
    assert_eq!(diff["identical"], true, "{diff}");
    assert!(diff["differences"].as_array().unwrap().is_empty());

    cmd()
        .args(["diff"])
        .arg(&a)
        .arg(&b)
        .arg("--hashes")
        .assert()
        .success()
        .stdout(predicates::str::contains("DCPs are identical"));
}

/// The similarity the CLI prints, and its verdict.
fn fingerprint_line(a: &Path, b: &Path) -> String {
    let output = cmd()
        .args(["diff"])
        .arg(a)
        .arg(b)
        .arg("--fingerprint")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output)
        .unwrap()
        .lines()
        .find(|l| l.starts_with("Picture fingerprint:"))
        .expect("the diff must print a fingerprint line")
        .to_string()
}

fn similarity_percent(line: &str) -> f64 {
    let at = line.find('(').expect(line);
    line[at + 1..]
        .split('%')
        .next()
        .unwrap()
        .trim()
        .parse()
        .expect(line)
}

#[test]
fn the_fingerprint_sees_past_a_re_encode_but_not_past_different_content() {
    let root = TempDir::new().unwrap();
    let bars = root.path().join("bars");
    let rebars = root.path().join("rebars");
    let flat = root.path().join("flat");
    for (dir, spec) in [
        (&bars, support::PackageSpec::default()),
        (
            &rebars,
            support::PackageSpec {
                // the same frames at a coarser quantizer, so a smaller picture track
                picture_quality: 40,
                ..support::PackageSpec::default()
            },
        ),
        (
            &flat,
            support::PackageSpec {
                picture: support::Picture::Flat,
                ..support::PackageSpec::default()
            },
        ),
    ] {
        std::fs::create_dir_all(dir).unwrap();
        support::write_package(dir, &spec);
    }
    assert!(
        std::fs::metadata(rebars.join(support::PICTURE_FILE))
            .unwrap()
            .len()
            < std::fs::metadata(bars.join(support::PICTURE_FILE))
                .unwrap()
                .len(),
        "the re-encode must really be another bitrate"
    );

    let same_picture_similarity = (1.0 - SAME_PICTURE_DISTANCE) * 100.0;

    let re_encode = fingerprint_line(&bars, &rebars);
    assert!(
        similarity_percent(&re_encode) >= same_picture_similarity,
        "a re-encode of the same frames must stay at or above {same_picture_similarity}% similar: {re_encode}"
    );
    assert!(re_encode.contains("same picture"), "{re_encode}");

    let other_content = fingerprint_line(&bars, &flat);
    assert!(
        similarity_percent(&other_content) < same_picture_similarity,
        "test bars against a flat colour must fall below {same_picture_similarity}% similar: {other_content}"
    );
    assert!(
        other_content.contains("different picture"),
        "{other_content}"
    );
}
