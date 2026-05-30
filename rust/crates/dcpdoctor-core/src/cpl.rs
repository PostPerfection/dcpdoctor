use quick_xml::Reader;
use quick_xml::events::Event;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::assetmap::strip_urn_uuid;

/// A reel essence reference (picture or sound).
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
    pub stereoscopic: bool,
}

/// Parsed Composition Playlist (CPL).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cpl {
    pub id: String,
    pub content_title: String,
    pub content_kind: String,
    pub issue_date: String,
    pub reels: Vec<Reel>,
}

impl Cpl {
    /// Parse a CPL XML file.
    pub fn parse(file: &Path) -> Option<Self> {
        let xml = std::fs::read_to_string(file).ok()?;

        // Quick check: is this a CompositionPlaylist?
        {
            let mut peek = Reader::from_str(&xml);
            let mut found = false;
            loop {
                match peek.read_event() {
                    Ok(Event::Start(e)) => {
                        let name = local_name(&e);
                        if name == "CompositionPlaylist" {
                            found = true;
                        }
                        break;
                    }
                    Ok(Event::Eof) => break,
                    Err(_) => break,
                    _ => {}
                }
            }
            if !found {
                return None;
            }
        }

        let mut reader = Reader::from_str(&xml);
        let mut cpl = Cpl::default();
        let mut in_reel = false;
        let mut in_main_picture = false;
        let mut in_main_sound = false;
        let mut current_reel = Reel::default();
        let mut current_tag = String::new();
        let mut depth_in_reel = 0u32;

        loop {
            match reader.read_event() {
                Ok(Event::Start(e)) => {
                    let name = local_name(&e);
                    match name.as_str() {
                        "Reel" => {
                            in_reel = true;
                            depth_in_reel = 0;
                            current_reel = Reel::default();
                        }
                        "MainPicture" | "MainStereoscopicPicture" if in_reel => {
                            in_main_picture = true;
                            if name == "MainStereoscopicPicture" {
                                current_reel.stereoscopic = true;
                            }
                        }
                        "MainSound" if in_reel => {
                            in_main_sound = true;
                        }
                        _ => {
                            if in_reel {
                                depth_in_reel += 1;
                            }
                            current_tag = name;
                        }
                    }
                }
                Ok(Event::End(e)) => {
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
                        _ => {
                            if in_reel && depth_in_reel > 0 {
                                depth_in_reel -= 1;
                            }
                        }
                    }
                    current_tag.clear();
                }
                Ok(Event::Text(e)) => {
                    let text = e.unescape().ok().map(|s| s.to_string()).unwrap_or_default();
                    if text.is_empty() {
                        continue;
                    }

                    if in_main_picture {
                        match current_tag.as_str() {
                            "Id" => current_reel.picture.id = strip_urn_uuid(&text),
                            "EditRate" => current_reel.picture.edit_rate = text,
                            "Duration" | "IntrinsicDuration" => {
                                current_reel.picture.duration = text.parse().unwrap_or(0)
                            }
                            "EntryPoint" => {
                                current_reel.picture.entry_point = text.parse().unwrap_or(0)
                            }
                            _ => {}
                        }
                    } else if in_main_sound {
                        match current_tag.as_str() {
                            "Id" => current_reel.sound.id = strip_urn_uuid(&text),
                            "EditRate" => current_reel.sound.edit_rate = text,
                            "Duration" | "IntrinsicDuration" => {
                                current_reel.sound.duration = text.parse().unwrap_or(0)
                            }
                            "EntryPoint" => {
                                current_reel.sound.entry_point = text.parse().unwrap_or(0)
                            }
                            _ => {}
                        }
                    } else if in_reel {
                        if current_tag == "Id" {
                            current_reel.id = strip_urn_uuid(&text);
                        }
                    } else {
                        match current_tag.as_str() {
                            "Id" => cpl.id = strip_urn_uuid(&text),
                            "ContentTitleText" => cpl.content_title = text,
                            "ContentKind" => cpl.content_kind = text,
                            "IssueDate" => cpl.issue_date = text,
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
}

fn local_name(e: &quick_xml::events::BytesStart) -> String {
    String::from_utf8_lossy(e.local_name().as_ref()).to_string()
}

fn local_name_end(e: &quick_xml::events::BytesEnd) -> String {
    String::from_utf8_lossy(e.local_name().as_ref()).to_string()
}
