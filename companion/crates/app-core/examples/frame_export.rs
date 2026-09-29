//! Dev-only frame exporter for the Deskmate landing page.
//!
//! Renders the card faces the marketing site's interactive panel replays, through
//! `preview_card_scene` -- the same scene builder the server's preview calls -- and
//! the firmware's own decoder and interpreter compiled into `lvgl-sim`. Frames are
//! PNGs on the logical 448x368 canvas.
//!
//! Run from `companion/`:
//! `cargo run -p app-core --example frame_export -- <out_dir>`
//!
//! `Simulator` allows one instance per process (`SimError::AlreadyClaimed`), so every
//! frame is rendered sequentially from a single simulator.
//!
//! What these frames prove: that this is what the scene renderer draws. They are not
//! photographs of a panel, and the site must not present them as such.

use std::fs;
use std::path::PathBuf;

use app_core::{
    AppConfig, CardAlert, CardField, CardFieldValue, CardSettings, DisplayTemplate, RefreshPolicy,
    WidgetTapAction,
};
use chrono::Utc;
use lvgl_sim::scene::{SceneRenderRequest, SceneTimer};
use lvgl_sim::{SimOrientation, Simulator};

/// The countdown window the site replays, in seconds. One frame per second is the
/// cadence the device itself ticks a pushed timer at.
const WINDOW_SECONDS: u32 = 60;
const DURATION_SECONDS: u32 = 25 * 60;

/// The countdown subtracts `second * 1000` from the total, so the window must fit
/// inside the duration. Asserted at compile time rather than left to underflow.
const _: () = assert!(WINDOW_SECONDS <= DURATION_SECONDS);

fn text(key: &str, value: &str) -> CardField {
    CardField {
        key: key.into(),
        value: CardFieldValue::Text {
            value: value.into(),
        },
    }
}

fn integer(key: &str, value: i64) -> CardField {
    CardField {
        key: key.into(),
        value: CardFieldValue::Integer { value },
    }
}

fn boolean(key: &str, value: bool) -> CardField {
    CardField {
        key: key.into(),
        value: CardFieldValue::Boolean { value },
    }
}

/// `AppConfig::default()` already carries a single clock card with id `clock`.
/// The focus card is added here so the pack has a timer face to render.
fn demo_config() -> AppConfig {
    let mut config = AppConfig::default();
    config.cards.push(CardSettings::Pomodoro {
        id: "focus".into(),
        label: "Focus".into(),
        duration_seconds: DURATION_SECONDS,
        template: DisplayTemplate::ProgressRing,
        tap_action: WidgetTapAction::StartPause,
        refresh: RefreshPolicy::DeviceLocal,
        alert: CardAlert::None,
        dwell_seconds: None,
    });
    config
}

/// The scene builders read a card's face from its published FIELDS, not from its
/// configuration: `build_progress_ring_scene` gates the countdown on
/// `duration_seconds >= 1` and bakes the literal "00:00" into the scene when that
/// is false (`crates/app-core/src/scene_build.rs:711,766`). Passing an empty slice
/// therefore yields a face that never counts, while the ring's unconditional
/// `timer.permille` binding keeps moving -- which looks like it works and is not.
fn render(
    sim: &mut Simulator,
    config: &AppConfig,
    card_id: &str,
    fields: &[CardField],
    timer: Option<SceneTimer>,
) -> Vec<u8> {
    let now = Utc::now();
    let scene = app_core::preview_card_scene(config, card_id, fields)
        .unwrap_or_else(|reason| panic!("scene for {card_id:?} did not build: {reason}"));
    let utc_offset_minutes = app_core::utc_offset_minutes(&config.preferences.timezone, now)
        .unwrap_or_else(|reason| panic!("timezone did not resolve: {reason}"));
    sim.render_scene_png(&SceneRenderRequest {
        scene,
        assets: Vec::new(),
        utc_offset_minutes,
        now_unix_seconds: now.timestamp(),
        timer,
        // The site shows the upright view a person sees, at either mounting.
        orientation: SimOrientation::Landscape,
    })
    .unwrap_or_else(|error| panic!("frame for {card_id:?} did not render: {error}"))
}

fn main() {
    let out: PathBuf = std::env::args()
        .nth(1)
        .expect("usage: frame_export <out_dir>")
        .into();
    fs::create_dir_all(&out).expect("output directory");

    let config = demo_config();
    let mut sim = Simulator::new().expect("simulator");

    // Fields, not configuration, are what the scene builders read. `AppConfig::default()`
    // declares `show_seconds: true`, so the exported clock says so too.
    let clock_fields = [boolean("show_seconds", true)];
    fs::write(
        out.join("clock.png"),
        render(&mut sim, &config, "clock", &clock_fields, None),
    )
    .expect("write clock frame");

    // `duration_seconds` must be >= 1 or the ring scene bakes a literal "00:00".
    let focus_fields = [
        text("label", "Focus"),
        integer("duration_seconds", i64::from(DURATION_SECONDS)),
    ];

    let total_ms = DURATION_SECONDS * 1_000;
    let paused = SceneTimer {
        total_ms,
        remaining_ms: total_ms,
        running: false,
    };
    fs::write(
        out.join("focus-paused.png"),
        render(&mut sim, &config, "focus", &focus_fields, Some(paused)),
    )
    .expect("write paused frame");

    for second in 0..=WINDOW_SECONDS {
        let timer = SceneTimer {
            total_ms,
            remaining_ms: total_ms - second * 1_000,
            running: true,
        };
        let png = render(&mut sim, &config, "focus", &focus_fields, Some(timer));
        fs::write(out.join(format!("focus-{second:03}.png")), png).expect("write frame");
    }

    println!("wrote {} frames to {}", WINDOW_SECONDS + 3, out.display());
}
