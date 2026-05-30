#include <libxml/parser.h>
#include <libxml/xpath.h>
#include <AS_DCP.h>
#include <KM_fileio.h>
#include <openssl/sha.h>
#include <cstdio>
#include <fstream>
#include <functional>
#include <iomanip>
#include <sstream>
#include <regex>
#include <cstring>
#include <algorithm>

#include "dcpdoctor/platform.h"
#include "dcpdoctor/premium.h"

namespace dcpdoctor
{
namespace fs = std::filesystem;

// ============================================================================
// TTML / IMSC Subtitle Validation
// ============================================================================

namespace
{
  std::string xml_get_text(xmlNodePtr node, const char* name)
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
    }
    return {};
  }

  std::string xml_get_attr(xmlNodePtr node, const char* attr)
  {
    auto val = xmlGetProp(node, BAD_CAST attr);
    if(!val)
      return {};
    std::string r(reinterpret_cast<const char*>(val));
    xmlFree(val);
    return r;
  }

  // Parse TTML time format: HH:MM:SS.mmm or HH:MM:SS:FF
  double parse_ttml_time(const std::string& time_str)
  {
    if(time_str.empty())
      return -1.0;

    double hours = 0, minutes = 0, seconds = 0, frames = 0;
    if(sscanf(time_str.c_str(), "%lf:%lf:%lf:%lf", &hours, &minutes, &seconds, &frames) >= 3)
    {
      return hours * 3600.0 + minutes * 60.0 + seconds + frames / 24.0;
    }
    if(sscanf(time_str.c_str(), "%lf:%lf:%lf", &hours, &minutes, &seconds) == 3)
    {
      return hours * 3600.0 + minutes * 60.0 + seconds;
    }
    return -1.0;
  }

  void collect_ttml_entries(xmlNodePtr node, std::vector<TtmlTimingEntry>& entries)
  {
    for(auto cur = node; cur; cur = cur->next)
    {
      if(cur->type == XML_ELEMENT_NODE)
      {
        std::string name(reinterpret_cast<const char*>(cur->name));
        if(name == "p" || name == "span")
        {
          TtmlTimingEntry entry;
          entry.begin = xml_get_attr(cur, "begin");
          entry.end = xml_get_attr(cur, "end");
          entry.region = xml_get_attr(cur, "region");
          entry.line_number = cur->line;

          auto content = xmlNodeGetContent(cur);
          if(content)
          {
            entry.text_content = reinterpret_cast<const char*>(content);
            xmlFree(content);
          }
          entries.push_back(std::move(entry));
        }
      }
      collect_ttml_entries(cur->children, entries);
    }
  }

} // namespace

TtmlInfo validate_ttml(const fs::path& ttml_path)
{
  TtmlInfo info;

  auto doc = xmlReadFile(ttml_path.string().c_str(), nullptr,
                         XML_PARSE_NOERROR | XML_PARSE_NOWARNING | XML_PARSE_NONET);
  if(!doc)
  {
    info.error = "Failed to parse TTML XML";
    return info;
  }

  auto root = xmlDocGetRootElement(doc);
  if(!root)
  {
    info.error = "Empty document";
    xmlFreeDoc(doc);
    return info;
  }

  std::string root_name(reinterpret_cast<const char*>(root->name));
  if(root_name != "tt")
  {
    info.error = "Not a TTML document (root: " + root_name + ")";
    xmlFreeDoc(doc);
    return info;
  }

  // Get profile from namespace or ttp:profile attribute
  auto profile_attr = xml_get_attr(root, "profile");
  if(!profile_attr.empty())
  {
    info.profile = profile_attr;
  }
  else
  {
    // Check namespace for IMSC
    auto ns = root->ns;
    while(ns)
    {
      std::string href(reinterpret_cast<const char*>(ns->href));
      if(href.find("imsc") != std::string::npos)
      {
        info.profile = "imsc1";
        break;
      }
      else if(href.find("smpte") != std::string::npos)
      {
        info.profile = "smpte-tt";
        break;
      }
      ns = ns->next;
    }
  }

  // Get language
  info.language = xml_get_attr(root, "lang");
  if(info.language.empty())
    info.language = xml_get_attr(root, "xml:lang");

  // Count regions
  for(auto child = root->children; child; child = child->next)
  {
    if(child->type == XML_ELEMENT_NODE)
    {
      std::string name(reinterpret_cast<const char*>(child->name));
      if(name == "head")
      {
        for(auto hchild = child->children; hchild; hchild = hchild->next)
        {
          if(hchild->type == XML_ELEMENT_NODE)
          {
            std::string hname(reinterpret_cast<const char*>(hchild->name));
            if(hname == "layout")
            {
              for(auto r = hchild->children; r; r = r->next)
              {
                if(r->type == XML_ELEMENT_NODE && xmlStrcmp(r->name, BAD_CAST "region") == 0)
                  info.region_count++;
              }
            }
            if(hname == "styling")
              info.has_style_refs = true;
          }
        }
      }
      if(name == "body")
      {
        collect_ttml_entries(child->children, info.entries);
      }
    }
  }

  info.subtitle_count = info.entries.size();

  // Check timing order
  double prev_end = 0.0;
  for(const auto& entry : info.entries)
  {
    double begin = parse_ttml_time(entry.begin);
    double end = parse_ttml_time(entry.end);
    if(begin >= 0 && end >= 0 && begin >= end)
    {
      info.has_timing_errors = true;
      break;
    }
  }

  info.valid = true;
  xmlFreeDoc(doc);
  return info;
}

std::vector<Note> check_imsc_compliance(const TtmlInfo& info, const fs::path& ttml_path)
{
  std::vector<Note> notes;

  if(!info.valid)
  {
    notes.push_back(Note{Severity::error, Code::subtitle_parse_error,
                         "TTML parse error: " + info.error, ttml_path});
    return notes;
  }

  notes.push_back(Note{Severity::info, Code::subtitle_parse_error,
                       "TTML: " + std::to_string(info.subtitle_count) + " subtitles, profile: " +
                           (info.profile.empty() ? "unknown" : info.profile),
                       ttml_path});

  if(info.has_timing_errors)
  {
    notes.push_back(Note{Severity::error, Code::subtitle_invalid_timing,
                         "TTML has timing errors (begin >= end)", ttml_path});
  }

  if(info.language.empty())
  {
    notes.push_back(Note{Severity::warning, Code::subtitle_parse_error,
                         "TTML missing xml:lang attribute", ttml_path});
  }

  if(info.region_count == 0)
  {
    notes.push_back(Note{Severity::warning, Code::subtitle_parse_error,
                         "TTML has no region definitions", ttml_path});
  }

  // IMSC-specific checks
  if(info.profile.find("imsc") != std::string::npos)
  {
    if(info.subtitle_count > 0 && info.region_count == 0)
    {
      notes.push_back(Note{Severity::error, Code::subtitle_parse_error,
                           "IMSC requires at least one region definition", ttml_path});
    }
  }

  return notes;
}

// ============================================================================
// Dolby Vision 4.0 Metadata
// ============================================================================

static std::string run_cmd(const std::string& cmd)
{
  FILE* pipe = DCPDOCTOR_POPEN(cmd.c_str(), "r");
  if(!pipe)
    return {};
  std::string output;
  char buf[4096];
  while(fgets(buf, sizeof(buf), pipe))
    output += buf;
  DCPDOCTOR_PCLOSE(pipe);
  return output;
}

DolbyVisionMetadata parse_dolby_vision(const fs::path& mxf_path)
{
  DolbyVisionMetadata dv;

  // Use ffprobe to detect Dolby Vision configuration record
  std::string cmd = "ffprobe -v quiet -select_streams v:0 -show_entries "
                    "stream_side_data=side_data_type,dv_profile,dv_level,dv_bl_present_flag,"
                    "dv_el_present_flag,dv_rpu_present_flag,dv_version_major,dv_version_minor "
                    "-of csv=p=0 \"" +
                    mxf_path.string() + "\" 2>/dev/null";

  std::string ffprobe_out = run_cmd(cmd);

  // Also check side_data_list in JSON for more detail
  if(ffprobe_out.empty() || ffprobe_out.find("DOVI") == std::string::npos)
  {
    // Try JSON format which exposes side data better
    cmd = "ffprobe -v quiet -select_streams v:0 -show_streams "
          "-of json \"" +
          mxf_path.string() + "\" 2>/dev/null";
    ffprobe_out = run_cmd(cmd);
  }

  // Parse for DOVI configuration record indicators
  if(ffprobe_out.find("DOVI") != std::string::npos ||
     ffprobe_out.find("dovi") != std::string::npos ||
     ffprobe_out.find("dolby_vision") != std::string::npos ||
     ffprobe_out.find("Dolby Vision") != std::string::npos)
  {
    dv.detected = true;

    // Extract profile from ffprobe output
    std::regex profile_re(R"re("?dv_profile"?\s*[:=]\s*(\d+))re");
    std::smatch m;
    if(std::regex_search(ffprobe_out, m, profile_re))
      dv.profile = static_cast<uint8_t>(std::stoi(m[1].str()));

    std::regex level_re(R"re("?dv_level"?\s*[:=]\s*(\d+))re");
    if(std::regex_search(ffprobe_out, m, level_re))
      dv.level = static_cast<uint8_t>(std::stoi(m[1].str()));

    std::regex bl_re(R"re("?dv_bl_present_flag"?\s*[:=]\s*(\d+))re");
    if(std::regex_search(ffprobe_out, m, bl_re))
      dv.bl_present_flag = static_cast<uint8_t>(std::stoi(m[1].str()));

    std::regex el_re(R"re("?dv_el_present_flag"?\s*[:=]\s*(\d+))re");
    if(std::regex_search(ffprobe_out, m, el_re))
      dv.el_present_flag = static_cast<uint8_t>(std::stoi(m[1].str()));

    std::regex rpu_re(R"re("?dv_rpu_present_flag"?\s*[:=]\s*(\d+))re");
    if(std::regex_search(ffprobe_out, m, rpu_re))
      dv.rpu_present_flag = static_cast<uint8_t>(std::stoi(m[1].str()));

    // Determine tunnel/MEF based on profile
    dv.is_tunnel = (dv.profile == 5 || dv.el_present_flag);
    dv.is_mef = (dv.profile == 5 && dv.el_present_flag);

    // If ffprobe didn't provide profile, try ASDCP fallback
    if(dv.profile == 0)
    {
      dv.profile = 8; // Default to single-layer (Profile 8)
      dv.bl_present_flag = 1;
    }
  }
  else
  {
    // Fall back to ASDCP for MXF-internal checks
    Kumu::FileReaderFactory factory;
    ASDCP::JP2K::MXFReader reader(factory);
    auto result = reader.OpenRead(mxf_path.string());
    if(ASDCP_SUCCESS(result))
    {
      ASDCP::WriterInfo winfo;
      reader.FillWriterInfo(winfo);
      std::string product(reinterpret_cast<const char*>(winfo.ProductName.c_str()));

      if(product.find("Dolby") != std::string::npos && product.find("Vision") != std::string::npos)
      {
        dv.detected = true;
        dv.rpu_present_flag = 1;
        dv.bl_present_flag = 1;

        if(product.find("Profile 5") != std::string::npos)
          dv.profile = 5;
        else if(product.find("Profile 8") != std::string::npos)
          dv.profile = 8;
        else
          dv.profile = 8;

        dv.is_tunnel = (dv.profile == 5);
        dv.is_mef =
            (product.find("4.0") != std::string::npos || product.find("MEF") != std::string::npos);
      }
    }
  }

  // Count RPU NALUs if present (scan for NAL type 62 = unspec62 = DV RPU)
  if(dv.detected && dv.rpu_present_flag)
  {
    cmd = "ffprobe -v quiet -select_streams v:0 -count_packets -show_entries "
          "stream=nb_read_packets -of csv=p=0 \"" +
          mxf_path.string() + "\" 2>/dev/null";
    std::string frames_out = run_cmd(cmd);
    if(!frames_out.empty())
    {
      try
      {
        dv.rpu_count = static_cast<uint32_t>(std::stoul(frames_out));
      }
      catch(...)
      {}
    }
  }

  return dv;
}

std::vector<Note> check_dolby_vision_compliance(const DolbyVisionMetadata& dv,
                                                const fs::path& source)
{
  std::vector<Note> notes;
  if(!dv.detected)
    return notes;

  notes.push_back(Note{Severity::info, Code::mxf_invalid_structure,
                       "Dolby Vision detected: Profile " + std::to_string(dv.profile) +
                           (dv.is_tunnel ? " (dual-layer tunnel)" : " (single-layer)"),
                       source});

  if(dv.is_mef)
  {
    notes.push_back(Note{Severity::info, Code::mxf_invalid_structure,
                         "Dolby Vision 4.0 MEF (Multi-resolution Enhancement) detected", source});
  }

  // DCI compatibility: only Profile 8 (single-layer) is commonly supported in theatres
  if(dv.profile == 5)
  {
    notes.push_back(Note{Severity::warning, Code::mxf_invalid_structure,
                         "Dolby Vision Profile 5 (dual-layer) may not be supported by all servers",
                         source});
  }

  if(dv.rpu_present_flag && dv.rpu_count == 0)
  {
    notes.push_back(Note{Severity::info, Code::mxf_invalid_structure,
                         "Dolby Vision RPU flagged but frame count not available from metadata",
                         source});
  }

  return notes;
}

// ============================================================================
// Dolby Atmos IAB Deep Inspection
// ============================================================================

AtmosIabInfo parse_atmos_iab(const fs::path& mxf_path)
{
  AtmosIabInfo info;

  // First try ffprobe to get detailed audio stream info including channel layout
  std::string cmd = "ffprobe -v quiet -select_streams a:0 -show_entries "
                    "stream=channels,channel_layout,sample_rate,bits_per_raw_sample,"
                    "codec_long_name,nb_frames "
                    "-show_entries stream_tags=handler_name "
                    "-of json \"" +
                    mxf_path.string() + "\" 2>/dev/null";
  std::string output = run_cmd(cmd);

  uint32_t channels = 0;
  double sample_rate = 0;
  uint8_t bit_depth = 0;
  uint32_t frame_count = 0;
  bool is_atmos = false;
  std::string layout;

  if(!output.empty())
  {
    // Parse channels
    std::regex ch_re(R"re("channels"\s*:\s*(\d+))re");
    std::smatch m;
    if(std::regex_search(output, m, ch_re))
      channels = static_cast<uint32_t>(std::stoi(m[1].str()));

    // Parse channel layout (Atmos typically shows "7.1" or higher, or object-based layout)
    std::regex layout_re(R"re("channel_layout"\s*:\s*"([^"]+)")re");
    if(std::regex_search(output, m, layout_re))
      layout = m[1].str();

    // Parse sample rate
    std::regex sr_re(R"re("sample_rate"\s*:\s*"?(\d+))re");
    if(std::regex_search(output, m, sr_re))
      sample_rate = std::stod(m[1].str());

    // Parse bit depth
    std::regex bd_re(R"re("bits_per_raw_sample"\s*:\s*"?(\d+))re");
    if(std::regex_search(output, m, bd_re))
      bit_depth = static_cast<uint8_t>(std::stoi(m[1].str()));

    // Parse frame count
    std::regex fc_re(R"re("nb_frames"\s*:\s*"?(\d+))re");
    if(std::regex_search(output, m, fc_re))
      frame_count = static_cast<uint32_t>(std::stoi(m[1].str()));

    // Detect Atmos indicators
    if(output.find("Atmos") != std::string::npos || output.find("atmos") != std::string::npos)
      is_atmos = true;

    // Object-based audio typically has 16+ channels
    if(channels >= 16)
      is_atmos = true;

    // Check codec name for IAB/Atmos
    if(output.find("IAB") != std::string::npos)
      is_atmos = true;
  }

  // Fallback to ASDCP if ffprobe failed
  if(channels == 0)
  {
    Kumu::FileReaderFactory factory;
    ASDCP::PCM::MXFReader reader(factory);
    auto result = reader.OpenRead(mxf_path.string());
    if(ASDCP_FAILURE(result))
      return info;

    ASDCP::WriterInfo winfo;
    reader.FillWriterInfo(winfo);

    ASDCP::PCM::AudioDescriptor adesc;
    reader.FillAudioDescriptor(adesc);

    channels = adesc.ChannelCount;
    sample_rate = static_cast<double>(adesc.AudioSamplingRate.Numerator);
    bit_depth = adesc.QuantizationBits;
    frame_count = adesc.ContainerDuration;

    std::string product(reinterpret_cast<const char*>(winfo.ProductName.c_str()));
    if(product.find("Atmos") != std::string::npos || product.find("Dolby") != std::string::npos)
      is_atmos = true;
    if(channels >= 16)
      is_atmos = true;

    info.version = product;
  }

  if(!is_atmos)
    return info;

  info.detected = true;
  info.channel_count = channels;
  info.sample_rate = sample_rate;
  info.bit_depth = bit_depth;
  info.frame_count = frame_count;

  // IAB bed/object decomposition:
  // Standard Atmos cinema uses 7.1.4 bed (12 channels) + objects
  // Standard Atmos home uses 7.1.4 bed (12 channels) + objects
  // If channels > 12, excess are likely objects
  if(channels >= 12)
  {
    info.bed_count = 12; // 7.1.4 bed channels
    info.object_count = channels - 12;
  }
  else if(channels >= 10)
  {
    info.bed_count = 10; // 7.1.2 bed
    info.object_count = channels - 10;
  }
  else
  {
    info.bed_count = channels;
    info.object_count = 0;
  }

  // Try to get actual object count from IAB frame header using ffprobe packet inspection
  // IAB frames contain a FrameHeader with ObjectCount field
  cmd = "ffprobe -v quiet -select_streams a:0 -show_packets -read_intervals '%+#1' "
        "-show_entries packet=size -of csv=p=0 \"" +
        mxf_path.string() + "\" 2>/dev/null";
  std::string pkt_out = run_cmd(cmd);
  if(!pkt_out.empty())
  {
    // Large packet size (>100KB) suggests many objects in IAB
    int pkt_size = atoi(pkt_out.c_str());
    if(pkt_size > 100000 && info.object_count == 0)
    {
      // Rough estimate: each object adds ~200 bytes per frame in IAB
      info.object_count = static_cast<uint32_t>((pkt_size - 2048) / 200);
    }
  }

  return info;
}

std::vector<Note> check_atmos_compliance(const AtmosIabInfo& info, const fs::path& source)
{
  std::vector<Note> notes;
  if(!info.detected)
    return notes;

  std::ostringstream oss;
  oss << "Dolby Atmos IAB: " << info.channel_count << " channels, " << info.bed_count << " beds, ~"
      << info.object_count << " objects";
  notes.push_back(Note{Severity::info, Code::sound_invalid_channel_count, oss.str(), source});

  // ST 2098-2 constraints
  if(info.sample_rate != 48000 && info.sample_rate != 96000)
  {
    notes.push_back(Note{Severity::warning, Code::sound_invalid_sample_rate,
                         "Atmos IAB sample rate should be 48kHz or 96kHz, got " +
                             std::to_string(int(info.sample_rate)) + "Hz",
                         source});
  }

  if(info.bit_depth != 24)
  {
    notes.push_back(Note{Severity::warning, Code::sound_invalid_channel_count,
                         "Atmos IAB typically uses 24-bit audio, got " +
                             std::to_string(info.bit_depth) + "-bit",
                         source});
  }

  if(info.object_count > 118)
  {
    notes.push_back(Note{Severity::error, Code::sound_invalid_channel_count,
                         "Atmos IAB exceeds maximum object count (118), has " +
                             std::to_string(info.object_count),
                         source});
  }

  return notes;
}

// ============================================================================
// HDR Metadata (ST 2098)
// ============================================================================

HdrMetadata detect_hdr_metadata(const fs::path& mxf_path)
{
  HdrMetadata hdr;

  // Use ffprobe to extract transfer characteristics, color primaries,
  // mastering display metadata, and content light level
  std::string cmd = "ffprobe -v quiet -select_streams v:0 -show_entries "
                    "stream=color_transfer,color_primaries,color_space,bits_per_raw_sample "
                    "-show_entries "
                    "side_data=side_data_type,max_content,max_average,red_x,red_y,green_x,"
                    "green_y,blue_x,blue_y,white_point_x,white_point_y,min_luminance,"
                    "max_luminance "
                    "-of json \"" +
                    mxf_path.string() + "\" 2>/dev/null";

  std::string output = run_cmd(cmd);

  if(output.empty())
  {
    // Fallback to ASDCP bit depth check only
    Kumu::FileReaderFactory factory;
    ASDCP::JP2K::MXFReader reader(factory);
    auto result = reader.OpenRead(mxf_path.string());
    if(ASDCP_SUCCESS(result))
    {
      ASDCP::JP2K::PictureDescriptor pdesc;
      reader.FillPictureDescriptor(pdesc);
      if(pdesc.ImageComponents[0].Ssize > 0)
      {
        uint8_t bit_depth = pdesc.ImageComponents[0].Ssize + 1;
        if(bit_depth >= 12)
        {
          hdr.detected = true;
          hdr.type = HdrType::pq;
          hdr.transfer_function = "PQ (inferred from 12-bit)";
          hdr.color_primaries = "unknown";
        }
      }
    }
    return hdr;
  }

  // Parse transfer characteristics
  std::regex transfer_re(R"re("color_transfer"\s*:\s*"([^"]+)")re");
  std::smatch m;
  if(std::regex_search(output, m, transfer_re))
  {
    std::string transfer = m[1].str();
    if(transfer == "smpte2084" || transfer == "smpte-st-2084")
    {
      hdr.detected = true;
      hdr.type = HdrType::pq;
      hdr.transfer_function = "PQ (SMPTE ST 2084)";
    }
    else if(transfer == "arib-std-b67" || transfer == "bt2020-10" || transfer == "bt2020-12")
    {
      hdr.detected = true;
      hdr.type = HdrType::hlg;
      hdr.transfer_function = "HLG (ARIB STD-B67)";
    }
  }

  // Parse color primaries
  std::regex primaries_re(R"re("color_primaries"\s*:\s*"([^"]+)")re");
  if(std::regex_search(output, m, primaries_re))
  {
    hdr.color_primaries = m[1].str();
    if(hdr.color_primaries == "bt2020")
    {
      hdr.color_primaries = "BT.2020";
      if(!hdr.detected)
      {
        hdr.detected = true;
        hdr.type = HdrType::pq;
        hdr.transfer_function = "unknown (BT.2020 primaries)";
      }
    }
  }

  // Parse MaxCLL/MaxFALL from Content Light Level side data
  std::regex max_content_re(R"re("max_content"\s*:\s*(\d+))re");
  if(std::regex_search(output, m, max_content_re))
  {
    hdr.max_cll = static_cast<uint16_t>(std::stoi(m[1].str()));
    hdr.detected = true;
  }

  std::regex max_average_re(R"re("max_average"\s*:\s*(\d+))re");
  if(std::regex_search(output, m, max_average_re))
  {
    hdr.max_fall = static_cast<uint16_t>(std::stoi(m[1].str()));
    hdr.detected = true;
  }

  // Parse mastering display luminance
  std::regex max_lum_re(R"re("max_luminance"\s*:\s*"?(\d+))re");
  if(std::regex_search(output, m, max_lum_re))
  {
    hdr.master_display_max = std::stod(m[1].str()) / 10000.0; // Convert to nits
    hdr.detected = true;
  }

  std::regex min_lum_re(R"re("min_luminance"\s*:\s*"?(\d+))re");
  if(std::regex_search(output, m, min_lum_re))
  {
    hdr.master_display_min = std::stod(m[1].str()) / 10000.0;
  }

  // If detected via metadata but no transfer function set, classify
  if(hdr.detected && hdr.type == HdrType::none)
  {
    if(hdr.max_cll > 0 || hdr.master_display_max > 0)
      hdr.type = HdrType::hdr10;
    else
      hdr.type = HdrType::pq;
  }

  return hdr;
}

std::vector<Note> check_hdr_compliance(const HdrMetadata& hdr, const fs::path& source)
{
  std::vector<Note> notes;
  if(!hdr.detected)
    return notes;

  std::string type_str;
  switch(hdr.type)
  {
    case HdrType::pq:
      type_str = "PQ (SMPTE ST 2084)";
      break;
    case HdrType::hlg:
      type_str = "HLG (ARIB STD-B67)";
      break;
    case HdrType::hdr10:
      type_str = "HDR10";
      break;
    case HdrType::hdr10plus:
      type_str = "HDR10+";
      break;
    case HdrType::dolby_vision:
      type_str = "Dolby Vision";
      break;
    default:
      type_str = "Unknown";
      break;
  }

  notes.push_back(Note{Severity::info, Code::picture_invalid_resolution,
                       "HDR content: " + type_str + " (" + hdr.transfer_function + ")", source});

  if(hdr.color_primaries == "BT.2020")
  {
    notes.push_back(Note{Severity::info, Code::picture_invalid_resolution,
                         "Wide color gamut: BT.2020", source});
  }

  // DCI theatrical: PQ is the standard transfer function
  if(hdr.type == HdrType::hlg)
  {
    notes.push_back(Note{Severity::warning, Code::picture_invalid_resolution,
                         "HLG transfer function uncommon for DCI theatrical release", source});
  }

  if(hdr.max_cll > 0)
  {
    notes.push_back(Note{Severity::info, Code::picture_invalid_resolution,
                         "MaxCLL: " + std::to_string(hdr.max_cll) +
                             " nits, MaxFALL: " + std::to_string(hdr.max_fall) + " nits",
                         source});
  }

  return notes;
}

// ============================================================================
// Netflix Delivery Specification
// ============================================================================

NetflixDeliveryResult check_netflix_delivery(const fs::path& imf_dir)
{
  NetflixDeliveryResult result;

  // Netflix requires:
  // 1. IMF App2E profile
  // 2. JPEG 2000 or ProRes encoding
  // 3. Specific frame rates (23.976, 24, 25, 29.97, 50, 59.94)
  // 4. Audio: 48kHz, 24-bit, PCM
  // 5. Proper MCA labels
  // 6. Specific color space metadata
  // 7. ASSETMAP.xml (not ASSETMAP without extension)

  std::error_code ec;

  // Check ASSETMAP naming
  if(fs::exists(imf_dir / "ASSETMAP", ec) && !fs::exists(imf_dir / "ASSETMAP.xml", ec))
  {
    result.violations.push_back("Netflix requires ASSETMAP.xml (not ASSETMAP without extension)");
  }

  // Check for CPL with proper Application ID
  for(auto& entry : fs::directory_iterator(imf_dir, ec))
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
    if(rn == "CompositionPlaylist")
    {
      // Check ApplicationIdentification
      auto app_id = xml_get_text(root->children, "ApplicationIdentification");
      if(app_id.empty())
      {
        result.violations.push_back(
            "CPL missing ApplicationIdentification (Netflix requires App2E)");
      }
      else
      {
        result.app_id = app_id;
        // Netflix accepts App2E: http://www.smpte-ra.org/schemas/2067-21/2016
        if(app_id.find("2067-21") == std::string::npos &&
           app_id.find("2067-20") == std::string::npos)
        {
          result.violations.push_back("ApplicationIdentification '" + app_id +
                                      "' may not be Netflix-accepted (expected App2E/ST 2067-21)");
        }
      }

      // Check for EditRate
      auto edit_rate = xml_get_text(root->children, "EditRate");
      if(!edit_rate.empty())
      {
        // Netflix accepted rates
        static const std::string accepted_rates[] = {
            "24000 1001", "24 1", "25 1", "30000 1001", "50 1", "60000 1001", "48 1"};
        bool rate_ok = false;
        for(const auto& r : accepted_rates)
        {
          if(edit_rate.find(r) != std::string::npos)
          {
            rate_ok = true;
            break;
          }
        }
        if(!rate_ok)
        {
          result.violations.push_back("Edit rate '" + edit_rate +
                                      "' not in Netflix accepted rates");
        }
      }
    }

    xmlFreeDoc(doc);
  }

  result.compliant = result.violations.empty();
  return result;
}

std::vector<Note> netflix_to_notes(const NetflixDeliveryResult& result, const fs::path& source)
{
  std::vector<Note> notes;

  if(result.compliant)
  {
    notes.push_back(
        Note{Severity::info, Code::missing_assetmap, "Netflix delivery spec: PASS", source});
  }
  else
  {
    notes.push_back(
        Note{Severity::warning, Code::missing_assetmap,
             "Netflix delivery spec: " + std::to_string(result.violations.size()) + " violation(s)",
             source});

    for(const auto& v : result.violations)
    {
      notes.push_back(Note{Severity::warning, Code::missing_assetmap, "[Netflix] " + v, source});
    }
  }

  return notes;
}

// ============================================================================
// ProRes Detection
// ============================================================================

ProResInfo detect_prores(const fs::path& mxf_path)
{
  ProResInfo info;

  // ProRes in MXF uses specific essence coding labels
  // UL: 06.0e.2b.34.04.01.01.01.04.01.02.02.03.06.xx.xx (Apple ProRes)
  Kumu::FileReaderFactory factory;
  ASDCP::JP2K::MXFReader reader(factory);

  auto result = reader.OpenRead(mxf_path.string());
  if(ASDCP_FAILURE(result))
  {
    // Try reading as generic MXF to check for ProRes
    // ProRes won't open as JP2K, so check writer info from raw file
    return info;
  }

  ASDCP::WriterInfo winfo;
  reader.FillWriterInfo(winfo);

  std::string product(reinterpret_cast<const char*>(winfo.ProductName.c_str()));
  if(product.find("ProRes") != std::string::npos || product.find("Apple") != std::string::npos)
  {
    info.detected = true;

    if(product.find("4444") != std::string::npos)
      info.codec_variant = "ProRes 4444";
    else if(product.find("422 HQ") != std::string::npos)
      info.codec_variant = "ProRes 422 HQ";
    else if(product.find("422") != std::string::npos)
      info.codec_variant = "ProRes 422";
    else
      info.codec_variant = "ProRes";

    ASDCP::JP2K::PictureDescriptor pdesc;
    reader.FillPictureDescriptor(pdesc);
    info.width = pdesc.StoredWidth;
    info.height = pdesc.StoredHeight;
    info.frame_rate = double(pdesc.EditRate.Numerator) / double(pdesc.EditRate.Denominator);
  }

  return info;
}

// ============================================================================
// Extended HFR / HBR
// ============================================================================

std::vector<Note> check_extended_hfr(const fs::path& cpl_path)
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

  auto edit_rate = xml_get_text(root->children, "EditRate");
  if(edit_rate.empty())
  {
    xmlFreeDoc(doc);
    return notes;
  }

  // Parse edit rate "N D" format
  int num = 0, den = 1;
  std::istringstream iss(edit_rate);
  iss >> num >> den;
  if(den <= 0)
    den = 1;
  double fps = double(num) / double(den);

  if(fps > 60.0)
  {
    notes.push_back(Note{Severity::info, Code::cpl_invalid_edit_rate,
                         "Ultra-HFR content: " + std::to_string(int(fps)) + " fps", cpl_path});

    // 120fps is supported by some next-gen systems
    if(fps > 120.0)
    {
      notes.push_back(Note{Severity::error, Code::cpl_invalid_edit_rate,
                           "Frame rate " + std::to_string(int(fps)) +
                               " fps exceeds maximum supported rate (120fps)",
                           cpl_path});
    }

    // At 120fps, maximum DCI bitrate applies
    notes.push_back(Note{Severity::info, Code::j2k_bitrate_exceeded,
                         "Ultra-HFR: DCI maximum bitrate is 500 Mbps", cpl_path});
  }

  xmlFreeDoc(doc);
  return notes;
}

// ============================================================================
// Accessibility Track Validation
// ============================================================================

std::vector<Note> check_accessibility(const fs::path& package_dir)
{
  std::vector<Note> notes;

  std::error_code ec;
  bool has_audio_desc = false;
  bool has_hi_subtitles = false;
  bool has_closed_captions = false;
  std::string ad_track_id;
  std::string hi_track_file;
  std::string cc_track_file;

  for(auto& entry : fs::directory_iterator(package_dir, ec))
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

    // Search for accessibility markers in CPL
    // Look for MCA labels, ContentKind, and track type elements
    std::function<void(xmlNodePtr)> scan = [&](xmlNodePtr node) {
      for(auto cur = node; cur; cur = cur->next)
      {
        if(cur->type == XML_ELEMENT_NODE)
        {
          std::string name(reinterpret_cast<const char*>(cur->name));

          // Check for MCA Sound Field labels (ST 377-4 / ST 429-2)
          if(name == "MCATagSymbol" || name == "MCATagName" || name == "MCALabelDictionary")
          {
            auto content = xmlNodeGetContent(cur);
            if(content)
            {
              std::string val(reinterpret_cast<const char*>(content));
              xmlFree(content);

              // ST 377-4 MCA labels for accessibility
              if(val.find("VI") != std::string::npos ||
                 val.find("VisuallyImpaired") != std::string::npos ||
                 val.find("AudioDescription") != std::string::npos ||
                 val.find("chAD") != std::string::npos)
              {
                has_audio_desc = true;
              }
              if(val.find("HI") != std::string::npos ||
                 val.find("HearingImpaired") != std::string::npos ||
                 val.find("chHI") != std::string::npos)
              {
                has_hi_subtitles = true;
              }
            }
          }

          // Check for MCA RFC5646 spoken language (accessibility tracks have specific tags)
          if(name == "RFC5646SpokenLanguage" || name == "MCAContent")
          {
            auto content = xmlNodeGetContent(cur);
            if(content)
            {
              std::string val(reinterpret_cast<const char*>(content));
              xmlFree(content);
              if(val.find("audiodesc") != std::string::npos ||
                 val.find("audio-desc") != std::string::npos)
                has_audio_desc = true;
            }
          }

          // Check for closed caption and subtitle assets
          if(name == "MainClosedCaption" || name == "ClosedCaption")
          {
            has_closed_captions = true;
            // Try to get the track file UUID for validation
            for(auto child = cur->children; child; child = child->next)
            {
              if(child->type == XML_ELEMENT_NODE)
              {
                std::string cname(reinterpret_cast<const char*>(child->name));
                if(cname == "Id" || cname == "TrackFileId")
                {
                  auto id_content = xmlNodeGetContent(child);
                  if(id_content)
                  {
                    cc_track_file = reinterpret_cast<const char*>(id_content);
                    xmlFree(id_content);
                  }
                }
              }
            }
          }

          // Check subtitle annotation for HI/SDH indicators
          if(name == "AnnotationText" || name == "ContentTitleText")
          {
            auto content = xmlNodeGetContent(cur);
            if(content)
            {
              std::string val(reinterpret_cast<const char*>(content));
              xmlFree(content);
              if(val.find("-HI") != std::string::npos || val.find("_HI") != std::string::npos ||
                 val.find("SDH") != std::string::npos || val.find("_AD") != std::string::npos ||
                 val.find("-AD") != std::string::npos)
              {
                if(val.find("HI") != std::string::npos || val.find("SDH") != std::string::npos)
                  has_hi_subtitles = true;
                if(val.find("AD") != std::string::npos)
                  has_audio_desc = true;
              }
            }
          }

          // Check ContentKind for accessibility variants
          if(name == "ContentKind")
          {
            auto content = xmlNodeGetContent(cur);
            if(content)
            {
              std::string val(reinterpret_cast<const char*>(content));
              xmlFree(content);
              if(val.find("caption") != std::string::npos)
                has_closed_captions = true;
            }
          }
        }
        scan(cur->children);
      }
    };

    scan(root->children);
    xmlFreeDoc(doc);
  }

  // Validate that referenced track files exist
  if(has_closed_captions && !cc_track_file.empty())
  {
    // Check if the CC track XML file exists in the package
    bool found_cc_file = false;
    for(auto& entry2 : fs::directory_iterator(package_dir, ec))
    {
      if(entry2.is_regular_file() && entry2.path().extension() == ".xml")
      {
        std::ifstream ifs(entry2.path());
        std::string content_str((std::istreambuf_iterator<char>(ifs)),
                                std::istreambuf_iterator<char>());
        if(content_str.find("SubtitleReel") != std::string::npos ||
           content_str.find("ClosedCaption") != std::string::npos)
        {
          found_cc_file = true;
          break;
        }
      }
    }
    if(!found_cc_file)
    {
      notes.push_back(Note{Severity::warning, Code::asset_not_found,
                           "Closed caption track referenced but asset file not found in package",
                           package_dir});
    }
  }

  // Report findings
  if(has_audio_desc)
  {
    notes.push_back(Note{Severity::info, Code::sound_invalid_channel_count,
                         "Accessibility: Audio Description (VI/AD) track present", package_dir});
  }
  if(has_hi_subtitles)
  {
    notes.push_back(Note{Severity::info, Code::subtitle_parse_error,
                         "Accessibility: Hearing Impaired (HI/SDH) subtitles present",
                         package_dir});
  }
  if(has_closed_captions)
  {
    notes.push_back(Note{Severity::info, Code::subtitle_parse_error,
                         "Accessibility: Closed Captions present", package_dir});
  }

  if(!has_audio_desc && !has_hi_subtitles && !has_closed_captions)
  {
    notes.push_back(Note{Severity::warning, Code::subtitle_parse_error,
                         "No accessibility tracks detected (AD/HI/CC) — consider adding for "
                         "compliance",
                         package_dir});
  }

  return notes;
}

// ============================================================================
// Content Fingerprinting (Perceptual Hash)
// ============================================================================

ContentFingerprint generate_fingerprint(const fs::path& mxf_path)
{
  ContentFingerprint fp;

  // Use ffmpeg to decode a representative frame and compute a perceptual hash.
  // Strategy: extract frame at 10% into the video (avoids slates/black leader),
  // scale to 32x32 grayscale, compute average hash (aHash) from raw pixel data.

  // First get duration to pick a good sample frame
  std::string dur_cmd = "ffprobe -v quiet -select_streams v:0 -show_entries "
                        "stream=nb_frames,width,height -of csv=p=0 \"" +
                        mxf_path.string() + "\" 2>/dev/null";
  std::string dur_out = run_cmd(dur_cmd);

  uint32_t total_frames = 0, width = 0, height = 0;
  if(!dur_out.empty())
    sscanf(dur_out.c_str(), "%u,%u,%u", &total_frames, &width, &height);

  fp.width = width;
  fp.height = height;

  // Sample at ~10% into the content (skip leader/slate)
  uint32_t sample_frame = total_frames > 10 ? total_frames / 10 : 0;
  fp.frame_sampled = sample_frame;

  // Extract frame as 32x32 grayscale raw pixels for perceptual hashing
  // Using rawvideo output: 32*32 = 1024 bytes of Y plane
  std::string cmd = "ffmpeg -v quiet -ss " + std::to_string(sample_frame) + " -i \"" +
                    mxf_path.string() + "\" -vf \"select=eq(n\\," + std::to_string(sample_frame) +
                    "),scale=32:32,format=gray\" -frames:v 1 -f rawvideo pipe:1 2>/dev/null";

  FILE* pipe = DCPDOCTOR_POPEN(cmd.c_str(), "r");
  if(!pipe)
    return fp;

  // Read 32x32 = 1024 grayscale pixels
  constexpr int HASH_SIZE = 32;
  constexpr int PIXEL_COUNT = HASH_SIZE * HASH_SIZE;
  uint8_t pixels[PIXEL_COUNT];
  size_t read_bytes = fread(pixels, 1, PIXEL_COUNT, pipe);
  DCPDOCTOR_PCLOSE(pipe);

  if(read_bytes < PIXEL_COUNT)
    return fp;

  // Compute average hash (aHash): compare each pixel to the mean
  uint64_t sum = 0;
  for(int i = 0; i < PIXEL_COUNT; ++i)
    sum += pixels[i];
  uint8_t mean = static_cast<uint8_t>(sum / PIXEL_COUNT);

  // Build 256-bit hash (32x32 / 4 = 256 bits in 32 bytes)
  // Actually 1024 bits, let's do a standard 64-bit pHash approach:
  // Use 8x8 center of the 32x32 for a compact 64-bit hash
  uint64_t hash_val = 0;
  for(int y = 12; y < 20; ++y)
  {
    for(int x = 12; x < 20; ++x)
    {
      hash_val <<= 1;
      if(pixels[y * HASH_SIZE + x] > mean)
        hash_val |= 1;
    }
  }

  // Convert to hex
  std::ostringstream oss;
  oss << std::hex << std::setfill('0') << std::setw(16) << hash_val;
  fp.hash = oss.str();

  return fp;
}

double compare_fingerprints(const ContentFingerprint& a, const ContentFingerprint& b)
{
  if(a.hash.empty() || b.hash.empty())
    return 1.0;
  if(a.hash == b.hash)
    return 0.0;

  // Compute normalized Hamming distance between 64-bit hashes
  uint64_t ha = 0, hb = 0;
  try
  {
    ha = std::stoull(a.hash, nullptr, 16);
    hb = std::stoull(b.hash, nullptr, 16);
  }
  catch(...)
  {
    return 1.0;
  }

  uint64_t diff = ha ^ hb;
  int distance = 0;
  while(diff)
  {
    distance += diff & 1;
    diff >>= 1;
  }

  // 64-bit hash: distance ranges 0–64
  return static_cast<double>(distance) / 64.0;
}

} // namespace dcpdoctor
