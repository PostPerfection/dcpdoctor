mod support;

use assert_cmd::Command;
use tempfile::TempDir;

const NEUTRAL_RGB_CODES: [u16; 3] = [2048, 2048, 2048];
const PKL_CPL_PICTURE_AND_SOUND_ASSETS: usize = 4;
const IMP_TITLE: &str = "App 2E sample fixture";

fn field<'a>(stdout: &'a str, label: &str) -> &'a str {
    stdout
        .lines()
        .find_map(|line| line.trim().strip_prefix(label))
        .unwrap_or_else(|| panic!("no {label} line in {stdout}"))
        .trim()
}

fn track<'a>(stdout: &'a str, kind: &str) -> &'a str {
    stdout
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with(kind))
        .unwrap_or_else(|| panic!("no {kind} track line in {stdout}"))
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
        PKL_CPL_PICTURE_AND_SOUND_ASSETS.to_string()
    );
    assert_eq!(field(&stdout, "CPLs:"), "1");
    assert_eq!(field(&stdout, "PKLs:"), "1");
}

#[test]
fn imp_info_reads_the_title_and_tracks_of_an_imf_cpl() {
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

    assert_eq!(field(&stdout, "Title:"), IMP_TITLE);
    assert_eq!(
        field(&stdout, "Edit rate:"),
        format!("{}/1", support::EDIT_RATE)
    );
    assert_eq!(
        field(&stdout, "Duration:"),
        format!("{} frames", support::IMP_FRAMES)
    );

    let picture = track(&stdout, "MainImage");
    assert!(
        picture.contains(&support::imp_picture_file()),
        "the picture track must name its track file: {picture}"
    );
    assert!(
        picture.contains(&format!(
            "{}x{}",
            support::PICTURE_WIDTH,
            support::PICTURE_HEIGHT
        )),
        "the picture track must report the raster its descriptor carries: {picture}"
    );
    assert!(
        picture.contains(&format!("{} frames", support::IMP_FRAMES)),
        "the picture track must report the frames its descriptor carries: {picture}"
    );

    let sound = track(&stdout, "MainAudio");
    assert!(
        sound.contains(&support::imp_sound_file()),
        "the sound track must name its track file: {sound}"
    );
    assert!(
        sound.contains(&format!("{} channels", support::CHANNELS)),
        "the sound track must report its channel count: {sound}"
    );
    assert!(
        sound.contains(&format!("{} Hz", support::SAMPLE_RATE)),
        "the sound track must report its sample rate: {sound}"
    );
}

#[test]
fn imp_info_refuses_a_dcp_and_names_the_command_that_reads_one() {
    let root = TempDir::new().unwrap();
    let dcp = root.path().join("dcp");
    std::fs::create_dir_all(&dcp).unwrap();
    support::write_package(&dcp, &support::PackageSpec::default());

    let output = Command::cargo_bin("dcpdoctor")
        .unwrap()
        .arg("imp-info")
        .arg(&dcp)
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "a DCP is not an IMP: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("dcpdoctor info"),
        "the refusal must name the command that reads a DCP: {stderr}"
    );
}
