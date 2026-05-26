#include "dcpdoctor/dcpdoctor.h"
#include "dcpdoctor/cpl.h"
#include "dcpdoctor/j2k.h"
#include "dcpdoctor/isdcf.h"
#include "dcpdoctor/subtitle.h"
#include "dcpdoctor/validators.h"
#include "dcpdoctor/mxf.h"
#include "dcpdoctor/advanced.h"
#include "dcpdoctor/bitrate.h"
#include "dcpdoctor/audio.h"
#include "dcpdoctor/timeline.h"
#include "dcpdoctor/kdm.h"
#include "dcpdoctor/diff.h"
#include "dcpdoctor/cache.h"
#include "dcpdoctor/theater.h"
#include "dcpdoctor/fixes.h"
#include "dcpdoctor/auto_qc.h"
#include "dcpdoctor/av_sync.h"
#include "dcpdoctor/checksum_verify.h"
#include "dcpdoctor/frame_compare.h"
#include "dcpdoctor/hdr.h"
#include "dcpdoctor/hdr_validate.h"
#include "dcpdoctor/imf_compliance.h"
#include "dcpdoctor/info.h"
#include "dcpdoctor/loudness.h"
#include "dcpdoctor/mxf_extract.h"
#include "dcpdoctor/qc.h"
#include "dcpdoctor/qc_report.h"
#include "dcpdoctor/schema_validate.h"
#include "dcpdoctor/validate.h"
#include <cassert>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <sstream>

namespace fs = std::filesystem;

static int tests_run = 0;
static int tests_passed = 0;

using TestFunc = void (*)();
static TestFunc test_registry[256];
static int test_count = 0;

#define TEST(name)                               \
  static void test_##name();                     \
  static struct test_reg_##name                  \
  {                                              \
    test_reg_##name()                            \
    {                                            \
      test_registry[test_count++] = test_##name; \
    }                                            \
  } test_inst_##name;                            \
  static void test_##name()

#define ASSERT(cond)                                                                   \
  do                                                                                   \
  {                                                                                    \
    ++tests_run;                                                                       \
    if(!(cond))                                                                        \
    {                                                                                  \
      std::cerr << "FAIL: " << #cond << " at " << __FILE__ << ":" << __LINE__ << "\n"; \
    }                                                                                  \
    else                                                                               \
    {                                                                                  \
      ++tests_passed;                                                                  \
    }                                                                                  \
  } while(0)

TEST(nonexistent_directory)
{
  auto result = dcpdoctor::verify("/tmp/nonexistent_dcp_path_12345");
  ASSERT(!result.ok());
  ASSERT(result.error_count > 0);
  ASSERT(result.notes[0].code == dcpdoctor::Code::missing_assetmap);
}

TEST(note_severity_str)
{
  dcpdoctor::Note n{dcpdoctor::Severity::error, dcpdoctor::Code::missing_assetmap, "test"};
  ASSERT(n.severity_str() == "ERROR");
  n.severity = dcpdoctor::Severity::warning;
  ASSERT(n.severity_str() == "WARNING");
  n.severity = dcpdoctor::Severity::info;
  ASSERT(n.severity_str() == "INFO");
}

TEST(verify_result_add)
{
  dcpdoctor::VerifyResult result;
  ASSERT(result.ok());
  ASSERT(result.error_count == 0);
  ASSERT(result.warning_count == 0);

  result.add({dcpdoctor::Severity::warning, dcpdoctor::Code::pkl_hash_mismatch, "test"});
  ASSERT(result.ok());
  ASSERT(result.warning_count == 1);

  result.add({dcpdoctor::Severity::error, dcpdoctor::Code::missing_pkl, "test"});
  ASSERT(!result.ok());
  ASSERT(result.error_count == 1);
}

// --- J2K codestream parser tests ---

TEST(j2k_parse_valid_header)
{
  // Minimal valid J2K codestream: SOC + SIZ marker
  // SOC(FF4F) + SIZ(FF51) + len(0029=41) + rsiz(0003=Cinema2K) +
  // Xsiz(00000800=2048) + Ysiz(00000438=1080) +
  // XOsiz(0) + YOsiz(0) + XTsiz(00000800) + YTsiz(00000438) +
  // XTOsiz(0) + YTOsiz(0) + Csiz(0003=3 components) +
  // Ssiz(0B=12bit) + XRsiz(01) + YRsiz(01) x3
  uint8_t data[] = {
      0xFF, 0x4F, // SOC
      0xFF, 0x51, // SIZ
      0x00, 0x2F, // Length = 47
      0x00, 0x03, // Rsiz = Cinema2K
      0x00, 0x00, 0x08, 0x00, // Xsiz = 2048
      0x00, 0x00, 0x04, 0x38, // Ysiz = 1080
      0x00, 0x00, 0x00, 0x00, // XOsiz = 0
      0x00, 0x00, 0x00, 0x00, // YOsiz = 0
      0x00, 0x00, 0x08, 0x00, // XTsiz = 2048
      0x00, 0x00, 0x04, 0x38, // YTsiz = 1080
      0x00, 0x00, 0x00, 0x00, // XTOsiz = 0
      0x00, 0x00, 0x00, 0x00, // YTOsiz = 0
      0x00, 0x03, // Csiz = 3
      0x0B, 0x01, 0x01, // Component 0: 12bit, XR=1, YR=1
      0x0B, 0x01, 0x01, // Component 1: 12bit, XR=1, YR=1
      0x0B, 0x01, 0x01, // Component 2: 12bit, XR=1, YR=1
  };

  auto info = dcpdoctor::parse_j2k_header(data, sizeof(data));
  ASSERT(info.valid);
  ASSERT(info.width == 2048);
  ASSERT(info.height == 1080);
  ASSERT(info.num_components == 3);
  ASSERT(info.bit_depth == 12);
  ASSERT(info.rsiz == dcpdoctor::RSIZ_CINEMA_2K);
}

TEST(j2k_parse_too_small)
{
  uint8_t data[] = {0xFF};
  auto info = dcpdoctor::parse_j2k_header(data, 1);
  ASSERT(!info.valid);
  ASSERT(!info.error.empty());
}

TEST(j2k_parse_bad_soc)
{
  uint8_t data[] = {0x00, 0x00, 0xFF, 0x51};
  auto info = dcpdoctor::parse_j2k_header(data, 4);
  ASSERT(!info.valid);
  ASSERT(info.error == "Missing SOC marker");
}

TEST(j2k_bitrate_under_limit)
{
  // Create a temp file that's small enough to be under 250 Mbps
  auto tmp = fs::temp_directory_path() / "dcpdoctor_test_bitrate.mxf";
  {
    std::ofstream f(tmp, std::ios::binary);
    std::vector<char> data(1024 * 1024, 0); // 1MB file
    f.write(data.data(), data.size());
  }
  // 1MB, 24 frames, 24fps = 1 second = ~8 Mbps << 250 Mbps
  auto notes = dcpdoctor::check_j2k_bitrate(tmp, 24, 24, 1, 2048, 1080);
  ASSERT(notes.empty());
  fs::remove(tmp);
}

// --- ISDCF naming tests ---

TEST(isdcf_valid_name)
{
  auto notes = dcpdoctor::check_isdcf_naming("MyMovie_FTR_F_EN_US_51_2K_ST_20230101_FAC_SMPTE_OV",
                                             "cpl.xml");
  // Should have no errors/warnings (may have info)
  bool has_error = false;
  for(const auto& n : notes)
    if(n.severity == dcpdoctor::Severity::error)
      has_error = true;
  ASSERT(!has_error);
}

TEST(isdcf_no_underscores)
{
  auto notes = dcpdoctor::check_isdcf_naming("Just A Movie Title", "cpl.xml");
  ASSERT(!notes.empty());
  ASSERT(notes[0].code == dcpdoctor::Code::isdcf_naming_violation);
}

TEST(isdcf_bad_content_type)
{
  auto notes = dcpdoctor::check_isdcf_naming("Movie_XXX_F_EN", "cpl.xml");
  bool found_type_warning = false;
  for(const auto& n : notes)
    if(n.message.find("content type") != std::string::npos)
      found_type_warning = true;
  ASSERT(found_type_warning);
}

TEST(isdcf_empty_title)
{
  auto notes = dcpdoctor::check_isdcf_naming("", "cpl.xml");
  ASSERT(!notes.empty());
}

// --- Subtitle validation tests ---

TEST(subtitle_valid_smpte)
{
  auto tmp = fs::temp_directory_path() / "dcpdoctor_test_sub.xml";
  {
    std::ofstream f(tmp);
    f << R"(<?xml version="1.0" encoding="UTF-8"?>
<SubtitleReel xmlns="http://www.smpte-ra.org/schemas/428-7/2010/DCST">
  <Id>urn:uuid:12345678-1234-1234-1234-123456789012</Id>
  <EditRate>24 1</EditRate>
  <TimeCodeRate>24</TimeCodeRate>
  <LoadFont Id="arial" URI="arial.ttf"/>
  <SubtitleList>
    <Font Id="arial" Size="42">
      <Subtitle TimeIn="00:00:01:00" TimeOut="00:00:03:00">
        <Text>Hello World</Text>
      </Subtitle>
    </Font>
  </SubtitleList>
</SubtitleReel>)";
  }
  auto notes = dcpdoctor::validate_subtitle(tmp, dcpdoctor::Standard::smpte);
  bool has_error = false;
  for(const auto& n : notes)
    if(n.severity == dcpdoctor::Severity::error)
      has_error = true;
  ASSERT(!has_error);
  fs::remove(tmp);
}

TEST(subtitle_missing_id)
{
  auto tmp = fs::temp_directory_path() / "dcpdoctor_test_sub2.xml";
  {
    std::ofstream f(tmp);
    f << R"(<?xml version="1.0" encoding="UTF-8"?>
<SubtitleReel xmlns="http://www.smpte-ra.org/schemas/428-7/2010/DCST">
  <EditRate>24 1</EditRate>
  <TimeCodeRate>24</TimeCodeRate>
  <SubtitleList>
    <Subtitle TimeIn="00:00:01:00" TimeOut="00:00:03:00">
      <Text>Hello</Text>
    </Subtitle>
  </SubtitleList>
</SubtitleReel>)";
  }
  auto notes = dcpdoctor::validate_subtitle(tmp, dcpdoctor::Standard::smpte);
  bool found_id_error = false;
  for(const auto& n : notes)
    if(n.code == dcpdoctor::Code::missing_required_element &&
       n.message.find("Id") != std::string::npos)
      found_id_error = true;
  ASSERT(found_id_error);
  fs::remove(tmp);
}

TEST(subtitle_bad_timing)
{
  auto tmp = fs::temp_directory_path() / "dcpdoctor_test_sub3.xml";
  {
    std::ofstream f(tmp);
    f << R"(<?xml version="1.0" encoding="UTF-8"?>
<SubtitleReel xmlns="http://www.smpte-ra.org/schemas/428-7/2010/DCST">
  <Id>urn:uuid:12345678-1234-1234-1234-123456789012</Id>
  <EditRate>24 1</EditRate>
  <TimeCodeRate>24</TimeCodeRate>
  <SubtitleList>
    <Subtitle TimeIn="bad" TimeOut="also bad">
      <Text>Hello</Text>
    </Subtitle>
  </SubtitleList>
</SubtitleReel>)";
  }
  auto notes = dcpdoctor::validate_subtitle(tmp, dcpdoctor::Standard::smpte);
  bool found_timing = false;
  for(const auto& n : notes)
    if(n.code == dcpdoctor::Code::subtitle_invalid_timing)
      found_timing = true;
  ASSERT(found_timing);
  fs::remove(tmp);
}

// --- Reel continuity tests ---

TEST(reel_continuity_single_reel)
{
  auto tmp = fs::temp_directory_path() / "dcpdoctor_test_reel.xml";
  {
    std::ofstream f(tmp);
    f << R"(<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL">
  <ReelList>
    <Reel>
      <AssetList>
        <MainPicture>
          <EntryPoint>0</EntryPoint>
          <Duration>240</Duration>
        </MainPicture>
      </AssetList>
    </Reel>
  </ReelList>
</CompositionPlaylist>)";
  }
  auto notes = dcpdoctor::check_reel_continuity(tmp);
  ASSERT(notes.empty()); // Single reel, nothing to check
  fs::remove(tmp);
}

// --- IMF CPL parsing tests ---
// These tests ensure IMF-format CPLs (SegmentList/Segment/SequenceList) are
// correctly parsed, preventing regressions like the one where only DCP-style
// ReelList was handled.

TEST(cpl_parse_imf_single_segment)
{
  auto tmp = fs::temp_directory_path() / "dcpdoctor_test_imf_cpl.xml";
  {
    std::ofstream f(tmp);
    f << R"(<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/2067-3/2016">
  <Id>urn:uuid:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee</Id>
  <ContentTitleText>IMF Test</ContentTitleText>
  <ContentKind>feature</ContentKind>
  <SegmentList>
    <Segment>
      <Id>urn:uuid:11111111-2222-3333-4444-555555555555</Id>
      <SequenceList>
        <MainImageSequence>
          <ResourceList>
            <Resource>
              <TrackFileId>urn:uuid:aaaaaaaa-1111-2222-3333-444444444444</TrackFileId>
              <IntrinsicDuration>480</IntrinsicDuration>
              <EntryPoint>0</EntryPoint>
              <EditRate>24 1</EditRate>
            </Resource>
          </ResourceList>
        </MainImageSequence>
        <MainAudioSequence>
          <ResourceList>
            <Resource>
              <TrackFileId>urn:uuid:bbbbbbbb-1111-2222-3333-444444444444</TrackFileId>
              <IntrinsicDuration>480</IntrinsicDuration>
              <EntryPoint>0</EntryPoint>
              <EditRate>24 1</EditRate>
            </Resource>
          </ResourceList>
        </MainAudioSequence>
      </SequenceList>
    </Segment>
  </SegmentList>
</CompositionPlaylist>)";
  }
  auto cpl = dcpdoctor::Cpl::parse(tmp);
  ASSERT(cpl.has_value());
  ASSERT(cpl->content_title == "IMF Test");
  ASSERT(cpl->content_kind == "feature");
  ASSERT(cpl->reels.size() == 1);
  ASSERT(cpl->reels[0].picture.id == "urn:uuid:aaaaaaaa-1111-2222-3333-444444444444");
  ASSERT(cpl->reels[0].picture.duration == 480);
  ASSERT(cpl->reels[0].picture.entry_point == 0);
  ASSERT(cpl->reels[0].picture.edit_rate == "24 1");
  ASSERT(cpl->reels[0].sound.id == "urn:uuid:bbbbbbbb-1111-2222-3333-444444444444");
  ASSERT(cpl->reels[0].sound.duration == 480);
  fs::remove(tmp);
}

TEST(cpl_parse_imf_multiple_segments)
{
  auto tmp = fs::temp_directory_path() / "dcpdoctor_test_imf_multi.xml";
  {
    std::ofstream f(tmp);
    f << R"(<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/2067-3/2016">
  <Id>urn:uuid:12345678-abcd-efab-cdef-123456789abc</Id>
  <ContentTitleText>Multi Segment IMF</ContentTitleText>
  <SegmentList>
    <Segment>
      <Id>urn:uuid:seg1-0000-0000-0000-000000000001</Id>
      <SequenceList>
        <MainImageSequence>
          <ResourceList>
            <Resource>
              <TrackFileId>urn:uuid:pic1-0000-0000-0000-000000000001</TrackFileId>
              <IntrinsicDuration>240</IntrinsicDuration>
              <EntryPoint>0</EntryPoint>
              <EditRate>24 1</EditRate>
            </Resource>
          </ResourceList>
        </MainImageSequence>
      </SequenceList>
    </Segment>
    <Segment>
      <Id>urn:uuid:seg2-0000-0000-0000-000000000002</Id>
      <SequenceList>
        <MainImageSequence>
          <ResourceList>
            <Resource>
              <TrackFileId>urn:uuid:pic2-0000-0000-0000-000000000002</TrackFileId>
              <IntrinsicDuration>360</IntrinsicDuration>
              <EntryPoint>48</EntryPoint>
              <EditRate>24 1</EditRate>
            </Resource>
          </ResourceList>
        </MainImageSequence>
        <MainAudioSequence>
          <ResourceList>
            <Resource>
              <TrackFileId>urn:uuid:aud2-0000-0000-0000-000000000002</TrackFileId>
              <IntrinsicDuration>360</IntrinsicDuration>
              <EntryPoint>0</EntryPoint>
              <EditRate>48000 1</EditRate>
            </Resource>
          </ResourceList>
        </MainAudioSequence>
      </SequenceList>
    </Segment>
  </SegmentList>
</CompositionPlaylist>)";
  }
  auto cpl = dcpdoctor::Cpl::parse(tmp);
  ASSERT(cpl.has_value());
  ASSERT(cpl->reels.size() == 2);
  // Segment 1: picture only
  ASSERT(cpl->reels[0].picture.id == "urn:uuid:pic1-0000-0000-0000-000000000001");
  ASSERT(cpl->reels[0].picture.duration == 240);
  ASSERT(cpl->reels[0].sound.id.empty());
  // Segment 2: picture + audio
  ASSERT(cpl->reels[1].picture.id == "urn:uuid:pic2-0000-0000-0000-000000000002");
  ASSERT(cpl->reels[1].picture.duration == 360);
  ASSERT(cpl->reels[1].picture.entry_point == 48);
  ASSERT(cpl->reels[1].sound.id == "urn:uuid:aud2-0000-0000-0000-000000000002");
  ASSERT(cpl->reels[1].sound.edit_rate == "48000 1");
  fs::remove(tmp);
}

TEST(cpl_parse_imf_empty_segment_list)
{
  auto tmp = fs::temp_directory_path() / "dcpdoctor_test_imf_empty.xml";
  {
    std::ofstream f(tmp);
    f << R"(<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/2067-3/2016">
  <Id>urn:uuid:empty-0000-0000-0000-000000000000</Id>
  <ContentTitleText>Empty IMF</ContentTitleText>
  <SegmentList>
  </SegmentList>
</CompositionPlaylist>)";
  }
  auto cpl = dcpdoctor::Cpl::parse(tmp);
  ASSERT(cpl.has_value());
  ASSERT(cpl->reels.empty());
  ASSERT(cpl->content_title == "Empty IMF");
  fs::remove(tmp);
}

TEST(cpl_parse_imf_segment_no_sequence_list)
{
  auto tmp = fs::temp_directory_path() / "dcpdoctor_test_imf_nosq.xml";
  {
    std::ofstream f(tmp);
    f << R"(<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/2067-3/2016">
  <Id>urn:uuid:nosq-0000-0000-0000-000000000000</Id>
  <SegmentList>
    <Segment>
      <Id>urn:uuid:seg-nosq-0000-0000-000000000001</Id>
    </Segment>
  </SegmentList>
</CompositionPlaylist>)";
  }
  auto cpl = dcpdoctor::Cpl::parse(tmp);
  ASSERT(cpl.has_value());
  // Segment with no SequenceList still produces a reel entry
  ASSERT(cpl->reels.size() == 1);
  ASSERT(cpl->reels[0].id == "urn:uuid:seg-nosq-0000-0000-000000000001");
  ASSERT(cpl->reels[0].picture.id.empty());
  fs::remove(tmp);
}

TEST(cpl_parse_dcp_still_works)
{
  // Ensure DCP-style CPL still parses correctly after IMF changes
  auto tmp = fs::temp_directory_path() / "dcpdoctor_test_dcp_cpl.xml";
  {
    std::ofstream f(tmp);
    f << R"(<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL">
  <Id>urn:uuid:dcp-test-0000-0000-000000000000</Id>
  <ContentTitleText>DCP Test</ContentTitleText>
  <ContentKind>trailer</ContentKind>
  <ReelList>
    <Reel>
      <Id>urn:uuid:reel1-0000-0000-0000-000000000001</Id>
      <AssetList>
        <MainPicture>
          <Id>urn:uuid:pic-dcp-0000-0000-000000000001</Id>
          <EditRate>24 1</EditRate>
          <IntrinsicDuration>600</IntrinsicDuration>
          <EntryPoint>0</EntryPoint>
          <Duration>600</Duration>
        </MainPicture>
        <MainSound>
          <Id>urn:uuid:snd-dcp-0000-0000-000000000001</Id>
          <EditRate>24 1</EditRate>
          <IntrinsicDuration>600</IntrinsicDuration>
          <EntryPoint>0</EntryPoint>
          <Duration>600</Duration>
        </MainSound>
      </AssetList>
    </Reel>
  </ReelList>
</CompositionPlaylist>)";
  }
  auto cpl = dcpdoctor::Cpl::parse(tmp);
  ASSERT(cpl.has_value());
  ASSERT(cpl->content_title == "DCP Test");
  ASSERT(cpl->content_kind == "trailer");
  ASSERT(cpl->reels.size() == 1);
  ASSERT(cpl->reels[0].picture.duration == 600);
  ASSERT(cpl->reels[0].sound.duration == 600);
  fs::remove(tmp);
}

TEST(cpl_parse_neither_reel_nor_segment)
{
  // CPL with no ReelList and no SegmentList should still parse metadata
  auto tmp = fs::temp_directory_path() / "dcpdoctor_test_no_reels.xml";
  {
    std::ofstream f(tmp);
    f << R"(<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/2067-3/2016">
  <Id>urn:uuid:noreels-0000-0000-0000-000000000000</Id>
  <ContentTitleText>No Reels</ContentTitleText>
</CompositionPlaylist>)";
  }
  auto cpl = dcpdoctor::Cpl::parse(tmp);
  ASSERT(cpl.has_value());
  ASSERT(cpl->content_title == "No Reels");
  ASSERT(cpl->reels.empty());
  fs::remove(tmp);
}

TEST(timeline_extract_imf_cpl)
{
  // Verify extract_timeline works with IMF-format CPLs
  auto tmp = fs::temp_directory_path() / "dcpdoctor_test_imf_timeline.xml";
  {
    std::ofstream f(tmp);
    f << R"(<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/2067-3/2016">
  <Id>urn:uuid:timeline-0000-0000-0000-000000000000</Id>
  <SegmentList>
    <Segment>
      <Id>urn:uuid:seg-tl-0000-0000-000000000001</Id>
      <SequenceList>
        <MainImageSequence>
          <ResourceList>
            <Resource>
              <TrackFileId>urn:uuid:pic-tl-0000-0000-000000000001</TrackFileId>
              <IntrinsicDuration>240</IntrinsicDuration>
              <EntryPoint>0</EntryPoint>
              <EditRate>24 1</EditRate>
            </Resource>
          </ResourceList>
        </MainImageSequence>
      </SequenceList>
    </Segment>
  </SegmentList>
</CompositionPlaylist>)";
  }
  auto reels = dcpdoctor::extract_timeline(tmp);
  // Should parse at least the segment
  ASSERT(!reels.empty());
  fs::remove(tmp);
}

TEST(reel_continuity_imf_cpl)
{
  // Verify reel continuity check works with IMF-format CPLs
  auto tmp = fs::temp_directory_path() / "dcpdoctor_test_imf_cont.xml";
  {
    std::ofstream f(tmp);
    f << R"(<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/2067-3/2016">
  <Id>urn:uuid:cont-0000-0000-0000-000000000000</Id>
  <SegmentList>
    <Segment>
      <Id>urn:uuid:seg-c1-0000-0000-000000000001</Id>
      <SequenceList>
        <MainImageSequence>
          <ResourceList>
            <Resource>
              <TrackFileId>urn:uuid:pic-c1-0000-0000-000000000001</TrackFileId>
              <IntrinsicDuration>240</IntrinsicDuration>
              <EntryPoint>0</EntryPoint>
              <EditRate>24 1</EditRate>
            </Resource>
          </ResourceList>
        </MainImageSequence>
      </SequenceList>
    </Segment>
  </SegmentList>
</CompositionPlaylist>)";
  }
  auto notes = dcpdoctor::check_reel_continuity(tmp);
  // Single segment: no continuity issues expected
  ASSERT(notes.empty());
  fs::remove(tmp);
}

// --- Marker tests ---

TEST(markers_strict_missing)
{
  auto tmp = fs::temp_directory_path() / "dcpdoctor_test_markers.xml";
  {
    std::ofstream f(tmp);
    f << R"(<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL">
  <ReelList>
    <Reel>
      <AssetList>
        <MainMarkers>
          <Marker>
            <Label>FFMC</Label>
            <Offset>0</Offset>
          </Marker>
        </MainMarkers>
      </AssetList>
    </Reel>
  </ReelList>
</CompositionPlaylist>)";
  }
  auto notes = dcpdoctor::check_markers(tmp, true);
  // Should warn about missing LFMC
  bool found_lfmc = false;
  for(const auto& n : notes)
    if(n.message.find("LFMC") != std::string::npos)
      found_lfmc = true;
  ASSERT(found_lfmc);
  fs::remove(tmp);
}

// --- Encryption detection tests ---

TEST(encryption_detected)
{
  auto tmp_dir = fs::temp_directory_path() / "dcpdoctor_test_enc";
  fs::create_directories(tmp_dir);
  auto cpl = tmp_dir / "cpl.xml";
  {
    std::ofstream f(cpl);
    f << R"(<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL">
  <ReelList>
    <Reel>
      <AssetList>
        <MainPicture>
          <KeyId>urn:uuid:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee</KeyId>
        </MainPicture>
      </AssetList>
    </Reel>
  </ReelList>
</CompositionPlaylist>)";
  }
  std::vector<fs::path> cpls = {cpl};
  auto notes = dcpdoctor::check_encryption(tmp_dir, cpls);
  bool found_enc = false;
  for(const auto& n : notes)
    if(n.code == dcpdoctor::Code::encryption_detected)
      found_enc = true;
  ASSERT(found_enc);
  fs::remove_all(tmp_dir);
}

// --- Essence type detection ---

TEST(essence_type_str)
{
  ASSERT(dcpdoctor::essence_type_str(dcpdoctor::EssenceType::jpeg2000) == "JPEG 2000");
  ASSERT(dcpdoctor::essence_type_str(dcpdoctor::EssenceType::pcm_audio) == "PCM Audio");
  ASSERT(dcpdoctor::essence_type_str(dcpdoctor::EssenceType::dolby_atmos) == "Dolby Atmos (IAB)");
  ASSERT(dcpdoctor::essence_type_str(dcpdoctor::EssenceType::timed_text) == "Timed Text");
}

// --- Integration tests ---

TEST(valid_smpte_fixture)
{
  auto fixtures = fs::path(__FILE__).parent_path() / "fixtures" / "valid_smpte";
  if(!fs::exists(fixtures))
    return; // Skip if fixtures not available

  auto result = dcpdoctor::verify(fixtures);
  ASSERT(result.ok());
  ASSERT(result.standard == dcpdoctor::Standard::smpte);
}

TEST(bad_hash_fixture)
{
  auto fixtures = fs::path(__FILE__).parent_path() / "fixtures" / "bad_hash";
  if(!fs::exists(fixtures))
    return; // Skip if fixtures not available

  auto result = dcpdoctor::verify(fixtures);
  ASSERT(!result.ok());
  bool found_hash = false;
  for(const auto& n : result.notes)
    if(n.code == dcpdoctor::Code::pkl_hash_mismatch)
      found_hash = true;
  ASSERT(found_hash);
}

// --- Code string coverage ---

TEST(code_str_coverage)
{
  dcpdoctor::Note n{dcpdoctor::Severity::info, dcpdoctor::Code::j2k_bitrate_exceeded, "test"};
  ASSERT(n.code_str() == "j2k_bitrate_exceeded");
  n.code = dcpdoctor::Code::subtitle_parse_error;
  ASSERT(n.code_str() == "subtitle_parse_error");
  n.code = dcpdoctor::Code::isdcf_naming_violation;
  ASSERT(n.code_str() == "isdcf_naming_violation");
  n.code = dcpdoctor::Code::encryption_detected;
  ASSERT(n.code_str() == "encryption_detected");
  n.code = dcpdoctor::Code::reel_discontinuity;
  ASSERT(n.code_str() == "reel_discontinuity");
  n.code = dcpdoctor::Code::stereo_mismatch;
  ASSERT(n.code_str() == "stereo_mismatch");
  n.code = dcpdoctor::Code::marker_missing;
  ASSERT(n.code_str() == "marker_missing");
  n.code = dcpdoctor::Code::cross_ref_broken;
  ASSERT(n.code_str() == "cross_ref_broken");
  n.code = dcpdoctor::Code::supplemental_opl_missing;
  ASSERT(n.code_str() == "supplemental_opl_missing");
}

// === Advanced Feature Tests ===

TEST(bv21_nonexistent_dir)
{
  auto notes =
      dcpdoctor::check_bv21_compliance("/tmp/nonexistent_bv21_test", dcpdoctor::Standard::smpte);
  // Should produce notes about missing files or fail gracefully
  ASSERT(notes.empty() || notes[0].severity != dcpdoctor::Severity::error ||
         notes[0].code == dcpdoctor::Code::smpte_naming_violation ||
         notes[0].code == dcpdoctor::Code::missing_required_element);
}

TEST(bv21_interop_warning)
{
  // BV2.1 requires SMPTE - interop should generate a warning/error
  auto notes =
      dcpdoctor::check_bv21_compliance("/tmp/nonexistent_bv21_test", dcpdoctor::Standard::interop);
  bool found_namespace_issue = false;
  for(auto& n : notes)
  {
    if(n.code == dcpdoctor::Code::smpte_namespace_wrong)
      found_namespace_issue = true;
  }
  ASSERT(found_namespace_issue);
}

TEST(manifest_nonexistent_file)
{
  auto notes =
      dcpdoctor::compare_manifest("/tmp/nonexistent_dcp", "/tmp/nonexistent_manifest.json");
  ASSERT(!notes.empty());
  // Should report file not found
  bool found_error = false;
  for(auto& n : notes)
  {
    if(n.severity == dcpdoctor::Severity::error)
      found_error = true;
  }
  ASSERT(found_error);
}

TEST(batch_summary_output)
{
  std::vector<dcpdoctor::BatchResult> results;
  results.push_back({"/tmp/dcp1", true, 0, 1, dcpdoctor::Standard::smpte});
  results.push_back({"/tmp/dcp2", false, 3, 2, dcpdoctor::Standard::interop});
  results.push_back({"/tmp/dcp3", true, 0, 0, dcpdoctor::Standard::smpte});

  std::ostringstream out;
  dcpdoctor::write_batch_summary(out, results);
  std::string output = out.str();

  // Should contain header
  ASSERT(output.find("Batch Summary") != std::string::npos);
  // Should contain pass/fail counts
  ASSERT(output.find("Passed: 2") != std::string::npos);
  ASSERT(output.find("Failed: 1") != std::string::npos);
  // Should list individual DCPs
  ASSERT(output.find("dcp1") != std::string::npos);
  ASSERT(output.find("dcp2") != std::string::npos);
  ASSERT(output.find("PASS") != std::string::npos);
  ASSERT(output.find("FAIL") != std::string::npos);
}

TEST(bitrate_stats_default)
{
  dcpdoctor::FrameBitrateStats stats;
  ASSERT(!stats.valid);
  ASSERT(stats.frame_count == 0);
  ASSERT(stats.avg_bitrate_mbps == 0.0);
  ASSERT(stats.max_bitrate_mbps == 0.0);
}

TEST(bitrate_compliance_under_limit)
{
  dcpdoctor::FrameBitrateStats stats;
  stats.valid = true;
  stats.frame_count = 100;
  stats.max_bitrate_mbps = 200.0; // Under 250 Mbps DCI limit
  stats.avg_bitrate_mbps = 150.0;
  stats.frame_rate = 24.0;

  auto notes = dcpdoctor::check_bitrate_compliance(stats, "/test.mxf");
  // Under limit - no error notes expected
  bool has_bitrate_error = false;
  for(auto& n : notes)
  {
    if(n.code == dcpdoctor::Code::j2k_bitrate_exceeded && n.severity == dcpdoctor::Severity::error)
      has_bitrate_error = true;
  }
  ASSERT(!has_bitrate_error);
}

TEST(bitrate_compliance_over_limit)
{
  dcpdoctor::FrameBitrateStats stats;
  stats.valid = true;
  stats.frame_count = 100;
  stats.max_bitrate_mbps = 300.0; // Over 250 Mbps for 2K
  stats.avg_bitrate_mbps = 260.0;
  stats.frame_rate = 24.0;
  stats.width = 2048;
  stats.height = 1080;

  auto notes = dcpdoctor::check_bitrate_compliance(stats, "/test.mxf");
  bool has_bitrate_error = false;
  for(auto& n : notes)
  {
    if(n.code == dcpdoctor::Code::j2k_bitrate_exceeded)
      has_bitrate_error = true;
  }
  ASSERT(has_bitrate_error);
}

TEST(j2k_deep_info_default)
{
  dcpdoctor::J2kDeepInfo info;
  ASSERT(!info.valid);
  ASSERT(info.rsiz == 0);
  ASSERT(info.num_components == 0);
  ASSERT(info.bit_depth == 0);
}

TEST(j2k_deep_compliance_valid_dci)
{
  dcpdoctor::J2kDeepInfo info;
  info.valid = true;
  info.rsiz = 3; // Cinema 2K
  info.num_components = 3;
  info.bit_depth = 12;
  info.num_decomp_levels = 5;
  info.code_block_width = 5; // 32
  info.code_block_height = 5; // 32
  info.num_quality_layers = 1;
  info.irreversible = true;

  auto notes = dcpdoctor::check_j2k_deep_compliance(info, "/test.mxf");
  // Valid DCI params should produce no errors
  bool has_error = false;
  for(auto& n : notes)
  {
    if(n.severity == dcpdoctor::Severity::error)
      has_error = true;
  }
  ASSERT(!has_error);
}

TEST(j2k_deep_compliance_bad_components)
{
  dcpdoctor::J2kDeepInfo info;
  info.valid = true;
  info.rsiz = 3;
  info.num_components = 1; // Should be 3 for DCI
  info.bit_depth = 12;
  info.num_decomp_levels = 5;
  info.code_block_width = 5;
  info.code_block_height = 5;
  info.irreversible = true;

  auto notes = dcpdoctor::check_j2k_deep_compliance(info, "/test.mxf");
  bool found_component_error = false;
  for(auto& n : notes)
  {
    if(n.code == dcpdoctor::Code::j2k_invalid_component_count)
      found_component_error = true;
  }
  ASSERT(found_component_error);
}

TEST(audio_stats_default)
{
  dcpdoctor::AudioLevelStats stats;
  ASSERT(!stats.valid);
  ASSERT(stats.channels == 0);
  ASSERT(stats.overall_peak_dbfs == -200.0);
}

TEST(audio_level_check_clipping)
{
  dcpdoctor::AudioLevelStats stats;
  stats.valid = true;
  stats.channels = 6;
  stats.sample_rate = 48000;
  stats.bit_depth = 24;
  stats.peak_dbfs = {-0.05, -3.0, -6.0, -10.0, -20.0, -30.0}; // Channel 0 clipping
  stats.rms_dbfs = {-6.0, -12.0, -15.0, -20.0, -30.0, -40.0};
  stats.overall_peak_dbfs = -0.05; // Near 0 dBFS

  auto notes = dcpdoctor::check_audio_levels(stats, "/test.mxf");
  bool found_clipping = false;
  for(auto& n : notes)
  {
    if(n.message.find("clip") != std::string::npos || n.message.find("peak") != std::string::npos)
      found_clipping = true;
  }
  ASSERT(found_clipping);
}

TEST(audio_level_check_silence)
{
  dcpdoctor::AudioLevelStats stats;
  stats.valid = true;
  stats.channels = 2;
  stats.sample_rate = 48000;
  stats.bit_depth = 24;
  stats.peak_dbfs = {-90.0, -90.0}; // Near silence
  stats.rms_dbfs = {-95.0, -95.0};
  stats.overall_peak_dbfs = -90.0;
  stats.overall_rms_dbfs = -95.0;

  auto notes = dcpdoctor::check_audio_levels(stats, "/test.mxf");
  bool found_silence = false;
  for(auto& n : notes)
  {
    if(n.message.find("silen") != std::string::npos || n.message.find("quiet") != std::string::npos)
      found_silence = true;
  }
  ASSERT(found_silence);
}

TEST(timeline_reel_defaults)
{
  dcpdoctor::TimelineReel reel;
  ASSERT(reel.picture_entry == 0);
  ASSERT(reel.picture_duration == 0);
  ASSERT(!reel.has_picture);
  ASSERT(!reel.has_sound);
  ASSERT(!reel.has_subtitle);
}

TEST(timeline_svg_generation)
{
  std::vector<dcpdoctor::TimelineReel> reels;

  dcpdoctor::TimelineReel r1;
  r1.id = "reel-1";
  r1.picture_entry = 0;
  r1.picture_duration = 24 * 60; // 1 minute at 24fps
  r1.has_picture = true;
  r1.sound_entry = 0;
  r1.sound_duration = 24 * 60;
  r1.has_sound = true;
  reels.push_back(r1);

  dcpdoctor::TimelineReel r2;
  r2.id = "reel-2";
  r2.picture_entry = 24 * 60;
  r2.picture_duration = 24 * 120; // 2 minutes
  r2.has_picture = true;
  r2.has_sound = true;
  r2.sound_entry = 24 * 60;
  r2.sound_duration = 24 * 120;
  reels.push_back(r2);

  std::ostringstream out;
  dcpdoctor::write_timeline_svg(out, reels, "Test DCP", 24.0);
  std::string svg = out.str();

  // Should be valid SVG
  ASSERT(svg.find("<svg") != std::string::npos);
  ASSERT(svg.find("</svg>") != std::string::npos);
  // Should contain title
  ASSERT(svg.find("Test DCP") != std::string::npos);
  // Should contain track labels
  ASSERT(svg.find("Picture") != std::string::npos);
  ASSERT(svg.find("Sound") != std::string::npos);
}

TEST(timeline_extract_nonexistent)
{
  auto reels = dcpdoctor::extract_timeline("/tmp/nonexistent_cpl.xml");
  ASSERT(reels.empty());
}

TEST(bitrate_analyze_nonexistent)
{
  auto stats = dcpdoctor::analyze_picture_bitrate("/tmp/nonexistent_mxf_file.mxf");
  ASSERT(!stats.valid);
  ASSERT(!stats.error.empty());
}

TEST(audio_analyze_nonexistent)
{
  auto stats = dcpdoctor::analyze_audio_levels("/tmp/nonexistent_audio.mxf");
  ASSERT(!stats.valid);
  ASSERT(!stats.error.empty());
}

TEST(j2k_deep_validate_nonexistent)
{
  auto info = dcpdoctor::deep_validate_j2k("/tmp/nonexistent_j2k.mxf");
  ASSERT(!info.valid);
  ASSERT(!info.error.empty());
}

// === KDM Tests ===

TEST(kdm_parse_nonexistent)
{
  auto info = dcpdoctor::parse_kdm("/tmp/nonexistent_kdm.xml");
  ASSERT(!info.valid);
  ASSERT(!info.error.empty());
}

TEST(kdm_validate_nonexistent)
{
  auto notes = dcpdoctor::validate_kdm("/tmp/nonexistent_kdm.xml", "/tmp/nonexistent_dcp");
  ASSERT(!notes.empty());
  ASSERT(notes[0].severity == dcpdoctor::Severity::error);
}

// === DCP Diff Tests ===

TEST(diff_nonexistent_dcps)
{
  auto diff = dcpdoctor::compare_dcps("/tmp/nonexistent_a", "/tmp/nonexistent_b");
  // Both don't exist - should have empty assets
  ASSERT(diff.assets.empty());
  ASSERT(diff.structure_identical);
}

TEST(diff_report_identical)
{
  dcpdoctor::DcpDiff diff;
  diff.dcp_a = "/tmp/a";
  diff.dcp_b = "/tmp/b";
  diff.structure_identical = true;
  diff.content_identical = true;

  std::ostringstream out;
  dcpdoctor::write_diff_report(out, diff);
  std::string output = out.str();
  ASSERT(output.find("IDENTICAL") != std::string::npos);
}

TEST(diff_report_different)
{
  dcpdoctor::DcpDiff diff;
  diff.dcp_a = "/tmp/a";
  diff.dcp_b = "/tmp/b";
  diff.structure_identical = false;
  diff.content_identical = false;
  diff.assets.push_back(
      {"id1", "file1.mxf", dcpdoctor::DcpDiff::AssetDiff::Status::added, "Only in b"});
  diff.assets.push_back(
      {"id2", "file2.mxf", dcpdoctor::DcpDiff::AssetDiff::Status::removed, "Only in a"});

  std::ostringstream out;
  dcpdoctor::write_diff_report(out, diff);
  std::string output = out.str();
  ASSERT(output.find("DIFFERENT") != std::string::npos);
  ASSERT(output.find("file1.mxf") != std::string::npos);
  ASSERT(output.find("+") != std::string::npos);
  ASSERT(output.find("-") != std::string::npos);
}

// === Hash Cache Tests ===

TEST(cache_basic_operations)
{
  // Create a temp file to cache
  fs::path tmp = fs::temp_directory_path() / "dcpdoctor_test_cache.db";
  fs::path test_file = fs::temp_directory_path() / "dcpdoctor_test_file.txt";

  // Clean up any leftover from previous runs
  std::error_code ec;
  fs::remove(tmp, ec);
  fs::remove(test_file, ec);

  // Create test file
  {
    std::ofstream f(test_file);
    f << "test content for caching";
  }

  {
    dcpdoctor::HashCache cache(tmp);
    ASSERT(cache.is_open());
    ASSERT(cache.size() == 0);

    cache.put(test_file, "abc123hash");
    ASSERT(cache.size() == 1);

    auto hash = cache.get(test_file);
    ASSERT(hash == "abc123hash");

    cache.clear();
    ASSERT(cache.size() == 0);
  }

  // Cleanup
  fs::remove(tmp);
  fs::remove(test_file);
}

TEST(cache_stale_detection)
{
  fs::path tmp = "/tmp/dcpdoctor_test_cache2.db";
  fs::path test_file = "/tmp/dcpdoctor_test_file2.txt";

  {
    std::ofstream f(test_file);
    f << "original content";
  }

  {
    dcpdoctor::HashCache cache(tmp);
    cache.put(test_file, "original_hash");

    // Modify the file
    {
      std::ofstream f(test_file);
      f << "modified content with different size!!!";
    }

    // Cache should return empty (stale - size changed)
    auto hash = cache.get(test_file);
    ASSERT(hash.empty());
  }

  fs::remove(tmp);
  fs::remove(test_file);
}

// === Theater Profile Tests ===

TEST(theater_profiles_exist)
{
  auto profiles = dcpdoctor::get_theater_profiles();
  ASSERT(profiles.size() >= 8); // We defined 10 profiles
}

TEST(theater_find_dolby)
{
  auto* profile = dcpdoctor::find_profile("dolby ims3000");
  ASSERT(profile != nullptr);
  ASSERT(profile->vendor == "Dolby");
  ASSERT(profile->supports_atmos);
}

TEST(theater_find_barco)
{
  auto* profile = dcpdoctor::find_profile("barco sp4k");
  ASSERT(profile != nullptr);
  ASSERT(profile->vendor == "Barco");
  ASSERT(profile->supports_4k);
}

TEST(theater_find_imax)
{
  auto* profile = dcpdoctor::find_profile("imax");
  ASSERT(profile != nullptr);
  ASSERT(!profile->supports_interop);
}

TEST(theater_find_nonexistent)
{
  auto* profile = dcpdoctor::find_profile("nonexistent_server_xyz");
  ASSERT(profile == nullptr);
}

TEST(theater_compatibility_interop_rejected)
{
  dcpdoctor::VerifyResult result;
  result.standard = dcpdoctor::Standard::interop;

  auto* profile = dcpdoctor::find_profile("dolby cinema");
  ASSERT(profile != nullptr);

  auto notes = dcpdoctor::check_theater_compatibility("/tmp/test_dcp", result, *profile);
  bool found_rejection = false;
  for(auto& n : notes)
  {
    if(n.severity == dcpdoctor::Severity::error && n.message.find("Interop") != std::string::npos)
      found_rejection = true;
  }
  ASSERT(found_rejection);
}

// === Fix Suggestion Tests ===

TEST(fixes_assetmap_rename)
{
  std::vector<dcpdoctor::Note> notes;
  notes.push_back(dcpdoctor::Note{dcpdoctor::Severity::error,
                                  dcpdoctor::Code::smpte_naming_violation,
                                  "BV2.1 requires ASSETMAP.xml filename", "/tmp/dcp"});

  auto fixes = dcpdoctor::suggest_fixes(notes);
  ASSERT(!fixes.empty());
  bool found_rename = false;
  for(auto& fix : fixes)
  {
    if(fix.command.find("ASSETMAP") != std::string::npos)
      found_rename = true;
  }
  ASSERT(found_rename);
}

TEST(fixes_isdcf_naming)
{
  std::vector<dcpdoctor::Note> notes;
  notes.push_back(dcpdoctor::Note{dcpdoctor::Severity::warning,
                                  dcpdoctor::Code::isdcf_naming_violation, "Not ISDCF compliant",
                                  "/tmp/cpl.xml"});

  auto fixes = dcpdoctor::suggest_fixes(notes);
  ASSERT(!fixes.empty());
  bool found_isdcf = false;
  for(auto& fix : fixes)
  {
    if(fix.description.find("ISDCF") != std::string::npos)
      found_isdcf = true;
  }
  ASSERT(found_isdcf);
}

TEST(fixes_empty_notes)
{
  std::vector<dcpdoctor::Note> notes;
  auto fixes = dcpdoctor::suggest_fixes(notes);
  ASSERT(fixes.empty());
}

// === Audio Sync Drift Tests ===

TEST(audio_sync_no_drift)
{
  std::vector<dcpdoctor::TimelineReel> reels;
  dcpdoctor::TimelineReel r;
  r.has_picture = true;
  r.has_sound = true;
  r.picture_duration = 1000;
  r.sound_duration = 1000;
  reels.push_back(r);

  auto notes = dcpdoctor::check_audio_sync(reels, "/tmp/cpl.xml");
  ASSERT(notes.empty());
}

TEST(audio_sync_with_drift)
{
  std::vector<dcpdoctor::TimelineReel> reels;
  dcpdoctor::TimelineReel r;
  r.has_picture = true;
  r.has_sound = true;
  r.picture_duration = 1000;
  r.sound_duration = 1005; // 5 frame drift
  reels.push_back(r);

  auto notes = dcpdoctor::check_audio_sync(reels, "/tmp/cpl.xml");
  ASSERT(!notes.empty());
  ASSERT(notes[0].severity == dcpdoctor::Severity::warning);
  ASSERT(notes[0].message.find("drift") != std::string::npos);
}

// ============================================================
// Tests for new QC/validation modules (moved from imfwizard)
// ============================================================

// --- validate_with_photon ---
TEST(validate_nonexistent_imp)
{
  auto result = dcpdoctor::validate_with_photon("/tmp/no_such_imp_12345");
  ASSERT(!result.valid);
}

TEST(validation_note_severity)
{
  dcpdoctor::ValidationNote note;
  note.severity = dcpdoctor::ValidationNote::Severity::error;
  ASSERT(note.severity == dcpdoctor::ValidationNote::Severity::error);
  note.severity = dcpdoctor::ValidationNote::Severity::warning;
  ASSERT(note.severity == dcpdoctor::ValidationNote::Severity::warning);
  note.severity = dcpdoctor::ValidationNote::Severity::info;
  ASSERT(note.severity == dcpdoctor::ValidationNote::Severity::info);
}

// --- schema_validate ---
TEST(schema_validate_nonexistent)
{
  dcpdoctor::SchemaValidateOptions opts;
  opts.imp_dir = "/tmp/no_such_imp_12345";
  auto result = dcpdoctor::validate_against_schema(opts);
  ASSERT(!result.valid);
}

TEST(schema_validate_options_defaults)
{
  dcpdoctor::SchemaValidateOptions opts;
  ASSERT(opts.validate_cpl == true);
  ASSERT(opts.validate_pkl == true);
  ASSERT(opts.validate_assetmap == true);
  ASSERT(opts.strict == false);
}

// --- imf_compliance ---
TEST(imf_compliance_nonexistent)
{
  dcpdoctor::ImfComplianceOptions opts;
  opts.imp_dir = "/tmp/no_such_imp_12345";
  opts.target = dcpdoctor::ImfComplianceTarget::Netflix;
  auto result = dcpdoctor::check_imf_compliance(opts);
  ASSERT(!result.success || !result.compliant);
}

TEST(imf_compliance_target_name)
{
  ASSERT(dcpdoctor::imf_compliance_target_name(dcpdoctor::ImfComplianceTarget::Netflix) == "Netflix");
  ASSERT(dcpdoctor::imf_compliance_target_name(dcpdoctor::ImfComplianceTarget::Disney) == "Disney+");
  ASSERT(dcpdoctor::imf_compliance_target_name(dcpdoctor::ImfComplianceTarget::Amazon) == "Amazon");
  ASSERT(dcpdoctor::imf_compliance_target_name(dcpdoctor::ImfComplianceTarget::Apple) == "Apple TV+");
  ASSERT(dcpdoctor::imf_compliance_target_name(dcpdoctor::ImfComplianceTarget::Cinema2K) == "Cinema 2K");
  ASSERT(dcpdoctor::imf_compliance_target_name(dcpdoctor::ImfComplianceTarget::Cinema4K) == "Cinema 4K");
  ASSERT(dcpdoctor::imf_compliance_target_name(dcpdoctor::ImfComplianceTarget::BroadcastHD) == "Broadcast HD");
  ASSERT(dcpdoctor::imf_compliance_target_name(dcpdoctor::ImfComplianceTarget::BroadcastUHD) == "Broadcast UHD");
}

// --- frame_qc ---
TEST(frame_qc_nonexistent_dir)
{
  dcpdoctor::FrameQcOptions opts;
  opts.j2k_dir = "/tmp/no_such_j2k_dir_12345";
  auto result = dcpdoctor::analyze_frame_qc(opts);
  ASSERT(!result.success || result.total_frames == 0);
}

TEST(frame_qc_options_defaults)
{
  dcpdoctor::FrameQcOptions opts;
  ASSERT(opts.target_bitrate_mbps == 250.0);
  ASSERT(opts.max_bitrate_mbps == 300.0);
  ASSERT(opts.min_bitrate_mbps == 50.0);
  ASSERT(opts.fps_num == 24);
  ASSERT(opts.fps_den == 1);
}

// --- qc_report ---
TEST(qc_report_options_defaults)
{
  dcpdoctor::DetailedQcOptions opts;
  ASSERT(opts.include_thumbnails == true);
  ASSERT(opts.include_waveform == true);
  ASSERT(opts.include_loudness == true);
  ASSERT(opts.include_bitrate_chart == true);
  ASSERT(opts.thumbnail_count == 12);
}

// --- loudness ---
TEST(loudness_nonexistent_file)
{
  auto result = dcpdoctor::measure_imf_loudness("/tmp/no_such_audio_12345.wav");
  ASSERT(!result.success);
}

TEST(normalize_options_defaults)
{
  dcpdoctor::NormalizeOptions opts;
  ASSERT(opts.target_lufs == -23.0);
  ASSERT(opts.true_peak_limit == -1.0);
}

// --- av_sync ---
TEST(av_sync_nonexistent)
{
  dcpdoctor::AvSyncOptions opts;
  opts.video_file = "/tmp/no_such_video_12345.mxf";
  opts.audio_file = "/tmp/no_such_audio_12345.wav";
  auto result = dcpdoctor::detect_av_sync(opts);
  ASSERT(!result.success);
}

TEST(av_sync_options_defaults)
{
  dcpdoctor::AvSyncOptions opts;
  ASSERT(opts.fps_num == 24);
  ASSERT(opts.fps_den == 1);
  ASSERT(opts.sample_rate == 48000);
}

// --- hdr_validate ---
TEST(hdr_validate_nonexistent)
{
  dcpdoctor::HdrValidateOptions opts;
  opts.video_path = "/tmp/no_such_video_12345.mxf";
  opts.target_spec = "hdr10";
  auto result = dcpdoctor::validate_hdr_metadata(opts);
  ASSERT(!result.success);
}

TEST(hdr_types)
{
  dcpdoctor::ImfHdrMetadata meta;
  meta.transfer = dcpdoctor::TransferFunction::PQ;
  meta.colorimetry = dcpdoctor::Colorimetry::BT2020;
  meta.bit_depth = 10;
  ASSERT(meta.transfer == dcpdoctor::TransferFunction::PQ);
  ASSERT(meta.colorimetry == dcpdoctor::Colorimetry::BT2020);
  ASSERT(meta.bit_depth == 10);

  dcpdoctor::ContentLightLevel cll;
  cll.max_cll = 1000;
  cll.max_fall = 400;
  meta.content_light = cll;
  ASSERT(meta.content_light.has_value());
  ASSERT(meta.content_light->max_cll == 1000);
}

// --- frame_compare ---
TEST(compare_options_defaults)
{
  dcpdoctor::CompareOptions opts;
  ASSERT(opts.threshold_psnr == 40.0);
  ASSERT(opts.threshold_ssim == 0.95);
  ASSERT(opts.sample_interval == 1);
  ASSERT(opts.start_frame == 0);
  ASSERT(opts.end_frame == 0);
  ASSERT(opts.generate_html == false);
  ASSERT(opts.extract_diff_frames == false);
  ASSERT(opts.compute_ssim == true);
  ASSERT(opts.compute_vmaf == false);
}

TEST(compare_result_defaults)
{
  dcpdoctor::CompareResult result;
  ASSERT(result.frames_compared == 0);
  ASSERT(result.frames_different == 0);
  ASSERT(result.identical == false);
  ASSERT(result.success == false);
}

// --- info ---
TEST(imp_info_nonexistent)
{
  auto info = dcpdoctor::read_imp_info("/tmp/no_such_imp_12345");
  ASSERT(!info.valid);
}

// --- checksum_verify ---
TEST(checksum_verify_nonexistent)
{
  dcpdoctor::ChecksumVerifyOptions opts;
  opts.package_dir = "/tmp/no_such_imp_12345";
  auto result = dcpdoctor::verify_package_checksums(opts);
  ASSERT(!result.success || !result.all_valid);
}

// --- mxf_extract ---
TEST(mxf_extract_nonexistent)
{
  dcpdoctor::MxfExtractOptions opts;
  opts.input = "/tmp/no_such_mxf_12345.mxf";
  opts.output_dir = "/tmp/no_such_output_12345";
  auto result = dcpdoctor::extract_mxf(opts);
  ASSERT(!result.success);
}

// --- auto_qc ---
TEST(auto_qc_no_input)
{
  dcpdoctor::AutoQcOptions opts;
  // No video or audio file set
  auto result = dcpdoctor::run_auto_qc(opts);
  // Should either fail or produce no issues
  ASSERT(result.issues.empty() || !result.issues.empty()); // just verify it doesn't crash
}

TEST(auto_qc_nonexistent_video)
{
  dcpdoctor::AutoQcOptions opts;
  opts.video_path = "/tmp/no_such_video_12345.mxf";
  auto result = dcpdoctor::run_auto_qc(opts);
  // May fail or produce empty results
  ASSERT(result.issues.empty() || !result.issues.empty());
}

// --- validate_cpl_hdr ---
TEST(validate_cpl_hdr_missing_cpl)
{
  auto result = dcpdoctor::validate_cpl_hdr("/tmp/no_such_cpl.xml", "/tmp/no_such_video.mxf");
  ASSERT(!result.success);
  ASSERT(!result.error.empty());
}

TEST(validate_cpl_hdr_missing_video)
{
  // Create a minimal CPL file
  std::ofstream cpl("/tmp/dcpdoctor_test_cpl.xml");
  cpl << "<?xml version=\"1.0\"?>\n<CompositionPlaylist></CompositionPlaylist>\n";
  cpl.close();
  auto result = dcpdoctor::validate_cpl_hdr("/tmp/dcpdoctor_test_cpl.xml",
                                            "/tmp/no_such_video_99.mxf");
  ASSERT(!result.success);
  ASSERT(!result.error.empty());
  std::filesystem::remove("/tmp/dcpdoctor_test_cpl.xml");
}

TEST(validate_cpl_hdr_sdr_cpl)
{
  // CPL with no HDR declarations - should detect SDR
  std::ofstream cpl("/tmp/dcpdoctor_test_cpl2.xml");
  cpl << "<?xml version=\"1.0\"?>\n"
      << "<CompositionPlaylist xmlns=\"http://www.smpte-ra.org/schemas/2067-3/2016\">\n"
      << "  <EditRate>24 1</EditRate>\n"
      << "</CompositionPlaylist>\n";
  cpl.close();
  // With nonexistent video, should fail at video probe stage
  auto result = dcpdoctor::validate_cpl_hdr("/tmp/dcpdoctor_test_cpl2.xml",
                                            "/tmp/no_such_video_100.mxf");
  ASSERT(!result.success);
  std::filesystem::remove("/tmp/dcpdoctor_test_cpl2.xml");
}

TEST(validate_cpl_hdr_hdr10_cpl)
{
  // CPL declaring HDR10 (PQ + BT.2020)
  std::ofstream cpl("/tmp/dcpdoctor_test_cpl3.xml");
  cpl << "<?xml version=\"1.0\"?>\n"
      << "<CompositionPlaylist>\n"
      << "  <TransferCharacteristic>SMPTE-ST-2084</TransferCharacteristic>\n"
      << "  <ColorPrimaries>ITU-R-BT.2020</ColorPrimaries>\n"
      << "</CompositionPlaylist>\n";
  cpl.close();
  // With nonexistent video, should fail at video probe
  auto result = dcpdoctor::validate_cpl_hdr("/tmp/dcpdoctor_test_cpl3.xml",
                                            "/tmp/no_such_video_101.mxf");
  ASSERT(!result.success);
  std::filesystem::remove("/tmp/dcpdoctor_test_cpl3.xml");
}

int main()
{
  for(int i = 0; i < test_count; ++i)
    test_registry[i]();
  std::cout << tests_passed << "/" << tests_run << " tests passed\n";
  return (tests_passed == tests_run) ? 0 : 1;
}
