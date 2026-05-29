use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};

/// Parsed ASSETMAP structure.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssetMap {
    pub id: String,
    pub assets: Vec<Asset>,
    pub is_smpte: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Asset {
    pub id: String,
    pub path: String,
    pub is_pkl: bool,
}

/// Parse an ASSETMAP XML string.
pub fn parse_assetmap(xml: &str) -> Result<AssetMap, String> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut assetmap = AssetMap::default();
    let mut current_asset = Asset::default();
    let mut in_asset = false;
    let mut current_tag = String::new();

    // Detect SMPTE vs Interop from namespace
    let is_smpte = xml.contains("http://www.smpte-ra.org/schemas/429-9/2007/AM");
    assetmap.is_smpte = is_smpte;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                current_tag = name.clone();
                if name == "Asset" {
                    in_asset = true;
                    current_asset = Asset::default();
                }
            }
            Ok(Event::End(ref e)) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                if name == "Asset" && in_asset {
                    assetmap.assets.push(current_asset.clone());
                    in_asset = false;
                }
            }
            Ok(Event::Text(ref e)) => {
                let text = e.unescape().unwrap_or_default().to_string();
                let text = text.trim().to_string();
                if text.is_empty() {
                    continue;
                }
                if !in_asset && current_tag == "Id" {
                    assetmap.id = text.trim_start_matches("urn:uuid:").to_string();
                } else if in_asset && current_tag == "Id" {
                    current_asset.id = text.trim_start_matches("urn:uuid:").to_string();
                } else if in_asset && current_tag == "Path" {
                    current_asset.path = text;
                } else if in_asset
                    && current_tag == "PackingList"
                    && (text == "true" || text == "1")
                {
                    current_asset.is_pkl = true;
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("XML parse error: {e}")),
            _ => {}
        }
        buf.clear();
    }

    Ok(assetmap)
}
