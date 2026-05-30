#pragma once

#include <cstdint>
#include <filesystem>
#include <string>
#include <vector>

namespace dcpdoctor
{

struct FrameDiff
{
  uint32_t frame_number = 0;
  double psnr = 0; // Peak Signal-to-Noise Ratio (dB)
  double ssim = 0; // Structural Similarity (0-1)
  double mse = 0; // Mean Squared Error
  bool significant = false; // above threshold
};

enum class CompareMetric
{
  PSNR,
  SSIM,
  VMAF
};

struct CompareOptions
{
  std::filesystem::path imp_a; // first IMP directory
  std::filesystem::path imp_b; // second IMP directory
  double threshold_psnr = 40.0; // dB threshold for "significant" diff
  double threshold_ssim = 0.95; // SSIM threshold
  uint32_t sample_interval = 1; // compare every Nth frame (1 = all)
  uint32_t start_frame = 0; // 0 = from beginning
  uint32_t end_frame = 0; // 0 = to end
  std::filesystem::path output_dir; // optional: write diff frames & reports here
  bool generate_report = true;
  bool generate_html = false; // generate HTML visual report
  bool extract_diff_frames = false; // extract PNG frames where diffs found
  bool compute_ssim = true;
  bool compute_vmaf = false; // requires vmaf model file
  std::filesystem::path vmaf_model; // path to VMAF model (optional)
};

struct CompareResult
{
  uint32_t frames_compared = 0;
  uint32_t frames_different = 0;
  double avg_psnr = 0;
  double min_psnr = 0;
  double avg_ssim = 0;
  double min_ssim = 0;
  double vmaf_score = 0; // average VMAF (0-100)
  std::vector<FrameDiff> diffs; // only significant diffs
  std::filesystem::path report_path;
  std::filesystem::path html_report_path;
  std::filesystem::path csv_path;
  bool identical = false;
  bool success = false;
  std::string error;
};

// Compare two IMPs frame-by-frame using PSNR/SSIM/VMAF
CompareResult compare_imps(const CompareOptions& opts);

// Compare two individual video files (MXF, MP4, etc.)
CompareResult compare_files(const std::filesystem::path& file_a,
                            const std::filesystem::path& file_b, const CompareOptions& opts);

} // namespace dcpdoctor
