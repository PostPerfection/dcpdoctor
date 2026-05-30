#include <filesystem>
#include <fstream>
#include <map>
#include <regex>
#include <sstream>

#include "dcpdoctor/fixes.h"
#include "dcpdoctor/hash.h"
#include "dcpdoctor/pkl.h"

namespace dcpdoctor
{
namespace fs = std::filesystem;

std::vector<FixSuggestion> suggest_fixes(const std::vector<Note>& notes)
{
  std::vector<FixSuggestion> suggestions;

  for(const auto& note : notes)
  {
    switch(note.code)
    {
      case Code::smpte_naming_violation:
        if(note.message.find("ASSETMAP") != std::string::npos)
        {
          suggestions.push_back(FixSuggestion{
              Code::smpte_naming_violation, "Rename ASSETMAP to ASSETMAP.xml for BV2.1 compliance",
              "mv ASSETMAP ASSETMAP.xml", true});
        }
        else if(note.message.find("PKL") != std::string::npos &&
                note.message.find(".xml") != std::string::npos)
        {
          suggestions.push_back(FixSuggestion{
              Code::smpte_naming_violation, "Rename PKL file to have .xml extension", "",
              false // Need to know exact filename
          });
        }
        break;

      case Code::smpte_namespace_wrong:
        suggestions.push_back(FixSuggestion{
            Code::smpte_namespace_wrong,
            "Fix namespace: replace Interop namespace with SMPTE namespace in XML", "", true});
        break;

      case Code::pkl_hash_mismatch:
        suggestions.push_back(FixSuggestion{
            Code::pkl_hash_mismatch, "Regenerate PKL hashes from actual file contents", "", true});
        break;

      case Code::missing_required_element:
        if(note.message.find("ContentVersion") != std::string::npos)
        {
          suggestions.push_back(
              FixSuggestion{Code::missing_required_element,
                            "Add ContentVersion element to CPL (required for BV2.1)", "", false});
        }
        else if(note.message.find("MainMarkers") != std::string::npos)
        {
          suggestions.push_back(FixSuggestion{
              Code::missing_required_element,
              "Add MainMarkers to first reel in CPL (FFOC, LFOC at minimum for BV2.1)", "", false});
        }
        break;

      case Code::j2k_bitrate_exceeded:
        suggestions.push_back(FixSuggestion{
            Code::j2k_bitrate_exceeded,
            "Re-encode picture at lower bitrate (DCI limit: 250 Mbps for 2K, 500 Mbps for 4K)", "",
            false});
        break;

      case Code::isdcf_naming_violation:
        suggestions.push_back(
            FixSuggestion{Code::isdcf_naming_violation,
                          "Rename content title to follow ISDCF naming convention: "
                          "Title_ContentType_AspectRatio_Language_Territory_AudioType_Resolution_"
                          "Studio_Date_Facility_Standard",
                          "", false});
        break;

      case Code::sound_invalid_channel_count:
        if(note.message.find("MCA") != std::string::npos)
        {
          suggestions.push_back(FixSuggestion{
              Code::sound_invalid_channel_count,
              "Add MCA (Multi-Channel Audio) labeling metadata to sound MXF", "", false});
        }
        break;

      case Code::encryption_detected:
        suggestions.push_back(FixSuggestion{
            Code::encryption_detected,
            "Obtain a valid KDM from the content distributor for this theater's certificate", "",
            false});
        break;

      case Code::marker_missing:
        suggestions.push_back(FixSuggestion{Code::marker_missing,
                                            "Add required markers to CPL (FFOC=first frame, "
                                            "LFOC=last frame, FFMC, LFMC for features)",
                                            "", false});
        break;

      case Code::subtitle_invalid_timing:
        suggestions.push_back(FixSuggestion{
            Code::subtitle_invalid_timing,
            "Fix subtitle timing: ensure all TimeIn < TimeOut and within reel duration", "",
            false});
        break;

      default:
        break;
    }
  }

  return suggestions;
}

int apply_fixes(const fs::path& dcp_dir, const std::vector<FixSuggestion>& suggestions)
{
  int applied = 0;

  for(const auto& fix : suggestions)
  {
    if(!fix.auto_fixable)
      continue;

    if(fix.related_code == Code::smpte_naming_violation &&
       fix.command == "mv ASSETMAP ASSETMAP.xml")
    {
      auto src = dcp_dir / "ASSETMAP";
      auto dst = dcp_dir / "ASSETMAP.xml";
      if(fs::exists(src) && !fs::exists(dst))
      {
        std::error_code ec;
        fs::rename(src, dst, ec);
        if(!ec)
          ++applied;
      }
    }

    if(fix.related_code == Code::smpte_namespace_wrong)
    {
      applied += fix_namespaces(dcp_dir);
    }

    if(fix.related_code == Code::pkl_hash_mismatch)
    {
      applied += fix_pkl_hashes(dcp_dir);
    }
  }

  return applied;
}

static std::string read_text_file(const fs::path& path)
{
  std::ifstream ifs(path, std::ios::binary);
  if(!ifs)
    return {};
  std::ostringstream ss;
  ss << ifs.rdbuf();
  return ss.str();
}

static bool write_text_file(const fs::path& path, const std::string& content)
{
  std::ofstream ofs(path, std::ios::binary);
  if(!ofs)
    return false;
  ofs << content;
  return ofs.good();
}

int fix_namespaces(const std::filesystem::path& dcp_dir)
{
  int fixed = 0;

  const std::string interop_cpl_ns = "http://www.digicine.com/PROTO-ASDCP-CPL-20040511#";
  const std::string smpte_cpl_ns = "http://www.smpte-ra.org/schemas/429-7/2006/CPL";

  // Fix XML files: replace Interop namespace with SMPTE
  for(auto& entry : fs::directory_iterator(dcp_dir))
  {
    if(!entry.is_regular_file())
      continue;
    auto ext = entry.path().extension().string();
    if(ext != ".xml")
      continue;

    auto content = read_text_file(entry.path());
    if(content.empty())
      continue;

    if(content.find(interop_cpl_ns) != std::string::npos)
    {
      std::string updated = content;
      size_t pos = 0;
      while((pos = updated.find(interop_cpl_ns, pos)) != std::string::npos)
      {
        updated.replace(pos, interop_cpl_ns.size(), smpte_cpl_ns);
        pos += smpte_cpl_ns.size();
      }
      if(write_text_file(entry.path(), updated))
        ++fixed;
    }
  }

  return fixed;
}

int fix_pkl_hashes(const std::filesystem::path& dcp_dir)
{
  int fixed = 0;

  // Find PKL files (look for files containing PackingList in their XML)
  for(auto& entry : fs::directory_iterator(dcp_dir))
  {
    if(!entry.is_regular_file())
      continue;
    auto ext = entry.path().extension().string();
    if(ext != ".xml")
      continue;

    auto content = read_text_file(entry.path());
    if(content.find("PackingList") == std::string::npos)
      continue;

    // Parse PKL to get asset hashes
    auto pkl = Pkl::parse(entry.path());
    if(!pkl)
      continue;

    bool modified = false;
    std::string updated = content;

    for(const auto& asset : pkl->assets)
    {
      if(asset.hash.empty())
        continue;

      // Find the actual file for this asset via ASSETMAP
      fs::path asset_path;

      // Try to find by original_filename first
      if(!asset.original_filename.empty())
      {
        auto candidate = dcp_dir / asset.original_filename;
        if(fs::exists(candidate))
          asset_path = candidate;
      }

      // If not found, scan directory for matching file
      if(asset_path.empty())
      {
        for(auto& f : fs::directory_iterator(dcp_dir))
        {
          if(!f.is_regular_file())
            continue;
          // Skip the PKL itself and other XML files for hash checking
          auto fe = f.path().extension().string();
          if(fe == ".xml")
            continue;
          // Check if this file's hash matches or if it's the only MXF etc.
        }
      }

      if(asset_path.empty() || !fs::exists(asset_path))
        continue;

      auto computed = sha1_base64(asset_path);
      if(!computed || *computed == asset.hash)
        continue;

      // Replace the old hash with the computed one in the PKL XML
      auto pos = updated.find(asset.hash);
      if(pos != std::string::npos)
      {
        updated.replace(pos, asset.hash.size(), *computed);
        modified = true;
        ++fixed;
      }
    }

    if(modified)
      write_text_file(entry.path(), updated);
  }

  return fixed;
}

int fix_content_kind(const std::filesystem::path& dcp_dir)
{
  int fixed = 0;

  static const std::map<std::string, std::string> kind_map = {
      {"Feature", "feature"},
      {"FEATURE", "feature"},
      {"Features", "feature"},
      {"Trailer", "trailer"},
      {"TRAILER", "trailer"},
      {"Trailers", "trailer"},
      {"Test", "test"},
      {"TEST", "test"},
      {"Teaser", "teaser"},
      {"TEASER", "teaser"},
      {"Rating", "rating"},
      {"RATING", "rating"},
      {"Advertisement", "advertisement"},
      {"ADVERTISEMENT", "advertisement"},
      {"Ad", "advertisement"},
      {"Short", "short"},
      {"SHORT", "short"},
      {"Transitional", "transitional"},
      {"PSA", "psa"},
      {"Policy", "policy"},
      {"Episode", "episode"},
  };

  for(auto& entry : fs::directory_iterator(dcp_dir))
  {
    if(!entry.is_regular_file())
      continue;
    auto ext = entry.path().extension().string();
    if(ext != ".xml")
      continue;

    auto content = read_text_file(entry.path());
    if(content.find("<ContentKind>") == std::string::npos)
      continue;

    std::regex re(R"(<ContentKind>([^<]+)</ContentKind>)");
    std::smatch m;
    if(!std::regex_search(content, m, re))
      continue;

    std::string original = m[1].str();
    // Trim whitespace
    auto trimmed = original;
    trimmed.erase(0, trimmed.find_first_not_of(" \t\n\r"));
    trimmed.erase(trimmed.find_last_not_of(" \t\n\r") + 1);

    std::string normalized;
    auto it = kind_map.find(trimmed);
    if(it != kind_map.end())
      normalized = it->second;
    else
    {
      // Try lowercase
      std::string lower = trimmed;
      for(auto& c : lower)
        c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
      normalized = lower;
    }

    if(normalized == trimmed)
      continue;

    std::string old_tag = "<ContentKind>" + original + "</ContentKind>";
    std::string new_tag = "<ContentKind>" + normalized + "</ContentKind>";
    auto pos = content.find(old_tag);
    if(pos != std::string::npos)
    {
      content.replace(pos, old_tag.size(), new_tag);
      if(write_text_file(entry.path(), content))
        ++fixed;
    }
  }

  return fixed;
}

} // namespace dcpdoctor
