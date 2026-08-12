use quick_xml::Reader;
use quick_xml::events::Event;
use serde::{Deserialize, Serialize};

use crate::{local_name, local_name_end, strip_urn_uuid};

/// A single asset entry in the ASSETMAP.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Asset {
    pub id: String,
    pub path: String,
    pub is_pkl: bool,
    /// The chunk's `Length` in bytes. Optional in the schema and absent from
    /// packages that write no Length at all, so None means "not declared"
    /// rather than zero.
    pub length: Option<u64>,
}

/// Parsed ASSETMAP.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssetMap {
    pub id: String,
    pub creator: String,
    pub issue_date: String,
    pub assets: Vec<Asset>,
}

/// Parse an ASSETMAP XML string.
pub fn parse_assetmap(xml: &str) -> Option<AssetMap> {
    let mut reader = Reader::from_str(xml);

    let mut am = AssetMap::default();
    let mut in_asset = false;
    let mut in_chunk = false;
    let mut current_asset = Asset::default();
    let mut current_tag = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = local_name(&e);
                match name.as_str() {
                    "AssetMap" => {}
                    "Asset" => {
                        in_asset = true;
                        current_asset = Asset::default();
                    }
                    "Chunk" => in_chunk = true,
                    _ => current_tag = name,
                }
            }
            Ok(Event::End(e)) => {
                let name = local_name_end(&e);
                match name.as_str() {
                    "Asset" => {
                        if in_asset && !current_asset.id.is_empty() {
                            am.assets.push(std::mem::take(&mut current_asset));
                        }
                        in_asset = false;
                    }
                    "Chunk" => in_chunk = false,
                    _ => {}
                }
                current_tag.clear();
            }
            Ok(Event::Text(e)) => {
                let text = crate::text_of(&e);
                if text.is_empty() {
                    continue;
                }
                if in_asset {
                    match current_tag.as_str() {
                        "Id" if !in_chunk => current_asset.id = strip_urn_uuid(&text),
                        "Path" if in_chunk => current_asset.path = text,
                        "Length" if in_chunk => current_asset.length = text.parse().ok(),
                        "PackingList" if text == "true" || text == "1" => {
                            current_asset.is_pkl = true
                        }
                        _ => {}
                    }
                } else {
                    match current_tag.as_str() {
                        "Id" => am.id = strip_urn_uuid(&text),
                        "Creator" => am.creator = text,
                        "IssueDate" => am.issue_date = text,
                        _ => {}
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => return None,
            _ => {}
        }
    }

    Some(am)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMPTE_AM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee</Id>
  <Creator>test</Creator>
  <IssueDate>2024-01-01</IssueDate>
  <AssetList>
    <Asset>
      <Id>urn:uuid:11111111-2222-3333-4444-555555555555</Id>
      <PackingList>true</PackingList>
      <ChunkList>
        <Chunk>
          <Path>pkl.xml</Path>
        </Chunk>
      </ChunkList>
    </Asset>
    <Asset>
      <Id>urn:uuid:66666666-7777-8888-9999-aaaaaaaaaaaa</Id>
      <ChunkList>
        <Chunk>
          <Path>cpl.xml</Path>
        </Chunk>
      </ChunkList>
    </Asset>
  </AssetList>
</AssetMap>"#;

    #[test]
    fn test_parse_assetmap() {
        let am = parse_assetmap(SMPTE_AM).unwrap();
        assert_eq!(am.id, "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee");
        assert_eq!(am.creator, "test");
        assert_eq!(am.assets.len(), 2);
        assert_eq!(am.assets[0].path, "pkl.xml");
        assert!(am.assets[0].is_pkl);
        assert_eq!(am.assets[1].id, "66666666-7777-8888-9999-aaaaaaaaaaaa");
        assert!(!am.assets[1].is_pkl);
    }

    #[test]
    fn chunk_length_is_read_when_present_and_none_when_absent() {
        let xml = SMPTE_AM.replace(
            "<Path>pkl.xml</Path>",
            "<Path>pkl.xml</Path><Length>1234</Length>",
        );
        let am = parse_assetmap(&xml).unwrap();
        assert_eq!(am.assets[0].length, Some(1234));
        assert_eq!(am.assets[1].length, None);
    }

    #[test]
    fn test_parse_invalid_xml() {
        // should not panic on invalid xml (parser is lenient)
        let _ = parse_assetmap("not xml at all < >");
    }
}
