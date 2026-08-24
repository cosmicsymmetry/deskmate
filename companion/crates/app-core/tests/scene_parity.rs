//! Task 9 (stage 2a): **the parity gate.**
//!
//! The whole argument for the scene renderer is that a display list pushed from
//! the host can reproduce a hand-written C template *exactly*. This test is that
//! claim, executed: for one pinned instant it renders the shipped `DigitalClock`
//! C face through `template_view_show()` and [`build_digital_clock_scene`]'s
//! scene through `scene_decode()` + `ui/scene_view.c`, in the same LVGL
//! simulator, and asserts the two 448x368 RGB565 framebuffers are
//! **byte-identical** — at both mount orientations, with the seconds shown and
//! hidden.
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

/// The mount offset the fixture pins, in minutes east of UTC. Non-zero on
/// purpose: a zero offset would let a bug that drops the offset entirely pass
/// on both sides.
const UTC_OFFSET_MINUTES: i16 = 240;

/// The local wall-clock instant both halves are drawn for.
///
/// Chosen so nothing about the frame is degenerate: the hour is past noon (so
/// the 12-hour dial's `hour % 12` is exercised rather than being the identity),
/// the minute is not a multiple of five (so the minute hand lands between the
/// dial's ticks rather than on one), the seconds are two distinct non-zero
/// digits, and the date is a two-digit day in a month whose name is not the
/// first in the table.
fn local_now() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 8, 12)
        .expect("a real date")
        .and_hms_opt(16, 9, 37)
        .expect("a real time")
}

/// The UTC instant the fixture pins, derived from [`local_now`] so the two
/// halves cannot drift apart.
///
/// This is the second of the two agreements Task 7's report demands. The scene's
/// reading and seconds are `time:` bindings the *device* evaluates from
/// `unix_seconds + utc_offset_minutes * 60`, exactly as
/// `digital_clock_tick()` evaluates `clock_source_now() + utc_offset_minutes *
/// 60`; the dial's hands and the date are host-computed statics baked in from
/// [`ClockCard::local_now`]. Deriving one from the other means the hands and the
/// digits agree *within* a frame as well as *between* the two frames.
fn now_unix_seconds() -> i64 {
    local_now().and_utc().timestamp() - i64::from(UTC_OFFSET_MINUTES) * 60
}

fn template_request(show_seconds: bool, orientation: SimOrientation) -> RenderRequest {
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
        utc_offset_minutes: UTC_OFFSET_MINUTES,
        now_unix_seconds: now_unix_seconds(),
        orientation,
    }
}

fn scene_request(show_seconds: bool, orientation: SimOrientation) -> SceneRenderRequest {
    let scene = build_digital_clock_scene(
        &ClockCard {
            revision: 1,
            show_seconds,
            local_now: local_now(),
        },
        &BakedFontMetrics::SHIPPED,
    );
    SceneRenderRequest {
        scene,
        assets: Vec::new(),
        utc_offset_minutes: UTC_OFFSET_MINUTES,
        now_unix_seconds: now_unix_seconds(),
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
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/scene-parity")
        .to_path_buf();
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
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), LOGICAL_WIDTH, LOGICAL_HEIGHT);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    writer
        .write_image_data(&rgb)
        .map_err(|error| std::io::Error::other(error.to_string()))
}

/// **The gate.** Four frames — seconds shown and hidden, at both mount
/// orientations — each rendered twice and compared byte for byte.
#[test]
fn the_host_built_scene_is_byte_identical_to_the_c_digital_clock() {
    let mut sim = Simulator::new().expect("simulator");
    let mut failures = Vec::new();

    for show_seconds in [true, false] {
        for (orientation_slug, orientation) in [
            ("landscape", SimOrientation::Landscape),
            ("flipped", SimOrientation::LandscapeFlipped),
        ] {
            let seconds_slug = if show_seconds { "seconds" } else { "no-seconds" };
            let name = format!("digital-clock--{seconds_slug}--{orientation_slug}");

            let template = sim
                .render(&template_request(show_seconds, orientation))
                .unwrap_or_else(|error| panic!("{name}: C template render failed: {error}"));
            let scene = sim
                .render_scene(&scene_request(show_seconds, orientation))
                .unwrap_or_else(|error| panic!("{name}: scene render failed: {error}"));

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

    assert!(
        failures.is_empty(),
        "the host-built scene is not byte-identical to the C DigitalClock:\n{}",
        failures.join("\n")
    );
}

/// The gate above compares two renders of the *same* instant, so a fixture that
/// silently rendered nothing — a blank canvas on both sides — would pass it.
/// This pins that both halves actually drew something.
#[test]
fn neither_half_of_the_gate_renders_a_blank_canvas() {
    let mut sim = Simulator::new().expect("simulator");
    let template = sim
        .render(&template_request(true, SimOrientation::Landscape))
        .expect("C template render");
    let scene = sim
        .render_scene(&scene_request(true, SimOrientation::Landscape))
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
    let with = sim
        .render(&template_request(true, SimOrientation::Landscape))
        .expect("C template with seconds");
    let without = sim
        .render(&template_request(false, SimOrientation::Landscape))
        .expect("C template without seconds");
    assert_ne!(with, without, "show_seconds did not change the C face");

    let with = sim
        .render_scene(&scene_request(true, SimOrientation::Landscape))
        .expect("scene with seconds");
    let without = sim
        .render_scene(&scene_request(false, SimOrientation::Landscape))
        .expect("scene without seconds");
    assert_ne!(with, without, "show_seconds did not change the scene");
}
