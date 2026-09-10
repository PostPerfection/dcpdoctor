//! MXF essence extraction via ffmpeg.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

// a run that succeeded has to leave stderr clean
const QUIET_LOG_LEVEL: [&str; 2] = ["-v", "error"];

/// Options for MXF extraction.
pub struct MxfExtractOptions {
    pub input: PathBuf,
    pub output_dir: PathBuf,
    pub extract_video: bool,
    pub extract_audio: bool,
    pub start_frame: u32,
    pub end_frame: u32,
}

/// Result of MXF extraction.
#[derive(Debug, Clone, Default, Serialize)]
pub struct MxfExtractResult {
    pub success: bool,
    pub error: String,
    pub extracted_files: Vec<PathBuf>,
    pub frames_extracted: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EssenceKind {
    Picture,
    Sound,
    Unknown,
}

/// Extract video/audio essence from an MXF file.
pub fn extract_mxf(opts: &MxfExtractOptions) -> MxfExtractResult {
    let mut result = MxfExtractResult::default();

    if !opts.input.exists() {
        result.error = format!("MXF file not found: {}", opts.input.display());
        return result;
    }

    if let Err(e) = std::fs::create_dir_all(&opts.output_dir) {
        result.error = format!("Cannot create output dir: {e}");
        return result;
    }

    let stem = opts.input.file_stem().unwrap_or_default().to_string_lossy();

    // the pass for the other essence kind has no stream to map
    let kind = essence_kind(&opts.input);

    if opts.extract_video && kind != EssenceKind::Sound {
        let out_path = opts.output_dir.join(format!("{stem}_video.mxf"));
        let mut args: Vec<String> = QUIET_LOG_LEVEL.map(String::from).to_vec();
        args.extend([
            "-y".into(),
            "-i".into(),
            crate::studio::ffmpeg_path_argument(&opts.input)
                .to_string_lossy()
                .into(),
        ]);

        if opts.start_frame > 0 {
            // Approximate start time (assume 24fps if we don't know)
            let start_sec = opts.start_frame as f64 / 24.0;
            args.push("-ss".into());
            args.push(format!("{start_sec}"));
        }
        if opts.end_frame > opts.start_frame {
            let count = opts.end_frame - opts.start_frame;
            args.push("-frames:v".into());
            args.push(count.to_string());
        }

        args.extend([
            "-map".into(),
            "0:v".into(),
            "-c".into(),
            "copy".into(),
            crate::studio::ffmpeg_path_argument(&out_path)
                .to_string_lossy()
                .into(),
        ]);

        let status = Command::new("ffmpeg").args(&args).status();
        if status.is_ok_and(|s| s.success()) && out_path.exists() {
            result.frames_extracted = frames_written(&out_path);
            result.extracted_files.push(out_path);
        }
    }

    if opts.extract_audio && kind != EssenceKind::Picture {
        let out_path = opts.output_dir.join(format!("{stem}_audio.wav"));
        let status = Command::new("ffmpeg")
            .args(QUIET_LOG_LEVEL)
            .args([
                "-y",
                "-i",
                &crate::studio::ffmpeg_path_argument(&opts.input).to_string_lossy(),
                "-map",
                "0:a",
                "-c",
                "pcm_s24le",
                &crate::studio::ffmpeg_path_argument(&out_path).to_string_lossy(),
            ])
            .status();

        if status.is_ok_and(|s| s.success()) && out_path.exists() {
            result.extracted_files.push(out_path);
        }
    }

    result.success = !result.extracted_files.is_empty();
    if !result.success && result.error.is_empty() {
        result.error = format!("No essence extracted from {}", opts.input.display());
    }

    result
}

// the essence type comes from the MXF header, the same reader the validators use
fn essence_kind(path: &Path) -> EssenceKind {
    use asdcplib::EssenceType;

    let Some(path) = path.to_str() else {
        return EssenceKind::Unknown;
    };
    match asdcplib::essence_type(path) {
        Ok(EssenceType::Jpeg2000 | EssenceType::Jpeg2000Stereo | EssenceType::As02Jpeg2000) => {
            EssenceKind::Picture
        }
        Ok(
            EssenceType::Pcm24b48k
            | EssenceType::Pcm24b96k
            | EssenceType::As02Pcm24b48k
            | EssenceType::As02Pcm24b96k,
        ) => EssenceKind::Sound,
        _ => EssenceKind::Unknown,
    }
}

// counted off the file ffmpeg wrote
fn frames_written(picture: &Path) -> u32 {
    let output = Command::new("ffprobe")
        .args(QUIET_LOG_LEVEL)
        .args([
            "-count_packets",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=nb_read_packets",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(crate::studio::ffmpeg_path_argument(picture))
        .output();

    output
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8_lossy(&output.stdout).trim().parse().ok())
        .unwrap_or(0)
}
