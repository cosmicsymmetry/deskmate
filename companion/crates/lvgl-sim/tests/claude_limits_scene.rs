//! Pins the claude-limits curated plugin -- the Claude Code subscription
//! limits face -- at four data states, to committed landscape PNGs under
//! `tests/golden/claude-limits/`. Same harness shape as
//! `plugin_scene.rs`, in its own golden subdirectory for the same orphan-check
//! reason. Golden-only by design: see `cases::claude_limits_scene_cases`'s
//! doc for why these rows do not join the hardware framebuffer matrix.

mod common;

use lvgl_sim::cases;

#[test]
fn claude_limits_goldens_match() {
    common::assert_goldens(
        "claude-limits",
        std::env::var("BLESS").is_ok(),
        None,
        "orphan golden with no matching case (run with BLESS=1 to delete)",
        cases::claude_limits_scene_cases(),
        |_, _| {},
        lvgl_sim::Simulator::render_scene_png,
    );
}
