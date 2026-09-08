//! Pins `lvgl_sim::cases::asset_font_scene_cases()` — one 72px runtime
//! font `SceneText` — to a committed landscape PNG under
//! `tests/golden/asset-font/`.

mod common;

use lvgl_sim::cases;

#[test]
fn asset_font_golden_matches() {
    common::assert_goldens(
        "asset-font",
        std::env::var("BLESS").is_ok(),
        Some("asset-font golden mismatches"),
        "orphan golden with no matching case (run with BLESS=1 to delete)",
        cases::asset_font_scene_cases(),
        |_, _| {},
        lvgl_sim::Simulator::render_scene_png,
    );
}
