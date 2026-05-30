#include <spdlog/spdlog.h>
#include <cmath>
#include <cstdio>
#include <filesystem>
#include <sstream>

#include "dcpdoctor/av_sync.h"
#include "dcpdoctor/platform.h"

namespace fs = std::filesystem;

namespace dcpdoctor
{

AvSyncResult detect_av_sync(const AvSyncOptions& opts)
{
  AvSyncResult result;

  if(!fs::exists(opts.video_file))
  {
    result.error = "Video file not found: " + opts.video_file.string();
    return result;
  }
  if(!fs::exists(opts.audio_file))
  {
    result.error = "Audio file not found: " + opts.audio_file.string();
    return result;
  }

  // Use ffprobe to get video duration in frames and audio duration in samples
  double video_duration = 0;
  double audio_duration = 0;

  // Get video duration
  std::string vcmd = "ffprobe -v quiet -show_entries format=duration -of csv=p=0 \"" +
                     opts.video_file.string() + "\"";
  FILE* vpipe = DCPDOCTOR_POPEN(vcmd.c_str(), "r");
  if(vpipe)
  {
    char buf[256];
    if(fgets(buf, sizeof(buf), vpipe))
      video_duration = std::stod(buf);
    DCPDOCTOR_PCLOSE(vpipe);
  }

  // Get audio duration
  std::string acmd = "ffprobe -v quiet -show_entries format=duration -of csv=p=0 \"" +
                     opts.audio_file.string() + "\"";
  FILE* apipe = DCPDOCTOR_POPEN(acmd.c_str(), "r");
  if(apipe)
  {
    char buf[256];
    if(fgets(buf, sizeof(buf), apipe))
      audio_duration = std::stod(buf);
    DCPDOCTOR_PCLOSE(apipe);
  }

  if(video_duration <= 0 || audio_duration <= 0)
  {
    result.error = "Failed to determine duration of video or audio";
    return result;
  }

  // Calculate drift
  double drift_seconds = audio_duration - video_duration;
  result.drift_ms = drift_seconds * 1000.0;
  result.drift_samples = static_cast<int32_t>(drift_seconds * opts.sample_rate);

  double frame_duration = static_cast<double>(opts.fps_den) / static_cast<double>(opts.fps_num);
  result.drift_frames = drift_seconds / frame_duration;

  // Within ±1 frame = in sync
  result.in_sync = (std::abs(result.drift_frames) < 1.0);

  if(result.drift_samples > 0)
  {
    result.recommendation = "Audio is " + std::to_string(std::abs(result.drift_samples)) +
                            " samples longer than video. Trim " +
                            std::to_string(std::abs(result.drift_samples)) +
                            " samples from audio tail.";
  }
  else if(result.drift_samples < 0)
  {
    result.recommendation = "Audio is " + std::to_string(std::abs(result.drift_samples)) +
                            " samples shorter than video. Pad " +
                            std::to_string(std::abs(result.drift_samples)) +
                            " samples of silence at audio tail.";
  }
  else
  {
    result.recommendation = "Audio and video are perfectly in sync.";
  }

  result.success = true;
  return result;
}

AvSyncFixResult fix_av_sync(const AvSyncFixOptions& opts)
{
  AvSyncFixResult result;

  if(!fs::exists(opts.audio_file))
  {
    result.error = "Audio file not found: " + opts.audio_file.string();
    return result;
  }

  std::string cmd;
  if(opts.trim_samples > 0)
  {
    // Trim from start
    double trim_seconds = static_cast<double>(opts.trim_samples) / opts.sample_rate;
    cmd = "ffmpeg -y -i \"" + opts.audio_file.string() + "\" -ss " + std::to_string(trim_seconds) +
          " -c:a pcm_s" + std::to_string(opts.bit_depth) + "le -ar " +
          std::to_string(opts.sample_rate) + " \"" + opts.output_file.string() + "\" 2>/dev/null";
  }
  else if(opts.trim_samples < 0)
  {
    // Pad start with silence
    double pad_seconds = static_cast<double>(-opts.trim_samples) / opts.sample_rate;
    cmd = "ffmpeg -y -f lavfi -t " + std::to_string(pad_seconds) +
          " -i anullsrc=r=" + std::to_string(opts.sample_rate) + " -i \"" +
          opts.audio_file.string() +
          "\" -filter_complex \"[0:a][1:a]concat=n=2:v=0:a=1\" "
          "-c:a pcm_s" +
          std::to_string(opts.bit_depth) + "le \"" + opts.output_file.string() + "\" 2>/dev/null";
  }
  else
  {
    // No adjustment needed — just copy
    fs::copy_file(opts.audio_file, opts.output_file, fs::copy_options::overwrite_existing);
    result.output_file = opts.output_file;
    result.samples_adjusted = 0;
    result.success = true;
    return result;
  }

  int ret = system(cmd.c_str());
  if(ret != 0)
  {
    result.error = "ffmpeg A/V sync fix failed";
    return result;
  }

  result.output_file = opts.output_file;
  result.samples_adjusted = opts.trim_samples;
  result.success = true;
  return result;
}

} // namespace dcpdoctor
