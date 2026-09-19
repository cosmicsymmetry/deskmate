use super::orientations;
use crate::scene::SceneRenderRequest;

/// 2026-05-13 08:34:56 UTC, which becomes Wednesday 12:34:56 at UTC+04:00.
pub const DATE_OVERFLOW_NOW_UNIX_SECONDS: i64 = 1_778_661_296;
/// Non-zero by construction: the date producer must apply this before
/// breaking the instant down into `Wed, May 13`.
pub const DATE_OVERFLOW_UTC_OFFSET_MINUTES: i16 = 240;
/// The exact LVGL BODY-font width of the produced date. Derived from the
/// shipped generated font in `tests/evidence_scene.rs`.
pub const DATE_OVERFLOW_BODY_WIDTH: i32 = 178;
/// `build_digital_clock_scene()`'s date content box.
pub const DATE_CONTENT_WIDTH: i32 = 176;
/// The local date text selected by the native `date` producer fixture.
/// Constructed from the chosen UTC instant and offset rather than written as
/// a test literal; the simulator independently evaluates the C producer.
pub fn date_overflow_text() -> String {
    use chrono::Datelike;

    let local_seconds =
        DATE_OVERFLOW_NOW_UNIX_SECONDS + i64::from(DATE_OVERFLOW_UTC_OFFSET_MINUTES) * 60;
    let local = chrono::DateTime::from_timestamp(local_seconds, 0)
        .expect("the date-overflow instant is representable")
        .naive_utc();
    format!(
        "{}, {} {}",
        local.format("%a"),
        local.format("%b"),
        local.day()
    )
}

/// A native `DigitalClock` scene whose real device-side `date` binding produces
/// a 178px BODY-font run inside the builder's 176px content box. Both the UTC
/// instant and the non-zero offset reach the C producer unchanged.
pub fn date_truncation_scene_cases() -> Vec<(String, SceneRenderRequest)> {
    let scene = app_core::build_digital_clock_scene(&app_core::ClockCard {
        revision: 1,
        show_seconds: false,
    });

    orientations()
        .into_iter()
        .map(|(orientation_slug, orientation)| {
            (
                format!("digital-clock--date-overflow--{orientation_slug}"),
                SceneRenderRequest {
                    scene: scene.clone(),
                    assets: Vec::new(),
                    utc_offset_minutes: DATE_OVERFLOW_UTC_OFFSET_MINUTES,
                    now_unix_seconds: DATE_OVERFLOW_NOW_UNIX_SECONDS,
                    timer: None,
                    orientation,
                },
            )
        })
        .collect()
}
