pub mod assetmap;
pub mod cpl;
pub mod dcp;
pub mod diff;
pub mod fix;
pub mod hash;
pub mod info;
pub mod mxf;
pub mod note;
pub mod pkl;
pub mod report;
pub mod schema;
pub mod server;
pub mod signature;
pub mod subtitle;
pub mod timeline;
pub mod validate;

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};

/// Severity level for validation notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Severity::Info => write!(f, "INFO"),
            Severity::Warning => write!(f, "WARNING"),
            Severity::Error => write!(f, "ERROR"),
        }
    }
}

/// Error codes for validation findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Code {
    // Structure
    MissingAssetmap,
    MissingPkl,
    MissingCpl,
    AssetNotFound,
    DuplicateAssetId,

    // XML
    XmlParseError,
    XmlSchemaViolation,
    InvalidUuid,
    MissingRequiredElement,

    // PKL
    PklHashMismatch,
    PklMissingAssetReference,

    // CPL
    CplInvalidDuration,
    CplMismatchedDurations,
    CplMissingReel,
    CplInvalidFrameRate,
    CplInvalidEditRate,
    CplInvalidContentKind,

    // MXF
    MxfUnreadable,
    MxfHashMismatch,
    MxfInvalidStructure,

    // Signature
    SignatureInvalid,
    CertificateExpired,
    CertificateChainBroken,

    // SMPTE compliance
    SmpteNamingViolation,
    SmpteNamespaceWrong,

    // Interop compliance
    InteropNamespaceWrong,

    // Picture
    PictureInvalidResolution,
    PictureInvalidFrameRate,
    J2kBitrateExceeded,
    J2kInvalidProfile,
    J2kInvalidComponentCount,

    // Sound
    SoundInvalidSampleRate,
    SoundInvalidChannelCount,

    // Subtitle
    SubtitleParseError,
    SubtitleInvalidTiming,
    SubtitleFontMissing,

    // ISDCF naming
    IsdcfNamingViolation,

    // Encryption
    EncryptionDetected,
    KdmRequired,

    // Reel continuity
    ReelDiscontinuity,

    // 3D
    StereoMismatch,

    // Markers
    MarkerMissing,
    MarkerInvalid,

    // Cross-reference
    CrossRefBroken,

    // Supplemental DCP
    SupplementalOplMissing,
}

impl Code {
    pub fn as_str(&self) -> &'static str {
        match self {
            Code::MissingAssetmap => "missing_assetmap",
            Code::MissingPkl => "missing_pkl",
            Code::MissingCpl => "missing_cpl",
            Code::AssetNotFound => "asset_not_found",
            Code::DuplicateAssetId => "duplicate_asset_id",
            Code::XmlParseError => "xml_parse_error",
            Code::XmlSchemaViolation => "xml_schema_violation",
            Code::InvalidUuid => "invalid_uuid",
            Code::MissingRequiredElement => "missing_required_element",
            Code::PklHashMismatch => "pkl_hash_mismatch",
            Code::PklMissingAssetReference => "pkl_missing_asset_reference",
            Code::CplInvalidDuration => "cpl_invalid_duration",
            Code::CplMismatchedDurations => "cpl_mismatched_durations",
            Code::CplMissingReel => "cpl_missing_reel",
            Code::CplInvalidFrameRate => "cpl_invalid_frame_rate",
            Code::CplInvalidEditRate => "cpl_invalid_edit_rate",
            Code::CplInvalidContentKind => "cpl_invalid_content_kind",
            Code::MxfUnreadable => "mxf_unreadable",
            Code::MxfHashMismatch => "mxf_hash_mismatch",
            Code::MxfInvalidStructure => "mxf_invalid_structure",
            Code::SignatureInvalid => "signature_invalid",
            Code::CertificateExpired => "certificate_expired",
            Code::CertificateChainBroken => "certificate_chain_broken",
            Code::SmpteNamingViolation => "smpte_naming_violation",
            Code::SmpteNamespaceWrong => "smpte_namespace_wrong",
            Code::InteropNamespaceWrong => "interop_namespace_wrong",
            Code::PictureInvalidResolution => "picture_invalid_resolution",
            Code::PictureInvalidFrameRate => "picture_invalid_frame_rate",
            Code::J2kBitrateExceeded => "j2k_bitrate_exceeded",
            Code::J2kInvalidProfile => "j2k_invalid_profile",
            Code::J2kInvalidComponentCount => "j2k_invalid_component_count",
            Code::SoundInvalidSampleRate => "sound_invalid_sample_rate",
            Code::SoundInvalidChannelCount => "sound_invalid_channel_count",
            Code::SubtitleParseError => "subtitle_parse_error",
            Code::SubtitleInvalidTiming => "subtitle_invalid_timing",
            Code::SubtitleFontMissing => "subtitle_font_missing",
            Code::IsdcfNamingViolation => "isdcf_naming_violation",
            Code::EncryptionDetected => "encryption_detected",
            Code::KdmRequired => "kdm_required",
            Code::ReelDiscontinuity => "reel_discontinuity",
            Code::StereoMismatch => "stereo_mismatch",
            Code::MarkerMissing => "marker_missing",
            Code::MarkerInvalid => "marker_invalid",
            Code::CrossRefBroken => "cross_ref_broken",
            Code::SupplementalOplMissing => "supplemental_opl_missing",
        }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A single validation finding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub severity: Severity,
    pub code: Code,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub line: u32,
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

impl fmt::Display for Note {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {} - {}", self.severity, self.code, self.message)?;
        if let Some(ref file) = self.file {
            write!(f, " ({})", file.display())?;
        }
        Ok(())
    }
}

/// DCP standard detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Standard {
    #[default]
    Unknown,
    Interop,
    Smpte,
}

impl fmt::Display for Standard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Standard::Unknown => write!(f, "Unknown"),
            Standard::Interop => write!(f, "Interop"),
            Standard::Smpte => write!(f, "SMPTE"),
        }
    }
}

/// Options for DCP verification.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VerifyOptions {
    pub check_hashes: bool,
    pub check_signatures: bool,
    pub check_picture_details: bool,
    pub strict_smpte: bool,
}

impl VerifyOptions {
    pub fn standard() -> Self {
        Self {
            check_hashes: true,
            check_signatures: true,
            check_picture_details: false,
            strict_smpte: false,
        }
    }

    pub fn strict() -> Self {
        Self {
            check_hashes: true,
            check_signatures: true,
            check_picture_details: true,
            strict_smpte: true,
        }
    }
}

/// Result of DCP verification.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VerifyResult {
    pub standard: Standard,
    pub notes: Vec<Note>,
    pub error_count: u32,
    pub warning_count: u32,
}

impl VerifyResult {
    pub fn ok(&self) -> bool {
        self.error_count == 0
    }

    pub fn add(&mut self, note: Note) {
        match note.severity {
            Severity::Error => self.error_count += 1,
            Severity::Warning => self.warning_count += 1,
            Severity::Info => {}
        }
        self.notes.push(note);
    }
}

/// Verify a DCP at the given path.
pub fn verify(dcp_dir: &Path, opts: &VerifyOptions) -> VerifyResult {
    validate::verify_dcp(dcp_dir, opts)
}
