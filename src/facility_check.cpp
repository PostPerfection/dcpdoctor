#include <spdlog/spdlog.h>
#include <filesystem>
#include <fstream>
#include <sstream>

#include "dcpdoctor/facility_check.h"
#include "dcpdoctor/checksum_verify.h"
#include "dcpdoctor/compliance.h"
#include "dcpdoctor/cpl.h"
#include "dcpdoctor/isdcf.h"
#include "dcpdoctor/validators.h"

namespace fs = std::filesystem;

namespace dcpdoctor
{

static CheckItem make_item(const std::string& category, const std::string& name, bool passed,
                           const std::string& detail = {}, const std::string& severity = "error")
{
  CheckItem item;
  item.category = category;
  item.check_name = name;
  item.passed = passed;
  item.detail = detail;
  item.severity = severity;
  return item;
}

FacilityCheckResult run_facility_check(const FacilityCheckOptions& opts)
{
  FacilityCheckResult result;

  if(!fs::exists(opts.dcp_dir) || !fs::is_directory(opts.dcp_dir))
  {
    result.error = "DCP directory not found: " + opts.dcp_dir.string();
    return result;
  }

  // --- Structure checks ---

  // Check ASSETMAP exists
  bool has_assetmap =
      fs::exists(opts.dcp_dir / "ASSETMAP") || fs::exists(opts.dcp_dir / "ASSETMAP.xml");
  result.items.push_back(make_item("structure", "ASSETMAP present", has_assetmap,
                                   has_assetmap ? "" : "Missing ASSETMAP or ASSETMAP.xml"));

  // Check VOLINDEX exists
  bool has_volindex =
      fs::exists(opts.dcp_dir / "VOLINDEX") || fs::exists(opts.dcp_dir / "VOLINDEX.xml");
  result.items.push_back(make_item("structure", "VOLINDEX present", has_volindex,
                                   has_volindex ? "" : "Missing VOLINDEX or VOLINDEX.xml"));

  // Find PKL files
  std::vector<fs::path> pkls;
  for(const auto& entry : fs::directory_iterator(opts.dcp_dir))
  {
    if(!entry.is_regular_file())
      continue;
    auto ext = entry.path().extension().string();
    if(ext == ".xml")
    {
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
  }
  result.items.push_back(make_item("structure", "PKL present", !pkls.empty(),
                                   pkls.empty() ? "No PackingList XML found" : ""));

  // Find CPL files
  std::vector<fs::path> cpls;
  for(const auto& entry : fs::directory_iterator(opts.dcp_dir))
  {
    if(!entry.is_regular_file())
      continue;
    auto cpl = Cpl::parse(entry.path());
    if(cpl.has_value() && !cpl->id.empty())
      cpls.push_back(entry.path());
  }
  result.items.push_back(make_item("structure", "CPL present", !cpls.empty(),
                                   cpls.empty() ? "No CompositionPlaylist XML found" : ""));

  // Check CPL has reels
  for(const auto& cpl_path : cpls)
  {
    auto cpl = Cpl::parse(cpl_path);
    if(cpl.has_value())
    {
      result.items.push_back(make_item("structure", "CPL has reels", !cpl->reels.empty(),
                                       cpl->reels.empty() ? "CPL has no reels/segments" : ""));
    }
  }

  // Check MXF files exist
  bool has_mxf = false;
  for(const auto& entry : fs::directory_iterator(opts.dcp_dir))
  {
    if(entry.path().extension() == ".mxf")
    {
      has_mxf = true;
      break;
    }
  }
  result.items.push_back(make_item("structure", "MXF essence files present", has_mxf,
                                   has_mxf ? "" : "No .mxf files found in DCP directory"));

  // --- Compliance checks ---
  auto compliance_notes = check_smpte_compliance(opts.dcp_dir, opts.expected_standard, opts.strict);
  bool compliance_ok = true;
  for(const auto& note : compliance_notes)
  {
    if(note.severity == Severity::error)
    {
      compliance_ok = false;
      break;
    }
  }
  result.items.push_back(make_item(
      "compliance", "SMPTE compliance", compliance_ok,
      compliance_ok ? "" : std::to_string(compliance_notes.size()) + " compliance issue(s)"));

  // Add individual compliance notes as items
  for(const auto& note : compliance_notes)
  {
    std::string sev = "info";
    if(note.severity == Severity::error)
      sev = "error";
    else if(note.severity == Severity::warning)
      sev = "warning";
    result.items.push_back(make_item("compliance", note.message, false, "", sev));
  }

  // --- Naming checks ---
  if(opts.check_naming)
  {
    for(const auto& cpl_path : cpls)
    {
      auto cpl = Cpl::parse(cpl_path);
      if(cpl.has_value())
      {
        auto naming_notes = check_isdcf_naming(cpl->content_title, cpl_path);
        bool naming_ok = naming_notes.empty();
        result.items.push_back(make_item(
            "naming", "ISDCF naming compliance", naming_ok,
            naming_ok ? "" : std::to_string(naming_notes.size()) + " naming issue(s)", "warning"));
      }
    }
  }

  // --- Hash verification ---
  if(opts.check_hashes)
  {
    ChecksumVerifyOptions hash_opts;
    hash_opts.package_dir = opts.dcp_dir;
    auto hash_result = verify_package_checksums(hash_opts);
    result.items.push_back(
        make_item("integrity", "File hash verification", hash_result.all_valid,
                  hash_result.all_valid
                      ? ""
                      : std::to_string(hash_result.hash_mismatches) + " hash mismatch(es)"));
  }

  // --- Reel continuity ---
  for(const auto& cpl_path : cpls)
  {
    auto cont_notes = check_reel_continuity(cpl_path);
    result.items.push_back(make_item("continuity", "Reel continuity", cont_notes.empty(),
                                     cont_notes.empty() ? "" : "Reel continuity issues detected",
                                     "warning"));
  }

  // --- Summarize ---
  for(const auto& item : result.items)
  {
    result.checks_total++;
    if(item.passed)
    {
      result.checks_passed++;
    }
    else
    {
      if(item.severity == "error")
        result.errors++;
      else if(item.severity == "warning")
        result.warnings++;
      else
        result.info_count++;
    }
  }

  result.ready = (result.errors == 0);
  result.summary = std::to_string(result.checks_passed) + "/" +
                   std::to_string(result.checks_total) + " checks passed";
  if(result.errors > 0)
    result.summary += ", " + std::to_string(result.errors) + " error(s)";
  if(result.warnings > 0)
    result.summary += ", " + std::to_string(result.warnings) + " warning(s)";

  spdlog::info("Facility check: {} — {}", result.ready ? "READY" : "NOT READY", result.summary);
  return result;
}

std::string facility_check_to_json(const FacilityCheckResult& result)
{
  std::ostringstream json;
  json << "{\n";
  json << "  \"ready\": " << (result.ready ? "true" : "false") << ",\n";
  json << "  \"summary\": \"" << result.summary << "\",\n";
  json << "  \"errors\": " << result.errors << ",\n";
  json << "  \"warnings\": " << result.warnings << ",\n";
  json << "  \"checks_passed\": " << result.checks_passed << ",\n";
  json << "  \"checks_total\": " << result.checks_total << ",\n";
  json << "  \"items\": [\n";
  for(size_t i = 0; i < result.items.size(); ++i)
  {
    const auto& item = result.items[i];
    json << "    {\"category\": \"" << item.category << "\", "
         << "\"check\": \"" << item.check_name << "\", "
         << "\"passed\": " << (item.passed ? "true" : "false") << ", "
         << "\"severity\": \"" << item.severity << "\"";
    if(!item.detail.empty())
      json << ", \"detail\": \"" << item.detail << "\"";
    json << "}";
    if(i + 1 < result.items.size())
      json << ",";
    json << "\n";
  }
  json << "  ]\n";
  json << "}\n";
  return json.str();
}

} // namespace dcpdoctor
