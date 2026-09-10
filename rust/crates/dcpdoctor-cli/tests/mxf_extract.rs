mod support;

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use tempfile::TempDir;

const SOC_AND_SIZ_MARKERS: [u8; 4] = [0xff, 0x4f, 0xff, 0x51];

const RIFF_HEADER_BYTES: usize = 12;
const FORMAT_CHUNK: &[u8; 4] = b"fmt ";
const DATA_CHUNK: &[u8; 4] = b"data";
const CHANNEL_COUNT_OFFSET: usize = 2;
const SAMPLE_RATE_OFFSET: usize = 4;
const BITS_PER_SAMPLE_OFFSET: usize = 14;

fn extract(root: &Path, track_file: &str, skip_other_essence: &str) -> serde_json::Value {
    let dcp = root.join("dcp");
    std::fs::create_dir_all(&dcp).unwrap();
    support::write_package(&dcp, &support::PackageSpec::default());

    let stdout = Command::cargo_bin("dcpdoctor")
        .unwrap()
        .args(["--json", "mxf-extract"])
        .arg(dcp.join(track_file))
        .arg("-o")
        .arg(root.join("extracted"))
        .arg(skip_other_essence)
        .output()
        .unwrap()
        .stdout;
    serde_json::from_slice(&stdout).unwrap()
}

fn only_extracted_file(result: &serde_json::Value) -> PathBuf {
    assert_eq!(result["success"], true, "extraction failed: {result:#}");
    let files = result["extracted_files"].as_array().unwrap();
    assert_eq!(files.len(), 1, "one essence kind was asked for: {result:#}");
    PathBuf::from(files[0].as_str().unwrap())
}

fn demux_codestreams(picture: &Path, into: &Path) -> Vec<PathBuf> {
    std::fs::create_dir_all(into).unwrap();
    let status = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-i"])
        .arg(picture)
        .args(["-c", "copy", "-f", "image2"])
        .arg(into.join("frame%04d.j2k"))
        .status()
        .expect("run ffmpeg");
    assert!(
        status.success(),
        "ffmpeg could not read {}",
        picture.display()
    );

    let mut paths: Vec<PathBuf> = std::fs::read_dir(into)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .collect();
    paths.sort();
    paths
}

fn wav_chunk<'a>(wav: &'a [u8], wanted: &[u8; 4]) -> Option<&'a [u8]> {
    let mut at = RIFF_HEADER_BYTES;
    while at + 8 <= wav.len() {
        let id = &wav[at..at + 4];
        let size = u32::from_le_bytes(wav[at + 4..at + 8].try_into().unwrap()) as usize;
        let end = (at + 8 + size).min(wav.len());
        if id == wanted {
            return Some(&wav[at + 8..end]);
        }
        // odd-sized chunks carry a pad byte
        at = end + (size & 1);
    }
    None
}

fn read_u16(chunk: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(chunk[at..at + 2].try_into().unwrap())
}

#[test]
fn extracting_a_picture_track_yields_one_jpeg_2000_codestream_per_frame() {
    let root = TempDir::new().unwrap();
    let result = extract(root.path(), support::PICTURE_FILE, "--no-audio");
    let picture = only_extracted_file(&result);

    let codestreams = demux_codestreams(&picture, &root.path().join("codestreams"));
    assert_eq!(
        codestreams.len(),
        support::FRAMES as usize,
        "the extracted picture must carry every frame the package was written with"
    );
    for codestream in &codestreams {
        let bytes = std::fs::read(codestream).unwrap();
        assert_eq!(
            &bytes[..SOC_AND_SIZ_MARKERS.len()],
            SOC_AND_SIZ_MARKERS,
            "{} does not start with the SOC and SIZ markers",
            codestream.display()
        );
    }
}

#[test]
fn extracting_a_sound_track_yields_the_packages_pcm_as_wav() {
    let root = TempDir::new().unwrap();
    let result = extract(root.path(), support::SOUND_FILE, "--no-video");
    let sound = only_extracted_file(&result);

    let wav = std::fs::read(&sound).unwrap();
    assert_eq!(&wav[..4], b"RIFF", "{} is not a WAV", sound.display());
    assert_eq!(&wav[8..12], b"WAVE", "{} is not a WAV", sound.display());

    let format = wav_chunk(&wav, FORMAT_CHUNK).expect("the WAV has a fmt chunk");
    assert_eq!(
        read_u16(format, CHANNEL_COUNT_OFFSET) as u32,
        support::CHANNELS
    );
    assert_eq!(
        u32::from_le_bytes(
            format[SAMPLE_RATE_OFFSET..SAMPLE_RATE_OFFSET + 4]
                .try_into()
                .unwrap()
        ),
        support::SAMPLE_RATE
    );
    assert_eq!(
        read_u16(format, BITS_PER_SAMPLE_OFFSET) as u32,
        support::BYTES_PER_SAMPLE * 8
    );

    let data = wav_chunk(&wav, DATA_CHUNK).expect("the WAV has a data chunk");
    let block_align = (support::CHANNELS * support::BYTES_PER_SAMPLE) as usize;
    assert_eq!(
        data.len() / block_align,
        (support::FRAMES * support::SAMPLE_RATE / support::EDIT_RATE) as usize,
        "the extracted sound must run as long as the package"
    );
}
