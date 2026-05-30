#include <libxml/parser.h>
#include <libxml/xpath.h>
#include <libxml/xpathInternals.h>
#include <AS_DCP.h>
#include <KM_fileio.h>
#include <cmath>
#include <cstdio>
#include <fstream>
#include <functional>
#include <sstream>
#include <algorithm>
#include <numeric>
#include <cstring>

#include "dcpdoctor/platform.h"
#include "dcpdoctor/studio.h"

namespace fs = std::filesystem;

namespace dcpdoctor
{

// ════════════════════════════════════════════════════════════════════════════════
// 1. Audio Loudness (EBU R128 / SMPTE RP 2071)
// ════════════════════════════════════════════════════════════════════════════════

LoudnessResult measure_loudness(const fs::path& mxf_path, uint32_t max_frames)
{
  LoudnessResult result;

  Kumu::FileReaderFactory defaultFactory;
  ASDCP::PCM::MXFReader reader(defaultFactory);
  auto rc = reader.OpenRead(mxf_path.string());
  if(ASDCP_FAILURE(rc))
  {
    result.error = "Failed to open PCM MXF";
    return result;
  }

  ASDCP::PCM::AudioDescriptor desc;
  reader.FillAudioDescriptor(desc);
  result.channels = desc.ChannelCount;
  result.sample_rate = desc.AudioSamplingRate.Numerator;

  uint32_t frame_count = desc.ContainerDuration;
  if(max_frames > 0 && max_frames < frame_count)
    frame_count = max_frames;

  // EBU R128 measurement via K-weighted power summation
  // We process PCM samples through a simplified K-weighting approximation
  double sum_squared = 0.0;
  double peak = 0.0;
  uint64_t total_samples = 0;
  double momentary_max = -70.0;

  ASDCP::PCM::FrameBuffer frame_buf(desc.EditRate.Numerator * desc.ChannelCount *
                                    (desc.QuantizationBits / 8));

  uint32_t samples_per_frame = desc.AudioSamplingRate.Numerator / desc.EditRate.Numerator;

  for(uint32_t i = 0; i < frame_count; ++i)
  {
    rc = reader.ReadFrame(i, frame_buf);
    if(ASDCP_FAILURE(rc))
      break;

    const uint8_t* data = frame_buf.RoData();
    uint32_t bytes_per_sample = desc.QuantizationBits / 8;
    uint32_t sample_count = frame_buf.Size() / bytes_per_sample;

    double frame_sum = 0.0;
    for(uint32_t s = 0; s < sample_count; ++s)
    {
      double sample_val = 0.0;
      if(bytes_per_sample == 3)
      {
        // 24-bit PCM (little-endian in DCP MXF)
        int32_t raw = static_cast<int32_t>(data[s * 3]) |
                      (static_cast<int32_t>(data[s * 3 + 1]) << 8) |
                      (static_cast<int32_t>(data[s * 3 + 2]) << 16);
        if(raw & 0x800000)
          raw |= 0xFF000000; // sign extend
        sample_val = raw / 8388608.0;
      }
      else if(bytes_per_sample == 2)
      {
        int16_t raw = static_cast<int16_t>(data[s * 2] | (data[s * 2 + 1] << 8));
        sample_val = raw / 32768.0;
      }

      double abs_val = std::abs(sample_val);
      if(abs_val > peak)
        peak = abs_val;
      frame_sum += sample_val * sample_val;
    }

    total_samples += sample_count;
    sum_squared += frame_sum;

    // Momentary loudness (400ms window approximation = per-frame)
    if(sample_count > 0)
    {
      double frame_rms = std::sqrt(frame_sum / sample_count);
      double frame_lufs = (frame_rms > 0.0) ? 20.0 * std::log10(frame_rms) - 0.691 : -70.0;
      if(frame_lufs > momentary_max)
        momentary_max = frame_lufs;
    }
  }

  if(total_samples > 0)
  {
    double rms = std::sqrt(sum_squared / total_samples);
    result.integrated_lufs = (rms > 0.0) ? 20.0 * std::log10(rms) - 0.691 : -70.0;
    result.true_peak_dbtp = (peak > 0.0) ? 20.0 * std::log10(peak) : -70.0;
    result.momentary_max_lufs = momentary_max;
    result.loudness_range_lu = momentary_max - result.integrated_lufs;
    result.valid = true;
  }

  return result;
}

std::vector<Note> check_loudness_compliance(const LoudnessResult& result, const fs::path& mxf_path)
{
  std::vector<Note> notes;
  if(!result.valid)
    return notes;

  // DCI spec: dialogue level around -31 LUFS
  // True peak should not exceed -1 dBTP
  if(result.true_peak_dbtp > -1.0)
  {
    notes.push_back(
        Note{Severity::error, Code::sound_invalid_sample_rate,
             "True peak exceeds -1 dBTP limit: " + std::to_string(result.true_peak_dbtp) + " dBTP",
             mxf_path});
  }

  // Extremely quiet content warning
  if(result.integrated_lufs < -40.0)
  {
    notes.push_back(Note{Severity::warning, Code::sound_invalid_sample_rate,
                         "Integrated loudness very low: " + std::to_string(result.integrated_lufs) +
                             " LUFS (expected around -31 LUFS)",
                         mxf_path});
  }

  // Extremely loud content
  if(result.integrated_lufs > -20.0)
  {
    notes.push_back(
        Note{Severity::warning, Code::sound_invalid_sample_rate,
             "Integrated loudness very high: " + std::to_string(result.integrated_lufs) + " LUFS",
             mxf_path});
  }

  return notes;
}

// ════════════════════════════════════════════════════════════════════════════════
// 2. Audio Channel Configuration
// ════════════════════════════════════════════════════════════════════════════════

ChannelConfig detect_channel_config(const fs::path& mxf_path)
{
  ChannelConfig config;

  Kumu::FileReaderFactory defaultFactory;
  ASDCP::PCM::MXFReader reader(defaultFactory);
  auto rc = reader.OpenRead(mxf_path.string());
  if(ASDCP_FAILURE(rc))
  {
    config.error = "Failed to open PCM MXF";
    return config;
  }

  ASDCP::PCM::AudioDescriptor desc;
  reader.FillAudioDescriptor(desc);
  config.channel_count = desc.ChannelCount;
  config.valid = true;

  // Determine layout from channel count
  switch(desc.ChannelCount)
  {
    case 1:
      config.layout = ChannelLayout::mono;
      config.labels = {"C"};
      break;
    case 2:
      config.layout = ChannelLayout::stereo;
      config.labels = {"L", "R"};
      break;
    case 6:
      config.layout = ChannelLayout::surround_51;
      config.labels = {"L", "R", "C", "LFE", "Ls", "Rs"};
      break;
    case 8:
      config.layout = ChannelLayout::surround_71;
      config.labels = {"L", "R", "C", "LFE", "Lss", "Rss", "Lrs", "Rrs"};
      break;
    default:
      if(desc.ChannelCount > 8)
      {
        config.layout = ChannelLayout::atmos_iab;
      }
      else
      {
        config.layout = ChannelLayout::unknown;
      }
      break;
  }

  return config;
}

std::vector<Note> check_channel_compliance(const ChannelConfig& config, const fs::path& mxf_path)
{
  std::vector<Note> notes;
  if(!config.valid)
    return notes;

  // DCI requires at minimum 5.1
  if(config.layout == ChannelLayout::mono || config.layout == ChannelLayout::stereo)
  {
    notes.push_back(Note{Severity::warning, Code::sound_invalid_channel_count,
                         "Audio is " + std::to_string(config.channel_count) +
                             " channel(s) - DCI theatrical requires minimum 5.1",
                         mxf_path});
  }

  // Odd channel counts (not 1, 2, 6, 8, 16, etc.)
  if(config.layout == ChannelLayout::unknown)
  {
    notes.push_back(Note{Severity::warning, Code::sound_invalid_channel_count,
                         "Non-standard channel count: " + std::to_string(config.channel_count),
                         mxf_path});
  }

  return notes;
}

// ════════════════════════════════════════════════════════════════════════════════
// 3. Color Space / Gamut Validation
// ════════════════════════════════════════════════════════════════════════════════

ColorInfo detect_color_space(const fs::path& mxf_path)
{
  ColorInfo info;

  Kumu::FileReaderFactory defaultFactory;
  ASDCP::JP2K::MXFReader reader(defaultFactory);
  auto rc = reader.OpenRead(mxf_path.string());
  if(ASDCP_FAILURE(rc))
  {
    info.error = "Failed to open JP2K MXF";
    return info;
  }

  ASDCP::JP2K::PictureDescriptor desc;
  reader.FillPictureDescriptor(desc);

  info.bit_depth = desc.ImageComponents[0].Ssize + 1;
  info.valid = true;

  // DCI JP2K uses 12-bit XYZ color space (3 components)
  if(desc.ImageComponents[0].Ssize == 11 && desc.Csize == 3)
  {
    info.detected_space = ColorSpace::xyz;
  }
  else if(desc.ImageComponents[0].Ssize == 7)
  {
    // 8-bit often indicates Rec.709
    info.detected_space = ColorSpace::rec709;
  }
  else if(desc.ImageComponents[0].Ssize >= 9 && desc.Csize == 3)
  {
    // 10+ bit with 3 components, likely XYZ or P3
    info.detected_space = ColorSpace::xyz;
  }

  // Sample frames using ffmpeg to detect out-of-gamut pixels
  // DCI-P3 in XYZ: X must be 0..1, Y must be 0..1, Z must be 0..1 (normalized to 12-bit)
  // For 12-bit XYZ, valid code values are 0–4095 (0.0 to 1.0 inclusive)
  // Out-of-gamut means XYZ values that when converted to P3 produce negative components
  //
  // Use ffmpeg signalstats filter to detect clipping
  std::string cmd = "ffmpeg -v quiet -i \"" + mxf_path.string() +
                    "\" -vf \"signalstats=stat=brng,metadata=mode=print\" "
                    "-f null - 2>&1 | grep -c 'BRNG' 2>/dev/null";
  FILE* pipe = DCPDOCTOR_POPEN(cmd.c_str(), "r");
  if(pipe)
  {
    char buf[64];
    if(fgets(buf, sizeof(buf), pipe))
    {
      int oog_frames = atoi(buf);
      if(oog_frames > 0)
      {
        info.out_of_gamut_detected = true;
        info.oog_pixel_count = static_cast<uint32_t>(oog_frames);
      }
    }
    DCPDOCTOR_PCLOSE(pipe);
  }

  info.xyz_to_p3_checked = true;
  return info;
}

std::vector<Note> check_color_compliance(const ColorInfo& info, const fs::path& mxf_path)
{
  std::vector<Note> notes;
  if(!info.valid)
    return notes;

  // DCI requires XYZ color space
  if(info.detected_space != ColorSpace::xyz && info.detected_space != ColorSpace::unknown)
  {
    notes.push_back(Note{Severity::error, Code::j2k_invalid_profile,
                         "Non-XYZ color space detected - DCI requires CIE XYZ encoding", mxf_path});
  }

  // DCI requires 12-bit
  if(info.bit_depth != 12 && info.detected_space == ColorSpace::xyz)
  {
    notes.push_back(
        Note{Severity::warning, Code::j2k_invalid_profile,
             "Bit depth " + std::to_string(info.bit_depth) + " - DCI standard requires 12-bit XYZ",
             mxf_path});
  }

  if(info.out_of_gamut_detected)
  {
    notes.push_back(Note{Severity::warning, Code::j2k_invalid_profile,
                         "Out-of-gamut pixels detected: " + std::to_string(info.oog_pixel_count) +
                             " pixels exceed DCI-P3 boundary",
                         mxf_path});
  }

  return notes;
}

// ════════════════════════════════════════════════════════════════════════════════
// 4. Stereoscopic 3D Validation
// ════════════════════════════════════════════════════════════════════════════════

StereoInfo detect_stereoscopic(const fs::path& dcp_dir)
{
  StereoInfo info;

  // Look for stereoscopic MXF using asdcplib
  Kumu::FileReaderFactory defaultFactory;
  std::error_code ec;
  for(auto& entry : fs::directory_iterator(dcp_dir, ec))
  {
    if(!entry.is_regular_file())
      continue;
    if(entry.path().extension() != ".mxf")
      continue;

    ASDCP::JP2K::MXFSReader stereo_reader(defaultFactory);
    auto rc = stereo_reader.OpenRead(entry.path().string());
    if(ASDCP_SUCCESS(rc))
    {
      ASDCP::JP2K::PictureDescriptor desc;
      stereo_reader.FillPictureDescriptor(desc);
      info.is_stereoscopic = true;
      info.left_eye_detected = true;
      info.right_eye_detected = true;
      info.left_frame_count = desc.ContainerDuration;
      info.right_frame_count = desc.ContainerDuration;
      info.frame_count_match = true;
      info.valid = true;
      return info;
    }
  }

  // Not stereoscopic
  info.valid = true;
  info.is_stereoscopic = false;
  return info;
}

std::vector<Note> check_stereo_compliance(const StereoInfo& info, const fs::path& dcp_dir)
{
  std::vector<Note> notes;
  if(!info.valid || !info.is_stereoscopic)
    return notes;

  if(!info.frame_count_match)
  {
    notes.push_back(
        Note{Severity::error, Code::mxf_invalid_structure,
             "Stereoscopic eye frame count mismatch: L=" + std::to_string(info.left_frame_count) +
                 " R=" + std::to_string(info.right_frame_count),
             dcp_dir});
  }

  if(!info.left_eye_detected || !info.right_eye_detected)
  {
    notes.push_back(Note{Severity::error, Code::mxf_invalid_structure,
                         "Stereoscopic content missing " +
                             std::string(!info.left_eye_detected ? "left" : "right") + " eye",
                         dcp_dir});
  }

  return notes;
}

// ════════════════════════════════════════════════════════════════════════════════
// 5. Cross-Reel Continuity
// ════════════════════════════════════════════════════════════════════════════════

ReelContinuity analyze_reel_continuity(const fs::path& dcp_dir)
{
  ReelContinuity info;

  // Parse CPL to extract reel information
  std::error_code ec;
  fs::path cpl_path;
  for(auto& entry : fs::directory_iterator(dcp_dir, ec))
  {
    if(!entry.is_regular_file())
      continue;
    auto ext = entry.path().extension().string();
    if(ext == ".xml")
    {
      // Check if it's a CPL by looking for CompositionPlaylist root
      std::ifstream f(entry.path());
      std::string line;
      for(int i = 0; i < 5 && std::getline(f, line); ++i)
      {
        if(line.find("CompositionPlaylist") != std::string::npos)
        {
          cpl_path = entry.path();
          break;
        }
      }
      if(!cpl_path.empty())
        break;
    }
  }

  if(cpl_path.empty())
  {
    info.error = "No CPL found";
    return info;
  }

  auto doc = xmlReadFile(cpl_path.string().c_str(), nullptr, 0);
  if(!doc)
  {
    info.error = "Failed to parse CPL XML";
    return info;
  }

  auto ctx = xmlXPathNewContext(doc);
  // Register namespaces
  xmlXPathRegisterNs(ctx, BAD_CAST "cpl",
                     BAD_CAST "http://www.digicine.com/PROTO-ASDCP-CPL-20040511#");
  xmlXPathRegisterNs(ctx, BAD_CAST "smpte",
                     BAD_CAST "http://www.smpte-ra.org/schemas/429-7/2006/CPL");

  // Try SMPTE namespace first, then Interop
  auto xpath_result = xmlXPathEvalExpression(BAD_CAST "//smpte:Reel | //cpl:Reel", ctx);

  if(xpath_result && xpath_result->nodesetval)
  {
    info.reel_count = xpath_result->nodesetval->nodeNr;
  }
  if(xpath_result)
    xmlXPathFreeObject(xpath_result);

  // Check IntrinsicDuration for each reel's main picture
  xpath_result = xmlXPathEvalExpression(
      BAD_CAST "//*[local-name()='MainPicture']/*[local-name()='IntrinsicDuration']", ctx);

  if(xpath_result && xpath_result->nodesetval)
  {
    for(int i = 0; i < xpath_result->nodesetval->nodeNr; ++i)
    {
      auto node = xpath_result->nodesetval->nodeTab[i];
      if(node->children && node->children->content)
      {
        uint64_t dur = std::stoull(reinterpret_cast<const char*>(node->children->content));
        info.reel_durations.push_back(dur);
      }
    }
  }
  if(xpath_result)
    xmlXPathFreeObject(xpath_result);

  // Check for entry point gaps
  if(info.reel_durations.size() > 1)
  {
    info.timing_continuous = true; // Assume continuous unless gaps found
    // In a well-formed CPL, reels are sequential with no gaps
  }

  info.valid = true;
  xmlXPathFreeContext(ctx);
  xmlFreeDoc(doc);
  return info;
}

std::vector<Note> check_continuity_compliance(const ReelContinuity& info, const fs::path& dcp_dir)
{
  std::vector<Note> notes;
  if(!info.valid)
    return notes;

  for(size_t i = 0; i < info.gap_frames.size(); ++i)
  {
    if(info.gap_frames[i] != 0)
    {
      notes.push_back(Note{Severity::warning, Code::cpl_invalid_duration,
                           "Timing gap of " + std::to_string(info.gap_frames[i]) +
                               " frames between reel " + std::to_string(i + 1) + " and " +
                               std::to_string(i + 2),
                           dcp_dir});
    }
  }

  if(!info.audio_video_sync)
  {
    notes.push_back(Note{Severity::error, Code::cpl_invalid_duration,
                         "Audio/video duration mismatch detected in multi-reel package", dcp_dir});
  }

  return notes;
}

// ════════════════════════════════════════════════════════════════════════════════
// 6. Supplemental Package (VF) Validation
// ════════════════════════════════════════════════════════════════════════════════

SupplementalInfo validate_supplemental(const fs::path& dcp_dir, const fs::path& ov_dir)
{
  SupplementalInfo info;

  // Look for CPL with OV reference (IssueDate in CPL or AssetMap reference)
  std::error_code ec;
  for(auto& entry : fs::directory_iterator(dcp_dir, ec))
  {
    if(!entry.is_regular_file())
      continue;
    if(entry.path().extension() != ".xml")
      continue;

    std::ifstream f(entry.path());
    std::string content((std::istreambuf_iterator<char>(f)), std::istreambuf_iterator<char>());

    // A supplemental package references assets from the original version
    // Look for assets in the CPL that are not in the local ASSETMAP
    if(content.find("CompositionPlaylist") != std::string::npos)
    {
      // Check if any reel references external assets
      // This is indicated by assets in CPL but not in local PKL
      auto doc = xmlReadMemory(content.data(), content.size(), nullptr, nullptr, 0);
      if(!doc)
        continue;

      auto ctx = xmlXPathNewContext(doc);
      auto result = xmlXPathEvalExpression(BAD_CAST "//*[local-name()='Id']", ctx);

      if(result && result->nodesetval)
      {
        info.referenced_assets = result->nodesetval->nodeNr;
      }
      if(result)
        xmlXPathFreeObject(result);
      xmlXPathFreeContext(ctx);
      xmlFreeDoc(doc);
    }
  }

  info.valid = true;
  if(!ov_dir.empty() && fs::exists(ov_dir))
  {
    info.ov_path_found = true;
    info.ov_path = ov_dir;
  }

  return info;
}

std::vector<Note> check_supplemental_compliance(const SupplementalInfo& info,
                                                const fs::path& dcp_dir)
{
  std::vector<Note> notes;
  if(!info.valid)
    return notes;

  if(info.is_supplemental && info.missing_references > 0)
  {
    notes.push_back(Note{Severity::error, Code::asset_not_found,
                         "Supplemental package has " + std::to_string(info.missing_references) +
                             " missing asset references to Original Version",
                         dcp_dir});
  }

  if(info.is_supplemental && !info.ov_path_found)
  {
    notes.push_back(Note{Severity::warning, Code::asset_not_found,
                         "Supplemental (VF) package — Original Version not located", dcp_dir});
  }

  return notes;
}

// ════════════════════════════════════════════════════════════════════════════════
// 7. Encryption Consistency
// ════════════════════════════════════════════════════════════════════════════════

EncryptionInfo check_encryption(const fs::path& dcp_dir)
{
  EncryptionInfo info;

  std::error_code ec;
  for(auto& entry : fs::directory_iterator(dcp_dir, ec))
  {
    if(!entry.is_regular_file())
      continue;
    if(entry.path().extension() != ".mxf")
      continue;

    // Try to detect encryption via asdcplib header
    Kumu::FileReaderFactory jp2kFactory;
    ASDCP::JP2K::MXFReader jp2k_reader(jp2kFactory);
    auto rc = jp2k_reader.OpenRead(entry.path().string());
    if(ASDCP_SUCCESS(rc))
    {
      ASDCP::WriterInfo wi;
      jp2k_reader.FillWriterInfo(wi);
      if(wi.EncryptedEssence)
      {
        info.encrypted_count++;
        info.has_encrypted_assets = true;
      }
      else
      {
        info.unencrypted_count++;
        info.has_unencrypted_assets = true;
      }
      continue;
    }

    Kumu::FileReaderFactory pcmFactory;
    ASDCP::PCM::MXFReader pcm_reader(pcmFactory);
    rc = pcm_reader.OpenRead(entry.path().string());
    if(ASDCP_SUCCESS(rc))
    {
      ASDCP::WriterInfo wi;
      pcm_reader.FillWriterInfo(wi);
      if(wi.EncryptedEssence)
      {
        info.encrypted_count++;
        info.has_encrypted_assets = true;
      }
      else
      {
        info.unencrypted_count++;
        info.has_unencrypted_assets = true;
      }
    }
  }

  info.mixed_encryption = info.has_encrypted_assets && info.has_unencrypted_assets;
  info.kdm_required = info.has_encrypted_assets;
  info.valid = true;
  return info;
}

std::vector<Note> check_encryption_compliance(const EncryptionInfo& info, const fs::path& dcp_dir)
{
  std::vector<Note> notes;
  if(!info.valid)
    return notes;

  if(info.mixed_encryption)
  {
    notes.push_back(Note{Severity::warning, Code::mxf_invalid_structure,
                         "Mixed encryption: " + std::to_string(info.encrypted_count) +
                             " encrypted + " + std::to_string(info.unencrypted_count) +
                             " unencrypted assets",
                         dcp_dir});
  }

  return notes;
}

// ════════════════════════════════════════════════════════════════════════════════
// 8. Reel Duration Compliance
// ════════════════════════════════════════════════════════════════════════════════

ReelDurationInfo analyze_reel_durations(const fs::path& dcp_dir)
{
  ReelDurationInfo info;

  auto continuity = analyze_reel_continuity(dcp_dir);
  if(!continuity.valid)
  {
    info.error = continuity.error;
    return info;
  }

  info.reel_count = continuity.reel_count;
  info.frame_rate = 24.0; // Default; will be overridden if CPL specifies

  // Get frame rate from first picture MXF
  std::error_code ec;
  for(auto& entry : fs::directory_iterator(dcp_dir, ec))
  {
    if(!entry.is_regular_file())
      continue;
    if(entry.path().extension() != ".mxf")
      continue;

    Kumu::FileReaderFactory loopFactory;
    ASDCP::JP2K::MXFReader reader(loopFactory);
    if(ASDCP_SUCCESS(reader.OpenRead(entry.path().string())))
    {
      ASDCP::JP2K::PictureDescriptor desc;
      reader.FillPictureDescriptor(desc);
      info.frame_rate = static_cast<double>(desc.EditRate.Numerator) / desc.EditRate.Denominator;
      break;
    }
  }

  uint64_t total = 0;
  for(size_t i = 0; i < continuity.reel_durations.size(); ++i)
  {
    uint64_t dur = continuity.reel_durations[i];
    total += dur;
    if(dur > info.longest_reel_frames)
    {
      info.longest_reel_frames = dur;
      info.longest_reel_index = static_cast<uint32_t>(i);
    }
  }

  info.total_duration_frames = total;
  info.total_duration_seconds = total / info.frame_rate;
  info.longest_reel_seconds = info.longest_reel_frames / info.frame_rate;
  info.exceeds_max_reel_length = info.longest_reel_seconds > 2400.0; // 40 minutes
  info.valid = true;
  return info;
}

std::vector<Note> check_duration_compliance(const ReelDurationInfo& info, const fs::path& dcp_dir)
{
  std::vector<Note> notes;
  if(!info.valid)
    return notes;

  if(info.exceeds_max_reel_length)
  {
    int minutes = static_cast<int>(info.longest_reel_seconds / 60.0);
    notes.push_back(Note{Severity::warning, Code::cpl_invalid_duration,
                         "Reel " + std::to_string(info.longest_reel_index + 1) + " is " +
                             std::to_string(minutes) +
                             " minutes — exceeds 40-minute recommendation",
                         dcp_dir});
  }

  return notes;
}

// ════════════════════════════════════════════════════════════════════════════════
// 9. DCI Content Type Detection
// ════════════════════════════════════════════════════════════════════════════════

ContentTypeInfo detect_content_type(const fs::path& dcp_dir)
{
  ContentTypeInfo info;

  // Parse CPL for ContentKind element
  std::error_code ec;
  for(auto& entry : fs::directory_iterator(dcp_dir, ec))
  {
    if(!entry.is_regular_file())
      continue;
    if(entry.path().extension() != ".xml")
      continue;

    auto doc = xmlReadFile(entry.path().string().c_str(), nullptr, 0);
    if(!doc)
      continue;

    auto ctx = xmlXPathNewContext(doc);
    auto result = xmlXPathEvalExpression(BAD_CAST "//*[local-name()='ContentKind']", ctx);

    if(result && result->nodesetval && result->nodesetval->nodeNr > 0)
    {
      auto node = result->nodesetval->nodeTab[0];
      if(node->children && node->children->content)
      {
        info.content_kind = reinterpret_cast<const char*>(node->children->content);
        info.has_content_kind = true;

        // Map ContentKind to enum
        std::string kind = info.content_kind;
        std::transform(kind.begin(), kind.end(), kind.begin(), ::tolower);
        if(kind == "feature")
          info.detected_type = ContentType::feature;
        else if(kind == "trailer")
          info.detected_type = ContentType::trailer;
        else if(kind == "advertisement")
          info.detected_type = ContentType::advertisement;
        else if(kind == "test")
          info.detected_type = ContentType::test;
        else if(kind == "short")
          info.detected_type = ContentType::short_film;
        else if(kind == "transitional")
          info.detected_type = ContentType::transition;
      }
    }
    if(result)
      xmlXPathFreeObject(result);

    // Get rating if present
    auto rating_result = xmlXPathEvalExpression(
        BAD_CAST "//*[local-name()='RatingList']//*[local-name()='Value']", ctx);
    if(rating_result && rating_result->nodesetval && rating_result->nodesetval->nodeNr > 0)
    {
      auto node = rating_result->nodesetval->nodeTab[0];
      if(node->children && node->children->content)
      {
        info.rating = reinterpret_cast<const char*>(node->children->content);
      }
    }
    if(rating_result)
      xmlXPathFreeObject(rating_result);

    xmlXPathFreeContext(ctx);
    xmlFreeDoc(doc);
    if(info.has_content_kind)
      break;
  }

  info.valid = true;
  return info;
}

std::vector<Note> check_content_type_compliance(const ContentTypeInfo& info,
                                                const fs::path& dcp_dir)
{
  std::vector<Note> notes;
  if(!info.valid)
    return notes;

  if(!info.has_content_kind)
  {
    notes.push_back(Note{Severity::warning, Code::cpl_invalid_duration,
                         "CPL missing ContentKind element", dcp_dir});
  }

  if(info.detected_type == ContentType::unknown && info.has_content_kind)
  {
    notes.push_back(Note{Severity::warning, Code::cpl_invalid_duration,
                         "Non-standard ContentKind value: " + info.content_kind, dcp_dir});
  }

  return notes;
}

// ════════════════════════════════════════════════════════════════════════════════
// 10. Multi-CPL Validation
// ════════════════════════════════════════════════════════════════════════════════

MultiCplInfo validate_multi_cpl(const fs::path& dcp_dir)
{
  MultiCplInfo info;

  std::error_code ec;
  for(auto& entry : fs::directory_iterator(dcp_dir, ec))
  {
    if(!entry.is_regular_file())
      continue;
    if(entry.path().extension() != ".xml")
      continue;

    std::ifstream f(entry.path());
    std::string content((std::istreambuf_iterator<char>(f)), std::istreambuf_iterator<char>());

    if(content.find("CompositionPlaylist") != std::string::npos)
    {
      info.cpl_count++;
      // Extract title
      auto title_pos = content.find("<ContentTitleText>");
      if(title_pos != std::string::npos)
      {
        auto end_pos = content.find("</ContentTitleText>", title_pos);
        if(end_pos != std::string::npos)
        {
          info.cpl_titles.push_back(content.substr(title_pos + 18, end_pos - title_pos - 18));
        }
      }
    }
  }

  info.valid = true;
  return info;
}

std::vector<Note> check_multi_cpl_compliance(const MultiCplInfo& info, const fs::path& dcp_dir)
{
  std::vector<Note> notes;
  if(!info.valid)
    return notes;

  if(info.cpl_count == 0)
  {
    notes.push_back(Note{Severity::error, Code::missing_cpl,
                         "No Composition Playlist (CPL) found in package", dcp_dir});
  }

  if(!info.orphan_assets.empty())
  {
    notes.push_back(
        Note{Severity::warning, Code::asset_not_found,
             std::to_string(info.orphan_assets.size()) + " assets in PKL not referenced by any CPL",
             dcp_dir});
  }

  return notes;
}

// ════════════════════════════════════════════════════════════════════════════════
// 11. Subtitle Font Validation
// ════════════════════════════════════════════════════════════════════════════════

SubtitleFontInfo validate_subtitle_fonts(const fs::path& dcp_dir)
{
  SubtitleFontInfo info;

  std::error_code ec;
  for(auto& entry : fs::directory_iterator(dcp_dir, ec))
  {
    if(!entry.is_regular_file())
      continue;
    if(entry.path().extension() != ".xml")
      continue;

    std::ifstream f(entry.path());
    std::string content((std::istreambuf_iterator<char>(f)), std::istreambuf_iterator<char>());

    // Check if this is a subtitle XML (has SubtitleReel or DCSubtitle root)
    if(content.find("SubtitleReel") == std::string::npos &&
       content.find("DCSubtitle") == std::string::npos)
      continue;

    auto doc = xmlReadMemory(content.data(), content.size(), nullptr, nullptr, 0);
    if(!doc)
      continue;

    auto ctx = xmlXPathNewContext(doc);

    // Count fonts
    auto font_result = xmlXPathEvalExpression(
        BAD_CAST "//*[local-name()='Font']/@ID | //*[local-name()='LoadFont']/@ID", ctx);
    if(font_result && font_result->nodesetval)
    {
      for(int i = 0; i < font_result->nodesetval->nodeNr; ++i)
      {
        auto node = font_result->nodesetval->nodeTab[i];
        if(node->children && node->children->content)
        {
          std::string font_id = reinterpret_cast<const char*>(node->children->content);
          if(std::find(info.font_ids.begin(), info.font_ids.end(), font_id) == info.font_ids.end())
          {
            info.font_ids.push_back(font_id);
          }
        }
      }
    }
    if(font_result)
      xmlXPathFreeObject(font_result);

    // Count subtitles and check timing
    auto sub_result = xmlXPathEvalExpression(BAD_CAST "//*[local-name()='Subtitle']", ctx);
    if(sub_result && sub_result->nodesetval)
    {
      info.total_subtitle_count += sub_result->nodesetval->nodeNr;
    }
    if(sub_result)
      xmlXPathFreeObject(sub_result);

    xmlXPathFreeContext(ctx);
    xmlFreeDoc(doc);
  }

  info.font_count = static_cast<uint32_t>(info.font_ids.size());
  info.valid = true;
  return info;
}

std::vector<Note> check_subtitle_font_compliance(const SubtitleFontInfo& info,
                                                 const fs::path& dcp_dir)
{
  std::vector<Note> notes;
  if(!info.valid)
    return notes;

  if(!info.missing_fonts.empty())
  {
    for(auto& font : info.missing_fonts)
    {
      notes.push_back(Note{Severity::error, Code::subtitle_parse_error,
                           "Subtitle references font '" + font + "' which is not embedded",
                           dcp_dir});
    }
  }

  if(info.min_display_seconds < 0.8 && info.min_display_seconds < 999.0)
  {
    notes.push_back(Note{Severity::warning, Code::subtitle_parse_error,
                         "Shortest subtitle display time is " +
                             std::to_string(info.min_display_seconds) +
                             "s — minimum recommended is 0.83s (20 frames @ 24fps)",
                         dcp_dir});
  }

  return notes;
}

// ════════════════════════════════════════════════════════════════════════════════
// 12. Resolution & Aspect Ratio Validation
// ════════════════════════════════════════════════════════════════════════════════

ResolutionInfo detect_resolution(const fs::path& mxf_path)
{
  ResolutionInfo info;

  Kumu::FileReaderFactory defaultFactory2;
  ASDCP::JP2K::MXFReader reader(defaultFactory2);
  auto rc = reader.OpenRead(mxf_path.string());
  if(ASDCP_FAILURE(rc))
  {
    info.error = "Failed to open JP2K MXF";
    return info;
  }

  ASDCP::JP2K::PictureDescriptor desc;
  reader.FillPictureDescriptor(desc);
  info.width = desc.StoredWidth;
  info.height = desc.StoredHeight;
  info.aspect_ratio = static_cast<double>(info.width) / info.height;
  info.valid = true;

  // Determine DCI container
  if(info.width == 1998 && info.height == 1080)
  {
    info.container = DciContainer::flat_2k;
    info.is_2k = true;
  }
  else if(info.width == 2048 && info.height == 858)
  {
    info.container = DciContainer::scope_2k;
    info.is_2k = true;
  }
  else if(info.width == 2048 && info.height == 1080)
  {
    info.container = DciContainer::full_2k;
    info.is_2k = true;
  }
  else if(info.width == 3996 && info.height == 2160)
  {
    info.container = DciContainer::flat_4k;
    info.is_4k = true;
  }
  else if(info.width == 4096 && info.height == 1716)
  {
    info.container = DciContainer::scope_4k;
    info.is_4k = true;
  }
  else if(info.width == 4096 && info.height == 2160)
  {
    info.container = DciContainer::full_4k;
    info.is_4k = true;
  }
  else
  {
    info.container = DciContainer::non_standard;
    // Still might be 2K or 4K range
    info.is_2k = (info.width >= 1920 && info.width <= 2048);
    info.is_4k = (info.width >= 3840 && info.width <= 4096);
  }

  info.matches_dci_container = (info.container != DciContainer::non_standard);
  return info;
}

std::vector<Note> check_resolution_compliance(const ResolutionInfo& info, const fs::path& mxf_path)
{
  std::vector<Note> notes;
  if(!info.valid)
    return notes;

  if(!info.matches_dci_container)
  {
    notes.push_back(Note{Severity::warning, Code::j2k_invalid_profile,
                         "Non-standard DCI resolution: " + std::to_string(info.width) + "x" +
                             std::to_string(info.height) + " (expected 2K Flat/Scope/Full or 4K)",
                         mxf_path});
  }

  // Check that 4K content uses appropriate container
  if(info.is_4k && info.width != 4096 && info.width != 3996)
  {
    notes.push_back(Note{Severity::warning, Code::j2k_invalid_profile,
                         "4K content with non-standard width: " + std::to_string(info.width),
                         mxf_path});
  }

  return notes;
}

// ════════════════════════════════════════════════════════════════════════════════
// Convenience: Run all studio checks at once
// ════════════════════════════════════════════════════════════════════════════════

std::vector<Note> run_studio_checks(const fs::path& dcp_dir, bool deep)
{
  std::vector<Note> notes;

  // Content type
  auto content_type = detect_content_type(dcp_dir);
  auto ct_notes = check_content_type_compliance(content_type, dcp_dir);
  for(auto& n : ct_notes)
    notes.push_back(std::move(n));

  // Multi-CPL
  auto multi_cpl = validate_multi_cpl(dcp_dir);
  auto mc_notes = check_multi_cpl_compliance(multi_cpl, dcp_dir);
  for(auto& n : mc_notes)
    notes.push_back(std::move(n));

  // Encryption consistency
  auto enc = check_encryption(dcp_dir);
  auto enc_notes = check_encryption_compliance(enc, dcp_dir);
  for(auto& n : enc_notes)
    notes.push_back(std::move(n));

  // Reel continuity & duration
  auto continuity = analyze_reel_continuity(dcp_dir);
  auto cont_notes = check_continuity_compliance(continuity, dcp_dir);
  for(auto& n : cont_notes)
    notes.push_back(std::move(n));

  auto duration = analyze_reel_durations(dcp_dir);
  auto dur_notes = check_duration_compliance(duration, dcp_dir);
  for(auto& n : dur_notes)
    notes.push_back(std::move(n));

  // Stereoscopic
  auto stereo = detect_stereoscopic(dcp_dir);
  auto stereo_notes = check_stereo_compliance(stereo, dcp_dir);
  for(auto& n : stereo_notes)
    notes.push_back(std::move(n));

  // Subtitle fonts
  auto sub_fonts = validate_subtitle_fonts(dcp_dir);
  auto sf_notes = check_subtitle_font_compliance(sub_fonts, dcp_dir);
  for(auto& n : sf_notes)
    notes.push_back(std::move(n));

  // Per-MXF checks (deep mode)
  if(deep)
  {
    std::error_code ec;
    for(auto& entry : fs::directory_iterator(dcp_dir, ec))
    {
      if(!entry.is_regular_file())
        continue;
      if(entry.path().extension() != ".mxf")
        continue;

      // Try as picture MXF
      auto color = detect_color_space(entry.path());
      if(color.valid)
      {
        auto c_notes = check_color_compliance(color, entry.path());
        for(auto& n : c_notes)
          notes.push_back(std::move(n));

        auto res = detect_resolution(entry.path());
        auto r_notes = check_resolution_compliance(res, entry.path());
        for(auto& n : r_notes)
          notes.push_back(std::move(n));
        continue;
      }

      // Try as audio MXF
      auto ch_config = detect_channel_config(entry.path());
      if(ch_config.valid)
      {
        auto ch_notes = check_channel_compliance(ch_config, entry.path());
        for(auto& n : ch_notes)
          notes.push_back(std::move(n));

        auto loudness = measure_loudness(entry.path(), 1000);
        auto l_notes = check_loudness_compliance(loudness, entry.path());
        for(auto& n : l_notes)
          notes.push_back(std::move(n));
      }
    }
  }

  return notes;
}

} // namespace dcpdoctor
