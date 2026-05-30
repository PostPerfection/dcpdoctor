#include <spdlog/spdlog.h>
#include <chrono>
#include <cstdio>
#include <filesystem>
#include <fstream>
#include <iomanip>
#include <sstream>

#include "dcpdoctor/qc_report.h"
#include "dcpdoctor/info.h"
#include "dcpdoctor/loudness.h"
#include "dcpdoctor/platform.h"

namespace fs = std::filesystem;

namespace dcpdoctor
{

static std::string current_timestamp()
{
  auto now = std::chrono::system_clock::now();
  auto time_t = std::chrono::system_clock::to_time_t(now);
  std::tm tm_buf;
  DCPDOCTOR_GMTIME_R(&time_t, &tm_buf);
  std::ostringstream oss;
  oss << std::put_time(&tm_buf, "%Y-%m-%d %H:%M:%S UTC");
  return oss.str();
}

DetailedQcResult generate_detailed_qc(const DetailedQcOptions& opts)
{
  DetailedQcResult result;

  if(!fs::exists(opts.imp_dir))
  {
    result.error = "IMP directory not found: " + opts.imp_dir.string();
    return result;
  }

  auto info = read_imp_info(opts.imp_dir);
  if(!info.valid)
  {
    result.error = "Cannot read IMP: " + info.error;
    return result;
  }

  // Generate HTML report (write to temp file if PDF is requested)
  bool want_pdf = opts.output_file.extension() == ".pdf";
  auto html_path =
      want_pdf ? fs::path(opts.output_file).replace_extension(".tmp.html") : opts.output_file;

  std::ofstream html(html_path);
  if(!html.is_open())
  {
    result.error = "Cannot create report file: " + html_path.string();
    return result;
  }

  html << "<!DOCTYPE html>\n<html>\n<head>\n";
  html << "<meta charset=\"utf-8\">\n";
  html << "<title>QC Report — " << (opts.title.empty() ? info.cpl_title : opts.title)
       << "</title>\n";
  html << "<style>\n";
  html << "body { font-family: -apple-system, sans-serif; max-width: 1000px; margin: 0 auto; "
          "padding: 2rem; }\n";
  html << "h1 { border-bottom: 2px solid #333; padding-bottom: 0.5rem; }\n";
  html << "table { width: 100%; border-collapse: collapse; margin: 1rem 0; }\n";
  html << "th, td { border: 1px solid #ddd; padding: 0.5rem; text-align: left; }\n";
  html << "th { background: #f5f5f5; }\n";
  html << ".pass { color: #16a34a; font-weight: bold; }\n";
  html << ".fail { color: #dc2626; font-weight: bold; }\n";
  html << ".meta { color: #666; font-size: 0.9rem; }\n";
  html << "</style>\n</head>\n<body>\n";

  html << "<h1>QC Report</h1>\n";
  html << "<p class=\"meta\">Generated: " << current_timestamp() << "</p>\n";
  if(!opts.client.empty())
    html << "<p class=\"meta\">Client: " << opts.client << "</p>\n";

  // Package info
  html << "<h2>Package Information</h2>\n";
  html << "<table>\n";
  html << "<tr><th>Title</th><td>" << info.cpl_title << "</td></tr>\n";
  html << "<tr><th>CPL UUID</th><td>" << info.cpl_uuid << "</td></tr>\n";
  html << "<tr><th>PKL UUID</th><td>" << info.pkl_uuid << "</td></tr>\n";
  html << "<tr><th>Issuer</th><td>" << info.issuer << "</td></tr>\n";
  html << "<tr><th>Issue Date</th><td>" << info.issue_date << "</td></tr>\n";
  html << "</table>\n";

  // Track files
  html << "<h2>Track Files</h2>\n";
  html << "<table>\n";
  html << "<tr><th>Type</th><th>Filename</th><th>UUID</th><th>Size</th></tr>\n";
  for(auto& t : info.tracks)
  {
    html << "<tr><td>" << t.type << "</td><td>" << t.filename << "</td><td>" << t.uuid
         << "</td><td>" << (t.size / 1024 / 1024) << " MB</td></tr>\n";
  }
  html << "</table>\n";

  // Loudness (if audio exists)
  if(opts.include_loudness)
  {
    for(auto& t : info.tracks)
    {
      if(t.type != "audio")
        continue;
      auto audio_path = opts.imp_dir / t.filename;
      auto loud = measure_imf_loudness(audio_path);
      if(loud.success)
      {
        html << "<h2>Audio Loudness</h2>\n";
        html << "<table>\n";
        html << "<tr><th>Metric</th><th>Value</th><th>Status</th></tr>\n";
        html << "<tr><td>Integrated Loudness</td><td>" << std::fixed << std::setprecision(1)
             << loud.integrated_lufs << " LUFS</td><td class=\""
             << (loud.compliant_r128 ? "pass" : "fail") << "\">"
             << (loud.compliant_r128 ? "PASS" : "FAIL") << "</td></tr>\n";
        html << "<tr><td>True Peak</td><td>" << loud.true_peak_dbtp << " dBTP</td><td class=\""
             << (loud.true_peak_dbtp <= -1.0 ? "pass" : "fail") << "\">"
             << (loud.true_peak_dbtp <= -1.0 ? "PASS" : "FAIL") << "</td></tr>\n";
        html << "<tr><td>Loudness Range</td><td>" << loud.loudness_range_lu
             << " LU</td><td>—</td></tr>\n";
        html << "</table>\n";
      }
      break;
    }
  }

  html << "</body>\n</html>\n";
  html.close();

  // If the output is PDF, convert HTML to PDF via wkhtmltopdf or weasyprint
  if(want_pdf)
  {
    std::string pdf_cmd = "wkhtmltopdf --quiet \"" + html_path.string() + "\" \"" +
                          opts.output_file.string() + "\" 2>/dev/null";
    int rc = std::system(pdf_cmd.c_str());
    if(rc != 0)
    {
      pdf_cmd = "weasyprint \"" + html_path.string() + "\" \"" + opts.output_file.string() +
                "\" 2>/dev/null";
      rc = std::system(pdf_cmd.c_str());
    }

    // Clean up temp HTML
    std::error_code ec;
    fs::remove(html_path, ec);

    if(rc != 0)
    {
      result.error = "PDF conversion failed — install wkhtmltopdf or weasyprint";
      return result;
    }
  }

  result.output_file = opts.output_file;
  result.pages = 1;
  result.success = true;
  return result;
}

} // namespace dcpdoctor
