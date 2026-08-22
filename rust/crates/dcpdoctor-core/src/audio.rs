/// Audio analysis: level analysis, clipping detection, silence detection.
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Per-channel audio analysis results.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AudioAnalysis {
    pub channels: Vec<ChannelInfo>,
    pub sample_rate: u32,
    pub duration_seconds: f64,
    /// false when astats gave nothing and the levels below are one aggregate
    /// figure over all channels rather than per channel
    pub per_channel: bool,
}

/// Analysis for a single audio channel.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelInfo {
    pub channel: u32,
    pub peak_dbfs: f64,
    pub rms_dbfs: f64,
    pub clipping: bool,
    pub silent: bool,
}

/// EBU R128 loudness measurement result.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoudnessResult {
    pub integrated_lufs: f64,
    pub true_peak_dbtp: f64,
    pub loudness_range_lu: f64,
    pub short_term_max_lufs: f64,
}

/// Analyze audio levels in an MXF or WAV file using ffmpeg.
pub fn analyze_audio(audio_path: &Path) -> Result<AudioAnalysis, String> {
    if !audio_path.exists() {
        return Err(format!("File not found: {}", audio_path.display()));
    }

    // Use ffmpeg astats filter for per-channel peak and RMS
    let output = std::process::Command::new("ffmpeg")
        .arg("-i")
        .arg(audio_path)
        .arg("-af")
        .arg("astats=metadata=1:reset=0")
        .arg("-f")
        .arg("null")
        .arg("-")
        .output()
        .map_err(|e| format!("Failed to run ffmpeg: {e}"))?;

    let stderr = String::from_utf8_lossy(&output.stderr);

    let mut analysis = AudioAnalysis::default();

    // Parse channel count and sample rate from stream info
    for line in stderr.lines() {
        if line.contains("Audio:")
            && let Some(hz_pos) = line.find(" Hz")
        {
            let before = &line[..hz_pos];
            if let Some(last_space) = before.rfind(|c: char| !c.is_ascii_digit())
                && let Ok(sr) = before[last_space + 1..].parse::<u32>()
            {
                analysis.sample_rate = sr;
            }
        }
        if line.contains("Duration:")
            && let Some(dur) = parse_ffmpeg_duration(line)
        {
            analysis.duration_seconds = dur;
        }
    }

    // Parse astats output
    let mut current_channel: Option<u32> = None;
    let mut channels: std::collections::HashMap<u32, ChannelInfo> =
        std::collections::HashMap::new();

    for line in stderr.lines() {
        let trimmed = line.trim();

        // Detect channel headers like "Channel: 1" or "[Parsed_astats...] Channel: 1"
        if trimmed.contains("Overall") {
            current_channel = None;
            continue;
        }
        if let Some(ch_str) = trimmed.strip_suffix("").and_then(|_| {
            if trimmed.contains("Channel:") {
                let parts: Vec<&str> = trimmed.split("Channel:").collect();
                parts
                    .get(1)
                    .and_then(|s| s.split_whitespace().next()?.parse::<u32>().ok())
            } else {
                None
            }
        }) {
            current_channel = Some(ch_str);
        } else if trimmed.contains("Channel:")
            && let Some(num) = trimmed
                .split("Channel:")
                .nth(1)
                .and_then(|s| s.split_whitespace().next())
                .and_then(|s| s.parse::<u32>().ok())
        {
            current_channel = Some(num);
        }

        if let Some(ch) = current_channel {
            let info = channels.entry(ch).or_insert_with(|| ChannelInfo {
                channel: ch,
                ..Default::default()
            });

            if (trimmed.contains("Peak level dB:") || trimmed.contains("Peak_level"))
                && let Some(val) = extract_db_value(trimmed)
            {
                info.peak_dbfs = val;
                info.clipping = val >= -0.5;
            }
            if (trimmed.contains("RMS level dB:") || trimmed.contains("RMS_level"))
                && let Some(val) = extract_db_value(trimmed)
            {
                info.rms_dbfs = val;
                info.silent = val < -80.0;
            }
        }
    }

    // If astats parsing failed, try with volumedetect filter per channel
    if channels.is_empty() {
        return analyze_audio_volumedetect(audio_path);
    }

    let mut sorted: Vec<ChannelInfo> = channels.into_values().collect();
    sorted.sort_by_key(|c| c.channel);
    analysis.channels = sorted;
    analysis.per_channel = true;

    Ok(analysis)
}

/// Fallback: analyze using volumedetect filter.
fn analyze_audio_volumedetect(audio_path: &Path) -> Result<AudioAnalysis, String> {
    let output = std::process::Command::new("ffmpeg")
        .arg("-i")
        .arg(audio_path)
        .arg("-af")
        .arg("volumedetect")
        .arg("-f")
        .arg("null")
        .arg("-")
        .output()
        .map_err(|e| format!("Failed to run ffmpeg: {e}"))?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut analysis = AudioAnalysis::default();
    let mut info = ChannelInfo::default();
    let mut parsed_level = false;

    for line in stderr.lines() {
        let trimmed = line.trim();
        if trimmed.contains("max_volume:")
            && let Some(val) = extract_db_value(trimmed)
        {
            info.peak_dbfs = val;
            info.clipping = val >= -0.5;
            parsed_level = true;
        }
        if trimmed.contains("mean_volume:")
            && let Some(val) = extract_db_value(trimmed)
        {
            info.rms_dbfs = val;
            info.silent = val < -80.0;
            parsed_level = true;
        }
        if line.contains("Duration:")
            && let Some(dur) = parse_ffmpeg_duration(line)
        {
            analysis.duration_seconds = dur;
        }
        if line.contains("Audio:")
            && let Some(hz_pos) = line.find(" Hz")
        {
            let before = &line[..hz_pos];
            if let Some(last_space) = before.rfind(|c: char| !c.is_ascii_digit())
                && let Ok(sr) = before[last_space + 1..].parse::<u32>()
            {
                analysis.sample_rate = sr;
            }
        }
    }

    // an all-zero ChannelInfo would read as a 0.0 dBFS peak, so nothing parsed
    // has to be an error
    if !parsed_level {
        return Err(format!(
            "ffmpeg produced neither astats nor volumedetect levels for {}",
            audio_path.display()
        ));
    }

    analysis.channels = vec![info];
    Ok(analysis)
}

/// Measure loudness using EBU R128 (ffmpeg ebur128 filter).
pub fn measure_loudness(audio_path: &Path) -> Result<LoudnessResult, String> {
    if !audio_path.exists() {
        return Err(format!("File not found: {}", audio_path.display()));
    }

    let output = std::process::Command::new("ffmpeg")
        .arg("-i")
        .arg(audio_path)
        .arg("-af")
        .arg("ebur128=peak=true")
        .arg("-f")
        .arg("null")
        .arg("-")
        .output()
        .map_err(|e| format!("Failed to run ffmpeg: {e}"))?;

    let stderr = String::from_utf8_lossy(&output.stderr);

    let integrated = parse_loudness_value(&stderr, "I:")
        .or_else(|| parse_loudness_value(&stderr, "Integrated loudness:"))
        .unwrap_or(f64::NAN);
    let true_peak = parse_loudness_value(&stderr, "Peak:")
        .or_else(|| parse_loudness_value(&stderr, "True peak:"))
        .unwrap_or(f64::NAN);
    let lra = parse_loudness_value(&stderr, "LRA:")
        .or_else(|| parse_loudness_value(&stderr, "Loudness range:"))
        .unwrap_or(f64::NAN);

    // nothing parsed means ffmpeg failed to open the file or lacks ebur128; don't report NaN as success
    if integrated.is_nan() && true_peak.is_nan() && lra.is_nan() {
        return Err("Failed to measure loudness (ffmpeg produced no ebur128 output)".to_string());
    }

    Ok(LoudnessResult {
        integrated_lufs: integrated,
        true_peak_dbtp: true_peak,
        loudness_range_lu: lra,
        short_term_max_lufs: f64::NAN,
    })
}

fn extract_db_value(s: &str) -> Option<f64> {
    // astats and volumedetect print -inf for a silent channel, and a value that
    // does not parse would be read back as a 0.0 dBFS peak
    if let Some(colon) = s.rfind(':') {
        let after = s[colon + 1..].trim();
        if let Some(rest) = after.strip_prefix('-')
            && rest.starts_with("inf")
        {
            return Some(f64::NEG_INFINITY);
        }
        if after.starts_with("inf") {
            return Some(f64::INFINITY);
        }
    }

    // Find a floating point number (possibly negative) followed by dB or just in the value position
    let mut found_num = false;
    let mut num_str = String::new();

    for ch in s.chars().rev() {
        if ch == 'B' || ch == 'b' || ch == 'd' || ch == 'D' {
            continue;
        }
        if ch.is_ascii_digit() || ch == '.' || ch == '-' {
            num_str.insert(0, ch);
            found_num = true;
        } else if found_num {
            break;
        }
    }

    // Try from the colon position instead
    if (num_str.is_empty() || num_str.parse::<f64>().is_err())
        && let Some(colon) = s.rfind(':')
    {
        let after = s[colon + 1..].trim();
        let val_str: String = after
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == ' ')
            .collect();
        return val_str.trim().parse::<f64>().ok();
    }

    num_str.parse::<f64>().ok()
}

fn parse_ffmpeg_duration(line: &str) -> Option<f64> {
    // Duration: HH:MM:SS.xx
    if let Some(pos) = line.find("Duration:") {
        let after = &line[pos + 9..];
        let dur_str: String = after
            .trim()
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == ':' || *c == '.')
            .collect();
        let parts: Vec<&str> = dur_str.split(':').collect();
        if parts.len() == 3 {
            let h: f64 = parts[0].parse().ok()?;
            let m: f64 = parts[1].parse().ok()?;
            let s: f64 = parts[2].parse().ok()?;
            return Some(h * 3600.0 + m * 60.0 + s);
        }
    }
    None
}

fn parse_loudness_value(output: &str, key: &str) -> Option<f64> {
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(pos) = trimmed.find(key) {
            let after = &trimmed[pos + key.len()..];
            let num_str: String = after
                .trim()
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
                .collect();
            if let Ok(val) = num_str.parse::<f64>() {
                return Some(val);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_silent_channel_reads_as_minus_infinity_not_zero_dbfs() {
        assert_eq!(
            extract_db_value("[Parsed_astats_0 @ 0x0] Peak level dB: -inf"),
            Some(f64::NEG_INFINITY)
        );
        assert_eq!(
            extract_db_value("[Parsed_astats_0 @ 0x0] Peak level dB: -18.061535"),
            Some(-18.061535)
        );
    }

    #[test]
    fn a_file_ffmpeg_cannot_read_is_an_error_not_a_zero_peak() {
        let dir = tempfile::tempdir().unwrap();
        let not_audio = dir.path().join("sound.wav");
        std::fs::write(&not_audio, b"this is not a wav").unwrap();

        let result = analyze_audio(&not_audio);
        assert!(
            result.is_err(),
            "no levels parsed must not come back as a 0.0 dBFS peak: {result:?}"
        );
    }
}
