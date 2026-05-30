#include <filesystem>
#include <set>
#include <unordered_map>

#include "dcpdoctor/dcpdoctor.h"
#include "dcpdoctor/assetmap.h"
#include "dcpdoctor/compliance.h"
#include "dcpdoctor/cpl.h"
#include "dcpdoctor/isdcf.h"
#include "dcpdoctor/j2k.h"
#include "dcpdoctor/pkl.h"
#include "dcpdoctor/hash.h"
#include "dcpdoctor/mxf.h"
#include "dcpdoctor/schema_validator.h"
#include "dcpdoctor/signature.h"
#include "dcpdoctor/subtitle.h"
#include "dcpdoctor/validators.h"

namespace fs = std::filesystem;

namespace dcpdoctor
{

static constexpr std::string_view smpte_cpl_ns = "http://www.smpte-ra.org/schemas/429-7/2006/CPL";
static constexpr std::string_view interop_cpl_ns =
    "http://www.digicine.com/PROTO-ASDCP-CPL-20040511#";

static Standard detect_standard(const fs::path& dcp_dir)
{
  // Check ASSETMAP vs ASSETMAP.xml to detect Interop vs SMPTE
  if(fs::exists(dcp_dir / "ASSETMAP.xml"))
    return Standard::smpte;
  if(fs::exists(dcp_dir / "ASSETMAP"))
    return Standard::interop;
  return Standard::unknown;
}

static fs::path find_assetmap(const fs::path& dcp_dir)
{
  if(fs::exists(dcp_dir / "ASSETMAP.xml"))
    return dcp_dir / "ASSETMAP.xml";
  if(fs::exists(dcp_dir / "ASSETMAP"))
    return dcp_dir / "ASSETMAP";
  return {};
}

VerifyResult verify(const fs::path& dcp_dir, const VerifyOptions& opts)
{
  VerifyResult result;

  if(!fs::is_directory(dcp_dir))
  {
    result.add({Severity::error, Code::missing_assetmap,
                "Path is not a directory: " + dcp_dir.string(), dcp_dir});
    return result;
  }

  result.standard = detect_standard(dcp_dir);

  // 1. Find and parse ASSETMAP
  auto assetmap_path = find_assetmap(dcp_dir);
  if(assetmap_path.empty())
  {
    result.add(
        {Severity::error, Code::missing_assetmap, "No ASSETMAP or ASSETMAP.xml found", dcp_dir});
    return result;
  }

  auto assetmap = AssetMap::parse(assetmap_path);
  if(!assetmap)
  {
    result.add({Severity::error, Code::xml_parse_error, "Failed to parse ASSETMAP", assetmap_path});
    return result;
  }

  // 2. Check for duplicate asset IDs
  std::set<std::string> seen_ids;
  for(const auto& asset : assetmap->assets)
  {
    if(!seen_ids.insert(asset.id).second)
    {
      result.add({Severity::error, Code::duplicate_asset_id, "Duplicate asset ID: " + asset.id,
                  assetmap_path});
    }
  }

  // 3. Verify all referenced files exist
  for(const auto& asset : assetmap->assets)
  {
    auto full_path = dcp_dir / asset.path;
    if(!fs::exists(full_path))
    {
      result.add({Severity::error, Code::asset_not_found, "Asset file not found: " + asset.path,
                  assetmap_path});
    }
  }

  // 4. Find and validate PKLs
  bool found_pkl = false;
  for(const auto& asset : assetmap->assets)
  {
    auto full_path = dcp_dir / asset.path;
    if(!fs::exists(full_path))
      continue;

    // Only try parsing XML files
    auto ext = full_path.extension().string();
    if(ext != ".xml" && full_path.filename().string().find("PKL") == std::string::npos &&
       full_path.filename().string().find("pkl") == std::string::npos)
      continue;

    auto pkl = Pkl::parse(full_path);
    if(!pkl)
      continue;

    found_pkl = true;

    // Build map from asset ID to file path for hash verification
    std::unordered_map<std::string, std::string> id_to_path;
    for(const auto& a : assetmap->assets)
      id_to_path[a.id] = a.path;

    // Verify PKL asset hashes if requested
    if(opts.check_hashes)
    {
      int file_index = 0;
      int total_files = static_cast<int>(pkl->assets.size());
      for(const auto& pkl_asset : pkl->assets)
      {
        // Check that referenced assets exist in ASSETMAP
        auto it = id_to_path.find(pkl_asset.id);
        if(it == id_to_path.end())
        {
          result.add({Severity::warning, Code::pkl_missing_asset_reference,
                      "PKL references unknown asset: " + pkl_asset.id, full_path});
          ++file_index;
          continue;
        }

        // Verify hash
        if(!pkl_asset.hash.empty())
        {
          auto asset_path = dcp_dir / it->second;
          if(fs::exists(asset_path))
          {
            std::optional<std::string> computed;
            if(opts.hash_progress)
            {
              computed = sha1_base64(asset_path, [&](std::uintmax_t bytes_done,
                                                     std::uintmax_t bytes_total) {
                opts.hash_progress(asset_path, file_index, total_files, bytes_done, bytes_total);
              });
            }
            else
            {
              computed = sha1_base64(asset_path);
            }
            if(computed && *computed != pkl_asset.hash)
            {
              result.add({Severity::error, Code::pkl_hash_mismatch,
                          "Hash mismatch for " + it->second + " (expected " + pkl_asset.hash +
                              ", got " + *computed + ")",
                          asset_path});
            }
          }
        }
        ++file_index;
      }
    }

    // Verify PKL signature if requested
    if(opts.check_signatures)
    {
      auto sig_notes = verify_signature(full_path);
      for(auto& note : sig_notes)
        result.add(std::move(note));
    }
  }

  if(!found_pkl)
  {
    result.add({Severity::error, Code::missing_pkl, "No valid PKL found in DCP", dcp_dir});
  }

  // 5. Find and validate CPLs
  bool found_cpl = false;
  std::vector<fs::path> cpl_paths;
  for(const auto& asset : assetmap->assets)
  {
    auto full_path = dcp_dir / asset.path;
    if(!fs::exists(full_path))
      continue;

    // Only try parsing XML files
    auto ext = full_path.extension().string();
    if(ext != ".xml" && full_path.filename().string().find("CPL") == std::string::npos &&
       full_path.filename().string().find("cpl") == std::string::npos)
      continue;

    auto cpl = Cpl::parse(full_path);
    if(!cpl)
      continue;

    found_cpl = true;
    cpl_paths.push_back(full_path);

    // Validate CPL structure
    if(cpl->reels.empty())
    {
      result.add({Severity::error, Code::cpl_missing_reel, "CPL has no reels", full_path});
    }

    for(const auto& reel : cpl->reels)
    {
      if(reel.picture.duration <= 0)
      {
        result.add({Severity::error, Code::cpl_invalid_duration,
                    "Reel has invalid picture duration", full_path});
      }
      if(reel.sound.duration > 0 && reel.sound.duration != reel.picture.duration)
      {
        result.add({Severity::warning, Code::cpl_mismatched_durations,
                    "Sound duration differs from picture duration in reel " + reel.id, full_path});
      }
    }

    // ISDCF naming check
    if(!cpl->content_title.empty())
    {
      auto isdcf_notes = check_isdcf_naming(cpl->content_title, full_path);
      for(auto& note : isdcf_notes)
        result.add(std::move(note));
    }

    // Verify CPL signature if requested
    if(opts.check_signatures)
    {
      auto sig_notes = verify_signature(full_path);
      for(auto& note : sig_notes)
        result.add(std::move(note));
    }
  }

  if(!found_cpl)
  {
    result.add({Severity::error, Code::missing_cpl, "No valid CPL found in DCP", dcp_dir});
  }

  // 6. Validate MXF assets
  if(opts.check_picture_details)
  {
    for(const auto& asset : assetmap->assets)
    {
      auto full_path = dcp_dir / asset.path;
      auto ext = full_path.extension().string();
      if(ext != ".mxf" && ext != ".MXF")
        continue;
      if(!fs::exists(full_path))
        continue;

      auto mxf = read_mxf_info(full_path);
      if(!mxf.valid)
      {
        result.add(
            {Severity::error, Code::mxf_unreadable, "Invalid MXF file: " + mxf.error, full_path});
        continue;
      }

      // Validate picture parameters
      if(mxf.picture)
      {
        auto& pic = *mxf.picture;
        // Check resolution (DCI: 2048x1080 or 4096x2160; 1998x1080 or 3996x2160 scope)
        if(pic.width > 0 && pic.height > 0)
        {
          bool valid_res = (pic.width == 2048 && pic.height == 1080) ||
                           (pic.width == 1998 && pic.height == 1080) ||
                           (pic.width == 4096 && pic.height == 2160) ||
                           (pic.width == 3996 && pic.height == 2160);
          if(!valid_res && opts.strict_smpte)
          {
            result.add({Severity::warning, Code::picture_invalid_resolution,
                        "Non-standard picture resolution: " + std::to_string(pic.width) + "x" +
                            std::to_string(pic.height),
                        full_path});
          }
        }

        // Check frame rate (24, 25, 30, 48, 60 fps)
        if(pic.frame_rate_num > 0 && pic.frame_rate_den > 0)
        {
          double fps = double(pic.frame_rate_num) / pic.frame_rate_den;
          bool valid_fps =
              (fps == 24.0 || fps == 25.0 || fps == 30.0 || fps == 48.0 || fps == 60.0);
          if(!valid_fps && opts.strict_smpte)
          {
            result.add({Severity::warning, Code::picture_invalid_frame_rate,
                        "Non-standard frame rate: " + std::to_string(pic.frame_rate_num) + "/" +
                            std::to_string(pic.frame_rate_den),
                        full_path});
          }
        }

        // Check J2K bitrate compliance
        if(pic.frame_count > 0 && pic.frame_rate_num > 0 && pic.width > 0)
        {
          auto bitrate_notes = check_j2k_bitrate(full_path, pic.frame_count, pic.frame_rate_num,
                                                 pic.frame_rate_den, pic.width, pic.height);
          for(auto& n : bitrate_notes)
            result.add(std::move(n));
        }
      }

      // Validate sound parameters
      if(mxf.sound)
      {
        auto& snd = *mxf.sound;
        if(snd.sample_rate > 0 && snd.sample_rate != 48000 && snd.sample_rate != 96000)
        {
          result.add({Severity::warning, Code::sound_invalid_sample_rate,
                      "Non-standard audio sample rate: " + std::to_string(snd.sample_rate) + " Hz",
                      full_path});
        }
        if(snd.channels > 0 && snd.channels > 16)
        {
          result.add({Severity::warning, Code::sound_invalid_channel_count,
                      "Excessive audio channel count: " + std::to_string(snd.channels), full_path});
        }
      }
    }
  }

  // 7. SMPTE/BV2.1 compliance checks
  if(opts.strict_smpte)
  {
    auto compliance_notes = check_smpte_compliance(dcp_dir, result.standard, true);
    for(auto& note : compliance_notes)
      result.add(std::move(note));
  }

  // 8. Additional validators (run on CPLs)
  if(!cpl_paths.empty())
  {
    // Encryption detection
    auto enc_notes = check_encryption(dcp_dir, cpl_paths);
    for(auto& note : enc_notes)
      result.add(std::move(note));

    // Collect known asset IDs for cross-reference check
    std::vector<std::string> known_ids;
    for(const auto& asset : assetmap->assets)
      known_ids.push_back(asset.id);

    // Cross-reference integrity
    auto xref_notes = check_cross_references(dcp_dir, known_ids, cpl_paths);
    for(auto& note : xref_notes)
      result.add(std::move(note));

    // Supplemental DCP detection
    auto supp_notes = check_supplemental(dcp_dir, cpl_paths);
    for(auto& note : supp_notes)
      result.add(std::move(note));

    // Per-CPL checks
    for(const auto& cpl_path : cpl_paths)
    {
      // Reel continuity
      auto reel_notes = check_reel_continuity(cpl_path);
      for(auto& note : reel_notes)
        result.add(std::move(note));

      // 3D stereoscopic
      auto stereo_notes = check_stereo(cpl_path);
      for(auto& note : stereo_notes)
        result.add(std::move(note));

      // Markers
      auto marker_notes = check_markers(cpl_path, opts.strict_smpte);
      for(auto& note : marker_notes)
        result.add(std::move(note));

      // Audio channel labeling
      auto audio_notes = check_audio_channels(cpl_path);
      for(auto& note : audio_notes)
        result.add(std::move(note));

      // Color space
      auto color_notes = check_color_space(cpl_path);
      for(auto& note : color_notes)
        result.add(std::move(note));
    }
  }

  // 9. Subtitle/Timed Text validation
  for(const auto& asset : assetmap->assets)
  {
    auto full_path = dcp_dir / asset.path;
    if(!fs::exists(full_path))
      continue;
    auto ext = full_path.extension().string();
    if(ext == ".xml")
    {
      // Check if this XML file is a subtitle file
      auto fname = full_path.filename().string();
      if(fname.find("sub") != std::string::npos || fname.find("Sub") != std::string::npos ||
         fname.find("SUB") != std::string::npos || fname.find("timed") != std::string::npos)
      {
        auto sub_notes = validate_subtitle(full_path, result.standard);
        for(auto& note : sub_notes)
          result.add(std::move(note));
      }
    }
  }

  return result;
}

} // namespace dcpdoctor
