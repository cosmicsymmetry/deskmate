//! Task 9 (stage 2a): **the parity gate.**
//!
//! The whole argument for the scene renderer is that a display list pushed from
//! the host can reproduce a hand-written C template *exactly*. This test is that
//! claim, executed: for each pinned instant it renders the shipped
//! `DigitalClock` C face through `template_view_show()` and
//! [`build_digital_clock_scene`]'s scene through `scene_decode()` +
//! `ui/scene_view.c`, in the same LVGL simulator, and asserts the two 448x368
//! RGB565 framebuffers are **byte-identical** — at both mount orientations, with
//! the seconds shown and hidden.
//!
//! There is deliberately no tolerance. A fuzzy pixel gate would prove nothing:
//! "close enough" is the failure mode the scene renderer exists to rule out.
//!
//! # Why it lives here and not in `lvgl-sim/src/cases.rs`
//!
//! The brief nominated `cases.rs`, which is where every other golden case lives.
//! It cannot be: the scene under test is built by `app-core::scene_build`, and
//! `app-core -> device -> lvgl-sim` is an existing dependency edge, so
//! `lvgl-sim` naming `app-core` would be a cycle. The gate therefore sits on the
//! `app-core` side with `lvgl-sim` as a dev-dependency. Nothing about the
//! comparison needs the case table: both halves are rendered in-process here,
//! and `golden_cases()` is left at its pinned 58-case shape.
//!
//! # Driving the C side
//!
//! Through `Simulator::render`, which goes through `template_view_show()` — the
//! full state-update push — and **not** a bare `digital_clock_create()` +
//! `digital_clock_tick()`. `digital_clock.c:144` creates the shared state footer
//! and never sets its text; `lv_label.c`'s constructor assigns
//! `LV_LABEL_DEFAULT_TEXT`, which `firmware/lv_conf.h`'s
//! `LV_WIDGETS_HAS_DEFAULT_VALUE 1` makes the literal string `"Text"`. Only
//! `template_view.c`'s `update_data_state()` clears it to `""`, which is the
//! state the scene models by omitting the node. See
//! [`build_digital_clock_scene`]'s own doc comment.

use app_core::{BakedFontMetrics, ClockCard, build_digital_clock_scene};
use chrono::{NaiveDate, NaiveDateTime};
use lvgl_sim::scene::SceneRenderRequest;
use lvgl_sim::{
    LOGICAL_HEIGHT, LOGICAL_WIDTH, RenderRequest, SimField, SimFieldValue, SimOrientation,
    SimTemplate, Simulator,
};

/// One instant the gate is run at: the local wall-clock time both halves are
/// drawn for, and the mount offset that produces it.
struct Instant {
    slug: &'static str,
    /// Minutes east of UTC. Varied across the table on purpose — a fixture
    /// pinned to one offset (or worse, to zero) would let a bug that drops or
    /// mis-signs the offset pass on both sides.
    utc_offset_minutes: i16,
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
}

impl Instant {
    fn local_now(&self) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(self.year, self.month, self.day)
            .expect("a real date")
            .and_hms_opt(self.hour, self.minute, self.second)
            .expect("a real time")
    }

    /// The UTC instant, **derived from the local one** so the two halves cannot
    /// drift apart.
    ///
    /// This is the second of the two agreements Task 7's report demands. The
    /// scene's reading and seconds are `time:` bindings the *device* evaluates
    /// from `unix_seconds + utc_offset_minutes * 60`, exactly as
    /// `digital_clock_tick()` evaluates `clock_source_now() +
    /// utc_offset_minutes * 60`; the dial's hands and the date are
    /// host-computed statics baked in from [`ClockCard::local_now`]. Deriving
    /// one from the other means the hands and the digits agree *within* a frame
    /// as well as *between* the two frames.
    fn now_unix_seconds(&self) -> i64 {
        self.local_now().and_utc().timestamp() - i64::from(self.utc_offset_minutes) * 60
    }
}

/// The instants the gate runs at.
///
/// One instant would satisfy the brief, and a second one would be padding — but
/// three of the face's four moving parts are *table lookups or modular
/// arithmetic done twice, once on each side*, and those are exactly what a
/// single sample cannot separate from a coincidence:
///
/// - the weekday, where `digital_clock_tick()` indexes `timefmt.c`'s `DOW` with
///   `(tm_wday + 6) % 7` and the builder indexes its own with chrono's
///   `num_days_from_monday()` — a table rotated by a constant agrees at some
///   weekdays and not others, so **all seven appear below**;
/// - the month name and the *unpadded* day `timefmt_date()` prints, so both
///   ends of the month table and both a one- and a two-digit day appear;
/// - the dial, where `hour % 12` is the identity for most of the day. Midnight
///   and noon are both here, and so is a minute that lands exactly on a tick
///   against several that do not.
const INSTANTS: &[Instant] = &[
    // Wed, Aug 12, 16:09:37 — the baseline case: afternoon (so `hour % 12` is
    // not the identity), a minute off every dial tick, two distinct non-zero
    // seconds digits, a two-digit day.
    Instant {
        slug: "wed-aug-12-1609",
        utc_offset_minutes: 240,
        year: 2026,
        month: 8,
        day: 12,
        hour: 16,
        minute: 9,
        second: 37,
    },
    // Sun, Aug 2, 00:00:00 — midnight, at a *negative* offset. Both hands
    // straight up, `hour % 12 == 0` from the low side, an all-zero reading, and
    // a single-digit day.
    Instant {
        slug: "sun-aug-2-midnight",
        utc_offset_minutes: -300,
        year: 2026,
        month: 8,
        day: 2,
        hour: 0,
        minute: 0,
        second: 0,
    },
    // Mon, Aug 3, 12:00:00 — noon: `hour % 12 == 0` from the high side, which
    // midnight alone does not distinguish from "the modulo was dropped".
    Instant {
        slug: "mon-aug-3-noon",
        utc_offset_minutes: 0,
        year: 2026,
        month: 8,
        day: 3,
        hour: 12,
        minute: 0,
        second: 0,
    },
    // Thu, Jan 1, 23:59:59 — the first month of the table, day 1, and the last
    // second before every field rolls over at once.
    Instant {
        slug: "thu-jan-1-2359",
        utc_offset_minutes: 240,
        year: 2026,
        month: 1,
        day: 1,
        hour: 23,
        minute: 59,
        second: 59,
    },
    // Fri, Dec 25, 09:05:03 — the last month of the table, at a half-hour
    // offset, with a leading-zero hour and a leading-zero second.
    Instant {
        slug: "fri-dec-25-0905",
        utc_offset_minutes: 330,
        year: 2026,
        month: 12,
        day: 25,
        hour: 9,
        minute: 5,
        second: 3,
    },
    // Sat, Feb 28, 03:15:00 — the minute hand exactly on the three o'clock
    // major tick, which is where a hand endpoint's rounding is most visible.
    Instant {
        slug: "sat-feb-28-0315",
        utc_offset_minutes: -480,
        year: 2026,
        month: 2,
        day: 28,
        hour: 3,
        minute: 15,
        second: 0,
    },
    // Tue, Jun 30, 21:33:07 — the last weekday not otherwise covered, in the
    // dial's third quadrant.
    Instant {
        slug: "tue-jun-30-2133",
        utc_offset_minutes: 60,
        year: 2026,
        month: 6,
        day: 30,
        hour: 21,
        minute: 33,
        second: 7,
    },
];

fn template_request(
    instant: &Instant,
    show_seconds: bool,
    orientation: SimOrientation,
) -> RenderRequest {
    RenderRequest {
        template: SimTemplate::DigitalClock,
        fields: vec![
            // `title` is a schema and wire field that no clock face draws
            // (see digital_clock_patch's comment), carried here because the
            // host always sends it.
            SimField {
                name: "title".to_string(),
                value: SimFieldValue::Text("Desk".to_string()),
            },
            SimField {
                name: "show_seconds".to_string(),
                value: SimFieldValue::Boolean(show_seconds),
            },
        ],
        utc_offset_minutes: instant.utc_offset_minutes,
        now_unix_seconds: instant.now_unix_seconds(),
        orientation,
    }
}

fn scene_request(
    instant: &Instant,
    show_seconds: bool,
    orientation: SimOrientation,
) -> SceneRenderRequest {
    let scene = build_digital_clock_scene(
        &ClockCard {
            revision: 1,
            show_seconds,
            local_now: instant.local_now(),
        },
        &BakedFontMetrics::SHIPPED,
    );
    SceneRenderRequest {
        scene,
        assets: Vec::new(),
        utc_offset_minutes: instant.utc_offset_minutes,
        now_unix_seconds: instant.now_unix_seconds(),
        // The digital clock has no timer binding; a context with no running
        // timer is what the device would carry for this face.
        timer: None,
        fields: Vec::new(),
        orientation,
    }
}

/// Where two frames differ, in enough detail to localise the failure without a
/// debugger: how many pixels, and the bounding box they fall in.
struct Difference {
    count: usize,
    min_x: u32,
    min_y: u32,
    max_x: u32,
    max_y: u32,
    /// Up to a handful of `(x, y, template, scene)` samples.
    samples: Vec<(u32, u32, u16, u16)>,
}

fn diff(template: &[u16], scene: &[u16]) -> Option<Difference> {
    let mut difference = Difference {
        count: 0,
        min_x: u32::MAX,
        min_y: u32::MAX,
        max_x: 0,
        max_y: 0,
        samples: Vec::new(),
    };
    for (index, (left, right)) in template.iter().zip(scene.iter()).enumerate() {
        if left == right {
            continue;
        }
        let index = u32::try_from(index).expect("a 448x368 frame indexes inside u32");
        let (x, y) = (index % LOGICAL_WIDTH, index / LOGICAL_WIDTH);
        difference.count += 1;
        difference.min_x = difference.min_x.min(x);
        difference.min_y = difference.min_y.min(y);
        difference.max_x = difference.max_x.max(x);
        difference.max_y = difference.max_y.max(y);
        if difference.samples.len() < 8 {
            difference.samples.push((x, y, *left, *right));
        }
    }
    (difference.count > 0).then_some(difference)
}

/// Writes both frames next to each other as PNGs so a failure can be *looked
/// at*. A byte count localises nothing; a picture localises it in seconds.
fn dump(name: &str, template: &[u16], scene: &[u16]) -> String {
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/scene-parity");
    if std::fs::create_dir_all(&directory).is_err() {
        return "  (could not create target/scene-parity to dump PNGs)".to_string();
    }
    let mut written = Vec::new();
    for (suffix, pixels) in [("template", template), ("scene", scene)] {
        let path = directory.join(format!("{name}--{suffix}.png"));
        if write_png(&path, pixels).is_ok() {
            written.push(path.display().to_string());
        }
    }
    format!("  dumped: {}", written.join(", "))
}

fn write_png(path: &std::path::Path, pixels: &[u16]) -> std::io::Result<()> {
    let mut rgb = Vec::with_capacity(pixels.len() * 3);
    for pixel in pixels {
        rgb.push((((pixel >> 11) & 0x1f) as u8) << 3);
        rgb.push((((pixel >> 5) & 0x3f) as u8) << 2);
        rgb.push(((pixel & 0x1f) as u8) << 3);
    }
    let file = std::fs::File::create(path)?;
    let mut encoder =
        png::Encoder::new(std::io::BufWriter::new(file), LOGICAL_WIDTH, LOGICAL_HEIGHT);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    writer
        .write_image_data(&rgb)
        .map_err(|error| std::io::Error::other(error.to_string()))
}

/// **The gate.** Every instant in [`INSTANTS`] × seconds shown and hidden ×
/// both mount orientations, each rendered twice and compared byte for byte.
#[test]
fn the_host_built_scene_is_byte_identical_to_the_c_digital_clock() {
    let mut sim = Simulator::new().expect("simulator");
    let mut failures = Vec::new();
    let mut compared = 0_usize;

    for instant in INSTANTS {
        for show_seconds in [true, false] {
            for (orientation_slug, orientation) in [
                ("landscape", SimOrientation::Landscape),
                ("flipped", SimOrientation::LandscapeFlipped),
            ] {
                let seconds_slug = if show_seconds {
                    "seconds"
                } else {
                    "no-seconds"
                };
                let name = format!(
                    "digital-clock--{}--{seconds_slug}--{orientation_slug}",
                    instant.slug
                );

                let template = sim
                    .render(&template_request(instant, show_seconds, orientation))
                    .unwrap_or_else(|error| panic!("{name}: C template render failed: {error}"));
                let scene = sim
                    .render_scene(&scene_request(instant, show_seconds, orientation))
                    .unwrap_or_else(|error| panic!("{name}: scene render failed: {error}"));
                compared += 1;

                if let Some(difference) = diff(&template, &scene) {
                    let samples = difference
                        .samples
                        .iter()
                        .map(|(x, y, left, right)| format!("({x},{y}) {left:#06x}/{right:#06x}"))
                        .collect::<Vec<_>>()
                        .join(" ");
                    failures.push(format!(
                        "{name}: {} of {} pixels differ, bounding box \
                         x {}..={} y {}..={}\n  samples (template/scene): {samples}\n{}",
                        difference.count,
                        template.len(),
                        difference.min_x,
                        difference.max_x,
                        difference.min_y,
                        difference.max_y,
                        dump(&name, &template, &scene),
                    ));
                }
            }
        }
    }

    assert_eq!(
        compared,
        INSTANTS.len() * 4,
        "every instant is compared with the seconds shown and hidden, at both \
         mount orientations"
    );
    assert!(
        failures.is_empty(),
        "the host-built scene is not byte-identical to the C DigitalClock:\n{}",
        failures.join("\n")
    );
}

/// The instant table's whole job is to separate a real agreement from a
/// coincidence, and it can only do that if it actually covers what its own doc
/// comment claims. A roll-call, not a count, so a replaced entry that silently
/// drops a weekday fails here rather than quietly weakening the gate.
#[test]
fn the_instant_table_covers_what_it_claims_to() {
    use chrono::{Datelike, Timelike};

    let mut weekdays = std::collections::BTreeSet::new();
    let mut months = std::collections::BTreeSet::new();
    let mut offsets = std::collections::BTreeSet::new();
    let mut hours_mod_12 = std::collections::BTreeSet::new();
    let mut single_digit_day = false;
    let mut two_digit_day = false;

    for instant in INSTANTS {
        let now = instant.local_now();
        weekdays.insert(now.weekday().num_days_from_monday());
        months.insert(now.month());
        offsets.insert(instant.utc_offset_minutes);
        hours_mod_12.insert(now.hour() % 12);
        single_digit_day |= now.day() < 10;
        two_digit_day |= now.day() >= 10;
    }

    assert_eq!(weekdays.len(), 7, "all seven weekdays must appear");
    assert!(months.contains(&1), "January, the first name in MON");
    assert!(months.contains(&12), "December, the last name in MON");
    assert!(
        single_digit_day && two_digit_day,
        "timefmt_date prints an unpadded day, so both widths must appear"
    );
    assert!(
        hours_mod_12.contains(&0),
        "an hour where `hour % 12` is zero must appear"
    );
    assert!(
        offsets.iter().any(|offset| *offset < 0)
            && offsets.iter().any(|offset| *offset > 0)
            && offsets.contains(&0),
        "the offset must be exercised negative, positive and zero"
    );
    assert!(
        offsets.iter().any(|offset| offset % 60 != 0),
        "a half-hour offset must appear; whole-hour offsets alone would not \
         catch a minutes-vs-hours mix-up"
    );
}

/// The gate above compares two renders of the *same* instant, so a fixture that
/// silently rendered nothing — a blank canvas on both sides — would pass it.
/// This pins that both halves actually drew something.
#[test]
fn neither_half_of_the_gate_renders_a_blank_canvas() {
    let mut sim = Simulator::new().expect("simulator");
    let instant = &INSTANTS[0];
    let template = sim
        .render(&template_request(instant, true, SimOrientation::Landscape))
        .expect("C template render");
    let scene = sim
        .render_scene(&scene_request(instant, true, SimOrientation::Landscape))
        .expect("scene render");
    for (which, pixels) in [("template", &template), ("scene", &scene)] {
        let background = pixels[0];
        assert!(
            pixels.iter().any(|pixel| *pixel != background),
            "{which} rendered a uniform canvas, so the parity gate would be vacuous"
        );
    }
}

/// `show_seconds` must actually change the frame, on both sides. Without this a
/// builder that ignored the flag — or a C patch path that stopped hiding the
/// label — would still satisfy the gate, because the gate only ever compares
/// like against like.
#[test]
fn hiding_the_seconds_changes_both_halves() {
    let mut sim = Simulator::new().expect("simulator");
    let instant = &INSTANTS[0];
    let with = sim
        .render(&template_request(instant, true, SimOrientation::Landscape))
        .expect("C template with seconds");
    let without = sim
        .render(&template_request(instant, false, SimOrientation::Landscape))
        .expect("C template without seconds");
    assert_ne!(with, without, "show_seconds did not change the C face");

    let with = sim
        .render_scene(&scene_request(instant, true, SimOrientation::Landscape))
        .expect("scene with seconds");
    let without = sim
        .render_scene(&scene_request(instant, false, SimOrientation::Landscape))
        .expect("scene without seconds");
    assert_ne!(with, without, "show_seconds did not change the scene");
}

/// The single most expensive way to get this gate wrong is to drive the C side
/// without `template_view.c`'s state update, which leaves `OBJ_STATE` holding
/// `LV_LABEL_DEFAULT_TEXT` — the literal word `"Text"`, bottom-centre, in a
/// region *neither* source file appears to draw. The scene omits that node
/// entirely, so the mistake would surface as an unexplained band of differing
/// pixels rather than as a fixture error.
///
/// This pins the hazard directly instead of leaving it to a comment. The face's
/// lowest drawn element is the two modules, which end at
/// `MODULE_Y + MODULE_H = 176 + 136 = 312`; the state footer would sit at
/// `LV_ALIGN_BOTTOM_MID` minus `2 * DESKMATE_GRID`, i.e. inside the strip below
/// it. So every pixel from y=316 down must be the canvas colour, on both sides.
#[test]
fn neither_half_draws_anything_in_the_state_footer_strip() {
    /// `MODULE_Y + MODULE_H`, plus a few rows of slack.
    const FOOTER_TOP: u32 = 316;
    /// `DESKMATE_COLOR_CANVAS` (0x000000) in RGB565.
    const CANVAS: u16 = 0x0000;

    let mut sim = Simulator::new().expect("simulator");
    let instant = &INSTANTS[0];
    let template = sim
        .render(&template_request(instant, true, SimOrientation::Landscape))
        .expect("C template render");
    let scene = sim
        .render_scene(&scene_request(instant, true, SimOrientation::Landscape))
        .expect("scene render");

    for (which, pixels) in [("C template", &template), ("scene", &scene)] {
        let start = (FOOTER_TOP * LOGICAL_WIDTH) as usize;
        let lit = pixels[start..]
            .iter()
            .filter(|pixel| **pixel != CANVAS)
            .count();
        assert_eq!(
            lit, 0,
            "{which} drew {lit} lit pixels below y={FOOTER_TOP}. For the C \
             template that means the fixture bypassed template_view.c's \
             update_data_state() and is rendering LV_LABEL_DEFAULT_TEXT; for \
             the scene it means a node moved into the footer strip"
        );
    }
}
