use std::path::{Path, PathBuf};

use serde::Serialize;

// ffmpeg's picture_black_ratio_th, the share of a frame that has to be black
const BLACK_PICTURE_RATIO: f64 = 0.98;

// one frame at 24 fps, so a single black frame is reported
const BLACK_MIN_SECONDS: f64 = 0.04;

const FREEZE_MIN_SECONDS: f64 = 0.5;

const SILENCE_MIN_SECONDS: f64 = 0.1;

pub struct AutoQcOptions {
    pub video: Option<PathBuf>,
    pub audio: Option<PathBuf>,
    // pixel luma at or below this fraction of full scale is black
    pub black_threshold: f64,
    // freeze noise tolerance as a fraction of full scale
    pub freeze_threshold: f64,
    // dBFS below which a stretch is silence
    pub silence_threshold: f64,
    // dBFS at or above which a channel peak is clipping
    pub clipping_threshold: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AutoQcResult {
    pub findings: Vec<String>,
}

impl AutoQcResult {
    pub fn pass(&self) -> bool {
        self.findings.is_empty()
    }
}

pub fn run(opts: &AutoQcOptions) -> AutoQcResult {
    let mut result = AutoQcResult::default();

    if let Some(video) = &opts.video {
        match picture_findings(video, opts.black_threshold, opts.freeze_threshold) {
            Ok(findings) => result.findings.extend(findings),
            Err(e) => result
                .findings
                .push(format!("Picture analysis failed: {e}")),
        }
    }

    if let Some(audio) = &opts.audio {
        result
            .findings
            .extend(silence_findings(audio, opts.silence_threshold));
        result
            .findings
            .extend(clipping_findings(audio, opts.clipping_threshold));
    }

    result
}

fn picture_findings(
    video: &Path,
    black_threshold: f64,
    freeze_threshold: f64,
) -> Result<Vec<String>, String> {
    let filters = format!(
        "blackdetect=d={BLACK_MIN_SECONDS}:pic_th={BLACK_PICTURE_RATIO}:pix_th={black_threshold},\
         freezedetect=n={freeze_threshold}:d={FREEZE_MIN_SECONDS}"
    );
    let stderr = run_filter(video, &["-vf", &filters])?;

    let mut findings = Vec::new();
    for run in intervals(&stderr, "black_start", "black_end") {
        findings.push(format!("Black frames {}", run.describe()));
    }
    for run in intervals(&stderr, "freeze_start", "freeze_end") {
        findings.push(format!("Freeze frames {}", run.describe()));
    }
    Ok(findings)
}

// a level taken over the whole file cannot show a stretch, so the filter runs
fn silence_findings(audio: &Path, silence_threshold: f64) -> Vec<String> {
    let filter = format!("silencedetect=noise={silence_threshold}dB:d={SILENCE_MIN_SECONDS}");
    let stderr = match run_filter(audio, &["-af", &filter]) {
        Ok(stderr) => stderr,
        Err(e) => return vec![format!("Silence analysis failed: {e}")],
    };

    intervals(&stderr, "silence_start", "silence_end")
        .into_iter()
        .map(|run| format!("Audio silence {}", run.describe()))
        .collect()
}

fn clipping_findings(audio: &Path, clipping_threshold: f64) -> Vec<String> {
    let analysis = match crate::audio::analyze_audio(audio) {
        Ok(analysis) => analysis,
        Err(e) => return vec![format!("Audio level analysis failed: {e}")],
    };

    let mut findings = Vec::new();
    if !analysis.per_channel {
        findings.push(
            "Per-channel audio levels unavailable, the level below is one aggregate over all channels"
                .to_string(),
        );
    }
    for channel in &analysis.channels {
        if channel.peak_dbfs >= clipping_threshold {
            findings.push(format!(
                "Audio clipping: channel {} peak {:.1} dBFS",
                channel.channel, channel.peak_dbfs
            ));
        }
    }
    findings
}

struct DetectedRun {
    start_seconds: f64,
    // none when the run lasted to the end of the file
    end_seconds: Option<f64>,
}

impl DetectedRun {
    fn describe(&self) -> String {
        match self.end_seconds {
            Some(end) => format!("from {:.2} s to {end:.2} s", self.start_seconds),
            None => format!("from {:.2} s to the end", self.start_seconds),
        }
    }
}

// blackdetect puts both ends on one line, freezedetect and silencedetect use one each
fn intervals(stderr: &str, start_key: &str, end_key: &str) -> Vec<DetectedRun> {
    let mut runs = Vec::new();
    let mut open: Option<f64> = None;

    for line in stderr.lines() {
        if let Some(start) = value_after(line, start_key) {
            open = Some(start);
        }
        if let Some(end) = value_after(line, end_key)
            && let Some(start) = open.take()
        {
            runs.push(DetectedRun {
                start_seconds: start,
                end_seconds: Some(end),
            });
        }
    }

    if let Some(start) = open {
        runs.push(DetectedRun {
            start_seconds: start,
            end_seconds: None,
        });
    }

    runs
}

fn value_after(line: &str, key: &str) -> Option<f64> {
    let position = line.find(key)?;
    let rest = line[position + key.len()..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let number: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '+')
        .collect();
    number.parse().ok()
}

// detection filters report on stderr, and only a full decode reports at all
fn run_filter(input: &Path, filter_args: &[&str]) -> Result<String, String> {
    let output = std::process::Command::new("ffmpeg")
        .arg("-i")
        .arg(crate::studio::ffmpeg_path_argument(input))
        .args(filter_args)
        .args(["-f", "null", "-"])
        .output()
        .map_err(|e| format!("failed to run ffmpeg: {e}"))?;

    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        let tail = stderr.trim().lines().last().unwrap_or("unknown error");
        return Err(format!("ffmpeg exited with an error: {tail}"));
    }
    Ok(stderr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track_fixtures::{SoundStretch, write_sound_track};

    // the thresholds the auto-qc subcommand defaults to
    fn options(video: Option<PathBuf>, audio: Option<PathBuf>) -> AutoQcOptions {
        AutoQcOptions {
            video,
            audio,
            black_threshold: 0.10,
            freeze_threshold: 0.001,
            silence_threshold: -60.0,
            clipping_threshold: -0.5,
        }
    }

    #[test]
    fn a_silent_stretch_inside_a_sound_track_is_named_by_the_second_it_starts() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sound.mxf");
        write_sound_track(
            &path,
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

        let result = run(&options(None, Some(path)));

        assert_eq!(
            result.findings,
            vec!["Audio silence from 1.00 s to 2.00 s".to_string()]
        );
    }

    #[test]
    fn a_full_scale_channel_is_reported_as_clipping() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sound.mxf");
        write_sound_track(
            &path,
            &[SoundStretch {
                seconds: 1.0,
                amplitude: 1.0,
            }],
        );

        let result = run(&options(None, Some(path.clone())));
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.starts_with("Audio clipping: channel 1 peak")),
            "{:?}",
            result.findings
        );

        let mut quiet_enough = options(None, Some(path));
        quiet_enough.clipping_threshold = 6.0;
        assert!(
            run(&quiet_enough).pass(),
            "a threshold above full scale must clear the finding"
        );
    }

    #[test]
    fn a_sound_track_at_a_steady_level_reports_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sound.mxf");
        write_sound_track(
            &path,
            &[SoundStretch {
                seconds: 2.0,
                amplitude: 0.5,
            }],
        );

        let result = run(&options(None, Some(path)));

        assert!(result.pass(), "{:?}", result.findings);
    }

    #[test]
    fn a_blackdetect_line_gives_one_run_with_both_ends() {
        let stderr =
            "[blackdetect @ 0x55] black_start:1.04167 black_end:1.54167 black_duration:0.5";
        let runs = intervals(stderr, "black_start", "black_end");
        assert_eq!(runs.len(), 1);
        assert!((runs[0].start_seconds - 1.04167).abs() < 1e-5);
        assert_eq!(runs[0].end_seconds, Some(1.54167));
    }

    #[test]
    fn a_silence_run_that_never_ends_is_reported_to_the_end() {
        let stderr = "[silencedetect @ 0x55] silence_start: 2.5";
        let runs = intervals(stderr, "silence_start", "silence_end");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].end_seconds, None);
        assert_eq!(runs[0].describe(), "from 2.50 s to the end");
    }

    #[test]
    fn the_silence_end_line_carries_a_duration_that_is_not_the_end_time() {
        let stderr = "\
[silencedetect @ 0x55] silence_start: 1
[silencedetect @ 0x55] silence_end: 2 | silence_duration: 1";
        let runs = intervals(stderr, "silence_start", "silence_end");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].end_seconds, Some(2.0));
    }
}
