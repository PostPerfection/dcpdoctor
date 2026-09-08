/// SVG timeline generation from CPL reel data.
use std::io::Write;

use crate::cpl::Cpl;
use crate::report::escape_markup;

/// Frame rate a reel is drawn at when neither it nor the CPL declares an edit
/// rate, so a timecode is still labelled at the DCI base rate.
const FALLBACK_FRAME_RATE: u32 = 24;

/// Generate an SVG timeline visualization of a CPL.
pub fn write_timeline_svg<W: Write>(cpl: &Cpl, writer: &mut W) -> std::io::Result<()> {
    let total_duration: i64 = cpl.reels.iter().map(|r| r.picture.duration).sum();
    if total_duration <= 0 {
        return Ok(());
    }

    let width = 800.0;
    let height = 140.0;
    let bar_height = 40.0;
    let bar_y = 50.0;

    let frame_rate = |edit_rate: &str| {
        let (numerator, denominator) = dcpdoctor_imf::parse::parse_edit_rate(edit_rate);
        match (numerator, denominator) {
            (0, _) | (_, 0) => None,
            _ => Some(numerator.div_ceil(denominator)),
        }
    };
    let composition_rate = frame_rate(&cpl.edit_rate).unwrap_or(FALLBACK_FRAME_RATE);

    writeln!(
        writer,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\">"
    )?;
    writeln!(
        writer,
        "<text x=\"10\" y=\"20\" font-family=\"sans-serif\" font-size=\"14\" fill=\"#333\">{}</text>",
        escape_markup(&cpl.content_title)
    )?;
    writeln!(
        writer,
        "<text x=\"10\" y=\"38\" font-family=\"sans-serif\" font-size=\"11\" fill=\"#666\">Total: {total_duration} frames ({})</text>",
        crate::j2k::frame_to_timecode(total_duration as u32, composition_rate)
    )?;

    let colors = [
        "#4a90d9", "#50c878", "#f5a623", "#d0021b", "#9b59b6", "#1abc9c",
    ];
    let mut x = 10.0;
    for (i, reel) in cpl.reels.iter().enumerate() {
        let reel_width = (reel.picture.duration as f64 / total_duration as f64) * (width - 20.0);
        let color = colors[i % colors.len()];
        let reel_rate = frame_rate(&reel.picture.edit_rate).unwrap_or(composition_rate);
        let duration = crate::j2k::frame_to_timecode(reel.picture.duration as u32, reel_rate);
        writeln!(
            writer,
            "<rect x=\"{x}\" y=\"{bar_y}\" width=\"{reel_width}\" height=\"{bar_height}\" fill=\"{color}\" stroke=\"#fff\" stroke-width=\"1\"><title>Reel {} {duration}</title></rect>",
            i + 1
        )?;
        if reel_width > 30.0 {
            let cx = x + reel_width / 2.0;
            let cy = bar_y + bar_height / 2.0 + 4.0;
            writeln!(
                writer,
                "<text x=\"{cx}\" y=\"{cy}\" font-family=\"sans-serif\" font-size=\"10\" fill=\"#fff\" text-anchor=\"middle\">R{}</text>",
                i + 1
            )?;
        }
        let label_y = bar_y + bar_height + 16.0;
        let label_x = x + reel_width / 2.0;
        writeln!(
            writer,
            "<text x=\"{label_x}\" y=\"{label_y}\" font-family=\"sans-serif\" font-size=\"10\" fill=\"#666\" text-anchor=\"middle\">{duration}</text>"
        )?;
        x += reel_width;
    }

    writeln!(writer, "</svg>")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpl::{Reel, ReelAsset};

    fn reel(duration: i64, edit_rate: &str) -> Reel {
        Reel {
            picture: ReelAsset {
                duration,
                edit_rate: edit_rate.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn every_reel_gets_its_duration_as_a_timecode() {
        let cpl = Cpl {
            content_title: "Two reels".into(),
            edit_rate: "24 1".into(),
            reels: vec![reel(36, "24 1"), reel(48, "24 1")],
            ..Default::default()
        };

        let mut svg = Vec::new();
        write_timeline_svg(&cpl, &mut svg).unwrap();
        let svg = String::from_utf8(svg).unwrap();

        assert_eq!(svg.matches("<rect").count(), 2);
        assert!(svg.contains("00:00:01:12"), "reel 1 is 36 frames: {svg}");
        assert!(svg.contains("00:00:02:00"), "reel 2 is 48 frames: {svg}");
        assert!(
            svg.contains("Total: 84 frames (00:00:03:12)"),
            "the total needs a timecode too: {svg}"
        );
    }

    #[test]
    fn a_reel_with_no_edit_rate_falls_back_to_the_composition_rate() {
        let cpl = Cpl {
            edit_rate: "25 1".into(),
            reels: vec![reel(30, "")],
            ..Default::default()
        };

        let mut svg = Vec::new();
        write_timeline_svg(&cpl, &mut svg).unwrap();
        let svg = String::from_utf8(svg).unwrap();

        assert!(svg.contains("00:00:01:05"), "30 frames at 25 fps: {svg}");
    }

    #[test]
    fn an_ampersand_in_the_title_does_not_break_the_xml() {
        let cpl = Cpl {
            content_title: "Salt & Pepper".into(),
            edit_rate: "24 1".into(),
            reels: vec![reel(24, "24 1")],
            ..Default::default()
        };

        let mut svg = Vec::new();
        write_timeline_svg(&cpl, &mut svg).unwrap();
        let svg = String::from_utf8(svg).unwrap();

        assert!(svg.contains("Salt &amp; Pepper"), "{svg}");
    }
}
