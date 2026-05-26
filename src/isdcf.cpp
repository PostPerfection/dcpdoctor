#include <algorithm>
#include <chrono>
#include <regex>
#include <set>
#include <sstream>

#include "dcpdoctor/isdcf.h"

namespace dcpdoctor
{

std::vector<Note> check_isdcf_naming(const std::string& content_title,
                                     const std::filesystem::path& cpl_path)
{
  std::vector<Note> notes;

  if(content_title.empty())
  {
    notes.push_back(
        {Severity::warning, Code::isdcf_naming_violation, "Content title is empty", cpl_path});
    return notes;
  }

  // ISDCF naming convention (v2):
  // FilmTitle_ContentType_AspectRatio_Language_Territory_AudioType_Resolution_Studio_Date_Facility_Standard
  // e.g., "MyMovie_FTR_F_EN_US_51_2K_ST_20230101_FAC_SMPTE_OV"
  //
  // Fields are separated by underscores
  // Minimum: FilmTitle_ContentType (at least 2 fields)

  // Count underscore-separated fields
  int field_count = 1;
  for(char c : content_title)
    if(c == '_')
      ++field_count;

  if(field_count < 2)
  {
    notes.push_back({Severity::warning, Code::isdcf_naming_violation,
                     "Content title does not follow ISDCF naming convention "
                     "(expected underscore-separated fields): " +
                         content_title,
                     cpl_path});
    return notes;
  }

  // Split into fields
  std::vector<std::string> fields;
  std::istringstream ss(content_title);
  std::string field;
  while(std::getline(ss, field, '_'))
    fields.push_back(field);

  // Field 1: Film title (required, non-empty, max 14 chars recommended)
  if(fields[0].empty())
  {
    notes.push_back(
        {Severity::warning, Code::isdcf_naming_violation, "Film title field is empty", cpl_path});
  }
  else if(fields[0].size() > 14)
  {
    notes.push_back({Severity::info, Code::isdcf_naming_violation,
                     "Film title exceeds 14 character recommendation: " + fields[0], cpl_path});
  }

  // Field 2: Content type
  if(fields.size() >= 2)
  {
    static const std::set<std::string> valid_types = {"FTR", "TLR", "TSR", "PRO", "TST", "RTG",
                                                      "SHR", "ADV", "XSN", "PSA", "POL", "CLT"};
    if(!valid_types.contains(fields[1]))
    {
      notes.push_back(
          {Severity::warning, Code::isdcf_naming_violation,
           "Non-standard content type: " + fields[1] + " (expected FTR, TLR, TSR, PRO, TST, etc.)",
           cpl_path});
    }
  }

  // Field 3: Aspect ratio (if present)
  if(fields.size() >= 3)
  {
    static const std::set<std::string> valid_aspects = {
        "F",     "S",     "C",     "F-133", "F-137", "F-138", "F-165", "F-166", "F-178",
        "F-185", "F-190", "F-200", "F-220", "F-239", "S-185", "S-239", "C-185", "C-239"};
    if(!valid_aspects.contains(fields[2]))
    {
      // Just info, many variations exist
      notes.push_back({Severity::info, Code::isdcf_naming_violation,
                       "Non-standard aspect ratio field: " + fields[2], cpl_path});
    }
  }

  // Field 4: Language (if present) - should be 2-3 uppercase letters
  if(fields.size() >= 4)
  {
    static const std::regex lang_re("^[A-Z]{2,3}(-[A-Z]{2,3})?$");
    if(!std::regex_match(fields[3], lang_re) && !fields[3].empty())
    {
      notes.push_back({Severity::info, Code::isdcf_naming_violation,
                       "Non-standard language field: " + fields[3], cpl_path});
    }
  }

  // Check for resolution field somewhere in the name
  bool has_resolution = false;
  for(const auto& f : fields)
  {
    if(f == "2K" || f == "4K")
    {
      has_resolution = true;
      break;
    }
  }
  if(!has_resolution && fields.size() >= 6)
  {
    notes.push_back({Severity::info, Code::isdcf_naming_violation,
                     "No resolution field (2K/4K) found in title", cpl_path});
  }

  // Check for SMPTE/IOP standard indicator at end
  if(fields.size() >= 3)
  {
    const auto& last = fields.back();
    if(last != "SMPTE" && last != "IOP" && last != "OV" && last != "VF")
    {
      // Not necessarily wrong, just informational
    }
  }

  return notes;
}

std::string generate_isdcf_name(const IsdcfNameParams& params)
{
  std::ostringstream name;

  // Field 1: Film title (truncate to 14, remove spaces/underscores)
  std::string title = params.film_title;
  std::erase(title, ' ');
  std::erase(title, '_');
  if(title.size() > 14)
    title = title.substr(0, 14);
  name << title;

  // Field 2: Content type
  name << "_" << params.content_type;

  // Field 3: Aspect ratio (with optional 3D suffix)
  name << "_" << params.aspect_ratio;
  if(params.is_3d)
    name << "-3D";

  // Field 4: Language
  name << "_" << params.language;

  // Field 5: Territory
  name << "_" << params.territory;

  // Field 6: Audio type
  name << "_" << params.audio_type;

  // Field 7: Resolution
  name << "_" << params.resolution;

  // Field 8: Studio (optional)
  if(!params.studio.empty())
    name << "_" << params.studio;

  // Field 9: Date
  if(!params.date.empty())
  {
    name << "_" << params.date;
  }
  else
  {
    // Auto-generate today's date
    auto now = std::chrono::system_clock::now();
    auto days = std::chrono::floor<std::chrono::days>(now);
    std::chrono::year_month_day ymd{days};
    char buf[16];
    std::snprintf(buf, sizeof(buf), "%04d%02u%02u",
                  static_cast<int>(ymd.year()),
                  static_cast<unsigned>(ymd.month()),
                  static_cast<unsigned>(ymd.day()));
    name << "_" << buf;
  }

  // Field 10: Facility (optional)
  if(!params.facility.empty())
    name << "_" << params.facility;

  // Field 11: Standard
  name << "_" << params.standard;

  // Field 12: Package type
  name << "_" << params.package_type;

  // Optional: luminance field
  if(!params.luminance.empty())
    name << "_" << params.luminance;

  // Optional: frame rate
  if(!params.frame_rate.empty())
    name << "_" << params.frame_rate;

  return name.str();
}

} // namespace dcpdoctor
