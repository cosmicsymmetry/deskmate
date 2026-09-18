//! What CI asserts about every face, and how a human gets to look at them.
//!
//! The assertions here are deliberately *structural* rather than pixel-exact.
//! A committed golden PNG of anti-aliased type is a test that fails on a
//! `resvg` upgrade and says nothing about whether the design is good. What is
//! checked instead is what a face can be wrong about without anybody noticing:
//! text escaping the canvas, a document that stops parsing, and a frame whose
//! size the asset path would refuse.

#![cfg(test)]

use super::cases;
use crate::face_render::{frame_from_svg, pixmap_from_svg};
use protocol::{SCENE_CANVAS_HEIGHT, SCENE_CANVAS_WIDTH};

/// Writes every case as a PNG when `DESKMATE_FACE_DUMP` names a directory.
///
/// This is the review loop, not an assertion. It is a normal test so it cannot
/// rot: if a face stops rendering, this fails alongside everything else.
#[test]
fn every_case_renders_and_can_be_dumped_for_review() {
    let dump = std::env::var_os("DESKMATE_FACE_DUMP").map(std::path::PathBuf::from);
    if let Some(directory) = &dump {
        std::fs::create_dir_all(directory).expect("the dump directory is creatable");
    }

    let cases = cases::all();
    assert!(!cases.is_empty(), "there is something to review");

    for case in &cases {
        let pixmap = pixmap_from_svg(&case.svg)
            .unwrap_or_else(|error| panic!("case {} did not render: {error}", case.name));
        assert_eq!(pixmap.width(), u32::try_from(SCENE_CANVAS_WIDTH).unwrap());
        assert_eq!(pixmap.height(), u32::try_from(SCENE_CANVAS_HEIGHT).unwrap());

        if let Some(directory) = &dump {
            let path = directory.join(format!("{}.png", case.name));
            pixmap
                .save_png(&path)
                .unwrap_or_else(|error| panic!("writing {}: {error}", path.display()));
            println!("wrote {}", path.display());
        }
    }
}

#[test]
fn every_case_produces_a_frame_the_asset_path_would_accept() {
    // The device's decoder reads the 12-byte LVGL header and then exactly
    // width*height*2 bytes. A face that produced anything else would be
    // refused on the wire, long after this test could have caught it.
    let expected = 12 + (SCENE_CANVAS_WIDTH as usize * SCENE_CANVAS_HEIGHT as usize * 2);
    for case in cases::all() {
        let frame = frame_from_svg(&case.svg)
            .unwrap_or_else(|error| panic!("case {} did not render: {error}", case.name));
        assert_eq!(
            frame.bytes.len(),
            expected,
            "case {} produced a frame of the wrong size",
            case.name
        );
        assert!(
            frame.digest.iter().any(|byte| *byte != 0),
            "case {} produced an unhashed frame",
            case.name
        );
    }
}

#[test]
fn no_case_places_a_baseline_or_a_box_outside_the_canvas() {
    // The failure this catches is the one a fixed panel makes invisible: text
    // that is laid out past the edge simply is not there, with no scrollbar,
    // no clipping artifact, and no error.
    for case in cases::all() {
        for (attribute, limit) in [
            ("y=\"", f64::from(SCENE_CANVAS_HEIGHT)),
            ("x=\"", f64::from(SCENE_CANVAS_WIDTH)),
        ] {
            for value in attribute_values(&case.svg, "<text ", attribute) {
                assert!(
                    value >= 0.0 && value <= limit,
                    "case {} positions text at {attribute}{value}, outside 0..={limit}",
                    case.name
                );
            }
        }
    }
}

#[test]
fn no_case_draws_a_text_run_past_the_canvas_edge() {
    // The check the position-only assertion above cannot make: a run can start
    // well inside the canvas and still end outside it. On a fixed panel the
    // overflow is invisible -- no clipping artifact, no scrollbar, just missing
    // words or two runs on top of each other.
    //
    // This is the test that a tracked eyebrow measured without its tracking
    // fails: "DUBAI, UNITED ARAB EMIRATES" at 15px tracked 1.4.
    for case in cases::all() {
        for run in text_runs(&case.svg) {
            let width =
                super::svg::tracked_width(&run.content, run.size, run.weight, run.letter_spacing);
            let (left, right) = match run.anchor.as_str() {
                "end" => (run.x - width, run.x),
                "middle" => (run.x - width / 2.0, run.x + width / 2.0),
                _ => (run.x, run.x + width),
            };
            assert!(
                left >= -0.5 && right <= f64::from(SCENE_CANVAS_WIDTH) + 0.5,
                "case {}: run {:?} spans {left:.1}..{right:.1}, outside 0..={}",
                case.name,
                run.content,
                SCENE_CANVAS_WIDTH
            );
        }
    }
}

#[test]
fn every_case_name_is_unique_and_filesystem_safe() {
    // The names become filenames in the dump directory, and a duplicate would
    // silently overwrite the case it collided with -- a review that quietly
    // shows one fewer face than it claims.
    let mut names: Vec<&str> = cases::all().iter().map(|case| case.name).collect();
    let count = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), count, "case names collide: {names:?}");
    for name in names {
        assert!(
            name.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "case name {name:?} is not filesystem-safe"
        );
    }
}

/// One `<text>` node, as the document carries it.
struct TextRun {
    x: f64,
    content: String,
    size: f64,
    weight: u16,
    anchor: String,
    letter_spacing: f64,
}

/// Reads every text node back out of an authored document.
///
/// Parsing our own output rather than instrumenting the builder: it means the
/// assertion is made against the bytes that will be rasterized, so a builder
/// that formats an attribute wrongly is caught too.
fn text_runs(svg: &str) -> Vec<TextRun> {
    svg.split("<text ")
        .skip(1)
        .filter_map(|node| {
            let end = node.find("</text>")?;
            let node = &node[..end];
            let (attributes, content) = node.split_once('>')?;
            Some(TextRun {
                x: attribute(attributes, "x")?.parse().ok()?,
                content: unescape(content),
                size: attribute(attributes, "font-size")?.parse().ok()?,
                weight: attribute(attributes, "font-weight")?.parse().ok()?,
                anchor: attribute(attributes, "text-anchor")
                    .unwrap_or("start")
                    .to_owned(),
                letter_spacing: attribute(attributes, "letter-spacing")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0.0),
            })
        })
        .collect()
}

fn attribute<'a>(attributes: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{name}=\"");
    let start = attributes.find(&needle)? + needle.len();
    let rest = &attributes[start..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

/// Reverses `svg::escape` so a run is measured as the glyphs it will draw,
/// not as its entity-encoded source.
fn unescape(content: &str) -> String {
    content
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Pulls one numeric attribute off every occurrence of an element.
fn attribute_values(svg: &str, element: &str, attribute: &str) -> Vec<f64> {
    svg.split(element)
        .skip(1)
        .filter_map(|node| {
            let start = node.find(attribute)? + attribute.len();
            let rest = &node[start..];
            let end = rest.find('"')?;
            rest[..end].parse::<f64>().ok()
        })
        .collect()
}
