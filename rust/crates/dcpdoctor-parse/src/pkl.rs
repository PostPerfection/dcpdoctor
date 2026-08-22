use quick_xml::NsReader;
use quick_xml::events::Event;
use quick_xml::name::ResolveResult;
use serde::{Deserialize, Serialize};

use crate::{local_name, local_name_end, root_is, strip_urn_uuid};

/// The namespace an element resolved to, empty when it is bound to none or to
/// an undeclared prefix.
fn resolved_namespace(resolved: ResolveResult) -> String {
    match resolved {
        ResolveResult::Bound(ns) => String::from_utf8_lossy(ns.as_ref()).to_string(),
        _ => String::new(),
    }
}

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
    /// The namespace the `HashAlgorithm` element itself resolved to. ST 2067-2
    /// declares the element in the PKL schema, so binding it to the xmldsig
    /// namespace of its `ds:DigestMethodType` type is wrong even though the
    /// local name matches.
    pub hash_algorithm_namespace: String,
    pub size: i64,
    /// The `Size` element held text that is no integer, so `size` is 0 without
    /// the PKL having declared 0.
    #[serde(default)]
    pub size_unparseable: bool,
}

/// Parsed Packing List (PKL).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Pkl {
    pub id: String,
    /// The namespace of the `PackingList` root element.
    pub namespace: String,
    pub annotation: String,
    pub creator: String,
    pub issuer: String,
    pub issue_date: String,
    pub assets: Vec<PklAsset>,
    /// Assets dropped because they carry no usable `Id`, so nothing can be
    /// checked against them.
    #[serde(default)]
    pub assets_without_id: usize,
}

/// Parse a PKL XML string.
pub fn parse_pkl(xml: &str) -> Option<Pkl> {
    if !root_is(xml, "PackingList") {
        return None;
    }

    let mut reader = NsReader::from_str(xml);
    let mut pkl = Pkl::default();
    let mut in_asset = false;
    let mut current_asset = PklAsset::default();
    let mut current_tag = String::new();
    // number of open elements, so top-level fields sit at depth 2
    let mut depth = 0usize;

    loop {
        match reader.read_resolved_event() {
            Ok((ns, Event::Start(e))) => {
                depth += 1;
                let name = local_name(&e);
                match name.as_str() {
                    "PackingList" if depth == 1 => pkl.namespace = resolved_namespace(ns),
                    "Asset" => {
                        in_asset = true;
                        current_asset = PklAsset::default();
                    }
                    "HashAlgorithm" if in_asset => {
                        current_asset.hash_algorithm =
                            crate::attribute(&e, "Algorithm").unwrap_or_default();
                        current_asset.hash_algorithm_namespace = resolved_namespace(ns);
                        current_tag = name;
                    }
                    _ => current_tag = name,
                }
            }
            Ok((ns, Event::Empty(e))) => {
                if in_asset && local_name(&e) == "HashAlgorithm" {
                    current_asset.hash_algorithm =
                        crate::attribute(&e, "Algorithm").unwrap_or_default();
                    current_asset.hash_algorithm_namespace = resolved_namespace(ns);
                }
            }
            Ok((_, Event::End(e))) => {
                depth = depth.saturating_sub(1);
                let name = local_name_end(&e);
                if name == "Asset" && in_asset {
                    if current_asset.id.is_empty() {
                        pkl.assets_without_id += 1;
                    } else {
                        pkl.assets.push(std::mem::take(&mut current_asset));
                    }
                    in_asset = false;
                }
                current_tag.clear();
            }
            Ok((_, Event::Text(e))) => {
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
                        "Size" => match text.parse() {
                            Ok(size) => current_asset.size = size,
                            Err(_) => current_asset.size_unparseable = true,
                        },
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
            Ok((_, Event::Eof)) => break,
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
    fn hash_algorithm_namespace_separates_the_pkl_binding_from_xmldsig() {
        let correct = PKL.replace(
            "<OriginalFileName>CPL.xml</OriginalFileName>",
            r#"<OriginalFileName>CPL.xml</OriginalFileName>
      <HashAlgorithm Algorithm="http://www.w3.org/2000/09/xmldsig#sha1"/>"#,
        );
        let pkl = parse_pkl(&correct).unwrap();
        assert_eq!(
            pkl.namespace,
            "http://www.smpte-ra.org/schemas/429-8/2007/PKL"
        );
        assert_eq!(
            pkl.assets[0].hash_algorithm_namespace,
            "http://www.smpte-ra.org/schemas/429-8/2007/PKL"
        );

        let wrong = PKL.replace(
            "<OriginalFileName>CPL.xml</OriginalFileName>",
            r#"<OriginalFileName>CPL.xml</OriginalFileName>
      <ds:HashAlgorithm xmlns:ds="http://www.w3.org/2000/09/xmldsig#" Algorithm="http://www.w3.org/2000/09/xmldsig#sha1"/>"#,
        );
        let pkl = parse_pkl(&wrong).unwrap();
        assert_eq!(
            pkl.assets[0].hash_algorithm_namespace, "http://www.w3.org/2000/09/xmldsig#",
            "an element bound to xmldsig must not be reported as the PKL one"
        );
    }

    #[test]
    fn hash_algorithm_is_empty_when_the_element_is_absent() {
        let pkl = parse_pkl(PKL).unwrap();
        assert!(pkl.assets[0].hash_algorithm.is_empty());
    }

    #[test]
    fn an_asset_with_no_id_is_counted_rather_than_dropped_silently() {
        let xml = PKL.replace("<Id>urn:uuid:b9867f6a-2aee-4869-bd9d-affb34a8c1d1</Id>", "");
        let pkl = parse_pkl(&xml).unwrap();
        assert!(pkl.assets.is_empty());
        assert_eq!(
            pkl.assets_without_id, 1,
            "an asset with no Id gets no size or hash check, so the drop has to be visible"
        );
    }

    #[test]
    fn a_size_that_is_no_integer_is_not_read_back_as_zero() {
        let xml = PKL.replace("<Size>816</Size>", "<Size>eight hundred</Size>");
        let pkl = parse_pkl(&xml).unwrap();
        assert!(pkl.assets[0].size_unparseable);

        let pkl = parse_pkl(PKL).unwrap();
        assert!(!pkl.assets[0].size_unparseable);
    }

    #[test]
    fn test_rejects_non_pkl_root() {
        assert!(parse_pkl(r#"<CompositionPlaylist><Id>x</Id></CompositionPlaylist>"#).is_none());
    }
}
