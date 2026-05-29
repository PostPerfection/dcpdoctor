use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};

/// Parsed PKL structure.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Pkl {
    pub id: String,
    pub annotation: String,
    pub issue_date: String,
    pub issuer: String,
    pub assets: Vec<PklAsset>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PklAsset {
    pub id: String,
    pub annotation: String,
    pub hash: String,
    pub size: u64,
    pub asset_type: String,
    pub original_filename: String,
}

/// Parse a PKL XML string.
pub fn parse_pkl(xml: &str) -> Result<Pkl, String> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut pkl = Pkl::default();
    let mut current_asset = PklAsset::default();
    let mut in_asset = false;
    let mut current_tag = String::new();
    let mut depth = 0;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                current_tag = name.clone();
                depth += 1;
                if name == "Asset" {
                    in_asset = true;
                    current_asset = PklAsset::default();
                }
            }
            Ok(Event::End(ref e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                depth -= 1;
                if name == "Asset" && in_asset {
                    pkl.assets.push(current_asset.clone());
                    in_asset = false;
                }
            }
            Ok(Event::Text(ref e)) => {
                let text = e.unescape().unwrap_or_default().to_string();
                let text = text.trim().to_string();
                if text.is_empty() {
                    continue;
                }
                if in_asset {
                    match current_tag.as_str() {
                        "Id" => current_asset.id = text.trim_start_matches("urn:uuid:").to_string(),
                        "AnnotationText" => current_asset.annotation = text,
                        "Hash" => current_asset.hash = text,
                        "Size" => current_asset.size = text.parse().unwrap_or(0),
                        "Type" => current_asset.asset_type = text,
                        "OriginalFileName" => current_asset.original_filename = text,
                        _ => {}
                    }
                } else {
                    match current_tag.as_str() {
                        "Id" if depth == 2 => {
                            pkl.id = text.trim_start_matches("urn:uuid:").to_string()
                        }
                        "AnnotationText" if depth == 2 => pkl.annotation = text,
                        "IssueDate" => pkl.issue_date = text,
                        "Issuer" => pkl.issuer = text,
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

    Ok(pkl)
}
