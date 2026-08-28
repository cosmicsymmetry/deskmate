//! The golden-frame case table: every firmware template x both mount
//! orientations x a field matrix chosen to exercise each template's
//! visually distinct states. `tests/golden.rs` renders every case and pins
//! the output to a committed PNG under `tests/golden/`. These rows now drive
//! the reference-only C oracle; the physical framebuffer diff drives both
//! [`scene_cases`] and [`face_scene_cases`] because shipping firmware no
//! longer has a template renderer. This lives under `src/` rather than
//! `tests/` so integration tests and hardware examples can reach it as
//! `lvgl_sim::cases`.
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

/// One host-owned data state used by the scene/C-template parity gate.
///
/// These fixtures live beside the simulator's other template field fixtures
/// because their C half must be expressed as real `stale`/`error` fields and
/// rendered through `template_view_show()`. They are intentionally not added
/// to [`golden_cases`]: the byte-exact scene parity gate owns this 24-row
/// matrix, while the existing row-list goldens already pin the C footer's
/// standalone appearance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateFooterFixture {
    pub slug: &'static str,
    pub stale: bool,
    pub error: Option<&'static str>,
}

impl StateFooterFixture {
    /// The real template fields that make `template_view.c` apply this state.
    pub fn fields(self) -> Vec<SimField> {
        vec![
            boolean("stale", self.stale),
            text("error", self.error.unwrap_or("")),
        ]
    }
}

/// The two non-OK shared-footer states, in the parity table's stable order.
pub const STATE_FOOTER_FIXTURES: [StateFooterFixture; 2] = [
    StateFooterFixture {
        slug: "stale",
        stale: true,
        error: None,
    },
    StateFooterFixture {
        slug: "error",
        // Deliberately true as well: every error parity row proves that the
        // C `template_view.c` path and the scene helper both give a non-empty
        // error precedence over stale.
        stale: true,
        error: Some("Sync failed"),
    },
];

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

    // A running ring mid-countdown is the ONLY pixel coverage of the running
    // arc-indicator hue: `running` drives the arc indicator and status text
    // colours (`progress_ring.c`, `palette.hue` vs
    // `DESKMATE_COLOR_TERTIARY`/`_PRIMARY`), and at a zero-length arc the
    // indicator is not drawn at all. So this case has to stay -- but it can
    // only be compared against a deterministic renderer.
    //
    // `current_remaining_ms` returns the pinned `remaining_ms` verbatim when the
    // ring is stopped and derives it from `lv_tick_get()` when running. The
    // simulator's fake tick is fixed (an 840ms anchor offset, see
    // `csrc/sim_shim.c`), so this golden is stable; real hardware's
    // push-to-capture latency is not, and the label's `(remaining_ms + 999) /
    // 1000` ceiling flips a whole second the moment that latency crosses 1000ms.
    // No running value avoids that: the label flips every second by
    // construction, whatever the duration.
    //
    // It is therefore golden-only, and `framebuffer_diff.rs` excludes it from
    // the physical comparison by name. See `exclusion_reason` there.
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
    // The hardware-comparable half of the case above: identical geometry at a
    // partial arc (900/1500), which no other progress-ring case covers, with the
    // ring stopped so both sides render the pinned value and the frame is stable.
    case(
        cases,
        "progress-ring",
        "paused-mid-countdown",
        SimTemplate::ProgressRing,
        &[
            text("label", "Pomodoro"),
            integer("duration_seconds", DURATION_SECONDS),
            integer("remaining_seconds", 900),
            boolean("running", false),
        ],
        NOW,
        0,
    );
    // The only running ring that hardware can be compared on, and so the only
    // on-device coverage of the running palette -- here the status text ("Done"
    // in `palette.hue`), since a zero-length arc draws no indicator.
    //
    // It is deterministic because `current_remaining_ms` clamps to 0 as soon as
    // `elapsed >= remaining_ms`, and with `remaining_ms` of 0 that holds for any
    // elapsed at all. No other running value has that property.
    case(
        cases,
        "progress-ring",
        "running-at-zero",
        SimTemplate::ProgressRing,
        &[
            text("label", "Pomodoro"),
            integer("duration_seconds", DURATION_SECONDS),
            integer("remaining_seconds", 0),
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

fn field_text<'a>(request: &'a RenderRequest, name: &str, default: &'a str) -> &'a str {
    request
        .fields
        .iter()
        .find_map(|field| {
            (field.name == name)
                .then_some(&field.value)
                .and_then(|value| {
                    if let SimFieldValue::Text(value) = value {
                        Some(value.as_str())
                    } else {
                        None
                    }
                })
        })
        .unwrap_or(default)
}

fn field_integer(request: &RenderRequest, name: &str, default: i64) -> i64 {
    request
        .fields
        .iter()
        .find_map(|field| {
            (field.name == name)
                .then_some(&field.value)
                .and_then(|value| {
                    if let SimFieldValue::Integer(value) = value {
                        Some(*value)
                    } else {
                        None
                    }
                })
        })
        .unwrap_or(default)
}

fn field_boolean(request: &RenderRequest, name: &str, default: bool) -> bool {
    request
        .fields
        .iter()
        .find_map(|field| {
            (field.name == name)
                .then_some(&field.value)
                .and_then(|value| {
                    if let SimFieldValue::Boolean(value) = value {
                        Some(*value)
                    } else {
                        None
                    }
                })
        })
        .unwrap_or(default)
}

/// The six shipping-face rows as device-pushable scenes.
///
/// This is the hardware-facing half of the scene parity matrix, promoted next
/// to [`scene_cases`] so examples do not depend on an integration test's
/// private `cases()`. The row names deliberately remain identical to
/// [`golden_cases`]: `framebuffer_diff`'s two historical exclusions are keyed
/// by those names, and changing the prefix would silently make both branches
/// unreachable again.
#[allow(clippy::too_many_lines)] // one explicit adapter arm per retired face
pub fn face_scene_cases() -> Vec<(String, SceneRenderRequest)> {
    use app_core::scene_build::{
        AnalogClockCard, BakedFontMetrics, BigNumberCard, ClockCard, IconBadgeCard,
        ProgressRingCard, RowListCard, SceneDataState, build_analog_clock_scene,
        build_big_number_label_scene, build_digital_clock_scene, build_icon_badge_text_scene,
        build_progress_ring_scene, build_row_list_scene, with_scene_data_state,
    };

    golden_cases()
        .into_iter()
        .map(|(name, request)| {
            let metrics = &BakedFontMetrics::SHIPPED;
            let scene = match request.template {
                SimTemplate::DigitalClock => {
                    let local_seconds =
                        request.now_unix_seconds + i64::from(request.utc_offset_minutes) * 60;
                    let local_now = chrono::DateTime::from_timestamp(local_seconds, 0)
                        .expect("golden clock instant is representable")
                        .naive_utc();
                    build_digital_clock_scene(
                        &ClockCard {
                            revision: 1,
                            show_seconds: field_boolean(&request, "show_seconds", true),
                            local_now,
                        },
                        metrics,
                    )
                }
                SimTemplate::AnalogClock => build_analog_clock_scene(
                    &AnalogClockCard {
                        revision: 1,
                        show_seconds: field_boolean(&request, "show_seconds", true),
                    },
                    metrics,
                ),
                SimTemplate::ProgressRing => build_progress_ring_scene(
                    &ProgressRingCard {
                        revision: 1,
                        label: field_text(&request, "label", "Pomodoro"),
                        duration_seconds: field_integer(&request, "duration_seconds", 0),
                    },
                    metrics,
                ),
                SimTemplate::RowList => build_row_list_scene(
                    &RowListCard {
                        revision: 1,
                        title: field_text(&request, "title", "Calendar"),
                        row0_title: field_text(&request, "row0_title", ""),
                        row0_time: field_text(&request, "row0_time", ""),
                        row1_title: field_text(&request, "row1_title", ""),
                        row1_time: field_text(&request, "row1_time", ""),
                        row2_title: field_text(&request, "row2_title", ""),
                        row2_time: field_text(&request, "row2_time", ""),
                        row3_title: field_text(&request, "row3_title", ""),
                        row3_time: field_text(&request, "row3_time", ""),
                        row4_title: field_text(&request, "row4_title", ""),
                        row4_time: field_text(&request, "row4_time", ""),
                    },
                    metrics,
                ),
                SimTemplate::BigNumberLabel => build_big_number_label_scene(
                    &BigNumberCard {
                        revision: 1,
                        title: field_text(&request, "title", ""),
                        value: field_text(&request, "value", "--"),
                        label: field_text(&request, "label", ""),
                    },
                    metrics,
                ),
                SimTemplate::IconBadgeText => build_icon_badge_text_scene(
                    &IconBadgeCard {
                        revision: 1,
                        title: field_text(&request, "title", ""),
                        icon: field_text(&request, "icon", "unknown"),
                        badge: field_text(&request, "badge", ""),
                        value: field_text(&request, "value", "--"),
                        label: field_text(&request, "label", ""),
                    },
                    app_core::scene_build::SHIPPED_SCENE_SURFACE_COLOR,
                    metrics,
                ),
            };
            let scene = with_scene_data_state(
                scene,
                SceneDataState {
                    stale: field_boolean(&request, "stale", false),
                    error: match field_text(&request, "error", "") {
                        "" => None,
                        error => Some(error),
                    },
                },
                metrics,
            );
            let timer = (request.template == SimTemplate::ProgressRing).then(|| {
                let to_milliseconds = |seconds: i64| {
                    u32::try_from(seconds * 1_000)
                        .expect("golden progress-ring fixture is inside u32")
                };
                SceneTimer {
                    total_ms: to_milliseconds(field_integer(&request, "duration_seconds", 0)),
                    remaining_ms: to_milliseconds(field_integer(&request, "remaining_seconds", 0)),
                    running: field_boolean(&request, "running", false),
                }
            });
            (
                name,
                SceneRenderRequest {
                    scene,
                    assets: Vec::new(),
                    utc_offset_minutes: request.utc_offset_minutes,
                    now_unix_seconds: request.now_unix_seconds,
                    timer,
                    fields: Vec::new(),
                    orientation: request.orientation,
                },
            )
        })
        .collect()
}

/// Task 12: one case rendering a runtime font asset, deliberately not a
/// `RenderRequest`/`SimTemplate` case. It exercises a wholly different
/// path than every case above — `font_registry_acquire` against a digest
/// registered in `csrc/sim_shim.c`'s RAM-backed asset store, not
/// `template_view_show` against a baked `deskmate_font_*` face — so it
/// requires `AssetBegin`/`AssetChunk`/`AssetCommit` provisioning on a device,
/// a whole transfer protocol out of scope for this simulator-only parity
/// check. Rendered by
/// `Simulator::render_asset_font_png` (`src/assets.rs`) and pinned by
/// `tests/asset_font.rs` against `tests/golden/asset-font/` — a
/// subdirectory, not `tests/golden/` directly, so `tests/golden.rs`'s
/// orphan-PNG check (which only lists `tests/golden/*.png`, non-recursive)
/// never sees these files and does not need to know about this case table.
pub struct AssetFontCase {
    pub digest: [u8; 32],
    pub ttf_bytes: &'static [u8],
    pub pixel_size: i32,
    pub text: String,
    pub orientation: SimOrientation,
}

/// `asset-font--72px-digits`: `12:34` at 72px, deliberately none of the
/// four baked sizes (18/28/56/96 — the reference oracle's
/// `template_internal.h` `DESKMATE_FONT_*`), so nothing about this render
/// could accidentally be satisfied by a baked face instead of the runtime
/// one. Both orientations, matching every other case in this file.
pub fn asset_font_cases() -> Vec<(String, AssetFontCase)> {
    let mut cases = Vec::new();
    for (orientation_slug, orientation) in orientations() {
        cases.push((
            format!("asset-font--72px-digits--{orientation_slug}"),
            AssetFontCase {
                digest: crate::assets::INTER_SUBSET_SHA256,
                ttf_bytes: crate::assets::INTER_SUBSET_TTF,
                pixel_size: 72,
                text: "12:34".to_string(),
                orientation,
            },
        ));
    }
    cases
}

// ---------------------------------------------------------------------------
// Task 8 (stage 2a): scene cases — one per `scene_node_kind_t`, at both
// orientations.
// ---------------------------------------------------------------------------

use protocol::{
    SCENE_CANVAS_WIDTH, Scene, SceneAlign, SceneArc, SceneClipRect, SceneFont, SceneFontTier,
    SceneGlyph, SceneImage, SceneLabel, SceneLabelAnchor, SceneLine, SceneNode, SceneRect,
    SceneRotRect, SceneScale, SceneText, SceneValue,
};

use crate::scene::{SceneAsset, SceneRenderRequest, SceneTimer};

/// `template_internal.h`'s palette, so a scene golden reads as a plausible
/// Deskmate frame rather than a test pattern.
const CANVAS: u32 = 0x0000_0000;
const PRIMARY: u32 = 0x00f5_f5f7;
const TERTIARY: u32 = 0x005c_5c66;
const ACCENT: u32 = 0x00ff_8f2e;
/// Three off-palette hues, used only where a case needs several strokes told
/// apart at a glance.
const BLUE: u32 = 0x003b_6cff;
const GREEN: u32 = 0x002e_cc71;
const PINK: u32 = 0x00ff_2e6c;

/// 2025-08-12 12:00:00 UTC at +240 minutes — 16:00 local, the instant the
/// template cases already use, so a scene clock and a `DigitalClock` golden
/// can be read side by side.
const SCENE_NOW: i64 = 1_755_000_000;
const SCENE_OFFSET: i16 = 240;

/// The synthetic image asset's digest.
///
/// Every other digest in this crate is the SHA-256 of committed bytes
/// ([`crate::assets::INTER_SUBSET_SHA256`]). This one cannot be: the bytes are
/// generated by [`quadrant_image_pixels`] at render time, so there is no file
/// to hash and nothing that could silently replace it. It is an arbitrary
/// written-down constant whose only job is to be the name
/// `scene_view.c`'s resolver looks up, and it is deliberately not all-equal
/// bytes so it cannot be confused with the `[0xAB; 32]` a "digest nothing
/// registered" test uses.
pub const SCENE_IMAGE_DIGEST: [u8; 32] = [
    0x5c, 0xe4, 0x1a, 0x90, 0x2d, 0x77, 0xb3, 0x08, 0x5c, 0xe4, 0x1a, 0x90, 0x2d, 0x77, 0xb3, 0x08,
    0x5c, 0xe4, 0x1a, 0x90, 0x2d, 0x77, 0xb3, 0x08, 0x5c, 0xe4, 0x1a, 0x90, 0x2d, 0x77, 0xb3, 0x08,
];

/// The synthetic image's side, in pixels. Square, and the same size the
/// `image` nodes below declare, so nothing is scaled — except the one node
/// that deliberately declares a smaller box to show the documented clipping.
const SCENE_IMAGE_SIDE: u32 = 64;

/// A 64x64 RGB565 test image: a 4px white frame around four coloured
/// quadrants. Asymmetric on both axes on purpose — a flipped golden of a
/// symmetric image would prove nothing about orientation, and a solid one
/// would prove nothing about placement.
fn quadrant_image_pixels() -> Vec<u16> {
    const WHITE: u16 = 0xffff;
    const RED: u16 = 0xf800;
    const GREEN565: u16 = 0x07e0;
    const BLUE565: u16 = 0x001f;
    const YELLOW: u16 = 0xffe0;
    const FRAME: u32 = 4;

    let side = SCENE_IMAGE_SIDE;
    let half = side / 2;
    let mut pixels = Vec::with_capacity((side * side) as usize);
    for y in 0..side {
        for x in 0..side {
            let on_frame = x < FRAME || y < FRAME || x >= side - FRAME || y >= side - FRAME;
            pixels.push(if on_frame {
                WHITE
            } else {
                match (x < half, y < half) {
                    (true, true) => RED,
                    (false, true) => GREEN565,
                    (true, false) => BLUE565,
                    (false, false) => YELLOW,
                }
            });
        }
    }
    pixels
}

/// The synthetic image asset, as a scene request lists it.
fn image_asset() -> SceneAsset {
    SceneAsset::Image {
        digest: SCENE_IMAGE_DIGEST,
        width: SCENE_IMAGE_SIDE,
        height: SCENE_IMAGE_SIDE,
        pixels: quadrant_image_pixels(),
    }
}

/// The runtime font asset, reused from the Task 12 golden rather than
/// vendored twice. Its subset is digits, colon, space and `A`-`Z` (see
/// `crate::assets`), which is what the glyph case's codepoints are chosen
/// from.
fn font_asset() -> SceneAsset {
    SceneAsset::Font {
        digest: crate::assets::INTER_SUBSET_SHA256,
        bytes: crate::assets::INTER_SUBSET_TTF,
    }
}

fn literal(text: &str) -> SceneValue {
    SceneValue::Literal(text.to_string())
}

fn binding(text: &str) -> SceneValue {
    SceneValue::Binding(text.to_string())
}

/// A hairline across the full canvas at `y`, drawn to show where a
/// baseline-anchored node's baseline actually is. `scene_view.c`'s
/// `baseline_box_top()` is the highest-risk arithmetic in the interpreter and
/// an error in it is invisible to every host test — so the text and glyph
/// cases draw their baselines and the type is expected to sit *on* the rule.
fn baseline_rule(y: i32) -> SceneNode {
    SceneNode::Line(SceneLine {
        xs: vec![0, SCENE_CANVAS_WIDTH],
        ys: vec![y, y],
        width: 1,
        color: BLUE,
        ..SceneLine::default()
    })
}

/// Appends one scene case at both orientations, mirroring [`case`]'s naming:
/// `scene-{kind_slug}--{orientation_slug}`.
fn scene_case(
    cases: &mut Vec<(String, SceneRenderRequest)>,
    kind_slug: &str,
    nodes: &[SceneNode],
    assets: &[SceneAsset],
    timer: Option<SceneTimer>,
    fields: &[(String, String)],
) {
    for (orientation_slug, orientation) in orientations() {
        cases.push((
            format!("scene-{kind_slug}--{orientation_slug}"),
            SceneRenderRequest {
                scene: Scene {
                    revision: 1,
                    background: CANVAS,
                    nodes: nodes.to_vec(),
                },
                assets: assets.to_vec(),
                utc_offset_minutes: SCENE_OFFSET,
                now_unix_seconds: SCENE_NOW,
                timer,
                fields: fields.to_vec(),
                orientation,
            },
        ));
    }
}

/// `rect`: corner radius at three settings (square, rounded, a full circle
/// where the radius reaches half the side) plus a translucent overlap cropped
/// by an absolute clip rectangle. The latter shows both that `opacity` reaches
/// `bg_opa` and that clipping is performed by a parent rather than by changing
/// the rectangle geometry.
fn scene_rect_nodes() -> Vec<SceneNode> {
    vec![
        SceneNode::Rect(SceneRect {
            x: 32,
            y: 48,
            w: 112,
            h: 112,
            radius: 0,
            fill: BLUE,
            opacity: 255,
            clip: None,
        }),
        SceneNode::Rect(SceneRect {
            x: 168,
            y: 48,
            w: 112,
            h: 112,
            radius: 24,
            fill: ACCENT,
            opacity: 255,
            clip: None,
        }),
        SceneNode::Rect(SceneRect {
            x: 304,
            y: 48,
            w: 112,
            h: 112,
            // Half the side: LVGL draws this as a circle, which is how the
            // templates make their pills and dots.
            radius: 56,
            fill: GREEN,
            opacity: 255,
            clip: None,
        }),
        SceneNode::Rect(SceneRect {
            x: 32,
            y: 224,
            w: 384,
            h: 56,
            radius: 28,
            fill: PRIMARY,
            opacity: 255,
            clip: None,
        }),
        SceneNode::Rect(SceneRect {
            x: 96,
            y: 252,
            w: 256,
            h: 56,
            radius: 0,
            fill: PINK,
            // Overlaps the pill above at just under half opacity, so the
            // blend against two different backgrounds is visible in one node.
            opacity: 120,
            clip: Some(SceneClipRect {
                x: 160,
                y: 252,
                w: 128,
                h: 56,
            }),
        }),
    ]
}

/// `arc`: the four things an arc node can be asked to do, at four radii so
/// they can be told apart — a full turn (spelled as a nonzero multiple of
/// 360, not `start == end`), a bound sweep scaled by `timer.pct`, a plain
/// quadrant that pins the clockwise direction, and a sweep that crosses 0.
fn scene_arc_nodes() -> Vec<SceneNode> {
    vec![
        // The track: twelve o'clock all the way round. `start == end` would
        // be a degenerate empty arc; 270 -> 630 is the whole circle.
        SceneNode::Arc(SceneArc {
            cx: 224,
            cy: 176,
            r: 128,
            start_deg: 270,
            end_deg: 630,
            width: 18,
            color: TERTIARY,
            running_color: None,
            opacity: 0x33,
            rounded: false,
            end_binding: String::new(),
        }),
        // The indicator over it: the same declared geometry, scaled to 35% by
        // the bound timer, so it should sweep 126 degrees clockwise from
        // twelve — ending a little past four o'clock — with rounded caps.
        SceneNode::Arc(SceneArc {
            cx: 224,
            cy: 176,
            r: 128,
            start_deg: 270,
            end_deg: 630,
            width: 18,
            color: ACCENT,
            running_color: None,
            opacity: u8::MAX,
            rounded: true,
            end_binding: "timer.pct".to_string(),
        }),
        // 0 degrees is three o'clock and angles increase clockwise, so this
        // is the bottom-right quadrant and nothing else.
        SceneNode::Arc(SceneArc {
            cx: 224,
            cy: 176,
            r: 80,
            start_deg: 0,
            end_deg: 90,
            width: 12,
            color: BLUE,
            running_color: None,
            opacity: u8::MAX,
            rounded: false,
            end_binding: String::new(),
        }),
        // Crosses zero: 330 -> 30 is a 60 degree sweep through three
        // o'clock, not a 300 degree one the other way.
        SceneNode::Arc(SceneArc {
            cx: 224,
            cy: 176,
            r: 44,
            start_deg: 330,
            end_deg: 30,
            width: 10,
            color: GREEN,
            running_color: None,
            opacity: u8::MAX,
            rounded: false,
            end_binding: String::new(),
        }),
    ]
}

/// `line`: a thick stroke whose rounded caps are the point (every `lv_line` in
/// the firmware is rounded, so scene lines are too), a hairline, a zigzag,
/// and one at the `SCENE_MAX_LINE_POINTS` ceiling of 8.
fn scene_line_nodes() -> Vec<SceneNode> {
    vec![
        SceneNode::Line(SceneLine {
            xs: vec![64, 384],
            ys: vec![80, 80],
            width: 24,
            color: ACCENT,
            ..SceneLine::default()
        }),
        SceneNode::Line(SceneLine {
            xs: vec![32, 416],
            ys: vec![136, 176],
            width: 2,
            color: BLUE,
            ..SceneLine::default()
        }),
        SceneNode::Line(SceneLine {
            xs: vec![32, 112, 192, 272, 352, 416],
            ys: vec![296, 216, 296, 216, 296, 256],
            width: 6,
            color: GREEN,
            ..SceneLine::default()
        }),
        SceneNode::Line(SceneLine {
            xs: vec![16, 80, 144, 208, 272, 336, 400, 432],
            ys: vec![352, 336, 352, 336, 352, 336, 352, 344],
            width: 4,
            color: PRIMARY,
            ..SceneLine::default()
        }),
    ]
}

/// `text`: every tier the node can name, every alignment, both long modes,
/// a `time:` binding, a `field.` binding that resolves and one that does not
/// — each on a drawn baseline rule.
///
/// The `HERO`/`DISPLAY` faces are digits-only subsets (spec §5.2), so the
/// only string set in one here is a clock reading.
fn scene_text_nodes() -> Vec<SceneNode> {
    vec![
        baseline_rule(140),
        baseline_rule(210),
        baseline_rule(262),
        baseline_rule(314),
        SceneNode::Text(SceneText {
            x: 0,
            baseline_y: 140,
            w: SCENE_CANVAS_WIDTH,
            align: SceneAlign::Center,
            font: SceneFont::Baked(SceneFontTier::Hero),
            color: PRIMARY,
            running_color: None,
            value: binding("time:HH:mm"),
            ellipsize: false,
        }),
        SceneNode::Text(SceneText {
            x: 24,
            baseline_y: 210,
            w: 200,
            align: SceneAlign::Left,
            font: SceneFont::Baked(SceneFontTier::Caption),
            color: TERTIARY,
            running_color: None,
            value: literal("LEFT CAPTION"),
            ellipsize: false,
        }),
        // Right-aligned in a box ending at x = 424, so the type should end
        // there rather than starting at x = 224.
        SceneNode::Text(SceneText {
            x: 224,
            baseline_y: 210,
            w: 200,
            align: SceneAlign::Right,
            font: SceneFont::Baked(SceneFontTier::Body),
            color: PRIMARY,
            running_color: None,
            value: literal("RIGHT"),
            ellipsize: false,
        }),
        SceneNode::Text(SceneText {
            x: 24,
            baseline_y: 262,
            w: 240,
            align: SceneAlign::Left,
            font: SceneFont::Baked(SceneFontTier::Body),
            color: PRIMARY,
            running_color: None,
            value: literal("Overlong, must ellipsize"),
            ellipsize: true,
        }),
        // The same overlong run without ellipsize: LV_LABEL_LONG_MODE_CLIP,
        // which must clip at the box edge on ONE line, never wrap.
        SceneNode::Text(SceneText {
            x: 24,
            baseline_y: 314,
            w: 240,
            align: SceneAlign::Left,
            font: SceneFont::Baked(SceneFontTier::Body),
            color: ACCENT,
            running_color: None,
            value: literal("Overlong, must clip"),
            ellipsize: false,
        }),
        SceneNode::Text(SceneText {
            x: 288,
            baseline_y: 262,
            w: 136,
            align: SceneAlign::Right,
            font: SceneFont::Baked(SceneFontTier::Caption),
            color: GREEN,
            running_color: None,
            value: binding("field.status"),
            ellipsize: false,
        }),
        // No provider has reported `absent`, so this renders the device's
        // "--" placeholder rather than nothing and rather than failing.
        SceneNode::Text(SceneText {
            x: 288,
            baseline_y: 314,
            w: 136,
            align: SceneAlign::Right,
            font: SceneFont::Baked(SceneFontTier::Caption),
            color: PINK,
            running_color: None,
            value: binding("field.absent"),
            ellipsize: false,
        }),
    ]
}

/// `image`: the asset as laid out, recoloured twice, and once in a box
/// smaller than the asset — which `scene_view.c` sizes from the node rather
/// than the header precisely so a mismatch cannot overflow its neighbours.
///
/// Two things this golden pins that are easy to get wrong later:
///
/// * **A short box is a CENTRE crop, not a top-left crop and not a scale.**
///   The half-size node below shows the asset's middle 32x32 — its white
///   frame is gone from all four edges. LVGL's default inner alignment
///   centres a source in its object and clips, and the node sets no scale.
///   So a declared box that disagrees with the asset loses content from
///   every side, which is why whatever produces these assets must emit the
///   size the node declares.
/// * **A recoloured node is a solid silhouette here, and that is the format's
///   doing, not a bug.** `scene_view.c` pins `image_recolor_opa` to
///   `LV_OPA_COVER`, so every pixel takes the node's colour; the test asset
///   is opaque RGB565, so the silhouette is the whole rectangle. A real icon
///   asset carrying alpha would keep its shape. What these two nodes do
///   prove is that the colour comes from the node rather than being fixed.
fn scene_image_nodes() -> Vec<SceneNode> {
    let side = i32::try_from(SCENE_IMAGE_SIDE).expect("the test image fits an i32");
    // Wider than the image and centred on it, so a caption is never clipped
    // by its own box -- clipping is the text case's subject, not this one's.
    let caption = |x: i32, text: &str| {
        SceneNode::Text(SceneText {
            x: x - 32,
            baseline_y: 180,
            w: side + 64,
            align: SceneAlign::Center,
            font: SceneFont::Baked(SceneFontTier::Caption),
            color: TERTIARY,
            running_color: None,
            value: literal(text),
            ellipsize: false,
        })
    };
    vec![
        SceneNode::Image(SceneImage {
            x: 48,
            y: 88,
            w: side,
            h: side,
            digest: SCENE_IMAGE_DIGEST,
            recolor: false,
            color: 0,
        }),
        SceneNode::Image(SceneImage {
            x: 176,
            y: 88,
            w: side,
            h: side,
            digest: SCENE_IMAGE_DIGEST,
            recolor: true,
            color: ACCENT,
        }),
        SceneNode::Image(SceneImage {
            x: 304,
            y: 88,
            w: side,
            h: side,
            digest: SCENE_IMAGE_DIGEST,
            recolor: true,
            color: BLUE,
        }),
        SceneNode::Image(SceneImage {
            x: 48,
            y: 232,
            w: side / 2,
            h: side / 2,
            digest: SCENE_IMAGE_DIGEST,
            recolor: false,
            color: 0,
        }),
        caption(48, "AS IS"),
        caption(176, "ORANGE"),
        caption(304, "BLUE"),
        SceneNode::Text(SceneText {
            x: 48,
            baseline_y: 320,
            w: 300,
            align: SceneAlign::Left,
            font: SceneFont::Baked(SceneFontTier::Caption),
            color: TERTIARY,
            running_color: None,
            value: literal("HALF BOX, CENTRE CROP"),
            ellipsize: false,
        }),
    ]
}

/// `glyph`: both resolved forms `scene_view.c` accepts — a `U+XXXX` hex
/// codepoint and the glyph's own UTF-8 bytes — plus the documented fallback,
/// where a name that resolved to nothing is drawn as literal text so a
/// missing icon is visible rather than silent. All on drawn baselines, since
/// a glyph is baseline-anchored exactly as text is.
fn scene_glyph_nodes() -> Vec<SceneNode> {
    let digest = crate::assets::INTER_SUBSET_SHA256;
    vec![
        baseline_rule(160),
        baseline_rule(280),
        SceneNode::Glyph(SceneGlyph {
            x: 40,
            baseline_y: 160,
            size: 72,
            digest,
            name: "U+0041".to_string(),
            color: PRIMARY,
        }),
        SceneNode::Glyph(SceneGlyph {
            x: 130,
            baseline_y: 160,
            size: 72,
            digest,
            name: "Z".to_string(),
            color: ACCENT,
        }),
        SceneNode::Glyph(SceneGlyph {
            x: 220,
            baseline_y: 160,
            size: 72,
            digest,
            name: "U+0038".to_string(),
            color: GREEN,
        }),
        SceneNode::Glyph(SceneGlyph {
            x: 40,
            baseline_y: 280,
            size: 40,
            digest,
            name: "NOPE".to_string(),
            color: TERTIARY,
        }),
    ]
}

/// `scale`: `digital_clock.c`'s own dial — a 112px box with 13 ticks and
/// every third major, so the majors land at twelve, three, six and nine —
/// with two ordinary line nodes standing in for its hands, because a scale
/// node carries no needle. A second, denser dial pins the tick machinery at
/// a minute scale rather than an hour one.
///
/// The hands are drawn from the scale's own centre, `(x + box/2, y + box/2)`,
/// which is the translation `scene_build.rs` has to do for real: LVGL
/// positions a scale's ticks from that centre and a scene has no nesting.
fn scene_scale_nodes() -> Vec<SceneNode> {
    const BOX: i32 = 112;
    const X: i32 = 168;
    const Y: i32 = 128;
    let cx = X + BOX / 2;
    let cy = Y + BOX / 2;
    vec![
        SceneNode::Scale(SceneScale {
            x: X,
            y: Y,
            box_size: BOX,
            total_tick_count: 13,
            major_tick_every: 3,
            major_tick_color: ACCENT,
        }),
        // Hour hand, 28 long, pointing at three o'clock.
        SceneNode::Line(SceneLine {
            xs: vec![cx, cx + 28],
            ys: vec![cy, cy],
            width: 6,
            color: PRIMARY,
            ..SceneLine::default()
        }),
        // Minute hand, 42 long, pointing at twelve — straight at the major
        // tick the scale's rotation of 270 puts there.
        SceneNode::Line(SceneLine {
            xs: vec![cx, cx],
            ys: vec![cy, cy - 42],
            width: 4,
            color: ACCENT,
            ..SceneLine::default()
        }),
        SceneNode::Scale(SceneScale {
            x: 24,
            y: 240,
            box_size: 96,
            total_tick_count: 61,
            major_tick_every: 5,
            major_tick_color: BLUE,
        }),
    ]
}

/// `label`: the opaque content-sized chip, the same node with transparent
/// fill as an eyebrow, a bound pill, and an empty chip that must disappear
/// whole instead of leaving its padding as a coloured blob.
fn scene_label_nodes() -> Vec<SceneNode> {
    vec![
        SceneNode::Label(SceneLabel {
            // Same x as the left-anchored row below: this pill must straddle
            // the guide while that one begins there. Ignoring the anchor
            // therefore changes pixels rather than merely weakening intent.
            x: 224,
            y: 48,
            horizontal_anchor: SceneLabelAnchor::Center,
            font: SceneFont::Baked(SceneFontTier::Caption),
            value: literal("WEATHER AV"),
            ink: 0x0004_1a24,
            fill: 0x0035_b6f5,
            fill_opacity: u8::MAX,
            radius: 12,
            pad_hor: 16,
            pad_ver: 3,
            letter_space: 1,
            hide_when_empty: true,
        }),
        SceneNode::Label(SceneLabel {
            x: 224,
            y: 96,
            horizontal_anchor: SceneLabelAnchor::Left,
            font: SceneFont::Baked(SceneFontTier::Caption),
            value: literal("LEFT EDGE"),
            ink: 0x0004_1a24,
            fill: 0x0035_b6f5,
            fill_opacity: u8::MAX,
            radius: 12,
            pad_hor: 16,
            pad_ver: 3,
            letter_space: 1,
            hide_when_empty: true,
        }),
        SceneNode::Label(SceneLabel {
            x: 32,
            y: 136,
            horizontal_anchor: SceneLabelAnchor::Left,
            font: SceneFont::Baked(SceneFontTier::Caption),
            value: literal("TRANSPARENT EYEBROW"),
            ink: TERTIARY,
            fill: PINK,
            fill_opacity: 0,
            radius: 0,
            pad_hor: 0,
            pad_ver: 0,
            letter_space: 2,
            hide_when_empty: false,
        }),
        SceneNode::Label(SceneLabel {
            x: 32,
            y: 216,
            horizontal_anchor: SceneLabelAnchor::Left,
            font: SceneFont::Baked(SceneFontTier::Body),
            value: binding("field.status"),
            ink: 0x000f_0726,
            fill: 0x008b_6cff,
            fill_opacity: u8::MAX,
            radius: 18,
            pad_hor: 16,
            pad_ver: 4,
            letter_space: 0,
            hide_when_empty: true,
        }),
        // A broken implementation draws this as a pink padded blob. The
        // correct one contributes no pixels at all.
        SceneNode::Label(SceneLabel {
            x: 320,
            y: 304,
            horizontal_anchor: SceneLabelAnchor::Left,
            font: SceneFont::Baked(SceneFontTier::Caption),
            value: literal(""),
            ink: PRIMARY,
            fill: PINK,
            fill_opacity: u8::MAX,
            radius: 12,
            pad_hor: 24,
            pad_ver: 4,
            letter_space: 1,
            hide_when_empty: true,
        }),
    ]
}

/// `rot_rect`: three pivoted clock hands at separate centres so the hour,
/// minute and second bindings are each visible even when two angles happen
/// to coincide at the pinned instant. A fourth fixed-angle hand proves the
/// empty-binding path uses the node's `rotation` unchanged. Its clip starts
/// partway along the transformed diagonal, cutting through both anti-aliased
/// edges rather than merely bounding the unrotated object.
fn scene_rot_rect_nodes() -> Vec<SceneNode> {
    let hand = |cx: i32, fill: u32, rotation_binding: &str| {
        SceneNode::RotRect(SceneRotRect {
            x: cx - 3,
            y: 64,
            w: 6,
            h: 104,
            radius: 3,
            fill,
            pivot_x: 3,
            pivot_y: 104,
            rotation: 0,
            rotation_binding: rotation_binding.to_string(),
            clip: None,
        })
    };
    vec![
        hand(96, PRIMARY, "time:hour"),
        hand(224, ACCENT, "time:minute"),
        hand(352, BLUE, "time:second"),
        SceneNode::RotRect(SceneRotRect {
            x: 221,
            y: 232,
            w: 6,
            h: 88,
            radius: 3,
            fill: GREEN,
            pivot_x: 3,
            pivot_y: 88,
            rotation: -450,
            rotation_binding: String::new(),
            clip: Some(SceneClipRect {
                x: 180,
                y: 240,
                w: 68,
                h: 80,
            }),
        }),
        SceneNode::Rect(SceneRect {
            x: 90,
            y: 162,
            w: 12,
            h: 12,
            radius: 6,
            fill: PRIMARY,
            opacity: u8::MAX,
            clip: None,
        }),
        SceneNode::Rect(SceneRect {
            x: 218,
            y: 162,
            w: 12,
            h: 12,
            radius: 6,
            fill: ACCENT,
            opacity: u8::MAX,
            clip: None,
        }),
        SceneNode::Rect(SceneRect {
            x: 346,
            y: 162,
            w: 12,
            h: 12,
            radius: 6,
            fill: BLUE,
            opacity: u8::MAX,
            clip: None,
        }),
        SceneNode::Rect(SceneRect {
            x: 218,
            y: 314,
            w: 12,
            h: 12,
            radius: 6,
            fill: GREEN,
            opacity: u8::MAX,
            clip: None,
        }),
    ]
}

/// One case per `scene_node_kind_t`, each at both mount orientations, pinned
/// by `tests/scene.rs` against `tests/golden/scene/`.
///
/// A separate table from [`golden_cases`] for the same reason
/// [`asset_font_cases`] is: these are `SceneRenderRequest`s rendered through
/// `Simulator::render_scene_png`, not `RenderRequest`s through the reference
/// template oracle. The physical harness drives these with `PushScene`, the
/// same message shipping firmware renders.
pub fn scene_cases() -> Vec<(String, SceneRenderRequest)> {
    let mut cases = Vec::new();
    scene_case(&mut cases, "rect", &scene_rect_nodes(), &[], None, &[]);
    scene_case(
        &mut cases,
        "arc",
        &scene_arc_nodes(),
        &[],
        Some(SceneTimer {
            total_ms: 100_000,
            remaining_ms: 35_000,
            running: false,
        }),
        &[],
    );
    scene_case(&mut cases, "line", &scene_line_nodes(), &[], None, &[]);
    scene_case(
        &mut cases,
        "text",
        &scene_text_nodes(),
        &[],
        None,
        &[("status".to_string(), "SYNCED".to_string())],
    );
    scene_case(
        &mut cases,
        "image",
        &scene_image_nodes(),
        &[image_asset()],
        None,
        &[],
    );
    scene_case(
        &mut cases,
        "glyph",
        &scene_glyph_nodes(),
        &[font_asset()],
        None,
        &[],
    );
    scene_case(&mut cases, "scale", &scene_scale_nodes(), &[], None, &[]);
    scene_case(
        &mut cases,
        "label",
        &scene_label_nodes(),
        &[],
        None,
        &[("status".to_string(), "READY".to_string())],
    );
    scene_case(
        &mut cases,
        "rot-rect",
        &scene_rot_rect_nodes(),
        &[],
        None,
        &[],
    );
    cases
}

// ---------------------------------------------------------------------------
// Task 8 (plugin-manifest stage): the two curated plugins' golden cases.
//
// Unlike every case above, these compile through the REAL production path
// -- `plugin::parse_manifest` + `plugin::resolve_assets` +
// `plugin::compile_scene_with_assets` against the real, committed
// `companion/plugins/{aqi,agenda}/manifest.toml` and its real committed
// assets -- rather than hand-building an equivalent `Scene`. Hand-building
// one here would risk it silently drifting from what the real compiler
// actually produces, which is exactly the kind of gap this stage has
// repeatedly found.
// ---------------------------------------------------------------------------

use std::path::{Path, PathBuf};

use plugin::{compile_scene_with_assets, parse_manifest, resolve_assets};
use protocol::AssetKind;

const AQI_FIXTURE: &str = include_str!("../../plugin/tests/fixtures/aqi_response.json");
const AGENDA_FIXTURE: &str = include_str!("../../plugin/tests/fixtures/agenda_response.json");

/// `companion/plugins/`, resolved from `lvgl-sim`'s own compile-time
/// `CARGO_MANIFEST_DIR` rather than the process's runtime working
/// directory -- this must work identically whether `cases::plugin_scene_cases`
/// is called from `cargo test -p lvgl-sim` or from the `framebuffer_diff`
/// example in a different crate entirely.
fn plugins_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins")
}

fn plugin_payload(raw: &str) -> serde_json::Value {
    let root: serde_json::Value = serde_json::from_str(raw).expect("fixture is valid JSON");
    assert_eq!(root["status"], "ok");
    root["payload"].clone()
}

/// Decodes one of this stage's own hand-built RGB565 image assets --
/// `companion/plugins/agenda/badge.rgb565` -- back into the
/// `width`/`height`/`pixels` [`SceneAsset::Image`] wants. The file IS the
/// device-native blob (see the manifest's own doc comment on why): a
/// 12-byte `lv_image_header_t` (magic 0x19, `LV_COLOR_FORMAT_RGB565` 0x12,
/// little-endian bitfields packed exactly as `sim_build_rgb565_image`
/// builds them) directly followed by raw host-endian RGB565 pixels. This is
/// the exact inverse of the encoding the badge was authored with, so
/// `sim_build_rgb565_image` re-wraps these fields into byte-identical
/// header bytes at registration time.
fn decode_rgb565_asset(bytes: &[u8]) -> (u32, u32, Vec<u16>) {
    const HEADER_LEN: usize = 12;
    assert!(
        bytes.len() > HEADER_LEN,
        "RGB565 asset is too short to carry even the 12-byte lv_image_header_t: {} bytes",
        bytes.len()
    );
    let magic = bytes[0];
    let color_format = bytes[1];
    assert_eq!(
        magic, 0x19,
        "lv_image_header_t magic must be LV_IMAGE_HEADER_MAGIC (0x19)"
    );
    assert_eq!(
        color_format, 0x12,
        "this decoder only understands LV_COLOR_FORMAT_RGB565 (0x12)"
    );
    let width = u32::from(u16::from_le_bytes([bytes[4], bytes[5]]));
    let height = u32::from(u16::from_le_bytes([bytes[6], bytes[7]]));
    let stride = u32::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    assert_eq!(
        stride,
        width * 2,
        "stride must be exactly width * 2 bytes for RGB565"
    );

    let pixel_bytes = &bytes[HEADER_LEN..];
    assert_eq!(
        pixel_bytes.len(),
        (width * height * 2) as usize,
        "pixel byte count must match width * height * 2 exactly"
    );
    let pixels = pixel_bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    (width, height, pixels)
}

/// Compiles `plugin_name`'s real manifest against `data`/`stale`/`error`,
/// returning the compiled [`Scene`] and the [`SceneAsset`]s a render
/// request must register for it -- resolved from the plugin's own real,
/// committed asset files, never hand-typed digests.
fn compile_plugin_scene(
    plugin_name: &str,
    data: serde_json::Value,
    stale: bool,
    error: Option<&str>,
) -> (Scene, Vec<SceneAsset>) {
    let dir = plugins_dir().join(plugin_name);
    let source = std::fs::read_to_string(dir.join("manifest.toml"))
        .unwrap_or_else(|error| panic!("read {plugin_name} manifest.toml: {error}"));
    let manifest = parse_manifest(&source)
        .unwrap_or_else(|error| panic!("{plugin_name} manifest must parse: {error:?}"));
    let assets = resolve_assets(&manifest, &dir)
        .unwrap_or_else(|error| panic!("{plugin_name} assets must resolve: {error:?}"));
    let metrics = app_core::BakedFontMetrics::SHIPPED;
    let snapshot = providers::ProviderSnapshot {
        value: data,
        refreshed_at: None,
        age: None,
        stale,
        error: error.map(str::to_string),
    };

    let scene = compile_scene_with_assets(&manifest, &snapshot, &metrics, 1, &assets)
        .unwrap_or_else(|error| panic!("{plugin_name} manifest must compile: {error:?}"));

    let scene_assets = assets
        .iter()
        .map(|(file, resolved)| match resolved.kind {
            AssetKind::Font | AssetKind::IconFont => {
                // aqi's `icons.ttf` is byte-identical to the already
                // committed, already-`'static` `INTER_SUBSET_TTF` (same
                // SHA-256 digest -- see `docs/plugins/manifest-v1.md`), so
                // this reuses that allocation instead of leaking a fresh
                // one just to satisfy `SceneAsset::Font`'s `'static` bound.
                assert_eq!(
                    &*resolved.bytes,
                    crate::assets::INTER_SUBSET_TTF,
                    "{plugin_name}'s font asset {file:?} is no longer byte-identical to \
                     INTER_SUBSET_TTF -- compile_plugin_scene's 'static reuse no longer holds"
                );
                SceneAsset::Font {
                    digest: resolved.digest,
                    bytes: crate::assets::INTER_SUBSET_TTF,
                }
            }
            AssetKind::Image => {
                let (width, height, pixels) = decode_rgb565_asset(&resolved.bytes);
                SceneAsset::Image {
                    digest: resolved.digest,
                    width,
                    height,
                    pixels,
                }
            }
        })
        .collect();

    (scene, scene_assets)
}

/// Appends one plugin's four data states (fresh, stale, error,
/// empty/missing-data) at both orientations -- 8 rows, name format
/// `plugin-{plugin_name}--{state_slug}--{orientation_slug}`.
fn plugin_case(
    cases: &mut Vec<(String, SceneRenderRequest)>,
    plugin_name: &str,
    fixture_payload: serde_json::Value,
    // `field.*` binding values to resolve against, per state -- see
    // `plugin_scene_cases`'s doc for why only `aqi`'s fresh/stale/error rows
    // populate this and its `empty` row deliberately does not.
    fields_by_state: [&[(&str, &str)]; 4],
) {
    let states: [(&str, serde_json::Value, bool, Option<&str>); 4] = [
        ("fresh", fixture_payload.clone(), false, None),
        ("stale", fixture_payload.clone(), true, None),
        (
            "error",
            fixture_payload,
            true,
            Some("upstream request timed out"),
        ),
        // Missing entirely, not merely stale -- every `data.*` path
        // resolves to `Missing`, and (aqi only) so does `icon()`'s
        // argument. This is deliberately not "stale with no error": it is
        // what a provider that has *never* successfully fetched anything
        // looks like, which is a different, real state.
        ("empty", serde_json::json!({}), false, None),
    ];

    for ((state_slug, data, stale, error), fields) in states.into_iter().zip(fields_by_state) {
        let (scene, assets) = compile_plugin_scene(plugin_name, data, stale, error);
        let fields: Vec<(String, String)> = fields
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect();
        for (orientation_slug, orientation) in orientations() {
            cases.push((
                format!("plugin-{plugin_name}--{state_slug}--{orientation_slug}"),
                SceneRenderRequest {
                    scene: scene.clone(),
                    assets: assets.clone(),
                    utc_offset_minutes: SCENE_OFFSET,
                    now_unix_seconds: SCENE_NOW,
                    timer: None,
                    fields: fields.clone(),
                    orientation,
                },
            ));
        }
    }
}

/// The two curated plugins' golden cases: `aqi` (a JSON object source, the
/// icon-font path, `field.*`) and `agenda` (a JSON source that is a list,
/// the bounded repeat form, a real truncation case, an image asset). Four
/// data states each, both orientations -- 16 rows, pinned by `tests/scene.rs`
/// against `tests/golden/scene/` exactly like [`scene_cases`], and pushed to
/// real hardware by `framebuffer_diff`, which is what finally exercises the
/// device's asset-transfer path (`AssetBegin`/`AssetChunk`/`AssetCommit`)
/// for the first time in this repo's test suite.
///
/// `aqi`'s three non-empty states resolve `field.title` against a real
/// value -- `field.*` has no production pusher yet (nothing calls
/// `push_data` for a plugin card), so without this every golden would only
/// ever show the device's "--" placeholder and the binding would still have
/// no pixel coverage of it actually displaying anything. The `empty` state
/// deliberately supplies no fields at all, so the matrix also keeps the
/// placeholder case. `agenda` never binds `field.*`, so its fields are
/// always empty.
pub fn plugin_scene_cases() -> Vec<(String, SceneRenderRequest)> {
    let mut cases = Vec::new();
    let live_title: &[(&str, &str)] = &[("title", "Downtown Monitoring")];
    let no_fields: &[(&str, &str)] = &[];
    plugin_case(
        &mut cases,
        "aqi",
        plugin_payload(AQI_FIXTURE),
        [live_title, live_title, live_title, no_fields],
    );
    plugin_case(
        &mut cases,
        "agenda",
        plugin_payload(AGENDA_FIXTURE),
        [no_fields, no_fields, no_fields, no_fields],
    );
    cases
}
