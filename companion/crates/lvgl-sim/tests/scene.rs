//! Pins `lvgl_sim::cases::scene_cases()` — one scene per `scene_node_kind_t`
//! — to committed landscape PNGs under
//! `tests/golden/scene/`.
//!
//! A separate test and golden subdirectory from `tests/golden.rs`, for the same
//! reason `tests/asset_font.rs` is: these cases exercise one scene node kind
//! each, rather than a shipped card face, so they are pinned apart from the
//! face goldens `tests/golden.rs` owns. Keeping the PNGs in a subdirectory means
//! `tests/golden.rs`'s orphan check, which lists only `tests/golden/*.png`
//! non-recursively, never sees them.

mod common;

use lvgl_sim::{SimOrientation, cases};
use protocol::SceneNode;

#[test]
fn scene_goldens_match() {
    common::assert_goldens(
        "scene",
        std::env::var("BLESS").is_ok(),
        "scene golden mismatches",
        "orphan golden with no matching case (run with BLESS=1 to delete)",
        cases::scene_cases(),
    );
}

fn node_kind_slug(node: &SceneNode) -> &'static str {
    match node {
        SceneNode::Rect(_) => "rect",
        SceneNode::Arc(_) => "arc",
        SceneNode::Line(_) => "line",
        SceneNode::Text(_) => "text",
        SceneNode::Image(_) => "image",
        SceneNode::Glyph(_) => "glyph",
        SceneNode::Scale(_) => "scale",
        SceneNode::Label(_) => "label",
        SceneNode::RotRect(_) => "rot-rect",
    }
}

/// Exhaustively handles Rust node variants, checks each advertised kind is
/// present, and requires two correctly oriented rows per kind.
#[test]
fn every_node_kind_has_a_case() {
    let cases = cases::scene_cases();
    for kind in [
        "rect", "arc", "line", "text", "image", "glyph", "scale", "label", "rot-rect",
    ] {
        for (suffix, orientation) in [
            ("landscape", SimOrientation::Landscape),
            ("flipped", SimOrientation::LandscapeFlipped),
        ] {
            let expected = format!("scene-{kind}--{suffix}");
            let (_, request) = cases
                .iter()
                .find(|(name, _)| name == &expected)
                .unwrap_or_else(|| {
                    panic!(
                        "no scene case named {expected}; every scene_node_kind_t needs \
                 one at both orientations"
                    )
                });
            assert_eq!(request.orientation, orientation, "{expected}: orientation");
            assert!(
                request
                    .scene
                    .nodes
                    .iter()
                    .any(|node| node_kind_slug(node) == kind),
                "{expected}: no {kind} node"
            );
        }
    }
    assert_eq!(cases.len(), 18, "9 node kinds x 2 orientations");
}
