//! IMF CPL XML parsing (pure, no I/O).

use dcpdoctor_parse::text_of as decode_text;
use quick_xml::events::Event;
use quick_xml::Reader;

use crate::types::*;

// IMF Namespaces
const NS_APP2E: &str = "http://www.smpte-ra.org/ns/2067-21/2021";
const NS_APP2E_2016: &str = "http://www.smpte-ra.org/ns/2067-21/2016";
const NS_APP5: &str = "http://www.smpte-ra.org/ns/2067-50/2017";

/// Parse an IMF CPL from XML string.
pub fn parse_imf_cpl(xml: &str) -> Result<ImfCpl, String> {
    let mut cpl = ImfCpl::default();

    // Extract namespaces from root element
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let local = e.local_name();
                let name = String::from_utf8_lossy(local.as_ref());
                if name == "CompositionPlaylist" {
                    for attr in e.attributes().flatten() {
                        let value = String::from_utf8_lossy(attr.value.as_ref()).to_string();
                        if value.contains("smpte-ra.org") || value.contains("2067") {
                            cpl.namespaces.push(value);
                        }
                    }
                    break;
                }
            }
            Ok(Event::Eof) => return Err("No CompositionPlaylist element found".to_string()),
            Err(e) => return Err(format!("XML parse error: {e}")),
            _ => {}
        }
    }

    cpl.application = detect_application(&cpl.namespaces, xml);

    // Full parse
    let mut reader = Reader::from_str(xml);
    let mut tag_stack: Vec<String> = Vec::new();
    let mut current_vt: Option<VirtualTrack> = None;
    let mut current_resource: Option<TrackResource> = None;
    let mut current_tag = String::new();
    let mut current_marker: Option<Marker> = None;
    let mut in_essence_descriptor_list = false;
    let mut current_ed: Option<EssenceDescriptor> = None;
    let mut current_ed_tag = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                tag_stack.push(name.clone());
                current_tag = name.clone();

                match name.as_str() {
                    "Segment" => cpl.segment_count += 1,
                    "MainImageSequence" => {
                        current_vt = Some(VirtualTrack {
                            track_type: TrackType::MainImage,
                            ..Default::default()
                        });
                    }
                    "MainAudioSequence" => {
                        current_vt = Some(VirtualTrack {
                            track_type: TrackType::MainAudio,
                            ..Default::default()
                        });
                    }
                    "SubtitlesSequence" | "TimedTextSequence" => {
                        current_vt = Some(VirtualTrack {
                            track_type: TrackType::Subtitle,
                            ..Default::default()
                        });
                    }
                    "HearingImpairedCaptionsSequence" => {
                        current_vt = Some(VirtualTrack {
                            track_type: TrackType::HearingImpaired,
                            ..Default::default()
                        });
                    }
                    "VisuallyImpairedTextSequence" => {
                        current_vt = Some(VirtualTrack {
                            track_type: TrackType::VisuallyImpaired,
                            ..Default::default()
                        });
                    }
                    "CommentarySequence" => {
                        current_vt = Some(VirtualTrack {
                            track_type: TrackType::Commentary,
                            ..Default::default()
                        });
                    }
                    "KaraokeSequence" => {
                        current_vt = Some(VirtualTrack {
                            track_type: TrackType::Karaoke,
                            ..Default::default()
                        });
                    }
                    "ForcedNarrativeSequence" => {
                        current_vt = Some(VirtualTrack {
                            track_type: TrackType::ForcedNarrative,
                            ..Default::default()
                        });
                    }
                    "IABSequence" | "ImmersiveAudioSequence" => {
                        current_vt = Some(VirtualTrack {
                            track_type: TrackType::IAB,
                            ..Default::default()
                        });
                    }
                    "MarkerSequence" => {
                        current_vt = Some(VirtualTrack {
                            track_type: TrackType::Marker,
                            ..Default::default()
                        });
                    }
                    "Resource" => {
                        current_resource = Some(TrackResource::default());
                    }
                    "Marker" => {
                        current_marker = Some(Marker::default());
                    }
                    "EssenceDescriptorList" => {
                        in_essence_descriptor_list = true;
                    }
                    "EssenceDescriptor" if in_essence_descriptor_list => {
                        current_ed = Some(EssenceDescriptor::default());
                    }
                    _ => {
                        if let Some(ref mut ed) = current_ed {
                            if ed.descriptor_type.is_empty()
                                && (name.contains("Descriptor")
                                    || name.contains("JPEG2000")
                                    || name.contains("CDCI")
                                    || name.contains("RGBA")
                                    || name.contains("Wave")
                                    || name.contains("TimedText"))
                            {
                                ed.descriptor_type = name.clone();
                            }
                        }
                        if in_essence_descriptor_list && current_ed.is_some() {
                            current_ed_tag = name;
                        }
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                tag_stack.pop();

                match name.as_str() {
                    "MainImageSequence"
                    | "MainAudioSequence"
                    | "SubtitlesSequence"
                    | "TimedTextSequence"
                    | "HearingImpairedCaptionsSequence"
                    | "VisuallyImpairedTextSequence"
                    | "CommentarySequence"
                    | "KaraokeSequence"
                    | "ForcedNarrativeSequence"
                    | "IABSequence"
                    | "ImmersiveAudioSequence"
                    | "MarkerSequence" => {
                        if let Some(vt) = current_vt.take() {
                            cpl.virtual_tracks.push(vt);
                        }
                    }
                    "Resource" => {
                        if let Some(res) = current_resource.take() {
                            if let Some(ref mut vt) = current_vt {
                                vt.resources.push(res);
                            }
                        }
                    }
                    "Marker" => {
                        if let Some(m) = current_marker.take() {
                            cpl.markers.push(m);
                        }
                    }
                    "EssenceDescriptorList" => {
                        in_essence_descriptor_list = false;
                    }
                    "EssenceDescriptor" if in_essence_descriptor_list => {
                        if let Some(ed) = current_ed.take() {
                            if !ed.linked_track_file_id.is_empty() {
                                cpl.essence_descriptors
                                    .insert(ed.linked_track_file_id.clone(), ed);
                            } else if !ed.id.is_empty() {
                                cpl.essence_descriptors.insert(ed.id.clone(), ed);
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(ref e)) => {
                let text = decode_text(e).trim().to_string();
                if text.is_empty() {
                    continue;
                }

                // Collect UUIDs from all Id elements
                if current_tag == "Id" {
                    let uuid_val = text.strip_prefix("urn:uuid:").unwrap_or(&text).to_string();
                    if !uuid_val.is_empty() {
                        cpl.all_uuids.push(uuid_val.clone());
                    }
                }

                // EssenceDescriptor fields
                if let Some(ref mut ed) = current_ed {
                    match current_ed_tag.as_str() {
                        "Id" if ed.id.is_empty() => {
                            ed.id = text.strip_prefix("urn:uuid:").unwrap_or(&text).to_string();
                        }
                        "TrackFileId" | "LinkedTrackFileId" => {
                            ed.linked_track_file_id =
                                text.strip_prefix("urn:uuid:").unwrap_or(&text).to_string();
                        }
                        "ContainerDuration" => {
                            ed.container_duration = parse_integer(
                                &text,
                                "ContainerDuration",
                                &mut cpl.integer_parse_failures,
                            )
                        }
                        "StoredWidth" => ed.stored_width = text.parse().unwrap_or(0),
                        "StoredHeight" => ed.stored_height = text.parse().unwrap_or(0),
                        "FrameLayout" => ed.frame_layout = text.parse().unwrap_or(0),
                        "ComponentDepth" => ed.component_depth = text.parse().unwrap_or(0),
                        "QuantizationBits" => ed.quantization_bits = text.parse().unwrap_or(0),
                        "ChannelCount" | "AudioChannelCount" => {
                            ed.channel_count = text.parse().unwrap_or(0)
                        }
                        "ColorPrimaries" => ed.color_primaries = text,
                        "TransferCharacteristic" => ed.transfer_characteristic = text,
                        "CodingEquations" => ed.coding_equations = text,
                        "SampleRate" | "AudioSamplingRate" => {
                            ed.audio_sampling_rate = parse_edit_rate(&text)
                        }
                        _ => {}
                    }
                    continue;
                }

                // Marker fields
                if let Some(ref mut m) = current_marker {
                    match current_tag.as_str() {
                        "Label" => m.label = text,
                        "Scope" => m.scope = text,
                        "Offset" => {
                            m.offset =
                                parse_integer(&text, "Offset", &mut cpl.integer_parse_failures)
                        }
                        _ => {}
                    }
                    continue;
                }

                let in_resource = current_resource.is_some();
                let in_vt = current_vt.is_some();

                match current_tag.as_str() {
                    "Id" if !in_resource && !in_vt && cpl.id.is_empty() => {
                        cpl.id = text.strip_prefix("urn:uuid:").unwrap_or(&text).to_string();
                    }
                    "Id" if in_vt && !in_resource => {
                        if let Some(ref mut vt) = current_vt {
                            if vt.id.is_empty() {
                                vt.id = text.strip_prefix("urn:uuid:").unwrap_or(&text).to_string();
                            }
                        }
                    }
                    "Id" if in_resource => {
                        if let Some(ref mut res) = current_resource {
                            res.id = text.strip_prefix("urn:uuid:").unwrap_or(&text).to_string();
                        }
                    }
                    "ContentTitle" | "ContentTitleText" => {
                        if cpl.content_title.is_empty() {
                            cpl.content_title = text;
                        }
                    }
                    "IssueDate" => {
                        if cpl.issue_date.is_empty() {
                            cpl.issue_date = text;
                        }
                    }
                    "ContentKind" => {
                        if cpl.content_kind.is_empty() {
                            cpl.content_kind = text;
                        }
                    }
                    "Annotation" | "AnnotationText" => {
                        if cpl.annotation.is_empty() {
                            cpl.annotation = text;
                        }
                    }
                    "Creator" => {
                        if cpl.creator.is_empty() {
                            cpl.creator = text;
                        }
                    }
                    "Issuer" => {
                        if cpl.issuer.is_empty() {
                            cpl.issuer = text;
                        }
                    }
                    "EditRate" => {
                        let rate = parse_edit_rate(&text);
                        if in_resource {
                            if let Some(ref mut res) = current_resource {
                                res.edit_rate = rate;
                            }
                        } else if cpl.edit_rate == (0, 0) {
                            cpl.edit_rate = rate;
                        }
                    }
                    "TrackFileId" | "SourceEncoding" => {
                        if let Some(ref mut res) = current_resource {
                            if res.track_file_id.is_empty() {
                                res.track_file_id =
                                    text.strip_prefix("urn:uuid:").unwrap_or(&text).to_string();
                            }
                        }
                    }
                    "IntrinsicDuration" => {
                        if let Some(ref mut res) = current_resource {
                            res.intrinsic_duration = parse_integer(
                                &text,
                                "IntrinsicDuration",
                                &mut cpl.integer_parse_failures,
                            );
                        }
                    }
                    "EntryPoint" => {
                        if let Some(ref mut res) = current_resource {
                            res.entry_point =
                                parse_integer(&text, "EntryPoint", &mut cpl.integer_parse_failures);
                        }
                    }
                    "SourceDuration" => {
                        if let Some(ref mut res) = current_resource {
                            res.source_duration = parse_integer(
                                &text,
                                "SourceDuration",
                                &mut cpl.integer_parse_failures,
                            );
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("XML parse error: {e}")),
            _ => {}
        }
    }

    // Calculate total duration from MainImage track
    for vt in &cpl.virtual_tracks {
        if vt.track_type == TrackType::MainImage {
            cpl.total_duration = vt.resources.iter().map(|r| r.effective_duration()).sum();
            break;
        }
    }

    Ok(cpl)
}

/// Parse an integer element's text, recording the element name when the text is
/// no integer. A 0 that no element declared would pass the duration, offset and
/// alignment checks, so the name has to reach the validate path.
fn parse_integer<T: std::str::FromStr + Default>(
    text: &str,
    element: &str,
    failures: &mut Vec<String>,
) -> T {
    match text.parse() {
        Ok(value) => value,
        Err(_) => {
            if !failures.iter().any(|failure| failure == element) {
                failures.push(element.to_string());
            }
            T::default()
        }
    }
}

/// Detect IMF application from namespaces and XML content.
pub fn detect_application(namespaces: &[String], xml: &str) -> ImfApplication {
    for ns in namespaces {
        if ns.contains("2067-21") || ns == NS_APP2E || ns == NS_APP2E_2016 {
            return ImfApplication::App2e;
        }
        if ns.contains("2067-50") || ns == NS_APP5 {
            return ImfApplication::App5Aces;
        }
    }
    if xml.contains("2067-21") {
        return ImfApplication::App2e;
    }
    if xml.contains("2067-50") {
        return ImfApplication::App5Aces;
    }
    ImfApplication::Unknown
}

/// Parse an edit rate string ("num den" or "num/den").
pub fn parse_edit_rate(s: &str) -> (u32, u32) {
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() == 2 {
        let num = parts[0].parse().unwrap_or(0);
        let den = parts[1].parse().unwrap_or(0);
        (num, den)
    } else if let Some((n, d)) = s.split_once('/') {
        (n.trim().parse().unwrap_or(0), d.trim().parse().unwrap_or(0))
    } else {
        (0, 0)
    }
}

/// Parse asset IDs from an ASSETMAP XML string. An XML error returns Err rather
/// than the ids read so far, which would look like an ASSETMAP that lists fewer
/// assets than it does.
pub fn parse_assetmap_ids(xml: &str) -> Result<std::collections::HashSet<String>, String> {
    let mut ids = std::collections::HashSet::new();
    let mut reader = Reader::from_str(xml);
    let mut in_id = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let local = e.local_name();
                let name = String::from_utf8_lossy(local.as_ref());
                in_id = name == "Id";
            }
            Ok(Event::Text(ref e)) if in_id => {
                let text = decode_text(e).trim().to_string();
                let id = text.strip_prefix("urn:uuid:").unwrap_or(&text).to_string();
                if !id.is_empty() {
                    ids.insert(id);
                }
                in_id = false;
            }
            Ok(Event::End(_)) => in_id = false,
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("XML parse error: {e}")),
            _ => {}
        }
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_edit_rate() {
        assert_eq!(parse_edit_rate("24 1"), (24, 1));
        assert_eq!(parse_edit_rate("24000 1001"), (24000, 1001));
        assert_eq!(parse_edit_rate("24/1"), (24, 1));
        assert_eq!(parse_edit_rate(""), (0, 0));
    }

    #[test]
    fn test_detect_application() {
        let ns = vec!["http://www.smpte-ra.org/ns/2067-21/2021".to_string()];
        assert_eq!(detect_application(&ns, ""), ImfApplication::App2e);

        let ns = vec!["http://www.smpte-ra.org/ns/2067-50/2017".to_string()];
        assert_eq!(detect_application(&ns, ""), ImfApplication::App5Aces);
    }

    #[test]
    fn test_parse_imf_cpl_minimal() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/2067-3/2016"
                     xmlns:cc="http://www.smpte-ra.org/schemas/2067-2/2016">
  <Id>urn:uuid:12345678-1234-1234-1234-123456789abc</Id>
  <ContentTitle>Test IMF</ContentTitle>
  <EditRate>24 1</EditRate>
  <IssueDate>2024-01-15T10:30:00+00:00</IssueDate>
  <ContentKind>feature</ContentKind>
  <SegmentList>
    <Segment>
      <MainImageSequence>
        <Id>urn:uuid:aaaaaaaa-1111-1111-1111-aaaaaaaaaaaa</Id>
        <ResourceList>
          <Resource>
            <Id>urn:uuid:bbbbbbbb-2222-2222-2222-bbbbbbbbbbbb</Id>
            <TrackFileId>urn:uuid:cccccccc-3333-3333-3333-cccccccccccc</TrackFileId>
            <EditRate>24 1</EditRate>
            <IntrinsicDuration>240</IntrinsicDuration>
            <EntryPoint>0</EntryPoint>
            <SourceDuration>240</SourceDuration>
          </Resource>
        </ResourceList>
      </MainImageSequence>
    </Segment>
  </SegmentList>
</CompositionPlaylist>"#;

        let cpl = parse_imf_cpl(xml).unwrap();
        assert_eq!(cpl.id, "12345678-1234-1234-1234-123456789abc");
        assert_eq!(cpl.content_title, "Test IMF");
        assert_eq!(cpl.edit_rate, (24, 1));
        assert_eq!(cpl.issue_date, "2024-01-15T10:30:00+00:00");
        assert_eq!(cpl.content_kind, "feature");
        assert_eq!(cpl.segment_count, 1);
        assert_eq!(cpl.virtual_tracks.len(), 1);
        assert_eq!(cpl.virtual_tracks[0].track_type, TrackType::MainImage);
        assert_eq!(cpl.virtual_tracks[0].resources.len(), 1);
        assert_eq!(cpl.total_duration, 240);
    }

    const ASSETMAP_ASSETS: &str = r#"<Asset><Id>urn:uuid:11111111-2222-3333-4444-555555555555</Id></Asset>
    <Asset><Id>urn:uuid:66666666-7777-8888-9999-aaaaaaaaaaaa</Id></Asset>"#;

    #[test]
    fn a_broken_assetmap_errors_instead_of_returning_the_ids_read_so_far() {
        let good = format!(
            r#"<?xml version="1.0"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <AssetList>
    {ASSETMAP_ASSETS}
  </AssetList>
</AssetMap>"#
        );
        assert_eq!(parse_assetmap_ids(&good).unwrap().len(), 2);

        // the second Asset closes under a different name, and a partial id set
        // would make every track reference to it look like a broken one
        let broken = good.replace("</Asset>\n  </AssetList>", "</Assset>\n  </AssetList>");
        assert!(
            parse_assetmap_ids(&broken).is_err(),
            "a partial id set must not read back as a complete ASSETMAP"
        );
    }

    #[test]
    fn a_duration_that_is_no_integer_is_named_rather_than_read_as_zero() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/2067-3/2016">
  <Id>urn:uuid:12345678-1234-1234-1234-123456789abc</Id>
  <EditRate>24 1</EditRate>
  <SegmentList><Segment>
    <MainImageSequence>
      <Id>urn:uuid:aaaaaaaa-1111-1111-1111-aaaaaaaaaaaa</Id>
      <ResourceList><Resource>
        <Id>urn:uuid:bbbbbbbb-2222-2222-2222-bbbbbbbbbbbb</Id>
        <TrackFileId>urn:uuid:cccccccc-3333-3333-3333-cccccccccccc</TrackFileId>
        <EditRate>24 1</EditRate>
        <IntrinsicDuration>two hundred forty</IntrinsicDuration>
        <EntryPoint>0</EntryPoint>
        <SourceDuration>240</SourceDuration>
      </Resource></ResourceList>
    </MainImageSequence>
  </Segment></SegmentList>
</CompositionPlaylist>"#;

        let cpl = parse_imf_cpl(xml).unwrap();
        assert_eq!(
            cpl.integer_parse_failures,
            vec!["IntrinsicDuration".to_string()],
            "a duration read as 0 would pass the bounds and alignment checks"
        );
        assert!(parse_imf_cpl(&xml.replace("two hundred forty", "240"))
            .unwrap()
            .integer_parse_failures
            .is_empty());
    }

    #[test]
    fn test_resource_effective_duration() {
        let res = TrackResource {
            intrinsic_duration: 100,
            entry_point: 10,
            source_duration: 50,
            ..Default::default()
        };
        assert_eq!(res.effective_duration(), 50);

        let res = TrackResource {
            intrinsic_duration: 100,
            entry_point: 10,
            source_duration: 0,
            ..Default::default()
        };
        assert_eq!(res.effective_duration(), 90);
    }
}
