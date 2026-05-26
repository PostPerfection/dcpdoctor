#include <spdlog/spdlog.h>
#include <array>
#include <cstdio>
#include <fstream>
#include <regex>
#include <sstream>

#include "dcpdoctor/hdr_validate.h"
#include "dcpdoctor/platform.h"

namespace dcpdoctor
{

static std::string exec_hdr(const std::string& cmd)
{
  std::array<char, 4096> buffer;
  std::string result;
  FILE* pipe = DCPDOCTOR_POPEN(cmd.c_str(), "r");
  if(!pipe)
    return {};
  while(fgets(buffer.data(), buffer.size(), pipe))
    result += buffer.data();
  DCPDOCTOR_PCLOSE(pipe);
  return result;
}

static std::string transfer_str(TransferFunction tf)
{
  switch(tf)
  {
    case TransferFunction::PQ: return "PQ (ST 2084)";
    case TransferFunction::HLG: return "HLG (ARIB STD-B67)";
    case TransferFunction::SDR_BT1886: return "SDR (BT.1886)";
    case TransferFunction::Linear: return "Linear";
  }
  return "Unknown";
}

static std::string colorimetry_str(Colorimetry c)
{
  switch(c)
  {
    case Colorimetry::BT709: return "BT.709";
    case Colorimetry::BT2020: return "BT.2020";
    case Colorimetry::P3D65: return "P3-D65";
    case Colorimetry::P3DCI: return "P3-DCI";
    case Colorimetry::ACES: return "ACES";
  }
  return "Unknown";
}

HdrValidateResult validate_hdr_metadata(const HdrValidateOptions& opts)
{
  HdrValidateResult result;

  if(opts.video_path.empty() || !std::filesystem::exists(opts.video_path))
  {
    result.error = "Video file not found";
    return result;
  }

  // Probe HDR metadata using ffprobe
  std::ostringstream cmd;
  cmd << "ffprobe -v error -select_streams v:0 "
      << "-show_frames -read_intervals \"%+1\" "
      << "-show_entries frame=color_space,color_primaries,color_transfer,"
      << "side_data_list "
      << "-of json \"" << opts.video_path.string() << "\" 2>&1";

  auto output = exec_hdr(cmd.str());

  // Also get stream-level metadata
  std::ostringstream cmd2;
  cmd2 << "ffprobe -v error -select_streams v:0 "
       << "-show_entries stream=color_space,color_primaries,color_transfer,"
       << "bits_per_raw_sample,width,height "
       << "-of json \"" << opts.video_path.string() << "\" 2>&1";

  auto stream_output = exec_hdr(cmd2.str());

  // Parse transfer characteristics
  TransferFunction detected_tf = TransferFunction::SDR_BT1886;
  if(stream_output.find("smpte2084") != std::string::npos ||
     stream_output.find("smpte-st-2084") != std::string::npos)
    detected_tf = TransferFunction::PQ;
  else if(stream_output.find("arib-std-b67") != std::string::npos)
    detected_tf = TransferFunction::HLG;
  result.detected.transfer = detected_tf;

  // Parse color primaries
  Colorimetry detected_color = Colorimetry::BT709;
  if(stream_output.find("bt2020") != std::string::npos)
    detected_color = Colorimetry::BT2020;
  else if(stream_output.find("smpte432") != std::string::npos ||
          stream_output.find("p3") != std::string::npos)
    detected_color = Colorimetry::P3D65;
  result.detected.colorimetry = detected_color;

  // Parse bit depth
  std::regex bits_re(R"re("bits_per_raw_sample"\s*:\s*"(\d+)")re");
  std::smatch m;
  if(std::regex_search(stream_output, m, bits_re))
    result.detected.bit_depth = static_cast<uint16_t>(std::stoul(m[1].str()));

  // Parse MaxCLL/MaxFALL from side data
  std::regex cll_re(R"re(max_content\s*:\s*(\d+))re");
  std::regex fall_re(R"re(max_average\s*:\s*(\d+))re");
  if(std::regex_search(output, m, cll_re))
  {
    ContentLightLevel cll;
    cll.max_cll = static_cast<uint16_t>(std::stoul(m[1].str()));
    if(std::regex_search(output, m, fall_re))
      cll.max_fall = static_cast<uint16_t>(std::stoul(m[1].str()));
    result.detected.content_light = cll;
  }

  // Parse mastering display metadata
  std::regex master_re(R"re(mastering_display.*?min_luminance=(\d+).*?max_luminance=(\d+))re");
  if(std::regex_search(output, m, master_re))
  {
    MasteringDisplay md;
    md.min_luminance = static_cast<uint32_t>(std::stoul(m[1].str()));
    md.max_luminance = static_cast<uint32_t>(std::stoul(m[2].str()));
    result.detected.mastering_display = md;
  }

  // Now validate against expected spec
  result.valid = true;

  // Check transfer function
  if(detected_tf != opts.expected_transfer)
  {
    HdrIssue issue;
    issue.field = "Transfer Function";
    issue.expected = transfer_str(opts.expected_transfer);
    issue.actual = transfer_str(detected_tf);
    issue.severity = "error";
    issue.description = "Transfer function mismatch";
    result.issues.push_back(issue);
    result.valid = false;
  }

  // Check colorimetry
  if(detected_color != opts.expected_colorimetry)
  {
    HdrIssue issue;
    issue.field = "Color Primaries";
    issue.expected = colorimetry_str(opts.expected_colorimetry);
    issue.actual = colorimetry_str(detected_color);
    issue.severity = "error";
    issue.description = "Color primaries mismatch";
    result.issues.push_back(issue);
    result.valid = false;
  }

  // Check bit depth
  if(result.detected.bit_depth > 0 && result.detected.bit_depth < opts.expected_bit_depth)
  {
    HdrIssue issue;
    issue.field = "Bit Depth";
    issue.expected = std::to_string(opts.expected_bit_depth);
    issue.actual = std::to_string(result.detected.bit_depth);
    issue.severity = "error";
    issue.description = "Bit depth below requirement";
    result.issues.push_back(issue);
    result.valid = false;
  }

  // Check MaxCLL if specified
  if(opts.expected_max_cll > 0 && result.detected.content_light)
  {
    if(result.detected.content_light->max_cll > opts.expected_max_cll)
    {
      HdrIssue issue;
      issue.field = "MaxCLL";
      issue.expected = "≤ " + std::to_string(opts.expected_max_cll) + " nits";
      issue.actual = std::to_string(result.detected.content_light->max_cll) + " nits";
      issue.severity = "warning";
      issue.description = "MaxCLL exceeds expected limit";
      result.issues.push_back(issue);
    }
  }

  // Check mastering display max luminance
  if(opts.expected_max_luminance > 0 && result.detected.mastering_display)
  {
    if(result.detected.mastering_display->max_luminance != opts.expected_max_luminance)
    {
      HdrIssue issue;
      issue.field = "Mastering Display Max Luminance";
      issue.expected = std::to_string(opts.expected_max_luminance) + " nits";
      issue.actual = std::to_string(result.detected.mastering_display->max_luminance) + " nits";
      issue.severity = "warning";
      issue.description = "Mastering display luminance does not match expected value";
      result.issues.push_back(issue);
    }
  }

  result.success = true;
  spdlog::info("HDR validation: {} ({})", result.valid ? "PASS" : "FAIL", result.issues.size());
  return result;
}

HdrValidateResult validate_cpl_hdr(const std::filesystem::path& cpl,
                                   const std::filesystem::path& video)
{
  HdrValidateResult result;
  namespace fs = std::filesystem;

  if(!fs::exists(cpl))
  {
    result.error = "CPL file not found: " + cpl.string();
    return result;
  }
  if(!fs::exists(video))
  {
    result.error = "Video MXF not found: " + video.string();
    return result;
  }

  // Read CPL for any color-related metadata
  std::ifstream cpl_file(cpl);
  std::string cpl_content((std::istreambuf_iterator<char>(cpl_file)),
                          std::istreambuf_iterator<char>());

  // Check for TransferCharacteristic in CPL (ST 2067-21 uses these)
  TransferFunction cpl_transfer = TransferFunction::SDR_BT1886;
  if(cpl_content.find("SMPTE-ST-2084") != std::string::npos ||
     cpl_content.find("ST2084") != std::string::npos)
    cpl_transfer = TransferFunction::PQ;
  else if(cpl_content.find("ARIB-STD-B67") != std::string::npos ||
          cpl_content.find("HLG") != std::string::npos)
    cpl_transfer = TransferFunction::HLG;

  // Check for ColorPrimaries
  Colorimetry cpl_color = Colorimetry::BT709;
  if(cpl_content.find("ITU-R-BT.2020") != std::string::npos ||
     cpl_content.find("BT.2020") != std::string::npos)
    cpl_color = Colorimetry::BT2020;
  else if(cpl_content.find("P3-D65") != std::string::npos ||
          cpl_content.find("SMPTE-RP-431") != std::string::npos)
    cpl_color = Colorimetry::P3D65;
  else if(cpl_content.find("P3-DCI") != std::string::npos)
    cpl_color = Colorimetry::P3DCI;

  // Probe the actual video file
  HdrValidateOptions opts;
  opts.video_path = video;
  opts.expected_transfer = cpl_transfer;
  opts.expected_colorimetry = cpl_color;

  auto video_result = validate_hdr_metadata(opts);

  // Transfer the detected metadata
  result.detected = video_result.detected;
  result.issues = video_result.issues;
  result.valid = video_result.valid;
  result.success = video_result.success;

  // Additional cross-check: if CPL declares no HDR but video has HDR
  if(cpl_transfer == TransferFunction::SDR_BT1886 &&
     result.detected.transfer != TransferFunction::SDR_BT1886)
  {
    HdrIssue issue;
    issue.field = "CPL Transfer Function";
    issue.expected = "SDR (no HDR metadata in CPL)";
    issue.actual = transfer_str(result.detected.transfer) + " (detected in MXF)";
    issue.severity = "warning";
    issue.description = "Video contains HDR metadata but CPL does not declare it";
    result.issues.push_back(issue);
  }

  // Cross-check: if CPL declares HDR but video is SDR
  if(cpl_transfer != TransferFunction::SDR_BT1886 &&
     result.detected.transfer == TransferFunction::SDR_BT1886)
  {
    HdrIssue issue;
    issue.field = "CPL Transfer Function";
    issue.expected = transfer_str(cpl_transfer) + " (declared in CPL)";
    issue.actual = "SDR (no HDR in MXF)";
    issue.severity = "error";
    issue.description = "CPL declares HDR but video does not contain HDR metadata";
    result.issues.push_back(issue);
    result.valid = false;
  }

  spdlog::info("CPL HDR cross-validation: {} ({} issues)",
               result.valid ? "PASS" : "FAIL", result.issues.size());
  return result;
}

} // namespace dcpdoctor
