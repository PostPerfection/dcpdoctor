use quick_xml::Reader;
use quick_xml::events::Event;
use serde::{Deserialize, Serialize};

use crate::{local_name, local_name_end, root_is, strip_urn_uuid};

/// A reel essence reference (picture, sound or subtitle).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReelAsset {
    pub id: String,
    pub edit_rate: String,
    pub duration: i64,
    pub entry_point: i64,
}

/// A single reel in the CPL.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Reel {
    pub id: String,
    pub picture: ReelAsset,
    pub sound: ReelAsset,
    pub subtitle: ReelAsset,
    pub stereoscopic: bool,
}

/// Parsed Composition Playlist (CPL).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cpl {
    pub id: String,
    pub content_title: String,
    pub content_kind: String,
    pub issue_date: String,
    pub issuer: String,
    pub annotation: String,
    pub edit_rate: String,
    pub has_signature: bool,
    pub reels: Vec<Reel>,
}

/// Parse a CPL XML string.
pub fn parse_cpl(xml: &str) -> Option<Cpl> {
    if !root_is(xml, "CompositionPlaylist") {
        return None;
    }

    let mut reader = Reader::from_str(xml);
    let mut cpl = Cpl {
        has_signature: xml.contains("Signature") && xml.contains("SignatureValue"),
        ..Default::default()
    };
    let mut in_reel = false;
    let mut in_main_picture = false;
    let mut in_main_sound = false;
    let mut in_main_subtitle = false;
    let mut current_reel = Reel::default();
    let mut current_tag = String::new();
    // number of open elements, so top-level fields sit at depth 2
    let mut depth = 0usize;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                depth += 1;
                let name = local_name(&e);
                match name.as_str() {
                    "Reel" => {
                        in_reel = true;
                        current_reel = Reel::default();
                    }
                    "MainPicture" | "MainStereoscopicPicture" if in_reel => {
                        in_main_picture = true;
                        if name == "MainStereoscopicPicture" {
                            current_reel.stereoscopic = true;
                        }
                    }
                    "MainSound" if in_reel => in_main_sound = true,
                    "MainSubtitle" if in_reel => in_main_subtitle = true,
                    _ => current_tag = name,
                }
            }
            Ok(Event::End(e)) => {
                depth = depth.saturating_sub(1);
                let name = local_name_end(&e);
                match name.as_str() {
                    "Reel" => {
                        if in_reel {
                            cpl.reels.push(std::mem::take(&mut current_reel));
                        }
                        in_reel = false;
                    }
                    "MainPicture" | "MainStereoscopicPicture" => in_main_picture = false,
                    "MainSound" => in_main_sound = false,
                    "MainSubtitle" => in_main_subtitle = false,
                    _ => {}
                }
                current_tag.clear();
            }
            Ok(Event::Text(e)) => {
                let text = e.unescape().ok().map(|s| s.to_string()).unwrap_or_default();
                if text.is_empty() {
                    continue;
                }

                if in_main_picture {
                    fill_reel_asset(&mut current_reel.picture, &current_tag, text);
                } else if in_main_sound {
                    fill_reel_asset(&mut current_reel.sound, &current_tag, text);
                } else if in_main_subtitle {
                    fill_reel_asset(&mut current_reel.subtitle, &current_tag, text);
                } else if in_reel {
                    if current_tag == "Id" {
                        current_reel.id = strip_urn_uuid(&text);
                    }
                } else {
                    match current_tag.as_str() {
                        // guarded so nested Ids (ContentVersion) don't clobber the CPL id
                        "Id" if depth <= 2 => cpl.id = strip_urn_uuid(&text),
                        "AnnotationText" if depth <= 2 => cpl.annotation = text,
                        "ContentTitleText" => cpl.content_title = text,
                        "ContentKind" => cpl.content_kind = text,
                        "IssueDate" => cpl.issue_date = text,
                        "Issuer" => cpl.issuer = text,
                        "EditRate" => cpl.edit_rate = text,
                        _ => {}
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => return None,
            _ => {}
        }
    }

    Some(cpl)
}

fn fill_reel_asset(asset: &mut ReelAsset, tag: &str, text: String) {
    match tag {
        "Id" => asset.id = strip_urn_uuid(&text),
        "EditRate" => asset.edit_rate = text,
        "Duration" | "IntrinsicDuration" => asset.duration = text.parse().unwrap_or(0),
        "EntryPoint" => asset.entry_point = text.parse().unwrap_or(0),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CPL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL">
  <Id>urn:uuid:96558952-39b8-42d3-825e-9ddd31298219</Id>
  <AnnotationText>note</AnnotationText>
  <IssueDate>2025-01-01T00:00:00+00:00</IssueDate>
  <Issuer>acme</Issuer>
  <ContentTitleText>Test Title</ContentTitleText>
  <ContentKind>feature</ContentKind>
  <ContentVersion>
    <Id>urn:uuid:deadbeef-0000-0000-0000-000000000000</Id>
  </ContentVersion>
  <ReelList>
    <Reel>
      <Id>urn:uuid:b353da2a-703e-4d3f-8fcd-659930713ece</Id>
      <AssetList>
        <MainPicture>
          <Id>urn:uuid:f76deec8-ab85-4f05-973d-089b67a55e5f</Id>
          <EditRate>24 1</EditRate>
          <IntrinsicDuration>96</IntrinsicDuration>
          <EntryPoint>12</EntryPoint>
          <Duration>48</Duration>
        </MainPicture>
        <MainSound>
          <Id>urn:uuid:6b6673ae-d44d-4153-93ad-5333d7af01fb</Id>
          <EditRate>24 1</EditRate>
          <Duration>48</Duration>
        </MainSound>
        <MainSubtitle>
          <Id>urn:uuid:11111111-2222-3333-4444-555555555555</Id>
          <Duration>48</Duration>
        </MainSubtitle>
      </AssetList>
    </Reel>
  </ReelList>
</CompositionPlaylist>"#;

    #[test]
    fn test_parse_cpl() {
        let cpl = parse_cpl(CPL).unwrap();
        assert_eq!(cpl.id, "96558952-39b8-42d3-825e-9ddd31298219");
        assert_eq!(cpl.content_title, "Test Title");
        assert_eq!(cpl.content_kind, "feature");
        assert_eq!(cpl.issuer, "acme");
        assert_eq!(cpl.annotation, "note");
        assert!(!cpl.has_signature);
        assert_eq!(cpl.reels.len(), 1);

        let reel = &cpl.reels[0];
        assert_eq!(reel.id, "b353da2a-703e-4d3f-8fcd-659930713ece");
        assert_eq!(reel.picture.id, "f76deec8-ab85-4f05-973d-089b67a55e5f");
        assert_eq!(reel.picture.edit_rate, "24 1");
        // Duration comes last and wins over IntrinsicDuration
        assert_eq!(reel.picture.duration, 48);
        assert_eq!(reel.picture.entry_point, 12);
        assert_eq!(reel.sound.id, "6b6673ae-d44d-4153-93ad-5333d7af01fb");
        assert_eq!(reel.subtitle.id, "11111111-2222-3333-4444-555555555555");
        assert!(!reel.stereoscopic);
    }

    #[test]
    fn test_rejects_non_cpl_root() {
        assert!(parse_cpl(r#"<PackingList><Id>x</Id></PackingList>"#).is_none());
    }

    #[test]
    fn test_stereoscopic() {
        let xml = CPL.replace("MainPicture", "MainStereoscopicPicture");
        assert!(parse_cpl(&xml).unwrap().reels[0].stereoscopic);
    }
}
