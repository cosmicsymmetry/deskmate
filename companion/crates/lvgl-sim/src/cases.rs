//! The golden-frame case table: every firmware template x both mount
//! orientations x a field matrix chosen to exercise each template's
//! visually distinct states. `tests/golden.rs` renders every case and pins
//! the output to a committed PNG under `tests/golden/`; the physical
//! framebuffer diff (Task 10, `companion/crates/device/examples/framebuffer_diff.rs`)
//! reuses [`golden_cases`] so the same matrix is compared against real
//! hardware output. Lives under `src/` rather than `tests/` so both
//! consumers — the in-crate integration test and the other crate's example
//! — can reach it as `lvgl_sim::cases`; a `tests/` file is only visible to
//! `cargo test` within this crate.
//!
//! Field names below are pulled from the firmware's own registry,
//! `firmware/main/core/template_fields.c` (`template_fields_registry`) — do
//! not rename a field here without checking that file first, since a wrong
//! name silently falls back to the field's default and produces a
//! meaningless golden.

use crate::{RenderRequest, SimField, SimFieldValue, SimOrientation, SimTemplate};

/// The wire's absolute per-field text ceiling
/// (`PROTOCOL_MAX_FIELD_TEXT_LENGTH` in
/// `firmware/main/core/protocol_message.h`). Used for the row-list
/// truncation-boundary case; note this exceeds `row0_title`'s own registry
/// maximum of 96, so this case exercises LVGL's ellipsis truncation at the
/// wire ceiling rather than the field's schema-declared maximum.
const PROTOCOL_MAX_FIELD_TEXT_LENGTH: usize = 128;

fn text(name: &str, value: &str) -> SimField {
    SimField {
        name: name.to_string(),
        value: SimFieldValue::Text(value.to_string()),
    }
}

fn boolean(name: &str, value: bool) -> SimField {
    SimField {
        name: name.to_string(),
        value: SimFieldValue::Boolean(value),
    }
}

fn integer(name: &str, value: i64) -> SimField {
    SimField {
        name: name.to_string(),
        value: SimFieldValue::Integer(value),
    }
}

fn orientations() -> [(&'static str, SimOrientation); 2] {
    [
        ("landscape", SimOrientation::Landscape),
        ("flipped", SimOrientation::LandscapeFlipped),
    ]
}

/// Appends one case at both orientations. Name format:
/// `{template_slug}--{case_slug}--{orientation_slug}`.
fn case(
    cases: &mut Vec<(String, RenderRequest)>,
    template_slug: &str,
    case_slug: &str,
    template: SimTemplate,
    fields: &[SimField],
    now_unix_seconds: i64,
    utc_offset_minutes: i16,
) {
    for (orientation_slug, orientation) in orientations() {
        cases.push((
            format!("{template_slug}--{case_slug}--{orientation_slug}"),
            RenderRequest {
                template,
                fields: fields.to_vec(),
                utc_offset_minutes,
                now_unix_seconds,
                orientation,
            },
        ));
    }
}

// Fields: title, show_seconds, stale, error
// (firmware/main/core/template_fields.c: s_digital_clock_fields)
fn digital_clock_cases(cases: &mut Vec<(String, RenderRequest)>) {
    const NOW: i64 = 1_755_000_000; // 2025-08-12 12:00:00 UTC
    const OFFSET: i16 = 240; // -> 16:00 local

    case(
        cases,
        "digital-clock",
        "typical",
        SimTemplate::DigitalClock,
        &[text("title", "Desk"), boolean("show_seconds", true)],
        NOW,
        OFFSET,
    );
    case(
        cases,
        "digital-clock",
        "no-seconds",
        SimTemplate::DigitalClock,
        &[text("title", "Desk"), boolean("show_seconds", false)],
        NOW,
        OFFSET,
    );
    // Tabular-figure pair (Task 9). Both instants render an HH:MM with no
    // repeated digit shape in common, at the same font and the same box, so
    // the two goldens' lit column span must be identical — proportional
    // figures would shift the second frame. `tests/tabular.rs` asserts that
    // equality in pixels; the goldens pin the frames it asserts over.
    case(
        cases,
        "digital-clock",
        "tabular-1135",
        SimTemplate::DigitalClock,
        &[text("title", "Desk"), boolean("show_seconds", false)],
        TABULAR_1135,
        0,
    );
    case(
        cases,
        "digital-clock",
        "tabular-0000",
        SimTemplate::DigitalClock,
        &[text("title", "Desk"), boolean("show_seconds", false)],
        TABULAR_0000,
        0,
    );
}

/// 2025-08-13 11:35:00 UTC — the "widest-looking" of the tabular pair.
pub const TABULAR_1135: i64 = 1_755_084_900;
/// 2025-08-13 00:00:00 UTC — the "narrowest-looking" of the tabular pair.
pub const TABULAR_0000: i64 = 1_755_043_200;

// Fields: title, show_seconds, stale, error
// (firmware/main/core/template_fields.c: s_analog_clock_fields)
fn analog_clock_cases(cases: &mut Vec<(String, RenderRequest)>) {
    // Pinned instant from the task-5 brief verbatim. Both `typical` and
    // `no-seconds` use this SAME instant on purpose: together the two
    // goldens pin that the hour/minute hands do not depend on
    // `show_seconds` (the open 2026-08-11 no-seconds defect's deterministic
    // form) — only the second hand's visibility should differ between them.
    const PINNED_INSTANT: i64 = 1_755_081_480;
    const MIDNIGHT_INSTANT: i64 = 1_755_043_200; // 2025-08-13 00:00:00 UTC

    case(
        cases,
        "analog-clock",
        "typical",
        SimTemplate::AnalogClock,
        &[text("title", "Desk"), boolean("show_seconds", true)],
        PINNED_INSTANT,
        0,
    );
    case(
        cases,
        "analog-clock",
        "no-seconds",
        SimTemplate::AnalogClock,
        &[text("title", "Desk"), boolean("show_seconds", false)],
        PINNED_INSTANT,
        0,
    );
    case(
        cases,
        "analog-clock",
        "midnight",
        SimTemplate::AnalogClock,
        &[text("title", "Desk"), boolean("show_seconds", true)],
        MIDNIGHT_INSTANT,
        0,
    );
}

// Fields: label, duration_seconds (required), remaining_seconds (required),
// running (required), stale, error
// (firmware/main/core/template_fields.c: s_progress_ring_fields)
fn progress_ring_cases(cases: &mut Vec<(String, RenderRequest)>) {
    const NOW: i64 = 1_755_000_000;
    const DURATION_SECONDS: i64 = 1500;

    case(
        cases,
        "progress-ring",
        "running-mid-countdown",
        SimTemplate::ProgressRing,
        &[
            text("label", "Pomodoro"),
            integer("duration_seconds", DURATION_SECONDS),
            integer("remaining_seconds", 900),
            boolean("running", true),
        ],
        NOW,
        0,
    );
    case(
        cases,
        "progress-ring",
        "finished",
        SimTemplate::ProgressRing,
        &[
            text("label", "Pomodoro"),
            integer("duration_seconds", DURATION_SECONDS),
            integer("remaining_seconds", 0),
            boolean("running", false),
        ],
        NOW,
        0,
    );
    case(
        cases,
        "progress-ring",
        "never-started",
        SimTemplate::ProgressRing,
        &[
            text("label", "Pomodoro"),
            integer("duration_seconds", DURATION_SECONDS),
            integer("remaining_seconds", DURATION_SECONDS),
            boolean("running", false),
        ],
        NOW,
        0,
    );
}

// Fields: title, row0_title/row0_time .. row4_title/row4_time, stale, error
// (firmware/main/core/template_fields.c: s_row_list_fields)
fn row_list_cases(cases: &mut Vec<(String, RenderRequest)>) {
    const NOW: i64 = 1_755_000_000;
    let truncation_boundary_title: String = "Boundary-"
        .chars()
        .cycle()
        .take(PROTOCOL_MAX_FIELD_TEXT_LENGTH)
        .collect();

    case(
        cases,
        "row-list",
        "five-full-rows",
        SimTemplate::RowList,
        &[
            text("title", "Calendar"),
            text("row0_title", "Standup"),
            text("row0_time", "09:00"),
            text("row1_title", "Design Review"),
            text("row1_time", "10:30"),
            text("row2_title", "Lunch with Sam"),
            text("row2_time", "12:00"),
            text("row3_title", "1:1 with Manager"),
            text("row3_time", "14:00"),
            text("row4_title", "Sprint Planning"),
            text("row4_time", "16:00"),
        ],
        NOW,
        0,
    );
    case(
        cases,
        "row-list",
        "one-row",
        SimTemplate::RowList,
        &[
            text("title", "Calendar"),
            text("row0_title", "Standup"),
            text("row0_time", "09:00"),
        ],
        NOW,
        0,
    );
    case(
        cases,
        "row-list",
        "zero-rows",
        SimTemplate::RowList,
        &[text("title", "Calendar")],
        NOW,
        0,
    );
    case(
        cases,
        "row-list",
        "truncation-boundary",
        SimTemplate::RowList,
        &[
            text("title", "Calendar"),
            text("row0_title", &truncation_boundary_title),
            text("row0_time", "09:00"),
        ],
        NOW,
        0,
    );
    case(
        cases,
        "row-list",
        "unicode-row",
        SimTemplate::RowList,
        &[
            text("title", "Calendar"),
            text("row0_title", "Café — Zürich ☂"),
            text("row0_time", "09:00"),
        ],
        NOW,
        0,
    );
    case(
        cases,
        "row-list",
        "stale",
        SimTemplate::RowList,
        &[text("title", "Calendar"), boolean("stale", true)],
        NOW,
        0,
    );
    case(
        cases,
        "row-list",
        "error",
        SimTemplate::RowList,
        &[text("title", "Calendar"), text("error", "Sync failed")],
        NOW,
        0,
    );
}

// Fields: title, value, label, stale, error
// (firmware/main/core/template_fields.c: s_big_number_label_fields)
fn big_number_label_cases(cases: &mut Vec<(String, RenderRequest)>) {
    const NOW: i64 = 1_755_000_000;
    // `value`'s registry maximum is 16 chars.
    let maximal_value = "9".repeat(16);

    case(
        cases,
        "big-number-label",
        "typical",
        SimTemplate::BigNumberLabel,
        &[
            text("title", "Steps"),
            text("value", "8123"),
            text("label", "today"),
        ],
        NOW,
        0,
    );
    case(
        cases,
        "big-number-label",
        "empty-value",
        SimTemplate::BigNumberLabel,
        &[
            text("title", "Steps"),
            text("value", ""),
            text("label", "today"),
        ],
        NOW,
        0,
    );
    case(
        cases,
        "big-number-label",
        "maximal-length-value",
        SimTemplate::BigNumberLabel,
        &[
            text("title", "Distance"),
            text("value", &maximal_value),
            text("label", "km run"),
        ],
        NOW,
        0,
    );
    // A json-feed boolean lands in `value` as free-form text. The HERO and
    // DISPLAY tiers are digits-only subsets, so this case pins the step down
    // to BODY; `typical` above pins the numeric case staying at HERO.
    case(
        cases,
        "big-number-label",
        "alphabetic-value",
        SimTemplate::BigNumberLabel,
        &[
            text("title", "Deploy gate"),
            text("value", "yes"),
            text("label", "main branch"),
        ],
        NOW,
        0,
    );
    case(
        cases,
        "big-number-label",
        "empty-label",
        SimTemplate::BigNumberLabel,
        &[
            text("title", "Steps"),
            text("value", "8123"),
            text("label", ""),
        ],
        NOW,
        0,
    );
}

// Fields: title, icon, badge, value, label, unit, temperature_tenths,
// apparent_temperature_tenths, stale, error
// (firmware/main/core/template_fields.c: s_icon_badge_text_fields).
// `icon_badge_text_patch` (firmware/main/ui/templates/icon_badge_text.c)
// only reads title/badge/value/label/icon; unit and the temperature fields
// are declared so `unknown_field_count` stays zero on a weather push, but
// are not drawn, so they are omitted here.
//
// Icon names come from `weather_icon_from_name`
// (firmware/main/ui/templates/weather_icon.c): sun, moon, cloud, cloud-sun,
// cloud-moon, rain, drizzle, snow, storm, fog, unknown (11 total). Any
// unrecognized name, including "volcano", falls back to the hollow-ring
// unknown icon.
fn icon_badge_text_cases(cases: &mut Vec<(String, RenderRequest)>) {
    const NOW: i64 = 1_755_000_000;

    case(
        cases,
        "icon-badge-text",
        "icon-sun",
        SimTemplate::IconBadgeText,
        &[
            text("title", "Weather"),
            text("icon", "sun"),
            text("badge", "Now"),
            text("value", "72"),
            text("label", "Feels 74"),
        ],
        NOW,
        0,
    );
    case(
        cases,
        "icon-badge-text",
        "icon-storm",
        SimTemplate::IconBadgeText,
        &[
            text("title", "Weather"),
            text("icon", "storm"),
            text("badge", "Alert"),
            text("value", "61"),
            text("label", "Feels 59"),
        ],
        NOW,
        0,
    );
    case(
        cases,
        "icon-badge-text",
        "icon-unknown",
        SimTemplate::IconBadgeText,
        &[
            text("title", "Weather"),
            text("icon", "volcano"),
            text("badge", "Unknown"),
            text("value", "--"),
            text("label", "No data"),
        ],
        NOW,
        0,
    );
    case(
        cases,
        "icon-badge-text",
        "empty-badge",
        SimTemplate::IconBadgeText,
        &[
            text("title", "Weather"),
            text("icon", "cloud"),
            text("badge", ""),
            text("value", "65"),
            text("label", "Partly cloudy"),
        ],
        NOW,
        0,
    );
    case(
        cases,
        "icon-badge-text",
        "empty-value",
        SimTemplate::IconBadgeText,
        &[
            text("title", "Weather"),
            text("icon", "rain"),
            text("badge", "Now"),
            text("value", ""),
            text("label", "Heavy"),
        ],
        NOW,
        0,
    );
}

/// The full golden-frame case matrix: every template x both orientations x
/// the field rows from the task-5 brief's case table (spec §3.2.1). Every
/// case pins `now_unix_seconds`/`utc_offset_minutes` explicitly; none reads
/// real time.
pub fn golden_cases() -> Vec<(String, RenderRequest)> {
    let mut cases = Vec::new();
    digital_clock_cases(&mut cases);
    analog_clock_cases(&mut cases);
    progress_ring_cases(&mut cases);
    row_list_cases(&mut cases);
    big_number_label_cases(&mut cases);
    icon_badge_text_cases(&mut cases);
    cases
}
