//! no std::fs and no native deps here: this crate has to build for wasm32-unknown-unknown.

mod assetmap;
mod cpl;
mod pkl;

pub use assetmap::{Asset, AssetMap, parse_assetmap};
pub use cpl::{Cpl, Reel, ReelAsset, parse_cpl};
pub use pkl::{Pkl, PklAsset, parse_pkl};

/// Strip "urn:uuid:" prefix from UUID strings.
pub fn strip_urn_uuid(s: &str) -> String {
    s.strip_prefix("urn:uuid:").unwrap_or(s).to_string()
}

/// Get local name from a start element (strip namespace prefix).
pub(crate) fn local_name(e: &quick_xml::events::BytesStart) -> String {
    String::from_utf8_lossy(e.local_name().as_ref()).to_string()
}

pub(crate) fn local_name_end(e: &quick_xml::events::BytesEnd) -> String {
    String::from_utf8_lossy(e.local_name().as_ref()).to_string()
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
