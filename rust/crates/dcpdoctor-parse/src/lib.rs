//! no std::fs and no native deps here: this crate has to build for wasm32-unknown-unknown.

mod assetmap;
mod cpl;
pub mod j2k;
mod pkl;

pub use assetmap::{Asset, AssetMap, parse_assetmap};
pub use cpl::{Cpl, Reel, ReelAsset, parse_cpl};
pub use pkl::{Pkl, PklAsset, parse_pkl};

/// Strip "urn:uuid:" prefix from UUID strings.
pub fn strip_urn_uuid(s: &str) -> String {
    s.strip_prefix("urn:uuid:").unwrap_or(s).to_string()
}

/// Decode and unescape a text event. quick-xml 0.41 dropped BytesText::unescape,
/// so this is decode + unescape, which is what it used to do.
pub fn text_of(e: &quick_xml::events::BytesText) -> String {
    e.decode()
        .ok()
        .and_then(|s| quick_xml::escape::unescape(&s).ok().map(|u| u.to_string()))
        .unwrap_or_default()
}

/// Get local name from a start element (strip namespace prefix).
pub(crate) fn local_name(e: &quick_xml::events::BytesStart) -> String {
    String::from_utf8_lossy(e.local_name().as_ref()).to_string()
}

pub(crate) fn local_name_end(e: &quick_xml::events::BytesEnd) -> String {
    String::from_utf8_lossy(e.local_name().as_ref()).to_string()
}

/// Value of an attribute by local name, ignoring any prefix on it.
pub(crate) fn attribute(e: &quick_xml::events::BytesStart, name: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        (a.key.local_name().as_ref() == name.as_bytes())
            .then(|| String::from_utf8_lossy(&a.value).to_string())
    })
}

/// Read the first start element and check its local name.
pub(crate) fn root_is(xml: &str, expected: &str) -> bool {
    let mut reader = quick_xml::Reader::from_str(xml);
    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(e)) => return local_name(&e) == expected,
            Ok(quick_xml::events::Event::Eof) | Err(_) => return false,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_urn_uuid() {
        assert_eq!(
            strip_urn_uuid("urn:uuid:12345678-1234-1234-1234-123456789abc"),
            "12345678-1234-1234-1234-123456789abc"
        );
        assert_eq!(strip_urn_uuid("plain-id"), "plain-id");
    }
}
