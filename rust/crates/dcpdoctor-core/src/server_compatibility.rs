use crate::mxf::TWO_K_MAX_STORED_WIDTH;

pub const SMALLEST_PLAYABLE_FRAME_BYTES: u64 = 16384;

const DOREMI_SHORTEST_REEL_SECONDS: f64 = 5.0;

const FLAT_2K_STORED_SIZE: (u32, u32) = (1998, 1080);
const FLAT_4K_STORED_SIZE: (u32, u32) = (3996, 2160);
const GDC_UNPLAYABLE_FLAT_FRAMES_PER_SECOND: u32 = 25;

const DOREMI_FOUR_K_MAX_FRAMES_PER_SECOND: u32 = 30;

// None is a rate with no safer rate to name
const NOT_WIDELY_PLAYED_FRAME_RATES: [(u32, Option<u32>); 5] = [
    (25, Some(24)),
    (30, None),
    (48, Some(24)),
    (50, Some(25)),
    (60, Some(30)),
];
const INTEROP_NOT_WIDELY_PLAYED_FRAMES_PER_SECOND: u32 = 25;

const TRUE_TYPE_SIGNATURE: [u8; 4] = [0x00, 0x01, 0x00, 0x00];
const APPLE_TRUE_TYPE_SIGNATURE: [u8; 4] = *b"true";

const PORTABLE_NAME_PUNCTUATION: [char; 3] = ['.', '_', '-'];

pub fn frame_rate_not_widely_played(frames_per_second: u32, interop: bool) -> Option<String> {
    let (_, instead) = NOT_WIDELY_PLAYED_FRAME_RATES
        .iter()
        .find(|(rate, _)| *rate == frames_per_second)?;
    let advice = match instead {
        Some(better) => format!(", so consider delivering at {better} fps instead"),
        None => String::new(),
    };
    let interop_advice = if interop
        && frames_per_second == INTEROP_NOT_WIDELY_PLAYED_FRAMES_PER_SECOND
    {
        format!(
            ". Interop at {INTEROP_NOT_WIDELY_PLAYED_FRAMES_PER_SECOND} fps plays on fewer still, so deliver it as SMPTE"
        )
    } else {
        String::new()
    };
    Some(format!(
        "DCP is {frames_per_second} fps, which not all projectors play{advice}{interop_advice}"
    ))
}

pub fn four_k_stereoscopic(stored_width: u32, stereoscopic: bool) -> Option<String> {
    if !stereoscopic || stored_width <= TWO_K_MAX_STORED_WIDTH {
        return None;
    }
    Some("DCP is 4K 3D, which only a very limited number of projectors play".to_string())
}

pub fn small_frame(frame_index: u32, frame_bytes: u64) -> Option<String> {
    if frame_bytes >= SMALLEST_PLAYABLE_FRAME_BYTES {
        return None;
    }
    Some(format!(
        "Frame {frame_index} is {frame_bytes} bytes, under the {SMALLEST_PLAYABLE_FRAME_BYTES} a Dolby DSS200 server needs to play without crashing"
    ))
}

pub fn short_reel(reel_number: usize, seconds: f64) -> Option<String> {
    if seconds >= DOREMI_SHORTEST_REEL_SECONDS {
        return None;
    }
    Some(format!(
        "Reel {reel_number} is {seconds:.2}s long. A Doremi server can stop playback on a reel shorter than {DOREMI_SHORTEST_REEL_SECONDS}s"
    ))
}

pub fn flat_at_25_fps(
    stored_width: u32,
    stored_height: u32,
    frames_per_second: u32,
) -> Option<String> {
    let is_flat =
        [FLAT_2K_STORED_SIZE, FLAT_4K_STORED_SIZE].contains(&(stored_width, stored_height));
    if !is_flat || frames_per_second != GDC_UNPLAYABLE_FLAT_FRAMES_PER_SECOND {
        return None;
    }
    Some(format!(
        "DCP is Flat at {GDC_UNPLAYABLE_FLAT_FRAMES_PER_SECOND} fps, which a GDC SX-2001 server will not play. It plays a 16:9 container at {GDC_UNPLAYABLE_FLAT_FRAMES_PER_SECOND} fps"
    ))
}

pub fn four_k_above_30_fps(stored_width: u32, frames_per_second: u32) -> Option<String> {
    if stored_width <= TWO_K_MAX_STORED_WIDTH
        || frames_per_second <= DOREMI_FOUR_K_MAX_FRAMES_PER_SECOND
    {
        return None;
    }
    Some(format!(
        "DCP is 4K at {frames_per_second} fps. A Doremi server plays 4K only up to {DOREMI_FOUR_K_MAX_FRAMES_PER_SECOND} fps"
    ))
}

pub fn picture_size_rules_apply(frames_per_second: u32) -> bool {
    frames_per_second == GDC_UNPLAYABLE_FLAT_FRAMES_PER_SECOND
        || frames_per_second > DOREMI_FOUR_K_MAX_FRAMES_PER_SECOND
}

pub fn top_aligned_subtitle(cue_time: &str) -> String {
    format!(
        "A subtitle at {cue_time} is aligned to the top. Servers place top-aligned text by its baseline, so it sits higher than its position says"
    )
}

pub fn font_not_true_type(font_name: &str, font_data: &[u8]) -> Option<String> {
    let is_true_type = font_data.starts_with(&TRUE_TYPE_SIGNATURE)
        || font_data.starts_with(&APPLE_TRUE_TYPE_SIGNATURE);
    if is_true_type {
        return None;
    }
    Some(format!(
        "Subtitle font {font_name} is not a TrueType font. Some distributors' QC rejects other font formats"
    ))
}

pub fn unportable_name(name: &str) -> Option<String> {
    if is_portable_name(name) {
        return None;
    }
    Some(format!(
        "File or folder name has characters other than {}: {name}",
        portable_characters_description()
    ))
}

pub fn unportable_dcp_folder_name(name: &str) -> Option<String> {
    if is_portable_name(name) {
        return None;
    }
    Some(format!(
        "DCP folder name has characters other than {}: {name}",
        portable_characters_description()
    ))
}

fn is_portable_name(name: &str) -> bool {
    name.chars().all(|character| {
        character.is_ascii_alphanumeric() || PORTABLE_NAME_PUNCTUATION.contains(&character)
    })
}

fn portable_characters_description() -> String {
    let [dot, underscore, hyphen] = PORTABLE_NAME_PUNCTUATION;
    format!("letters, digits, '{dot}', '{underscore}' and '{hyphen}'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_rates_not_widely_played_warn_with_the_rate_to_deliver_instead() {
        for (rate, expected) in [
            (
                25,
                "DCP is 25 fps, which not all projectors play, so consider delivering at 24 fps instead",
            ),
            (30, "DCP is 30 fps, which not all projectors play"),
            (
                48,
                "DCP is 48 fps, which not all projectors play, so consider delivering at 24 fps instead",
            ),
            (
                50,
                "DCP is 50 fps, which not all projectors play, so consider delivering at 25 fps instead",
            ),
            (
                60,
                "DCP is 60 fps, which not all projectors play, so consider delivering at 30 fps instead",
            ),
        ] {
            assert_eq!(
                frame_rate_not_widely_played(rate, false).as_deref(),
                Some(expected)
            );
        }
    }

    #[test]
    fn twenty_four_fps_is_silent() {
        assert!(frame_rate_not_widely_played(24, false).is_none());
        assert!(frame_rate_not_widely_played(24, true).is_none());
    }

    #[test]
    fn interop_at_25_fps_is_told_to_deliver_smpte() {
        assert_eq!(
            frame_rate_not_widely_played(25, true).as_deref(),
            Some(
                "DCP is 25 fps, which not all projectors play, so consider delivering at 24 fps instead. Interop at 25 fps plays on fewer still, so deliver it as SMPTE"
            )
        );
        assert!(
            !frame_rate_not_widely_played(25, false)
                .unwrap()
                .contains("SMPTE")
        );
        assert!(
            !frame_rate_not_widely_played(48, true)
                .unwrap()
                .contains("SMPTE")
        );
    }

    #[test]
    fn four_k_stereoscopic_warns() {
        assert_eq!(
            four_k_stereoscopic(4096, true).as_deref(),
            Some("DCP is 4K 3D, which only a very limited number of projectors play")
        );
    }

    #[test]
    fn two_k_stereoscopic_and_four_k_monoscopic_are_silent() {
        assert!(four_k_stereoscopic(2048, true).is_none());
        assert!(four_k_stereoscopic(4096, false).is_none());
    }

    #[test]
    fn a_frame_under_the_dss200_minimum_warns_with_its_index_and_size() {
        let message = small_frame(12, 557).expect("557 bytes is under the minimum");
        assert_eq!(
            message,
            "Frame 12 is 557 bytes, under the 16384 a Dolby DSS200 server needs to play without crashing"
        );
    }

    #[test]
    fn a_frame_at_the_dss200_minimum_is_silent() {
        assert!(small_frame(0, SMALLEST_PLAYABLE_FRAME_BYTES).is_none());
    }

    #[test]
    fn a_reel_under_five_seconds_warns() {
        let message = short_reel(2, 3.5).expect("3.5s is under 5s");
        assert_eq!(
            message,
            "Reel 2 is 3.50s long. A Doremi server can stop playback on a reel shorter than 5s"
        );
    }

    #[test]
    fn a_reel_of_five_seconds_is_silent() {
        assert!(short_reel(1, 5.0).is_none());
    }

    #[test]
    fn flat_at_25_fps_warns_at_2k_and_4k() {
        for (width, height) in [(1998, 1080), (3996, 2160)] {
            let message = flat_at_25_fps(width, height, 25).expect("Flat at 25 fps");
            assert_eq!(
                message,
                "DCP is Flat at 25 fps, which a GDC SX-2001 server will not play. It plays a 16:9 container at 25 fps"
            );
        }
    }

    #[test]
    fn flat_at_24_fps_and_a_full_container_at_25_fps_are_silent() {
        assert!(flat_at_25_fps(1998, 1080, 24).is_none());
        assert!(flat_at_25_fps(2048, 1080, 25).is_none());
        assert!(flat_at_25_fps(2048, 858, 25).is_none());
    }

    #[test]
    fn four_k_above_30_fps_warns_with_the_rate() {
        let message = four_k_above_30_fps(4096, 48).expect("4K at 48 fps");
        assert_eq!(
            message,
            "DCP is 4K at 48 fps. A Doremi server plays 4K only up to 30 fps"
        );
    }

    #[test]
    fn four_k_at_30_fps_and_2k_at_48_fps_are_silent() {
        assert!(four_k_above_30_fps(4096, 30).is_none());
        assert!(four_k_above_30_fps(2048, 48).is_none());
    }

    #[test]
    fn the_picture_size_matters_only_at_25_fps_and_above_30_fps() {
        assert!(picture_size_rules_apply(25));
        assert!(picture_size_rules_apply(48));
        assert!(!picture_size_rules_apply(24));
        assert!(!picture_size_rules_apply(30));
    }

    #[test]
    fn a_top_aligned_subtitle_names_its_time() {
        assert_eq!(
            top_aligned_subtitle("5.000s"),
            "A subtitle at 5.000s is aligned to the top. Servers place top-aligned text by its baseline, so it sits higher than its position says"
        );
    }

    #[test]
    fn open_type_cff_and_collection_fonts_warn() {
        for data in [b"OTTO\x00\x0a".as_slice(), b"ttcf\x00\x01".as_slice()] {
            let message = font_not_true_type("arial.otf", data).expect("not TrueType");
            assert_eq!(
                message,
                "Subtitle font arial.otf is not a TrueType font. Some distributors' QC rejects other font formats"
            );
        }
    }

    #[test]
    fn true_type_fonts_are_silent() {
        assert!(font_not_true_type("a.ttf", &[0x00, 0x01, 0x00, 0x00, 0x00, 0x0a]).is_none());
        assert!(font_not_true_type("a.ttf", b"true\x00\x0a").is_none());
    }

    #[test]
    fn a_name_with_a_space_warns() {
        assert_eq!(
            unportable_name("my reel.mxf").expect("a space is unportable"),
            "File or folder name has characters other than letters, digits, '.', '_' and '-': my reel.mxf"
        );
        assert_eq!(
            unportable_dcp_folder_name("My Film").expect("a space is unportable"),
            "DCP folder name has characters other than letters, digits, '.', '_' and '-': My Film"
        );
    }

    #[test]
    fn letters_digits_dots_underscores_and_hyphens_are_silent() {
        assert!(unportable_name("MyFilm_FTR-1_F_EN-XX_51_2K_20261003_SMPTE_OV.xml").is_none());
        assert!(unportable_dcp_folder_name("MyFilm_FTR-1_F_EN-XX").is_none());
    }
}
