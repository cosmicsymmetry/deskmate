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
    AppConfig, CardAlert, CardSettings, DisplayTemplate, RefreshPolicy, WidgetTapAction,
};
use chrono::Utc;
use lvgl_sim::scene::{SceneRenderRequest, SceneTimer};
use lvgl_sim::{SimOrientation, Simulator};

/// The countdown window the site replays, in seconds. One frame per second is the
/// cadence the device itself ticks a pushed timer at.
const WINDOW_SECONDS: u32 = 60;
const DURATION_SECONDS: u32 = 25 * 60;

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

fn render(
    sim: &mut Simulator,
    config: &AppConfig,
    card_id: &str,
    timer: Option<SceneTimer>,
) -> Vec<u8> {
    let now = Utc::now();
    let scene = app_core::preview_card_scene(config, card_id, &[])
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

    fs::write(
        out.join("clock.png"),
        render(&mut sim, &config, "clock", None),
    )
    .expect("write clock frame");

    let total_ms = DURATION_SECONDS * 1_000;
    let paused = SceneTimer {
        total_ms,
        remaining_ms: total_ms,
        running: false,
    };
    fs::write(
        out.join("focus-paused.png"),
        render(&mut sim, &config, "focus", Some(paused)),
    )
    .expect("write paused frame");

    for second in 0..=WINDOW_SECONDS {
        let timer = SceneTimer {
            total_ms,
            remaining_ms: total_ms - second * 1_000,
            running: true,
        };
        let png = render(&mut sim, &config, "focus", Some(timer));
        fs::write(out.join(format!("focus-{second:03}.png")), png).expect("write frame");
    }

    println!("wrote {} frames to {}", WINDOW_SECONDS + 3, out.display());
}
