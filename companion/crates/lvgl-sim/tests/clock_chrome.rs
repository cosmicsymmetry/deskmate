//! The clock faces draw no card title.
//!
//! `title` remains a schema field — it names the card in the companion's
//! window — but no clock face renders it. See
//! `docs/superpowers/specs/2026-08-17-deskmate-clock-title-removal-design.md`.
//! Golden PNGs cannot state this: they pin one frame per case, not the
//! relationship between two frames that differ only in their title. This test
//! measures that relationship, so a chip reintroduced by accident fails here
//! rather than quietly re-blessing itself into the goldens.
//!
//! Drives the production path — a configured card through
//! `app_core::preview_card_scene` — rather than a hand-posed request, so it
//! also covers the card-to-scene step where a title could leak back in.

use app_core::{
    AppConfig, CardAlert, CardSettings, DisplayTemplate, RefreshPolicy, WidgetTapAction,
};
use lvgl_sim::scene::SceneRenderRequest;
use lvgl_sim::{LOGICAL_WIDTH, SimOrientation, Simulator};

/// 2025-08-12 12:00:00 UTC at +4, matching `cases.rs`'s clock instants.
const NOW: i64 = 1_755_000_000;
const OFFSET: i16 = 240;

fn clock_config(template: &DisplayTemplate, title: &str) -> AppConfig {
    AppConfig {
        cards: vec![CardSettings::Clock {
            id: "clock".into(),
            title: title.into(),
            show_seconds: true,
            template: template.clone(),
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::DeviceLocal,
            alert: CardAlert::None,
            dwell_seconds: None,
        }],
        ..AppConfig::default()
    }
}

fn render(
    sim: &mut Simulator,
    template: &DisplayTemplate,
    title: &str,
    orientation: SimOrientation,
) -> Vec<u16> {
    let config = clock_config(template, title);
    let scene = app_core::preview_card_scene(&config, "clock", &[]).expect("a clock scene");
    sim.render_scene(&SceneRenderRequest {
        scene,
        assets: Vec::new(),
        utc_offset_minutes: OFFSET,
        now_unix_seconds: NOW,
        timer: None,
        fields: Vec::new(),
        orientation,
    })
    .expect("render the clock scene")
}

/// Renders one face three times at both orientations, varying only the title,
/// and asserts the frames are identical. A helper rather than an inner loop
/// over a template array, so this file never contains a single-element `for`,
/// which `clippy::single_element_loop` rejects under `-D warnings`.
fn assert_title_is_not_rendered(sim: &mut Simulator, template: &DisplayTemplate) {
    for orientation in [SimOrientation::Landscape, SimOrientation::LandscapeFlipped] {
        let short = render(sim, template, "Desk", orientation);
        let long = render(sim, template, "WWWWWWWWWWWWWWWW", orientation);
        let empty = render(sim, template, "", orientation);
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
    assert_title_is_not_rendered(&mut sim, &DisplayTemplate::DigitalClock);
    assert_title_is_not_rendered(&mut sim, &DisplayTemplate::AnalogClock);
}

/// The rows the title chip used to occupy: it sat at `y = 2 * DESKMATE_GRID`
/// (16) and stood `DESKMATE_CHIP_HEIGHT` (32) tall. The re-centered hero
/// starts at y = 64, so nothing may light this band.
const OLD_CHIP_BAND: std::ops::Range<usize> = 16..48;

#[test]
fn digital_clock_leaves_the_old_chip_band_dark() {
    let mut sim = Simulator::new().expect("simulator");
    let pixels = render(
        &mut sim,
        &DisplayTemplate::DigitalClock,
        "Desk",
        SimOrientation::Landscape,
    );
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
