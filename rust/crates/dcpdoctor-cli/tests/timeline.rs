//! `dcpdoctor validate --timeline`: the reel structure diagram.

mod support;

use assert_cmd::Command;
use quick_xml::Reader;
use quick_xml::events::Event;
use tempfile::TempDir;

/// Reel durations in frames at 24 fps, and the timecodes they come to.
const REEL_FRAMES: [i64; 3] = [24, 12, 36];
const REEL_TIMECODES: [&str; 3] = ["00:00:01:00", "00:00:00:12", "00:00:01:12"];
const TOTAL_TIMECODE: &str = "00:00:03:00";

/// Every element name in document order, and all the text between elements.
fn parse_svg(svg: &str) -> (Vec<String>, Vec<String>) {
    let mut reader = Reader::from_str(svg);
    let mut elements = Vec::new();
    let mut text = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                elements.push(String::from_utf8_lossy(e.name().as_ref()).into_owned());
            }
            Ok(Event::Text(e)) => {
                let value = e.decode().unwrap().trim().to_string();
                if !value.is_empty() {
                    text.push(value);
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => panic!("the timeline SVG must parse as XML: {e}"),
        }
    }
    (elements, text)
}

#[test]
fn the_timeline_draws_one_reel_per_reel_with_its_duration_in_timecode() {
    let dir = TempDir::new().unwrap();
    support::write_package(
        dir.path(),
        &support::PackageSpec {
            reel_durations: REEL_FRAMES.to_vec(),
            ..Default::default()
        },
    );

    let svg_path = dir.path().join("timeline.svg");
    Command::cargo_bin("dcpdoctor")
        .unwrap()
        .args(["validate"])
        .arg(dir.path())
        .arg("--timeline")
        .arg(&svg_path)
        .assert()
        .stderr(predicates::str::contains("Timeline SVG written"));

    let svg = std::fs::read_to_string(&svg_path).unwrap();
    let (elements, text) = parse_svg(&svg);

    assert_eq!(elements.first().map(String::as_str), Some("svg"));
    assert_eq!(
        elements.iter().filter(|e| *e == "rect").count(),
        REEL_FRAMES.len(),
        "one bar per reel: {svg}"
    );

    for timecode in REEL_TIMECODES {
        assert!(
            text.iter().any(|t| t == timecode),
            "reel duration {timecode} is missing from {svg}"
        );
    }
    assert!(
        text.iter().any(|t| t.contains(TOTAL_TIMECODE)),
        "the total duration in timecode is missing from {svg}"
    );
}
