#pragma once

#include "dcpdoctor/dcpdoctor.h"
#include <filesystem>
#include <string>
#include <vector>

namespace dcpdoctor
{

/// Individual DCI conformance test result
struct ConformanceTest
{
  std::string test_id; // e.g. "DCI-1.1", "SMPTE-429-2"
  std::string description; // What the test checks
  std::string spec_reference; // Standard reference (e.g. "SMPTE ST 429-7:2006")
  bool passed = false;
  std::string detail; // Explanation / evidence
};

/// DCI Conformance report
struct ConformanceReport
{
  std::string tool_version;
  std::string report_date;
  std::filesystem::path dcp_dir;
  Standard detected_standard = Standard::unknown;

  // DCP metadata
  std::string content_title;
  std::string cpl_id;
  std::string issue_date;

  // Test results grouped by category
  std::vector<ConformanceTest> structure_tests;
  std::vector<ConformanceTest> cpl_tests;
  std::vector<ConformanceTest> picture_tests;
  std::vector<ConformanceTest> audio_tests;
  std::vector<ConformanceTest> security_tests;

  // Summary
  int total_tests = 0;
  int tests_passed = 0;
  int tests_failed = 0;
  bool conformant = false; // true only if all mandatory tests pass
  std::string error;
};

struct ConformanceOptions
{
  std::filesystem::path dcp_dir;
  bool check_picture_profile = true; // J2K profile compliance
  bool check_security = true; // Encryption / KDM checks
  bool verbose = false;
};

/// Run DCI conformance test suite on a DCP
ConformanceReport run_conformance_tests(const ConformanceOptions& opts);

/// Output conformance report as JSON
std::string conformance_to_json(const ConformanceReport& report);

/// Output conformance report as HTML
std::string conformance_to_html(const ConformanceReport& report);

} // namespace dcpdoctor
