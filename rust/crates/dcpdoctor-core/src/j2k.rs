/// J2K codestream deep analysis.
///
/// Parses JPEG 2000 codestream headers to extract:
/// - Profile (RSIZ / capabilities)
/// - Decomposition levels
/// - Code-block size
/// - Wavelet transform type
/// - Component count and bit depth
use crate::{Code, Note};
use asdcplib::as02::jp2k::MxfReader as As02MxfReader;
use asdcplib::jp2k::{MxfReader, StereoMxfReader, StereoscopicPhase};
use dcpdoctor_parse::j2k::{
    COD, MarkerScan, QCD, SIZ, SOC, find_first_sot, main_header_segment, scan_markers, tile_parts,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// J2K codestream parameters extracted from SIZ and COD markers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct J2kCodestreamInfo {
    /// RSIZ capabilities field (profile indicator)
    pub rsiz: u16,
    /// Profile name
    pub profile: String,
    /// Image width
    pub width: u32,
    /// Image height
    pub height: u32,
    /// Number of components
    pub components: u16,
    /// Bits per component (from first component)
    pub bit_depth: u8,
    /// Number of decomposition levels
    pub decomposition_levels: u8,
    /// Code-block width exponent (actual size = 2^(exp+2))
    pub codeblock_width_exp: u8,
    /// Code-block height exponent (actual size = 2^(exp+2))
    pub codeblock_height_exp: u8,
    /// Wavelet transform: true = irreversible (9-7), false = reversible (5-3)
    pub irreversible_transform: bool,
    /// Number of quality layers
    pub layers: u16,
    /// Progression order
    pub progression_order: String,
    /// Frame size in bytes
    pub frame_bytes: u64,
}

impl J2kCodestreamInfo {
    /// Code-block dimensions in samples, from the COD exponents.
    pub fn codeblock_size(&self) -> (u32, u32) {
        (
            1u32 << (self.codeblock_width_exp + 2),
            1u32 << (self.codeblock_height_exp + 2),
        )
    }
}

/// Analyze a JPEG 2000 codestream file or extract from MXF.
///
/// For direct .j2c files, reads the codestream header.
/// For MXF files, uses ffprobe to extract J2K parameters.
pub fn analyze_j2k(path: &Path) -> Result<J2kCodestreamInfo, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "j2c" | "j2k" | "jp2" => parse_j2k_codestream(path),
        "mxf" => analyze_j2k_from_mxf(path),
        _ => Err(format!("Unsupported file type: .{ext}")),
    }
}

/// Build our codestream info from postkit's J2K header (its parser now carries
/// every SIZ/COD field we need: code-block exponents and the wavelet type
/// included) plus the frame size in bytes.
fn info_from_header(hdr: &postkit::j2k::J2kHeader, frame_bytes: u64) -> J2kCodestreamInfo {
    J2kCodestreamInfo {
        rsiz: hdr.profile,
        profile: rsiz_to_profile(hdr.profile),
        width: hdr.width,
        height: hdr.height,
        components: hdr.num_components,
        bit_depth: hdr.bit_depth,
        decomposition_levels: hdr.num_decomp_levels,
        codeblock_width_exp: hdr.codeblock_width_exp,
        codeblock_height_exp: hdr.codeblock_height_exp,
        irreversible_transform: hdr.irreversible_transform,
        layers: hdr.num_layers,
        progression_order: progression_order_name(hdr.progression_order),
        frame_bytes,
    }
}

/// Parse J2K codestream directly from a .j2c/.j2k file.
fn parse_j2k_codestream(path: &Path) -> Result<J2kCodestreamInfo, String> {
    let data = std::fs::read(path).map_err(|e| format!("Failed to read file: {e}"))?;

    if data.len() < 4 {
        return Err("File too small to be a J2K codestream".into());
    }

    let hdr = postkit::j2k::parse_j2k_header(&data)
        .ok_or("Missing SOC marker (not a valid J2K codestream)")?;

    Ok(info_from_header(&hdr, data.len() as u64))
}

fn progression_order_name(po: u8) -> String {
    match po {
        0 => "LRCP".into(),
        1 => "RLCP".into(),
        2 => "RPCL".into(),
        3 => "PCRL".into(),
        4 => "CPRL".into(),
        n => format!("Unknown({n})"),
    }
}

fn rsiz_to_profile(rsiz: u16) -> String {
    match rsiz {
        0 => "No profile (unrestricted)".into(),
        1 => "Profile 0 (DCI 2K)".into(),
        2 => "Profile 1 (DCI 4K)".into(),
        3 => "Cinema 2K".into(),
        4 => "Cinema 4K".into(),
        0x0100..=0x01FF => format!("Broadcast Profile (RSIZ=0x{rsiz:04X})"),
        _ => format!("Unknown (RSIZ=0x{rsiz:04X})"),
    }
}

/// Analyze J2K parameters from a picture MXF. Reads frame 0's real codestream
/// via postkit's asdcplib reader (AS-DCP OP-Atom, gives the true RSIZ/COD
/// fields), falling back to ffprobe for AS-02 (OP1a, IMF) essence the OP-Atom
/// reader can't open.
fn analyze_j2k_from_mxf(path: &Path) -> Result<J2kCodestreamInfo, String> {
    if let Ok(frame) = postkit::j2k::read_mxf_j2k_frame(path, 0)
        && let Some(hdr) = postkit::j2k::parse_j2k_header(&frame)
    {
        return Ok(info_from_header(&hdr, frame.len() as u64));
    }
    analyze_j2k_from_mxf_ffprobe(path)
}

/// ffprobe fallback: for AS-02/OP1a essence the asdcplib OP-Atom reader rejects.
/// ffprobe can't see the codestream markers, so only the fields it exposes
/// (dimensions, bit depth) are filled; the rest is guessed from the resolution
/// the way the DCI checks expect. `frame_bytes` stays 0: ffprobe reports the
/// container size, not a codestream length.
fn analyze_j2k_from_mxf_ffprobe(path: &Path) -> Result<J2kCodestreamInfo, String> {
    let output = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_streams",
            "-show_format",
        ])
        .arg(path)
        .output()
        .map_err(|e| format!("Failed to run ffprobe: {e}"))?;

    if !output.status.success() {
        return Err("ffprobe failed to read MXF file".into());
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Failed to parse ffprobe output: {e}"))?;

    let stream = json["streams"]
        .as_array()
        .and_then(|s| {
            s.iter()
                .find(|s| s["codec_name"].as_str() == Some("jpeg2000"))
        })
        .or_else(|| {
            json["streams"]
                .as_array()
                .and_then(|s| s.iter().find(|s| s["codec_type"].as_str() == Some("video")))
        });

    let stream = match stream {
        Some(s) => s,
        None => return Err("No video/J2K stream found in MXF".into()),
    };

    let width = stream["width"].as_u64().unwrap_or(0) as u32;
    let height = stream["height"].as_u64().unwrap_or(0) as u32;

    let bit_depth = stream["bits_per_raw_sample"]
        .as_str()
        .and_then(|s| s.parse::<u8>().ok())
        .unwrap_or(12);

    Ok(J2kCodestreamInfo {
        width,
        height,
        components: 3,
        bit_depth,
        profile: if width > 2048 {
            "Cinema 4K (from MXF)".into()
        } else {
            "Cinema 2K (from MXF)".into()
        },
        irreversible_transform: true, // DCI requires 9-7
        ..Default::default()
    })
}

/// Validate J2K codestream against DCI requirements.
pub fn validate_j2k_dci(info: &J2kCodestreamInfo) -> Vec<Note> {
    let mut notes = Vec::new();

    // DCI requires 9-7 irreversible wavelet
    if !info.irreversible_transform && info.decomposition_levels > 0 {
        notes.push(Note::error(
            Code::J2kInvalidProfile,
            "DCI requires irreversible (9-7) wavelet transform; found reversible (5-3)",
        ));
    }

    // DCI requires 5 or 6 decomposition levels
    if info.decomposition_levels > 0 && !(5..=6).contains(&info.decomposition_levels) {
        notes.push(Note::warning(
            Code::J2kInvalidProfile,
            format!(
                "DCI typically requires 5-6 decomposition levels; found {}",
                info.decomposition_levels
            ),
        ));
    }

    // DCI requires code-block size 32x32 or 64x64 (exp = 3 or 4 → size = 2^(exp+2))
    if info.codeblock_width_exp > 0 {
        let (cb_w, cb_h) = info.codeblock_size();
        if !matches!((cb_w, cb_h), (32, 32) | (64, 64)) {
            notes.push(Note::warning(
                Code::J2kInvalidProfile,
                format!("Non-standard code-block size: {cb_w}x{cb_h} (DCI uses 32x32)"),
            ));
        }
    }

    // Component count: DCI requires 3 (XYZ)
    if info.components > 0 && info.components != 3 {
        notes.push(Note::error(
            Code::J2kInvalidComponentCount,
            format!("DCI requires 3 components (XYZ); found {}", info.components),
        ));
    }

    // Bit depth: DCI is 12 bits per component
    if info.bit_depth > 0 && info.bit_depth != 12 {
        notes.push(Note::warning(
            Code::J2kInvalidProfile,
            format!(
                "DCI standard uses 12 bits per component; found {}",
                info.bit_depth
            ),
        ));
    }

    // a codestream header carries no frame rate, so the peak bitrate is measured
    // over the whole essence by check_bitrate_compliance instead

    // RSIZ profile validation
    if info.rsiz > 0 {
        match info.rsiz {
            1 | 3 => {
                // Profile 0 / Cinema 2K
                if info.width > 2048 || info.height > 1080 {
                    notes.push(Note::error(
                        Code::J2kInvalidProfile,
                        format!(
                            "Profile claims Cinema 2K but resolution is {}x{}",
                            info.width, info.height
                        ),
                    ));
                }
            }
            2 | 4 if info.width > 4096 || info.height > 2160 => {
                // Profile 1 / Cinema 4K
                notes.push(Note::error(
                    Code::J2kInvalidProfile,
                    format!(
                        "Resolution {}x{} exceeds Cinema 4K maximum (4096x2160)",
                        info.width, info.height
                    ),
                ));
            }
            _ => {}
        }
    }

    notes
}

/// Analyze per-frame bitrate for a directory of J2K codestream files.
pub fn analyze_frame_bitrates(dir: &Path, fps: f64) -> Result<BitrateStats, String> {
    let mut frames: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("Failed to read directory: {e}"))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| matches!(e, "j2c" | "j2k" | "jp2"))
        })
        .collect();

    if frames.is_empty() {
        return Err("No J2K frame files found".into());
    }

    frames.sort();

    let mut sizes: Vec<u64> = Vec::with_capacity(frames.len());
    for f in &frames {
        let size = std::fs::metadata(f).map(|m| m.len()).unwrap_or(0);
        sizes.push(size);
    }

    let total: u64 = sizes.iter().sum();
    let count = sizes.len() as f64;
    let avg = total as f64 / count;
    let min = *sizes.iter().min().unwrap_or(&0);
    let max = *sizes.iter().max().unwrap_or(&0);

    let avg_bitrate = avg * fps * 8.0 / 1_000_000.0;
    let max_bitrate = max as f64 * fps * 8.0 / 1_000_000.0;
    let min_bitrate = min as f64 * fps * 8.0 / 1_000_000.0;

    Ok(BitrateStats {
        frame_count: sizes.len() as u64,
        avg_frame_bytes: avg as u64,
        min_frame_bytes: min,
        max_frame_bytes: max,
        avg_bitrate_mbps: avg_bitrate,
        min_bitrate_mbps: min_bitrate,
        max_bitrate_mbps: max_bitrate,
        fps,
    })
}

// ─── 0xFFFF legacy-decoder constraint (SMPTE Cat. 862 / DoM #2740) ─────────────

/// Detect the SMPTE Legacy Compatibility Note 1 (Dolby Cat. 862 / DoM #2740)
/// error condition: two consecutive 0xFF bytes at a byte position that is
/// 254 mod 256, counted from the codestream start over the main header plus the
/// first tile-part, and realigned by each preceding tile-part length for later
/// tile-parts. Legacy Dolby decoders (DSS200 Cat. 862, DSP100) fail on such
/// streams. Returns the offending byte offset, if any.
pub fn detect_legacy_ffff(codestream: &[u8]) -> Option<usize> {
    if codestream.len() < 4 || !starts_with_soc(codestream) {
        return None;
    }
    let lmh = find_first_sot(codestream)?;

    // (region_start, phase_offset). Region 0 spans the main header plus the
    // first tile-part with offset 0; each later tile-part starts a region whose
    // offset is the sum of the preceding tile-part lengths.
    let mut regions: Vec<(usize, usize)> = Vec::new();
    let mut cum = 0usize;
    for (i, (start, psot)) in tile_parts(codestream, lmh).iter().enumerate() {
        if i == 0 {
            regions.push((0, 0));
        } else {
            regions.push((*start, cum));
        }
        cum += psot;
    }
    if regions.is_empty() {
        regions.push((0, 0));
    }

    for p in 0..codestream.len().saturating_sub(1) {
        let mut offset = 0usize;
        for (start, off) in &regions {
            if *start <= p {
                offset = *off;
            } else {
                break;
            }
        }
        if (p - offset) % 256 == 254 && codestream[p] == 0xFF && codestream[p + 1] == 0xFF {
            return Some(p);
        }
    }
    None
}

// ─── ISO/IEC 15444-1 cinema J2K constraints (SMPTE ST 429-4 / DoM #2451, #1664) ─

/// SIZ marker fields needed for the cinema constraints.
struct SizInfo {
    rsiz: u16,
    xsiz: u32,
    ysiz: u32,
    xosiz: u32,
    yosiz: u32,
    xtsiz: u32,
    ytsiz: u32,
}

impl SizInfo {
    fn width(&self) -> u32 {
        self.xsiz.saturating_sub(self.xosiz)
    }
    fn height(&self) -> u32 {
        self.ysiz.saturating_sub(self.yosiz)
    }
    /// Number of tiles the image is divided into.
    fn tiles(&self) -> u64 {
        if self.xtsiz == 0 || self.ytsiz == 0 {
            return 0;
        }
        let cols = self.width().div_ceil(self.xtsiz) as u64;
        let rows = self.height().div_ceil(self.ytsiz) as u64;
        cols * rows
    }
}

fn parse_siz(data: &[u8]) -> Option<SizInfo> {
    let s = main_header_segment(data, SIZ)?;
    if s.len() < 26 {
        return None;
    }
    let u16at = |i: usize| u16::from_be_bytes([s[i], s[i + 1]]);
    let u32at = |i: usize| u32::from_be_bytes([s[i], s[i + 1], s[i + 2], s[i + 3]]);
    Some(SizInfo {
        rsiz: u16at(0),
        xsiz: u32at(2),
        ysiz: u32at(6),
        xosiz: u32at(10),
        yosiz: u32at(14),
        xtsiz: u32at(18),
        ytsiz: u32at(22),
    })
}

/// Number of guard bits declared in the QCD marker (the top 3 bits of Sqcd).
fn qcd_guard_bits(data: &[u8]) -> Option<u8> {
    let s = main_header_segment(data, QCD)?;
    s.first().map(|sqcd| sqcd >> 5)
}

/// POC markers the DCI profiles allow in a codestream's main header: none for
/// 2K, exactly one for 4K. The 4K profile codes the 2K portion and the 4K
/// portion as two progressions, and the POC marker is what signals the switch,
/// which is why the profiles differ. A POC marker in a tile-part header is
/// permitted by neither.
///
/// The browser build used to reject a POC marker outright, which is a false
/// positive on every conformant 4K DCP.
const POC_MARKERS_IN_MAIN_HEADER_2K: usize = 0;
const POC_MARKERS_IN_MAIN_HEADER_4K: usize = 1;

/// TLM and POC findings from a full marker walk: libdcp's
/// MISSING_JPEG2000_TLM_MARKER, INCORRECT_JPEG2000_POC_MARKER_COUNT_FOR_2K/4K,
/// INVALID_JPEG2000_POC_MARKER_LOCATION and INCORRECT_JPEG2000_POC_MARKER.
fn marker_placement_notes(scan: &MarkerScan, is_4k: bool, label: &str) -> Vec<Note> {
    let mut notes = Vec::new();

    if !scan.tlm_present {
        notes.push(Note::error(
            Code::J2kMissingTlm,
            "JPEG 2000 codestream has no TLM (tile-part length) marker",
        ));
    }

    let expected_poc = if is_4k {
        POC_MARKERS_IN_MAIN_HEADER_4K
    } else {
        POC_MARKERS_IN_MAIN_HEADER_2K
    };
    if scan.poc_in_main_header != expected_poc {
        notes.push(Note::error(
            Code::J2kPocInvalid,
            format!(
                "DCI {label} requires {expected_poc} POC marker(s) in the main header; codestream has {}",
                scan.poc_in_main_header
            ),
        ));
    }
    if scan.poc_after_main_header > 0 {
        notes.push(Note::error(
            Code::J2kPocInvalid,
            format!(
                "{} POC marker(s) sit in a tile-part header, where no DCI profile permits one",
                scan.poc_after_main_header
            ),
        ));
    }
    for mismatch in &scan.poc_field_mismatches {
        notes.push(Note::error(
            Code::J2kPocInvalid,
            format!(
                "POC {} is {}, expected {}",
                mismatch.field, mismatch.found, mismatch.expected
            ),
        ));
    }

    notes
}

/// Validate a JPEG 2000 codestream against the SMPTE ST 429-4 / ISO 15444-1
/// digital-cinema profile constraints beyond what `validate_j2k_dci` covers:
/// single tile, required marker segments, guard bits, tile-part organisation,
/// frame size vs profile, and per-colour-component byte limits (which scale with
/// frame rate). `fps` is the picture edit rate.
pub fn validate_cinema_j2k(data: &[u8], fps: f64) -> Vec<Note> {
    let mut notes = Vec::new();
    if data.len() < 4 || !starts_with_soc(data) {
        return notes; // not a J2K codestream
    }

    let Some(siz) = parse_siz(data) else {
        notes.push(Note::error(
            Code::J2kInvalidProfile,
            "JPEG 2000 codestream missing or truncated SIZ marker",
        ));
        return notes;
    };

    let is_4k = siz.rsiz == 4 || siz.width() > 2048 || siz.height() > 1080;
    let label = if is_4k { "Cinema 4K" } else { "Cinema 2K" };

    // required main-header markers
    if main_header_segment(data, COD).is_none() {
        notes.push(Note::error(
            Code::J2kInvalidProfile,
            "JPEG 2000 codestream missing COD marker",
        ));
    }
    if qcd_guard_bits(data).is_none() {
        notes.push(Note::error(
            Code::J2kInvalidProfile,
            "JPEG 2000 codestream missing QCD marker",
        ));
    }

    // single tile (ST 429-4: the whole image is one tile)
    let tiles = siz.tiles();
    if tiles > 1 {
        notes.push(Note::error(
            Code::J2kInvalidProfile,
            format!("DCI requires a single tile; codestream has {tiles} tiles"),
        ));
    }

    // frame size vs profile
    let (max_w, max_h) = if is_4k { (4096, 2160) } else { (2048, 1080) };
    if siz.width() > max_w || siz.height() > max_h {
        notes.push(Note::error(
            Code::J2kInvalidProfile,
            format!(
                "Resolution {}x{} exceeds the {label} maximum {max_w}x{max_h}",
                siz.width(),
                siz.height()
            ),
        ));
    }

    // guard-bit count is checked alongside these in check_picture_j2k_mxf, which
    // reads the profile off the MXF descriptor rather than the codestream

    notes.extend(marker_placement_notes(&scan_markers(data), is_4k, label));

    // tile-part organisation: 2K has 3 tile-parts (one per colour component),
    // 4K has 6 (three 2K-portion, three 4K-portion)
    let parts = find_first_sot(data)
        .map(|lmh| tile_parts(data, lmh))
        .unwrap_or_default();
    let expected_parts = if is_4k { 6 } else { 3 };
    if !parts.is_empty() && parts.len() != expected_parts {
        notes.push(Note::warning(
            Code::J2kInvalidProfile,
            format!(
                "DCI {label} uses {expected_parts} tile-parts; codestream has {}",
                parts.len()
            ),
        ));
    }

    // per-colour-component byte limit (200 Mbps-equivalent, scaling with fps):
    // each of the first three tile-parts carries one 2K colour component.
    let fps = if fps > 0.0 { fps } else { 24.0 };
    let per_comp_max = (200_000_000.0 / (8.0 * fps)) as usize;
    for (i, (_start, psot)) in parts.iter().take(3).enumerate() {
        if *psot > per_comp_max {
            notes.push(Note::error(
                Code::J2kBitrateExceeded,
                format!(
                    "Colour component {} is {psot} bytes, exceeding the DCI per-component maximum {per_comp_max} bytes at {fps:.0} fps",
                    i + 1
                ),
            ));
        }
    }

    notes
}

// ─── guard-bit constraint (SMPTE RDD 52 / DoM #2984) ──────────────────────────

/// true if the buffer begins with the SOC marker, i.e. is a raw J2K codestream
/// (encrypted essence read without a KDM does not).
fn starts_with_soc(data: &[u8]) -> bool {
    data.len() >= 2 && u16::from_be_bytes([data[0], data[1]]) == SOC
}

/// guard bits RDD 52 requires for a codestream of the given width: 1 for 2K, 2 for
/// 4K (stored width above 2048).
fn expected_guard_bits(width: u32) -> u8 {
    if width > 2048 { 2 } else { 1 }
}

/// returns (expected, actual) when a frame violates the RDD 52 guard-bit rule, or
/// None when it conforms, isn't a codestream, or has no QCD to read.
fn guard_bit_violation(codestream: &[u8], width: u32) -> Option<(u8, u8)> {
    if !starts_with_soc(codestream) {
        return None;
    }
    let actual = qcd_guard_bits(codestream)?;
    let expected = expected_guard_bits(width);
    (actual != expected).then_some((expected, actual))
}

/// frame index -> SMPTE timecode HH:MM:SS:FF at the given integer frame rate.
fn frame_to_timecode(frame: u32, fps: u32) -> String {
    let fps = fps.max(1);
    let secs = frame / fps;
    format!(
        "{:02}:{:02}:{:02}:{:02}",
        secs / 3600,
        (secs / 60) % 60,
        secs % 60,
        frame % fps
    )
}

/// Frame buffer for reading picture essence. DCI caps a frame near 1.3 MB (2K)
/// and 2.6 MB (4K), so this is headroom for a non-conformant asset too.
const FRAME_BUFFER_BYTES: usize = 16 * 1024 * 1024;

/// Distinct findings one asset's scan reports before it stops collecting. A
/// stream that is wrong in one way is wrong that way on every frame, so this
/// only bounds pathological essence.
const MAX_CODESTREAM_FINDINGS: usize = 20;

/// One distinct finding across a scan, with the first frame that showed it.
struct CodestreamFinding {
    note: Note,
    first_frame: u32,
    frames: u32,
}

/// Collects per-frame findings into one note each, so a fault present on every
/// frame of a feature reports once rather than 170,000 times.
#[derive(Default)]
struct CodestreamFindings {
    findings: Vec<CodestreamFinding>,
}

impl CodestreamFindings {
    fn record(&mut self, frame: u32, note: Note) {
        if let Some(seen) = self
            .findings
            .iter_mut()
            .find(|f| f.note.code == note.code && f.note.message == note.message)
        {
            seen.frames += 1;
            return;
        }
        if self.findings.len() < MAX_CODESTREAM_FINDINGS {
            self.findings.push(CodestreamFinding {
                note,
                first_frame: frame,
                frames: 1,
            });
        }
    }

    /// Finish each note with the file it came from and, once more than one frame
    /// has been read, where in the asset it was seen.
    fn into_notes(self, path: &Path, fps: u32, frames_scanned: u32) -> Vec<Note> {
        self.findings
            .into_iter()
            .map(|finding| {
                let mut note = finding.note;
                if frames_scanned > 1 {
                    note.message.push_str(&format!(
                        " (first at frame {} / {})",
                        finding.first_frame,
                        frame_to_timecode(finding.first_frame, fps)
                    ));
                    if finding.frames > 1 {
                        note.message
                            .push_str(&format!(", {} frames affected", finding.frames));
                    }
                }
                note.with_file(path)
            })
            .collect()
    }
}

/// The codestream parameters the first frame of a track sets, which every later
/// frame is compared against.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CodestreamReference {
    pub info: J2kCodestreamInfo,
    /// Tiles the image is divided into, from the SIZ image and tile dimensions.
    pub tile_count: u64,
    pub tile_part_count: u32,
    pub multiple_component_transform: bool,
    pub tlm_present: bool,
    pub poc_present: bool,
}

/// One codestream parameter that changed partway through a track, and where.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterDeviation {
    pub parameter: String,
    pub first_frame: u32,
    pub frames: u32,
}

/// What one pass over a picture track's codestreams found: the parameters the
/// first frame sets, every later frame that departs from them, and how close the
/// fattest codestream comes to the DCI per-frame byte cap, where one applies.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CodestreamForensics {
    /// Codestreams read, which is two per edit unit on stereoscopic essence.
    pub frames_scanned: u32,
    pub stereoscopic: bool,
    /// Picture edit rate, rounded, which the frame timecodes are derived from.
    pub frame_rate: u32,
    pub reference: CodestreamReference,
    pub deviations: Vec<ParameterDeviation>,
    pub max_tile_part_count: u32,
    pub max_tile_part_count_frame: u32,
    pub worst_frame_bytes: u64,
    pub worst_frame_index: u32,
    /// The DCI per-frame byte cap this essence is held to. IMF essence has none.
    pub dci_frame_byte_cap: Option<u64>,
    pub frames_over_dci_cap: u32,
}

/// Tiles the image is divided into, from the SIZ image and tile dimensions.
fn header_tile_count(header: &postkit::j2k::J2kHeader) -> u64 {
    if header.tile_width == 0 || header.tile_height == 0 {
        return 0;
    }
    header.width.div_ceil(header.tile_width) as u64
        * header.height.div_ceil(header.tile_height) as u64
}

/// Reads one codestream parameter off a header as a comparable value.
type ParameterReader = fn(&postkit::j2k::J2kHeader) -> u64;

/// Codestream parameters an encoder holds constant over a track, each reduced to
/// one comparable value. The names are the codestream header's own, so a warning
/// points at something greppable in the standard.
const CONSTANT_PARAMETERS: &[(&str, ParameterReader)] = &[
    ("image_size", |h| {
        ((h.width as u64) << 32) | (h.height as u64)
    }),
    ("num_components", |h| h.num_components as u64),
    ("bit_depth", |h| h.bit_depth as u64),
    ("profile", |h| h.profile as u64),
    ("num_decomp_levels", |h| h.num_decomp_levels as u64),
    ("codeblock_size", |h| {
        ((h.codeblock_width_exp as u64) << 32) | (h.codeblock_height_exp as u64)
    }),
    ("irreversible_transform", |h| {
        h.irreversible_transform as u64
    }),
    ("num_layers", |h| h.num_layers as u64),
    ("progression_order", |h| h.progression_order as u64),
    ("mct", |h| h.mct as u64),
    ("tlm_present", |h| h.tlm_present as u64),
    ("poc_present", |h| h.poc_present as u64),
    ("tile_count", header_tile_count),
];

/// Parameters this frame's header sets differently from the first frame's.
fn deviating_parameters(
    reference: &postkit::j2k::J2kHeader,
    header: &postkit::j2k::J2kHeader,
) -> Vec<&'static str> {
    CONSTANT_PARAMETERS
        .iter()
        .filter(|(_, value)| value(reference) != value(header))
        .map(|(name, _)| *name)
        .collect()
}

impl CodestreamForensics {
    /// Start a scan from the first frame's header. `frame_rate` drives the
    /// timecodes, and `dci_frame_byte_cap` is the per-frame byte budget the
    /// essence is held to, which IMF essence does not have.
    fn from_first_frame(
        header: &postkit::j2k::J2kHeader,
        frame_bytes: u64,
        frame_rate: u32,
        dci_frame_byte_cap: Option<u64>,
        stereoscopic: bool,
    ) -> Self {
        Self {
            stereoscopic,
            frame_rate,
            reference: CodestreamReference {
                info: info_from_header(header, frame_bytes),
                tile_count: header_tile_count(header),
                tile_part_count: header.tile_part_count,
                multiple_component_transform: header.mct,
                tlm_present: header.tlm_present,
                poc_present: header.poc_present,
            },
            dci_frame_byte_cap,
            ..Default::default()
        }
    }

    /// Fold one codestream into the running totals. The tile-part count is
    /// allowed to vary from frame to frame, so it is reported, not flagged.
    fn observe_frame(&mut self, frame: u32, header: &postkit::j2k::J2kHeader, frame_bytes: u64) {
        self.frames_scanned += 1;
        if header.tile_part_count > self.max_tile_part_count {
            self.max_tile_part_count = header.tile_part_count;
            self.max_tile_part_count_frame = frame;
        }
        if frame_bytes > self.worst_frame_bytes {
            self.worst_frame_bytes = frame_bytes;
            self.worst_frame_index = frame;
        }
        if self.dci_frame_byte_cap.is_some_and(|cap| frame_bytes > cap) {
            self.frames_over_dci_cap += 1;
        }
    }

    fn record_deviation(&mut self, parameter: &str, frame: u32) {
        if let Some(seen) = self
            .deviations
            .iter_mut()
            .find(|deviation| deviation.parameter == parameter)
        {
            seen.frames += 1;
            return;
        }
        self.deviations.push(ParameterDeviation {
            parameter: parameter.to_string(),
            first_frame: frame,
            frames: 1,
        });
    }

    /// Worst frame as a percentage of the DCI per-frame byte cap, for the
    /// essence that has one.
    pub fn cap_percentage(&self) -> Option<f64> {
        let cap = self.dci_frame_byte_cap.filter(|cap| *cap > 0)?;
        Some(self.worst_frame_bytes as f64 * 100.0 / cap as f64)
    }

    /// SMPTE timecode of the worst frame, at the picture edit rate.
    pub fn worst_frame_timecode(&self) -> String {
        frame_to_timecode(self.worst_frame_index, self.frame_rate)
    }

    /// "yes", or "no" and every parameter that changed, with where.
    pub fn parameters_constant_text(&self) -> String {
        if self.deviations.is_empty() {
            return "yes".to_string();
        }
        let varying: Vec<String> = self
            .deviations
            .iter()
            .map(|deviation| {
                format!(
                    "{} from frame {} ({} frames)",
                    deviation.parameter, deviation.first_frame, deviation.frames
                )
            })
            .collect();
        format!("no: {}", varying.join(", "))
    }

    /// The one-line summary the whole-asset scan reports as an INFO note.
    fn summary(&self) -> String {
        let reference = &self.reference;
        let info = &reference.info;
        let (codeblock_width, codeblock_height) = info.codeblock_size();
        let constancy = if self.deviations.is_empty() {
            format!("parameters identical across {} frames", self.frames_scanned)
        } else {
            let varying: Vec<&str> = self
                .deviations
                .iter()
                .map(|deviation| deviation.parameter.as_str())
                .collect();
            format!(
                "{} parameter(s) vary across {} frames: {}",
                varying.len(),
                self.frames_scanned,
                varying.join(", ")
            )
        };
        let over_cap = if self.frames_over_dci_cap > 0 {
            format!(", {} frames over the cap", self.frames_over_dci_cap)
        } else {
            String::new()
        };
        let against_cap = match self.dci_frame_byte_cap.zip(self.cap_percentage()) {
            Some((cap, percentage)) => format!(", {percentage:.0}% of the {cap} byte DCI cap"),
            None => String::new(),
        };
        format!(
            "JPEG 2000 codestream: {}x{}, {}, {} decomposition levels, {codeblock_width}x{codeblock_height} code-blocks, {} tile(s), up to {} tile-parts, {}, {}, {}{}; {constancy}; worst frame {} bytes{against_cap}, at frame {} / {}{over_cap}",
            info.width,
            info.height,
            info.profile,
            info.decomposition_levels,
            reference.tile_count,
            self.max_tile_part_count,
            if reference.tlm_present {
                "TLM present"
            } else {
                "no TLM"
            },
            if reference.poc_present {
                "POC present"
            } else {
                "no POC"
            },
            if reference.multiple_component_transform {
                "MCT on"
            } else {
                "MCT off"
            },
            if self.stereoscopic { ", both eyes" } else { "" },
            self.worst_frame_bytes,
            self.worst_frame_index,
            self.worst_frame_timecode(),
        )
    }
}

/// The first frame's header and the running forensics, held together so a later
/// frame has something to be compared against.
struct PictureScanState {
    reference: postkit::j2k::J2kHeader,
    forensics: CodestreamForensics,
}

/// Which package a picture track belongs to, which decides both the reader its
/// essence is opened with and the per-frame rules it is held to: the DCI cinema
/// constraints apply to a DCP and to nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PictureEssenceFamily {
    /// A DCP's AS-DCP OP-Atom essence, monoscopic or stereoscopic.
    Cinema,
    /// An IMP's AS-02 essence, which is monoscopic.
    Imf,
}

/// A picture track's codestream reader: AS-DCP OP-Atom monoscopic essence, the
/// stereoscopic form whose every edit unit carries a left and a right eye, or
/// AS-02 essence. Shared with the bitrate walk, which reads the same frames.
pub(crate) enum PictureEssenceReader {
    Monoscopic(MxfReader),
    Stereoscopic(StereoMxfReader),
    As02(As02MxfReader),
}

/// The eyes to read per edit unit. The monoscopic reader ignores the phase.
const MONOSCOPIC_EYES: &[StereoscopicPhase] = &[StereoscopicPhase::Left];
const STEREOSCOPIC_EYES: &[StereoscopicPhase] =
    &[StereoscopicPhase::Left, StereoscopicPhase::Right];

impl PictureEssenceReader {
    /// Open the essence with the readers its family wraps its picture in. The
    /// families do not fall through to each other: an OP-Atom file the AS-DCP
    /// reader rejects is a broken DCP asset, not an IMF one.
    pub(crate) fn open(path: &str, family: PictureEssenceFamily) -> Option<Self> {
        if family == PictureEssenceFamily::Imf {
            let mut as02 = As02MxfReader::new();
            return as02.open_read(path).is_ok().then_some(Self::As02(as02));
        }
        let mut monoscopic = MxfReader::new();
        if monoscopic.open_read(path).is_ok() {
            return Some(Self::Monoscopic(monoscopic));
        }
        let mut stereoscopic = StereoMxfReader::new();
        if stereoscopic.open_read(path).is_ok() {
            return Some(Self::Stereoscopic(stereoscopic));
        }
        None
    }

    pub(crate) fn picture_descriptor(
        &mut self,
    ) -> asdcplib::Result<asdcplib::jp2k::PictureDescriptor> {
        match self {
            Self::Monoscopic(reader) => reader.picture_descriptor(),
            Self::Stereoscopic(reader) => reader.picture_descriptor(),
            Self::As02(reader) => reader.picture_descriptor(),
        }
    }

    pub(crate) fn writer_info(&mut self) -> asdcplib::Result<asdcplib::WriterInfo> {
        match self {
            Self::Monoscopic(reader) => reader.writer_info(),
            Self::Stereoscopic(reader) => reader.writer_info(),
            Self::As02(reader) => reader.writer_info(),
        }
    }

    pub(crate) fn eyes(&self) -> &'static [StereoscopicPhase] {
        match self {
            Self::Monoscopic(_) | Self::As02(_) => MONOSCOPIC_EYES,
            Self::Stereoscopic(_) => STEREOSCOPIC_EYES,
        }
    }

    pub(crate) fn read_frame(
        &mut self,
        index: u32,
        eye: StereoscopicPhase,
        buffer: &mut [u8],
        decrypt: Option<&mut asdcplib::crypto::AesDecContext>,
        hmac: Option<&mut asdcplib::crypto::HmacContext>,
    ) -> asdcplib::Result<usize> {
        match self {
            Self::Monoscopic(reader) => reader.read_frame(index, buffer, decrypt, hmac),
            Self::Stereoscopic(reader) => reader.read_frame(index, eye, buffer, decrypt, hmac),
            Self::As02(reader) => reader.read_frame(index, buffer, decrypt, hmac),
        }
    }
}

/// Read a picture MXF's codestreams and run every per-frame check on each. On
/// `Cinema` essence that is the 0xFFFF legacy-decoder constraint (DoM #2740),
/// the ISO 15444-1 cinema profile constraints (DoM #2451/#1664, TLM and POC
/// placement included) and the RDD 52 guard-bit rule (DoM #2984); none of the
/// three applies to `Imf` essence, which is not encoded to the cinema profile.
/// The same pass collects the codestream forensics for both families, which is
/// what the report section and the INFO summary are built from.
///
/// `scan_every_frame` false reads only frame 0, which catches an encoder that was
/// wrong for the whole asset; reading the rest is what catches a stream that goes
/// non-conformant partway through, and costs a pass over the essence. Non-picture
/// or unreadable MXFs yield no notes. Encrypted essence is decrypted with `keys`;
/// without a covering key it skips (with a note when a KDM was supplied but lacks
/// the KeyId).
pub fn check_picture_j2k_mxf(
    path: &Path,
    keys: &crate::kdm::ContentKeys,
    family: PictureEssenceFamily,
    scan_every_frame: bool,
) -> (Vec<Note>, Option<CodestreamForensics>) {
    let mut notes = Vec::new();
    let Some(s) = path.to_str() else {
        return (notes, None);
    };
    let Some(mut reader) = PictureEssenceReader::open(s, family) else {
        return (notes, None);
    };
    let Ok(desc) = reader.picture_descriptor() else {
        return (notes, None);
    };
    let Ok(info) = reader.writer_info() else {
        return (notes, None);
    };
    let essence = keys.resolve(&info);
    if essence.is_missing() {
        notes.extend(essence.skip_note(path));
        return (notes, None);
    }
    let mut ctx = match essence.contexts() {
        Ok(c) => c,
        Err(e) => {
            notes.push(Note::error(Code::MxfUnreadable, e).with_file(path));
            return (notes, None);
        }
    };

    let width = desc.stored_width;
    let edit_rate = desc.edit_rate.numerator as f64 / desc.edit_rate.denominator.max(1) as f64;
    let timecode_rate = edit_rate.round() as u32;
    let stereoscopic = matches!(reader, PictureEssenceReader::Stereoscopic(_));
    // stereoscopic essence sends a left and a right codestream per edit unit, so
    // each eye lives on half an edit unit's byte budget
    let codestream_rate = if stereoscopic {
        edit_rate * 2.0
    } else {
        edit_rate
    };
    let eyes = reader.eyes();
    let cinema = family == PictureEssenceFamily::Cinema;
    // ST 2067-21 sets no per-frame byte budget on IMF picture essence
    let dci_frame_byte_cap =
        cinema.then(|| postkit::j2k::dci_codestream_byte_cap(codestream_rate.round() as u32));
    let frames = if scan_every_frame {
        desc.container_duration
    } else {
        1
    };

    let mut findings = CodestreamFindings::default();
    let mut buf = vec![0u8; FRAME_BUFFER_BYTES];
    let mut scanned = 0u32;
    let mut scan: Option<PictureScanState> = None;
    'edit_units: for i in 0..frames {
        for &eye in eyes {
            let (dec, hmac) = match ctx.as_mut() {
                Some(c) => (Some(&mut c.dec), Some(&mut c.hmac)),
                None => (None, None),
            };
            let n = match reader.read_frame(i, eye, &mut buf, dec, hmac) {
                Ok(n) => n,
                Err(e) => {
                    // with a key set, a read failure on frame 0 is a decrypt/MIC
                    // integrity failure; report it. Later frames just end the scan.
                    if scanned == 0 && info.encrypted_essence {
                        notes.push(
                            Note::error(
                                Code::MxfHashMismatch,
                                format!("frame 0 integrity check (HMAC/MIC) failed: {e}"),
                            )
                            .with_file(path),
                        );
                    }
                    break 'edit_units;
                }
            };
            let frame = &buf[..n];
            // essence that isn't a readable codestream (encrypted without a KDM, or
            // not picture) can't be checked; bail on the first frame.
            if scanned == 0 && !starts_with_soc(frame) {
                return (notes, None);
            }
            scanned += 1;

            if cinema {
                if let Some(offset) = detect_legacy_ffff(frame) {
                    findings.record(
                        i,
                        Note::warning(
                            Code::J2kLegacyFfff,
                            format!(
                                "JPEG 2000 codestream triggers the 0xFFFF legacy-decoder error condition (SMPTE Cat. 862) at byte offset {offset}"
                            ),
                        ),
                    );
                }
                if let Some((expected, actual)) = guard_bit_violation(frame, width) {
                    let profile = if width > 2048 { "4K" } else { "2K" };
                    findings.record(
                        i,
                        Note::error(
                            Code::J2kGuardBits,
                            format!(
                                "JPEG 2000 QCD declares {actual} guard bit(s); SMPTE RDD 52 requires {expected} for {profile}"
                            ),
                        ),
                    );
                }
                for note in validate_cinema_j2k(frame, codestream_rate) {
                    findings.record(i, note);
                }
            }

            if let Some(header) = postkit::j2k::parse_j2k_header(frame) {
                let state = scan.get_or_insert_with(|| PictureScanState {
                    reference: header.clone(),
                    forensics: CodestreamForensics::from_first_frame(
                        &header,
                        n as u64,
                        timecode_rate,
                        dci_frame_byte_cap,
                        stereoscopic,
                    ),
                });
                for parameter in deviating_parameters(&state.reference, &header) {
                    state.forensics.record_deviation(parameter, i);
                    findings.record(
                        i,
                        Note::warning(
                            Code::J2kParametersVary,
                            format!(
                                "JPEG 2000 codestream parameter {parameter} differs from the first frame's"
                            ),
                        ),
                    );
                }
                state.forensics.observe_frame(i, &header, n as u64);
            }
        }
    }

    let forensics = scan.map(|state| state.forensics);
    if scan_every_frame && let Some(summarised) = &forensics {
        notes.push(Note::info(Code::J2kCodestreamSummary, summarised.summary()).with_file(path));
    }
    notes.extend(findings.into_notes(path, timecode_rate, scanned));
    (notes, forensics)
}

use std::path::PathBuf;

/// Per-frame bitrate statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BitrateStats {
    pub frame_count: u64,
    pub avg_frame_bytes: u64,
    pub min_frame_bytes: u64,
    pub max_frame_bytes: u64,
    pub avg_bitrate_mbps: f64,
    pub min_bitrate_mbps: f64,
    pub max_bitrate_mbps: f64,
    pub fps: f64,
}

#[cfg(test)]
mod ffff_tests {
    use super::*;

    // single-tile-part codestream of `total` bytes: SOC, a filler SIZ marker,
    // SOT (with Psot spanning to the end), SOD, then zeroed tile data.
    fn codestream(total: usize) -> Vec<u8> {
        let mut d = vec![0u8; total];
        d[0] = 0xFF;
        d[1] = 0x4F; // SOC
        // filler main-header marker at 2: FF51 (SIZ), Lmar = 4 (2 len + 2 param)
        d[2] = 0xFF;
        d[3] = 0x51;
        d[4] = 0x00;
        d[5] = 0x04;
        // first SOT at offset 8: FF90, Lsot=10, Isot=0, Psot, TPsot=0, TNsot=1
        d[8] = 0xFF;
        d[9] = 0x90;
        d[10] = 0x00;
        d[11] = 0x0A;
        let psot = (total - 8) as u32;
        d[14..18].copy_from_slice(&psot.to_be_bytes());
        d[19] = 0x01; // TNsot
        // SOD at 20
        d[20] = 0xFF;
        d[21] = 0x93;
        d
    }

    #[test]
    fn ffff_at_254_triggers() {
        let mut d = codestream(600);
        d[254] = 0xFF;
        d[255] = 0xFF;
        assert_eq!(detect_legacy_ffff(&d), Some(254));
    }

    #[test]
    fn clean_stream_is_silent() {
        let d = codestream(600);
        assert_eq!(detect_legacy_ffff(&d), None);
    }

    #[test]
    fn ffff_off_the_254_grid_is_silent() {
        // consecutive 0xFF at a non-254-mod-256 position is harmless
        let mut d = codestream(600);
        d[300] = 0xFF;
        d[301] = 0xFF;
        assert_eq!(detect_legacy_ffff(&d), None);
    }

    #[test]
    fn non_j2k_buffer_is_silent() {
        assert_eq!(detect_legacy_ffff(&[0u8; 600]), None);
    }
}

#[cfg(test)]
mod cinema_tests {
    use super::*;

    /// COM, another length-prefixed main-header segment, used to retag a marker
    /// so it disappears without changing any offset in the stream.
    const COM: u16 = 0xFF64;

    /// The conformant two-progression POC parameter bytes: RSpoc, CSpoc,
    /// LYEpoc(2), REpoc, CEpoc, Ppoc, twice.
    const CONFORMANT_POC_PARAMETERS: &[u8] = &[0, 0, 0, 1, 6, 3, 4, 6, 0, 0, 1, 7, 3, 4];

    /// Retag the first `marker` as `replacement`, in place. Both must be
    /// length-prefixed segment markers so the stream stays walkable.
    fn retag_marker(data: &mut [u8], marker: u16, replacement: u16) {
        let needle = marker.to_be_bytes();
        let at = data
            .windows(2)
            .position(|w| w == needle)
            .expect("the stream carries the marker being retagged");
        data[at..at + 2].copy_from_slice(&replacement.to_be_bytes());
    }

    // Assemble a codestream: SOC, SIZ, COD, QCD, TLM, the 4K POC when the image is
    // 4K, then one tile-part per entry in `tp_data_sizes` (each SOT+SOD plus that
    // many data bytes), then EOC.
    pub(super) fn build_j2k(
        rsiz: u16,
        width: u32,
        height: u32,
        tile_w: u32,
        tile_h: u32,
        guard_bits: u8,
        tp_data_sizes: &[usize],
    ) -> Vec<u8> {
        let mut d = vec![0xFF, 0x4F]; // SOC

        // SIZ (FF51)
        let csiz: u16 = 3;
        let mut siz = Vec::new();
        siz.extend_from_slice(&rsiz.to_be_bytes());
        siz.extend_from_slice(&width.to_be_bytes()); // Xsiz (XOsiz=0)
        siz.extend_from_slice(&height.to_be_bytes()); // Ysiz
        siz.extend_from_slice(&0u32.to_be_bytes()); // XOsiz
        siz.extend_from_slice(&0u32.to_be_bytes()); // YOsiz
        siz.extend_from_slice(&tile_w.to_be_bytes()); // XTsiz
        siz.extend_from_slice(&tile_h.to_be_bytes()); // YTsiz
        siz.extend_from_slice(&0u32.to_be_bytes()); // XTOsiz
        siz.extend_from_slice(&0u32.to_be_bytes()); // YTOsiz
        siz.extend_from_slice(&csiz.to_be_bytes());
        for _ in 0..csiz {
            siz.extend_from_slice(&[11, 1, 1]); // Ssiz (12-bit), XRsiz, YRsiz
        }
        d.extend_from_slice(&[0xFF, 0x51]);
        d.extend_from_slice(&((2 + siz.len()) as u16).to_be_bytes());
        d.extend_from_slice(&siz);

        // COD (FF52), minimal 12 param bytes
        d.extend_from_slice(&[0xFF, 0x52]);
        d.extend_from_slice(&(2u16 + 12).to_be_bytes());
        d.extend_from_slice(&[0u8; 12]);

        // QCD (FF5C): Sqcd guard bits in the top 3 bits
        d.extend_from_slice(&[0xFF, 0x5C]);
        d.extend_from_slice(&(2u16 + 4).to_be_bytes());
        d.extend_from_slice(&[guard_bits << 5, 0, 0, 0]);

        // TLM (FF55): Ztlm, Stlm, then per-tile-part lengths the walk ignores
        d.extend_from_slice(&[0xFF, 0x55]);
        d.extend_from_slice(&(2u16 + 2).to_be_bytes());
        d.extend_from_slice(&[0, 0]);

        // POC (FF5F): the 4K profile requires exactly one, 2K none
        if rsiz == 4 || width > 2048 || height > 1080 {
            d.extend_from_slice(&[0xFF, 0x5F]);
            d.extend_from_slice(&((2 + CONFORMANT_POC_PARAMETERS.len()) as u16).to_be_bytes());
            d.extend_from_slice(CONFORMANT_POC_PARAMETERS);
        }

        // tile-parts
        let count = tp_data_sizes.len() as u8;
        for (i, &dsize) in tp_data_sizes.iter().enumerate() {
            let psot = (12 + 2 + dsize) as u32; // SOT(12) + SOD(2) + data
            d.extend_from_slice(&[0xFF, 0x90]); // SOT
            d.extend_from_slice(&10u16.to_be_bytes()); // Lsot
            d.extend_from_slice(&0u16.to_be_bytes()); // Isot
            d.extend_from_slice(&psot.to_be_bytes()); // Psot
            d.push(i as u8); // TPsot
            d.push(count); // TNsot
            d.extend_from_slice(&[0xFF, 0x93]); // SOD
            d.extend(std::iter::repeat_n(0u8, dsize));
        }
        d.extend_from_slice(&[0xFF, 0xD9]); // EOC
        d
    }

    #[test]
    fn well_formed_2k_is_clean() {
        let d = build_j2k(3, 2048, 1080, 2048, 1080, 1, &[1000, 1000, 1000]);
        assert!(
            validate_cinema_j2k(&d, 24.0).is_empty(),
            "conformant 2K codestream must be clean"
        );
    }

    #[test]
    fn multiple_tiles_flagged() {
        // tile width half the image -> 2 tiles
        let d = build_j2k(3, 2048, 1080, 1024, 1080, 1, &[1000, 1000, 1000]);
        assert!(
            validate_cinema_j2k(&d, 24.0)
                .iter()
                .any(|n| n.code == Code::J2kInvalidProfile && n.message.contains("single tile")),
            "multi-tile must be flagged"
        );
    }

    #[test]
    fn wrong_tile_part_count_flagged() {
        let d = build_j2k(3, 2048, 1080, 2048, 1080, 1, &[1000, 1000]);
        assert!(
            validate_cinema_j2k(&d, 24.0)
                .iter()
                .any(|n| n.message.contains("tile-parts")),
            "2 tile-parts for 2K must be flagged"
        );
    }

    #[test]
    fn oversize_component_flagged_and_fps_scales() {
        // per-component max at 24 fps is 1,041,666 bytes
        let big = build_j2k(3, 2048, 1080, 2048, 1080, 1, &[1_100_000, 1000, 1000]);
        assert!(
            validate_cinema_j2k(&big, 24.0)
                .iter()
                .any(|n| n.code == Code::J2kBitrateExceeded),
            "oversized colour component must be flagged at 24 fps"
        );
        // the same component is within the (larger) budget at 12 fps
        assert!(
            !validate_cinema_j2k(&big, 12.0)
                .iter()
                .any(|n| n.code == Code::J2kBitrateExceeded),
            "per-component budget doubles at 12 fps, so it must pass"
        );
    }

    // ─── TLM and POC placement ────────────────────────────────────────────

    #[test]
    fn well_formed_4k_is_clean() {
        let d = build_j2k(4, 4096, 2160, 4096, 2160, 2, &[1000; 6]);
        assert!(
            validate_cinema_j2k(&d, 24.0).is_empty(),
            "conformant 4K codestream must be clean, POC marker included"
        );
    }

    #[test]
    fn missing_tlm_fires() {
        let mut d = build_j2k(3, 2048, 1080, 2048, 1080, 1, &[1000, 1000, 1000]);
        retag_marker(&mut d, dcpdoctor_parse::j2k::TLM, COM);
        assert!(
            validate_cinema_j2k(&d, 24.0)
                .iter()
                .any(|n| n.code == Code::J2kMissingTlm),
            "a codestream with no TLM marker must fire"
        );
    }

    // libdcp requires exactly one main-header POC for 4K and none for 2K, so the
    // browser build's "a POC marker is never permitted" was a false positive on
    // every conformant 4K DCP.
    #[test]
    fn poc_count_follows_the_profile() {
        let mut without_poc = build_j2k(4, 4096, 2160, 4096, 2160, 2, &[1000; 6]);
        retag_marker(&mut without_poc, dcpdoctor_parse::j2k::POC, COM);
        assert!(
            validate_cinema_j2k(&without_poc, 24.0)
                .iter()
                .any(|n| n.code == Code::J2kPocInvalid && n.message.contains("requires 1")),
            "a 4K codestream with no POC marker must fire"
        );

        let clean_2k = build_j2k(3, 2048, 1080, 2048, 1080, 1, &[1000, 1000, 1000]);
        assert!(
            !validate_cinema_j2k(&clean_2k, 24.0)
                .iter()
                .any(|n| n.code == Code::J2kPocInvalid),
            "a 2K codestream with no POC marker is conformant"
        );
    }

    #[test]
    fn poc_in_a_tile_part_header_fires_and_field_values_are_reported() {
        let scan = MarkerScan {
            tlm_present: true,
            poc_in_main_header: 1,
            poc_after_main_header: 1,
            poc_field_mismatches: vec![dcpdoctor_parse::j2k::PocFieldMismatch {
                field: "Ppoc of progression 1".into(),
                expected: 4,
                found: 0,
            }],
        };
        let notes = marker_placement_notes(&scan, true, "Cinema 4K");
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::J2kPocInvalid && n.message.contains("tile-part header")),
            "a POC outside the main header must fire, got: {notes:?}"
        );
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::J2kPocInvalid && n.message.contains("Ppoc")),
            "a wrong POC parameter must be named, got: {notes:?}"
        );
        assert!(
            !notes.iter().any(|n| n.code == Code::J2kMissingTlm),
            "TLM was present, got: {notes:?}"
        );
    }

    #[test]
    fn oversize_4k_resolution_flagged() {
        let d = build_j2k(4, 5000, 2160, 5000, 2160, 2, &[1000; 6]);
        assert!(
            validate_cinema_j2k(&d, 24.0)
                .iter()
                .any(|n| n.message.contains("exceeds the Cinema 4K maximum")),
            "resolution beyond 4K must be flagged"
        );
    }
}

#[cfg(test)]
pub(crate) mod frame_scan_tests {
    use super::cinema_tests::build_j2k;
    use super::*;
    use asdcplib::as02::jp2k::MxfWriter as As02MxfWriter;
    use asdcplib::crypto::{AesEncContext, HmacContext};
    use asdcplib::jp2k::{MxfWriter, PictureDescriptor, StereoMxfWriter};
    use asdcplib::{LabelSet, Rational, WriterInfo};

    /// Frames in the guard-bit fixture asset, and the one that goes
    /// non-conformant. Frame 0 is deliberately clean: that is the whole point of
    /// scanning past it.
    const FIXTURE_FRAMES: u32 = 6;
    const BAD_FRAME: u32 = 3;

    /// Decomposition levels a conformant DCI codestream declares, and the count
    /// the deviating fixture frame declares instead.
    const CONFORMANT_DECOMPOSITION_LEVELS: u8 = 5;
    const DEVIATING_DECOMPOSITION_LEVELS: u8 = 3;

    /// Tile-part payload bytes in a fixture frame, and in the one fat frame the
    /// worst-frame assertions look for.
    const FIXTURE_PAYLOAD_BYTES: usize = 64;
    const FAT_PAYLOAD_BYTES: usize = 512;

    fn fixture_writer_info() -> WriterInfo {
        WriterInfo {
            asset_uuid: [7; 16],
            label_set: LabelSet::Smpte,
            ..Default::default()
        }
    }

    fn fixture_descriptor(frames: u32) -> PictureDescriptor {
        PictureDescriptor {
            edit_rate: Rational::new(24, 1),
            sample_rate: Rational::new(24, 1),
            stored_width: 2048,
            stored_height: 1080,
            aspect_ratio: Rational::new(2048, 1080),
            container_duration: frames,
            component_count: 3,
        }
    }

    /// Overwrite the COD decomposition-level count, the parameter the deviation
    /// fixture varies. COD parameters start 4 bytes past the marker, and the
    /// level count is the sixth of them.
    fn set_decomposition_levels(codestream: &mut [u8], levels: u8) {
        let marker = COD.to_be_bytes();
        let at = codestream
            .windows(2)
            .position(|window| window == marker)
            .expect("the fixture codestream carries a COD marker");
        codestream[at + 4 + 5] = levels;
    }

    /// A 2K picture MXF whose frames all conform except `BAD_FRAME`, which
    /// declares 0 guard bits where RDD 52 requires 1.
    pub(crate) fn write_mxf(dir: &Path) -> PathBuf {
        let path = dir.join("pic.mxf");
        let mut writer = MxfWriter::new();
        writer
            .open_write(
                path.to_str().unwrap(),
                &fixture_writer_info(),
                &fixture_descriptor(FIXTURE_FRAMES),
                16_384,
            )
            .unwrap();
        for frame in 0..FIXTURE_FRAMES {
            let guard_bits = if frame == BAD_FRAME { 0 } else { 1 };
            let codestream = build_j2k(3, 2048, 1080, 2048, 1080, guard_bits, &[64, 64, 64]);
            writer.write_frame(&codestream, None, None).unwrap();
        }
        writer.finalize().unwrap();
        path
    }

    /// A 2K picture MXF, one conformant codestream per entry in `frames`, each
    /// declaring that entry's decomposition levels and carrying that many payload
    /// bytes per tile-part.
    pub(crate) fn write_picture_mxf(dir: &Path, name: &str, frames: &[(u8, usize)]) -> PathBuf {
        let path = dir.join(name);
        let mut writer = MxfWriter::new();
        writer
            .open_write(
                path.to_str().unwrap(),
                &fixture_writer_info(),
                &fixture_descriptor(frames.len() as u32),
                16_384,
            )
            .unwrap();
        for (levels, payload) in frames {
            let mut codestream = build_j2k(3, 2048, 1080, 2048, 1080, 1, &[*payload; 3]);
            set_decomposition_levels(&mut codestream, *levels);
            writer.write_frame(&codestream, None, None).unwrap();
        }
        writer.finalize().unwrap();
        path
    }

    /// Broadcast-profile RSIZ, what an IMF codestream declares instead of one of
    /// the cinema profiles, and the guard-bit count RDD 52 forbids at 2K, so a
    /// DCP scan of the same codestream has something to reject.
    const BROADCAST_PROFILE_RSIZ: u16 = 0x0102;
    const NON_CINEMA_GUARD_BITS: u8 = 0;

    /// A broadcast-profile codestream of `payload` bytes in its single tile-part,
    /// declaring `levels` decomposition levels.
    fn build_broadcast_j2k(width: u32, height: u32, levels: u8, payload: usize) -> Vec<u8> {
        let mut codestream = build_j2k(
            BROADCAST_PROFILE_RSIZ,
            width,
            height,
            width,
            height,
            NON_CINEMA_GUARD_BITS,
            &[payload],
        );
        set_decomposition_levels(&mut codestream, levels);
        codestream
    }

    /// An AS-02 picture MXF, the wrapping an IMP carries, one broadcast-profile
    /// codestream per entry in `frames`.
    pub(crate) fn write_as02_picture_mxf(
        dir: &Path,
        name: &str,
        width: u32,
        height: u32,
        frames: &[(u8, usize)],
    ) -> PathBuf {
        write_as02(dir, name, width, height, frames, None)
    }

    /// The same, encrypted with `content_key` under `key_id` and HMAC-guarded, the
    /// way asdcplib wraps an encrypted track file.
    pub(crate) fn write_encrypted_as02_picture_mxf(
        dir: &Path,
        name: &str,
        width: u32,
        height: u32,
        frames: &[(u8, usize)],
        key_id: uuid::Uuid,
        content_key: [u8; 16],
    ) -> PathBuf {
        write_as02(
            dir,
            name,
            width,
            height,
            frames,
            Some((key_id, content_key)),
        )
    }

    /// Plaintext bytes in one fixture frame, which is the size the bitrate of an
    /// encrypted track has to be measured from: its ciphertext frames are longer.
    pub(crate) fn as02_frame_bytes(width: u32, height: u32, levels: u8, payload: usize) -> usize {
        build_broadcast_j2k(width, height, levels, payload).len()
    }

    fn write_as02(
        dir: &Path,
        name: &str,
        width: u32,
        height: u32,
        frames: &[(u8, usize)],
        encryption: Option<(uuid::Uuid, [u8; 16])>,
    ) -> PathBuf {
        let path = dir.join(name);
        let mut descriptor = fixture_descriptor(frames.len() as u32);
        descriptor.stored_width = width;
        descriptor.stored_height = height;
        descriptor.aspect_ratio = Rational::new(width as i32, height as i32);

        let mut info = fixture_writer_info();
        let mut crypto = encryption.map(|(key_id, content_key)| {
            info.context_id = *uuid::Uuid::new_v4().as_bytes();
            info.cryptographic_key_id = *key_id.as_bytes();
            info.encrypted_essence = true;
            info.uses_hmac = true;
            let mut encrypt = AesEncContext::new();
            encrypt.init_key(&content_key).unwrap();
            let mut hmac = HmacContext::new();
            hmac.init_key(&content_key, LabelSet::Smpte).unwrap();
            (encrypt, hmac)
        });

        let mut writer = As02MxfWriter::new();
        writer
            .open_write(path.to_str().unwrap(), &info, &descriptor, 16_384)
            .unwrap();
        for (levels, payload) in frames {
            let codestream = build_broadcast_j2k(width, height, *levels, *payload);
            let (encrypt, hmac) = match crypto.as_mut() {
                Some((encrypt, hmac)) => (Some(encrypt), Some(hmac)),
                None => (None, None),
            };
            writer.write_frame(&codestream, encrypt, hmac).unwrap();
        }
        writer.finalize().unwrap();
        path
    }

    /// The same, stereoscopic: every edit unit carries a left and a right eye.
    fn write_stereoscopic_mxf(dir: &Path, edit_units: u32) -> PathBuf {
        let path = dir.join("pic_3d.mxf");
        let mut descriptor = fixture_descriptor(edit_units);
        descriptor.sample_rate = Rational::new(48, 1);
        let mut writer = StereoMxfWriter::new();
        writer
            .open_write(
                path.to_str().unwrap(),
                &fixture_writer_info(),
                &descriptor,
                16_384,
            )
            .unwrap();
        let mut codestream = build_j2k(3, 2048, 1080, 2048, 1080, 1, &[FIXTURE_PAYLOAD_BYTES; 3]);
        set_decomposition_levels(&mut codestream, CONFORMANT_DECOMPOSITION_LEVELS);
        for _ in 0..edit_units {
            for eye in [StereoscopicPhase::Left, StereoscopicPhase::Right] {
                writer.write_frame(&codestream, eye, None, None).unwrap();
            }
        }
        writer.finalize().unwrap();
        path
    }

    /// Five frames where frame 3 alone declares different decomposition levels,
    /// and frame 1 alone is fat.
    fn deviating_frames() -> Vec<(u8, usize)> {
        (0..5)
            .map(|frame| {
                let levels = if frame == BAD_FRAME {
                    DEVIATING_DECOMPOSITION_LEVELS
                } else {
                    CONFORMANT_DECOMPOSITION_LEVELS
                };
                let payload = if frame == 1 {
                    FAT_PAYLOAD_BYTES
                } else {
                    FIXTURE_PAYLOAD_BYTES
                };
                (levels, payload)
            })
            .collect()
    }

    // reading frame 0 only is blind to a stream that goes non-conformant later,
    // which is what the whole-asset scan exists for.
    #[test]
    fn a_fault_after_frame_zero_is_only_seen_by_the_whole_asset_scan() {
        let dir = tempfile::tempdir().unwrap();
        let mxf = write_mxf(dir.path());
        let keys = crate::kdm::ContentKeys::none();

        let (first_frame_only, _) =
            check_picture_j2k_mxf(&mxf, &keys, PictureEssenceFamily::Cinema, false);
        assert!(
            !first_frame_only
                .iter()
                .any(|n| n.code == Code::J2kGuardBits),
            "frame 0 conforms, so the cheap scan must stay silent, got: {first_frame_only:?}"
        );

        let (whole_asset, _) =
            check_picture_j2k_mxf(&mxf, &keys, PictureEssenceFamily::Cinema, true);
        let guard_bits: Vec<_> = whole_asset
            .iter()
            .filter(|n| n.code == Code::J2kGuardBits)
            .collect();
        assert_eq!(
            guard_bits.len(),
            1,
            "one note for the whole asset, got: {whole_asset:?}"
        );
        assert!(
            guard_bits[0]
                .message
                .contains(&format!("frame {BAD_FRAME}")),
            "the note must name the first offending frame, got: {}",
            guard_bits[0].message
        );
        assert_eq!(guard_bits[0].file.as_deref(), Some(&*mxf));
    }

    #[test]
    fn a_parameter_change_partway_through_is_reported_with_its_frame() {
        let dir = tempfile::tempdir().unwrap();
        let mxf = write_picture_mxf(dir.path(), "varying.mxf", &deviating_frames());
        let keys = crate::kdm::ContentKeys::none();

        let (notes, forensics) =
            check_picture_j2k_mxf(&mxf, &keys, PictureEssenceFamily::Cinema, true);
        let forensics = forensics.expect("a readable picture track yields forensics");

        assert_eq!(forensics.frames_scanned, 5, "every frame is scanned");
        assert_eq!(forensics.deviations.len(), 1, "{:?}", forensics.deviations);
        let deviation = &forensics.deviations[0];
        assert_eq!(deviation.parameter, "num_decomp_levels");
        assert_eq!(deviation.first_frame, BAD_FRAME);
        assert_eq!(deviation.frames, 1);
        assert_eq!(
            forensics.reference.info.decomposition_levels, CONFORMANT_DECOMPOSITION_LEVELS,
            "the reference comes from frame 0"
        );
        assert_eq!(
            forensics.worst_frame_index, 1,
            "frame 1 carries the fattest codestream"
        );
        assert!(
            forensics.worst_frame_bytes > FAT_PAYLOAD_BYTES as u64,
            "the worst frame is the fat one, got {} bytes",
            forensics.worst_frame_bytes
        );
        assert_eq!(forensics.reference.tile_part_count, 3);
        assert_eq!(forensics.max_tile_part_count, 3);
        assert_eq!(
            forensics.dci_frame_byte_cap,
            Some(postkit::j2k::dci_codestream_byte_cap(24))
        );
        assert_eq!(forensics.frames_over_dci_cap, 0);

        let varying: Vec<_> = notes
            .iter()
            .filter(|n| n.code == Code::J2kParametersVary)
            .collect();
        assert_eq!(varying.len(), 1, "one note for the asset, got: {notes:?}");
        assert!(
            varying[0].message.contains("num_decomp_levels")
                && varying[0].message.contains(&format!("frame {BAD_FRAME}")),
            "the warning must name the parameter and the first frame, got: {}",
            varying[0].message
        );
    }

    #[test]
    fn a_track_encoded_the_same_throughout_reports_no_deviation() {
        let dir = tempfile::tempdir().unwrap();
        let frames = vec![(CONFORMANT_DECOMPOSITION_LEVELS, FIXTURE_PAYLOAD_BYTES); 5];
        let mxf = write_picture_mxf(dir.path(), "constant.mxf", &frames);

        let (notes, forensics) = check_picture_j2k_mxf(
            &mxf,
            &crate::kdm::ContentKeys::none(),
            PictureEssenceFamily::Cinema,
            true,
        );
        let forensics = forensics.expect("a readable picture track yields forensics");

        assert!(
            forensics.deviations.is_empty(),
            "{:?}",
            forensics.deviations
        );
        assert!(
            !notes.iter().any(|n| n.code == Code::J2kParametersVary),
            "no parameter changed, got: {notes:?}"
        );
    }

    #[test]
    fn the_summary_note_carries_the_reference_parameters_and_the_worst_frame() {
        let dir = tempfile::tempdir().unwrap();
        let frames = vec![(CONFORMANT_DECOMPOSITION_LEVELS, FIXTURE_PAYLOAD_BYTES); 5];
        let mxf = write_picture_mxf(dir.path(), "summary.mxf", &frames);
        let keys = crate::kdm::ContentKeys::none();

        let (notes, _) = check_picture_j2k_mxf(&mxf, &keys, PictureEssenceFamily::Cinema, true);
        let summary = notes
            .iter()
            .find(|n| n.code == Code::J2kCodestreamSummary)
            .expect("the whole-asset scan reports a summary");
        assert_eq!(summary.severity, crate::Severity::Info);
        for expected in [
            "2048x1080",
            "Cinema 2K",
            "5 decomposition levels",
            "1 tile(s)",
            "up to 3 tile-parts",
            "TLM present",
            "no POC",
            "MCT off",
            "parameters identical across 5 frames",
            "% of the 1302083 byte DCI cap",
            "at frame 0 / 00:00:00:00",
        ] {
            assert!(
                summary.message.contains(expected),
                "summary must carry {expected:?}, got: {}",
                summary.message
            );
        }

        let (cheap, _) = check_picture_j2k_mxf(&mxf, &keys, PictureEssenceFamily::Cinema, false);
        assert!(
            !cheap.iter().any(|n| n.code == Code::J2kCodestreamSummary),
            "the frame-0 scan has nothing to summarise, got: {cheap:?}"
        );
    }

    /// IMF picture resolution the DCP path has no profile for.
    const IMF_WIDTH: u32 = 1920;
    const IMF_HEIGHT: u32 = 1080;

    #[test]
    fn a_parameter_change_partway_through_as02_essence_is_reported_with_its_frame() {
        let dir = tempfile::tempdir().unwrap();
        let mxf = write_as02_picture_mxf(
            dir.path(),
            "varying_as02.mxf",
            IMF_WIDTH,
            IMF_HEIGHT,
            &deviating_frames(),
        );

        let (notes, forensics) = check_picture_j2k_mxf(
            &mxf,
            &crate::kdm::ContentKeys::none(),
            PictureEssenceFamily::Imf,
            true,
        );
        let forensics = forensics.expect("a readable AS-02 picture track yields forensics");

        assert_eq!(forensics.frames_scanned, 5, "every frame is scanned");
        assert_eq!(forensics.deviations.len(), 1, "{:?}", forensics.deviations);
        let deviation = &forensics.deviations[0];
        assert_eq!(deviation.parameter, "num_decomp_levels");
        assert_eq!(deviation.first_frame, BAD_FRAME);
        assert_eq!(deviation.frames, 1);
        assert_eq!(forensics.worst_frame_index, 1, "frame 1 is the fat one");
        assert_eq!(forensics.dci_frame_byte_cap, None, "no DCI cap for IMF");

        let varying: Vec<_> = notes
            .iter()
            .filter(|n| n.code == Code::J2kParametersVary)
            .collect();
        assert_eq!(varying.len(), 1, "one note for the asset, got: {notes:?}");
        assert!(
            varying[0].message.contains("num_decomp_levels")
                && varying[0].message.contains(&format!("frame {BAD_FRAME}")),
            "the warning must name the parameter and the first frame, got: {}",
            varying[0].message
        );
    }

    #[test]
    fn constant_as02_essence_draws_no_cinema_profile_notes() {
        let dir = tempfile::tempdir().unwrap();
        let frames = vec![(CONFORMANT_DECOMPOSITION_LEVELS, FIXTURE_PAYLOAD_BYTES); 5];
        let mxf = write_as02_picture_mxf(
            dir.path(),
            "constant_as02.mxf",
            IMF_WIDTH,
            IMF_HEIGHT,
            &frames,
        );

        // the same codestream fails the cinema checks, so silence here is the
        // family gate and not a stream that happens to conform
        let codestream = build_broadcast_j2k(
            IMF_WIDTH,
            IMF_HEIGHT,
            CONFORMANT_DECOMPOSITION_LEVELS,
            FIXTURE_PAYLOAD_BYTES,
        );
        assert!(
            !validate_cinema_j2k(&codestream, 24.0).is_empty(),
            "the fixture codestream must be one a DCP scan rejects"
        );
        assert!(
            guard_bit_violation(&codestream, IMF_WIDTH).is_some(),
            "the fixture codestream must break the RDD 52 guard-bit rule"
        );

        let (notes, forensics) = check_picture_j2k_mxf(
            &mxf,
            &crate::kdm::ContentKeys::none(),
            PictureEssenceFamily::Imf,
            true,
        );
        let forensics = forensics.expect("a readable AS-02 picture track yields forensics");

        assert!(
            forensics.deviations.is_empty(),
            "{:?}",
            forensics.deviations
        );
        let unexpected: Vec<_> = notes
            .iter()
            .filter(|n| n.code != Code::J2kCodestreamSummary)
            .collect();
        assert!(
            unexpected.is_empty(),
            "IMF essence gets the summary and nothing else, got: {unexpected:?}"
        );
    }

    #[test]
    fn the_imf_summary_note_reports_no_dci_cap() {
        let dir = tempfile::tempdir().unwrap();
        let frames = vec![(CONFORMANT_DECOMPOSITION_LEVELS, FIXTURE_PAYLOAD_BYTES); 5];
        let mxf = write_as02_picture_mxf(
            dir.path(),
            "summary_as02.mxf",
            IMF_WIDTH,
            IMF_HEIGHT,
            &frames,
        );

        let (notes, _) = check_picture_j2k_mxf(
            &mxf,
            &crate::kdm::ContentKeys::none(),
            PictureEssenceFamily::Imf,
            true,
        );
        let summary = notes
            .iter()
            .find(|n| n.code == Code::J2kCodestreamSummary)
            .expect("the whole-asset scan reports a summary");
        assert_eq!(summary.severity, crate::Severity::Info);
        for expected in [
            "1920x1080",
            "Broadcast Profile (RSIZ=0x0102)",
            "5 decomposition levels",
            "1 tile(s)",
            "up to 1 tile-parts",
            "parameters identical across 5 frames",
            "at frame 0 / 00:00:00:00",
        ] {
            assert!(
                summary.message.contains(expected),
                "summary must carry {expected:?}, got: {}",
                summary.message
            );
        }
        assert!(
            !summary.message.contains("DCI cap"),
            "IMF essence is held to no DCI cap, got: {}",
            summary.message
        );
    }

    #[test]
    fn stereoscopic_essence_scans_both_eyes() {
        let dir = tempfile::tempdir().unwrap();
        let edit_units = 4;
        let mxf = write_stereoscopic_mxf(dir.path(), edit_units);

        let (notes, forensics) = check_picture_j2k_mxf(
            &mxf,
            &crate::kdm::ContentKeys::none(),
            PictureEssenceFamily::Cinema,
            true,
        );
        let forensics = forensics.expect("stereoscopic picture essence yields forensics");

        assert!(forensics.stereoscopic);
        assert_eq!(
            forensics.frames_scanned,
            edit_units * 2,
            "both eyes of every edit unit are read"
        );
        assert!(forensics.deviations.is_empty(), "both eyes match frame 0");
        assert_eq!(
            forensics.dci_frame_byte_cap,
            Some(postkit::j2k::dci_codestream_byte_cap(48)),
            "each eye gets half an edit unit's byte cap"
        );
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::J2kCodestreamSummary && n.message.contains("both eyes")),
            "the summary must say both eyes were scanned, got: {notes:?}"
        );
    }
}

#[cfg(test)]
mod as02_tests {
    use super::*;
    use postkit::mxf_wrap::{EssenceType, MxfStandard, MxfWrapOptions, mxf_wrap};

    // Wrap a synthetic 2K codestream as AS-02 (OP1a, IMF) picture essence.
    fn write_as02_mxf(dir: &Path, width: u32, height: u32) -> PathBuf {
        let frame = super::cinema_tests::build_j2k(3, width, height, width, height, 1, &[64; 3]);
        let j2c = dir.join("frame.j2c");
        std::fs::write(&j2c, &frame).unwrap();
        let out = dir.join("as02.mxf");
        let result = mxf_wrap(&MxfWrapOptions {
            input_files: vec![j2c],
            output: out.clone(),
            essence_type: EssenceType::J2k,
            standard: MxfStandard::As02,
            fps_num: 24,
            fps_den: 1,
            partition_size: 0,
            encryption: None,
            mca_config: None,
            resource_ids: vec![],
            hdr: None,
            asset_uuid: None,
        });
        assert!(result.success, "AS-02 wrap failed: {}", result.error);
        out
    }

    #[test]
    fn as02_picture_falls_back_to_ffprobe() {
        if std::process::Command::new("ffprobe")
            .arg("-version")
            .output()
            .is_err()
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let mxf = write_as02_mxf(dir.path(), 2048, 1080);

        assert!(
            postkit::j2k::read_mxf_j2k_frame(&mxf, 0).is_err(),
            "the OP-Atom reader must reject AS-02 essence, otherwise this test never reaches the fallback"
        );

        let info = analyze_j2k_from_mxf(&mxf).expect("ffprobe fallback must read the AS-02 MXF");
        assert_eq!((info.width, info.height), (2048, 1080), "dimensions");
        assert_eq!(info.components, 3, "components");
        assert_eq!(info.bit_depth, 12, "bit depth");
        assert!(info.irreversible_transform, "9-7 assumed for DCI");
        assert_eq!(
            info.profile, "Cinema 2K (from MXF)",
            "profile guessed from width"
        );
        assert_eq!(
            info.frame_bytes, 0,
            "ffprobe sees the container size, not a codestream length"
        );
        // the fallback can't see the codestream, so the marker-derived fields stay unset
        assert_eq!(info.rsiz, 0, "rsiz is not visible to ffprobe");
        assert_eq!(
            info.decomposition_levels, 0,
            "decomposition levels are not visible to ffprobe"
        );
    }

    #[test]
    fn as02_4k_profile_guess_follows_width() {
        if std::process::Command::new("ffprobe")
            .arg("-version")
            .output()
            .is_err()
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let mxf = write_as02_mxf(dir.path(), 3840, 2160);
        let info = analyze_j2k_from_mxf(&mxf).expect("ffprobe fallback must read the AS-02 MXF");
        assert_eq!((info.width, info.height), (3840, 2160), "dimensions");
        assert_eq!(
            info.profile, "Cinema 4K (from MXF)",
            "profile guessed from width"
        );
    }
}

#[cfg(test)]
mod guard_bit_tests {
    use super::*;

    // minimal main header: SOC, QCD (Sqcd carrying `gb` guard bits, no quant),
    // then SOT so the walk terminates like a real stream.
    fn codestream(gb: u8) -> Vec<u8> {
        let mut d = vec![0xFF, 0x4F]; // SOC
        d.extend_from_slice(&[0xFF, 0x5C]); // QCD
        d.extend_from_slice(&[0x00, 0x04]); // Lqcd = 4 (len + Sqcd + 1 SPqcd)
        d.push(gb << 5); // Sqcd: guard bits in high 3 bits, no quantization
        d.push(0x00); // one SPqcd byte
        d.extend_from_slice(&[0xFF, 0x90]); // SOT
        d
    }

    #[test]
    fn reads_guard_bits() {
        assert_eq!(qcd_guard_bits(&codestream(1)), Some(1));
        assert_eq!(qcd_guard_bits(&codestream(2)), Some(2));
    }

    #[test]
    fn no_qcd_is_none() {
        // SOC then SOT with no QCD in between
        let d = vec![0xFF, 0x4F, 0xFF, 0x90];
        assert_eq!(qcd_guard_bits(&d), None);
    }

    #[test]
    fn conforming_frames_are_silent() {
        assert_eq!(guard_bit_violation(&codestream(1), 2048), None); // 2K wants 1
        assert_eq!(guard_bit_violation(&codestream(2), 4096), None); // 4K wants 2
    }

    #[test]
    fn wrong_guard_bits_fire() {
        assert_eq!(guard_bit_violation(&codestream(1), 4096), Some((2, 1)));
        assert_eq!(guard_bit_violation(&codestream(0), 2048), Some((1, 0)));
    }

    #[test]
    fn timecode_from_frame() {
        assert_eq!(frame_to_timecode(0, 24), "00:00:00:00");
        assert_eq!(frame_to_timecode(25, 24), "00:00:01:01");
        assert_eq!(frame_to_timecode(24 * 3661 + 5, 24), "01:01:01:05");
    }
}
