#include <libxml/parser.h>
#include <algorithm>
#include <iomanip>
#include <sstream>

#include "dcpdoctor/timeline.h"

namespace dcpdoctor
{
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

  std::string format_timecode(uint64_t frames, double fps)
  {
    if(fps <= 0)
      fps = 24.0;
    double total_sec = double(frames) / fps;
    int hours = int(total_sec) / 3600;
    int mins = (int(total_sec) % 3600) / 60;
    int secs = int(total_sec) % 60;
    int fr = int(frames) % int(fps);

    std::ostringstream ss;
    ss << std::setfill('0') << std::setw(2) << hours << ":" << std::setw(2) << mins << ":"
       << std::setw(2) << secs << ":" << std::setw(2) << fr;
    return ss.str();
  }

} // namespace

std::vector<TimelineReel> extract_timeline(const std::filesystem::path& cpl_path)
{
  std::vector<TimelineReel> reels;

  auto doc = xmlReadFile(cpl_path.string().c_str(), nullptr,
                         XML_PARSE_NOERROR | XML_PARSE_NOWARNING | XML_PARSE_NONET);
  if(!doc)
    return reels;

  auto root = xmlDocGetRootElement(doc);
  if(!root)
  {
    xmlFreeDoc(doc);
    return reels;
  }

  // Find ReelList (DCP) or SegmentList (IMF)
  bool found_reels = false;
  for(auto rl = root->children; rl; rl = rl->next)
  {
    if(rl->type != XML_ELEMENT_NODE)
      continue;
    if(xmlStrcmp(rl->name, BAD_CAST "ReelList") != 0)
      continue;

    found_reels = true;
    for(auto reel_node = rl->children; reel_node; reel_node = reel_node->next)
    {
      if(reel_node->type != XML_ELEMENT_NODE)
        continue;
      if(xmlStrcmp(reel_node->name, BAD_CAST "Reel") != 0)
        continue;

      TimelineReel reel;
      reel.id = get_text(reel_node, "Id");

      // Find AssetList
      for(auto al = reel_node->children; al; al = al->next)
      {
        if(al->type != XML_ELEMENT_NODE)
          continue;
        if(xmlStrcmp(al->name, BAD_CAST "AssetList") != 0)
          continue;

        for(auto asset = al->children; asset; asset = asset->next)
        {
          if(asset->type != XML_ELEMENT_NODE)
            continue;
          std::string name(reinterpret_cast<const char*>(asset->name));

          if(name == "MainPicture" || name == "MainImage" || name == "MainStereoscopicPicture")
          {
            reel.has_picture = true;
            auto ep = get_text(asset, "EntryPoint");
            auto dur = get_text(asset, "Duration");
            if(!ep.empty())
              reel.picture_entry = std::stoull(ep);
            if(!dur.empty())
              reel.picture_duration = std::stoull(dur);
          }
          else if(name == "MainSound")
          {
            reel.has_sound = true;
            auto ep = get_text(asset, "EntryPoint");
            auto dur = get_text(asset, "Duration");
            if(!ep.empty())
              reel.sound_entry = std::stoull(ep);
            if(!dur.empty())
              reel.sound_duration = std::stoull(dur);
          }
          else if(name == "MainSubtitle" || name == "ClosedCaption")
          {
            reel.has_subtitle = true;
            auto ep = get_text(asset, "EntryPoint");
            auto dur = get_text(asset, "Duration");
            if(!ep.empty())
              reel.subtitle_entry = std::stoull(ep);
            if(!dur.empty())
              reel.subtitle_duration = std::stoull(dur);
          }
        }
      }

      reels.push_back(std::move(reel));
    }
  }

  // IMF fallback: parse SegmentList/Segment/SequenceList
  if(!found_reels)
  {
    for(auto sl = root->children; sl; sl = sl->next)
    {
      if(sl->type != XML_ELEMENT_NODE)
        continue;
      if(xmlStrcmp(sl->name, BAD_CAST "SegmentList") != 0)
        continue;

      for(auto seg = sl->children; seg; seg = seg->next)
      {
        if(seg->type != XML_ELEMENT_NODE)
          continue;
        if(xmlStrcmp(seg->name, BAD_CAST "Segment") != 0)
          continue;

        TimelineReel reel;
        reel.id = get_text(seg, "Id");

        // Find SequenceList
        for(auto sql = seg->children; sql; sql = sql->next)
        {
          if(sql->type != XML_ELEMENT_NODE)
            continue;
          if(xmlStrcmp(sql->name, BAD_CAST "SequenceList") != 0)
            continue;

          for(auto seq = sql->children; seq; seq = seq->next)
          {
            if(seq->type != XML_ELEMENT_NODE)
              continue;
            std::string name(reinterpret_cast<const char*>(seq->name));

            if(name == "MainImageSequence")
            {
              reel.has_picture = true;
              // Look in ResourceList for duration info
              for(auto rl2 = seq->children; rl2; rl2 = rl2->next)
              {
                if(rl2->type != XML_ELEMENT_NODE)
                  continue;
                if(xmlStrcmp(rl2->name, BAD_CAST "ResourceList") != 0)
                  continue;
                for(auto res = rl2->children; res; res = res->next)
                {
                  if(res->type != XML_ELEMENT_NODE)
                    continue;
                  auto ep = get_text(res, "EntryPoint");
                  auto dur = get_text(res, "IntrinsicDuration");
                  if(!ep.empty())
                    reel.picture_entry = std::stoull(ep);
                  if(!dur.empty())
                    reel.picture_duration = std::stoull(dur);
                  break; // First resource
                }
              }
            }
            else if(name == "MainAudioSequence")
            {
              reel.has_sound = true;
              for(auto rl2 = seq->children; rl2; rl2 = rl2->next)
              {
                if(rl2->type != XML_ELEMENT_NODE)
                  continue;
                if(xmlStrcmp(rl2->name, BAD_CAST "ResourceList") != 0)
                  continue;
                for(auto res = rl2->children; res; res = res->next)
                {
                  if(res->type != XML_ELEMENT_NODE)
                    continue;
                  auto ep = get_text(res, "EntryPoint");
                  auto dur = get_text(res, "IntrinsicDuration");
                  if(!ep.empty())
                    reel.sound_entry = std::stoull(ep);
                  if(!dur.empty())
                    reel.sound_duration = std::stoull(dur);
                  break;
                }
              }
            }
          }
        }

        reels.push_back(std::move(reel));
      }
    }
  }

  xmlFreeDoc(doc);
  return reels;
}

void write_timeline_svg(std::ostream& out, const std::vector<TimelineReel>& reels,
                        const std::string& title, double frame_rate)
{
  if(reels.empty())
    return;

  // Calculate total duration
  uint64_t total_frames = 0;
  for(const auto& r : reels)
    total_frames += r.picture_duration;

  if(total_frames == 0)
    return;

  // SVG dimensions
  const int width = 1200;
  const int margin = 60;
  const int track_height = 40;
  const int track_gap = 10;
  const int header_height = 50;
  int num_tracks = 3; // picture, sound, subtitle
  int height = header_height + num_tracks * (track_height + track_gap) + margin;

  double pixels_per_frame = double(width - 2 * margin) / double(total_frames);

  out << R"(<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width=")"
      << width << R"(" height=")" << height << R"(">
<style>
  .title { font: bold 16px sans-serif; fill: #333; }
  .label { font: 11px sans-serif; fill: #666; }
  .tc { font: 10px monospace; fill: #999; }
  .pic { fill: #4a90d9; stroke: #2171b5; stroke-width: 1; }
  .snd { fill: #65b365; stroke: #3a8f3a; stroke-width: 1; }
  .sub { fill: #d9a04a; stroke: #b57721; stroke-width: 1; }
  .reel-line { stroke: #ddd; stroke-width: 1; stroke-dasharray: 4; }
</style>
)";

  // Title
  out << "<text x=\"" << margin << "\" y=\"30\" class=\"title\">" << title << " ("
      << format_timecode(total_frames, frame_rate) << ")</text>\n";

  // Track labels
  int y_pic = header_height;
  int y_snd = y_pic + track_height + track_gap;
  int y_sub = y_snd + track_height + track_gap;

  out << "<text x=\"5\" y=\"" << y_pic + track_height / 2 + 4
      << "\" class=\"label\">Picture</text>\n";
  out << "<text x=\"5\" y=\"" << y_snd + track_height / 2 + 4
      << "\" class=\"label\">Sound</text>\n";
  out << "<text x=\"5\" y=\"" << y_sub + track_height / 2 + 4
      << "\" class=\"label\">Subtitle</text>\n";

  // Draw reels
  uint64_t offset = 0;
  for(size_t i = 0; i < reels.size(); ++i)
  {
    const auto& r = reels[i];
    double x = margin + offset * pixels_per_frame;
    double w = r.picture_duration * pixels_per_frame;

    // Reel separator
    if(i > 0)
    {
      out << "<line x1=\"" << int(x) << "\" y1=\"" << header_height << "\" x2=\"" << int(x)
          << "\" y2=\"" << height - 20 << "\" class=\"reel-line\"/>\n";
    }

    // Picture track
    if(r.has_picture)
    {
      out << "<rect x=\"" << int(x) << "\" y=\"" << y_pic << "\" width=\"" << (std::max)(1, int(w))
          << "\" height=\"" << track_height << "\" class=\"pic\" rx=\"3\"/>\n";
    }

    // Sound track
    if(r.has_sound)
    {
      double sw = r.sound_duration * pixels_per_frame;
      out << "<rect x=\"" << int(x) << "\" y=\"" << y_snd << "\" width=\"" << (std::max)(1, int(sw))
          << "\" height=\"" << track_height << "\" class=\"snd\" rx=\"3\"/>\n";
    }

    // Subtitle track
    if(r.has_subtitle)
    {
      double subw = r.subtitle_duration * pixels_per_frame;
      out << "<rect x=\"" << int(x) << "\" y=\"" << y_sub << "\" width=\""
          << (std::max)(1, int(subw)) << "\" height=\"" << track_height
          << "\" class=\"sub\" rx=\"3\"/>\n";
    }

    // Timecode at reel start
    out << "<text x=\"" << int(x + 2) << "\" y=\"" << height - 5 << "\" class=\"tc\">"
        << format_timecode(offset, frame_rate) << "</text>\n";

    offset += r.picture_duration;
  }

  // End timecode
  out << "<text x=\"" << int(margin + total_frames * pixels_per_frame - 60) << "\" y=\""
      << height - 5 << "\" class=\"tc\">" << format_timecode(total_frames, frame_rate)
      << "</text>\n";

  out << "</svg>\n";
}

std::vector<Note> check_audio_sync(const std::vector<TimelineReel>& reels,
                                   const std::filesystem::path& cpl_path)
{
  std::vector<Note> notes;

  for(size_t i = 0; i < reels.size(); ++i)
  {
    const auto& reel = reels[i];
    if(!reel.has_picture || !reel.has_sound)
      continue;

    int64_t drift =
        static_cast<int64_t>(reel.sound_duration) - static_cast<int64_t>(reel.picture_duration);

    if(drift != 0)
    {
      std::string msg = "Reel " + std::to_string(i + 1) + ": audio/picture duration mismatch (" +
                        std::to_string(drift) + " frames drift)";
      Severity sev = (std::abs(drift) > 1) ? Severity::warning : Severity::info;
      notes.push_back(Note{sev, Code::reel_discontinuity, msg, cpl_path});
    }
  }

  return notes;
}

} // namespace dcpdoctor
