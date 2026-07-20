pub mod advanced;
pub mod assetmap;
pub mod audio;
pub mod auto_qc;
pub mod av_sync;
pub mod bitrate;
pub mod cert_rules;
pub mod checksum_verify;
pub mod compliance;
pub mod conformance;
pub mod cpl;
pub mod dci_ctp;
pub mod dcp;
pub mod diff;
pub mod facility_check;
pub mod fix;
pub mod fixes;
pub mod frame_compare;
pub mod hash;
pub mod hdr_validate;
pub mod hfr_stereo;
pub mod imf;
pub mod imf_compliance;
pub mod info;
pub mod isdcf;
pub mod j2k;
pub mod kdm;
pub mod kdm_advanced;
pub mod loudness;
pub mod mxf;
pub mod mxf_advanced;
pub mod mxf_extract;
pub mod note;
pub mod photon;
pub mod pkl;
pub mod premium;
pub mod profiles;
pub mod qc;
pub mod qc_report;
pub mod report;
pub mod schema;
pub mod schema_validate;
pub mod server;
pub mod signature;
pub mod studio;
pub mod subtitle;
pub mod timeline;
pub mod validate;
pub mod validators;

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
    CertificateBasicConstraintsInvalid,
    CertificateKeyUsageInvalid,
    CertificateKeySizeInvalid,
    CertificateSignatureAlgorithmInvalid,
    CertificateRoleInvalid,
    CertificateThumbprintInvalid,
    CertificateOrganizationInconsistent,

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
    SoundClipping,
    SoundSilent,

    // Subtitle
    SubtitleParseError,
    SubtitleInvalidTiming,
    SubtitleFontMissing,

    // ISDCF naming
    IsdcfNamingViolation,

    // Encryption
    EncryptionDetected,
    KdmRequired,
    KdmExpired,
    KdmNotYetValid,

    // Reel continuity
    ReelDiscontinuity,
    ReelIncoherent,

    // 3D
    StereoMismatch,

    // Markers
    MarkerMissing,
    MarkerInvalid,

    // Cross-reference
    CrossRefBroken,

    // Supplemental DCP
    SupplementalOplMissing,
    SupplementalOvNotProvided,
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
            Code::CplInvalidEditRate => "cpl_invalid_edit_rate",
            Code::CplInvalidContentKind => "cpl_invalid_content_kind",
            Code::MxfUnreadable => "mxf_unreadable",
            Code::MxfHashMismatch => "mxf_hash_mismatch",
            Code::MxfInvalidStructure => "mxf_invalid_structure",
            Code::SignatureInvalid => "signature_invalid",
            Code::CertificateExpired => "certificate_expired",
            Code::CertificateChainBroken => "certificate_chain_broken",
            Code::CertificateBasicConstraintsInvalid => "certificate_basic_constraints_invalid",
            Code::CertificateKeyUsageInvalid => "certificate_key_usage_invalid",
            Code::CertificateKeySizeInvalid => "certificate_key_size_invalid",
            Code::CertificateSignatureAlgorithmInvalid => "certificate_signature_algorithm_invalid",
            Code::CertificateRoleInvalid => "certificate_role_invalid",
            Code::CertificateThumbprintInvalid => "certificate_thumbprint_invalid",
            Code::CertificateOrganizationInconsistent => "certificate_organization_inconsistent",
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
            Code::SoundClipping => "sound_clipping",
            Code::SoundSilent => "sound_silent",
            Code::SubtitleParseError => "subtitle_parse_error",
            Code::SubtitleInvalidTiming => "subtitle_invalid_timing",
            Code::SubtitleFontMissing => "subtitle_font_missing",
            Code::IsdcfNamingViolation => "isdcf_naming_violation",
            Code::EncryptionDetected => "encryption_detected",
            Code::KdmRequired => "kdm_required",
            Code::KdmExpired => "kdm_expired",
            Code::KdmNotYetValid => "kdm_not_yet_valid",
            Code::ReelDiscontinuity => "reel_discontinuity",
            Code::ReelIncoherent => "reel_incoherent",
            Code::StereoMismatch => "stereo_mismatch",
            Code::MarkerMissing => "marker_missing",
            Code::MarkerInvalid => "marker_invalid",
            Code::CrossRefBroken => "cross_ref_broken",
            Code::SupplementalOplMissing => "supplemental_opl_missing",
            Code::SupplementalOvNotProvided => "supplemental_ov_not_provided",
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
    /// OV IMP directory to resolve cross-package references when validating a
    /// supplemental IMF package. Ignored for plain DCPs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ov: Option<PathBuf>,
}

impl VerifyOptions {
    pub fn standard() -> Self {
        Self {
            check_hashes: true,
            check_signatures: true,
            check_picture_details: false,
            strict_smpte: false,
            ov: None,
        }
    }

    pub fn strict() -> Self {
        Self {
            check_hashes: true,
            check_signatures: true,
            check_picture_details: true,
            strict_smpte: true,
            ov: None,
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
