use std::path::Path;

pub use dcpdoctor_parse::{Asset, AssetMap, strip_urn_uuid};

/// Read and parse one of the DCP XML files from disk. The parsing itself lives in
/// dcpdoctor-parse so the wasm build can share it.
pub trait ParseXmlFile: Sized {
    fn parse(file: &Path) -> Option<Self>;
}

impl ParseXmlFile for AssetMap {
    fn parse(file: &Path) -> Option<Self> {
        std::fs::read_to_string(file)
            .ok()
            .and_then(|xml| dcpdoctor_parse::parse_assetmap(&xml))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_assetmap_file() {
        let dir = tempfile::tempdir().unwrap();
        let am_path = dir.path().join("ASSETMAP.xml");
        std::fs::write(
            &am_path,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee</Id>
  <Creator>test</Creator>
  <IssueDate>2024-01-01</IssueDate>
  <AssetList>
    <Asset>
      <Id>urn:uuid:11111111-2222-3333-4444-555555555555</Id>
      <ChunkList>
        <Chunk>
          <Path>pkl.xml</Path>
        </Chunk>
      </ChunkList>
    </Asset>
  </AssetList>
</AssetMap>"#,
        )
        .unwrap();

        let am = AssetMap::parse(&am_path).unwrap();
        assert_eq!(am.id, "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee");
        assert_eq!(am.creator, "test");
        assert_eq!(am.assets.len(), 1);
        assert_eq!(am.assets[0].path, "pkl.xml");
    }

    #[test]
    fn test_parse_missing_file() {
        assert!(AssetMap::parse(Path::new("/nonexistent/ASSETMAP.xml")).is_none());
    }
}
