#include <spdlog/spdlog.h>
#include <cmath>
#include <cstdio>
#include <filesystem>

#include "dcpdoctor/imf_compliance.h"
#include "dcpdoctor/info.h"
#include "dcpdoctor/loudness.h"
#include "dcpdoctor/platform.h"

namespace fs = std::filesystem;

namespace dcpdoctor
{

std::string imf_compliance_target_name(ImfComplianceTarget target)
{
  switch(target)
  {
    case ImfComplianceTarget::Netflix:
      return "Netflix";
    case ImfComplianceTarget::Disney:
      return "Disney+";
    case ImfComplianceTarget::Amazon:
      return "Amazon";
    case ImfComplianceTarget::Apple:
      return "Apple TV+";
    case ImfComplianceTarget::Cinema2K:
      return "Cinema 2K";
    case ImfComplianceTarget::Cinema4K:
      return "Cinema 4K";
    case ImfComplianceTarget::BroadcastHD:
      return "Broadcast HD";
    case ImfComplianceTarget::BroadcastUHD:
      return "Broadcast UHD";
  }
  return "Unknown";
}

static void check_resolution(ImfComplianceResult& result, const ImpInfo& info, uint32_t max_w,
                             uint32_t max_h)
{
  ImfComplianceCheck check;
  check.rule = "resolution";
  check.description = "Video resolution within platform limits";
  check.expected_value = std::to_string(max_w) + "x" + std::to_string(max_h);

  for(auto& t : info.tracks)
  {
    if(t.type != "video")
      continue;
    auto video_path = fs::path(info.path) / t.filename;
    std::string cmd = "ffprobe -v quiet -select_streams v:0 -show_entries stream=width,height "
                      "-of csv=p=0 \"" +
                      video_path.string() + "\"";
    FILE* pipe = DCPDOCTOR_POPEN(cmd.c_str(), "r");
    if(pipe)
    {
      char buf[256];
      if(fgets(buf, sizeof(buf), pipe))
      {
        uint32_t w = 0, h = 0;
        sscanf(buf, "%u,%u", &w, &h);
        check.actual_value = std::to_string(w) + "x" + std::to_string(h);
        check.passed = (w <= max_w && h <= max_h);
      }
      DCPDOCTOR_PCLOSE(pipe);
    }
    break;
  }

  result.checks.push_back(check);
}

static void check_framerate(ImfComplianceResult& result, const ImpInfo& info,
                            const std::vector<double>& allowed_fps)
{
  ImfComplianceCheck check;
  check.rule = "framerate";
  check.description = "Frame rate is in allowed set";

  std::string allowed_str;
  for(size_t i = 0; i < allowed_fps.size(); ++i)
  {
    if(i > 0)
      allowed_str += ", ";
    allowed_str += std::to_string(allowed_fps[i]);
  }
  check.expected_value = allowed_str;

  for(auto& t : info.tracks)
  {
    if(t.type != "video")
      continue;
    auto video_path = fs::path(info.path) / t.filename;
    std::string cmd = "ffprobe -v quiet -select_streams v:0 -show_entries stream=r_frame_rate "
                      "-of csv=p=0 \"" +
                      video_path.string() + "\"";
    FILE* pipe = DCPDOCTOR_POPEN(cmd.c_str(), "r");
    if(pipe)
    {
      char buf[256];
      if(fgets(buf, sizeof(buf), pipe))
      {
        int num = 0, den = 1;
        if(sscanf(buf, "%d/%d", &num, &den) == 2 && den > 0)
        {
          double fps = static_cast<double>(num) / static_cast<double>(den);
          check.actual_value = std::to_string(fps) + " fps";

          // Check if fps matches any allowed value (within 0.01 tolerance)
          check.passed = false;
          for(double allowed : allowed_fps)
          {
            if(std::abs(fps - allowed) < 0.01)
            {
              check.passed = true;
              break;
            }
          }
        }
        else
        {
          check.actual_value = buf;
          check.passed = false;
        }
      }
      DCPDOCTOR_PCLOSE(pipe);
    }
    break;
  }

  result.checks.push_back(check);
}

static void check_audio_loudness(ImfComplianceResult& result, const ImpInfo& info,
                                 double target_lufs, double tolerance)
{
  ImfComplianceCheck check;
  check.rule = "loudness";
  check.description = "Audio loudness within target range";
  check.expected_value =
      std::to_string(target_lufs) + " LUFS ±" + std::to_string(tolerance) + " LU";

  for(auto& t : info.tracks)
  {
    if(t.type != "audio")
      continue;
    auto audio_path = fs::path(info.path) / t.filename;
    auto loud = measure_imf_loudness(audio_path);
    if(loud.success)
    {
      check.actual_value = std::to_string(loud.integrated_lufs) + " LUFS";
      check.passed = (loud.integrated_lufs >= target_lufs - tolerance &&
                      loud.integrated_lufs <= target_lufs + tolerance);
    }
    break;
  }

  result.checks.push_back(check);
}

ImfComplianceResult check_imf_compliance(const ImfComplianceOptions& opts)
{
  ImfComplianceResult result;
  result.target = opts.target;

  auto info = read_imp_info(opts.imp_dir);
  if(!info.valid)
  {
    result.error = "Cannot read IMP: " + info.error;
    return result;
  }

  switch(opts.target)
  {
    case ImfComplianceTarget::Netflix:
      check_resolution(result, info, 3840, 2160);
      check_framerate(result, info, {23.976, 24.0, 25.0, 29.97, 30.0, 50.0, 59.94, 60.0});
      check_audio_loudness(result, info, -27.0, 2.0);
      break;
    case ImfComplianceTarget::Disney:
      check_resolution(result, info, 3840, 2160);
      check_framerate(result, info, {23.976, 24.0, 25.0});
      check_audio_loudness(result, info, -27.0, 2.0);
      break;
    case ImfComplianceTarget::Amazon:
      check_resolution(result, info, 3840, 2160);
      check_framerate(result, info, {23.976, 24.0, 25.0, 29.97});
      check_audio_loudness(result, info, -24.0, 2.0);
      break;
    case ImfComplianceTarget::Apple:
      check_resolution(result, info, 3840, 2160);
      check_framerate(result, info, {23.976, 24.0, 25.0, 29.97});
      check_audio_loudness(result, info, -24.0, 2.0);
      break;
    case ImfComplianceTarget::Cinema2K:
      check_resolution(result, info, 2048, 1080);
      check_framerate(result, info, {24.0, 48.0});
      check_audio_loudness(result, info, -20.0, 5.0);
      break;
    case ImfComplianceTarget::Cinema4K:
      check_resolution(result, info, 4096, 2160);
      check_framerate(result, info, {24.0, 48.0});
      check_audio_loudness(result, info, -20.0, 5.0);
      break;
    case ImfComplianceTarget::BroadcastHD:
      check_resolution(result, info, 1920, 1080);
      check_framerate(result, info, {25.0, 29.97, 50.0, 59.94});
      check_audio_loudness(result, info, -23.0, 1.0);
      break;
    case ImfComplianceTarget::BroadcastUHD:
      check_resolution(result, info, 3840, 2160);
      check_framerate(result, info, {50.0, 59.94});
      check_audio_loudness(result, info, -23.0, 1.0);
      break;
  }

  for(auto& c : result.checks)
  {
    if(c.passed)
      result.passed++;
    else
      result.failed++;
  }

  result.compliant = (result.failed == 0);
  result.success = true;
  return result;
}

} // namespace dcpdoctor
