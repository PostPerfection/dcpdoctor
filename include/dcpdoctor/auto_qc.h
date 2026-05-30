#pragma once

#include <cstdint>
#include <filesystem>
#include <string>
#include <vector>

namespace dcpdoctor
{

enum class QcIssueType
{
  BlackFrame,
  FreezeFrame,
  AudioSilence,
  AudioClipping
};

struct QcIssue
{
  QcIssueType type;
  double start_timecode_sec = 0;
  double end_timecode_sec = 0;
  double duration_sec = 0;
  std::string description;
  std::string severity; // "error", "warning", "info"
};

struct AutoQcOptions
{
  std::filesystem::path video_path; // Video file (MXF, MP4, etc.)
  std::filesystem::path audio_path; // Audio file (WAV, MXF) — optional
  double black_threshold = 0.98; // pixel ratio for black detection
  double black_duration_min = 0.5; // minimum seconds to flag
  double freeze_threshold = 0.003; // noise threshold for freeze
  double freeze_duration_min = 2.0; // minimum seconds to flag
  double silence_threshold = -60.0; // dB threshold for silence
  double silence_duration_min = 1.0;
  double clipping_threshold = -0.5; // dBFS max volume trigger
};

struct AutoQcResult
{
  std::vector<QcIssue> issues;
  uint32_t total_frames = 0;
  double duration_seconds = 0;
  bool success = false;
  std::string error;
};

/// Run automated QC analysis on a video/audio file
AutoQcResult run_auto_qc(const AutoQcOptions& opts);

/// Generate JSON string from QC results
std::string auto_qc_to_json(const AutoQcResult& result);

} // namespace dcpdoctor
