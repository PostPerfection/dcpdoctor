#pragma once

#include <cstdint>
#include <filesystem>
#include <string>
#include <vector>

namespace dcpdoctor
{

/// User preferences stored in ~/.config/dcpdoctor/preferences.json
struct Preferences
{
  // Verification defaults
  std::string default_standard = "SMPTE"; // "SMPTE" or "Interop"
  bool verify_hashes = true;
  bool verify_schemas = true;
  bool check_bitrate = true;
  bool check_loudness = true;
  uint32_t max_bitrate_mbps = 250;

  // Report
  std::string report_format = "html"; // "html", "json", "text"
  std::string default_output_dir;

  // Schema paths
  std::string schema_dir; // custom XSD directory

  // KDM
  std::string signing_certificate_path;
  std::string signing_key_path;

  // Server
  uint32_t server_port = 8080;
  std::string server_bind = "127.0.0.1";

  // Theater database
  struct TheaterEntry
  {
    std::string name;
    std::string certificate_path;
  };
  std::vector<TheaterEntry> theater_list;

  // GUI
  std::string theme = "dark";
  bool show_advanced_options = false;
};

/// Get the platform-specific preferences file path.
std::filesystem::path preferences_path();

/// Load preferences from disk (returns defaults if file doesn't exist).
Preferences load_preferences();

/// Save preferences to disk.
int save_preferences(const Preferences& prefs);

} // namespace dcpdoctor
