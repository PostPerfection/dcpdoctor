#include <spdlog/spdlog.h>
#include <filesystem>
#include <fstream>
#include <regex>
#include <unordered_map>

#include "dcpdoctor/checksum_verify.h"
#include "dcpdoctor/hash.h"
#include "dcpdoctor/pkl.h"

namespace fs = std::filesystem;

namespace dcpdoctor
{

// Parse AssetMap to build uuid -> relative path mapping
static std::unordered_map<std::string, std::string> parse_assetmap(const fs::path& dir)
{
  std::unordered_map<std::string, std::string> map;

  // Try both ASSETMAP.xml (SMPTE) and ASSETMAP (Interop)
  fs::path am_path;
  if(fs::exists(dir / "ASSETMAP.xml"))
    am_path = dir / "ASSETMAP.xml";
  else if(fs::exists(dir / "ASSETMAP"))
    am_path = dir / "ASSETMAP";
  else
    return map;

  std::ifstream f(am_path);
  std::string content((std::istreambuf_iterator<char>(f)), std::istreambuf_iterator<char>());

  // Parse Asset entries: extract Id and Path
  std::regex asset_re(
      R"re(<Asset>[\s\S]*?<Id>urn:uuid:([^<]+)</Id>[\s\S]*?<Path>([^<]+)</Path>[\s\S]*?</Asset>)re");
  auto begin = std::sregex_iterator(content.begin(), content.end(), asset_re);
  auto end = std::sregex_iterator();

  for(auto it = begin; it != end; ++it)
  {
    std::string uuid = (*it)[1].str();
    std::string path = (*it)[2].str();
    map[uuid] = path;
  }

  return map;
}

ChecksumVerifyResult verify_package_checksums(const ChecksumVerifyOptions& opts)
{
  ChecksumVerifyResult result;

  if(!fs::exists(opts.package_dir))
  {
    result.error = "Package directory does not exist: " + opts.package_dir.string();
    return result;
  }

  // Find PKL file(s)
  std::vector<fs::path> pkl_files;
  for(const auto& entry : fs::directory_iterator(opts.package_dir))
  {
    if(!entry.is_regular_file())
      continue;
    auto name = entry.path().filename().string();
    // PKL files are typically PKL_*.xml or just contain "PackingList" element
    if(name.find("PKL") != std::string::npos || name.find("pkl") != std::string::npos)
    {
      pkl_files.push_back(entry.path());
      continue;
    }
    // Check XML files that might be PKLs
    if(entry.path().extension() == ".xml")
    {
      std::ifstream f(entry.path());
      char buf[512];
      f.read(buf, sizeof(buf));
      std::string header(buf, static_cast<size_t>(f.gcount()));
      if(header.find("PackingList") != std::string::npos)
        pkl_files.push_back(entry.path());
    }
  }

  if(pkl_files.empty())
  {
    result.error = "No PKL found in " + opts.package_dir.string();
    return result;
  }

  // Build asset map for path resolution
  auto asset_paths = parse_assetmap(opts.package_dir);

  // Process each PKL
  for(const auto& pkl_path : pkl_files)
  {
    auto pkl = Pkl::parse(pkl_path);
    if(!pkl)
    {
      spdlog::warn("Failed to parse PKL: {}", pkl_path.string());
      continue;
    }

    for(const auto& asset : pkl->assets)
    {
      ChecksumEntry entry;
      entry.asset_id = asset.id;
      entry.expected_hash = asset.hash;
      entry.expected_size = asset.size;

      // Resolve file path
      fs::path file_path;
      if(!asset.original_filename.empty())
        file_path = opts.package_dir / asset.original_filename;
      else if(asset_paths.count(asset.id))
        file_path = opts.package_dir / asset_paths[asset.id];
      else
      {
        // Try to find by uuid in filename
        for(const auto& f : fs::directory_iterator(opts.package_dir))
        {
          if(f.path().filename().string().find(asset.id) != std::string::npos)
          {
            file_path = f.path();
            break;
          }
        }
      }

      entry.filename = file_path.filename().string();
      entry.file_exists = fs::exists(file_path);

      if(!entry.file_exists)
      {
        result.missing_files++;
        result.entries.push_back(entry);
        result.total_assets++;
        if(opts.stop_on_first_error)
          goto done;
        continue;
      }

      // Size check
      if(opts.verify_sizes && entry.expected_size > 0)
      {
        entry.actual_size = static_cast<int64_t>(fs::file_size(file_path));
        entry.size_match = (entry.actual_size == entry.expected_size);
        if(!entry.size_match)
          result.size_mismatches++;
      }
      else
      {
        entry.size_match = true;
      }

      // Hash check
      if(opts.verify_hashes && !entry.expected_hash.empty())
      {
        auto computed = sha1_base64(file_path);
        if(computed)
        {
          entry.computed_hash = *computed;
          entry.hash_match = (entry.computed_hash == entry.expected_hash);
          if(!entry.hash_match)
            result.hash_mismatches++;
        }
        else
        {
          entry.hash_match = false;
          result.hash_mismatches++;
        }
      }
      else
      {
        entry.hash_match = true;
      }

      if(entry.hash_match && entry.size_match && entry.file_exists)
        result.verified_ok++;

      result.entries.push_back(entry);
      result.total_assets++;

      if(opts.stop_on_first_error && (!entry.hash_match || !entry.size_match))
        goto done;
    }
  }

done:
  result.all_valid =
      (result.hash_mismatches == 0 && result.size_mismatches == 0 && result.missing_files == 0);
  result.success = true;
  return result;
}

} // namespace dcpdoctor
