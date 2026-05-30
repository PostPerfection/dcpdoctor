//! IMF data types shared between all dcpdoctor crates.

use std::collections::HashMap;

/// Known IMF Application profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImfApplication {
    App2e,
    App5Aces,
    #[default]
    Unknown,
}

impl std::fmt::Display for ImfApplication {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImfApplication::App2e => write!(f, "Application 2E (ST 2067-21)"),
            ImfApplication::App5Aces => write!(f, "Application 5 ACES (ST 2067-50)"),
            ImfApplication::Unknown => write!(f, "Unknown Application"),
        }
    }
}

/// IMF Composition Playlist with extended metadata.
#[derive(Debug, Clone, Default)]
pub struct ImfCpl {
    pub id: String,
    pub content_title: String,
    pub edit_rate: (u32, u32),
    pub namespaces: Vec<String>,
    pub application: ImfApplication,
    pub virtual_tracks: Vec<VirtualTrack>,
    pub total_duration: u64,
    pub issue_date: String,
    pub content_kind: String,
    pub annotation: String,
    pub creator: String,
    pub issuer: String,
    pub essence_descriptors: HashMap<String, EssenceDescriptor>,
    pub all_uuids: Vec<String>,
    pub segment_count: u32,
    pub markers: Vec<Marker>,
}

/// A virtual track (sequence) in the IMF CPL.
#[derive(Debug, Clone, Default)]
pub struct VirtualTrack {
    pub id: String,
    pub track_type: TrackType,
    pub resources: Vec<TrackResource>,
}

/// Type of virtual track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrackType {
    #[default]
    MainImage,
    MainAudio,
    Subtitle,
    HearingImpaired,
    VisuallyImpaired,
    Commentary,
    Karaoke,
    ForcedNarrative,
    IAB,
    Marker,
    Other,
}

/// A resource (segment) within a virtual track.
#[derive(Debug, Clone, Default)]
pub struct TrackResource {
    pub id: String,
    pub track_file_id: String,
    pub edit_rate: (u32, u32),
    pub intrinsic_duration: u64,
    pub entry_point: u64,
    pub source_duration: u64,
}

impl TrackResource {
    /// Effective duration (source_duration if set, else intrinsic - entry_point).
    pub fn effective_duration(&self) -> u64 {
        if self.source_duration > 0 {
            self.source_duration
        } else {
            self.intrinsic_duration.saturating_sub(self.entry_point)
        }
    }
}

/// Parsed essence descriptor from EssenceDescriptorList.
#[derive(Debug, Clone, Default)]
pub struct EssenceDescriptor {
    pub id: String,
    pub linked_track_file_id: String,
    pub descriptor_type: String,
    pub container_duration: u64,
    pub sample_rate: (u32, u32),
    pub stored_width: u32,
    pub stored_height: u32,
    pub frame_layout: u8,
    pub color_primaries: String,
    pub transfer_characteristic: String,
    pub coding_equations: String,
    pub component_depth: u32,
    pub quantization_bits: u32,
    pub channel_count: u32,
    pub audio_sampling_rate: (u32, u32),
}

/// A marker annotation within a MarkerSequence.
#[derive(Debug, Clone, Default)]
pub struct Marker {
    pub label: String,
    pub scope: String,
    pub offset: u64,
}

/// A validation note/finding.
#[derive(Debug, Clone)]
pub struct ImfNote {
    pub severity: ImfSeverity,
    pub code: &'static str,
    pub message: String,
}

/// Severity levels for validation notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImfSeverity {
    Error,
    Warning,
    Info,
}
