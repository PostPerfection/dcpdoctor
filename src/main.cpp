#include <CLI/CLI.hpp>
#include <spdlog/spdlog.h>
#include <libxml/parser.h>
#include <libxml/xmlerror.h>
#include <cstdio>
#include <filesystem>
#include <fstream>
#include <sstream>
#include <string>
#include <vector>
#ifdef _WIN32
#include <io.h>
#else
#include <unistd.h>
#endif
#include "dcpdoctor/dcpdoctor.h"
#include "dcpdoctor/advanced.h"
#include "dcpdoctor/auto_qc.h"
#include "dcpdoctor/av_sync.h"
#include "dcpdoctor/checksum_verify.h"
#include "dcpdoctor/diff.h"
#include "dcpdoctor/fixes.h"
#include "dcpdoctor/frame_compare.h"
#include "dcpdoctor/hdr_validate.h"
#include "dcpdoctor/imf_compliance.h"
#include "dcpdoctor/info.h"
#include "dcpdoctor/kdm.h"
#include "dcpdoctor/loudness.h"
#include "dcpdoctor/mxf_extract.h"
#include "dcpdoctor/photon.h"
#include "dcpdoctor/premium.h"
#include "dcpdoctor/qc.h"
#include "dcpdoctor/qc_report.h"
#include "dcpdoctor/schema_validate.h"
#include "dcpdoctor/studio.h"
#include "dcpdoctor/report.h"
#include "dcpdoctor/server.h"
#include "dcpdoctor/theater.h"
#include "dcpdoctor/timeline.h"
#include "dcpdoctor/validate.h"

namespace fs = std::filesystem;

static void suppress_xml_error(void* /*ctx*/, const char* /*msg*/, ...) {}

int main(int argc, char* argv[])
{
  xmlSetGenericErrorFunc(nullptr, suppress_xml_error);
  xmlSetStructuredErrorFunc(nullptr, nullptr);

  CLI::App app{"DcpDoctor - DCP validator and verifier"};
  app.set_version_flag("--version", "0.1.0");
  app.require_subcommand(0, 1);

  // === VALIDATE subcommand (default) ===
  auto* validate_cmd = app.add_subcommand("validate", "Validate DCP directories");

  bool verbose = false, quiet = false, json = false, html = false;
  bool no_hashes = false, no_signatures = false, check_mxf = false, strict = false;
  bool bv21 = false, deep_j2k = false;
  std::string output_file, manifest_file, timeline_file;
  std::vector<std::string> dcp_dirs;

  validate_cmd->add_flag("-v,--verbose", verbose, "Show info-level notes");
  validate_cmd->add_flag("-q,--quiet", quiet, "Only show errors");
  validate_cmd->add_flag("--json", json, "JSON output");
  validate_cmd->add_flag("--html", html, "HTML report output");
  validate_cmd->add_flag("--no-hashes", no_hashes, "Skip hash verification");
  validate_cmd->add_flag("--no-signatures", no_signatures, "Skip signature verification");
  validate_cmd->add_flag("--check-mxf", check_mxf, "Inspect MXF essence metadata");
  validate_cmd->add_flag("--strict", strict, "Strict SMPTE compliance");
  validate_cmd->add_flag("--bv21", bv21, "Check BV2.1 application profile");
  validate_cmd->add_flag("--deep-j2k", deep_j2k, "Deep J2K codestream validation");
  validate_cmd->add_option("-o,--output", output_file, "Write report to file");
  validate_cmd->add_option("--manifest", manifest_file, "Compare against manifest JSON");
  validate_cmd->add_option("--timeline", timeline_file, "Write SVG timeline to file");
  validate_cmd->add_option("dcp_dirs", dcp_dirs, "DCP directories to validate")
      ->required()
      ->check(CLI::ExistingDirectory);

  // === WATCH subcommand ===
  auto* watch_cmd = app.add_subcommand("watch", "Watch directory for new DCPs");
  std::string watch_dir;
  int poll_interval = 5000;
  watch_cmd->add_option("directory", watch_dir, "Directory to watch")
      ->required()
      ->check(CLI::ExistingDirectory);
  watch_cmd->add_option("--interval", poll_interval, "Poll interval in ms (default: 5000)");

  // === SERVE subcommand ===
  auto* serve_cmd = app.add_subcommand("serve", "Start REST API server");
  std::string bind_addr = "0.0.0.0";
  int port = 8080;
  serve_cmd->add_option("--bind", bind_addr, "Bind address (default: 0.0.0.0)");
  serve_cmd->add_option("--port,-p", port, "Port (default: 8080)");

  // === DIFF subcommand ===
  auto* diff_cmd = app.add_subcommand("diff", "Compare two DCPs");
  std::string diff_a, diff_b;
  bool diff_hashes = false;
  diff_cmd->add_option("dcp_a", diff_a, "First DCP directory")
      ->required()
      ->check(CLI::ExistingDirectory);
  diff_cmd->add_option("dcp_b", diff_b, "Second DCP directory")
      ->required()
      ->check(CLI::ExistingDirectory);
  diff_cmd->add_flag("--hashes", diff_hashes, "Compare content hashes (slow)");

  // === PROFILES subcommand ===
  auto* profiles_cmd =
      app.add_subcommand("profiles", "List or check theater compatibility profiles");
  std::string profile_name;
  std::string profile_dcp;
  profiles_cmd->add_option("--check", profile_name, "Check DCP against named profile");
  profiles_cmd->add_option("--dcp", profile_dcp, "DCP directory to check")
      ->check(CLI::ExistingDirectory);

  // === KDM subcommand ===
  auto* kdm_cmd = app.add_subcommand("kdm", "Validate a KDM file");
  std::string kdm_file, kdm_dcp;
  kdm_cmd->add_option("kdm_file", kdm_file, "KDM XML file")->required()->check(CLI::ExistingFile);
  kdm_cmd->add_option("--dcp", kdm_dcp, "DCP directory to validate against")
      ->check(CLI::ExistingDirectory);

  // === CHECKSUM-VERIFY subcommand ===
  auto* checksum_cmd = app.add_subcommand("checksum-verify", "Verify PKL checksums for DCP/IMF");
  std::string checksum_dir;
  bool checksum_no_hash = false, checksum_no_size = false, checksum_stop = false;
  bool checksum_json = false;
  checksum_cmd->add_option("directory", checksum_dir, "DCP or IMP directory")
      ->required()
      ->check(CLI::ExistingDirectory);
  checksum_cmd->add_flag("--no-hash", checksum_no_hash, "Skip hash verification (fast)");
  checksum_cmd->add_flag("--no-size", checksum_no_size, "Skip size verification");
  checksum_cmd->add_flag("--stop-on-error", checksum_stop, "Stop on first mismatch");
  checksum_cmd->add_flag("--json", checksum_json, "JSON output");

  // === MXF-EXTRACT subcommand ===
  auto* mxf_extract_cmd = app.add_subcommand("mxf-extract", "Extract essence from MXF container");
  std::string mxf_input, mxf_output_dir;
  bool mxf_no_video = false, mxf_no_audio = false;
  uint32_t mxf_start = 0, mxf_end = 0;
  mxf_extract_cmd->add_option("input", mxf_input, "MXF file")->required()->check(CLI::ExistingFile);
  mxf_extract_cmd->add_option("-o,--output", mxf_output_dir, "Output directory")->required();
  mxf_extract_cmd->add_flag("--no-video", mxf_no_video, "Skip video extraction");
  mxf_extract_cmd->add_flag("--no-audio", mxf_no_audio, "Skip audio extraction");
  mxf_extract_cmd->add_option("--start-frame", mxf_start, "Start frame (0 = beginning)");
  mxf_extract_cmd->add_option("--end-frame", mxf_end, "End frame (0 = end)");

  // === AUTO-QC subcommand ===
  auto* auto_qc_cmd = app.add_subcommand("auto-qc", "Automated QC: black/freeze/silence/clipping");
  std::string qc_video, qc_audio;
  bool qc_json_flag = false;
  double qc_black_thresh = 0.98, qc_freeze_thresh = 0.003;
  double qc_silence_thresh = -60.0, qc_clip_thresh = -0.5;
  auto_qc_cmd->add_option("-v,--video", qc_video, "Video file (MXF, MP4, etc.)")
      ->check(CLI::ExistingFile);
  auto_qc_cmd->add_option("-a,--audio", qc_audio, "Audio file (WAV, MXF)")
      ->check(CLI::ExistingFile);
  auto_qc_cmd->add_flag("--json", qc_json_flag, "JSON output");
  auto_qc_cmd->add_option("--black-threshold", qc_black_thresh, "Black pixel ratio")
      ->default_val(0.98);
  auto_qc_cmd->add_option("--freeze-threshold", qc_freeze_thresh, "Freeze noise threshold")
      ->default_val(0.003);
  auto_qc_cmd->add_option("--silence-threshold", qc_silence_thresh, "Silence dB threshold")
      ->default_val(-60.0);
  auto_qc_cmd->add_option("--clipping-threshold", qc_clip_thresh, "Clipping dBFS threshold")
      ->default_val(-0.5);

  // === VALIDATE-IMP subcommand ===
  auto* validate_imp_cmd = app.add_subcommand("validate-imp", "Validate IMP via Netflix Photon");
  std::string vimp_dir;
  validate_imp_cmd->add_option("imp_dir", vimp_dir, "IMP directory")
      ->required()
      ->check(CLI::ExistingDirectory);

  // === SCHEMA-VALIDATE subcommand ===
  auto* schema_val_cmd =
      app.add_subcommand("schema-validate", "XML schema validation against SMPTE XSDs");
  std::string sv_imp_dir, sv_schema_dir;
  bool sv_cpl = true, sv_pkl = true, sv_assetmap = true;
  schema_val_cmd->add_option("imp_dir", sv_imp_dir, "IMP directory")
      ->required()
      ->check(CLI::ExistingDirectory);
  schema_val_cmd->add_option("--schema-dir", sv_schema_dir, "Directory containing XSD files");
  schema_val_cmd->add_flag("--no-cpl", [&](auto) { sv_cpl = false; }, "Skip CPL validation");
  schema_val_cmd->add_flag("--no-pkl", [&](auto) { sv_pkl = false; }, "Skip PKL validation");
  schema_val_cmd->add_flag(
      "--no-assetmap", [&](auto) { sv_assetmap = false; }, "Skip ASSETMAP validation");

  // === IMF-COMPLIANCE subcommand ===
  auto* imfcomp_cmd =
      app.add_subcommand("imf-compliance", "Platform-specific IMF compliance checks");
  std::string imfcomp_dir, imfcomp_target;
  bool imfcomp_strict = true;
  imfcomp_cmd->add_option("imp_dir", imfcomp_dir, "IMP directory")
      ->required()
      ->check(CLI::ExistingDirectory);
  imfcomp_cmd->add_option("-t,--target", imfcomp_target, "Target platform")
      ->required()
      ->check(CLI::IsMember({"netflix", "disney", "amazon", "apple", "cinema2k", "cinema4k",
                             "broadcast-hd", "broadcast-uhd"}));
  imfcomp_cmd->add_flag(
      "--no-strict", [&](auto) { imfcomp_strict = false; }, "Allow warnings to pass");

  // === FRAME-QC subcommand ===
  auto* frameqc_cmd = app.add_subcommand("frame-qc", "Frame-level bitrate QC analysis");
  std::string fqc_dir;
  double fqc_max_br = 300.0, fqc_min_br = 50.0, fqc_target_br = 250.0;
  frameqc_cmd->add_option("j2k_dir", fqc_dir, "J2K codestream directory")
      ->required()
      ->check(CLI::ExistingDirectory);
  frameqc_cmd->add_option("--target-bitrate", fqc_target_br, "Target bitrate Mbps")
      ->default_val(250.0);
  frameqc_cmd->add_option("--max-bitrate", fqc_max_br, "Max bitrate Mbps")->default_val(300.0);
  frameqc_cmd->add_option("--min-bitrate", fqc_min_br, "Min bitrate Mbps")->default_val(50.0);

  // === QC-REPORT subcommand ===
  auto* qcreport_cmd = app.add_subcommand("qc-report", "Generate detailed QC report (HTML/PDF)");
  std::string qcr_imp_dir, qcr_output, qcr_title, qcr_client;
  bool qcr_thumbnails = true, qcr_waveform = true, qcr_loudness = true, qcr_bitrate = true;
  uint32_t qcr_thumb_count = 12;
  qcreport_cmd->add_option("imp_dir", qcr_imp_dir, "IMP directory")
      ->required()
      ->check(CLI::ExistingDirectory);
  qcreport_cmd->add_option("-o,--output", qcr_output, "Output file (.html or .pdf)")->required();
  qcreport_cmd->add_option("--title", qcr_title, "Report title");
  qcreport_cmd->add_option("--client", qcr_client, "Client name");
  qcreport_cmd->add_option("--thumbnails", qcr_thumb_count, "Number of thumbnails")
      ->default_val(12);
  qcreport_cmd->add_flag(
      "--no-thumbnails", [&](auto) { qcr_thumbnails = false; }, "Skip thumbnails");
  qcreport_cmd->add_flag("--no-waveform", [&](auto) { qcr_waveform = false; }, "Skip waveform");
  qcreport_cmd->add_flag(
      "--no-loudness", [&](auto) { qcr_loudness = false; }, "Skip loudness chart");
  qcreport_cmd->add_flag("--no-bitrate", [&](auto) { qcr_bitrate = false; }, "Skip bitrate chart");

  // === LOUDNESS subcommand ===
  auto* loudness_cmd = app.add_subcommand("loudness", "EBU R128 / ATSC loudness measurement");
  std::string loud_audio, loud_output;
  double loud_target = -23.0;
  bool loud_normalize = false;
  loudness_cmd->add_option("audio", loud_audio, "Audio file (WAV, MXF)")
      ->required()
      ->check(CLI::ExistingFile);
  loudness_cmd->add_option("-o,--output", loud_output, "Normalized output file");
  loudness_cmd->add_option("--target", loud_target, "Target loudness LUFS")->default_val(-23.0);
  loudness_cmd->add_flag("--normalize", loud_normalize, "Normalize audio to target");

  // === AV-SYNC subcommand ===
  auto* avsync_cmd = app.add_subcommand("av-sync", "Audio/video sync drift detection");
  std::string avs_video, avs_audio;
  uint32_t avs_fps_num = 24, avs_fps_den = 1, avs_sample_rate = 48000;
  avsync_cmd->add_option("-v,--video", avs_video, "Video file")
      ->required()
      ->check(CLI::ExistingFile);
  avsync_cmd->add_option("-a,--audio", avs_audio, "Audio file")
      ->required()
      ->check(CLI::ExistingFile);
  avsync_cmd->add_option("--fps-num", avs_fps_num, "FPS numerator")->default_val(24);
  avsync_cmd->add_option("--fps-den", avs_fps_den, "FPS denominator")->default_val(1);
  avsync_cmd->add_option("--sample-rate", avs_sample_rate, "Audio sample rate")->default_val(48000);

  // === HDR-VALIDATE subcommand ===
  auto* hdrval_cmd = app.add_subcommand("hdr-validate", "HDR metadata validation");
  std::string hv_video, hv_spec;
  uint16_t hv_max_cll = 0, hv_max_fall = 0;
  uint32_t hv_bit_depth = 0;
  hdrval_cmd->add_option("video", hv_video, "Video file")->required()->check(CLI::ExistingFile);
  hdrval_cmd->add_option("-s,--spec", hv_spec, "HDR spec")
      ->required()
      ->check(CLI::IsMember({"hdr10", "hlg", "dolby_vision", "hdr10plus"}));
  hdrval_cmd->add_option("--max-cll", hv_max_cll, "Expected MaxCLL");
  hdrval_cmd->add_option("--max-fall", hv_max_fall, "Expected MaxFALL");
  hdrval_cmd->add_option("--bit-depth", hv_bit_depth, "Expected bit depth");

  // === FRAME-COMPARE subcommand ===
  auto* fcomp_cmd = app.add_subcommand("frame-compare", "Frame-by-frame PSNR/SSIM/VMAF comparison");
  std::string fc_imp_a, fc_imp_b, fc_file_a, fc_file_b, fc_output;
  double fc_threshold = 40.0;
  uint32_t fc_start = 0, fc_end = 0;
  bool fc_html = false, fc_extract_diffs = false, fc_ssim = true, fc_vmaf = false;
  fcomp_cmd->add_option("--imp-a", fc_imp_a, "First IMP directory");
  fcomp_cmd->add_option("--imp-b", fc_imp_b, "Second IMP directory");
  fcomp_cmd->add_option("--file-a", fc_file_a, "First file (direct comparison)");
  fcomp_cmd->add_option("--file-b", fc_file_b, "Second file (direct comparison)");
  fcomp_cmd->add_option("-t,--threshold", fc_threshold, "PSNR threshold (dB)")->default_val(40.0);
  fcomp_cmd->add_option("-o,--output", fc_output, "Output directory for reports/diffs");
  fcomp_cmd->add_option("--start-frame", fc_start, "Start frame");
  fcomp_cmd->add_option("--end-frame", fc_end, "End frame");
  fcomp_cmd->add_flag("--html", fc_html, "Generate HTML report");
  fcomp_cmd->add_flag("--extract-diffs", fc_extract_diffs, "Extract diff images");
  fcomp_cmd->add_flag("--vmaf", fc_vmaf, "Compute VMAF scores");
  fcomp_cmd->add_flag("--no-ssim", [&](auto) { fc_ssim = false; }, "Skip SSIM computation");

  // === IMP-INFO subcommand ===
  auto* impinfo_cmd = app.add_subcommand("imp-info", "Display IMP package information");
  std::string impinfo_dir;
  impinfo_cmd->add_option("imp_dir", impinfo_dir, "IMP directory")
      ->required()
      ->check(CLI::ExistingDirectory);

  // === FIX subcommand ===
  auto* fix_cmd =
      app.add_subcommand("fix", "Auto-fix common DCP issues (hashes, namespaces, ContentKind)");
  std::string fix_dir;
  bool fix_dry_run = false;
  fix_cmd->add_option("dcp_dir", fix_dir, "DCP directory to fix")
      ->required()
      ->check(CLI::ExistingDirectory);
  fix_cmd->add_flag("--dry-run", fix_dry_run, "Preview fixes without applying them");

  // Additional validate flags
  bool suggest_fixes_flag = false;
  bool imf_mode = false;
  bool netflix_mode = false;
  bool accessibility_check = false;
  bool hdr_check = false;
  bool atmos_deep = false;
  bool studio_mode = false;
  bool deep_mode = false;
  std::string photon_dir_opt;
  validate_cmd->add_flag("--fix", suggest_fixes_flag, "Show fix suggestions for detected issues");
  validate_cmd->add_flag("--imf", imf_mode, "Validate as IMF package (uses Netflix Photon)");
  validate_cmd->add_flag("--netflix", netflix_mode, "Check Netflix IMF delivery specs");
  validate_cmd->add_flag("--accessibility", accessibility_check,
                         "Check accessibility tracks (AD/HI/CC)");
  validate_cmd->add_flag("--hdr", hdr_check, "Detect and validate HDR metadata");
  validate_cmd->add_flag("--atmos", atmos_deep, "Deep Dolby Atmos IAB inspection");
  validate_cmd->add_flag("--studio", studio_mode,
                         "Run studio-level checks (loudness, color, resolution, encryption)");
  validate_cmd->add_flag("--deep", deep_mode,
                         "Deep per-MXF analysis (color, loudness, resolution)");
  validate_cmd->add_option("--photon-dir", photon_dir_opt, "Path to Photon source directory");
  app.add_flag("--fix", suggest_fixes_flag, "Show fix suggestions for detected issues");
  app.add_flag("--imf", imf_mode, "Validate as IMF package (uses Netflix Photon)");
  app.add_flag("--netflix", netflix_mode, "Check Netflix IMF delivery specs");
  app.add_flag("--accessibility", accessibility_check, "Check accessibility tracks (AD/HI/CC)");
  app.add_flag("--hdr", hdr_check, "Detect and validate HDR metadata");
  app.add_flag("--atmos", atmos_deep, "Deep Dolby Atmos IAB inspection");
  app.add_flag("--studio", studio_mode,
               "Run studio-level checks (loudness, color, resolution, encryption)");
  app.add_flag("--deep", deep_mode, "Deep per-MXF analysis (color, loudness, resolution)");
  app.add_option("--photon-dir", photon_dir_opt, "Path to Photon source directory");

  // Also allow validate args on the main app for backward compat
  app.add_flag("-v,--verbose", verbose, "Show info-level notes");
  app.add_flag("-q,--quiet", quiet, "Only show errors");
  app.add_flag("--json", json, "JSON output");
  app.add_flag("--html", html, "HTML report output");
  app.add_flag("--no-hashes", no_hashes, "Skip hash verification");
  app.add_flag("--no-signatures", no_signatures, "Skip signature verification");
  app.add_flag("--check-mxf", check_mxf, "Inspect MXF essence metadata");
  app.add_flag("--strict", strict, "Strict SMPTE compliance");
  app.add_flag("--bv21", bv21, "Check BV2.1 application profile");
  app.add_flag("--deep-j2k", deep_j2k, "Deep J2K codestream validation");
  app.add_option("-o,--output", output_file, "Write report to file");
  app.add_option("--manifest", manifest_file, "Compare against manifest JSON");
  app.add_option("--timeline", timeline_file, "Write SVG timeline to file");
  app.add_option("dcp_dirs", dcp_dirs, "DCP directories to validate")
      ->check(CLI::ExistingDirectory);

  CLI11_PARSE(app, argc, argv);

  // Configure logging
  if(verbose)
    spdlog::set_level(spdlog::level::debug);
  else if(quiet)
    spdlog::set_level(spdlog::level::err);
  else
    spdlog::set_level(spdlog::level::warn);

  // === WATCH mode ===
  if(watch_cmd->parsed())
  {
    dcpdoctor::VerifyOptions opts;
    opts.check_hashes = true;
    opts.check_signatures = true;
    opts.check_picture_details = true;

    dcpdoctor::watch_directory(
        fs::path(watch_dir), opts,
        [](const fs::path& path, const dcpdoctor::VerifyResult& result) {
          std::string status = result.ok() ? "PASS" : "FAIL";
          spdlog::info("{}: {} ({} errors, {} warnings)", path.string(), status, result.error_count,
                       result.warning_count);
          std::ostringstream oss;
          dcpdoctor::write_report(result, path, oss, dcpdoctor::ReportFormat::text);
          spdlog::info("{}", oss.str());
        },
        poll_interval);
    return 0;
  }

  // === SERVE mode ===
  if(serve_cmd->parsed())
  {
    dcpdoctor::VerifyOptions opts;
    opts.check_hashes = true;
    opts.check_signatures = true;
    opts.check_picture_details = true;
    dcpdoctor::serve_api(bind_addr, port, opts);
    return 0;
  }

  // === DIFF mode ===
  if(diff_cmd->parsed())
  {
    auto diff = dcpdoctor::compare_dcps(fs::path(diff_a), fs::path(diff_b), diff_hashes);
    std::ostringstream oss;
    dcpdoctor::write_diff_report(oss, diff);
    spdlog::info("{}", oss.str());
    return diff.content_identical ? 0 : 1;
  }

  // === PROFILES mode ===
  if(profiles_cmd->parsed())
  {
    if(profile_name.empty())
    {
      // List all profiles
      auto profiles = dcpdoctor::get_theater_profiles();
      spdlog::info("Theater Compatibility Profiles:\n");
      for(const auto& p : profiles)
      {
        spdlog::info("  {} ({})", p.name, p.vendor);
        spdlog::info("    4K: {}  HFR: {}  Atmos: {}  Interop: {}", p.supports_4k ? "Yes" : "No",
                     p.supports_hfr ? "Yes" : "No", p.supports_atmos ? "Yes" : "No",
                     p.supports_interop ? "Yes" : "No");
      }
    }
    else if(!profile_dcp.empty())
    {
      auto* profile = dcpdoctor::find_profile(profile_name);
      if(!profile)
      {
        spdlog::error("Unknown profile: {}", profile_name);
        return 2;
      }
      auto result = dcpdoctor::verify(fs::path(profile_dcp));
      auto notes = dcpdoctor::check_theater_compatibility(fs::path(profile_dcp), result, *profile);
      spdlog::info("Theater compatibility: {}\n", profile->name);
      for(const auto& n : notes)
        spdlog::info("[{}] {}", n.severity_str(), n.message);
      if(notes.empty())
        spdlog::info("No compatibility issues detected.");
    }
    else
    {
      spdlog::error("Use --check PROFILE --dcp DIR to check compatibility");
      return 2;
    }
    return 0;
  }

  // === KDM mode ===
  if(kdm_cmd->parsed())
  {
    auto info = dcpdoctor::parse_kdm(fs::path(kdm_file));
    if(!info.valid)
    {
      spdlog::error("Invalid KDM: {}", info.error);
      return 1;
    }
    spdlog::info("KDM Information:");
    spdlog::info("  Content: {}", info.content_title);
    spdlog::info("  CPL ID:  {}", info.cpl_id);
    spdlog::info("  Status:  {}", info.is_expired         ? "EXPIRED"
                                  : info.is_not_yet_valid ? "NOT YET VALID"
                                                          : "VALID");

    if(!kdm_dcp.empty())
    {
      auto notes = dcpdoctor::validate_kdm(fs::path(kdm_file), fs::path(kdm_dcp));
      for(const auto& n : notes)
        spdlog::info("[{}] {}", n.severity_str(), n.message);
      if(notes.empty())
        spdlog::info("\nKDM validates against DCP.");
    }
    return 0;
  }

  // === CHECKSUM-VERIFY mode ===
  if(checksum_cmd->parsed())
  {
    dcpdoctor::ChecksumVerifyOptions cv_opts;
    cv_opts.package_dir = checksum_dir;
    cv_opts.verify_hashes = !checksum_no_hash;
    cv_opts.verify_sizes = !checksum_no_size;
    cv_opts.stop_on_first_error = checksum_stop;

    auto result = dcpdoctor::verify_package_checksums(cv_opts);
    if(!result.success)
    {
      spdlog::error("Error: {}", result.error);
      return 1;
    }

    if(checksum_json)
    {
      spdlog::info("{{");
      spdlog::info("  \"total\": {},", result.total_assets);
      spdlog::info("  \"verified_ok\": {},", result.verified_ok);
      spdlog::info("  \"hash_mismatches\": {},", result.hash_mismatches);
      spdlog::info("  \"size_mismatches\": {},", result.size_mismatches);
      spdlog::info("  \"missing_files\": {},", result.missing_files);
      spdlog::info("  \"all_valid\": {}", result.all_valid ? "true" : "false");
      spdlog::info("}}");
    }
    else
    {
      spdlog::info("Checksum Verification: {}", checksum_dir);
      spdlog::info("  Total assets: {}", result.total_assets);
      spdlog::info("  Verified OK:  {}", result.verified_ok);
      if(result.hash_mismatches > 0)
        spdlog::info("  Hash mismatches: {}", result.hash_mismatches);
      if(result.size_mismatches > 0)
        spdlog::info("  Size mismatches: {}", result.size_mismatches);
      if(result.missing_files > 0)
        spdlog::info("  Missing files: {}", result.missing_files);
      spdlog::info("  Result: {}", result.all_valid ? "PASS" : "FAIL");

      // Show individual failures
      for(const auto& e : result.entries)
      {
        if(!e.file_exists)
          spdlog::info("  MISSING: {}", e.filename);
        else if(!e.hash_match)
          spdlog::info("  HASH MISMATCH: {}", e.filename);
        else if(!e.size_match)
          spdlog::info("  SIZE MISMATCH: {} (expected {}, got {})", e.filename, e.expected_size,
                       e.actual_size);
      }
    }
    return result.all_valid ? 0 : 1;
  }

  // === MXF-EXTRACT mode ===
  if(mxf_extract_cmd->parsed())
  {
    dcpdoctor::MxfExtractOptions ex_opts;
    ex_opts.input = mxf_input;
    ex_opts.output_dir = mxf_output_dir;
    ex_opts.extract_video = !mxf_no_video;
    ex_opts.extract_audio = !mxf_no_audio;
    ex_opts.start_frame = mxf_start;
    ex_opts.end_frame = mxf_end;

    auto result = dcpdoctor::extract_mxf(ex_opts);
    if(!result.success)
    {
      spdlog::error("Error: {}", result.error);
      return 1;
    }
    spdlog::info("Extracted {} file(s):", result.extracted_files.size());
    for(const auto& f : result.extracted_files)
      spdlog::info("  {}", f.string());
    if(result.frames_extracted > 0)
      spdlog::info("Frames: {}", result.frames_extracted);
    return 0;
  }

  // === AUTO-QC mode ===
  if(auto_qc_cmd->parsed())
  {
    if(qc_video.empty() && qc_audio.empty())
    {
      spdlog::error("Error: specify --video and/or --audio");
      return 2;
    }

    dcpdoctor::AutoQcOptions qc_opts;
    qc_opts.video_path = qc_video;
    qc_opts.audio_path = qc_audio;
    qc_opts.black_threshold = qc_black_thresh;
    qc_opts.freeze_threshold = qc_freeze_thresh;
    qc_opts.silence_threshold = qc_silence_thresh;
    qc_opts.clipping_threshold = qc_clip_thresh;

    auto result = dcpdoctor::run_auto_qc(qc_opts);
    if(!result.success)
    {
      spdlog::error("Error: {}", result.error);
      return 1;
    }

    if(qc_json_flag)
    {
      spdlog::info("{}", dcpdoctor::auto_qc_to_json(result));
    }
    else
    {
      spdlog::info("Auto QC Results:");
      spdlog::info("  Issues found: {}\n", result.issues.size());
      for(const auto& issue : result.issues)
      {
        spdlog::info("  [{}] {}", issue.severity, issue.description);
      }
      if(result.issues.empty())
        spdlog::info("  No issues detected.");
    }
    return result.issues.empty() ? 0 : 1;
  }

  // === VALIDATE-IMP mode ===
  if(validate_imp_cmd->parsed())
  {
    auto vr = dcpdoctor::validate_with_photon(vimp_dir);
    spdlog::info("IMP Validation: {}", vr.valid ? "PASS" : "FAIL");
    if(!vr.notes.empty())
    {
      for(const auto& n : vr.notes)
      {
        const char* sev = "INFO";
        if(n.severity == dcpdoctor::ValidationNote::Severity::error)
          sev = "ERROR";
        else if(n.severity == dcpdoctor::ValidationNote::Severity::warning)
          sev = "WARN";
        spdlog::info("  [{}] {}", sev, n.message);
      }
    }
    return vr.valid ? 0 : 1;
  }

  // === SCHEMA-VALIDATE mode ===
  if(schema_val_cmd->parsed())
  {
    dcpdoctor::SchemaValidateOptions sv_opts;
    sv_opts.imp_dir = sv_imp_dir;
    if(!sv_schema_dir.empty())
      sv_opts.schema_dir = sv_schema_dir;
    sv_opts.validate_cpl = sv_cpl;
    sv_opts.validate_pkl = sv_pkl;
    sv_opts.validate_assetmap = sv_assetmap;

    auto sr = dcpdoctor::validate_against_schema(sv_opts);
    spdlog::info("Schema Validation: {}", sr.valid ? "PASS" : "FAIL");
    if(!sr.schema_version.empty())
      spdlog::info("  Schema: {}", sr.schema_version);
    for(const auto& e : sr.errors)
    {
      spdlog::info("  {} {}:{}:{} {}", e.is_warning ? "WARN" : "ERROR", e.file, e.line, e.column,
                   e.message);
    }
    return sr.valid ? 0 : 1;
  }

  // === IMF-COMPLIANCE mode ===
  if(imfcomp_cmd->parsed())
  {
    dcpdoctor::ImfComplianceOptions ic_opts;
    ic_opts.imp_dir = imfcomp_dir;
    ic_opts.strict = imfcomp_strict;
    if(imfcomp_target == "netflix")
      ic_opts.target = dcpdoctor::ImfComplianceTarget::Netflix;
    else if(imfcomp_target == "disney")
      ic_opts.target = dcpdoctor::ImfComplianceTarget::Disney;
    else if(imfcomp_target == "amazon")
      ic_opts.target = dcpdoctor::ImfComplianceTarget::Amazon;
    else if(imfcomp_target == "apple")
      ic_opts.target = dcpdoctor::ImfComplianceTarget::Apple;
    else if(imfcomp_target == "cinema2k")
      ic_opts.target = dcpdoctor::ImfComplianceTarget::Cinema2K;
    else if(imfcomp_target == "cinema4k")
      ic_opts.target = dcpdoctor::ImfComplianceTarget::Cinema4K;
    else if(imfcomp_target == "broadcast-hd")
      ic_opts.target = dcpdoctor::ImfComplianceTarget::BroadcastHD;
    else
      ic_opts.target = dcpdoctor::ImfComplianceTarget::BroadcastUHD;

    auto cr = dcpdoctor::check_imf_compliance(ic_opts);
    if(!cr.success)
    {
      spdlog::error("Error: {}", cr.error);
      return 1;
    }
    spdlog::info("{} Compliance: {}", dcpdoctor::imf_compliance_target_name(cr.target),
                 cr.compliant ? "PASS" : "FAIL");
    for(const auto& c : cr.checks)
    {
      if(!c.actual_value.empty())
        spdlog::info("  {} {}: {} (got {}, expected {})", c.passed ? "PASS" : "FAIL", c.rule,
                     c.description, c.actual_value, c.expected_value);
      else
        spdlog::info("  {} {}: {}", c.passed ? "PASS" : "FAIL", c.rule, c.description);
    }
    return cr.compliant ? 0 : 1;
  }

  // === FRAME-QC mode ===
  if(frameqc_cmd->parsed())
  {
    dcpdoctor::FrameQcOptions fqc_opts;
    fqc_opts.j2k_dir = fqc_dir;
    fqc_opts.target_bitrate_mbps = fqc_target_br;
    fqc_opts.max_bitrate_mbps = fqc_max_br;
    fqc_opts.min_bitrate_mbps = fqc_min_br;

    auto fr = dcpdoctor::analyze_frame_qc(fqc_opts);
    if(!fr.success)
    {
      spdlog::error("Error: {}", fr.error);
      return 1;
    }
    spdlog::info("Frame QC Analysis:");
    spdlog::info("  Total frames: {}", fr.total_frames);
    spdlog::info("  Average bitrate: {} Mbps", fr.average_bitrate_mbps);
    spdlog::info("  Peak bitrate: {} Mbps", fr.peak_bitrate_mbps);
    spdlog::info("  Over budget: {}", fr.over_budget_count);
    spdlog::info("  Under budget: {}", fr.under_budget_count);
    return (fr.over_budget_count == 0 && fr.under_budget_count == 0) ? 0 : 1;
  }

  // === QC-REPORT mode ===
  if(qcreport_cmd->parsed())
  {
    dcpdoctor::DetailedQcOptions qcr_opts;
    qcr_opts.imp_dir = qcr_imp_dir;
    qcr_opts.output_file = qcr_output;
    qcr_opts.title = qcr_title;
    qcr_opts.client = qcr_client;
    qcr_opts.include_thumbnails = qcr_thumbnails;
    qcr_opts.include_waveform = qcr_waveform;
    qcr_opts.include_loudness = qcr_loudness;
    qcr_opts.include_bitrate_chart = qcr_bitrate;
    qcr_opts.thumbnail_count = qcr_thumb_count;

    auto qr = dcpdoctor::generate_detailed_qc(qcr_opts);
    if(!qr.success)
    {
      spdlog::error("Error: {}", qr.error);
      return 1;
    }
    spdlog::info("QC report written to: {} ({} pages)", qr.output_file.string(), qr.pages);
    return 0;
  }

  // === LOUDNESS mode ===
  if(loudness_cmd->parsed())
  {
    if(loud_normalize && !loud_output.empty())
    {
      dcpdoctor::NormalizeOptions norm_opts;
      norm_opts.input_file = loud_audio;
      norm_opts.output_file = loud_output;
      norm_opts.target_lufs = loud_target;
      auto nr = dcpdoctor::normalize_loudness(norm_opts);
      if(!nr.success)
      {
        spdlog::error("Error: {}", nr.error);
        return 1;
      }
      spdlog::info("Normalized to {} LUFS", loud_target);
      spdlog::info("Output: {}", nr.output_file.string());
      return 0;
    }
    else
    {
      auto lr = dcpdoctor::measure_imf_loudness(loud_audio);
      if(!lr.success)
      {
        spdlog::error("Error: {}", lr.error);
        return 1;
      }
      spdlog::info("Loudness Measurement:");
      spdlog::info("  Integrated: {} LUFS", lr.integrated_lufs);
      spdlog::info("  Range: {} LU", lr.loudness_range_lu);
      spdlog::info("  True peak: {} dBTP", lr.true_peak_dbtp);
      spdlog::info("  EBU R128: {}", lr.compliant_r128 ? "PASS" : "FAIL");
      spdlog::info("  ATSC A/85: {}", lr.compliant_atsc ? "PASS" : "FAIL");
      return (lr.compliant_r128 || lr.compliant_atsc) ? 0 : 1;
    }
  }

  // === AV-SYNC mode ===
  if(avsync_cmd->parsed())
  {
    dcpdoctor::AvSyncOptions avs_opts;
    avs_opts.video_file = avs_video;
    avs_opts.audio_file = avs_audio;
    avs_opts.fps_num = avs_fps_num;
    avs_opts.fps_den = avs_fps_den;
    avs_opts.sample_rate = avs_sample_rate;

    auto ar = dcpdoctor::detect_av_sync(avs_opts);
    if(!ar.success)
    {
      spdlog::error("Error: {}", ar.error);
      return 1;
    }
    spdlog::info("AV Sync: {}", ar.in_sync ? "IN SYNC" : "OUT OF SYNC");
    spdlog::info("  Drift: {} ms ({} frames)", ar.drift_ms, ar.drift_frames);
    if(!ar.recommendation.empty())
      spdlog::info("  Recommendation: {}", ar.recommendation);
    return ar.in_sync ? 0 : 1;
  }

  // === HDR-VALIDATE mode ===
  if(hdrval_cmd->parsed())
  {
    dcpdoctor::HdrValidateOptions hv_opts;
    hv_opts.video_path = hv_video;
    hv_opts.target_spec = hv_spec;
    hv_opts.expected_max_cll = hv_max_cll;
    hv_opts.expected_max_fall = hv_max_fall;
    hv_opts.expected_bit_depth = hv_bit_depth;

    if(hv_spec == "hdr10")
    {
      hv_opts.expected_transfer = dcpdoctor::TransferFunction::PQ;
      hv_opts.expected_colorimetry = dcpdoctor::Colorimetry::BT2020;
    }
    else if(hv_spec == "hlg")
    {
      hv_opts.expected_transfer = dcpdoctor::TransferFunction::HLG;
      hv_opts.expected_colorimetry = dcpdoctor::Colorimetry::BT2020;
    }

    auto hr = dcpdoctor::validate_hdr_metadata(hv_opts);
    if(!hr.success)
    {
      spdlog::error("Error: {}", hr.error);
      return 1;
    }
    spdlog::info("HDR Validation: {}", hr.valid ? "PASS" : "FAIL");
    for(const auto& issue : hr.issues)
    {
      spdlog::info("  [{}] {}: expected {}, got {}", issue.severity, issue.field, issue.expected,
                   issue.actual);
    }
    return hr.valid ? 0 : 1;
  }

  // === FRAME-COMPARE mode ===
  if(fcomp_cmd->parsed())
  {
    if(!fc_file_a.empty() && !fc_file_b.empty())
    {
      dcpdoctor::CompareOptions cfo;
      cfo.threshold_psnr = fc_threshold;
      cfo.start_frame = fc_start;
      cfo.end_frame = fc_end;
      cfo.generate_html = fc_html;
      cfo.extract_diff_frames = fc_extract_diffs;
      cfo.compute_ssim = fc_ssim;
      cfo.compute_vmaf = fc_vmaf;
      if(!fc_output.empty())
        cfo.output_dir = fc_output;

      auto cr = dcpdoctor::compare_files(fc_file_a, fc_file_b, cfo);
      if(!cr.success)
      {
        spdlog::error("Error: {}", cr.error);
        return 1;
      }
      spdlog::info("Comparison: {} frames, {} significant differences", cr.frames_compared,
                   cr.frames_different);
      spdlog::info("  Avg PSNR: {} dB", cr.avg_psnr);
      return cr.frames_different == 0 ? 0 : 1;
    }
    else
    {
      dcpdoctor::CompareOptions co;
      co.imp_a = fc_imp_a;
      co.imp_b = fc_imp_b;
      co.threshold_psnr = fc_threshold;
      co.start_frame = fc_start;
      co.end_frame = fc_end;
      co.generate_html = fc_html;
      co.extract_diff_frames = fc_extract_diffs;
      co.compute_ssim = fc_ssim;
      co.compute_vmaf = fc_vmaf;
      if(!fc_output.empty())
        co.output_dir = fc_output;

      auto cr = dcpdoctor::compare_imps(co);
      if(!cr.success)
      {
        spdlog::error("Error: {}", cr.error);
        return 1;
      }
      spdlog::info("Comparison: {} frames, {} significant differences", cr.frames_compared,
                   cr.frames_different);
      spdlog::info("  Avg PSNR: {} dB", cr.avg_psnr);
      return cr.frames_different == 0 ? 0 : 1;
    }
  }

  // === IMP-INFO mode ===
  if(impinfo_cmd->parsed())
  {
    auto info = dcpdoctor::read_imp_info(impinfo_dir);
    if(!info.valid)
    {
      spdlog::error("Error: {}", info.error);
      return 1;
    }
    spdlog::info("IMP: {}", info.path.string());
    spdlog::info("  CPL: {} ({})", info.cpl_uuid, info.cpl_title);
    spdlog::info("  PKL: {}", info.pkl_uuid);
    spdlog::info("  Issuer: {}", info.issuer);
    spdlog::info("  Date: {}", info.issue_date);
    spdlog::info("  Tracks: {}", info.tracks.size());
    for(const auto& t : info.tracks)
    {
      spdlog::info("    {}: {} ({} bytes)", t.type, t.filename, t.size);
    }
    return 0;
  }

  // === FIX mode ===
  if(fix_cmd->parsed())
  {
    // First validate to find issues
    dcpdoctor::VerifyOptions fix_opts;
    fix_opts.check_hashes = true;
    fix_opts.check_signatures = false;
    fix_opts.strict_smpte = true;
    auto vresult = dcpdoctor::verify(fix_dir, fix_opts);

    auto suggestions = dcpdoctor::suggest_fixes(vresult.notes);
    // Also check for ContentKind issues
    bool has_content_kind_issue = false;
    for(const auto& note : vresult.notes)
    {
      if(note.message.find("ContentKind") != std::string::npos)
        has_content_kind_issue = true;
    }

    int fixable_count = 0;
    for(const auto& s : suggestions)
    {
      if(s.auto_fixable)
        ++fixable_count;
    }
    if(has_content_kind_issue)
      ++fixable_count;

    if(fixable_count == 0)
    {
      spdlog::info("No auto-fixable issues found.");
      return 0;
    }

    spdlog::info("Found {} auto-fixable issue(s):", fixable_count);
    for(const auto& s : suggestions)
    {
      if(s.auto_fixable)
        spdlog::info("  - {}", s.description);
    }
    if(has_content_kind_issue)
      spdlog::info("  - Normalize ContentKind to canonical lowercase");

    if(fix_dry_run)
    {
      spdlog::info("Dry run: no changes applied.");
      return 0;
    }

    int applied = dcpdoctor::apply_fixes(fix_dir, suggestions);
    if(has_content_kind_issue)
      applied += dcpdoctor::fix_content_kind(fix_dir);

    spdlog::info("Applied {} fix(es).", applied);
    return applied > 0 ? 0 : 1;
  }

  // === VALIDATE mode (default) ===
  if(dcp_dirs.empty())
  {
    spdlog::error("{}", app.help());
    return 2;
  }

  dcpdoctor::VerifyOptions opts;
  opts.check_hashes = !no_hashes;
  opts.check_signatures = !no_signatures;
  opts.check_picture_details = check_mxf || strict || deep_j2k;
  opts.strict_smpte = strict;

  // Show per-file hash progress on TTY
#ifdef _WIN32
  bool is_tty = ::_isatty(_fileno(stderr));
#else
  bool is_tty = ::isatty(STDERR_FILENO);
#endif
  if(opts.check_hashes && is_tty && !json)
  {
    opts.hash_progress = [](const std::filesystem::path& file, int file_idx, int total_files,
                            std::uintmax_t bytes_done, std::uintmax_t bytes_total) {
      int pct = bytes_total > 0 ? static_cast<int>((bytes_done * 100) / bytes_total) : 0;
      std::fprintf(stderr, "\r\033[K  Hashing [%d/%d] %s (%d%%)", file_idx + 1, total_files,
                   file.filename().c_str(), pct);
    };
  }

  dcpdoctor::ReportFormat format = dcpdoctor::ReportFormat::text;
  if(json)
    format = dcpdoctor::ReportFormat::json;
  else if(html)
    format = dcpdoctor::ReportFormat::html;

  spdlog::debug("Validating {} DCP(s)", dcp_dirs.size());

  bool all_passed = true;
  std::vector<dcpdoctor::BatchResult> batch_results;
  dcpdoctor::ProgressBar progress(dcp_dirs.size(), dcp_dirs.size() > 1 ? "Validating" : "");

  for(size_t idx = 0; idx < dcp_dirs.size(); ++idx)
  {
    fs::path dir(dcp_dirs[idx]);
    if(dcp_dirs.size() > 1)
      progress.update(idx);
    spdlog::debug("Processing: {}", dir.string());

    auto result = dcpdoctor::verify(dir, opts);
    if(opts.hash_progress)
      std::fprintf(stderr, "\r\033[K");

    // BV2.1 compliance
    if(bv21)
    {
      auto bv21_notes = dcpdoctor::check_bv21_compliance(dir, result.standard);
      for(auto& note : bv21_notes)
        result.add(std::move(note));
    }

    // Manifest comparison
    if(!manifest_file.empty())
    {
      auto manifest_notes = dcpdoctor::compare_manifest(dir, fs::path(manifest_file));
      for(auto& note : manifest_notes)
        result.add(std::move(note));
    }

    // IMF validation via Netflix Photon
    if(imf_mode)
    {
      auto photon_config = dcpdoctor::default_photon_config();
      if(!photon_dir_opt.empty())
        photon_config.photon_dir = photon_dir_opt;
      auto photon_result = dcpdoctor::run_photon(dir, photon_config);
      auto photon_notes = dcpdoctor::photon_to_notes(photon_result, dir);
      for(auto& note : photon_notes)
        result.add(std::move(note));
    }

    // Netflix delivery spec
    if(netflix_mode)
    {
      auto netflix_result = dcpdoctor::check_netflix_delivery(dir);
      auto netflix_notes = dcpdoctor::netflix_to_notes(netflix_result, dir);
      for(auto& note : netflix_notes)
        result.add(std::move(note));
    }

    // Accessibility tracks
    if(accessibility_check)
    {
      auto acc_notes = dcpdoctor::check_accessibility(dir);
      for(auto& note : acc_notes)
        result.add(std::move(note));
    }

    // HDR metadata detection
    if(hdr_check)
    {
      std::error_code ec2;
      for(auto& mxf_entry : fs::directory_iterator(dir, ec2))
      {
        if(!mxf_entry.is_regular_file())
          continue;
        if(mxf_entry.path().extension() != ".mxf")
          continue;
        auto hdr_info = dcpdoctor::detect_hdr_metadata(mxf_entry.path());
        if(hdr_info.detected)
        {
          auto hdr_notes = dcpdoctor::check_hdr_compliance(hdr_info, mxf_entry.path());
          for(auto& note : hdr_notes)
            result.add(std::move(note));
          break; // Only check first picture MXF
        }
      }
    }

    // Deep Atmos IAB inspection
    if(atmos_deep)
    {
      std::error_code ec2;
      for(auto& mxf_entry : fs::directory_iterator(dir, ec2))
      {
        if(!mxf_entry.is_regular_file())
          continue;
        if(mxf_entry.path().extension() != ".mxf")
          continue;
        auto atmos_info = dcpdoctor::parse_atmos_iab(mxf_entry.path());
        if(atmos_info.detected)
        {
          auto atmos_notes = dcpdoctor::check_atmos_compliance(atmos_info, mxf_entry.path());
          for(auto& note : atmos_notes)
            result.add(std::move(note));
        }
      }
    }

    // Studio-level checks (loudness, color, resolution, encryption, etc.)
    if(studio_mode)
    {
      auto studio_notes = dcpdoctor::run_studio_checks(dir, deep_mode);
      for(auto& note : studio_notes)
        result.add(std::move(note));
    }

    if(!result.ok())
      all_passed = false;

    // Batch tracking
    batch_results.push_back(
        {dir, result.ok(), result.error_count, result.warning_count, result.standard});

    spdlog::debug("Result: {} errors, {} warnings", result.error_count, result.warning_count);

    if(!quiet)
    {
      if(!output_file.empty())
      {
        std::ofstream out(output_file);
        dcpdoctor::write_report(result, dir, out, format);
      }
      else
      {
        std::ostringstream oss;
        dcpdoctor::write_report(result, dir, oss, format);
        spdlog::info("{}", oss.str());
      }

      // Fix suggestions
      if(suggest_fixes_flag && !result.notes.empty())
      {
        auto fixes = dcpdoctor::suggest_fixes(result.notes);
        if(!fixes.empty())
        {
          spdlog::info("\nSuggested Fixes:");
          for(size_t fi = 0; fi < fixes.size(); ++fi)
          {
            if(!fixes[fi].command.empty())
              spdlog::info("  {}. {}\n     Command: {}", fi + 1, fixes[fi].description,
                           fixes[fi].command);
            else
              spdlog::info("  {}. {}", fi + 1, fixes[fi].description);
          }
        }
      }
    }

    // Timeline SVG generation
    if(!timeline_file.empty())
    {
      // Find CPL in DCP
      std::error_code ec;
      for(auto& entry : fs::directory_iterator(dir, ec))
      {
        if(!entry.is_regular_file())
          continue;
        if(entry.path().extension() != ".xml")
          continue;
        auto reels = dcpdoctor::extract_timeline(entry.path());
        if(!reels.empty())
        {
          std::ofstream svg_out(timeline_file);
          dcpdoctor::write_timeline_svg(svg_out, reels, dir.filename().string(), 24.0);
          spdlog::info("Timeline written to: {}", timeline_file);
          break;
        }
      }
    }
  }

  if(dcp_dirs.size() > 1)
    progress.finish();

  // Batch summary for multiple DCPs
  if(batch_results.size() > 1)
  {
    std::ostringstream oss;
    dcpdoctor::write_batch_summary(oss, batch_results);
    spdlog::info("\n{}", oss.str());
  }

  return all_passed ? 0 : 1;
}
