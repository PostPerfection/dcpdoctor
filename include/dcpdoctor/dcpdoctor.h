#pragma once

#include <cstdint>
#include <filesystem>
#include <functional>
#include <string>
#include <string_view>
#include <vector>

namespace dcpdoctor
{

enum class Severity
{
  error,
  warning,
  info
};

enum class Code
{
  // Structure
  missing_assetmap,
  missing_pkl,
  missing_cpl,
  asset_not_found,
  duplicate_asset_id,

  // XML
  xml_parse_error,
  xml_schema_violation,
  invalid_uuid,
  missing_required_element,

  // PKL
  pkl_hash_mismatch,
  pkl_missing_asset_reference,

  // CPL
  cpl_invalid_duration,
  cpl_mismatched_durations,
  cpl_missing_reel,
  cpl_invalid_frame_rate,
  cpl_invalid_edit_rate,

  // MXF
  mxf_unreadable,
  mxf_hash_mismatch,
  mxf_invalid_structure,

  // Signature
  signature_invalid,
  certificate_expired,
  certificate_chain_broken,

  // SMPTE compliance
  smpte_naming_violation,
  smpte_namespace_wrong,

  // Interop compliance
  interop_namespace_wrong,

  // Picture
  picture_invalid_resolution,
  picture_invalid_frame_rate,
  j2k_bitrate_exceeded,
  j2k_invalid_profile,
  j2k_invalid_component_count,

  // Sound
  sound_invalid_sample_rate,
  sound_invalid_channel_count,

  // Subtitle
  subtitle_parse_error,
  subtitle_invalid_timing,
  subtitle_font_missing,

  // ISDCF naming
  isdcf_naming_violation,

  // Encryption
  encryption_detected,
  kdm_required,

  // Reel continuity
  reel_discontinuity,

  // 3D
  stereo_mismatch,

  // Markers
  marker_missing,
  marker_invalid,

  // Cross-reference
  cross_ref_broken,

  // Supplemental DCP
  supplemental_opl_missing,
};

struct Note
{
  Severity severity;
  Code code;
  std::string message;
  std::filesystem::path file;
  int line = 0;

  [[nodiscard]] std::string_view severity_str() const;
  [[nodiscard]] std::string_view code_str() const;
};

enum class Standard
{
  unknown,
  interop,
  smpte
};

/// Progress callback: (current_file, file_index, total_files, bytes_done, bytes_total)
using HashProgressCallback =
    std::function<void(const std::filesystem::path&, int, int, std::uintmax_t, std::uintmax_t)>;

struct VerifyOptions
{
  bool check_hashes = true;
  bool check_signatures = true;
  bool check_picture_details = false;
  bool strict_smpte = false;
  HashProgressCallback hash_progress;
};

struct VerifyResult
{
  Standard standard = Standard::unknown;
  std::vector<Note> notes;
  int error_count = 0;
  int warning_count = 0;

  [[nodiscard]] bool ok() const
  {
    return error_count == 0;
  }
  void add(Note note);
};

/// Verify a DCP at the given path
VerifyResult verify(const std::filesystem::path& dcp_dir, const VerifyOptions& opts = {});

} // namespace dcpdoctor
