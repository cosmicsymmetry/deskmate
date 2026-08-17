//! The clock faces draw no card title.
//!
//! `title` remains a schema and wire field — it names the card in the
//! companion's library — but no clock face renders it. See
//! `docs/superpowers/specs/2026-08-17-deskmate-clock-title-removal-design.md`.
//! Golden PNGs cannot state this: they pin one frame per case, not the
//! relationship between two frames that differ only in their title. This test
//! measures that relationship, so a chip reintroduced by accident fails here
//! rather than quietly re-blessing itself into the goldens.

use lvgl_sim::{
    LOGICAL_WIDTH, RenderRequest, SimField, SimFieldValue, SimOrientation, SimTemplate, Simulator,
};

/// 2025-08-12 12:00:00 UTC at +4, matching `cases.rs`'s clock instants.
const NOW: i64 = 1_755_000_000;
const OFFSET: i16 = 240;

fn request(template: SimTemplate, title: &str, orientation: SimOrientation) -> RenderRequest {
    RenderRequest {
        template,
        fields: vec![
            SimField {
                name: "title".to_string(),
                value: SimFieldValue::Text(title.to_string()),
            },
            SimField {
                name: "show_seconds".to_string(),
                value: SimFieldValue::Boolean(true),
            },
        ],
        utc_offset_minutes: OFFSET,
        now_unix_seconds: NOW,
        orientation,
    }
}

/// Renders one face three times at both orientations, varying only the title,
/// and asserts the frames are identical. A helper rather than an inner loop
/// over a template array, so Task 2 can add the analog face with one line —
/// and so this file never contains a single-element `for`, which
/// `clippy::single_element_loop` rejects under `-D warnings`.
fn assert_title_is_not_rendered(sim: &mut Simulator, template: SimTemplate) {
    for orientation in [SimOrientation::Landscape, SimOrientation::LandscapeFlipped] {
        let short = sim
            .render(&request(template, "Desk", orientation))
            .expect("short title");
        let long = sim
            .render(&request(template, "WWWWWWWWWWWWWWWW", orientation))
            .expect("long title");
        let empty = sim
            .render(&request(template, "", orientation))
            .expect("empty title");
        assert_eq!(
            short, long,
            "{template:?} at {orientation:?}: a longer title changed the frame"
        );
        assert_eq!(
            short, empty,
            "{template:?} at {orientation:?}: an empty title changed the frame"
        );
    }
}

#[test]
fn clock_faces_ignore_the_title_field() {
    let mut sim = Simulator::new().expect("simulator");
    assert_title_is_not_rendered(&mut sim, SimTemplate::DigitalClock);
}

/// The rows the title chip used to occupy: it sat at `y = 2 * DESKMATE_GRID`
/// (16) and stood `DESKMATE_CHIP_HEIGHT` (32) tall. The re-centered hero
/// starts at y = 64, so nothing may light this band.
const OLD_CHIP_BAND: std::ops::Range<usize> = 16..48;

#[test]
fn digital_clock_leaves_the_old_chip_band_dark() {
    let mut sim = Simulator::new().expect("simulator");
    let pixels = sim
        .render(&request(
            SimTemplate::DigitalClock,
            "Desk",
            SimOrientation::Landscape,
        ))
        .expect("digital clock");
    let width = LOGICAL_WIDTH as usize;
    let lit: Vec<usize> = OLD_CHIP_BAND
        .clone()
        .filter(|row| (0..width).any(|column| pixels[row * width + column] != 0))
        .collect();
    assert!(
        lit.is_empty(),
        "rows {lit:?} are lit inside the removed chip's band"
    );
}
