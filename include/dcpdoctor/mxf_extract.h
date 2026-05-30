#pragma once

#include <cstdint>
#include <filesystem>
#include <string>
#include <vector>

namespace dcpdoctor
{

struct MxfExtractOptions
{
  std::filesystem::path input; // MXF file
  std::filesystem::path output_dir; // Where to extract essences
  bool extract_video = true;
  bool extract_audio = true;
  uint32_t start_frame = 0; // 0 = from beginning
  uint32_t end_frame = 0; // 0 = to end
};

struct MxfExtractResult
{
  std::vector<std::filesystem::path> extracted_files;
  uint32_t frames_extracted = 0;
  bool success = false;
  std::string error;
};

/// Extract essence from an MXF container using ffmpeg
MxfExtractResult extract_mxf(const MxfExtractOptions& opts);

} // namespace dcpdoctor
