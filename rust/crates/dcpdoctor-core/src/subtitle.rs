/// Subtitle/timed text validation (SMPTE ST 428-7 DCST and Interop).
use crate::{Code, Note, Severity, Standard};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Validate a subtitle/timed text XML file held on disk.
pub fn validate_subtitle(file: &Path, standard: Standard) -> Vec<Note> {
    let xml = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            return vec![Note {
                severity: Severity::Error,
                code: Code::SubtitleParseError,
                message: format!("Failed to read subtitle file: {e}"),
                file: Some(file.to_path_buf()),
                line: 0,
            }];
        }
    };
    validate_subtitle_xml(&xml, file, standard)
}

/// The same structural rules against a document already in memory, so an
/// MXF-wrapped ST 429-5 asset gets everything a loose XML one does. `file` is
/// the path findings name: the XML asset, or the MXF it was unwrapped from.
pub fn validate_subtitle_xml(xml: &str, file: &Path, standard: Standard) -> Vec<Note> {
    let mut notes = Vec::new();

    // Check if the namespace matches the standard. ST 428-7 §6.1: the DCST
    // namespace name shall be the fixed string, so a wrong namespace makes the
    // document non-conformant and unparseable by a compliant player (ERROR).
    match standard {
        Standard::Smpte => {
            // ST 428-7 has three published namespaces and a package may use any
            // of them; accepting only one would reject a conformant 2007 or 2014
            // document, which nothing caught while this ran on loose XML only.
            if !crate::schema::smpte_subtitle_namespaces().any(|ns| xml.contains(ns)) {
                notes.push(err(
                    Code::SmpteNamespaceWrong,
                    "Subtitle file does not use a SMPTE ST 428-7 namespace".into(),
                    file,
                ));
            }
        }
        Standard::Interop => {
            // Interop DCSubtitle is the DTD-era format and carries no namespace;
            // only flag a doc that is neither a DCSubtitle nor the namespaced form.
            if !xml.contains("<DCSubtitle")
                && !xml.contains("http://www.digicine.com/PROTO-ASDCP-TT-DEF")
            {
                notes.push(err(
                    Code::InteropNamespaceWrong,
                    "Subtitle file does not use Interop namespace".into(),
                    file,
                ));
            }
        }
        Standard::Unknown => {}
    }

    check_structure(xml, file, standard, &mut notes);

    notes
}

fn warn(code: Code, msg: &str, file: &Path) -> Note {
    Note {
        severity: Severity::Warning,
        code,
        message: msg.to_string(),
        file: Some(file.to_path_buf()),
        line: 0,
    }
}

fn err(code: Code, msg: String, file: &Path) -> Note {
    Note {
        severity: Severity::Error,
        code,
        message: msg,
        file: Some(file.to_path_buf()),
        line: 0,
    }
}

struct Span {
    time_in: u64,
    time_out: u64,
    spot: String,
    raw_in: String,
    raw_out: String,
}

#[derive(Default)]
struct Scan {
    is_subtitle: bool,
    has_reel_number: bool,
    has_language: bool,
    has_load_font: bool,
    has_subtitle_id: bool,
    has_text: bool,
    capture_id: bool,
    /// text of the document's `StartTime`, absent when the element is not there
    start_time: Option<String>,
    capture_start_time: bool,
    /// text of the document's `IssueDate`, absent when the element is not there
    issue_date: Option<String>,
    capture_issue_date: bool,
    /// `xmlns` declarations on the root element
    root_namespaces: usize,
    seen_root: bool,
    /// `<Subtitle>` elements seen, whether or not their times parsed
    subtitle_count: usize,
    /// LoadFont ids in document order, which a later `<Font>` must name
    load_font_ids: Vec<String>,
    /// first `<Font>` naming an id no earlier `<LoadFont>` introduced
    font_without_load_font: Option<String>,
    /// nesting depth inside a `<Text>`, and whether this one has said anything
    text_depth: u32,
    text_has_content: bool,
    has_empty_text: bool,
    spans: Vec<Span>,
}

impl Scan {
    fn on_start(&mut self, e: &BytesStart, empty: bool) {
        let qname = e.name();
        let local = qname.local_name();
        let name = String::from_utf8_lossy(local.as_ref()).into_owned();

        self.capture_id = false;
        self.capture_start_time = false;
        self.capture_issue_date = false;
        match name.as_str() {
            // SMPTE DCST root or Interop root
            "SubtitleReel" | "DCSubtitle" => {
                self.is_subtitle = true;
                if !self.seen_root {
                    self.seen_root = true;
                    self.root_namespaces = namespace_declarations(e);
                }
                // Interop carries the identifier as a SubtitleID attribute
                if attr(e, "SubtitleID").is_some_and(|v| !v.trim().is_empty()) {
                    self.has_subtitle_id = true;
                }
            }
            "ReelNumber" => self.has_reel_number = true,
            "Language" => self.has_language = true,
            "LoadFont" => {
                self.has_load_font = true;
                if let Some(id) = font_id(e) {
                    self.load_font_ids.push(id);
                }
            }
            "Font" => {
                // a Font may only name a font an earlier LoadFont introduced.
                // libdcp reads only the lowercase `Id` here, so a document using
                // the ST 428-7 `ID` spelling gets past it.
                if let Some(id) = font_id(e)
                    && !self.load_font_ids.contains(&id)
                    && self.font_without_load_font.is_none()
                {
                    self.font_without_load_font = Some(id);
                }
            }
            "Text" => {
                self.has_text = true;
                if empty {
                    self.has_empty_text = true;
                } else {
                    if self.text_depth == 0 {
                        self.text_has_content = false;
                    }
                    self.text_depth += 1;
                }
            }
            // SMPTE DCST uses <Id>; Interop DCSubtitle uses a <SubtitleID> element
            "Id" | "SubtitleID" => self.capture_id = true,
            "StartTime" => {
                self.capture_start_time = true;
                // an empty element still counts as present, and still not zero
                self.start_time.get_or_insert_default();
            }
            "IssueDate" => {
                self.capture_issue_date = true;
                self.issue_date.get_or_insert_default();
            }
            "Subtitle" => {
                self.subtitle_count += 1;
                let spot = attr(e, "SpotNumber").unwrap_or_default();
                let raw_in = attr(e, "TimeIn").unwrap_or_default();
                let raw_out = attr(e, "TimeOut").unwrap_or_default();
                if let (Some(time_in), Some(time_out)) = (parse_time(&raw_in), parse_time(&raw_out))
                {
                    self.spans.push(Span {
                        time_in,
                        time_out,
                        spot,
                        raw_in,
                        raw_out,
                    });
                }
            }
            _ => {}
        }
    }

    fn on_end(&mut self, name: &str) {
        if name == "Text" && self.text_depth > 0 {
            self.text_depth -= 1;
            if self.text_depth == 0 && !self.text_has_content {
                self.has_empty_text = true;
            }
        }
    }

    fn on_text(&mut self, text: &str) {
        // whitespace counts as content, as in libdcp: DCP-o-matic fills a reel
        // without subtitles with a one-space cue
        if self.text_depth > 0 && !text.is_empty() {
            self.text_has_content = true;
        }
    }
}

/// The `ID` a LoadFont or Font declares. ST 428-7 spells it `ID`, Interop `Id`.
fn font_id(e: &BytesStart) -> Option<String> {
    attr(e, "ID")
        .or_else(|| attr(e, "Id"))
        .filter(|id| !id.trim().is_empty())
}

/// Count the `xmlns` declarations an element carries.
fn namespace_declarations(e: &BytesStart) -> usize {
    e.attributes()
        .flatten()
        .filter(|a| {
            let key = a.key.as_ref();
            key == b"xmlns" || key.starts_with(b"xmlns:")
        })
        .count()
}

fn check_structure(xml: &str, file: &Path, standard: Standard, notes: &mut Vec<Note>) {
    let mut reader = Reader::from_str(xml);
    let mut scan = Scan::default();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => scan.on_start(&e, false),
            Ok(Event::Empty(e)) => scan.on_start(&e, true),
            Ok(Event::End(e)) => {
                let name = String::from_utf8_lossy(e.name().local_name().as_ref()).into_owned();
                scan.on_end(&name);
            }
            Ok(Event::Text(e)) => {
                let text = dcpdoctor_parse::text_of(&e);
                if scan.capture_id {
                    scan.capture_id = false;
                    if !text.trim().is_empty() {
                        scan.has_subtitle_id = true;
                    }
                }
                if scan.capture_start_time {
                    scan.capture_start_time = false;
                    scan.start_time = Some(text.trim().to_string());
                }
                if scan.capture_issue_date {
                    scan.capture_issue_date = false;
                    scan.issue_date = Some(text.trim().to_string());
                }
                scan.on_text(&text);
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                notes.push(err(
                    Code::SubtitleParseError,
                    format!("Subtitle XML parse error: {e}"),
                    file,
                ));
                return;
            }
            _ => {}
        }
    }

    // only run structural checks on actual subtitle documents
    if !scan.is_subtitle {
        notes.push(err(
            Code::SubtitleParseError,
            "File is not a recognized subtitle reel (no SubtitleReel/DCSubtitle root)".into(),
            file,
        ));
        return;
    }

    if !scan.has_subtitle_id {
        notes.push(err(
            Code::MissingRequiredElement,
            "Subtitle reel is missing its identifier (SubtitleID/Id)".into(),
            file,
        ));
    }
    if !scan.has_reel_number {
        notes.push(warn(
            Code::MissingRequiredElement,
            "Missing ReelNumber",
            file,
        ));
    }
    if !scan.has_language {
        notes.push(warn(Code::MissingRequiredElement, "Missing Language", file));
    }
    if !scan.has_load_font {
        // ST 428-7:2014: when one or more Text elements are present, at least one
        // LoadFont shall also be present (image-only subtitles may omit it).
        if scan.has_text {
            notes.push(err(
                Code::SubtitleFontMissing,
                "Subtitle has Text but no LoadFont".into(),
                file,
            ));
        } else {
            notes.push(warn(Code::SubtitleFontMissing, "Missing LoadFont", file));
        }
    }

    notes.extend(start_time_notes(&scan, file, standard));
    notes.extend(issue_date_notes(&scan, file, standard));

    // ST 428-7 and Interop both declare one namespace on the root; a second one
    // means the document was assembled from two schema versions, and players
    // disagree about which wins.
    if scan.root_namespaces > 1 {
        notes.push(warn(
            Code::SubtitleNamespaceCount,
            &format!(
                "Subtitle root declares {} namespaces, which must be 1",
                scan.root_namespaces
            ),
            file,
        ));
    }

    // an Interop asset with no cues at all displays nothing, and nothing
    // downstream notices because the reel still has its duration
    if standard == Standard::Interop && scan.subtitle_count == 0 {
        notes.push(err(
            Code::MissingRequiredElement,
            "Interop subtitle asset contains no subtitles".into(),
            file,
        ));
    }

    if scan.has_empty_text {
        notes.push(err(
            Code::SubtitleEmptyText,
            "Subtitle has a <Text> element with no content".into(),
            file,
        ));
    }

    if let Some(id) = &scan.font_without_load_font {
        notes.push(err(
            Code::SubtitleFontMissing,
            format!("<Font> names '{id}', which no <LoadFont> introduces"),
            file,
        ));
    }

    // TimeIn must precede TimeOut on each cue
    for s in &scan.spans {
        if s.time_in >= s.time_out {
            notes.push(err(
                Code::SubtitleInvalidTiming,
                format!(
                    "Subtitle {}: TimeIn ({}) is not before TimeOut ({})",
                    spot_label(&s.spot),
                    s.raw_in,
                    s.raw_out
                ),
                file,
            ));
        }
    }

    // overlap detection: sort by start, flag any cue starting before the previous ends
    let mut ordered: Vec<&Span> = scan.spans.iter().collect();
    ordered.sort_by_key(|s| s.time_in);
    for pair in ordered.windows(2) {
        let (prev, cur) = (pair[0], pair[1]);
        if cur.time_in < prev.time_out {
            notes.push(warn(
                Code::SubtitleInvalidTiming,
                &format!(
                    "Subtitle {} overlaps subtitle {} (starts {} before previous ends {})",
                    spot_label(&cur.spot),
                    spot_label(&prev.spot),
                    cur.raw_in,
                    prev.raw_out
                ),
                file,
            ));
        }
    }
}

/// ST 428-7 requires a SMPTE timed-text document to declare `StartTime`, and
/// Bv2.1 §7.2.2 requires it to be zero: a non-zero value shifts every cue in the
/// reel by that amount, which is the classic reason a package plays with its
/// subtitles out of sync. Interop documents have no such element.
fn start_time_notes(scan: &Scan, file: &Path, standard: Standard) -> Vec<Note> {
    if standard != Standard::Smpte {
        return Vec::new();
    }
    let Some(start_time) = &scan.start_time else {
        return vec![err(
            Code::MissingRequiredElement,
            "SMPTE timed text has no <StartTime>".into(),
            file,
        )];
    };
    if parse_time(start_time) == Some(0) {
        return Vec::new();
    }
    vec![err(
        Code::SubtitleInvalidTiming,
        format!(
            "<StartTime> is '{start_time}', which shifts every cue; Bv2.1 requires 00:00:00:000"
        ),
        file,
    )]
}

/// No standard requires a particular `IssueDate` form, but Deluxe QC rejects a
/// SMPTE package whose date is not `yyyy-mm-ddThh:mm:ss`, so libdcp warns and so
/// do we. A missing date cannot be checked for form at all, which is why absence
/// warns here even though libdcp leaves it to the schema.
fn issue_date_notes(scan: &Scan, file: &Path, standard: Standard) -> Vec<Note> {
    if standard != Standard::Smpte {
        return Vec::new();
    }
    let Some(issue_date) = &scan.issue_date else {
        return vec![warn(
            Code::SubtitleInvalidIssueDate,
            "SMPTE timed text has no <IssueDate>",
            file,
        )];
    };
    if is_deluxe_issue_date(issue_date) {
        return Vec::new();
    }
    vec![warn(
        Code::SubtitleInvalidIssueDate,
        &format!("<IssueDate> is '{issue_date}', not yyyy-mm-ddThh:mm:ss"),
        file,
    )]
}

/// `yyyy-mm-ddThh:mm:ss`, with no timezone offset and no fractional seconds.
fn is_deluxe_issue_date(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 19 {
        return false;
    }
    const DIGITS: [usize; 14] = [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18];
    const DASHES: [usize; 2] = [4, 7];
    const COLONS: [usize; 2] = [13, 16];
    DIGITS.iter().all(|&i| bytes[i].is_ascii_digit())
        && DASHES.iter().all(|&i| bytes[i] == b'-')
        && COLONS.iter().all(|&i| bytes[i] == b':')
        && bytes[10] == b'T'
}

fn spot_label(spot: &str) -> String {
    if spot.is_empty() {
        "(unnumbered)".into()
    } else {
        format!("#{spot}")
    }
}

fn attr(e: &BytesStart, name: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        let key = a.key.local_name();
        if key.as_ref() == name.as_bytes() {
            let raw = String::from_utf8_lossy(&a.value);
            Some(
                quick_xml::escape::unescape(&raw)
                    .map(|c| c.to_string())
                    .unwrap_or_else(|_| raw.into_owned()),
            )
        } else {
            None
        }
    })
}

/// Parse a subtitle timecode into an order-preserving key.
///
/// Accepts `HH:MM:SS:FFF` (editable units / frames) and `HH:MM:SS.mmm`.
/// The key preserves ordering within a single file (consistent tick base).
fn parse_time(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    // 4-field colon form splits the fractional (ticks/frames) off the tail
    let (hms, tick) = if s.matches(':').count() == 3 {
        let idx = s.rfind(':').unwrap();
        (&s[..idx], Some(&s[idx + 1..]))
    } else {
        (s, None)
    };

    let parts: Vec<&str> = hms.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let h: u64 = parts[0].parse().ok()?;
    let m: u64 = parts[1].parse().ok()?;
    let (sec, milli) = match parts[2].split_once('.') {
        Some((s1, s2)) => (s1.parse::<u64>().ok()?, s2),
        None => (parts[2].parse::<u64>().ok()?, ""),
    };

    let frac: u64 = if let Some(t) = tick {
        t.parse().ok()?
    } else if !milli.is_empty() {
        let take = &milli[..milli.len().min(3)];
        format!("{take:0<3}").parse().unwrap_or(0)
    } else {
        0
    };

    Some((((h * 60) + m) * 60 + sec) * 100_000 + frac)
}

/// A font declared by a subtitle document via LoadFont.
///
/// SMPTE ST 428-7 carries the font asset id as the element text (`<LoadFont
/// ID="f">urn:uuid:...</LoadFont>`); Interop DCSubtitle carries a file reference
/// in the URI attribute (`<LoadFont Id="f" URI="arial.ttf"/>`).
#[derive(Debug, Clone, Default)]
pub struct FontDecl {
    pub id: String,
    pub urn: Option<String>,
    pub uri: Option<String>,
}

/// first cue a code point appears in, and how many cues use it.
#[derive(Default)]
struct Usage {
    time: String,
    spot: String,
    count: u32,
}

#[derive(Default)]
struct GlyphScan {
    fonts: HashMap<String, FontDecl>,
    font_stack: Vec<String>,
    capture_urn_for: Option<String>,
    cur_time: Option<String>,
    cur_spot: String,
    usage: HashMap<(String, char), Usage>,
}

impl GlyphScan {
    fn active_font(&self) -> Option<String> {
        if let Some(f) = self.font_stack.last() {
            return Some(f.clone());
        }
        // a document with a single font needn't wrap text in a Font element
        if self.fonts.len() == 1 {
            return self.fonts.keys().next().cloned();
        }
        None
    }

    fn on_start(&mut self, e: &BytesStart, empty: bool) {
        let qname = e.name();
        let name = String::from_utf8_lossy(qname.local_name().as_ref()).into_owned();
        match name.as_str() {
            "LoadFont" => {
                let id = attr(e, "ID").or_else(|| attr(e, "Id")).unwrap_or_default();
                let uri = attr(e, "URI");
                let decl = FontDecl {
                    id: id.clone(),
                    urn: None,
                    uri,
                };
                self.fonts.insert(id.clone(), decl);
                // SMPTE keeps the asset id as element text, read on the next Text
                if !empty {
                    self.capture_urn_for = Some(id);
                }
            }
            "Font" => {
                let id = attr(e, "ID")
                    .or_else(|| attr(e, "Id"))
                    .or_else(|| self.font_stack.last().cloned())
                    .unwrap_or_default();
                if !empty {
                    self.font_stack.push(id);
                }
            }
            "Subtitle" => {
                self.cur_spot = attr(e, "SpotNumber").unwrap_or_default();
                self.cur_time = attr(e, "TimeIn");
            }
            _ => {}
        }
    }

    fn on_end(&mut self, name: &str) {
        match name {
            "Font" => {
                self.font_stack.pop();
            }
            "Subtitle" => {
                self.cur_time = None;
                self.cur_spot.clear();
            }
            _ => {}
        }
    }

    fn on_text(&mut self, text: &str) {
        if let Some(id) = self.capture_urn_for.take() {
            let t = text.trim();
            if !t.is_empty()
                && let Some(f) = self.fonts.get_mut(&id)
            {
                f.urn = Some(t.to_string());
            }
            return;
        }
        let (Some(time), Some(font)) = (self.cur_time.clone(), self.active_font()) else {
            return;
        };
        for c in text.chars() {
            if c.is_whitespace() || c.is_control() {
                continue;
            }
            let e = self
                .usage
                .entry((font.clone(), c))
                .or_insert_with(|| Usage {
                    time: time.clone(),
                    spot: self.cur_spot.clone(),
                    count: 0,
                });
            e.count += 1;
        }
    }
}

/// Check that every code point used in a subtitle document has a glyph in its
/// referenced font (dom#3080, dom#838). `resolve_font` maps a LoadFont
/// declaration to an on-disk font file; fonts that don't resolve are skipped
/// silently (the structural check already warns on a missing LoadFont).
pub fn check_glyph_coverage(
    file: &Path,
    resolve_font: impl Fn(&FontDecl) -> Option<PathBuf>,
) -> Vec<Note> {
    let xml = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    glyph_notes(&xml, file, |decl| {
        resolve_font(decl).and_then(|p| std::fs::read(p).ok())
    })
}

/// Glyph coverage for a wrapped asset already read out of its MXF: check every
/// used code point against the embedded font a LoadFont urn resolves to.
pub fn check_glyph_coverage_wrapped(asset: &WrappedTimedText, source: &Path) -> Vec<Note> {
    glyph_notes(&asset.xml, source, |decl| {
        decl.urn
            .as_deref()
            .and_then(parse_urn_uuid)
            .and_then(|u| asset.fonts.get(&u).cloned())
    })
}

/// Everything a SMPTE ST 429-5 wrapped timed-text asset holds: the document, its
/// embedded OpenType fonts keyed by resource uuid, the ResourceID the descriptor
/// declares and the duration the essence carries. Read once so the structural,
/// schema, identity and glyph passes share one trip through the MXF.
#[derive(Debug, Default)]
pub struct WrappedTimedText {
    pub xml: String,
    pub fonts: HashMap<[u8; 16], Vec<u8>>,
    /// Total bytes of the embedded fonts, which Bv2.1 caps.
    pub font_bytes: usize,
    /// `AssetID` of the timed-text descriptor, which ST 429-5 calls the
    /// ResourceID and requires to equal the document's own `<Id>`.
    pub resource_id: [u8; 16],
    /// `AssetUUID` from the MXF header, the id the CPL and PKL reference.
    pub asset_id: [u8; 16],
    /// Frames of essence the container declares.
    pub container_duration: u32,
    /// Decryption or skip notes produced while reading.
    pub notes: Vec<Note>,
}

impl WrappedTimedText {
    /// True when the essence could not be read (encrypted with no covering key),
    /// so callers skip the document-level rules rather than reporting on nothing.
    pub fn is_unreadable(&self) -> bool {
        self.xml.is_empty()
    }
}

/// Whether a read needs the embedded fonts, which are the bulk of a timed-text
/// MXF. The document alone answers every rule except glyph coverage and the font
/// size cap, so the reel-level checks omit them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontData {
    Include,
    Omit,
}

/// Read a timed-text MXF: its document, identity fields, duration and (when
/// asked) its embedded OpenType fonts, decrypting with `keys` where the essence
/// is encrypted. `None` when the MXF can't be opened or its document can't be
/// read. An asset encrypted with no covering key comes back with its identity
/// fields but an empty document, so callers can tell "unreadable" from "absent".
/// Non-font resources (Png, Binary) are ignored for glyph coverage but still
/// count toward the font byte total.
pub fn read_wrapped_timed_text(
    mxf_path: &Path,
    keys: &crate::kdm::ContentKeys,
    fonts: FontData,
) -> Option<WrappedTimedText> {
    let s = mxf_path.to_str()?;
    let mut reader = asdcplib::timed_text::MxfReader::new();
    reader.open_read(s).ok()?;

    let info = reader.writer_info().ok()?;
    let descriptor = reader.descriptor().ok()?;
    let identity = WrappedTimedText {
        resource_id: descriptor.asset_id,
        asset_id: info.asset_uuid,
        container_duration: descriptor.container_duration,
        ..Default::default()
    };

    let essence = keys.resolve(&info);
    if essence.is_missing() {
        // encrypted with no key: the identity fields are still readable, but the
        // document is not, so hand back what we have plus the skip note (which
        // only exists when a KDM was supplied).
        return Some(WrappedTimedText {
            notes: essence.skip_note(mxf_path).into_iter().collect(),
            ..identity
        });
    }
    let mut ctx = essence.contexts().ok()?;

    let mut buf: Vec<u8> = Vec::new();
    let (dec, hmac) = split_ctx(ctx.as_mut());
    let xml = match reader.read_timed_text_resource(&mut buf, dec, hmac) {
        Ok(n) => String::from_utf8_lossy(&buf[..n]).into_owned(),
        Err(asdcplib::Error::BufferTooSmall { needed, .. }) => {
            buf = vec![0u8; needed];
            let (dec, hmac) = split_ctx(ctx.as_mut());
            let n = reader.read_timed_text_resource(&mut buf, dec, hmac).ok()?;
            String::from_utf8_lossy(&buf[..n]).into_owned()
        }
        Err(_) => return None,
    };

    let mut font_data: HashMap<[u8; 16], Vec<u8>> = HashMap::new();
    let mut font_bytes = 0usize;
    let count = if fonts == FontData::Include {
        reader.ancillary_resource_count().ok()?
    } else {
        0
    };
    for i in 0..count {
        let Ok(info) = reader.ancillary_resource_info(i) else {
            continue;
        };
        let is_font = info.mime_type == asdcplib::timed_text::MimeType::OpenType;
        let mut fbuf: Vec<u8> = Vec::new();
        let (dec, hmac) = split_ctx(ctx.as_mut());
        let bytes = match reader.read_ancillary_resource(&info.uuid, &mut fbuf, dec, hmac) {
            Ok(n) => fbuf[..n].to_vec(),
            Err(asdcplib::Error::BufferTooSmall { needed, .. }) => {
                fbuf = vec![0u8; needed];
                let (dec, hmac) = split_ctx(ctx.as_mut());
                match reader.read_ancillary_resource(&info.uuid, &mut fbuf, dec, hmac) {
                    Ok(n) => fbuf[..n].to_vec(),
                    Err(_) => continue, // unreadable resource: skip
                }
            }
            Err(_) => continue,
        };
        if !is_font {
            continue; // Png bitmap subs / Binary are not fonts
        }
        font_bytes += bytes.len();
        font_data.insert(info.uuid, bytes);
    }
    Some(WrappedTimedText {
        xml,
        fonts: font_data,
        font_bytes,
        ..identity
    })
}

/// Split optional decrypt contexts into the (dec, hmac) pair the readers take.
fn split_ctx(
    ctx: Option<&mut crate::kdm::DecryptContexts>,
) -> (
    Option<&mut asdcplib::crypto::AesDecContext>,
    Option<&mut asdcplib::crypto::HmacContext>,
) {
    match ctx {
        Some(c) => (Some(&mut c.dec), Some(&mut c.hmac)),
        None => (None, None),
    }
}

/// The `<Id>` a timed-text document declares, as raw uuid bytes. ST 429-5 calls
/// this the resource id and requires the MXF descriptor to repeat it. Reads the
/// first `Id` element, which in both DCST and DCSubtitle is the document's own.
pub fn document_id(xml: &str) -> Option<[u8; 16]> {
    let mut reader = Reader::from_str(xml);
    let mut in_id = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.name().local_name().as_ref()).into_owned();
                in_id = name == "Id" || name == "SubtitleID";
            }
            Ok(Event::Text(e)) if in_id => {
                return parse_urn_uuid(&dcpdoctor_parse::text_of(&e));
            }
            Ok(Event::Eof) | Err(_) => return None,
            _ => {}
        }
    }
}

/// Parse `urn:uuid:xxxxxxxx-xxxx-...` into its 16 raw bytes. Returns `None` on a
/// urn that isn't a well-formed uuid. An Interop `SubtitleID` carries the bare
/// uuid with no urn prefix, so that form is accepted too.
fn parse_urn_uuid(urn: &str) -> Option<[u8; 16]> {
    let trimmed = urn.trim();
    let hex = trimmed
        .strip_prefix("urn:uuid:")
        .unwrap_or(trimmed)
        .replace('-', "");
    if hex.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// Parse a subtitle document and warn on any used code point missing from its
/// resolved font. `resolve_font` hands back the font's raw bytes (from disk for
/// plain XML, from the MXF for wrapped subs); fonts that don't resolve are
/// skipped silently.
fn glyph_notes(
    xml: &str,
    file: &Path,
    resolve_font: impl Fn(&FontDecl) -> Option<Vec<u8>>,
) -> Vec<Note> {
    use skrifa::{FontRef, MetadataProvider};

    let mut reader = Reader::from_str(xml);
    let mut scan = GlyphScan::default();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => scan.on_start(&e, false),
            Ok(Event::Empty(e)) => scan.on_start(&e, true),
            Ok(Event::End(e)) => {
                let n = String::from_utf8_lossy(e.name().local_name().as_ref()).into_owned();
                scan.on_end(&n);
            }
            Ok(Event::Text(e)) => {
                let t = dcpdoctor_parse::text_of(&e);
                scan.on_text(&t);
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    // load and cache each resolvable font's byte data once
    let mut font_bytes: HashMap<String, Option<Vec<u8>>> = HashMap::new();
    for (id, decl) in &scan.fonts {
        font_bytes.insert(id.clone(), resolve_font(decl));
    }

    // stable output: sort by (font id, spot, code point)
    let mut used: Vec<(&(String, char), &Usage)> = scan.usage.iter().collect();
    used.sort_by(|a, b| (&a.0.0, &a.1.spot, a.0.1 as u32).cmp(&(&b.0.0, &b.1.spot, b.0.1 as u32)));

    let mut notes = Vec::new();
    for ((font_id, ch), usage) in used {
        let Some(Some(bytes)) = font_bytes.get(font_id) else {
            continue; // unresolvable font: skip silently
        };
        let Ok(face) = FontRef::new(bytes) else {
            continue;
        };
        let cmap = face.charmap();
        let present = cmap.map(*ch).is_some_and(|g| g.to_u32() != 0);
        if !present {
            let more = if usage.count > 1 {
                format!(" ({} cues)", usage.count)
            } else {
                String::new()
            };
            let (time, spot) = (&usage.time, &usage.spot);
            notes.push(Note {
                severity: Severity::Warning,
                code: Code::SubtitleGlyphMissing,
                message: format!(
                    "Subtitle {} ({time}): character {:?} (U+{:04X}) has no glyph in font '{font_id}'{more}",
                    spot_label(spot),
                    ch,
                    *ch as u32,
                ),
                file: Some(file.to_path_buf()),
                line: 0,
            });
        }
    }
    notes
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    fn write_tmp(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::Builder::new().suffix(".xml").tempfile().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    /// A conformant ST 428-7 document in the namespace given: every element the
    /// DCST schema requires, one namespace on the root, a zero `StartTime` and an
    /// `IssueDate` in the form Deluxe QC demands. The schema tests validate this
    /// same document against the vendored XSDs, so one fixture has to satisfy
    /// both the structural rules here and the real schema.
    pub(crate) fn smpte_subtitle_doc(namespace: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<dcst:SubtitleReel xmlns:dcst="{namespace}">
  <dcst:Id>urn:uuid:22222222-2222-3333-4444-555555555555</dcst:Id>
  <dcst:ContentTitleText>Test</dcst:ContentTitleText>
  <dcst:IssueDate>2024-01-01T00:00:00</dcst:IssueDate>
  <dcst:ReelNumber>1</dcst:ReelNumber>
  <dcst:Language>en</dcst:Language>
  <dcst:EditRate>24 1</dcst:EditRate>
  <dcst:TimeCodeRate>24</dcst:TimeCodeRate>
  <dcst:StartTime>00:00:00:000</dcst:StartTime>
  <dcst:LoadFont ID="theFont">urn:uuid:33333333-2222-3333-4444-555555555555</dcst:LoadFont>
  <dcst:SubtitleList>
    <dcst:Font ID="theFont">
      <dcst:Subtitle SpotNumber="1" TimeIn="00:00:05:000" TimeOut="00:00:07:000">
        <dcst:Text>Hello</dcst:Text>
      </dcst:Subtitle>
      <dcst:Subtitle SpotNumber="2" TimeIn="00:00:08:000" TimeOut="00:00:10:000">
        <dcst:Text>Again</dcst:Text>
      </dcst:Subtitle>
    </dcst:Font>
  </dcst:SubtitleList>
</dcst:SubtitleReel>"#
        )
    }

    /// The same document in the 2010 namespace, which most cases here mutate.
    fn valid_smpte() -> String {
        smpte_subtitle_doc("http://www.smpte-ra.org/schemas/428-7/2010/DCST")
    }

    #[test]
    fn valid_smpte_has_no_findings() {
        let f = write_tmp(&valid_smpte());
        let notes = validate_subtitle(f.path(), Standard::Smpte);
        assert!(notes.is_empty(), "expected clean, got: {notes:?}");
    }

    #[test]
    fn missing_start_time_is_error() {
        let stripped =
            valid_smpte().replace("  <dcst:StartTime>00:00:00:000</dcst:StartTime>\n", "");
        let f = write_tmp(&stripped);
        let notes = validate_subtitle(f.path(), Standard::Smpte);
        assert!(
            notes.iter().any(|n| n.code == Code::MissingRequiredElement
                && n.severity == Severity::Error
                && n.message.contains("StartTime")),
            "got: {notes:?}"
        );
    }

    // a non-zero StartTime shifts every cue in the reel, which is the usual
    // reason a package plays with its subtitles out of sync
    #[test]
    fn non_zero_start_time_is_error() {
        let shifted = valid_smpte().replace(
            "<dcst:StartTime>00:00:00:000</dcst:StartTime>",
            "<dcst:StartTime>00:00:04:000</dcst:StartTime>",
        );
        let f = write_tmp(&shifted);
        let notes = validate_subtitle(f.path(), Standard::Smpte);
        assert!(
            notes.iter().any(|n| n.code == Code::SubtitleInvalidTiming
                && n.severity == Severity::Error
                && n.message.contains("00:00:04:000")),
            "got: {notes:?}"
        );
    }

    // Interop has no StartTime element, so the SMPTE rule must not reach it
    #[test]
    fn interop_is_not_asked_for_a_start_time() {
        let interop = r#"<?xml version="1.0"?>
<DCSubtitle Version="1.0" xmlns="http://www.digicine.com/PROTO-ASDCP-TT-DEF"
    SubtitleID="urn:uuid:abcd">
  <ReelNumber>1</ReelNumber>
  <Language>en</Language>
  <LoadFont Id="Arial" URI="arial.ttf"/>
  <Font Id="Arial">
    <Subtitle SpotNumber="1" TimeIn="00:00:05:000" TimeOut="00:00:07:000"/>
  </Font>
</DCSubtitle>"#;
        let f = write_tmp(interop);
        let notes = validate_subtitle(f.path(), Standard::Interop);
        assert!(notes.is_empty(), "expected clean, got: {notes:?}");
    }

    #[test]
    fn a_malformed_issue_date_warns() {
        // the offset form real tools write, which Deluxe QC rejects
        let offset = valid_smpte().replace(
            "<dcst:IssueDate>2024-01-01T00:00:00</dcst:IssueDate>",
            "<dcst:IssueDate>2024-01-01T00:00:00.000-00:00</dcst:IssueDate>",
        );
        let f = write_tmp(&offset);
        let notes = validate_subtitle(f.path(), Standard::Smpte);
        assert!(
            notes.iter().any(
                |n| n.code == Code::SubtitleInvalidIssueDate && n.severity == Severity::Warning
            ),
            "got: {notes:?}"
        );
    }

    #[test]
    fn a_missing_issue_date_warns() {
        let stripped = valid_smpte().replace(
            "  <dcst:IssueDate>2024-01-01T00:00:00</dcst:IssueDate>\n",
            "",
        );
        let f = write_tmp(&stripped);
        let notes = validate_subtitle(f.path(), Standard::Smpte);
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::SubtitleInvalidIssueDate),
            "got: {notes:?}"
        );
    }

    #[test]
    fn a_second_root_namespace_warns() {
        let two = valid_smpte().replace(
            r#"<dcst:SubtitleReel xmlns:dcst="http://www.smpte-ra.org/schemas/428-7/2010/DCST">"#,
            r#"<dcst:SubtitleReel xmlns:dcst="http://www.smpte-ra.org/schemas/428-7/2010/DCST" xmlns:other="http://www.smpte-ra.org/schemas/428-7/2007/DCST">"#,
        );
        let f = write_tmp(&two);
        let notes = validate_subtitle(f.path(), Standard::Smpte);
        assert!(
            notes.iter().any(|n| n.code == Code::SubtitleNamespaceCount),
            "got: {notes:?}"
        );
    }

    #[test]
    fn an_empty_text_element_is_error() {
        for empty in ["<dcst:Text></dcst:Text>", "<dcst:Text/>"] {
            let doc = valid_smpte().replace("<dcst:Text>Hello</dcst:Text>", empty);
            let f = write_tmp(&doc);
            let notes = validate_subtitle(f.path(), Standard::Smpte);
            assert!(
                notes
                    .iter()
                    .any(|n| n.code == Code::SubtitleEmptyText && n.severity == Severity::Error),
                "{empty} must fire, got: {notes:?}"
            );
        }
    }

    // the one-space placeholder cue DCP-o-matic writes on a reel without
    // subtitles passes libdcp, so it passes here
    #[test]
    fn a_whitespace_only_text_is_content() {
        let doc = valid_smpte().replace("<dcst:Text>Hello</dcst:Text>", "<dcst:Text> </dcst:Text>");
        let f = write_tmp(&doc);
        let notes = validate_subtitle(f.path(), Standard::Smpte);
        assert!(
            !notes.iter().any(|n| n.code == Code::SubtitleEmptyText),
            "got: {notes:?}"
        );
    }

    // text nested in a formatting element still counts as content
    #[test]
    fn text_with_only_nested_content_is_not_empty() {
        let doc = valid_smpte().replace(
            "<dcst:Text>Hello</dcst:Text>",
            "<dcst:Text><dcst:Ruby><dcst:Rb>Hello</dcst:Rb></dcst:Ruby></dcst:Text>",
        );
        let f = write_tmp(&doc);
        let notes = validate_subtitle(f.path(), Standard::Smpte);
        assert!(
            !notes.iter().any(|n| n.code == Code::SubtitleEmptyText),
            "got: {notes:?}"
        );
    }

    #[test]
    fn a_font_with_no_load_font_is_error() {
        let doc = valid_smpte().replace(
            r#"<dcst:Font ID="theFont">"#,
            r#"<dcst:Font ID="neverDeclared">"#,
        );
        let f = write_tmp(&doc);
        let notes = validate_subtitle(f.path(), Standard::Smpte);
        assert!(
            notes.iter().any(|n| n.code == Code::SubtitleFontMissing
                && n.message.contains("neverDeclared")),
            "got: {notes:?}"
        );
    }

    #[test]
    fn an_interop_asset_with_no_subtitles_is_error() {
        let empty = r#"<?xml version="1.0"?>
<DCSubtitle Version="1.0" xmlns="http://www.digicine.com/PROTO-ASDCP-TT-DEF"
    SubtitleID="urn:uuid:abcd">
  <ReelNumber>1</ReelNumber>
  <Language>en</Language>
  <LoadFont Id="Arial" URI="arial.ttf"/>
</DCSubtitle>"#;
        let f = write_tmp(empty);
        let notes = validate_subtitle(f.path(), Standard::Interop);
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::MissingRequiredElement
                    && n.message.contains("no subtitles")),
            "got: {notes:?}"
        );
    }

    #[test]
    fn timein_after_timeout_is_error() {
        let bad = valid_smpte().replace(
            r#"TimeIn="00:00:05:000" TimeOut="00:00:07:000""#,
            r#"TimeIn="00:00:07:000" TimeOut="00:00:05:000""#,
        );
        let f = write_tmp(&bad);
        let notes = validate_subtitle(f.path(), Standard::Smpte);
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::SubtitleInvalidTiming && n.severity == Severity::Error),
            "got: {notes:?}"
        );
    }

    #[test]
    fn overlapping_cues_warn() {
        let overlap = valid_smpte().replace(
            r#"TimeIn="00:00:08:000" TimeOut="00:00:10:000""#,
            r#"TimeIn="00:00:06:000" TimeOut="00:00:10:000""#,
        );
        let f = write_tmp(&overlap);
        let notes = validate_subtitle(f.path(), Standard::Smpte);
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::SubtitleInvalidTiming && n.message.contains("overlaps")),
            "got: {notes:?}"
        );
    }

    #[test]
    fn missing_reel_number_and_language_warn() {
        let stripped = valid_smpte()
            .replace("  <dcst:ReelNumber>1</dcst:ReelNumber>\n", "")
            .replace("  <dcst:Language>en</dcst:Language>\n", "");
        let f = write_tmp(&stripped);
        let notes = validate_subtitle(f.path(), Standard::Smpte);
        assert!(notes.iter().any(|n| n.message.contains("ReelNumber")));
        assert!(notes.iter().any(|n| n.message.contains("Language")));
    }

    #[test]
    fn missing_id_is_error() {
        let stripped = valid_smpte().replace(
            "  <dcst:Id>urn:uuid:22222222-2222-3333-4444-555555555555</dcst:Id>\n",
            "",
        );
        let f = write_tmp(&stripped);
        let notes = validate_subtitle(f.path(), Standard::Smpte);
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::MissingRequiredElement
                    && n.message.contains("identifier")),
            "got: {notes:?}"
        );
    }

    #[test]
    fn interop_subtitle_id_attribute_is_detected() {
        let interop = r#"<?xml version="1.0"?>
<DCSubtitle Version="1.0" xmlns="http://www.digicine.com/PROTO-ASDCP-TT-DEF"
    SubtitleID="urn:uuid:abcd">
  <ReelNumber>1</ReelNumber>
  <Language>en</Language>
  <LoadFont Id="Arial" URI="arial.ttf"/>
  <Font>
    <Subtitle SpotNumber="1" TimeIn="00:00:05:000" TimeOut="00:00:07:000"/>
  </Font>
</DCSubtitle>"#;
        let f = write_tmp(interop);
        let notes = validate_subtitle(f.path(), Standard::Interop);
        assert!(notes.is_empty(), "expected clean, got: {notes:?}");
    }

    #[test]
    fn non_subtitle_file_is_error() {
        let f = write_tmp(r#"<CompositionPlaylist><Id>x</Id></CompositionPlaylist>"#);
        let notes = validate_subtitle(f.path(), Standard::Unknown);
        assert!(notes.iter().any(|n| n.code == Code::SubtitleParseError));
    }

    // minimal sfnt with a single cmap (format 12) mapping the given chars to
    // sequential glyph ids; chars not listed have no glyph.
    pub(crate) fn make_font(chars: &[char]) -> Vec<u8> {
        let n = chars.len() as u32;
        let mut sub = Vec::new();
        sub.extend_from_slice(&12u16.to_be_bytes()); // format 12
        sub.extend_from_slice(&0u16.to_be_bytes()); // reserved
        sub.extend_from_slice(&(16 + 12 * n).to_be_bytes()); // length
        sub.extend_from_slice(&0u32.to_be_bytes()); // language
        sub.extend_from_slice(&n.to_be_bytes()); // numGroups
        for (i, c) in chars.iter().enumerate() {
            let code = *c as u32;
            sub.extend_from_slice(&code.to_be_bytes()); // startCharCode
            sub.extend_from_slice(&code.to_be_bytes()); // endCharCode
            sub.extend_from_slice(&(i as u32 + 1).to_be_bytes()); // startGlyphID
        }

        let mut cmap = Vec::new();
        cmap.extend_from_slice(&0u16.to_be_bytes()); // version
        cmap.extend_from_slice(&1u16.to_be_bytes()); // numTables
        cmap.extend_from_slice(&3u16.to_be_bytes()); // platform: Windows
        cmap.extend_from_slice(&10u16.to_be_bytes()); // encoding: Unicode full
        cmap.extend_from_slice(&12u32.to_be_bytes()); // subtable offset
        cmap.extend_from_slice(&sub);

        let mut font = Vec::new();
        font.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // sfnt version
        font.extend_from_slice(&1u16.to_be_bytes()); // numTables
        font.extend_from_slice(&16u16.to_be_bytes()); // searchRange
        font.extend_from_slice(&0u16.to_be_bytes()); // entrySelector
        font.extend_from_slice(&0u16.to_be_bytes()); // rangeShift
        font.extend_from_slice(b"cmap");
        font.extend_from_slice(&0u32.to_be_bytes()); // checksum
        font.extend_from_slice(&28u32.to_be_bytes()); // offset
        font.extend_from_slice(&(cmap.len() as u32).to_be_bytes()); // length
        font.extend_from_slice(&cmap);
        font
    }

    fn sub_with_text(text: &str) -> tempfile::NamedTempFile {
        let doc = format!(
            r#"<?xml version="1.0"?>
<dcst:SubtitleReel xmlns:dcst="http://www.smpte-ra.org/schemas/428-7/2010/DCST">
  <dcst:LoadFont ID="f1">urn:uuid:font</dcst:LoadFont>
  <dcst:SubtitleList>
    <dcst:Font ID="f1">
      <dcst:Subtitle SpotNumber="1" TimeIn="00:00:05:000" TimeOut="00:00:07:000">
        <dcst:Text>{text}</dcst:Text>
      </dcst:Subtitle>
    </dcst:Font>
  </dcst:SubtitleList>
</dcst:SubtitleReel>"#
        );
        write_tmp(&doc)
    }

    #[test]
    fn missing_glyph_fires() {
        let font = tempfile::Builder::new().suffix(".ttf").tempfile().unwrap();
        std::fs::write(font.path(), make_font(&['H', 'i'])).unwrap();
        let f = sub_with_text("Hé"); // 'é' has no glyph
        let path = font.path().to_path_buf();
        let notes = check_glyph_coverage(f.path(), |_decl| Some(path.clone()));
        assert_eq!(notes.len(), 1, "got: {notes:?}");
        assert_eq!(notes[0].code, Code::SubtitleGlyphMissing);
        assert!(
            notes[0].message.contains("U+00E9"),
            "got: {}",
            notes[0].message
        );
        assert!(notes[0].message.contains("00:00:05:000"));
    }

    #[test]
    fn full_coverage_is_silent() {
        let font = tempfile::Builder::new().suffix(".ttf").tempfile().unwrap();
        std::fs::write(font.path(), make_font(&['H', 'i'])).unwrap();
        let f = sub_with_text("Hi");
        let path = font.path().to_path_buf();
        let notes = check_glyph_coverage(f.path(), |_decl| Some(path.clone()));
        assert!(notes.is_empty(), "expected clean, got: {notes:?}");
    }

    #[test]
    fn unresolvable_font_is_skipped() {
        let f = sub_with_text("Hé");
        let notes = check_glyph_coverage(f.path(), |_decl| None);
        assert!(notes.is_empty(), "expected silent skip, got: {notes:?}");
    }

    // ─── MXF-wrapped (SMPTE ST 429-5) glyph coverage ─────────────────────────

    fn uuid_urn(b: &[u8; 16]) -> String {
        let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
        format!(
            "urn:uuid:{}-{}-{}-{}-{}",
            &h[0..8],
            &h[8..12],
            &h[12..16],
            &h[16..20],
            &h[20..32]
        )
    }

    fn mxf_doc(text: &str, font_urn: &str) -> String {
        format!(
            r#"<?xml version="1.0"?>
<dcst:SubtitleReel xmlns:dcst="http://www.smpte-ra.org/schemas/428-7/2010/DCST">
  <dcst:Id>urn:uuid:11111111-2222-3333-4444-555555555555</dcst:Id>
  <dcst:LoadFont ID="f1">{font_urn}</dcst:LoadFont>
  <dcst:SubtitleList>
    <dcst:Font ID="f1">
      <dcst:Subtitle SpotNumber="1" TimeIn="00:00:05:000" TimeOut="00:00:07:000">
        <dcst:Text>{text}</dcst:Text>
      </dcst:Subtitle>
    </dcst:Font>
  </dcst:SubtitleList>
</dcst:SubtitleReel>"#
        )
    }

    /// AssetUUID of the fixture track file. ST 429-5 wants it distinct from both
    /// the ResourceID and the document's own Id, so the fixtures below are
    /// conformant unless a case deliberately breaks that.
    pub(crate) const FIXTURE_ASSET_UUID: [u8; 16] = [4; 16];

    /// Frames of essence the fixture declares, which the reel Duration must match.
    pub(crate) const FIXTURE_CONTAINER_DURATION: u32 = 96;

    /// Build a timed-text MXF at `dir/sub.mxf` carrying `doc`, optionally embedding
    /// `font` as an OpenType ancillary resource keyed by `font_uuid`. The
    /// descriptor's ResourceID is taken from the document's own Id, which is what
    /// ST 429-5 requires.
    pub(crate) fn write_mxf(dir: &Path, doc: &str, font: Option<(&[u8], [u8; 16])>) -> PathBuf {
        let resource_id = document_id(doc).unwrap_or([5; 16]);
        write_mxf_with_ids(dir, doc, font, FIXTURE_ASSET_UUID, resource_id)
    }

    /// Same, with the two MXF-level ids given explicitly so a case can break the
    /// ST 429-5 identity relationships on purpose.
    pub(crate) fn write_mxf_with_ids(
        dir: &Path,
        doc: &str,
        font: Option<(&[u8], [u8; 16])>,
        asset_uuid: [u8; 16],
        resource_id: [u8; 16],
    ) -> PathBuf {
        use asdcplib::timed_text::*;
        use asdcplib::{EDIT_RATE_24, WriterInfo};

        let path = dir.join("sub.mxf");
        let ps = path.to_string_lossy().to_string();
        let info = WriterInfo {
            asset_uuid,
            ..Default::default()
        };
        let desc = TimedTextDescriptor {
            edit_rate: EDIT_RATE_24,
            container_duration: FIXTURE_CONTAINER_DURATION,
            asset_id: resource_id,
        };
        let mut writer = MxfWriter::new();
        match font {
            Some((bytes, uuid)) => {
                writer
                    .open_write_with_resources(
                        &ps,
                        &info,
                        &desc,
                        &[AncillaryResourceInfo {
                            uuid,
                            mime_type: MimeType::OpenType,
                        }],
                        32_768,
                    )
                    .unwrap();
                writer.write_timed_text_resource(doc, None, None).unwrap();
                writer
                    .write_ancillary_resource(
                        bytes,
                        &uuid,
                        "application/x-font-opentype",
                        None,
                        None,
                    )
                    .unwrap();
            }
            None => {
                writer.open_write(&ps, &info, &desc, 16_384).unwrap();
                writer.write_timed_text_resource(doc, None, None).unwrap();
            }
        }
        writer.finalize().unwrap();
        path
    }

    // the XSD pass validates wrapped subtitles too, which means unwrapping the
    // document out of the essence rather than reading a file.
    #[test]
    fn wrapped_timed_text_document_is_readable_for_schema_validation() {
        if !crate::schema::xmllint_available() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let uuid = [0xCD; 16];
        let doc = mxf_doc("Hi", &uuid_urn(&uuid));
        let path = write_mxf(dir.path(), &doc, Some((&make_font(&['H', 'i']), uuid)));

        let asset =
            read_wrapped_timed_text(&path, &crate::kdm::ContentKeys::none(), FontData::Include)
                .expect("the wrapped asset must be readable");
        let xml = asset.xml;
        assert!(xml.contains("SubtitleReel"), "got: {xml}");

        // that document omits required ST 428-7 elements, so the XSD must object,
        // and the finding must name the MXF rather than a temporary file
        let schema_dir = crate::schema::locate_schema_dir().expect("the repository vendors XSDs");
        let notes = crate::schema::check_schema_xml(&xml, &path, &schema_dir);
        assert!(
            notes
                .iter()
                .any(|n| n.code == Code::XmlSchemaViolation && n.file.as_deref() == Some(&*path)),
            "wrapped timed text must reach its XSD, got: {notes:?}"
        );
    }

    /// Glyph coverage for a wrapped asset, read straight off disk.
    fn wrapped_glyph_notes(path: &Path) -> Vec<Note> {
        match read_wrapped_timed_text(path, &crate::kdm::ContentKeys::none(), FontData::Include) {
            Some(asset) => check_glyph_coverage_wrapped(&asset, path),
            None => Vec::new(),
        }
    }

    #[test]
    fn mxf_missing_glyph_fires() {
        let dir = tempfile::tempdir().unwrap();
        let uuid = [0xAB; 16];
        let doc = mxf_doc("Hé", &uuid_urn(&uuid)); // 'é' absent from the embedded font
        let path = write_mxf(dir.path(), &doc, Some((&make_font(&['H', 'i']), uuid)));
        let notes = wrapped_glyph_notes(&path);
        assert_eq!(notes.len(), 1, "got: {notes:?}");
        assert_eq!(notes[0].code, Code::SubtitleGlyphMissing);
        assert!(
            notes[0].message.contains("U+00E9"),
            "got: {}",
            notes[0].message
        );
        assert!(notes[0].message.contains("00:00:05:000"));
    }

    #[test]
    fn mxf_full_coverage_is_silent() {
        let dir = tempfile::tempdir().unwrap();
        let uuid = [0xCD; 16];
        let doc = mxf_doc("Hi", &uuid_urn(&uuid));
        let path = write_mxf(dir.path(), &doc, Some((&make_font(&['H', 'i']), uuid)));
        let notes = wrapped_glyph_notes(&path);
        assert!(notes.is_empty(), "expected clean, got: {notes:?}");
    }

    // every published ST 428-7 namespace is conformant, and the structural rules
    // now run on every SMPTE package, so accepting only 2010 would have turned a
    // valid 2007 or 2014 document into an error on real packages.
    #[test]
    fn all_three_st_428_7_namespaces_are_accepted() {
        let dir = tempfile::tempdir().unwrap();
        for namespace in crate::schema::smpte_subtitle_namespaces() {
            let doc = smpte_subtitle_doc(namespace);
            let path = dir.path().join("sub.xml");
            std::fs::write(&path, &doc).unwrap();
            let notes = validate_subtitle(&path, Standard::Smpte);
            assert!(
                !notes.iter().any(|n| n.code == Code::SmpteNamespaceWrong),
                "{namespace} is a published ST 428-7 namespace, got: {notes:?}"
            );
        }
    }

    #[test]
    fn a_document_in_no_st_428_7_namespace_still_fires() {
        let dir = tempfile::tempdir().unwrap();
        let doc = smpte_subtitle_doc("http://example.invalid/DCST");
        let path = dir.path().join("sub.xml");
        std::fs::write(&path, &doc).unwrap();
        assert!(
            validate_subtitle(&path, Standard::Smpte)
                .iter()
                .any(|n| n.code == Code::SmpteNamespaceWrong),
            "an unpublished namespace must still be rejected"
        );
    }

    // the structural rule set used to run only on loose .xml assets, so a SMPTE
    // package (whose subtitles are always wrapped) got none of it.
    #[test]
    fn wrapped_documents_reach_the_structural_rules() {
        let dir = tempfile::tempdir().unwrap();
        let uuid = [0xCD; 16];
        // mxf_doc omits ReelNumber and Language, which the structural rules want
        let path = write_mxf(
            dir.path(),
            &mxf_doc("Hi", &uuid_urn(&uuid)),
            Some((&make_font(&['H', 'i']), uuid)),
        );
        let asset =
            read_wrapped_timed_text(&path, &crate::kdm::ContentKeys::none(), FontData::Include)
                .expect("the wrapped asset must be readable");
        let notes = validate_subtitle_xml(&asset.xml, &path, Standard::Smpte);
        for missing in ["ReelNumber", "Language"] {
            assert!(
                notes
                    .iter()
                    .any(|n| n.code == Code::MissingRequiredElement && n.message.contains(missing)),
                "a wrapped document missing {missing} must fire, got: {notes:?}"
            );
        }
        assert!(
            notes.iter().all(|n| n.file.as_deref() == Some(&*path)),
            "findings must name the MXF the document came from, got: {notes:?}"
        );
    }

    #[test]
    fn mxf_without_font_resource_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        // urn points at a font that isn't embedded: no OpenType resource to check
        let doc = mxf_doc("Hé", &uuid_urn(&[0xEF; 16]));
        let path = write_mxf(dir.path(), &doc, None);
        let notes = wrapped_glyph_notes(&path);
        assert!(notes.is_empty(), "expected silent skip, got: {notes:?}");
    }
}
