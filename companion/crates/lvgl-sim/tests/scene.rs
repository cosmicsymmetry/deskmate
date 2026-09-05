//! Task 8 (stage 2a): pins `lvgl_sim::cases::scene_cases()` — one scene per
//! `scene_node_kind_t` — to committed landscape PNGs under
//! `tests/golden/scene/`.
//!
//! A separate test and golden subdirectory from `tests/golden.rs`, for the same
//! reason `tests/asset_font.rs` is: these cases are `SceneRenderRequest`s
//! rendered through `Simulator::render_scene_png` — the firmware's scene
//! decoder and `ui/scene_view.c` interpreter — not `RenderRequest`s through
//! `template_view_show`. Keeping the PNGs in a subdirectory means
//! `tests/golden.rs`'s orphan check, which lists only `tests/golden/*.png`
//! non-recursively, never sees them.

mod common;

use lvgl_sim::cases;

#[test]
fn scene_goldens_match() {
    common::assert_goldens(
        "scene",
        std::env::var("BLESS").is_ok(),
        Some("scene golden mismatches"),
        "orphan golden with no matching case (run with BLESS=1 to delete)",
        cases::scene_cases(),
        |_, _| {},
        lvgl_sim::Simulator::render_scene_png,
    );
}

/// Every `scene_node_kind_t` is covered. Written as an explicit roll-call
/// rather than a count so that adding another node kind to the model fails
/// here, loudly, instead of leaving the new kind with no golden at all.
#[test]
fn every_node_kind_has_a_case() {
    let names: Vec<String> = cases::scene_cases()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    for kind in [
        "rect", "arc", "line", "text", "image", "glyph", "scale", "label", "rot-rect",
    ] {
        for orientation in ["landscape", "flipped"] {
            let expected = format!("scene-{kind}--{orientation}");
            assert!(
                names.contains(&expected),
                "no scene case named {expected}; every scene_node_kind_t needs \
                 one at both orientations"
            );
        }
    }
    assert_eq!(names.len(), 18, "9 node kinds x 2 orientations");
}
