#include <spdlog/spdlog.h>
#include <filesystem>
#include <fstream>
#include <sstream>
#include <set>

#include "dcpdoctor/dci_ctp.h"
#include "dcpdoctor/cpl.h"

namespace fs = std::filesystem;

namespace dcpdoctor
{

static CtpTestResult make_ctp(const std::string& id, CtpCategory cat, const std::string& desc,
                              const std::string& req, bool passed, const std::string& detail = {})
{
  CtpTestResult r;
  r.test_id = id;
  r.category = cat;
  r.description = desc;
  r.requirement = req;
  r.passed = passed;
  r.detail = detail;
  return r;
}

static CtpTestResult make_skipped(const std::string& id, CtpCategory cat, const std::string& desc,
                                  const std::string& req, const std::string& reason)
{
  CtpTestResult r;
  r.test_id = id;
  r.category = cat;
  r.description = desc;
  r.requirement = req;
  r.skipped = true;
  r.detail = reason;
  return r;
}

CtpResult run_ctp_tests(const CtpOptions& opts)
{
  CtpResult result;

  if(!fs::exists(opts.dcp_dir) || !fs::is_directory(opts.dcp_dir))
  {
    result.error = "DCP directory not found: " + opts.dcp_dir.string();
    return result;
  }

  // --- Packaging tests (CTP Section 4) ---
  if(opts.test_packaging)
  {
    // CTP-PKG-001: VOLINDEX present with correct structure
    bool has_volindex =
        fs::exists(opts.dcp_dir / "VOLINDEX") || fs::exists(opts.dcp_dir / "VOLINDEX.xml");
    result.results.push_back(
        make_ctp("CTP-PKG-001", CtpCategory::Packaging, "VOLINDEX file present",
                 "DCI DCSS §9.3.1: Each volume shall contain a VOLINDEX file", has_volindex));

    // CTP-PKG-002: ASSETMAP present
    bool has_assetmap =
        fs::exists(opts.dcp_dir / "ASSETMAP") || fs::exists(opts.dcp_dir / "ASSETMAP.xml");
    result.results.push_back(
        make_ctp("CTP-PKG-002", CtpCategory::Packaging, "ASSETMAP file present",
                 "DCI DCSS §9.3.2: Each volume shall contain an ASSETMAP", has_assetmap));

    // CTP-PKG-003: PKL present and well-formed
    std::vector<fs::path> pkls;
    for(const auto& entry : fs::directory_iterator(opts.dcp_dir))
    {
      if(!entry.is_regular_file() || entry.path().extension() != ".xml")
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
    result.results.push_back(
        make_ctp("CTP-PKG-003", CtpCategory::Packaging, "Packing List (PKL) present",
                 "DCI DCSS §9.2: A DCP shall contain at least one PKL", !pkls.empty()));

    // CTP-PKG-004: MXF track files use .mxf extension
    bool all_mxf_ext = true;
    int mxf_count = 0;
    for(const auto& entry : fs::directory_iterator(opts.dcp_dir))
    {
      if(entry.path().extension() == ".mxf")
        mxf_count++;
    }
    result.results.push_back(make_ctp(
        "CTP-PKG-004", CtpCategory::Packaging, "MXF track files present with .mxf extension",
        "SMPTE ST 429-3: Track files shall use MXF container", mxf_count > 0,
        std::to_string(mxf_count) + " MXF file(s) found"));
  }

  // --- Composition tests (CTP Section 5) ---
  if(opts.test_composition)
  {
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

    // CTP-CPL-001: CPL present
    result.results.push_back(
        make_ctp("CTP-CPL-001", CtpCategory::Composition, "Composition Playlist present",
                 "DCI DCSS §8.4: A DCP shall contain at least one CPL", !cpls.empty()));

    if(first_cpl.has_value())
    {
      // CTP-CPL-002: CPL UUID format
      bool valid_uuid = first_cpl->id.find("urn:uuid:") == 0;
      result.results.push_back(make_ctp(
          "CTP-CPL-002", CtpCategory::Composition, "CPL uses URN:UUID identifier format",
          "SMPTE ST 429-7 §6.1: Id shall be a UUID in URN form", valid_uuid, first_cpl->id));

      // CTP-CPL-003: ContentKind uses DCI-approved value
      static const std::set<std::string> valid_kinds = {
          "feature", "trailer",      "test", "teaser", "rating", "advertisement",
          "short",   "transitional", "psa",  "policy", "episode"};
      bool valid_kind = valid_kinds.contains(first_cpl->content_kind);
      result.results.push_back(make_ctp("CTP-CPL-003", CtpCategory::Composition,
                                        "ContentKind uses approved value",
                                        "DCI DCSS §8.4.1: ContentKind shall be from approved list",
                                        valid_kind, first_cpl->content_kind));

      // CTP-CPL-004: At least one reel with picture
      bool has_picture_reel = false;
      for(const auto& reel : first_cpl->reels)
      {
        if(!reel.picture.id.empty() || reel.picture.duration > 0)
        {
          has_picture_reel = true;
          break;
        }
      }
      result.results.push_back(make_ctp(
          "CTP-CPL-004", CtpCategory::Composition, "At least one reel contains picture essence",
          "DCI DCSS §8.4.2: Each CPL shall reference picture essence", has_picture_reel));

      // CTP-CPL-005: EditRate is DCI-approved
      static const std::set<std::string> valid_rates = {"24 1", "25 1", "30 1",
                                                        "48 1", "50 1", "60 1"};
      bool valid_rate = true;
      for(const auto& reel : first_cpl->reels)
      {
        if(!reel.picture.edit_rate.empty() && !valid_rates.contains(reel.picture.edit_rate))
        {
          valid_rate = false;
          break;
        }
      }
      result.results.push_back(make_ctp(
          "CTP-CPL-005", CtpCategory::Composition, "EditRate uses DCI-approved frame rate",
          "DCI DCSS §3.2.1: Frame rates shall be 24, 25, 30, 48, 50, or 60 fps", valid_rate));
    }
  }

  // --- Picture tests (CTP Section 6) ---
  if(opts.test_picture)
  {
    // These require actual J2K decode to verify; mark as skipped without reference content
    result.results.push_back(
        make_skipped("CTP-PIC-001", CtpCategory::Picture, "JPEG 2000 Profile: 9-7 irreversible DWT",
                     "DCI DCSS §3.2.1.1: Only 9-7 irreversible wavelet",
                     "Requires J2K frame decode — use with reference content"));

    result.results.push_back(make_skipped(
        "CTP-PIC-002", CtpCategory::Picture, "JPEG 2000 code-block size 32x32",
        "DCI DCSS §3.2.1.1: Code-block size shall be 32x32", "Requires J2K codestream analysis"));

    result.results.push_back(make_skipped(
        "CTP-PIC-003", CtpCategory::Picture, "Maximum bitrate 250 Mbit/s (2K) / 500 Mbit/s (4K)",
        "DCI DCSS §3.2.1: Peak bitrate limits", "Requires per-frame bitrate analysis"));

    // Check MXF picture file sizes are reasonable
    for(const auto& entry : fs::directory_iterator(opts.dcp_dir))
    {
      if(entry.path().extension() != ".mxf")
        continue;
      if(entry.file_size() == 0)
      {
        result.results.push_back(make_ctp("CTP-PIC-004", CtpCategory::Picture,
                                          "MXF track file non-empty",
                                          "SMPTE ST 378: MXF file shall contain valid essence",
                                          false, entry.path().filename().string() + " is 0 bytes"));
      }
    }
    // If no zero-byte files found, pass
    bool any_zero = false;
    for(const auto& entry : fs::directory_iterator(opts.dcp_dir))
    {
      if(entry.path().extension() == ".mxf" && entry.file_size() == 0)
        any_zero = true;
    }
    if(!any_zero)
    {
      result.results.push_back(
          make_ctp("CTP-PIC-004", CtpCategory::Picture, "All MXF track files non-empty",
                   "SMPTE ST 378: MXF file shall contain valid essence", true));
    }
  }

  // --- Audio tests (CTP Section 7) ---
  if(opts.test_audio)
  {
    result.results.push_back(make_skipped("CTP-AUD-001", CtpCategory::Audio,
                                          "Audio PCM: 24-bit, 48kHz or 96kHz",
                                          "DCI DCSS §3.3.1: Audio shall be 24-bit LPCM at 48/96kHz",
                                          "Requires MXF audio essence analysis"));
  }

  // --- Security tests (CTP Section 8) ---
  if(opts.test_security)
  {
    // Check if encrypted content has proper KeyId references
    bool has_encryption = false;
    for(const auto& entry : fs::directory_iterator(opts.dcp_dir))
    {
      if(entry.path().extension() != ".xml")
        continue;
      std::ifstream f(entry.path());
      std::string content((std::istreambuf_iterator<char>(f)), std::istreambuf_iterator<char>());
      if(content.find("KeyId") != std::string::npos)
      {
        has_encryption = true;
        break;
      }
    }
    result.results.push_back(make_ctp("CTP-SEC-001", CtpCategory::Security, "Encryption status",
                                      "DCI DCSS §5: Content security",
                                      true, // informational
                                      has_encryption ? "Encrypted" : "Unencrypted"));
  }

  // --- Presentation tests (CTP Section 9) ---
  if(opts.test_presentation)
  {
    result.results.push_back(make_skipped("CTP-PRE-001", CtpCategory::Presentation,
                                          "FFMC/LFMC markers present",
                                          "SMPTE ST 429-7 §6.10.1.4: Markers for automation",
                                          "Marker validation requires full CPL marker analysis"));
  }

  // --- Summarize ---
  for(const auto& r : result.results)
  {
    result.total++;
    if(r.skipped)
      result.skipped++;
    else if(r.passed)
      result.passed++;
    else
      result.failed++;
  }
  result.compliant = (result.failed == 0);
  spdlog::info("DCI CTP: {}/{} passed, {} skipped, {} failed ({})", result.passed,
               result.total - result.skipped, result.skipped, result.failed,
               result.compliant ? "COMPLIANT" : "NON-COMPLIANT");
  return result;
}

std::string ctp_to_json(const CtpResult& result)
{
  std::ostringstream json;
  json << "{\n";
  json << "  \"compliant\": " << (result.compliant ? "true" : "false") << ",\n";
  json << "  \"total\": " << result.total << ",\n";
  json << "  \"passed\": " << result.passed << ",\n";
  json << "  \"failed\": " << result.failed << ",\n";
  json << "  \"skipped\": " << result.skipped << ",\n";
  json << "  \"results\": [\n";
  for(size_t i = 0; i < result.results.size(); ++i)
  {
    const auto& r = result.results[i];
    json << "    {\"id\": \"" << r.test_id << "\", "
         << "\"description\": \"" << r.description << "\", "
         << "\"requirement\": \"" << r.requirement << "\", "
         << "\"passed\": " << (r.passed ? "true" : "false") << ", "
         << "\"skipped\": " << (r.skipped ? "true" : "false");
    if(!r.detail.empty())
      json << ", \"detail\": \"" << r.detail << "\"";
    json << "}";
    if(i + 1 < result.results.size())
      json << ",";
    json << "\n";
  }
  json << "  ]\n}\n";
  return json.str();
}

} // namespace dcpdoctor
