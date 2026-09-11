//! The settings window's preview chain, end to end minus the Tauri glue.
//!
//! `render_card_preview` builds `app_core::preview_card_scene(..)` and hands it
//! to this simulator. The dev browser harness CANNOT check this: it mocks
//! `render_card_preview` outright (`src/dev/mockPreview.ts`), so a broken Rust
//! preview path renders perfectly there. This test is the honest check.

use app_core::{AppConfig, CardAlert, CardSettings, RefreshPolicy, WidgetTapAction};
use lvgl_sim::scene::{SceneRenderRequest, SceneTimer};
use lvgl_sim::{SimOrientation, Simulator};

fn render(scene: protocol::Scene, timer: Option<SceneTimer>) -> Vec<u16> {
    let mut simulator = Simulator::new().expect("simulator");
    simulator
        .render_scene(&SceneRenderRequest {
            scene,
            assets: Vec::new(),
            utc_offset_minutes: 0,
            now_unix_seconds: 1_767_225_600,
            timer,
            orientation: SimOrientation::Landscape,
        })
        .expect("render the preview scene")
}

fn distinct_colours(pixels: &[u16]) -> usize {
    let mut seen = pixels.to_vec();
    seen.sort_unstable();
    seen.dedup();
    seen.len()
}

#[test]
fn a_clock_card_preview_renders_a_drawn_frame() {
    let config = AppConfig::default();
    let card_id = config.cards[0].id().to_owned();
    let scene = app_core::preview_card_scene(&config, &card_id, &[]).expect("clock previews");

    let pixels = render(scene, None);

    assert_eq!(pixels.len(), 448 * 368, "the preview is a full canvas");
    assert!(
        distinct_colours(&pixels) > 1,
        "a blank frame means the preview chain drew nothing"
    );
}

#[test]
fn a_pomodoro_card_preview_renders_a_drawn_frame() {
    let mut config = AppConfig::default();
    config.cards.push(CardSettings::Pomodoro {
        id: "focus".into(),
        label: "Focus".into(),
        duration_seconds: 1_500,
        template: app_core::DisplayTemplate::ProgressRing,
        tap_action: WidgetTapAction::StartPause,
        refresh: RefreshPolicy::DeviceLocal,
        alert: CardAlert::None,
        dwell_seconds: None,
    });
    let scene = app_core::preview_card_scene(&config, "focus", &[]).expect("pomodoro previews");

    let pixels = render(
        scene,
        Some(SceneTimer {
            total_ms: 1_500_000,
            remaining_ms: 300_000,
            running: false,
        }),
    );

    assert!(
        distinct_colours(&pixels) > 1,
        "a blank frame means the preview chain drew nothing"
    );
}
