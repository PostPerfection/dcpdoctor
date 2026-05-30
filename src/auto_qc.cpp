#include <spdlog/spdlog.h>
#include <cstdio>
#include <regex>
#include <sstream>

#include "dcpdoctor/auto_qc.h"
#include "dcpdoctor/platform.h"

namespace dcpdoctor
{

static std::string run_ffmpeg(const std::string& cmd)
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

static std::vector<QcIssue> detect_black_frames(const std::filesystem::path& video,
                                                double threshold, double min_duration)
{
  std::vector<QcIssue> issues;

  std::string cmd = "ffmpeg -i \"" + video.string() +
                    "\" -vf \"blackdetect=d=" + std::to_string(min_duration) +
                    ":pic_th=" + std::to_string(threshold) + "\" -an -f null - 2>&1";

  std::string output = run_ffmpeg(cmd);

  std::regex re(R"re(black_start:([\d.]+)\s+black_end:([\d.]+)\s+black_duration:([\d.]+))re");
  auto begin = std::sregex_iterator(output.begin(), output.end(), re);
  auto end = std::sregex_iterator();

  for(auto it = begin; it != end; ++it)
  {
    QcIssue issue;
    issue.type = QcIssueType::BlackFrame;
    issue.start_timecode_sec = std::stod((*it)[1].str());
    issue.end_timecode_sec = std::stod((*it)[2].str());
    issue.duration_sec = std::stod((*it)[3].str());
    issue.severity = issue.duration_sec > 5.0 ? "warning" : "info";
    issue.description = "Black frames: " + std::to_string(issue.duration_sec) + "s at " +
                        std::to_string(issue.start_timecode_sec) + "s";
    issues.push_back(issue);
  }

  return issues;
}

static std::vector<QcIssue> detect_freeze_frames(const std::filesystem::path& video,
                                                 double threshold, double min_duration)
{
  std::vector<QcIssue> issues;

  std::string cmd = "ffmpeg -i \"" + video.string() +
                    "\" -vf \"freezedetect=n=" + std::to_string(threshold) +
                    ":d=" + std::to_string(min_duration) + "\" -an -f null - 2>&1";

  std::string output = run_ffmpeg(cmd);

  std::regex start_re(R"re(freeze_start:\s*([\d.]+))re");
  std::regex end_re(R"re(freeze_end:\s*([\d.]+)\s*\|\s*freeze_duration:\s*([\d.]+))re");

  std::vector<double> starts;
  auto begin = std::sregex_iterator(output.begin(), output.end(), start_re);
  auto end_it = std::sregex_iterator();
  for(auto it = begin; it != end_it; ++it)
    starts.push_back(std::stod((*it)[1].str()));

  size_t idx = 0;
  begin = std::sregex_iterator(output.begin(), output.end(), end_re);
  for(auto it = begin; it != end_it; ++it)
  {
    QcIssue issue;
    issue.type = QcIssueType::FreezeFrame;
    issue.end_timecode_sec = std::stod((*it)[1].str());
    issue.duration_sec = std::stod((*it)[2].str());
    issue.start_timecode_sec =
        idx < starts.size() ? starts[idx] : issue.end_timecode_sec - issue.duration_sec;
    issue.severity = issue.duration_sec > 10.0 ? "warning" : "info";
    issue.description = "Freeze frame: " + std::to_string(issue.duration_sec) + "s at " +
                        std::to_string(issue.start_timecode_sec) + "s";
    issues.push_back(issue);
    idx++;
  }

  return issues;
}

static std::vector<QcIssue> detect_silence(const std::filesystem::path& audio, double threshold_db,
                                           double min_duration)
{
  std::vector<QcIssue> issues;

  std::string cmd = "ffmpeg -i \"" + audio.string() +
                    "\" -af \"silencedetect=noise=" + std::to_string(threshold_db) +
                    "dB:d=" + std::to_string(min_duration) + "\" -vn -f null - 2>&1";

  std::string output = run_ffmpeg(cmd);

  std::regex start_re(R"re(silence_start:\s*([\d.]+))re");
  std::regex end_re(R"re(silence_end:\s*([\d.]+)\s*\|\s*silence_duration:\s*([\d.]+))re");

  std::vector<double> starts;
  auto begin = std::sregex_iterator(output.begin(), output.end(), start_re);
  auto end_it = std::sregex_iterator();
  for(auto it = begin; it != end_it; ++it)
    starts.push_back(std::stod((*it)[1].str()));

  size_t idx = 0;
  begin = std::sregex_iterator(output.begin(), output.end(), end_re);
  for(auto it = begin; it != end_it; ++it)
  {
    QcIssue issue;
    issue.type = QcIssueType::AudioSilence;
    issue.end_timecode_sec = std::stod((*it)[1].str());
    issue.duration_sec = std::stod((*it)[2].str());
    issue.start_timecode_sec =
        idx < starts.size() ? starts[idx] : issue.end_timecode_sec - issue.duration_sec;
    issue.severity = issue.duration_sec > 5.0 ? "warning" : "info";
    issue.description = "Audio silence: " + std::to_string(issue.duration_sec) + "s at " +
                        std::to_string(issue.start_timecode_sec) + "s";
    issues.push_back(issue);
    idx++;
  }

  return issues;
}

static std::vector<QcIssue> detect_clipping(const std::filesystem::path& audio,
                                            double threshold_dbfs)
{
  std::vector<QcIssue> issues;

  std::string cmd = "ffmpeg -i \"" + audio.string() + "\" -af \"volumedetect\" -vn -f null - 2>&1";

  std::string output = run_ffmpeg(cmd);

  std::regex maxvol_re(R"re(max_volume:\s*([-\d.]+)\s*dB)re");
  std::smatch m;
  if(std::regex_search(output, m, maxvol_re))
  {
    double max_vol = std::stod(m[1].str());
    if(max_vol > threshold_dbfs)
    {
      QcIssue issue;
      issue.type = QcIssueType::AudioClipping;
      issue.severity = "error";
      issue.description = "Audio clipping detected: max volume " + std::to_string(max_vol) +
                          " dBFS (threshold: " + std::to_string(threshold_dbfs) + " dBFS)";
      issues.push_back(issue);
    }
  }

  return issues;
}

AutoQcResult run_auto_qc(const AutoQcOptions& opts)
{
  AutoQcResult result;

  if(!std::filesystem::exists(opts.video_path) && !std::filesystem::exists(opts.audio_path))
  {
    result.error = "No valid input files specified";
    return result;
  }

  // Video checks
  if(std::filesystem::exists(opts.video_path))
  {
    auto black =
        detect_black_frames(opts.video_path, opts.black_threshold, opts.black_duration_min);
    result.issues.insert(result.issues.end(), black.begin(), black.end());

    auto freeze =
        detect_freeze_frames(opts.video_path, opts.freeze_threshold, opts.freeze_duration_min);
    result.issues.insert(result.issues.end(), freeze.begin(), freeze.end());

    // Also check audio from video file if no separate audio specified
    if(opts.audio_path.empty())
    {
      auto silence =
          detect_silence(opts.video_path, opts.silence_threshold, opts.silence_duration_min);
      result.issues.insert(result.issues.end(), silence.begin(), silence.end());

      auto clipping = detect_clipping(opts.video_path, opts.clipping_threshold);
      result.issues.insert(result.issues.end(), clipping.begin(), clipping.end());
    }
  }

  // Separate audio checks
  if(!opts.audio_path.empty() && std::filesystem::exists(opts.audio_path))
  {
    auto silence =
        detect_silence(opts.audio_path, opts.silence_threshold, opts.silence_duration_min);
    result.issues.insert(result.issues.end(), silence.begin(), silence.end());

    auto clipping = detect_clipping(opts.audio_path, opts.clipping_threshold);
    result.issues.insert(result.issues.end(), clipping.begin(), clipping.end());
  }

  result.success = true;
  return result;
}

std::string auto_qc_to_json(const AutoQcResult& result)
{
  std::ostringstream json;
  json << "{\n";
  json << "  \"success\": " << (result.success ? "true" : "false") << ",\n";
  json << "  \"total_issues\": " << result.issues.size() << ",\n";
  json << "  \"issues\": [\n";
  for(size_t i = 0; i < result.issues.size(); ++i)
  {
    const auto& issue = result.issues[i];
    json << "    {\n";
    json << "      \"type\": \"";
    switch(issue.type)
    {
      case QcIssueType::BlackFrame:
        json << "black_frame";
        break;
      case QcIssueType::FreezeFrame:
        json << "freeze_frame";
        break;
      case QcIssueType::AudioSilence:
        json << "audio_silence";
        break;
      case QcIssueType::AudioClipping:
        json << "audio_clipping";
        break;
    }
    json << "\",\n";
    json << "      \"start\": " << issue.start_timecode_sec << ",\n";
    json << "      \"end\": " << issue.end_timecode_sec << ",\n";
    json << "      \"duration\": " << issue.duration_sec << ",\n";
    json << "      \"severity\": \"" << issue.severity << "\",\n";
    json << "      \"description\": \"" << issue.description << "\"\n";
    json << "    }" << (i + 1 < result.issues.size() ? "," : "") << "\n";
  }
  json << "  ]\n";
  json << "}\n";
  return json.str();
}

} // namespace dcpdoctor
