//! MXF essence extraction via ffmpeg.

use std::path::PathBuf;
use std::process::Command;

use serde::Serialize;

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

    if opts.extract_video {
        let out_path = opts.output_dir.join(format!("{stem}_video.mxf"));
        let mut args: Vec<String> = vec![
            "-y".into(),
            "-i".into(),
            opts.input.to_string_lossy().into(),
        ];

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
            out_path.to_string_lossy().into(),
        ]);

        let status = Command::new("ffmpeg").args(&args).status();
        if status.is_ok_and(|s| s.success()) && out_path.exists() {
            result.extracted_files.push(out_path);
            result.frames_extracted = opts.end_frame.saturating_sub(opts.start_frame);
        }
    }

    if opts.extract_audio {
        let out_path = opts.output_dir.join(format!("{stem}_audio.wav"));
        let status = Command::new("ffmpeg")
            .args([
                "-y",
                "-i",
                &opts.input.to_string_lossy(),
                "-map",
                "0:a",
                "-c",
                "pcm_s24le",
                &out_path.to_string_lossy(),
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
