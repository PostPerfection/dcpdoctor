#pragma once

#include "dcpdoctor/dcpdoctor.h"
#include <filesystem>
#include <string>
#include <vector>

namespace dcpdoctor
{

/// DCI CTP test categories (based on DCI Compliance Test Plan v1.3)
enum class CtpCategory
{
  Picture, // 2K/4K J2K codec compliance
  Audio, // PCM audio format compliance
  Subtitles, // Timed text compliance
  Security, // AES-CBC encryption, KDM format
  Packaging, // MXF wrapping, ASSETMAP, PKL, VOLINDEX
  Composition, // CPL structure and constraints
  Presentation // Presentation requirements (markers, duration)
};

/// Individual CTP test result
struct CtpTestResult
{
  std::string test_id; // e.g. "CTP-PIC-001"
  CtpCategory category;
  std::string description;
  std::string requirement; // DCI spec requirement text
  bool passed = false;
  bool skipped = false; // e.g. no reference content available
  std::string detail;
};

/// Full CTP run result
struct CtpResult
{
  std::vector<CtpTestResult> results;
  int total = 0;
  int passed = 0;
  int failed = 0;
  int skipped = 0;
  bool compliant = false;
  std::string error;
};

struct CtpOptions
{
  std::filesystem::path dcp_dir;
  bool test_picture = true;
  bool test_audio = true;
  bool test_subtitles = true;
  bool test_security = true;
  bool test_packaging = true;
  bool test_composition = true;
  bool test_presentation = true;
};

/// Run DCI CTP-style compliance tests
/// Note: Full CTP testing requires reference content files.
/// This runs the structural/metadata checks that can be verified
/// without proprietary test patterns.
CtpResult run_ctp_tests(const CtpOptions& opts);

/// Output CTP results as JSON
std::string ctp_to_json(const CtpResult& result);

} // namespace dcpdoctor
