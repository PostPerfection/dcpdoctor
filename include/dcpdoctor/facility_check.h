#pragma once

#include "dcpdoctor/dcpdoctor.h"
#include <filesystem>
#include <string>
#include <vector>

namespace dcpdoctor
{

/// Individual check result in the facility readiness report
struct CheckItem
{
  std::string category; // e.g. "structure", "compliance", "naming", "audio"
  std::string check_name; // e.g. "ASSETMAP present"
  bool passed = false;
  std::string detail; // explanation if failed
  std::string severity; // "error", "warning", "info"
};

/// Overall facility readiness result
struct FacilityCheckResult
{
  std::vector<CheckItem> items;
  bool ready = false; // true only if zero errors
  int errors = 0;
  int warnings = 0;
  int info_count = 0;
  int checks_passed = 0;
  int checks_total = 0;
  std::string summary; // Human-readable one-line summary
  std::string error; // Fatal error (directory not found, etc.)
};

struct FacilityCheckOptions
{
  std::filesystem::path dcp_dir;
  Standard expected_standard = Standard::smpte; // smpte or interop
  bool strict = true; // strict SMPTE compliance
  bool check_hashes = true; // verify file hashes
  bool check_bitrate = true; // validate J2K bitrate limits
  bool check_audio = true; // validate audio levels
  bool check_naming = true; // validate ISDCF naming
  bool check_subtitles = true; // validate subtitle compliance
};

/// Run comprehensive facility readiness check on a DCP
/// This is a one-stop "is this DCP ready to ship?" function
FacilityCheckResult run_facility_check(const FacilityCheckOptions& opts);

/// Output facility check result as JSON
std::string facility_check_to_json(const FacilityCheckResult& result);

} // namespace dcpdoctor
