//! JPEG 2000 codestream marker walking, shared by the native validator and the
//! wasm build. Byte slices in, findings out, so it builds for
//! wasm32-unknown-unknown like the rest of this crate.

/// Marker codes from ITU-T T.800 Table A.2.
pub const SOC: u16 = 0xFF4F;
pub const SIZ: u16 = 0xFF51;
pub const COD: u16 = 0xFF52;
pub const TLM: u16 = 0xFF55;
pub const QCD: u16 = 0xFF5C;
pub const POC: u16 = 0xFF5F;
pub const SOT: u16 = 0xFF90;
pub const SOD: u16 = 0xFF93;
pub const EOC: u16 = 0xFFD9;

/// Lowest marker byte that can follow 0xFF outside a packet body. T.800 §B.10.1
/// bit-stuffs packet data so 0xFF is never followed by a byte above 0x8F, which
/// is what makes scanning for one a safe way past a packet.
const FIRST_MARKER_AFTER_PACKET_DATA: u8 = 0x90;

/// CPRL, the progression order the DCI profiles fix (T.800 Table A.16).
const PROGRESSION_ORDER_CPRL: u16 = 4;

/// Bytes one POC progression occupies: RSpoc, CSpoc, LYEpoc (2), REpoc, CEpoc,
/// Ppoc.
const POC_PROGRESSION_BYTES: usize = 7;

/// A POC parameter that is not the value the 4K profile fixes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PocFieldMismatch {
    /// Field name as T.800 §A.6.6 spells it, suffixed with its progression index.
    pub field: String,
    pub expected: u16,
    pub found: u16,
}

/// One progression of the two the DCI 4K profile fixes.
struct PocProgression {
    rspoc: u16,
    cspoc: u16,
    lyepoc: u16,
    repoc: u16,
    cepoc: u16,
    ppoc: u16,
}

/// ISO 15444-1 Annex A pins the 4K profile's POC marker to exactly two
/// progressions: the 2K portion (resolutions 0 to 5, which a 2K decoder reads on
/// its own) then the 4K portion (resolution 6), both CPRL over three components.
/// These are the values libdcp's verify_j2k requires.
const REQUIRED_POC_PROGRESSIONS: &[PocProgression] = &[
    PocProgression {
        rspoc: 0,
        cspoc: 0,
        lyepoc: 1,
        repoc: 6,
        cepoc: 3,
        ppoc: PROGRESSION_ORDER_CPRL,
    },
    PocProgression {
        rspoc: 6,
        cspoc: 0,
        lyepoc: 1,
        repoc: 7,
        cepoc: 3,
        ppoc: PROGRESSION_ORDER_CPRL,
    },
];

/// What a full marker walk sees that a main-header parse alone cannot: whether
/// the TLM marker is present, and where the POC markers sit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MarkerScan {
    pub tlm_present: bool,
    /// POC markers in the main header, before the first tile-part.
    pub poc_in_main_header: usize,
    /// POC markers in a tile-part header, which no cinema profile permits.
    pub poc_after_main_header: usize,
    /// Main-header POC parameters that differ from the two fixed progressions.
    pub poc_field_mismatches: Vec<PocFieldMismatch>,
    /// The walk stopped before the EOC marker, so the counts above cover only
    /// the part of the codestream it reached.
    pub truncated: bool,
}

fn read_u16(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*data.get(at)?, *data.get(at + 1)?]))
}

/// Walk every marker segment of a codestream and report the TLM/POC facts the
/// DCI profiles constrain. A truncated or desynchronised stream stops the walk,
/// reports what it saw up to that point, and sets `truncated`.
pub fn scan_markers(data: &[u8]) -> MarkerScan {
    let mut scan = MarkerScan::default();
    if read_u16(data, 0) != Some(SOC) {
        return scan;
    }

    let mut pos = 2; // SOC is a delimiting marker with no segment
    let mut main_header_finished = false;

    loop {
        let Some(marker) = read_u16(data, pos) else {
            scan.truncated = true; // ran out of bytes before the EOC
            break;
        };
        if marker & 0xFF00 != 0xFF00 {
            scan.truncated = true; // lost sync
            break;
        }
        pos += 2;

        if marker == EOC {
            break;
        }
        if marker == SOD {
            // packet data carries no length, so skip to the next marker
            while pos + 1 < data.len()
                && !(data[pos] == 0xFF && data[pos + 1] >= FIRST_MARKER_AFTER_PACKET_DATA)
            {
                pos += 1;
            }
            continue;
        }

        let Some(segment_length) = read_u16(data, pos).map(usize::from) else {
            scan.truncated = true;
            break;
        };
        if segment_length < 2 {
            scan.truncated = true;
            break;
        }
        let parameters = data.get(pos + 2..pos + segment_length);

        match marker {
            SOT => main_header_finished = true,
            TLM => scan.tlm_present = true,
            POC if main_header_finished => scan.poc_after_main_header += 1,
            POC => {
                scan.poc_in_main_header += 1;
                scan.poc_field_mismatches
                    .extend(poc_field_mismatches(parameters.unwrap_or_default()));
            }
            _ => {}
        }

        pos += segment_length;
    }

    scan
}

/// Compare a POC marker's parameter bytes against the two fixed progressions.
/// A marker too short to hold them reports the fields it is missing as absent.
fn poc_field_mismatches(parameters: &[u8]) -> Vec<PocFieldMismatch> {
    let mut mismatches = Vec::new();

    for (index, progression) in REQUIRED_POC_PROGRESSIONS.iter().enumerate() {
        let base = index * POC_PROGRESSION_BYTES;
        let byte = |at: usize| parameters.get(base + at).copied().map(u16::from);
        let pair = |at: usize| read_u16(parameters, base + at);

        let found = [
            ("RSpoc", progression.rspoc, byte(0)),
            ("CSpoc", progression.cspoc, byte(1)),
            ("LYEpoc", progression.lyepoc, pair(2)),
            ("REpoc", progression.repoc, byte(4)),
            ("CEpoc", progression.cepoc, byte(5)),
            ("Ppoc", progression.ppoc, byte(6)),
        ];
        for (field, expected, actual) in found {
            // a truncated marker leaves nothing to compare, so report the whole
            // progression as absent once rather than six times
            let Some(actual) = actual else {
                mismatches.push(PocFieldMismatch {
                    field: format!("progression {}", index + 1),
                    expected: expected.max(1),
                    found: 0,
                });
                break;
            };
            if actual != expected {
                mismatches.push(PocFieldMismatch {
                    field: format!("{field} of progression {}", index + 1),
                    expected,
                    found: actual,
                });
            }
        }
    }

    mismatches
}

/// A main-header marker segment's parameter bytes (excluding the 2 marker bytes
/// and the 2 length bytes). `None` when the marker is absent before the first
/// tile (SOT).
pub fn main_header_segment(data: &[u8], marker: u16) -> Option<&[u8]> {
    let mut pos = 2; // skip SOC
    while pos + 4 <= data.len() {
        let m = read_u16(data, pos)?;
        if m == SOT || m == SOD || m == EOC {
            return None;
        }
        let segment_length = read_u16(data, pos + 2)? as usize;
        if segment_length < 2 {
            return None;
        }
        if m == marker {
            return data.get(pos + 4..pos + 2 + segment_length);
        }
        pos += 2 + segment_length;
    }
    None
}

/// Index of the first SOT marker, i.e. the main-header length Lmh. Walks only
/// the main-header marker segments so a 0xFF90 inside packet data is never
/// mistaken for a tile-part start.
pub fn find_first_sot(data: &[u8]) -> Option<usize> {
    let mut pos = 2; // skip SOC
    while pos + 4 <= data.len() {
        let marker = read_u16(data, pos)?;
        if marker == SOT {
            return Some(pos);
        }
        if marker == SOD || marker == EOC {
            return None;
        }
        let segment_length = read_u16(data, pos + 2)? as usize;
        if segment_length < 2 {
            return None;
        }
        pos += 2 + segment_length;
    }
    None
}

/// Tile-parts as (start offset, Psot length) pairs, walked from the first SOT at
/// `lmh`. Stops at a Psot of 0, which a final tile-part may use to mean "to the
/// end of the codestream".
pub fn tile_parts(data: &[u8], lmh: usize) -> Vec<(usize, usize)> {
    let mut parts = Vec::new();
    let mut pos = lmh;
    while pos + 12 <= data.len() && read_u16(data, pos) == Some(SOT) {
        // SOT: marker, Lsot(2), Isot(2), Psot(4), TPsot(1), TNsot(1)
        let psot = u32::from_be_bytes([data[pos + 6], data[pos + 7], data[pos + 8], data[pos + 9]])
            as usize;
        parts.push((pos, psot));
        if psot == 0 {
            break;
        }
        pos += psot;
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A codestream with the markers named, in order. Each entry is a marker and
    /// its parameter bytes; SOC and EOC are added around them.
    fn codestream(segments: &[(u16, Vec<u8>)]) -> Vec<u8> {
        let mut data = SOC.to_be_bytes().to_vec();
        for (marker, parameters) in segments {
            data.extend_from_slice(&marker.to_be_bytes());
            if *marker == SOD {
                continue; // delimiting marker: parameters are packet data
            }
            data.extend_from_slice(&((parameters.len() + 2) as u16).to_be_bytes());
            data.extend_from_slice(parameters);
        }
        data.extend_from_slice(&EOC.to_be_bytes());
        data
    }

    /// A SOT segment body: Isot(2), Psot(4), TPsot(1), TNsot(1).
    fn sot_body() -> Vec<u8> {
        vec![0, 0, 0, 0, 0, 0, 0, 1]
    }

    /// The conformant two-progression 4K POC parameter bytes.
    fn poc_body() -> Vec<u8> {
        vec![0, 0, 0, 1, 6, 3, 4, 6, 0, 0, 1, 7, 3, 4]
    }

    #[test]
    fn tlm_presence_is_seen() {
        let with = codestream(&[(TLM, vec![0, 0]), (SOT, sot_body())]);
        assert!(scan_markers(&with).tlm_present);
        let without = codestream(&[(SOT, sot_body())]);
        assert!(!scan_markers(&without).tlm_present);
    }

    #[test]
    fn poc_is_counted_by_where_it_sits() {
        let main = codestream(&[(POC, poc_body()), (SOT, sot_body())]);
        let scan = scan_markers(&main);
        assert_eq!(scan.poc_in_main_header, 1);
        assert_eq!(scan.poc_after_main_header, 0);

        // a POC in a tile-part header is what INVALID_JPEG2000_POC_MARKER_LOCATION
        // is about, and only a walk past the first SOT can see it
        let tile_part = codestream(&[(SOT, sot_body()), (POC, poc_body())]);
        let scan = scan_markers(&tile_part);
        assert_eq!(scan.poc_in_main_header, 0);
        assert_eq!(scan.poc_after_main_header, 1);
    }

    #[test]
    fn the_fixed_4k_progressions_pass_and_a_changed_field_is_named() {
        let good = codestream(&[(POC, poc_body()), (SOT, sot_body())]);
        assert!(scan_markers(&good).poc_field_mismatches.is_empty());

        let mut body = poc_body();
        body[6] = 0; // Ppoc of the first progression: LRCP instead of CPRL
        let bad = codestream(&[(POC, body), (SOT, sot_body())]);
        let mismatches = scan_markers(&bad).poc_field_mismatches;
        assert_eq!(mismatches.len(), 1, "got: {mismatches:?}");
        assert!(
            mismatches[0].field.starts_with("Ppoc"),
            "got: {mismatches:?}"
        );
        assert_eq!(mismatches[0].expected, PROGRESSION_ORDER_CPRL);
        assert_eq!(mismatches[0].found, 0);
    }

    #[test]
    fn packet_data_is_walked_past_without_finding_markers_in_it() {
        // 0xFF followed by a byte below 0x90 is legal packet data, so a scan that
        // does not skip the packet body would read it as a marker
        let mut data = codestream(&[(SOT, sot_body()), (SOD, Vec::new())]);
        data.truncate(data.len() - 2); // drop the EOC, then write the body + EOC
        data.extend_from_slice(&[0xFF, 0x5F, 0xFF, 0x00, 0x12, 0x34]);
        data.extend_from_slice(&EOC.to_be_bytes());
        let scan = scan_markers(&data);
        assert_eq!(
            scan.poc_after_main_header, 0,
            "0xFF5F inside a packet body is data, not a POC marker"
        );
    }

    #[test]
    fn a_walk_that_stops_before_the_eoc_is_reported_as_truncated() {
        let whole = codestream(&[(TLM, vec![0, 0]), (SOT, sot_body())]);
        assert!(
            !scan_markers(&whole).truncated,
            "a stream walked to its EOC is not truncated"
        );

        // cut inside the SOT segment: the markers past the cut are never walked
        let cut = &whole[..whole.len() - 6];
        assert!(
            scan_markers(cut).truncated,
            "a walk that runs out of bytes must say so"
        );

        // a segment length below 2 leaves nowhere to continue from
        let mut short_segment = codestream(&[(TLM, vec![0, 0]), (SOT, sot_body())]);
        short_segment[4] = 0;
        short_segment[5] = 1;
        assert!(scan_markers(&short_segment).truncated);

        // a byte pair that is no marker at all
        let mut desynchronised = codestream(&[(TLM, vec![0, 0]), (SOT, sot_body())]);
        desynchronised[2] = 0x00;
        assert!(scan_markers(&desynchronised).truncated);
    }

    #[test]
    fn a_buffer_that_is_not_a_codestream_reports_nothing() {
        assert_eq!(scan_markers(&[0u8; 64]), MarkerScan::default());
        assert_eq!(scan_markers(&[]), MarkerScan::default());
    }

    #[test]
    fn main_header_segments_stop_at_the_first_tile() {
        let data = codestream(&[(QCD, vec![0x20, 0]), (SOT, sot_body()), (TLM, vec![0, 0])]);
        assert_eq!(main_header_segment(&data, QCD), Some(&[0x20u8, 0][..]));
        assert_eq!(
            main_header_segment(&data, TLM),
            None,
            "a marker past the first SOT is not in the main header"
        );
    }
}
