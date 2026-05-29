use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};

/// Parsed CPL structure.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cpl {
    pub id: String,
    pub content_title: String,
    pub content_kind: String,
    pub issue_date: String,
    pub issuer: String,
    pub annotation: String,
    pub edit_rate: String,
    pub reels: Vec<Reel>,
    pub has_signature: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Reel {
    pub id: String,
    pub picture_asset_id: String,
    pub sound_asset_id: String,
    pub subtitle_asset_id: String,
    pub duration: u64,
    pub entry_point: u64,
}

/// Parse a CPL XML string.
pub fn parse_cpl(xml: &str) -> Result<Cpl, String> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut cpl = Cpl::default();
    let mut current_reel = Reel::default();
    let mut in_reel = false;
    let mut current_tag = String::new();
    let mut tag_stack: Vec<String> = Vec::new();

    cpl.has_signature = xml.contains("Signature") && xml.contains("SignatureValue");

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                tag_stack.push(name.clone());
                current_tag = name.clone();
                if name == "Reel" {
                    in_reel = true;
                    current_reel = Reel::default();
                }
            }
            Ok(Event::End(ref e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                if name == "Reel" && in_reel {
                    cpl.reels.push(current_reel.clone());
                    in_reel = false;
                }
                tag_stack.pop();
            }
            Ok(Event::Text(ref e)) => {
                let text = e.unescape().unwrap_or_default().to_string();
                let text = text.trim().to_string();
                if text.is_empty() {
                    continue;
                }
                let in_picture = tag_stack
                    .iter()
                    .any(|t| t == "MainPicture" || t == "MainStereoscopicPicture");
                let in_sound = tag_stack.iter().any(|t| t == "MainSound");
                let in_subtitle = tag_stack.iter().any(|t| t == "MainSubtitle");

                if in_reel {
                    match current_tag.as_str() {
                        "Id" if !in_picture && !in_sound && !in_subtitle => {
                            current_reel.id = text.trim_start_matches("urn:uuid:").to_string();
                        }
                        "Id" if in_picture => {
                            current_reel.picture_asset_id =
                                text.trim_start_matches("urn:uuid:").to_string();
                        }
                        "Id" if in_sound => {
                            current_reel.sound_asset_id =
                                text.trim_start_matches("urn:uuid:").to_string();
                        }
                        "Id" if in_subtitle => {
                            current_reel.subtitle_asset_id =
                                text.trim_start_matches("urn:uuid:").to_string();
                        }
                        "Duration" | "IntrinsicDuration" => {
                            if current_reel.duration == 0 {
                                current_reel.duration = text.parse().unwrap_or(0);
                            }
                        }
                        "EntryPoint" => {
                            current_reel.entry_point = text.parse().unwrap_or(0);
                        }
                        _ => {}
                    }
                } else {
                    match current_tag.as_str() {
                        "Id" if tag_stack.len() <= 2 => {
                            cpl.id = text.trim_start_matches("urn:uuid:").to_string()
                        }
                        "ContentTitleText" => cpl.content_title = text,
                        "ContentKind" => cpl.content_kind = text,
                        "IssueDate" => cpl.issue_date = text,
                        "Issuer" => cpl.issuer = text,
                        "AnnotationText" if tag_stack.len() <= 2 => cpl.annotation = text,
                        "EditRate" => cpl.edit_rate = text,
                        _ => {}
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("XML parse error: {e}")),
            _ => {}
        }
        buf.clear();
    }

    Ok(cpl)
}
