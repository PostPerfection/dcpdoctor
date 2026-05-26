#include <libxml/parser.h>
#include <libxml/tree.h>
#include <cstring>
#include <regex>
#include <set>

#include "dcpdoctor/compliance.h"

namespace dcpdoctor
{
namespace
{

  xmlNodePtr find_child(xmlNodePtr parent, const char* name)
  {
    for(auto child = parent->children; child; child = child->next)
    {
      if(child->type == XML_ELEMENT_NODE &&
         std::strcmp(reinterpret_cast<const char*>(child->name), name) == 0)
        return child;
    }
    return nullptr;
  }

  xmlNodePtr find_child_recursive(xmlNodePtr parent, const char* name)
  {
    for(auto child = parent->children; child; child = child->next)
    {
      if(child->type == XML_ELEMENT_NODE &&
         std::strcmp(reinterpret_cast<const char*>(child->name), name) == 0)
        return child;
      auto found = find_child_recursive(child, name);
      if(found)
        return found;
    }
    return nullptr;
  }

  std::string get_text(xmlNodePtr node)
  {
    if(!node)
      return {};
    xmlChar* content = xmlNodeGetContent(node);
    if(!content)
      return {};
    std::string s(reinterpret_cast<const char*>(content));
    xmlFree(content);
    return s;
  }

  std::string get_ns_uri(xmlNodePtr node)
  {
    if(!node || !node->ns || !node->ns->href)
      return {};
    return reinterpret_cast<const char*>(node->ns->href);
  }

  // SMPTE namespaces
  constexpr const char* SMPTE_CPL_NS = "http://www.smpte-ra.org/schemas/429-7/2006/CPL";
  constexpr const char* SMPTE_PKL_NS = "http://www.smpte-ra.org/schemas/429-8/2007/PKL";
  constexpr const char* SMPTE_AM_NS = "http://www.smpte-ra.org/schemas/429-9/2007/AM";
  constexpr const char* INTEROP_CPL_NS = "http://www.digicine.com/PROTO-ASDCP-CPL-20040511#";
  constexpr const char* INTEROP_PKL_NS = "http://www.digicine.com/PROTO-ASDCP-PKL-20040311#";
  constexpr const char* INTEROP_AM_NS = "http://www.digicine.com/PROTO-ASDCP-AM-20040311#";

  // UUID pattern: urn:uuid:XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX
  const std::regex UUID_PATTERN(
      R"(urn:uuid:[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})");

  // DCI Content Title pattern (SMPTE 429-7 Annex B)
  // FTR_TITLE_VERSION_FACILITY_DATE_TYPE_RATIO_AUDIO_RESOLUTION_STUDIO_LANGUAGE
  const std::regex DCI_TITLE_PATTERN(R"([A-Za-z0-9_-]+)");

  // Valid edit rates for DCI
  const std::set<std::string> VALID_EDIT_RATES = {"24 1", "25 1",       "30 1",       "48 1",
                                                  "60 1", "24000 1001", "30000 1001", "60000 1001"};

  // Valid frame rates
  const std::set<std::string> VALID_FRAME_RATES = {"24 1", "25 1", "30 1", "48 1", "60 1"};

  void check_uuid(const std::string& value, const std::string& context,
                  const std::filesystem::path& file, std::vector<Note>& notes)
  {
    if(!value.empty() && !std::regex_match(value, UUID_PATTERN))
    {
      notes.push_back({Severity::warning, Code::invalid_uuid,
                       "Invalid UUID format in " + context + ": " + value, file});
    }
  }

  void check_namespace(xmlNodePtr root, Standard standard, const std::filesystem::path& file,
                       std::vector<Note>& notes)
  {
    std::string ns = get_ns_uri(root);
    const char* name = reinterpret_cast<const char*>(root->name);

    if(standard == Standard::smpte)
    {
      if(std::strcmp(name, "CompositionPlaylist") == 0 && ns != SMPTE_CPL_NS)
      {
        notes.push_back({Severity::error, Code::smpte_namespace_wrong,
                         "CPL uses non-SMPTE namespace: " + ns, file});
      }
      else if(std::strcmp(name, "PackingList") == 0 && ns != SMPTE_PKL_NS)
      {
        notes.push_back({Severity::error, Code::smpte_namespace_wrong,
                         "PKL uses non-SMPTE namespace: " + ns, file});
      }
      else if(std::strcmp(name, "AssetMap") == 0 && ns != SMPTE_AM_NS)
      {
        notes.push_back({Severity::error, Code::smpte_namespace_wrong,
                         "ASSETMAP uses non-SMPTE namespace: " + ns, file});
      }
    }
    else if(standard == Standard::interop)
    {
      if(std::strcmp(name, "CompositionPlaylist") == 0 && ns != INTEROP_CPL_NS)
      {
        notes.push_back({Severity::error, Code::interop_namespace_wrong,
                         "CPL uses non-Interop namespace: " + ns, file});
      }
      else if(std::strcmp(name, "PackingList") == 0 && ns != INTEROP_PKL_NS)
      {
        notes.push_back({Severity::error, Code::interop_namespace_wrong,
                         "PKL uses non-Interop namespace: " + ns, file});
      }
    }
  }

  void check_cpl_compliance(const std::filesystem::path& cpl_path, Standard standard, bool strict,
                            std::vector<Note>& notes)
  {
    xmlDocPtr doc = xmlReadFile(cpl_path.string().c_str(), nullptr,
                                XML_PARSE_NONET | XML_PARSE_NOERROR | XML_PARSE_NOWARNING);
    if(!doc)
      return;

    xmlNodePtr root = xmlDocGetRootElement(doc);
    if(!root)
    {
      xmlFreeDoc(doc);
      return;
    }

    // Check namespace
    check_namespace(root, standard, cpl_path, notes);

    // Check CPL Id is valid UUID
    auto id_node = find_child(root, "Id");
    if(id_node)
      check_uuid(get_text(id_node), "CPL Id", cpl_path, notes);

    // Check ContentTitleText exists
    auto title_node = find_child(root, "ContentTitleText");
    if(!title_node)
    {
      notes.push_back({Severity::warning, Code::missing_required_element,
                       "CPL missing ContentTitleText", cpl_path});
    }

    // Check IssueDate
    auto issue_date = find_child(root, "IssueDate");
    if(!issue_date)
    {
      notes.push_back(
          {Severity::warning, Code::missing_required_element, "CPL missing IssueDate", cpl_path});
    }

    // Check ContentKind
    auto content_kind = find_child(root, "ContentKind");
    if(content_kind && strict)
    {
      std::string kind = get_text(content_kind);
      static const std::set<std::string> valid_kinds = {
          "feature", "trailer",      "test", "teaser", "rating", "advertisement",
          "short",   "transitional", "psa",  "policy", "episode"};
      if(valid_kinds.find(kind) == valid_kinds.end())
      {
        notes.push_back({Severity::warning, Code::smpte_naming_violation,
                         "Non-standard ContentKind: " + kind, cpl_path});
      }
    }

    // Check ReelList (DCP) or SegmentList (IMF)
    auto reel_list = find_child(root, "ReelList");
    bool is_imf_segments = false;
    if(!reel_list)
    {
      reel_list = find_child(root, "SegmentList");
      is_imf_segments = (reel_list != nullptr);
    }
    if(reel_list)
    {
      int reel_idx = 0;
      for(auto reel = reel_list->children; reel; reel = reel->next)
      {
        if(reel->type != XML_ELEMENT_NODE)
          continue;
        auto node_name = reinterpret_cast<const char*>(reel->name);
        if(std::strcmp(node_name, "Reel") != 0 && std::strcmp(node_name, "Segment") != 0)
          continue;
        reel_idx++;

        auto reel_id = find_child(reel, "Id");
        if(reel_id)
          check_uuid(get_text(reel_id), "Reel " + std::to_string(reel_idx) + " Id", cpl_path,
                     notes);

        // DCP uses AssetList; IMF uses SequenceList
        auto asset_list = find_child(reel, "AssetList");
        xmlNodePtr main_pic = nullptr;
        xmlNodePtr main_snd = nullptr;

        if(asset_list)
        {
          main_pic = find_child(asset_list, "MainPicture");
          main_snd = find_child(asset_list, "MainSound");
        }
        else if(is_imf_segments)
        {
          // IMF: Segment/SequenceList/MainImageSequence/ResourceList/Resource
          auto seq_list = find_child(reel, "SequenceList");
          if(seq_list)
          {
            auto img_seq = find_child(seq_list, "MainImageSequence");
            if(img_seq)
            {
              auto res_list = find_child(img_seq, "ResourceList");
              if(res_list)
              {
                // Use first Resource as the "main picture" for compliance checks
                for(auto res = res_list->children; res; res = res->next)
                {
                  if(res->type == XML_ELEMENT_NODE)
                  {
                    main_pic = res;
                    break;
                  }
                }
              }
            }
            auto aud_seq = find_child(seq_list, "MainAudioSequence");
            if(aud_seq)
            {
              auto res_list = find_child(aud_seq, "ResourceList");
              if(res_list)
              {
                for(auto res = res_list->children; res; res = res->next)
                {
                  if(res->type == XML_ELEMENT_NODE)
                  {
                    main_snd = res;
                    break;
                  }
                }
              }
            }
          }
        }

        if(!main_pic)
          continue;

        // Check picture element (MainPicture for DCP, Resource for IMF)
        {
          auto edit_rate = find_child(main_pic, "EditRate");
          if(edit_rate && strict)
          {
            std::string rate = get_text(edit_rate);
            if(VALID_EDIT_RATES.find(rate) == VALID_EDIT_RATES.end())
            {
              notes.push_back(
                  {Severity::warning, Code::cpl_invalid_edit_rate,
                   "Non-standard EditRate in Reel " + std::to_string(reel_idx) + ": " + rate,
                   cpl_path});
            }
          }

          auto frame_rate = find_child(main_pic, "FrameRate");
          if(frame_rate && strict)
          {
            std::string rate = get_text(frame_rate);
            if(VALID_FRAME_RATES.find(rate) == VALID_FRAME_RATES.end())
            {
              notes.push_back(
                  {Severity::warning, Code::cpl_invalid_frame_rate,
                   "Non-standard FrameRate in Reel " + std::to_string(reel_idx) + ": " + rate,
                   cpl_path});
            }
          }

          // Check Duration > 0 (DCP uses Duration, IMF uses IntrinsicDuration)
          auto duration = find_child(main_pic, "Duration");
          if(!duration)
            duration = find_child(main_pic, "IntrinsicDuration");
          if(duration)
          {
            int dur = std::atoi(get_text(duration).c_str());
            if(dur <= 0)
            {
              notes.push_back({Severity::error, Code::cpl_invalid_duration,
                               "Zero or negative Duration in Reel " + std::to_string(reel_idx),
                               cpl_path});
            }
          }

          // Check IntrinsicDuration >= Duration
          auto intrinsic = find_child(main_pic, "IntrinsicDuration");
          auto dur_node = find_child(main_pic, "Duration");
          auto entry_pt = find_child(main_pic, "EntryPoint");
          if(intrinsic && dur_node && entry_pt)
          {
            int id_val = std::atoi(get_text(intrinsic).c_str());
            int dur_val = std::atoi(get_text(dur_node).c_str());
            int ep_val = std::atoi(get_text(entry_pt).c_str());
            if(ep_val + dur_val > id_val)
            {
              notes.push_back({Severity::error, Code::cpl_invalid_duration,
                               "EntryPoint + Duration exceeds IntrinsicDuration in Reel " +
                                   std::to_string(reel_idx),
                               cpl_path});
            }
          }
        }

        // Check sound has matching duration
        if(main_snd)
        {
          auto pic_dur = find_child(main_pic, "Duration");
          if(!pic_dur)
            pic_dur = find_child(main_pic, "IntrinsicDuration");
          auto snd_dur = find_child(main_snd, "Duration");
          if(!snd_dur)
            snd_dur = find_child(main_snd, "IntrinsicDuration");
          if(pic_dur && snd_dur)
          {
            int pd = std::atoi(get_text(pic_dur).c_str());
            int sd = std::atoi(get_text(snd_dur).c_str());
            if(pd != sd && strict)
            {
              notes.push_back({Severity::warning, Code::cpl_mismatched_durations,
                               "Picture/Sound duration mismatch in Reel " +
                                   std::to_string(reel_idx) + " (" + std::to_string(pd) + " vs " +
                                   std::to_string(sd) + ")",
                               cpl_path});
            }
          }
        }
      }
    }

    xmlFreeDoc(doc);
  }

  void check_pkl_compliance(const std::filesystem::path& pkl_path, Standard standard,
                            std::vector<Note>& notes)
  {
    xmlDocPtr doc = xmlReadFile(pkl_path.string().c_str(), nullptr,
                                XML_PARSE_NONET | XML_PARSE_NOERROR | XML_PARSE_NOWARNING);
    if(!doc)
      return;

    xmlNodePtr root = xmlDocGetRootElement(doc);
    if(!root)
    {
      xmlFreeDoc(doc);
      return;
    }

    check_namespace(root, standard, pkl_path, notes);

    // Check PKL Id
    auto id_node = find_child(root, "Id");
    if(id_node)
      check_uuid(get_text(id_node), "PKL Id", pkl_path, notes);

    // Check required elements
    auto issue_date = find_child(root, "IssueDate");
    if(!issue_date)
    {
      notes.push_back(
          {Severity::warning, Code::missing_required_element, "PKL missing IssueDate", pkl_path});
    }

    auto issuer = find_child(root, "Issuer");
    if(!issuer)
    {
      notes.push_back(
          {Severity::warning, Code::missing_required_element, "PKL missing Issuer", pkl_path});
    }

    auto creator = find_child(root, "Creator");
    if(!creator)
    {
      notes.push_back(
          {Severity::warning, Code::missing_required_element, "PKL missing Creator", pkl_path});
    }

    // Check asset list entries
    auto asset_list = find_child(root, "AssetList");
    if(asset_list)
    {
      for(auto asset = asset_list->children; asset; asset = asset->next)
      {
        if(asset->type != XML_ELEMENT_NODE)
          continue;
        if(std::strcmp(reinterpret_cast<const char*>(asset->name), "Asset") != 0)
          continue;

        auto asset_id = find_child(asset, "Id");
        if(asset_id)
          check_uuid(get_text(asset_id), "PKL Asset Id", pkl_path, notes);

        auto hash = find_child(asset, "Hash");
        if(!hash)
        {
          notes.push_back({Severity::warning, Code::missing_required_element,
                           "PKL Asset missing Hash element", pkl_path});
        }

        auto type = find_child(asset, "Type");
        if(!type)
        {
          notes.push_back({Severity::warning, Code::missing_required_element,
                           "PKL Asset missing Type element", pkl_path});
        }
      }
    }

    xmlFreeDoc(doc);
  }

  void check_assetmap_compliance(const std::filesystem::path& am_path, Standard standard,
                                 std::vector<Note>& notes)
  {
    xmlDocPtr doc = xmlReadFile(am_path.string().c_str(), nullptr,
                                XML_PARSE_NONET | XML_PARSE_NOERROR | XML_PARSE_NOWARNING);
    if(!doc)
      return;

    xmlNodePtr root = xmlDocGetRootElement(doc);
    if(!root)
    {
      xmlFreeDoc(doc);
      return;
    }

    check_namespace(root, standard, am_path, notes);

    // Check ASSETMAP Id
    auto id_node = find_child(root, "Id");
    if(id_node)
      check_uuid(get_text(id_node), "ASSETMAP Id", am_path, notes);

    // Check VolumeCount == 1
    auto vol_count = find_child(root, "VolumeCount");
    if(vol_count)
    {
      int vc = std::atoi(get_text(vol_count).c_str());
      if(vc != 1)
      {
        notes.push_back({Severity::warning, Code::smpte_naming_violation,
                         "VolumeCount != 1 (multi-volume DCPs are unusual): " + std::to_string(vc),
                         am_path});
      }
    }

    // Check all asset Ids are valid UUIDs
    auto asset_list = find_child(root, "AssetList");
    if(asset_list)
    {
      std::set<std::string> seen_ids;
      for(auto asset = asset_list->children; asset; asset = asset->next)
      {
        if(asset->type != XML_ELEMENT_NODE)
          continue;
        if(std::strcmp(reinterpret_cast<const char*>(asset->name), "Asset") != 0)
          continue;

        auto asset_id = find_child(asset, "Id");
        if(asset_id)
        {
          std::string id = get_text(asset_id);
          check_uuid(id, "ASSETMAP Asset Id", am_path, notes);

          // Check for duplicates
          if(!id.empty())
          {
            if(seen_ids.count(id))
            {
              notes.push_back({Severity::error, Code::duplicate_asset_id,
                               "Duplicate asset Id: " + id, am_path});
            }
            seen_ids.insert(id);
          }
        }
      }
    }

    xmlFreeDoc(doc);
  }

} // namespace

std::vector<Note> check_smpte_compliance(const std::filesystem::path& dcp_dir, Standard standard,
                                         bool strict)
{
  std::vector<Note> notes;
  namespace fs = std::filesystem;

  // Check ASSETMAP naming (SMPTE uses ASSETMAP.xml, Interop uses ASSETMAP)
  auto am_xml = dcp_dir / "ASSETMAP.xml";
  auto am_plain = dcp_dir / "ASSETMAP";

  if(standard == Standard::smpte)
  {
    if(fs::exists(am_plain) && !fs::exists(am_xml) && strict)
    {
      notes.push_back({Severity::warning, Code::smpte_naming_violation,
                       "SMPTE DCP should use ASSETMAP.xml (not ASSETMAP)", dcp_dir});
    }
  }

  // Check VOLINDEX.xml presence (SMPTE requirement)
  if(standard == Standard::smpte && strict)
  {
    auto volindex = dcp_dir / "VOLINDEX.xml";
    auto volindex2 = dcp_dir / "VOLINDEX";
    if(!fs::exists(volindex) && !fs::exists(volindex2))
    {
      notes.push_back({Severity::warning, Code::smpte_naming_violation,
                       "SMPTE DCP missing VOLINDEX.xml", dcp_dir});
    }
  }

  // Validate ASSETMAP
  fs::path am_path = fs::exists(am_xml) ? am_xml : am_plain;
  if(fs::exists(am_path))
  {
    check_assetmap_compliance(am_path, standard, notes);
  }

  // Find and validate PKLs and CPLs
  for(auto& entry : fs::directory_iterator(dcp_dir))
  {
    if(!entry.is_regular_file())
      continue;
    auto ext = entry.path().extension().string();
    if(ext != ".xml" && ext != ".XML")
      continue;

    xmlDocPtr doc = xmlReadFile(entry.path().string().c_str(), nullptr,
                                XML_PARSE_NONET | XML_PARSE_NOERROR | XML_PARSE_NOWARNING);
    if(!doc)
      continue;

    xmlNodePtr root = xmlDocGetRootElement(doc);
    if(root)
    {
      const char* name = reinterpret_cast<const char*>(root->name);
      if(std::strcmp(name, "PackingList") == 0)
      {
        xmlFreeDoc(doc);
        check_pkl_compliance(entry.path(), standard, notes);
        continue;
      }
      if(std::strcmp(name, "CompositionPlaylist") == 0)
      {
        xmlFreeDoc(doc);
        check_cpl_compliance(entry.path(), standard, strict, notes);
        continue;
      }
    }
    xmlFreeDoc(doc);
  }

  return notes;
}

} // namespace dcpdoctor
