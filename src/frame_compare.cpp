#include <spdlog/spdlog.h>
#include <cmath>
#include <cstdio>
#include <filesystem>
#include <fstream>
#include <regex>
#include <sstream>

#include "dcpdoctor/frame_compare.h"
#include "dcpdoctor/info.h"
#include "dcpdoctor/platform.h"

namespace fs = std::filesystem;

namespace dcpdoctor
{

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

static void write_csv(const fs::path& path, const std::vector<FrameDiff>& all_frames,
                      uint32_t total)
{
  std::ofstream csv(path);
  csv << "frame,psnr,ssim,significant\n";
  for(const auto& f : all_frames)
    csv << f.frame_number << "," << f.psnr << "," << f.ssim << ","
        << (f.significant ? "true" : "false") << "\n";
}

static void write_html_report(const fs::path& path, const CompareResult& result,
                              const CompareOptions& opts)
{
  std::ofstream html(path);
  html << "<!DOCTYPE html>\n<html><head><title>Frame Comparison Report</title>\n";
  html << "<style>\n";
  html << "body { font-family: sans-serif; margin: 2em; }\n";
  html << "table { border-collapse: collapse; width: 100%; }\n";
  html << "th, td { border: 1px solid #ddd; padding: 8px; text-align: left; }\n";
  html << "th { background: #333; color: white; }\n";
  html << "tr:nth-child(even) { background: #f2f2f2; }\n";
  html << ".bad { color: red; font-weight: bold; }\n";
  html << ".good { color: green; }\n";
  html << ".summary { display: grid; grid-template-columns: 1fr 1fr; gap: 1em; "
          "margin-bottom: 2em; }\n";
  html << ".card { border: 1px solid #ccc; padding: 1em; border-radius: 4px; }\n";
  html << "</style>\n</head><body>\n";
  html << "<h1>Frame Comparison Report</h1>\n";

  html << "<div class=\"summary\">\n";
  html << "<div class=\"card\"><h3>Summary</h3>\n";
  html << "<p>Frames compared: " << result.frames_compared << "</p>\n";
  html << "<p>Frames different: <span class=\"" << (result.frames_different > 0 ? "bad" : "good")
       << "\">" << result.frames_different << "</span></p>\n";
  html << "<p>Identical: " << (result.identical ? "Yes" : "No") << "</p>\n";
  html << "</div>\n";

  html << "<div class=\"card\"><h3>Metrics</h3>\n";
  html << "<p>Avg PSNR: " << result.avg_psnr << " dB</p>\n";
  html << "<p>Min PSNR: " << result.min_psnr << " dB</p>\n";
  if(opts.compute_ssim)
  {
    html << "<p>Avg SSIM: " << result.avg_ssim << "</p>\n";
    html << "<p>Min SSIM: " << result.min_ssim << "</p>\n";
  }
  if(opts.compute_vmaf)
    html << "<p>VMAF: " << result.vmaf_score << "</p>\n";
  html << "</div></div>\n";

  if(!result.diffs.empty())
  {
    html << "<h2>Significant Differences</h2>\n";
    html << "<table><tr><th>Frame</th><th>PSNR (dB)</th><th>SSIM</th></tr>\n";
    for(const auto& d : result.diffs)
    {
      html << "<tr><td>" << d.frame_number << "</td>";
      html << "<td class=\"bad\">" << d.psnr << "</td>";
      html << "<td>" << d.ssim << "</td></tr>\n";
    }
    html << "</table>\n";

    // Show diff frame images if they were extracted
    if(opts.extract_diff_frames && !opts.output_dir.empty())
    {
      html << "<h2>Visual Differences</h2>\n";
      for(const auto& d : result.diffs)
      {
        auto img = "diff_frame_" + std::to_string(d.frame_number) + ".png";
        if(fs::exists(opts.output_dir / img))
          html << "<p>Frame " << d.frame_number << ":</p><img src=\"" << img
               << "\" style=\"max-width:100%\">\n";
      }
    }
  }

  html << "</body></html>\n";
}

static void extract_diff_frame(const fs::path& video_a, const fs::path& video_b, uint32_t frame,
                               const fs::path& output_dir)
{
  // Extract frame from A and B, create a side-by-side + diff composite
  fs::create_directories(output_dir);
  std::string out = (output_dir / ("diff_frame_" + std::to_string(frame) + ".png")).string();

  // Create side-by-side with difference visualization
  std::string cmd = "ffmpeg -y -ss " + std::to_string(frame) + " -i \"" + video_a.string() +
                    "\" -ss " + std::to_string(frame) + " -i \"" + video_b.string() +
                    "\" -frames:v 1 -filter_complex "
                    "\"[0:v][1:v]hstack=inputs=2[top];"
                    "[0:v][1:v]blend=all_mode=difference[diff];"
                    "[top][diff]vstack=inputs=2\""
                    " \"" +
                    out + "\" 2>/dev/null";
  run_cmd(cmd);
}

CompareResult compare_files(const fs::path& file_a, const fs::path& file_b,
                            const CompareOptions& opts)
{
  CompareResult result;

  if(!fs::exists(file_a) || !fs::exists(file_b))
  {
    result.error = "One or both files do not exist";
    return result;
  }

  fs::path stats_file = fs::temp_directory_path() / "dcpdoctor_psnr.txt";
  fs::path ssim_stats = fs::temp_directory_path() / "dcpdoctor_ssim.txt";

  // Build ffmpeg filter
  std::string filter;

  // Frame range selection
  std::string seek_a, seek_b, frames_opt;
  if(opts.start_frame > 0)
  {
    seek_a = " -ss " + std::to_string(opts.start_frame);
    seek_b = seek_a;
  }
  if(opts.end_frame > 0 && opts.end_frame > opts.start_frame)
  {
    uint32_t count = opts.end_frame - opts.start_frame;
    frames_opt = " -frames:v " + std::to_string(count);
  }

  // Sample interval via frame select
  std::string select_filter;
  if(opts.sample_interval > 1)
    select_filter = "select='not(mod(n\\," + std::to_string(opts.sample_interval) + "))',";

  // PSNR
  std::string psnr_cmd = "ffmpeg" + seek_a + " -i \"" + file_a.string() + "\"" + seek_b + " -i \"" +
                         file_b.string() + "\"" + frames_opt + " -lavfi \"" + select_filter +
                         "psnr=stats_file=" + stats_file.string() + "\" -f null - 2>&1";

  std::string output = run_cmd(psnr_cmd);

  // Parse average PSNR from stderr
  auto avg_pos = output.find("average:");
  if(avg_pos != std::string::npos)
    result.avg_psnr = std::stod(output.substr(avg_pos + 8));

  // Parse per-frame PSNR stats
  std::vector<FrameDiff> all_frames;
  {
    std::ifstream stats(stats_file);
    std::string line;
    std::regex psnr_line_re(R"re(n:(\d+)\s+.*?psnr_avg:([\d.inf]+))re");
    while(std::getline(stats, line))
    {
      std::smatch m;
      if(std::regex_search(line, m, psnr_line_re))
      {
        FrameDiff diff;
        diff.frame_number = static_cast<uint32_t>(std::stoul(m[1].str()));
        std::string psnr_str = m[2].str();
        diff.psnr = (psnr_str == "inf") ? 100.0 : std::stod(psnr_str);
        diff.significant = (diff.psnr < opts.threshold_psnr);
        all_frames.push_back(diff);
        result.frames_compared++;

        if(diff.significant)
        {
          result.frames_different++;
          result.diffs.push_back(diff);
        }
        if(diff.psnr < result.min_psnr || result.min_psnr == 0)
          result.min_psnr = diff.psnr;
      }
    }
    fs::remove(stats_file);
  }

  // SSIM
  if(opts.compute_ssim)
  {
    std::string ssim_cmd = "ffmpeg" + seek_a + " -i \"" + file_a.string() + "\"" + seek_b +
                           " -i \"" + file_b.string() + "\"" + frames_opt + " -lavfi \"" +
                           select_filter + "ssim=stats_file=" + ssim_stats.string() +
                           "\" -f null - 2>&1";

    std::string ssim_out = run_cmd(ssim_cmd);

    // Parse average SSIM
    std::regex ssim_avg_re(R"re(All:([\d.]+))re");
    std::smatch sm;
    if(std::regex_search(ssim_out, sm, ssim_avg_re))
      result.avg_ssim = std::stod(sm[1].str());

    // Parse per-frame SSIM
    std::ifstream ssim_f(ssim_stats);
    std::string line;
    std::regex ssim_line_re(R"re(n:(\d+)\s+.*?All:([\d.]+))re");
    size_t idx = 0;
    result.min_ssim = 1.0;
    while(std::getline(ssim_f, line))
    {
      std::smatch m;
      if(std::regex_search(line, m, ssim_line_re))
      {
        double ssim_val = std::stod(m[2].str());
        if(idx < all_frames.size())
          all_frames[idx].ssim = ssim_val;
        if(ssim_val < result.min_ssim)
          result.min_ssim = ssim_val;

        // Also check SSIM threshold for significant diffs
        if(ssim_val < opts.threshold_ssim && idx < all_frames.size() &&
           !all_frames[idx].significant)
        {
          all_frames[idx].significant = true;
          result.frames_different++;
          result.diffs.push_back(all_frames[idx]);
        }
        idx++;
      }
    }
    fs::remove(ssim_stats);
  }

  // VMAF
  if(opts.compute_vmaf)
  {
    std::string model_opt;
    if(!opts.vmaf_model.empty())
      model_opt = ":model_path=" + opts.vmaf_model.string();

    std::string vmaf_cmd = "ffmpeg" + seek_a + " -i \"" + file_a.string() + "\"" + seek_b +
                           " -i \"" + file_b.string() + "\"" + frames_opt + " -lavfi \"" +
                           select_filter + "libvmaf" + model_opt + "\" -f null - 2>&1";

    std::string vmaf_out = run_cmd(vmaf_cmd);
    std::regex vmaf_re(R"re(VMAF score:\s*([\d.]+))re");
    std::smatch vm;
    if(std::regex_search(vmaf_out, vm, vmaf_re))
      result.vmaf_score = std::stod(vm[1].str());
  }

  // Extract diff frames as images
  if(opts.extract_diff_frames && !opts.output_dir.empty())
  {
    for(const auto& d : result.diffs)
      extract_diff_frame(file_a, file_b, d.frame_number, opts.output_dir);
  }

  result.identical = (result.frames_different == 0);
  result.success = true;

  // Write reports
  if(!opts.output_dir.empty() && result.frames_compared > 0)
  {
    fs::create_directories(opts.output_dir);

    // CSV
    result.csv_path = opts.output_dir / "per_frame_metrics.csv";
    write_csv(result.csv_path, all_frames, result.frames_compared);

    // JSON
    if(opts.generate_report)
    {
      result.report_path = opts.output_dir / "comparison_report.json";
      std::ofstream report(result.report_path);
      report << "{\n";
      report << "  \"frames_compared\": " << result.frames_compared << ",\n";
      report << "  \"frames_different\": " << result.frames_different << ",\n";
      report << "  \"avg_psnr\": " << result.avg_psnr << ",\n";
      report << "  \"min_psnr\": " << result.min_psnr << ",\n";
      report << "  \"avg_ssim\": " << result.avg_ssim << ",\n";
      report << "  \"min_ssim\": " << result.min_ssim << ",\n";
      if(opts.compute_vmaf)
        report << "  \"vmaf_score\": " << result.vmaf_score << ",\n";
      report << "  \"identical\": " << (result.identical ? "true" : "false") << "\n";
      report << "}\n";
    }

    // HTML
    if(opts.generate_html)
    {
      result.html_report_path = opts.output_dir / "comparison_report.html";
      write_html_report(result.html_report_path, result, opts);
    }
  }

  return result;
}

CompareResult compare_imps(const CompareOptions& opts)
{
  CompareResult result;

  if(!fs::exists(opts.imp_a) || !fs::exists(opts.imp_b))
  {
    result.error = "One or both IMP directories do not exist";
    return result;
  }

  auto info_a = read_imp_info(opts.imp_a);
  auto info_b = read_imp_info(opts.imp_b);

  if(!info_a.valid || !info_b.valid)
  {
    result.error = "Failed to read IMP info: " + info_a.error + " / " + info_b.error;
    return result;
  }

  // Find video track files
  fs::path video_a, video_b;
  for(auto& t : info_a.tracks)
  {
    if(t.type == "video")
    {
      video_a = opts.imp_a / t.filename;
      break;
    }
  }
  for(auto& t : info_b.tracks)
  {
    if(t.type == "video")
    {
      video_b = opts.imp_b / t.filename;
      break;
    }
  }

  if(video_a.empty() || video_b.empty())
  {
    result.error = "Cannot find video tracks in one or both IMPs";
    return result;
  }

  return compare_files(video_a, video_b, opts);
}

} // namespace dcpdoctor
