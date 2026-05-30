#pragma once

#include "dcpdoctor/hdr.h"
#include <filesystem>
#include <string>
#include <vector>

namespace dcpdoctor
{

// HDR validation issue
struct HdrIssue
{
  std::string field;
  std::string expected;
  std::string actual;
  std::string severity; // "error", "warning"
  std::string description;
};

struct HdrValidateOptions
{
  std::filesystem::path video_path; // Video file to probe
  std::filesystem::path cpl_path; // CPL XML to check metadata in
  std::string target_spec; // "hdr10", "hlg", "dolby_vision", "hdr10plus"

  // Expected values for validation
  uint16_t expected_max_cll = 0; // 0 = don't check
  uint16_t expected_max_fall = 0;
  uint32_t expected_max_luminance = 0; // mastering display
  TransferFunction expected_transfer = TransferFunction::PQ;
  Colorimetry expected_colorimetry = Colorimetry::BT2020;
  uint16_t expected_bit_depth = 10;
};

struct HdrValidateResult
{
  std::vector<HdrIssue> issues;
  ImfHdrMetadata detected; // What was found in the file
  bool valid = false;
  bool success = false;
  std::string error;
};

// Validate HDR metadata from a video file against expected spec
HdrValidateResult validate_hdr_metadata(const HdrValidateOptions& opts);

// Check if CPL color metadata matches the actual video content
HdrValidateResult validate_cpl_hdr(const std::filesystem::path& cpl,
                                   const std::filesystem::path& video);

} // namespace dcpdoctor
