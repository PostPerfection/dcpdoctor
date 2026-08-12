use quick_xml::Reader;
use quick_xml::events::Event;
use serde::{Deserialize, Serialize};

use crate::{local_name, local_name_end, root_is, strip_urn_uuid};

/// A single asset in the PKL.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PklAsset {
    pub id: String,
    #[serde(rename = "type")]
    pub asset_type: String,
    pub annotation: String,
    pub original_filename: String,
    pub hash: String,
    /// The `Algorithm` URI of the asset's `HashAlgorithm` element. Empty when the
    /// element is absent, which is required in an IMF PKL and not allowed in a
    /// DCP one. The element itself is empty, so the URI is the whole value.
    pub hash_algorithm: String,
    pub size: i64,
}

/// Parsed Packing List (PKL).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Pkl {
    pub id: String,
    pub annotation: String,
    pub creator: String,
    pub issuer: String,
    pub issue_date: String,
    pub assets: Vec<PklAsset>,
}

/// Parse a PKL XML string.
pub fn parse_pkl(xml: &str) -> Option<Pkl> {
    if !root_is(xml, "PackingList") {
        return None;
    }

    let mut reader = Reader::from_str(xml);
    let mut pkl = Pkl::default();
    let mut in_asset = false;
    let mut current_asset = PklAsset::default();
    let mut current_tag = String::new();
    // number of open elements, so top-level fields sit at depth 2
    let mut depth = 0usize;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                depth += 1;
                let name = local_name(&e);
                match name.as_str() {
                    "Asset" => {
                        in_asset = true;
                        current_asset = PklAsset::default();
                    }
                    "HashAlgorithm" if in_asset => {
                        current_asset.hash_algorithm =
                            crate::attribute(&e, "Algorithm").unwrap_or_default();
                        current_tag = name;
                    }
                    _ => current_tag = name,
                }
            }
            Ok(Event::Empty(e)) => {
                if in_asset && local_name(&e) == "HashAlgorithm" {
                    current_asset.hash_algorithm =
                        crate::attribute(&e, "Algorithm").unwrap_or_default();
                }
            }
            Ok(Event::End(e)) => {
                depth = depth.saturating_sub(1);
                let name = local_name_end(&e);
                if name == "Asset" && in_asset {
                    if !current_asset.id.is_empty() {
                        pkl.assets.push(std::mem::take(&mut current_asset));
                    }
                    in_asset = false;
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
                        "Id" => current_asset.id = strip_urn_uuid(&text),
                        "Type" => current_asset.asset_type = text,
                        "AnnotationText" => current_asset.annotation = text,
                        "OriginalFileName" => current_asset.original_filename = text,
                        "Hash" => current_asset.hash = text,
                        "Size" => current_asset.size = text.parse().unwrap_or(0),
                        _ => {}
                    }
                } else {
                    match current_tag.as_str() {
                        // guarded so nested Ids don't clobber the PKL id
                        "Id" if depth <= 2 => pkl.id = strip_urn_uuid(&text),
                        "AnnotationText" if depth <= 2 => pkl.annotation = text,
                        "Creator" => pkl.creator = text,
                        "Issuer" => pkl.issuer = text,
                        "IssueDate" => pkl.issue_date = text,
                        _ => {}
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => return None,
            _ => {}
        }
    }

    Some(pkl)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PKL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<PackingList xmlns="http://www.smpte-ra.org/schemas/429-8/2007/PKL">
  <Id>urn:uuid:4ceb5f2c-38ca-40c5-978f-cc2dc98c3772</Id>
  <AnnotationText>pkl note</AnnotationText>
  <IssueDate>2025-01-01T00:00:00+00:00</IssueDate>
  <Issuer>dci-ctp</Issuer>
  <Creator>dci-ctp</Creator>
  <AssetList>
    <Asset>
      <Id>urn:uuid:b9867f6a-2aee-4869-bd9d-affb34a8c1d1</Id>
      <AnnotationText>the cpl</AnnotationText>
      <Hash>Q0lqBCWQW113PAgvQLJhCKV49Z4=</Hash>
      <Size>816</Size>
      <Type>text/xml</Type>
      <OriginalFileName>CPL.xml</OriginalFileName>
    </Asset>
  </AssetList>
</PackingList>"#;

    #[test]
    fn test_parse_pkl() {
        let pkl = parse_pkl(PKL).unwrap();
        assert_eq!(pkl.id, "4ceb5f2c-38ca-40c5-978f-cc2dc98c3772");
        assert_eq!(pkl.annotation, "pkl note");
        assert_eq!(pkl.creator, "dci-ctp");
        assert_eq!(pkl.issuer, "dci-ctp");
        assert_eq!(pkl.assets.len(), 1);

        let a = &pkl.assets[0];
        assert_eq!(a.id, "b9867f6a-2aee-4869-bd9d-affb34a8c1d1");
        assert_eq!(a.annotation, "the cpl");
        assert_eq!(a.hash, "Q0lqBCWQW113PAgvQLJhCKV49Z4=");
        assert_eq!(a.size, 816);
        assert_eq!(a.asset_type, "text/xml");
        assert_eq!(a.original_filename, "CPL.xml");
    }

    #[test]
    fn hash_algorithm_comes_from_the_algorithm_attribute() {
        // ST 2067-2 declares HashAlgorithm as ds:DigestMethodType, whose value is
        // the Algorithm attribute of an otherwise empty element.
        let xml = PKL.replace(
            "<OriginalFileName>CPL.xml</OriginalFileName>",
            r#"<OriginalFileName>CPL.xml</OriginalFileName>
      <HashAlgorithm Algorithm="http://www.w3.org/2000/09/xmldsig#sha1"/>"#,
        );
        let pkl = parse_pkl(&xml).unwrap();
        assert_eq!(
            pkl.assets[0].hash_algorithm,
            "http://www.w3.org/2000/09/xmldsig#sha1"
        );
    }

    #[test]
    fn hash_algorithm_is_empty_when_the_element_is_absent() {
        let pkl = parse_pkl(PKL).unwrap();
        assert!(pkl.assets[0].hash_algorithm.is_empty());
    }

    #[test]
    fn test_rejects_non_pkl_root() {
        assert!(parse_pkl(r#"<CompositionPlaylist><Id>x</Id></CompositionPlaylist>"#).is_none());
    }
}
