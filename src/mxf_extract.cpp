#include <spdlog/spdlog.h>
#include <cstdio>
#include <filesystem>

#include "dcpdoctor/mxf_extract.h"
#include "dcpdoctor/mxf.h"
#include "dcpdoctor/platform.h"

namespace fs = std::filesystem;

namespace dcpdoctor
{

MxfExtractResult extract_mxf(const MxfExtractOptions& opts)
{
  MxfExtractResult result;

  if(!fs::exists(opts.input))
  {
    result.error = "MXF file not found: " + opts.input.string();
    return result;
  }

  fs::create_directories(opts.output_dir);

  // Probe the MXF to determine essence type
  auto info = read_mxf_info(opts.input);
  if(!info.valid)
  {
    result.error = "Failed to read MXF: " + info.error;
    return result;
  }

  std::string input_escaped = opts.input.string();

  if(info.picture && opts.extract_video)
  {
    fs::path out_path = opts.output_dir / (opts.input.stem().string() + "_video.mxf");
    std::string cmd = "ffmpeg -y -i \"" + input_escaped + "\"";

    if(opts.start_frame > 0 && info.picture->frame_rate_num > 0)
    {
      double start_sec = static_cast<double>(opts.start_frame) * info.picture->frame_rate_den /
                         info.picture->frame_rate_num;
      cmd += " -ss " + std::to_string(start_sec);
    }
    if(opts.end_frame > 0 && opts.end_frame > opts.start_frame && info.picture->frame_rate_num > 0)
    {
      uint32_t count = opts.end_frame - opts.start_frame;
      cmd += " -frames:v " + std::to_string(count);
    }

    cmd += " -map 0:v -c copy \"" + out_path.string() + "\" 2>/dev/null";

    FILE* pipe = DCPDOCTOR_POPEN(cmd.c_str(), "r");
    if(pipe)
    {
      char buf[256];
      while(fgets(buf, sizeof(buf), pipe))
        ;
      int ret = DCPDOCTOR_PCLOSE(pipe);
      if(ret == 0 && fs::exists(out_path))
      {
        result.extracted_files.push_back(out_path);
        if(opts.end_frame > opts.start_frame)
          result.frames_extracted = opts.end_frame - opts.start_frame;
        else if(info.picture->frame_count > 0)
          result.frames_extracted = static_cast<uint32_t>(info.picture->frame_count);
      }
      else
      {
        spdlog::warn("Video extraction failed for {}", opts.input.string());
      }
    }
  }

  if(info.sound && opts.extract_audio)
  {
    fs::path out_path = opts.output_dir / (opts.input.stem().string() + "_audio.wav");
    std::string cmd = "ffmpeg -y -i \"" + input_escaped + "\" -map 0:a -c pcm_s24le \"" +
                      out_path.string() + "\" 2>/dev/null";

    FILE* pipe = DCPDOCTOR_POPEN(cmd.c_str(), "r");
    if(pipe)
    {
      char buf[256];
      while(fgets(buf, sizeof(buf), pipe))
        ;
      int ret = DCPDOCTOR_PCLOSE(pipe);
      if(ret == 0 && fs::exists(out_path))
        result.extracted_files.push_back(out_path);
      else
        spdlog::warn("Audio extraction failed for {}", opts.input.string());
    }
  }

  result.success = !result.extracted_files.empty();
  if(!result.success && result.error.empty())
    result.error = "No essence extracted from " + opts.input.string();

  return result;
}

} // namespace dcpdoctor
