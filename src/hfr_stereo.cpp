#include <libxml/parser.h>
#include <functional>
#include <sstream>
#include <set>
#include <map>

#include "dcpdoctor/hfr_stereo.h"

namespace dcpdoctor
{
namespace fs = std::filesystem;
namespace
{

  std::string get_text(xmlNodePtr parent, const char* name)
  {
    for(auto child = parent->children; child; child = child->next)
    {
      if(child->type == XML_ELEMENT_NODE && xmlStrcmp(child->name, BAD_CAST name) == 0)
      {
        auto content = xmlNodeGetContent(child);
        if(content)
        {
          std::string r(reinterpret_cast<const char*>(content));
          xmlFree(content);
          return r;
        }
      }
    }
    return {};
  }

  std::string find_recursive(xmlNodePtr node, const char* name)
  {
    for(auto cur = node; cur; cur = cur->next)
    {
      if(cur->type == XML_ELEMENT_NODE && xmlStrcmp(cur->name, BAD_CAST name) == 0)
      {
        auto content = xmlNodeGetContent(cur);
        if(content)
        {
          std::string r(reinterpret_cast<const char*>(content));
          xmlFree(content);
          return r;
        }
      }
      auto child_result = find_recursive(cur->children, name);
      if(!child_result.empty())
        return child_result;
    }
    return {};
  }

  double parse_edit_rate(const std::string& str)
  {
    std::istringstream iss(str);
    int num = 0, den = 1;
    iss >> num >> den;
    if(den <= 0)
      den = 1;
    return double(num) / double(den);
  }

} // namespace

std::vector<Note> check_hfr_compliance(const fs::path& cpl_path)
{
  std::vector<Note> notes;

  auto doc = xmlReadFile(cpl_path.string().c_str(), nullptr,
                         XML_PARSE_NOERROR | XML_PARSE_NOWARNING | XML_PARSE_NONET);
  if(!doc)
    return notes;

  auto root = xmlDocGetRootElement(doc);
  if(!root)
  {
    xmlFreeDoc(doc);
    return notes;
  }

  std::string root_name(reinterpret_cast<const char*>(root->name));
  if(root_name != "CompositionPlaylist")
  {
    xmlFreeDoc(doc);
    return notes;
  }

  auto edit_rate_str = find_recursive(root->children, "EditRate");
  if(edit_rate_str.empty())
  {
    xmlFreeDoc(doc);
    return notes;
  }

  double fps = parse_edit_rate(edit_rate_str);

  if(fps > 30.0)
  {
    // HFR content - additional DCI constraints apply
    notes.push_back(Note{Severity::info, Code::cpl_invalid_edit_rate,
                         "HFR content detected: " + std::to_string(int(fps)) + " fps", cpl_path});

    // DCI maximum is 48fps for 4K, 60fps for 2K
    if(fps > 60.0)
    {
      notes.push_back(Note{Severity::warning, Code::cpl_invalid_edit_rate,
                           "Frame rate " + std::to_string(int(fps)) +
                               " fps exceeds DCI maximum (60fps for 2K, 48fps for 4K)",
                           cpl_path});
    }

    // HFR with 4K: DCI limits to 48fps max
    // Check if resolution suggests 4K
    auto stored_width = find_recursive(root->children, "StoredWidth");
    if(!stored_width.empty() && std::stoi(stored_width) > 2048 && fps > 48.0)
    {
      notes.push_back(Note{Severity::error, Code::cpl_invalid_edit_rate,
                           "4K content at " + std::to_string(int(fps)) +
                               " fps exceeds DCI 4K HFR limit of 48fps",
                           cpl_path});
    }

    // BV2.1 approved rates
    bool bv21_rate = (fps == 48.0 || fps == 60.0);
    if(!bv21_rate && fps > 30.0)
    {
      notes.push_back(Note{Severity::warning, Code::cpl_invalid_edit_rate,
                           "Frame rate " + std::to_string(int(fps)) +
                               " fps is HFR but not a BV2.1 approved rate (48 or 60)",
                           cpl_path});
    }

    // HFR bitrate constraint: max 500 Mbps regardless of resolution
    notes.push_back(Note{Severity::info, Code::j2k_bitrate_exceeded,
                         "HFR content: DCI maximum bitrate is 500 Mbps for all HFR content",
                         cpl_path});
  }

  xmlFreeDoc(doc);
  return notes;
}

MultiCplInfo analyze_multi_cpl(const fs::path& dcp_dir)
{
  MultiCplInfo info;

  std::error_code ec;
  std::set<double> frame_rates;

  for(auto& entry : fs::directory_iterator(dcp_dir, ec))
  {
    if(!entry.is_regular_file())
      continue;
    if(entry.path().extension() != ".xml")
      continue;

    auto doc = xmlReadFile(entry.path().string().c_str(), nullptr,
                           XML_PARSE_NOERROR | XML_PARSE_NOWARNING | XML_PARSE_NONET);
    if(!doc)
      continue;

    auto root = xmlDocGetRootElement(doc);
    if(!root)
    {
      xmlFreeDoc(doc);
      continue;
    }

    std::string rn(reinterpret_cast<const char*>(root->name));
    if(rn != "CompositionPlaylist")
    {
      xmlFreeDoc(doc);
      continue;
    }

    MultiCplInfo::CplEntry cpl;
    cpl.id = get_text(root, "Id");
    cpl.content_title = get_text(root, "ContentTitleText");
    cpl.edit_rate = find_recursive(root->children, "EditRate");

    double fps = parse_edit_rate(cpl.edit_rate);
    frame_rates.insert(fps);

    // Count reels (DCP: ReelList, IMF: SegmentList)
    for(auto child = root->children; child; child = child->next)
    {
      if(child->type != XML_ELEMENT_NODE)
        continue;
      if(xmlStrcmp(child->name, BAD_CAST "ReelList") == 0)
      {
        for(auto reel = child->children; reel; reel = reel->next)
        {
          if(reel->type == XML_ELEMENT_NODE && xmlStrcmp(reel->name, BAD_CAST "Reel") == 0)
            cpl.reel_count++;
        }
      }
      else if(xmlStrcmp(child->name, BAD_CAST "SegmentList") == 0)
      {
        for(auto seg = child->children; seg; seg = seg->next)
        {
          if(seg->type == XML_ELEMENT_NODE && xmlStrcmp(seg->name, BAD_CAST "Segment") == 0)
            cpl.reel_count++;
        }
      }
    }

    // Determine type from content title
    std::string title_lower;
    for(char c : cpl.content_title)
      title_lower += std::tolower(c);
    if(title_lower.find("trailer") != std::string::npos)
      cpl.type = "trailer";
    else if(title_lower.find("advert") != std::string::npos)
      cpl.type = "advertisement";
    else if(title_lower.find("test") != std::string::npos)
      cpl.type = "test";
    else
      cpl.type = "main";

    info.cpls.push_back(std::move(cpl));
    xmlFreeDoc(doc);
  }

  info.consistent_frame_rate = (frame_rates.size() <= 1);
  return info;
}

Stereo3dInfo analyze_stereo3d(const fs::path& cpl_path)
{
  Stereo3dInfo info;

  auto doc = xmlReadFile(cpl_path.string().c_str(), nullptr,
                         XML_PARSE_NOERROR | XML_PARSE_NOWARNING | XML_PARSE_NONET);
  if(!doc)
    return info;

  auto root = xmlDocGetRootElement(doc);
  if(!root)
  {
    xmlFreeDoc(doc);
    return info;
  }

  // Look for MainStereoscopicPicture elements
  std::function<void(xmlNodePtr)> scan = [&](xmlNodePtr node) {
    for(auto cur = node; cur; cur = cur->next)
    {
      if(cur->type == XML_ELEMENT_NODE)
      {
        std::string name(reinterpret_cast<const char*>(cur->name));
        if(name == "MainStereoscopicPicture")
        {
          info.is_stereo = true;
          info.has_stereo_metadata = true;
          info.stereo_type = "frame-sequential";

          auto dur = get_text(cur, "Duration");
          if(!dur.empty())
          {
            uint64_t d = std::stoull(dur);
            info.left_duration += d / 2;
            info.right_duration += d / 2;
          }
        }
      }
      scan(cur->children);
    }
  };

  scan(root->children);
  xmlFreeDoc(doc);

  if(info.is_stereo)
  {
    info.eye_offset =
        static_cast<int64_t>(info.left_duration) - static_cast<int64_t>(info.right_duration);
    info.eyes_aligned = (info.eye_offset == 0);
  }

  return info;
}

std::vector<Note> check_stereo3d_compliance(const Stereo3dInfo& info, const fs::path& cpl_path)
{
  std::vector<Note> notes;
  if(!info.is_stereo)
    return notes;

  if(!info.eyes_aligned)
  {
    notes.push_back(Note{Severity::error, Code::stereo_mismatch,
                         "3D eye alignment error: " + std::to_string(info.eye_offset) +
                             " frame offset between left and right eyes",
                         cpl_path});
  }

  if(!info.has_stereo_metadata)
  {
    notes.push_back(Note{Severity::warning, Code::stereo_mismatch,
                         "3D content missing stereoscopic metadata", cpl_path});
  }

  notes.push_back(Note{Severity::info, Code::stereo_mismatch,
                       "3D stereoscopic content (" + info.stereo_type +
                           "): L=" + std::to_string(info.left_duration) +
                           " R=" + std::to_string(info.right_duration) + " frames",
                       cpl_path});

  return notes;
}

std::vector<CplChainEntry> trace_cpl_chain(const fs::path& dcp_dir)
{
  std::vector<CplChainEntry> chain;

  std::error_code ec;
  for(auto& entry : fs::directory_iterator(dcp_dir, ec))
  {
    if(!entry.is_regular_file())
      continue;
    if(entry.path().extension() != ".xml")
      continue;

    auto doc = xmlReadFile(entry.path().string().c_str(), nullptr,
                           XML_PARSE_NOERROR | XML_PARSE_NOWARNING | XML_PARSE_NONET);
    if(!doc)
      continue;

    auto root = xmlDocGetRootElement(doc);
    if(!root)
    {
      xmlFreeDoc(doc);
      continue;
    }

    std::string rn(reinterpret_cast<const char*>(root->name));
    if(rn != "CompositionPlaylist")
    {
      xmlFreeDoc(doc);
      continue;
    }

    CplChainEntry cpl;
    cpl.cpl_id = get_text(root, "Id");
    cpl.content_title = get_text(root, "ContentTitleText");
    cpl.content_version_id = find_recursive(root->children, "VersionNumber");
    cpl.content_version_label = find_recursive(root->children, "LabelText");

    // Check for IssueDate to determine version ordering
    // Check for OriginalContentId (indicates supplemental)
    auto opl = find_recursive(root->children, "OriginalPackageList");
    if(!opl.empty())
    {
      cpl.is_supplemental = true;
      cpl.original_cpl_id = opl;
    }

    chain.push_back(std::move(cpl));
    xmlFreeDoc(doc);
  }

  return chain;
}

std::vector<Note> check_cpl_chain(const std::vector<CplChainEntry>& chain, const fs::path& dcp_dir)
{
  std::vector<Note> notes;

  int supplemental_count = 0;
  for(const auto& entry : chain)
  {
    if(entry.is_supplemental)
      supplemental_count++;
  }

  if(supplemental_count > 0)
  {
    notes.push_back(Note{Severity::info, Code::supplemental_opl_missing,
                         "DCP contains " + std::to_string(supplemental_count) +
                             " supplemental CPL(s) in version chain",
                         dcp_dir});

    // Check that original CPL IDs are resolvable
    std::set<std::string> known_ids;
    for(const auto& entry : chain)
      known_ids.insert(entry.cpl_id);

    for(const auto& entry : chain)
    {
      if(entry.is_supplemental && !entry.original_cpl_id.empty())
      {
        if(!known_ids.contains(entry.original_cpl_id))
        {
          notes.push_back(Note{Severity::warning, Code::supplemental_opl_missing,
                               "Supplemental CPL references original " + entry.original_cpl_id +
                                   " not found in this package",
                               dcp_dir});
        }
      }
    }
  }

  return notes;
}

} // namespace dcpdoctor
