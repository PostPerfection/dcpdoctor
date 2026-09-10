use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::imf::TrackType;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImpInfo {
    pub standard: String,
    pub title: String,
    pub content_kind: String,
    pub edit_rate: String,
    pub asset_count: usize,
    pub cpl_count: usize,
    pub pkl_count: usize,
    pub total_duration_frames: u64,
    pub tracks: Vec<ImpTrack>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpTrack {
    pub kind: String,
    pub file: String,
    pub essence: TrackEssence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackEssence {
    Picture {
        width: u32,
        height: u32,
        frames: u32,
    },
    Sound {
        channels: u32,
        sample_rate: u32,
    },
    Unread,
}

pub fn get_imp_info(imp_dir: &Path) -> Option<ImpInfo> {
    let package = crate::dcp::open_dcp(imp_dir).ok()?;
    let cpl = package.cpls.iter().find_map(|(path, _)| {
        let xml = std::fs::read_to_string(path).ok()?;
        dcpdoctor_imf::parse_imf_cpl(&xml).ok()
    })?;

    let (rate_numerator, rate_denominator) = cpl.edit_rate;
    let mut info = ImpInfo {
        standard: format!("{}", package.standard),
        title: cpl.content_title.clone(),
        content_kind: cpl.content_kind.clone(),
        edit_rate: format!("{rate_numerator}/{rate_denominator}"),
        asset_count: package.assetmap.assets.len(),
        cpl_count: package.cpls.len(),
        pkl_count: package.pkls.len(),
        total_duration_frames: cpl.total_duration,
        tracks: Vec::new(),
    };

    for track in &cpl.virtual_tracks {
        let mut listed = HashSet::new();
        for resource in &track.resources {
            if resource.track_file_id.is_empty()
                || !listed.insert(resource.track_file_id.to_lowercase())
            {
                continue;
            }
            let Some(asset) = package.assetmap.assets.iter().find(|asset| {
                asset
                    .id
                    .eq_ignore_ascii_case(resource.track_file_id.as_str())
            }) else {
                continue;
            };
            let edit_rate = if resource.edit_rate.1 > 0 {
                resource.edit_rate
            } else {
                cpl.edit_rate
            };
            info.tracks.push(ImpTrack {
                // the variant names are the CPL's own sequence names
                kind: format!("{:?}", track.track_type),
                file: asset.path.clone(),
                essence: read_essence(&imp_dir.join(&asset.path), track.track_type, edit_rate),
            });
        }
    }

    Some(info)
}

fn read_essence(path: &Path, track_type: TrackType, edit_rate: (u32, u32)) -> TrackEssence {
    let Some(path) = path.to_str() else {
        return TrackEssence::Unread;
    };
    match track_type {
        TrackType::MainImage => read_picture(path),
        TrackType::MainAudio => read_sound(path, edit_rate),
        _ => TrackEssence::Unread,
    }
}

fn read_picture(path: &str) -> TrackEssence {
    let mut reader = asdcplib::as02::jp2k::MxfReader::new();
    if reader.open_read(path).is_err() {
        return TrackEssence::Unread;
    }
    match reader.picture_descriptor() {
        Ok(descriptor) => TrackEssence::Picture {
            width: descriptor.stored_width,
            height: descriptor.stored_height,
            frames: descriptor.container_duration,
        },
        Err(_) => TrackEssence::Unread,
    }
}

// the clip-wrapped sound reader wants the edit rate the CPL plays the resource at
fn read_sound(path: &str, edit_rate: (u32, u32)) -> TrackEssence {
    let (numerator, denominator) = edit_rate;
    let Ok(numerator) = i32::try_from(numerator) else {
        return TrackEssence::Unread;
    };
    let Ok(denominator) = i32::try_from(denominator.max(1)) else {
        return TrackEssence::Unread;
    };
    let mut reader = asdcplib::as02::pcm::MxfReader::new();
    if reader
        .open_read(path, asdcplib::Rational::new(numerator, denominator))
        .is_err()
    {
        return TrackEssence::Unread;
    }
    match reader.audio_descriptor() {
        Ok(descriptor) => TrackEssence::Sound {
            channels: descriptor.channel_count,
            sample_rate: sampling_rate_hz(descriptor.audio_sampling_rate),
        },
        Err(_) => TrackEssence::Unread,
    }
}

fn sampling_rate_hz(rate: asdcplib::Rational) -> u32 {
    if rate.denominator == 0 {
        return 0;
    }
    u32::try_from(rate.numerator / rate.denominator).unwrap_or(0)
}
