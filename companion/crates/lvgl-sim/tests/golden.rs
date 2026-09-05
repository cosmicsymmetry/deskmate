mod common;

use lvgl_sim::cases;

#[test]
fn golden_frames_match() {
    common::assert_goldens(
        "",
        std::env::var("BLESS").is_ok(),
        Some("golden mismatches"),
        "orphan golden with no matching case (run with BLESS=1 to delete)",
        cases::golden_cases(),
        |_, _| {},
        lvgl_sim::Simulator::render_png,
    );
}
