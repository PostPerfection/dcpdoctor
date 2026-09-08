// end-to-end runs of the QC subcommands over synthetic packages and track files

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use std::path::Path;
use tempfile::TempDir;

fn cmd() -> Command {
    Command::cargo_bin("dcpdoctor").unwrap()
}

fn ffmpeg_video(path: &Path, args: &[&str]) {
    let output = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y"])
        .args(args)
        .args(["-c:v", "ffv1"])
        .arg(path)
        .output()
        .expect("ffmpeg has to be on PATH for the QC tests");
    assert!(
        output.status.success(),
        "ffmpeg failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// a second of colour bars, half a second of the given colour, a second of bars
fn video_with_middle_colour(path: &Path, colour: &str) {
    ffmpeg_video(
        path,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=128x128:rate=24:duration=1",
            "-f",
            "lavfi",
            "-i",
            &format!("color=c={colour}:size=128x128:rate=24:duration=0.5"),
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=128x128:rate=24:duration=1",
            "-filter_complex",
            "[0:v][1:v][2:v]concat=n=3:v=1:a=0",
        ],
    );
}

fn findings(output: &[u8]) -> Vec<String> {
    let report: serde_json::Value = serde_json::from_slice(output).unwrap();
    report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn auto_qc_names_the_seconds_a_black_run_covers() {
    let directory = TempDir::new().unwrap();
    let video = directory.path().join("black.mkv");
    video_with_middle_colour(&video, "black");

    let output = cmd()
        .args(["auto-qc", "--video", video.to_str().unwrap(), "--json"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();

    let findings = findings(&output);
    assert!(
        findings.contains(&"Black frames from 1.00 s to 1.50 s".to_string()),
        "{findings:?}"
    );
    assert!(
        findings.contains(&"Freeze frames from 1.00 s to 1.50 s".to_string()),
        "{findings:?}"
    );
}

#[test]
fn auto_qc_passes_a_picture_and_sound_with_nothing_wrong() {
    let directory = TempDir::new().unwrap();
    let video = directory.path().join("clean.mkv");
    ffmpeg_video(
        &video,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=128x128:rate=24:duration=2",
        ],
    );
    let audio = directory.path().join("clean.mxf");
    dcpdoctor_core::track_fixtures::write_sound_track(
        &audio,
        &[dcpdoctor_core::track_fixtures::SoundStretch {
            seconds: 2.0,
            amplitude: 0.5,
        }],
    );

    cmd()
        .args([
            "auto-qc",
            "--video",
            video.to_str().unwrap(),
            "--audio",
            audio.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("Auto-QC PASS"));
}

#[test]
fn the_black_threshold_decides_whether_a_dim_run_is_black() {
    let directory = TempDir::new().unwrap();
    let video = directory.path().join("dim.mkv");
    video_with_middle_colour(&video, "#333333");

    let at_default = cmd()
        .args(["auto-qc", "--video", video.to_str().unwrap(), "--json"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    assert!(
        !findings(&at_default)
            .iter()
            .any(|f| f.starts_with("Black frames")),
        "{:?}",
        findings(&at_default)
    );

    let output = cmd()
        .args([
            "auto-qc",
            "--video",
            video.to_str().unwrap(),
            "--black-threshold",
            "0.3",
            "--json",
        ])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();

    assert!(
        findings(&output)
            .iter()
            .any(|f| f.starts_with("Black frames from 1.00 s")),
        "{:?}",
        findings(&output)
    );
}

// an App 2E CPL carrying the ST 2067-21 clause 7.5 light levels and nothing else
fn write_content_light_cpl(path: &Path, max_cll: u32, max_fall: u32) {
    std::fs::write(
        path,
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/2067-3/2016">
  <Id>urn:uuid:4b0c85d9-b65d-4b1a-9cfd-92f0b28ca5f0</Id>
  <ExtensionProperties>
    <app2e:MaxCLL xmlns:app2e="http://www.smpte-ra.org/ns/2067-21/2020">{max_cll}</app2e:MaxCLL>
    <app2e:MaxFALL xmlns:app2e="http://www.smpte-ra.org/ns/2067-21/2020">{max_fall}</app2e:MaxFALL>
  </ExtensionProperties>
</CompositionPlaylist>"#
        ),
    )
    .unwrap();
}

#[test]
fn validate_hdr_reports_the_transfer_and_the_cpl_light_levels() {
    use dcpdoctor_core::track_fixtures::{pq_bt2020, write_picture_track};

    let directory = TempDir::new().unwrap();
    write_picture_track(&directory.path().join("PICTURE.mxf"), 2, Some(pq_bt2020()));
    write_content_light_cpl(&directory.path().join("CPL.xml"), 993, 362);

    cmd()
        .args([
            "validate",
            directory.path().to_str().unwrap(),
            "--hdr",
            "--verbose",
        ])
        .assert()
        .stdout(predicates::str::contains("HDR: HDR10, BT.2020 primaries"))
        .stdout(predicates::str::contains(
            "MaxCLL: 993 nits, MaxFALL: 362 nits",
        ));
}

#[test]
fn validate_hdr_flags_a_max_fall_above_its_max_cll() {
    use dcpdoctor_core::track_fixtures::{pq_bt2020, write_picture_track};

    let directory = TempDir::new().unwrap();
    write_picture_track(&directory.path().join("PICTURE.mxf"), 2, Some(pq_bt2020()));
    write_content_light_cpl(&directory.path().join("CPL.xml"), 400, 900);

    cmd()
        .args(["validate", directory.path().to_str().unwrap(), "--hdr"])
        .assert()
        .failure()
        .stdout(predicates::str::contains("hdr_metadata_invalid"))
        .stdout(predicates::str::contains(
            "MaxFALL 900 nits exceeds MaxCLL 400 nits",
        ));
}

#[test]
fn validate_hdr_says_nothing_about_a_rec_709_picture() {
    use dcpdoctor_core::track_fixtures::{bt709, write_picture_track};

    let directory = TempDir::new().unwrap();
    write_picture_track(&directory.path().join("PICTURE.mxf"), 2, Some(bt709()));
    write_content_light_cpl(&directory.path().join("CPL.xml"), 993, 362);

    cmd()
        .args([
            "validate",
            directory.path().to_str().unwrap(),
            "--hdr",
            "--verbose",
        ])
        .assert()
        .stdout(predicates::str::contains("HDR:").not())
        .stdout(predicates::str::contains("MaxCLL").not());
}

#[test]
fn validate_atmos_names_the_object_count_and_leaves_a_pcm_track_alone() {
    use dcpdoctor_core::track_fixtures::{SoundStretch, write_atmos_track, write_sound_track};

    let directory = TempDir::new().unwrap();
    write_atmos_track(&directory.path().join("ATMOS.mxf"), 24, 42);

    cmd()
        .args([
            "validate",
            directory.path().to_str().unwrap(),
            "--atmos",
            "--verbose",
        ])
        .assert()
        .stdout(predicates::str::contains(
            "Dolby Atmos (ST 429-18): 42 objects",
        ));

    let pcm_only = TempDir::new().unwrap();
    write_sound_track(
        &pcm_only.path().join("SOUND.mxf"),
        &[SoundStretch {
            seconds: 1.0,
            amplitude: 0.5,
        }],
    );

    cmd()
        .args([
            "validate",
            pcm_only.path().to_str().unwrap(),
            "--atmos",
            "--verbose",
        ])
        .assert()
        .stdout(predicates::str::contains("Atmos").not())
        .stdout(predicates::str::contains("Immersive audio").not());
}

#[test]
fn av_sync_flags_the_reel_whose_sound_is_offset_and_clears_the_others() {
    use dcpdoctor_core::track_fixtures::{ReelTiming, write_reel_cpl};

    let directory = TempDir::new().unwrap();
    write_reel_cpl(
        &directory.path().join("CPL.xml"),
        &[
            ReelTiming {
                picture_entry: 0,
                picture_duration: 48,
                sound_entry: 0,
                sound_duration: 48,
            },
            ReelTiming {
                picture_entry: 0,
                picture_duration: 48,
                sound_entry: 12,
                sound_duration: 48,
            },
        ],
    );

    cmd()
        .args(["av-sync", directory.path().to_str().unwrap()])
        .assert()
        .failure()
        .stdout(predicates::str::contains("Reel 1: in sync"))
        .stdout(predicates::str::contains(
            "Reel 2: sound enters 12 frames (500.0 ms) after picture",
        ));
}

#[test]
fn av_sync_passes_a_package_whose_reels_line_up() {
    use dcpdoctor_core::track_fixtures::{ReelTiming, write_reel_cpl};

    let directory = TempDir::new().unwrap();
    write_reel_cpl(
        &directory.path().join("CPL.xml"),
        &[ReelTiming {
            picture_entry: 0,
            picture_duration: 48,
            sound_entry: 0,
            sound_duration: 48,
        }],
    );

    cmd()
        .args(["av-sync", directory.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicates::str::contains("Reel 1: in sync"));
}

#[test]
fn auto_qc_names_the_silent_stretch_in_a_sound_track_file() {
    use dcpdoctor_core::track_fixtures::{SoundStretch, write_sound_track};

    let directory = TempDir::new().unwrap();
    let audio = directory.path().join("sound.mxf");
    write_sound_track(
        &audio,
        &[
            SoundStretch {
                seconds: 1.0,
                amplitude: 0.5,
            },
            SoundStretch {
                seconds: 1.0,
                amplitude: 0.0,
            },
            SoundStretch {
                seconds: 1.0,
                amplitude: 0.5,
            },
        ],
    );

    let output = cmd()
        .args(["auto-qc", "--audio", audio.to_str().unwrap(), "--json"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();

    assert_eq!(
        findings(&output),
        vec!["Audio silence from 1.00 s to 2.00 s"]
    );
}
