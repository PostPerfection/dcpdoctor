//! Audio/video sync detection and repair via ffprobe/ffmpeg.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

/// Options for A/V sync detection.
pub struct AvSyncOptions {
    pub video_file: PathBuf,
    pub audio_file: PathBuf,
    pub fps_num: u32,
    pub fps_den: u32,
    pub sample_rate: u32,
}

/// Result of A/V sync detection.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AvSyncResult {
    pub success: bool,
    pub error: String,
    pub drift_ms: f64,
    pub drift_samples: i32,
    pub drift_frames: f64,
    pub in_sync: bool,
    pub recommendation: String,
}

/// Options for fixing A/V sync.
pub struct AvSyncFixOptions {
    pub audio_file: PathBuf,
    pub output_file: PathBuf,
    pub trim_samples: i32,
    pub sample_rate: u32,
    pub bit_depth: u32,
}

/// Result of A/V sync fix.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AvSyncFixResult {
    pub success: bool,
    pub error: String,
    pub output_file: PathBuf,
    pub samples_adjusted: i32,
}

fn probe_duration(file: &Path) -> Option<f64> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
            &file.to_string_lossy(),
        ])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&output.stdout);
    s.trim().parse().ok()
}

/// Detect A/V sync drift between video and audio files.
pub fn detect_av_sync(opts: &AvSyncOptions) -> AvSyncResult {
    let mut result = AvSyncResult::default();

    if !opts.video_file.exists() {
        result.error = format!("Video file not found: {}", opts.video_file.display());
        return result;
    }
    if !opts.audio_file.exists() {
        result.error = format!("Audio file not found: {}", opts.audio_file.display());
        return result;
    }

    let video_duration = match probe_duration(&opts.video_file) {
        Some(d) if d > 0.0 => d,
        _ => {
            result.error = "Failed to determine video duration".into();
            return result;
        }
    };

    let audio_duration = match probe_duration(&opts.audio_file) {
        Some(d) if d > 0.0 => d,
        _ => {
            result.error = "Failed to determine audio duration".into();
            return result;
        }
    };

    let drift_seconds = audio_duration - video_duration;
    result.drift_ms = drift_seconds * 1000.0;
    result.drift_samples = (drift_seconds * opts.sample_rate as f64) as i32;

    let frame_duration = opts.fps_den as f64 / opts.fps_num as f64;
    result.drift_frames = drift_seconds / frame_duration;

    // Within ±1 frame = in sync
    result.in_sync = result.drift_frames.abs() < 1.0;

    result.recommendation = if result.drift_samples > 0 {
        format!(
            "Audio is {} samples longer than video. Trim {} samples from audio tail.",
            result.drift_samples.unsigned_abs(),
            result.drift_samples.unsigned_abs()
        )
    } else if result.drift_samples < 0 {
        format!(
            "Audio is {} samples shorter than video. Pad {} samples of silence at audio tail.",
            result.drift_samples.unsigned_abs(),
            result.drift_samples.unsigned_abs()
        )
    } else {
        "Audio and video are perfectly in sync.".into()
    };

    result.success = true;
    result
}

// how one reel's sound timing compares with its picture, in the reel's own edit units
#[derive(Debug, Clone, Default, Serialize)]
pub struct ReelSync {
    pub reel: usize,
    pub picture_frames: i64,
    pub sound_frames: i64,
    // sound entering later than picture plays the reel's sound early
    pub sound_offset_frames: i64,
    pub sound_offset_ms: f64,
    pub duration_difference_frames: i64,
    pub duration_difference_ms: f64,
    pub in_sync: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PackageAvSyncResult {
    pub success: bool,
    pub error: String,
    pub reels: Vec<ReelSync>,
    // reasons a reel could not be compared
    pub skipped: Vec<String>,
    pub in_sync: bool,
}

// the CPL's own timing is what a server plays, so drift is read from the reels
pub fn detect_package_av_sync(dcp_dir: &Path) -> PackageAvSyncResult {
    use crate::assetmap::ParseXmlFile;

    let mut result = PackageAvSyncResult::default();

    let Some(cpl_path) = find_cpl(dcp_dir) else {
        result.error = format!("no CPL found in {}", dcp_dir.display());
        return result;
    };
    let Some(cpl) = crate::cpl::Cpl::parse(&cpl_path) else {
        result.error = format!("cannot parse {}", cpl_path.display());
        return result;
    };
    if cpl.reels.is_empty() {
        result.error = format!("{} lists no reels", cpl_path.display());
        return result;
    }

    for (index, reel) in cpl.reels.iter().enumerate() {
        let reel_number = index + 1;
        if reel.sound.id.is_empty() {
            result
                .skipped
                .push(format!("reel {reel_number} has no sound asset"));
            continue;
        }
        if reel.picture.id.is_empty() {
            result
                .skipped
                .push(format!("reel {reel_number} has no picture asset"));
            continue;
        }
        if reel.picture.duration_unparseable
            || reel.sound.duration_unparseable
            || reel.picture.entry_point_unparseable
            || reel.sound.entry_point_unparseable
        {
            result.skipped.push(format!(
                "reel {reel_number} declares a duration or entry point that is no integer"
            ));
            continue;
        }

        let fps = frames_per_second(&reel.picture.edit_rate)
            .or_else(|| frames_per_second(&reel.sound.edit_rate))
            .or_else(|| frames_per_second(&cpl.edit_rate));
        let Some(fps) = fps else {
            result.skipped.push(format!(
                "reel {reel_number} declares no edit rate this can be measured in"
            ));
            continue;
        };

        let sound_offset_frames =
            reel.sound.entry_point.unwrap_or(0) - reel.picture.entry_point.unwrap_or(0);
        let duration_difference_frames = reel.sound.duration - reel.picture.duration;
        let milliseconds = |frames: i64| frames as f64 / fps * 1000.0;

        result.reels.push(ReelSync {
            reel: reel_number,
            picture_frames: reel.picture.duration,
            sound_frames: reel.sound.duration,
            sound_offset_frames,
            sound_offset_ms: milliseconds(sound_offset_frames),
            duration_difference_frames,
            duration_difference_ms: milliseconds(duration_difference_frames),
            in_sync: sound_offset_frames == 0 && duration_difference_frames == 0,
        });
    }

    result.in_sync = result.reels.iter().all(|reel| reel.in_sync);
    result.success = true;
    result
}

impl ReelSync {
    pub fn describe(&self) -> String {
        if self.in_sync {
            return format!("Reel {}: in sync", self.reel);
        }
        let mut parts = Vec::new();
        if self.sound_offset_frames != 0 {
            parts.push(format!(
                "sound enters {} frames ({:.1} ms) {} picture",
                self.sound_offset_frames.abs(),
                self.sound_offset_ms.abs(),
                if self.sound_offset_frames > 0 {
                    "after"
                } else {
                    "before"
                }
            ));
        }
        if self.duration_difference_frames != 0 {
            parts.push(format!(
                "sound runs {} frames ({:.1} ms) {} picture",
                self.duration_difference_frames.abs(),
                self.duration_difference_ms.abs(),
                if self.duration_difference_frames > 0 {
                    "longer than"
                } else {
                    "shorter than"
                }
            ));
        }
        format!("Reel {}: {}", self.reel, parts.join(", "))
    }
}

fn find_cpl(dcp_dir: &Path) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dcp_dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("xml") {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&path)
            && content.contains("CompositionPlaylist")
        {
            return Some(path);
        }
    }
    None
}

// a CPL edit rate is "numerator denominator"
fn frames_per_second(edit_rate: &str) -> Option<f64> {
    let mut parts = edit_rate.split_whitespace();
    let numerator: f64 = parts.next()?.parse().ok()?;
    let denominator: f64 = parts.next().unwrap_or("1").parse().unwrap_or(1.0);
    if numerator <= 0.0 || denominator <= 0.0 {
        return None;
    }
    Some(numerator / denominator)
}

/// Fix A/V sync by trimming or padding audio.
pub fn fix_av_sync(opts: &AvSyncFixOptions) -> AvSyncFixResult {
    let mut result = AvSyncFixResult::default();

    if !opts.audio_file.exists() {
        result.error = format!("Audio file not found: {}", opts.audio_file.display());
        return result;
    }

    if opts.trim_samples == 0 {
        // No adjustment needed — just copy
        if let Err(e) = std::fs::copy(&opts.audio_file, &opts.output_file) {
            result.error = format!("Copy failed: {e}");
            return result;
        }
        result.output_file = opts.output_file.clone();
        result.samples_adjusted = 0;
        result.success = true;
        return result;
    }

    let out_str = opts.output_file.to_string_lossy();
    let status = if opts.trim_samples > 0 {
        let trim_seconds = opts.trim_samples as f64 / opts.sample_rate as f64;
        Command::new("ffmpeg")
            .args([
                "-y",
                "-i",
                &opts.audio_file.to_string_lossy(),
                "-ss",
                &format!("{trim_seconds}"),
                "-c:a",
                &format!("pcm_s{}le", opts.bit_depth),
                "-ar",
                &opts.sample_rate.to_string(),
                out_str.as_ref(),
            ])
            .status()
    } else {
        let pad_seconds = (-opts.trim_samples) as f64 / opts.sample_rate as f64;
        Command::new("ffmpeg")
            .args([
                "-y",
                "-f",
                "lavfi",
                "-t",
                &format!("{pad_seconds}"),
                "-i",
                &format!("anullsrc=r={}", opts.sample_rate),
                "-i",
                &opts.audio_file.to_string_lossy(),
                "-filter_complex",
                "[0:a][1:a]concat=n=2:v=0:a=1",
                "-c:a",
                &format!("pcm_s{}le", opts.bit_depth),
                out_str.as_ref(),
            ])
            .status()
    };

    match status {
        Ok(s) if s.success() => {
            result.output_file = opts.output_file.clone();
            result.samples_adjusted = opts.trim_samples;
            result.success = true;
        }
        _ => {
            result.error = "ffmpeg A/V sync fix failed".into();
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track_fixtures::{ReelTiming, write_reel_cpl};

    fn matched(picture_duration: i64) -> ReelTiming {
        ReelTiming {
            picture_entry: 0,
            picture_duration,
            sound_entry: 0,
            sound_duration: picture_duration,
        }
    }

    fn package(reels: &[ReelTiming]) -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().to_path_buf();
        write_reel_cpl(&path.join("CPL.xml"), reels);
        (directory, path)
    }

    #[test]
    fn a_sound_entry_point_past_the_pictures_is_drift_on_that_reel_alone() {
        let (_directory, path) = package(&[
            matched(48),
            ReelTiming {
                picture_entry: 0,
                picture_duration: 48,
                sound_entry: 12,
                sound_duration: 48,
            },
        ]);

        let result = detect_package_av_sync(&path);

        assert!(result.success, "{}", result.error);
        assert!(!result.in_sync);
        assert_eq!(result.reels.len(), 2);
        assert!(result.reels[0].in_sync, "{:?}", result.reels[0]);
        assert_eq!(result.reels[1].sound_offset_frames, 12);
        assert_eq!(result.reels[1].sound_offset_ms, 500.0);
        assert_eq!(
            result.reels[1].describe(),
            "Reel 2: sound enters 12 frames (500.0 ms) after picture"
        );
    }

    #[test]
    fn a_sound_track_shorter_than_its_picture_is_drift() {
        let (_directory, path) = package(&[ReelTiming {
            picture_entry: 0,
            picture_duration: 48,
            sound_entry: 0,
            sound_duration: 24,
        }]);

        let result = detect_package_av_sync(&path);

        assert_eq!(result.reels[0].duration_difference_frames, -24);
        assert_eq!(
            result.reels[0].describe(),
            "Reel 1: sound runs 24 frames (1000.0 ms) shorter than picture"
        );
    }

    #[test]
    fn a_reel_whose_picture_and_sound_are_trimmed_alike_is_in_sync() {
        let (_directory, path) = package(&[ReelTiming {
            picture_entry: 12,
            picture_duration: 48,
            sound_entry: 12,
            sound_duration: 48,
        }]);

        let result = detect_package_av_sync(&path);

        assert!(result.in_sync, "{:?}", result.reels);
    }

    #[test]
    fn a_directory_with_no_cpl_says_so_instead_of_reporting_no_drift() {
        let directory = tempfile::tempdir().unwrap();

        let result = detect_package_av_sync(directory.path());

        assert!(!result.success);
        assert!(result.error.contains("no CPL"), "{}", result.error);
    }
}
