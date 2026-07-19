use std::path::Path;

pub use dcpdoctor_parse::{Cpl, Reel, ReelAsset};

use crate::assetmap::ParseXmlFile;

impl ParseXmlFile for Cpl {
    fn parse(file: &Path) -> Option<Self> {
        std::fs::read_to_string(file)
            .ok()
            .and_then(|xml| dcpdoctor_parse::parse_cpl(&xml))
    }
}
