/// Subtitle/timed text validation (SMPTE ST 428-7 DCST and Interop).
use crate::{Code, Note, Severity, Standard};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use std::path::Path;

/// Validate a subtitle/timed text XML file.
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

    let mut notes = Vec::new();

    // Check if the namespace matches the standard
    match standard {
        Standard::Smpte => {
            if !xml.contains("http://www.smpte-ra.org/schemas/428-7/2010/DCST") {
                notes.push(warn(
                    Code::SmpteNamespaceWrong,
                    "Subtitle file does not use SMPTE namespace",
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
                notes.push(warn(
                    Code::InteropNamespaceWrong,
                    "Subtitle file does not use Interop namespace",
                    file,
                ));
            }
        }
        Standard::Unknown => {}
    }

    check_structure(&xml, file, &mut notes);

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
    capture_id: bool,
    spans: Vec<Span>,
}

impl Scan {
    fn on_start(&mut self, e: &BytesStart) {
        let qname = e.name();
        let local = qname.local_name();
        let name = String::from_utf8_lossy(local.as_ref()).into_owned();

        self.capture_id = false;
        match name.as_str() {
            // SMPTE DCST root or Interop root
            "SubtitleReel" | "DCSubtitle" => {
                self.is_subtitle = true;
                // Interop carries the identifier as a SubtitleID attribute
                if attr(e, "SubtitleID").is_some_and(|v| !v.trim().is_empty()) {
                    self.has_subtitle_id = true;
                }
            }
            "ReelNumber" => self.has_reel_number = true,
            "Language" => self.has_language = true,
            "LoadFont" => self.has_load_font = true,
            // SMPTE DCST uses <Id>; Interop DCSubtitle uses a <SubtitleID> element
            "Id" | "SubtitleID" => self.capture_id = true,
            "Subtitle" => {
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
}

fn check_structure(xml: &str, file: &Path, notes: &mut Vec<Note>) {
    let mut reader = Reader::from_str(xml);
    let mut scan = Scan::default();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => scan.on_start(&e),
            Ok(Event::Empty(e)) => scan.on_start(&e),
            Ok(Event::Text(e)) => {
                if scan.capture_id {
                    scan.capture_id = false;
                    if !dcpdoctor_parse::text_of(&e).trim().is_empty() {
                        scan.has_subtitle_id = true;
                    }
                }
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
        notes.push(warn(Code::SubtitleFontMissing, "Missing LoadFont", file));
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_tmp(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::Builder::new().suffix(".xml").tempfile().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    const VALID_SMPTE: &str = r#"<?xml version="1.0"?>
<dcst:SubtitleReel xmlns:dcst="http://www.smpte-ra.org/schemas/428-7/2010/DCST">
  <dcst:Id>urn:uuid:11111111-2222-3333-4444-555555555555</dcst:Id>
  <dcst:ReelNumber>1</dcst:ReelNumber>
  <dcst:Language>en</dcst:Language>
  <dcst:LoadFont ID="theFont">urn:uuid:aaaa</dcst:LoadFont>
  <dcst:SubtitleList>
    <dcst:Subtitle SpotNumber="1" TimeIn="00:00:05:000" TimeOut="00:00:07:000"/>
    <dcst:Subtitle SpotNumber="2" TimeIn="00:00:08:000" TimeOut="00:00:10:000"/>
  </dcst:SubtitleList>
</dcst:SubtitleReel>"#;

    #[test]
    fn valid_smpte_has_no_findings() {
        let f = write_tmp(VALID_SMPTE);
        let notes = validate_subtitle(f.path(), Standard::Smpte);
        assert!(notes.is_empty(), "expected clean, got: {notes:?}");
    }

    #[test]
    fn timein_after_timeout_is_error() {
        let bad = VALID_SMPTE.replace(
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
        let overlap = VALID_SMPTE.replace(
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
        let stripped = VALID_SMPTE
            .replace("  <dcst:ReelNumber>1</dcst:ReelNumber>\n", "")
            .replace("  <dcst:Language>en</dcst:Language>\n", "");
        let f = write_tmp(&stripped);
        let notes = validate_subtitle(f.path(), Standard::Smpte);
        assert!(notes.iter().any(|n| n.message.contains("ReelNumber")));
        assert!(notes.iter().any(|n| n.message.contains("Language")));
    }

    #[test]
    fn missing_id_is_error() {
        let stripped = VALID_SMPTE.replace(
            "  <dcst:Id>urn:uuid:11111111-2222-3333-4444-555555555555</dcst:Id>\n",
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
}
