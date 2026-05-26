#include <spdlog/spdlog.h>
#include <chrono>
#include <filesystem>
#include <fstream>
#include <sstream>

#include "dcpdoctor/conformance.h"
#include "dcpdoctor/compliance.h"
#include "dcpdoctor/cpl.h"
#include "dcpdoctor/validators.h"

namespace fs = std::filesystem;

namespace dcpdoctor
{

static std::string today_iso()
{
  auto now = std::chrono::system_clock::now();
  auto days = std::chrono::floor<std::chrono::days>(now);
  std::chrono::year_month_day ymd{days};
  char buf[16];
  std::snprintf(buf, sizeof(buf), "%04d-%02u-%02u",
                static_cast<int>(ymd.year()),
                static_cast<unsigned>(ymd.month()),
                static_cast<unsigned>(ymd.day()));
  return buf;
}

static ConformanceTest make_test(const std::string& id, const std::string& desc,
                                 const std::string& spec, bool passed,
                                 const std::string& detail = {})
{
  ConformanceTest t;
  t.test_id = id;
  t.description = desc;
  t.spec_reference = spec;
  t.passed = passed;
  t.detail = detail;
  return t;
}

ConformanceReport run_conformance_tests(const ConformanceOptions& opts)
{
  ConformanceReport report;
  report.report_date = today_iso();
  report.tool_version = "dcpdoctor 1.0";
  report.dcp_dir = opts.dcp_dir;

  if(!fs::exists(opts.dcp_dir) || !fs::is_directory(opts.dcp_dir))
  {
    report.error = "DCP directory not found: " + opts.dcp_dir.string();
    return report;
  }

  // --- Structure tests (SMPTE ST 429-9) ---

  bool has_assetmap = fs::exists(opts.dcp_dir / "ASSETMAP") ||
                      fs::exists(opts.dcp_dir / "ASSETMAP.xml");
  report.structure_tests.push_back(
      make_test("DCI-STRUCT-1", "ASSETMAP present", "SMPTE ST 429-9:2014",
                has_assetmap, has_assetmap ? "Found" : "Missing"));

  bool has_volindex = fs::exists(opts.dcp_dir / "VOLINDEX") ||
                      fs::exists(opts.dcp_dir / "VOLINDEX.xml");
  report.structure_tests.push_back(
      make_test("DCI-STRUCT-2", "VOLINDEX present", "SMPTE ST 429-9:2014",
                has_volindex, has_volindex ? "Found" : "Missing"));

  // Find PKL
  std::vector<fs::path> pkls;
  for(const auto& entry : fs::directory_iterator(opts.dcp_dir))
  {
    if(!entry.is_regular_file())
      continue;
    if(entry.path().extension() != ".xml")
      continue;
    std::ifstream f(entry.path());
    std::string line;
    for(int i = 0; i < 5 && std::getline(f, line); ++i)
    {
      if(line.find("PackingList") != std::string::npos)
      {
        pkls.push_back(entry.path());
        break;
      }
    }
  }
  report.structure_tests.push_back(
      make_test("DCI-STRUCT-3", "PackingList (PKL) present", "SMPTE ST 429-8:2014",
                !pkls.empty(), pkls.empty() ? "No PKL found" : "Found " + std::to_string(pkls.size())));

  // Find CPL
  std::vector<fs::path> cpls;
  std::optional<Cpl> first_cpl;
  for(const auto& entry : fs::directory_iterator(opts.dcp_dir))
  {
    if(!entry.is_regular_file())
      continue;
    auto cpl = Cpl::parse(entry.path());
    if(cpl.has_value() && !cpl->id.empty())
    {
      cpls.push_back(entry.path());
      if(!first_cpl.has_value())
        first_cpl = cpl;
    }
  }
  report.structure_tests.push_back(
      make_test("DCI-STRUCT-4", "CompositionPlaylist (CPL) present", "SMPTE ST 429-7:2006",
                !cpls.empty(), cpls.empty() ? "No CPL found" : "Found " + std::to_string(cpls.size())));

  // MXF files
  bool has_mxf = false;
  int mxf_count = 0;
  for(const auto& entry : fs::directory_iterator(opts.dcp_dir))
  {
    if(entry.path().extension() == ".mxf")
    {
      has_mxf = true;
      mxf_count++;
    }
  }
  report.structure_tests.push_back(
      make_test("DCI-STRUCT-5", "MXF track files present", "SMPTE ST 429-3:2006",
                has_mxf, std::to_string(mxf_count) + " MXF file(s)"));

  // --- CPL tests (SMPTE ST 429-7) ---
  if(first_cpl.has_value())
  {
    report.content_title = first_cpl->content_title;
    report.cpl_id = first_cpl->id;
    report.issue_date = first_cpl->issue_date;

    // CPL has valid UUID
    bool valid_id = first_cpl->id.find("urn:uuid:") == 0 &&
                    first_cpl->id.size() > 20;
    report.cpl_tests.push_back(
        make_test("DCI-CPL-1", "CPL Id is valid URN UUID", "SMPTE ST 429-7:2006 §6.1",
                  valid_id, first_cpl->id));

    // CPL has content title
    report.cpl_tests.push_back(
        make_test("DCI-CPL-2", "ContentTitleText present", "SMPTE ST 429-7:2006 §6.2",
                  !first_cpl->content_title.empty(), first_cpl->content_title));

    // CPL has content kind
    report.cpl_tests.push_back(
        make_test("DCI-CPL-3", "ContentKind present", "SMPTE ST 429-7:2006 §6.4",
                  !first_cpl->content_kind.empty(), first_cpl->content_kind));

    // CPL has at least one reel
    report.cpl_tests.push_back(
        make_test("DCI-CPL-4", "At least one Reel present", "SMPTE ST 429-7:2006 §6.10",
                  !first_cpl->reels.empty(),
                  std::to_string(first_cpl->reels.size()) + " reel(s)"));

    // Each reel has picture
    bool all_reels_have_pic = true;
    for(const auto& reel : first_cpl->reels)
    {
      if(reel.picture.id.empty() && reel.picture.duration == 0)
      {
        all_reels_have_pic = false;
        break;
      }
    }
    report.cpl_tests.push_back(
        make_test("DCI-CPL-5", "All reels have MainPicture", "SMPTE ST 429-7:2006 §6.10.1",
                  all_reels_have_pic));

    // Issue date present
    report.cpl_tests.push_back(
        make_test("DCI-CPL-6", "IssueDate present", "SMPTE ST 429-7:2006 §6.3",
                  !first_cpl->issue_date.empty(), first_cpl->issue_date));
  }

  // --- Picture tests (SMPTE ST 429-4) ---
  if(opts.check_picture_profile)
  {
    // Check for valid picture MXFs (at minimum, verify they exist and have size > 0)
    for(const auto& entry : fs::directory_iterator(opts.dcp_dir))
    {
      if(entry.path().extension() != ".mxf")
        continue;
      if(entry.file_size() == 0)
      {
        report.picture_tests.push_back(
            make_test("DCI-PIC-1", "MXF file has non-zero size", "SMPTE ST 429-3:2006",
                      false, entry.path().filename().string() + " is empty"));
      }
    }
    if(report.picture_tests.empty())
    {
      report.picture_tests.push_back(
          make_test("DCI-PIC-1", "All MXF files have non-zero size", "SMPTE ST 429-3:2006",
                    has_mxf));
    }
  }

  // --- Audio tests ---
  // Basic: check that sound assets are referenced
  if(first_cpl.has_value())
  {
    bool has_audio = false;
    for(const auto& reel : first_cpl->reels)
    {
      if(!reel.sound.id.empty())
      {
        has_audio = true;
        break;
      }
    }
    report.audio_tests.push_back(
        make_test("DCI-AUD-1", "Audio track referenced in CPL", "SMPTE ST 429-7:2006",
                  has_audio, has_audio ? "MainSound present" : "No MainSound in any reel"));
  }

  // --- Security tests ---
  if(opts.check_security)
  {
    // Check for encryption indicators
    bool has_encryption = false;
    for(const auto& cpl_path : cpls)
    {
      std::ifstream f(cpl_path);
      std::string content((std::istreambuf_iterator<char>(f)),
                          std::istreambuf_iterator<char>());
      if(content.find("KeyId") != std::string::npos)
        has_encryption = true;
    }
    report.security_tests.push_back(
        make_test("DCI-SEC-1", "Encryption status detected", "SMPTE ST 429-6:2006",
                  true, // informational, not a pass/fail
                  has_encryption ? "Encrypted (KeyId found)" : "Not encrypted"));
  }

  // --- Detect standard ---
  for(const auto& cpl_path : cpls)
  {
    std::ifstream f(cpl_path);
    std::string content((std::istreambuf_iterator<char>(f)),
                        std::istreambuf_iterator<char>());
    if(content.find("smpte-ra.org") != std::string::npos)
      report.detected_standard = Standard::smpte;
    else if(content.find("digicine.com") != std::string::npos ||
            content.find("cinecert.com") != std::string::npos)
      report.detected_standard = Standard::interop;
  }

  // --- Summarize ---
  auto count_tests = [&](const std::vector<ConformanceTest>& tests) {
    for(const auto& t : tests)
    {
      report.total_tests++;
      if(t.passed)
        report.tests_passed++;
      else
        report.tests_failed++;
    }
  };
  count_tests(report.structure_tests);
  count_tests(report.cpl_tests);
  count_tests(report.picture_tests);
  count_tests(report.audio_tests);
  count_tests(report.security_tests);

  report.conformant = (report.tests_failed == 0);
  spdlog::info("DCI conformance: {}/{} tests passed ({})",
               report.tests_passed, report.total_tests,
               report.conformant ? "CONFORMANT" : "NON-CONFORMANT");
  return report;
}

static void json_escape(std::ostream& out, const std::string& s)
{
  for(char c : s)
  {
    switch(c)
    {
      case '"': out << "\\\""; break;
      case '\\': out << "\\\\"; break;
      case '\n': out << "\\n"; break;
      default: out << c;
    }
  }
}

std::string conformance_to_json(const ConformanceReport& report)
{
  std::ostringstream json;
  json << "{\n";
  json << "  \"tool_version\": \"" << report.tool_version << "\",\n";
  json << "  \"report_date\": \"" << report.report_date << "\",\n";
  json << "  \"dcp_dir\": \"";
  json_escape(json, report.dcp_dir.string());
  json << "\",\n";
  json << "  \"content_title\": \"";
  json_escape(json, report.content_title);
  json << "\",\n";
  json << "  \"cpl_id\": \"" << report.cpl_id << "\",\n";
  json << "  \"detected_standard\": \""
       << (report.detected_standard == Standard::smpte ? "SMPTE" :
           report.detected_standard == Standard::interop ? "Interop" : "Unknown")
       << "\",\n";
  json << "  \"conformant\": " << (report.conformant ? "true" : "false") << ",\n";
  json << "  \"total_tests\": " << report.total_tests << ",\n";
  json << "  \"tests_passed\": " << report.tests_passed << ",\n";
  json << "  \"tests_failed\": " << report.tests_failed << ",\n";

  auto write_tests = [&](const std::string& name, const std::vector<ConformanceTest>& tests) {
    json << "  \"" << name << "\": [\n";
    for(size_t i = 0; i < tests.size(); ++i)
    {
      const auto& t = tests[i];
      json << "    {\"id\": \"" << t.test_id << "\", "
           << "\"description\": \"";
      json_escape(json, t.description);
      json << "\", "
           << "\"spec\": \"" << t.spec_reference << "\", "
           << "\"passed\": " << (t.passed ? "true" : "false");
      if(!t.detail.empty())
      {
        json << ", \"detail\": \"";
        json_escape(json, t.detail);
        json << "\"";
      }
      json << "}";
      if(i + 1 < tests.size())
        json << ",";
      json << "\n";
    }
    json << "  ]";
  };

  write_tests("structure_tests", report.structure_tests);
  json << ",\n";
  write_tests("cpl_tests", report.cpl_tests);
  json << ",\n";
  write_tests("picture_tests", report.picture_tests);
  json << ",\n";
  write_tests("audio_tests", report.audio_tests);
  json << ",\n";
  write_tests("security_tests", report.security_tests);
  json << "\n}\n";
  return json.str();
}

std::string conformance_to_html(const ConformanceReport& report)
{
  std::ostringstream html;
  html << "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\">\n";
  html << "<title>DCI Conformance Report</title>\n";
  html << "<style>\n"
       << "body { font-family: -apple-system, sans-serif; max-width: 900px; margin: 2em auto; }\n"
       << "h1 { border-bottom: 2px solid #333; }\n"
       << "table { border-collapse: collapse; width: 100%; margin: 1em 0; }\n"
       << "th, td { border: 1px solid #ddd; padding: 8px; text-align: left; }\n"
       << "th { background: #f5f5f5; }\n"
       << ".pass { color: #2e7d32; font-weight: bold; }\n"
       << ".fail { color: #c62828; font-weight: bold; }\n"
       << ".badge { display: inline-block; padding: 4px 12px; border-radius: 4px; "
       << "font-weight: bold; color: white; }\n"
       << ".badge-pass { background: #2e7d32; }\n"
       << ".badge-fail { background: #c62828; }\n"
       << "</style>\n</head><body>\n";

  html << "<h1>DCI Conformance Report</h1>\n";
  html << "<p><strong>Tool:</strong> " << report.tool_version << "</p>\n";
  html << "<p><strong>Date:</strong> " << report.report_date << "</p>\n";
  html << "<p><strong>DCP:</strong> " << report.dcp_dir.string() << "</p>\n";
  if(!report.content_title.empty())
    html << "<p><strong>Title:</strong> " << report.content_title << "</p>\n";
  html << "<p><strong>Standard:</strong> "
       << (report.detected_standard == Standard::smpte ? "SMPTE" :
           report.detected_standard == Standard::interop ? "Interop" : "Unknown")
       << "</p>\n";

  html << "<p><span class=\"badge "
       << (report.conformant ? "badge-pass" : "badge-fail") << "\">"
       << (report.conformant ? "CONFORMANT" : "NON-CONFORMANT")
       << "</span> " << report.tests_passed << "/" << report.total_tests
       << " tests passed</p>\n";

  auto write_table = [&](const std::string& title, const std::vector<ConformanceTest>& tests) {
    if(tests.empty())
      return;
    html << "<h2>" << title << "</h2>\n";
    html << "<table><tr><th>ID</th><th>Test</th><th>Spec</th><th>Result</th><th>Detail</th></tr>\n";
    for(const auto& t : tests)
    {
      html << "<tr><td>" << t.test_id << "</td><td>" << t.description
           << "</td><td>" << t.spec_reference << "</td><td class=\""
           << (t.passed ? "pass" : "fail") << "\">"
           << (t.passed ? "PASS" : "FAIL") << "</td><td>" << t.detail
           << "</td></tr>\n";
    }
    html << "</table>\n";
  };

  write_table("Structure", report.structure_tests);
  write_table("Composition Playlist", report.cpl_tests);
  write_table("Picture", report.picture_tests);
  write_table("Audio", report.audio_tests);
  write_table("Security", report.security_tests);

  html << "</body></html>\n";
  return html.str();
}

} // namespace dcpdoctor
