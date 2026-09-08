//! Task 8 (plugin-manifest stage): pins `lvgl_sim::cases::plugin_scene_cases()`
//! -- the two curated plugins (`aqi`, `agenda`), each at four data states
//! (fresh, stale, error, empty/missing-data) -- to committed landscape PNGs
//! under `tests/golden/plugin-scene/`.
//!
//! A separate test and golden subdirectory from `tests/scene.rs`, for the
//! same reason that file's is separate from `tests/golden.rs`: these cases
//! render a `Scene` `plugin::compile_scene_with_assets` actually produced
//! from real on-disk manifests, not a hand-built one, and keeping the PNGs
//! in their own subdirectory means `tests/golden.rs`'s orphan check (which
//! lists only `tests/golden/*.png`, non-recursively) never sees them.

mod common;

use lvgl_sim::cases;
use protocol::AssetKind;

#[test]
fn plugin_scene_goldens_match() {
    common::assert_goldens(
        "plugin-scene",
        std::env::var("BLESS").is_ok(),
        Some("plugin scene golden mismatches"),
        "orphan golden with no matching case (run with BLESS=1 to delete)",
        cases::plugin_scene_cases(),
        |_, _| {},
        lvgl_sim::Simulator::render_scene_png,
    );
}

/// Every plugin x state combination is covered, named explicitly (not just
/// counted) so a state silently dropped from `plugin_case`'s state table
/// fails here by name rather than only shrinking a total.
#[test]
fn every_plugin_state_has_a_case() {
    let names: Vec<String> = cases::plugin_scene_cases()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    for plugin_name in ["aqi", "agenda"] {
        for state in ["fresh", "stale", "error", "empty"] {
            for orientation in ["landscape", "flipped"] {
                let expected = format!("plugin-{plugin_name}--{state}--{orientation}");
                assert!(
                    names.contains(&expected),
                    "no plugin scene case named {expected}"
                );
            }
        }
    }
    assert_eq!(names.len(), 16, "2 plugins x 4 states x 2 orientations");
}

#[test]
fn aqi_asset_keeps_its_icon_font_wire_kind() {
    let (_, request) = cases::plugin_scene_cases()
        .into_iter()
        .find(|(name, _)| name == "plugin-aqi--fresh--landscape")
        .expect("fresh landscape AQI case");
    assert_eq!(request.assets.len(), 1);
    assert_eq!(request.assets[0].kind, AssetKind::IconFont);
}
