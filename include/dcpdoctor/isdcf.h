#pragma once

#include "dcpdoctor/dcpdoctor.h"
#include <string>
#include <vector>

namespace dcpdoctor
{

/// Check if a CPL content title text follows ISDCF naming convention
/// Returns notes for any naming violations
std::vector<Note> check_isdcf_naming(const std::string& content_title,
                                     const std::filesystem::path& cpl_path);

/// Parameters for generating an ISDCF-compliant name
struct IsdcfNameParams
{
  std::string film_title; // e.g. "MyMovie" (max 14 chars, no spaces)
  std::string content_type = "FTR"; // FTR, TLR, TSR, PRO, TST, etc.
  std::string aspect_ratio = "F"; // F (Flat), S (Scope), C (Custom)
  std::string language = "EN"; // ISO 639-1 uppercase
  std::string territory = "XX"; // ISO 3166-1 uppercase (XX = unspecified)
  std::string audio_type = "51"; // 51, 71, ATMOS, etc.
  std::string resolution = "2K"; // 2K or 4K
  std::string studio; // Studio abbreviation (optional)
  std::string date; // YYYYMMDD (optional, auto-filled if empty)
  std::string facility; // Facility code (optional)
  std::string standard = "SMPTE"; // SMPTE or IOP
  std::string package_type = "OV"; // OV or VF
  std::string luminance; // Optional: empty, or e.g. "PQ", "HLG"
  std::string frame_rate; // Optional: e.g. "24", "48HI"
  bool is_3d = false; // If true, adds "-3D" suffix to aspect
};

/// Generate an ISDCF-compliant content title string from parameters
std::string generate_isdcf_name(const IsdcfNameParams& params);

} // namespace dcpdoctor
