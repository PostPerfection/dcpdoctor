pub mod advanced;
pub mod assetmap;
pub mod audio;
pub mod av_sync;
pub mod bitrate;
pub mod cert_rules;
pub mod checksum_verify;
pub mod compliance;
pub mod conformance;
pub mod cpl;
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
pub mod loudness;
pub mod mxf;
pub mod mxf_advanced;
pub mod mxf_extract;
pub mod note;
pub mod photon;
pub mod pkl;
pub mod premium;
pub mod profiles;
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
    AssetmapInvalidName,
    AssetmapSizeMismatch,

    // XML
    XmlParseError,
    XmlSchemaViolation,
    SchemaValidationSkipped,
    /// A check that should have run did not (tool missing, input unreadable,
    /// measurement failed). The message names the check and the reason, so a
    /// skipped check is never mistaken for a clean one.
    CheckSkipped,
    InvalidUuid,
    MissingRequiredElement,

    // PKL
    PklHashMismatch,
    PklSizeMismatch,
    PklMissingAssetReference,
    PklAnnotationTextMismatch,

    // CPL
    CplInvalidDuration,
    CplMismatchedDurations,
    CplMissingReel,
    CplInvalidEditRate,
    CplInvalidContentKind,
    CplMissingHash,
    CplPklHashMismatch,
    CplAnnotationTextMismatch,
    CplActiveAreaInvalid,
    CplInvalidLanguage,

    // MXF
    MxfUnreadable,
    MxfHashMismatch,
    MxfInvalidStructure,
    MxfAssetIdMismatch,

    // Signature
    SignatureInvalid,
    DcpNotSigned,
    UnencryptedDcpNotSigned,
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
    PictureBitrateMeasured,
    J2kInvalidProfile,
    J2kInvalidComponentCount,
    J2kLegacyFfff,
    J2kGuardBits,
    J2kMissingTlm,
    J2kPocInvalid,
    J2kParametersVary,
    J2kCodestreamSummary,

    // Sound
    SoundInvalidSampleRate,
    SoundInvalidChannelCount,
    SoundInvalidQuantization,
    SoundInvalidBlockAlign,
    SoundClipping,
    SoundSilent,
    MainSoundConfigInvalid,
    SoundChannelConfigInvalid,

    // Subtitle
    SubtitleParseError,
    SubtitleInvalidTiming,
    SubtitleFrameRateMismatch,
    SubtitleFontMissing,
    SubtitleGlyphMissing,
    SubtitleFirstEventEarly,
    SubtitleLineCount,
    SubtitleLineLength,
    SubtitleDuration,
    SubtitleSpacing,
    ClosedCaptionLineCount,
    ClosedCaptionLineLength,
    ClosedCaptionCharset,
    ClosedCaptionLayout,
    TimedTextSizeExceeded,
    TimedTextIdMismatch,
    SubtitleEmptyText,
    SubtitleInvalidIssueDate,
    SubtitleNamespaceCount,
    SubtitleEntryPoint,
    ClosedCaptionEntryPoint,
    SubtitleOverlapsReel,
    SubtitleMissingFromReel,
    SubtitleLanguageMismatch,
    ClosedCaptionCountMismatch,
    SubtitleFontTooLarge,
    ClosedCaptionInteropOverlap,
    PartiallyEncrypted,

    // Playback compatibility
    ProjectorFrameRateSupport,
    ProjectorFourKStereoSupport,
    DistributorAudioChannelCount,

    // ISDCF naming
    IsdcfNamingViolation,

    // Encryption
    EncryptionDetected,
    KdmRequired,
    KdmExpired,
    KdmNotYetValid,
    KdmThumbprintInvalid,
    KdmContentAuthenticatorInvalid,
    KdmAssumeTrustConflict,

    // Reel continuity
    ReelDiscontinuity,
    ReelIncoherent,
    ReelTooShort,
    ReelEditRateMismatch,
    CompositionMetadataAssetMismatch,

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

    // Auxiliary data (ST 429-18, e.g. Dolby Atmos IAB)
    AuxDataDetected,

    // Package hygiene
    ForeignFileInPackage,
    EmptyFileInPackage,
    NonAsciiFilename,
}

impl Code {
    pub fn as_str(&self) -> &'static str {
        match self {
            Code::MissingAssetmap => "missing_assetmap",
            Code::MissingPkl => "missing_pkl",
            Code::MissingCpl => "missing_cpl",
            Code::AssetNotFound => "asset_not_found",
            Code::DuplicateAssetId => "duplicate_asset_id",
            Code::AssetmapInvalidName => "assetmap_invalid_name",
            Code::AssetmapSizeMismatch => "assetmap_size_mismatch",
            Code::XmlParseError => "xml_parse_error",
            Code::XmlSchemaViolation => "xml_schema_violation",
            Code::SchemaValidationSkipped => "schema_validation_skipped",
            Code::CheckSkipped => "check_skipped",
            Code::InvalidUuid => "invalid_uuid",
            Code::MissingRequiredElement => "missing_required_element",
            Code::PklHashMismatch => "pkl_hash_mismatch",
            Code::PklSizeMismatch => "pkl_size_mismatch",
            Code::PklMissingAssetReference => "pkl_missing_asset_reference",
            Code::PklAnnotationTextMismatch => "pkl_annotation_text_mismatch",
            Code::CplInvalidDuration => "cpl_invalid_duration",
            Code::CplMismatchedDurations => "cpl_mismatched_durations",
            Code::CplMissingReel => "cpl_missing_reel",
            Code::CplInvalidEditRate => "cpl_invalid_edit_rate",
            Code::CplInvalidContentKind => "cpl_invalid_content_kind",
            Code::CplMissingHash => "cpl_missing_hash",
            Code::CplPklHashMismatch => "cpl_pkl_hash_mismatch",
            Code::CplAnnotationTextMismatch => "cpl_annotation_text_mismatch",
            Code::CplActiveAreaInvalid => "cpl_active_area_invalid",
            Code::CplInvalidLanguage => "cpl_invalid_language",
            Code::MxfUnreadable => "mxf_unreadable",
            Code::MxfHashMismatch => "mxf_hash_mismatch",
            Code::MxfInvalidStructure => "mxf_invalid_structure",
            Code::MxfAssetIdMismatch => "mxf_asset_id_mismatch",
            Code::SignatureInvalid => "signature_invalid",
            Code::DcpNotSigned => "dcp_not_signed",
            Code::UnencryptedDcpNotSigned => "unencrypted_dcp_not_signed",
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
            Code::PictureBitrateMeasured => "picture_bitrate_measured",
            Code::J2kInvalidProfile => "j2k_invalid_profile",
            Code::J2kInvalidComponentCount => "j2k_invalid_component_count",
            Code::J2kLegacyFfff => "j2k_legacy_ffff",
            Code::J2kGuardBits => "j2k_guard_bits",
            Code::J2kMissingTlm => "j2k_missing_tlm",
            Code::J2kPocInvalid => "j2k_poc_invalid",
            Code::J2kParametersVary => "j2k_parameters_vary",
            Code::J2kCodestreamSummary => "j2k_codestream_summary",
            Code::SoundInvalidSampleRate => "sound_invalid_sample_rate",
            Code::SoundInvalidChannelCount => "sound_invalid_channel_count",
            Code::SoundInvalidQuantization => "sound_invalid_quantization",
            Code::SoundInvalidBlockAlign => "sound_invalid_block_align",
            Code::SoundClipping => "sound_clipping",
            Code::SoundSilent => "sound_silent",
            Code::MainSoundConfigInvalid => "main_sound_config_invalid",
            Code::SoundChannelConfigInvalid => "sound_channel_config_invalid",
            Code::SubtitleParseError => "subtitle_parse_error",
            Code::SubtitleInvalidTiming => "subtitle_invalid_timing",
            Code::SubtitleFrameRateMismatch => "subtitle_frame_rate_mismatch",
            Code::SubtitleFontMissing => "subtitle_font_missing",
            Code::SubtitleGlyphMissing => "subtitle_glyph_missing",
            Code::SubtitleFirstEventEarly => "subtitle_first_event_early",
            Code::SubtitleLineCount => "subtitle_line_count",
            Code::SubtitleLineLength => "subtitle_line_length",
            Code::SubtitleDuration => "subtitle_duration",
            Code::SubtitleSpacing => "subtitle_spacing",
            Code::ClosedCaptionLineCount => "closed_caption_line_count",
            Code::ClosedCaptionLineLength => "closed_caption_line_length",
            Code::ClosedCaptionCharset => "closed_caption_charset",
            Code::ClosedCaptionLayout => "closed_caption_layout",
            Code::TimedTextSizeExceeded => "timed_text_size_exceeded",
            Code::TimedTextIdMismatch => "timed_text_id_mismatch",
            Code::SubtitleEmptyText => "subtitle_empty_text",
            Code::SubtitleInvalidIssueDate => "subtitle_invalid_issue_date",
            Code::SubtitleNamespaceCount => "subtitle_namespace_count",
            Code::SubtitleEntryPoint => "subtitle_entry_point",
            Code::ClosedCaptionEntryPoint => "closed_caption_entry_point",
            Code::SubtitleOverlapsReel => "subtitle_overlaps_reel",
            Code::SubtitleMissingFromReel => "subtitle_missing_from_reel",
            Code::SubtitleLanguageMismatch => "subtitle_language_mismatch",
            Code::ClosedCaptionCountMismatch => "closed_caption_count_mismatch",
            Code::SubtitleFontTooLarge => "subtitle_font_too_large",
            Code::ClosedCaptionInteropOverlap => "closed_caption_interop_overlap",
            Code::PartiallyEncrypted => "partially_encrypted",
            Code::ProjectorFrameRateSupport => "projector_frame_rate_support",
            Code::ProjectorFourKStereoSupport => "projector_4k_stereo_support",
            Code::DistributorAudioChannelCount => "distributor_audio_channel_count",
            Code::IsdcfNamingViolation => "isdcf_naming_violation",
            Code::EncryptionDetected => "encryption_detected",
            Code::KdmRequired => "kdm_required",
            Code::KdmExpired => "kdm_expired",
            Code::KdmNotYetValid => "kdm_not_yet_valid",
            Code::KdmThumbprintInvalid => "kdm_thumbprint_invalid",
            Code::KdmContentAuthenticatorInvalid => "kdm_content_authenticator_invalid",
            Code::KdmAssumeTrustConflict => "kdm_assume_trust_conflict",
            Code::ReelDiscontinuity => "reel_discontinuity",
            Code::ReelIncoherent => "reel_incoherent",
            Code::ReelTooShort => "reel_too_short",
            Code::ReelEditRateMismatch => "reel_edit_rate_mismatch",
            Code::CompositionMetadataAssetMismatch => "composition_metadata_asset_mismatch",
            Code::StereoMismatch => "stereo_mismatch",
            Code::MarkerMissing => "marker_missing",
            Code::MarkerInvalid => "marker_invalid",
            Code::CrossRefBroken => "cross_ref_broken",
            Code::SupplementalOplMissing => "supplemental_opl_missing",
            Code::SupplementalOvNotProvided => "supplemental_ov_not_provided",
            Code::AuxDataDetected => "aux_data_detected",
            Code::ForeignFileInPackage => "foreign_file_in_package",
            Code::EmptyFileInPackage => "empty_file_in_package",
            Code::NonAsciiFilename => "non_ascii_filename",
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
    /// Run the picture codestream checks over every frame instead of only the
    /// first. Expensive on a feature, so it is opt-in the way hash checking is.
    #[serde(default)]
    pub scan_every_frame: bool,
    pub strict_smpte: bool,
    /// OV IMP directory to resolve cross-package references when validating a
    /// supplemental IMF package. Ignored for plain DCPs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ov: Option<PathBuf>,
    /// KDM XML for an encrypted DCP. With `recipient_key`, content keys are
    /// unwrapped and used to decrypt essence so the frame-level checks run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kdm: Option<PathBuf>,
    /// Recipient RSA private key (PEM) matching the KDM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipient_key: Option<PathBuf>,
}

impl VerifyOptions {
    pub fn standard() -> Self {
        Self {
            check_hashes: true,
            check_signatures: true,
            check_picture_details: false,
            scan_every_frame: false,
            strict_smpte: false,
            ov: None,
            kdm: None,
            recipient_key: None,
        }
    }

    pub fn strict() -> Self {
        Self {
            check_hashes: true,
            check_signatures: true,
            check_picture_details: true,
            scan_every_frame: false,
            strict_smpte: true,
            ov: None,
            kdm: None,
            recipient_key: None,
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
