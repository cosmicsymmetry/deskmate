//! Host-side scene builders for Deskmate's card faces: `DigitalClock`,
//! `AnalogClock` and `ProgressRing`.
//!
//! Every numeric and coordinate constant here originated in a hand-written C
//! template that the device used to compile in. Those templates stopped
//! shipping at stage 3a and their reference copies were deleted on 2026-09-11
//! with the parity gate that compared against them; what pins these builders
//! now is `crates/lvgl-sim/tests/golden/`, which renders the same faces
//! through the real firmware scene interpreter.
//!
//! # What the host decides, and what the device still decides
//!
//! The host does all layout. It does **not** do all evaluation: the reading and
//! the seconds, date, and dial hands are bindings the device re-renders on its
//! own tick, so a scene pushed once keeps the whole face current through minute
//! boundaries and local midnight.
//!
//! # `DigitalClock`'s three coordinate systems
//!
//! The `DigitalClock` oracle's `digital_clock.c` nests its objects: the hero and
//! the modules sit on a full-canvas, style-stripped root; the date label sits
//! inside the date module; the dial sits inside the dial module; and **the two
//! hands are children of the `lv_scale`**, which positions them from its own
//! box centre.
//! A scene has no nesting — every node is a sibling under one full-canvas
//! container at the canvas origin — so this module resolves each chain to
//! absolute canvas coordinates. `hand_line()` is where that matters most; see
//! its comment.

use chrono::{NaiveDateTime, Timelike};
use protocol::{
    SCENE_CANVAS_HEIGHT, SCENE_CANVAS_WIDTH, Scene, SceneAlign, SceneArc, SceneClipRect, SceneFont,
    SceneFontTier, SceneLabel, SceneLabelAnchor, SceneLine, SceneNode, SceneRect, SceneRotRect,
    SceneScale, SceneText, SceneValue,
};

// ---------------------------------------------------------------------------
// The design-system constants, from template_internal.h.
// ---------------------------------------------------------------------------

/// `DESKMATE_GRID`.
const GRID: i32 = 8;
/// `DESKMATE_MARGIN`.
const MARGIN: i32 = 3 * GRID;
/// `DESKMATE_RADIUS_MODULE`.
const RADIUS_MODULE: i32 = 3 * GRID;

/// `DESKMATE_COLOR_CANVAS`.
const COLOR_CANVAS: u32 = 0x0000_0000;
/// `DESKMATE_COLOR_PRIMARY`.
const COLOR_PRIMARY: u32 = 0x00f5_f5f7;
/// `DESKMATE_COLOR_TERTIARY`.
const COLOR_TERTIARY: u32 = 0x005c_5c66;
/// `DESKMATE_COLOR_SURFACE`.
const COLOR_SURFACE: u32 = 0x001a_1a1f;
/// The surface colour behind built-in icon artwork in the shipped theme.
pub const SHIPPED_SCENE_SURFACE_COLOR: u32 = COLOR_SURFACE;
/// `DESKMATE_COLOR_STALE`, reserved for the shared state footer.
const COLOR_STALE: u32 = 0x00f2_c94c;
/// `DESKMATE_COLOR_ERROR`, reserved for the shared state footer.
const COLOR_ERROR: u32 = 0x00ff_6b6b;
/// LVGL's `LV_OPA_20` constant.
const OPACITY_20_PERCENT: u8 = 51;
/// `deskmate_palette(PROTOCOL_TEMPLATE_DIGITAL_CLOCK).hue`. Both clock faces
/// share one identity, so this is also the analog clock's hue.
const CLOCK_HUE: u32 = 0x00ff_8f2e;
/// `deskmate_palette(PROTOCOL_TEMPLATE_PROGRESS_RING).hue`.
const PROGRESS_RING_HUE: u32 = 0x00ff_5a3d;
/// `deskmate_palette(PROTOCOL_TEMPLATE_PROGRESS_RING).tint`.
const PROGRESS_RING_TINT: u32 = 0x00ff_9a85;
/// `DESKMATE_COLOR_SECONDARY`.
const COLOR_SECONDARY: u32 = 0x009a_9aa5;

// ---------------------------------------------------------------------------
// analog_clock.c's own geometry. Ported, not re-derived.
// ---------------------------------------------------------------------------

/// `FACE_DIAMETER`.
const ANALOG_FACE_DIAMETER: i32 = SCENE_CANVAS_HEIGHT - 2 * MARGIN;
/// `FACE_RADIUS`.
const ANALOG_FACE_RADIUS: i32 = ANALOG_FACE_DIAMETER / 2;
/// `CHAPTER_RING_WIDTH`.
const ANALOG_CHAPTER_RING_WIDTH: i32 = 4 * GRID;
/// `TICK_MAJOR_LEN`.
const ANALOG_TICK_MAJOR_LENGTH: i32 = 2 * GRID;
/// `TICK_MINOR_LEN`.
const ANALOG_TICK_MINOR_LENGTH: i32 = GRID;
/// `TICK_MAJOR_WIDTH`.
const ANALOG_TICK_MAJOR_WIDTH: i32 = 4;
/// `TICK_MINOR_WIDTH`.
const ANALOG_TICK_MINOR_WIDTH: i32 = 3;
/// `HAND_HOUR_LEN`.
const ANALOG_HOUR_LENGTH: i32 = 13 * GRID;
/// `HAND_MINUTE_LEN`.
const ANALOG_MINUTE_LENGTH: i32 = 18 * GRID;
/// `HAND_SECOND_LEN`.
const ANALOG_SECOND_LENGTH: i32 = 19 * GRID;
/// `HAND_HOUR_WIDTH`.
const ANALOG_HOUR_WIDTH: i32 = 6;
/// `HAND_MINUTE_WIDTH`.
const ANALOG_MINUTE_WIDTH: i32 = 4;
/// `HAND_SECOND_WIDTH`.
const ANALOG_SECOND_WIDTH: i32 = 2;
/// `HUB_DIAMETER`.
const ANALOG_HUB_DIAMETER: i32 = 2 * GRID;

// ---------------------------------------------------------------------------
// progress_ring.c's own geometry. Ported, not re-derived.
// ---------------------------------------------------------------------------

/// `RING_DIAMETER`.
const PROGRESS_RING_DIAMETER: i32 = 30 * GRID;
/// `RING_X`.
const PROGRESS_RING_X: i32 = 2 * GRID;
/// `RING_Y`.
const PROGRESS_RING_Y: i32 = 8 * GRID;
/// `RING_CENTER_X`.
const PROGRESS_RING_CENTER_X: i32 = PROGRESS_RING_X + PROGRESS_RING_DIAMETER / 2;
/// `RING_CENTER_Y`.
const PROGRESS_RING_CENTER_Y: i32 = PROGRESS_RING_Y + PROGRESS_RING_DIAMETER / 2;
/// `RING_WIDTH`.
const PROGRESS_RING_WIDTH: i32 = 26;
/// `CHIP_X`.
const PROGRESS_CHIP_X: i32 = 34 * GRID;
/// `CHIP_W`.
const PROGRESS_CHIP_W: i32 = 19 * GRID;
/// `CHIP_H`.
const PROGRESS_CHIP_H: i32 = 10 * GRID;
/// `CHIP_GAP`.
const PROGRESS_CHIP_GAP: i32 = 2 * GRID;
/// `CHIP_FIRST_Y`.
const PROGRESS_CHIP_FIRST_Y: i32 = 6 * GRID;
/// `CHIP_PAD`.
const PROGRESS_CHIP_PAD: i32 = 2 * GRID;
/// `RING_TEXT_W`.
const PROGRESS_RING_TEXT_W: i32 = 25 * GRID;
/// `RING_TEXT_X`.
const PROGRESS_RING_TEXT_X: i32 = PROGRESS_RING_CENTER_X - PROGRESS_RING_TEXT_W / 2;

// ---------------------------------------------------------------------------
// row_list.c's own geometry. Ported, not re-derived.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// icon_badge_text.c's own geometry. Ported, not re-derived.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// digital_clock.c's own geometry. Ported, not re-derived.
// ---------------------------------------------------------------------------

/// `TIME_Y`.
const TIME_Y: i32 = 8 * GRID;
/// `MODULE_Y`.
const MODULE_Y: i32 = 22 * GRID;
/// `MODULE_H`.
const MODULE_H: i32 = 17 * GRID;
/// `DATE_W`.
const DATE_W: i32 = 28 * GRID;
/// `DIAL_X`.
const DIAL_X: i32 = 33 * GRID;
/// `DIAL_W`.
const DIAL_W: i32 = 20 * GRID;
/// `DIAL_BOX`.
const DIAL_BOX: i32 = 14 * GRID;
/// `HAND_HOUR_LEN`.
const HAND_HOUR_LEN: i32 = 28;
/// `HAND_MINUTE_LEN`.
const HAND_MINUTE_LEN: i32 = 42;
/// `lv_obj_set_style_line_width(OBJ_HAND_HOUR, 6, 0)`, `digital_clock.c:135`.
const HAND_HOUR_WIDTH: i32 = 6;
/// `lv_obj_set_style_line_width(OBJ_HAND_MINUTE, 4, 0)`, `digital_clock.c:141`.
const HAND_MINUTE_WIDTH: i32 = 4;
/// `lv_scale_set_total_tick_count(OBJ_DIAL, 13)` — one tick per hour, with the
/// thirteenth landing back on twelve.
const DIAL_TOTAL_TICKS: u32 = 13;
/// `lv_scale_set_major_tick_every(OBJ_DIAL, 3)`.
const DIAL_MAJOR_TICK_EVERY: u32 = 3;
/// The gap `digital_clock.c:203` puts between the hero and the seconds.
const SECONDS_GAP: i32 = 2 * GRID;

/// The reading, evaluated on the device so it ticks between pushes.
const TIME_BINDING: &str = "time:HH:mm";
/// The superior figure, on the hero's baseline.
const SECONDS_BINDING: &str = "time:ss";
const DATE_BINDING: &str = "date";
const HOUR_ANGLE_BINDING: &str = "time:angle:hour";
const MINUTE_ANGLE_BINDING: &str = "time:angle:minute";

// ---------------------------------------------------------------------------
// Baked font metrics.
// ---------------------------------------------------------------------------

/// Whole-pixel advance widths for the glyph classes the digits-only
/// `DISPLAY`/`HERO` subsets carry (spec §5.2: `0-9`, `:`, `-`, `%`, U+00B0).
///
/// One `digit` field covers all ten, because every baked face gives the ten
/// digits the same advance — tabular figures are the point of the typeface
/// choice. `the_baked_font_metrics_match_the_shipped_font_sources` reads the
/// shipped sources and fails if that ever stops being true.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumericAdvances {
    pub digit: i32,
    pub colon: i32,
    pub hyphen: i32,
    pub percent: i32,
    pub degree: i32,
}

impl NumericAdvances {
    fn advance(self, character: char) -> Option<i32> {
        match character {
            '0'..='9' => Some(self.digit),
            ':' => Some(self.colon),
            '-' => Some(self.hyphen),
            '%' => Some(self.percent),
            '\u{b0}' => Some(self.degree),
            _ => None,
        }
    }
}

/// One baked tier's metrics, as the device reads them out of the `lv_font_t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TierMetrics {
    /// `lv_font_get_line_height(font)`.
    pub line_height: i32,
    /// `font->base_line`, measured up from the bottom of the line box.
    pub base_line: i32,
    /// `None` for a full-range tier, whose advances this table deliberately
    /// does not reproduce: `deskmate_number_font()` never measures one, and a
    /// partial table that looked complete would be worse than no table.
    pub numeric: Option<NumericAdvances>,
}

/// The four faces `tools/genfonts.sh` bakes, addressed by the role the C
/// templates address them by.
///
/// The numbers are transcribed from `firmware/main/ui/fonts/deskmate_font_*.c`
/// — the same bytes the device links — and a test re-reads those files and
/// fails on any disagreement. That test is the whole provenance argument: a
/// baseline offset invented here would be invisible to every other check and
/// would surface only as a failed pixel diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BakedFontMetrics {
    pub caption: TierMetrics,
    pub body: TierMetrics,
    pub display: TierMetrics,
    pub hero: TierMetrics,
}

impl BakedFontMetrics {
    /// The metrics of the faces this repository ships.
    pub const SHIPPED: Self = Self {
        // deskmate_font_18.c — full range, so no numeric table.
        caption: TierMetrics {
            line_height: 22,
            base_line: 4,
            numeric: None,
        },
        // deskmate_font_28.c — full range: the only tier with letters, and the
        // only one carrying the '.' LV_LABEL_LONG_DOT appends.
        body: TierMetrics {
            line_height: 36,
            base_line: 7,
            numeric: None,
        },
        // deskmate_font_56.c — digits-only subset.
        display: TierMetrics {
            line_height: 43,
            base_line: 1,
            numeric: Some(NumericAdvances {
                digit: 36,
                colon: 18,
                hyphen: 26,
                percent: 56,
                degree: 26,
            }),
        },
        // deskmate_font_96.c — digits-only subset.
        hero: TierMetrics {
            line_height: 72,
            base_line: 1,
            numeric: Some(NumericAdvances {
                digit: 62,
                colon: 31,
                hyphen: 45,
                percent: 96,
                degree: 44,
            }),
        },
    };

    pub fn tier(&self, tier: SceneFontTier) -> TierMetrics {
        match tier {
            SceneFontTier::Caption => self.caption,
            SceneFontTier::Body => self.body,
            SceneFontTier::Display => self.display,
            SceneFontTier::Hero => self.hero,
        }
    }

    /// `digital_clock.c:44`'s `baseline_offset()`: the distance from a label
    /// box's top edge to the baseline of the type in it.
    ///
    /// `scene_view.c`'s `baseline_box_top()` is the exact inverse, so a text
    /// node's `baseline_y` must be `box_top + baseline_offset(tier)`.
    pub fn baseline_offset(&self, tier: SceneFontTier) -> i32 {
        let metrics = self.tier(tier);
        metrics.line_height - metrics.base_line
    }

    /// `lv_text_get_size()`'s width for one line of `text` at `tier`, with
    /// `letter_space` 0.
    ///
    /// Returns `None` when the tier carries no numeric table, or when `text`
    /// steps outside the subset — the two cases where this side must not
    /// pretend to know what the device would measure.
    pub fn measure(&self, tier: SceneFontTier, text: &str) -> Option<i32> {
        let numeric = self.tier(tier).numeric?;
        // lv_text_get_width sums the advances and trims one trailing
        // letter_space; at letter_space 0 that is a plain sum.
        text.chars().try_fold(0, |width, character| {
            numeric.advance(character).map(|advance| width + advance)
        })
    }
}

// ---------------------------------------------------------------------------
// Tier selection — a port of template_internal.h.
// ---------------------------------------------------------------------------

/// `deskmate_text_is_numeric()` (`template_internal.h:109`).
///
/// True when every character is covered by the digits-only `DISPLAY`/`HERO`
/// subsets. The C walks bytes and special-cases U+00B0's two-byte UTF-8
/// sequence; walking `char`s is the same predicate over valid UTF-8, which a
/// Rust `&str` always is. An empty string is not numeric — callers substitute
/// their own placeholder first.
pub fn text_is_numeric(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| matches!(c, '0'..='9' | ':' | '-' | '%' | '\u{b0}'))
}

/// `deskmate_number_font()` (`template_internal.h:136`): the largest tier that
/// both covers `text`'s glyphs and fits `max_width` on one line, walking
/// `HERO` → `DISPLAY` → `BODY`.
///
/// `BODY` is the floor for the C's two reasons: it is the only tier with
/// letters, and the only one that can ellipsize, since its range includes the
/// `.` `LV_LABEL_LONG_DOT` appends.
///
/// `digital_clock.c` does **not** call this — it pins `DESKMATE_FONT_HERO` for
/// the reading — so neither does [`build_digital_clock_scene`]. It is ported
/// here for the faces that do, and because pinning a tier is only safe if the
/// rule would have agreed; `the_ported_tier_rule_agrees_with_the_heros_pin`
/// asserts it does.
pub fn number_font_tier(text: &str, max_width: i32, metrics: &BakedFontMetrics) -> SceneFontTier {
    if !text_is_numeric(text) {
        return SceneFontTier::Body;
    }
    for tier in [SceneFontTier::Hero, SceneFontTier::Display] {
        if metrics
            .measure(tier, text)
            .is_some_and(|width| width <= max_width)
        {
            return tier;
        }
    }
    SceneFontTier::Body
}

// ---------------------------------------------------------------------------
// The builder.
// ---------------------------------------------------------------------------

/// The card-level inputs `digital_clock.c` draws from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockCard {
    /// Echoed as the scene's own revision.
    pub revision: u32,
    /// `digital_clock_patch()`'s `show_seconds` field. False hides the seconds
    /// in the C and drops the node here; both draw the same pixels, and
    /// neither moves anything else, because the hero is left-anchored.
    pub show_seconds: bool,
    /// The local wall-clock instant used only to measure the fixed-width hero
    /// reading while constructing the scene. Every visible time/date value is
    /// a device-side binding and continues changing between pushes.
    pub local_now: NaiveDateTime,
}

/// The card-level inputs `analog_clock.c` draws from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalogClockCard {
    /// Echoed as the scene's own revision.
    pub revision: u32,
    /// `analog_clock_patch()`'s `show_seconds` field. False hides the second
    /// hand without moving any other part of the face.
    pub show_seconds: bool,
}

/// The host-known inputs needed to build the static `ProgressRing` scene.
/// Remaining time and running state deliberately are not carried here: the
/// device supplies them through the scene binding context, so putting them on
/// this struct would imply that changing them changes the built scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgressRingCard<'a> {
    /// Echoed as the scene's own revision.
    pub revision: u32,
    /// `progress_ring_patch()`'s `label` field.
    pub label: &'a str,
    /// Selects the C template's duration-zero initial state. For a positive
    /// duration, TOTAL, ELAPSED, STATUS, and the countdown are live bindings;
    /// for a non-positive duration the C patch leaves their constructed text.
    pub duration_seconds: i64,
}

/// The host-owned data state shared by all six scene builders.
///
/// This deliberately is not a scene binding. The host learns that upstream data
/// state changed, rebuilds the scene, and pushes it with the new facts. A non-empty
/// error wins over stale exactly as it does in
/// `template_view.c::update_data_state()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneDataState<'a> {
    /// Whether the last-good value has aged past its freshness bound.
    pub stale: bool,
    /// The host-visible data/configuration error, when one exists.
    pub error: Option<&'a str>,
}

/// Applies the shared C-template data-state footer to a freshly built scene.
///
/// The six builders emit the OK state directly, with no footer node. A host
/// that owns a stale/error fact passes the builder result through this
/// function before pushing it, keeping the stateful footer path shared.
pub fn with_scene_data_state(
    mut scene: Scene,
    state: SceneDataState<'_>,
    metrics: &BakedFontMetrics,
) -> Scene {
    push_state_footer(&mut scene.nodes, state, metrics);
    // A five-row RowList is the binding node-count case: its 23 face nodes
    // plus this footer land exactly at protocol::MAX_SCENE_NODES. Validate
    // through the protocol's canonical bounds so adding one more RowList node
    // fails here with the real reason instead of surfacing later as an opaque
    // decoder refusal only when that card becomes stale or errored.
    #[cfg(debug_assertions)]
    {
        let validation = protocol::validate_scene(&scene);
        debug_assert!(
            validation.is_ok(),
            "scene became invalid after adding the shared data-state footer: {validation:?}"
        );
    }
    scene
}

/// `template_view.c::update_data_state()`, ported once for every builder.
fn push_state_footer(
    nodes: &mut Vec<SceneNode>,
    state: SceneDataState<'_>,
    metrics: &BakedFontMetrics,
) {
    let (text, color) = match state.error.filter(|error| !error.is_empty()) {
        Some(error) => (error, COLOR_ERROR),
        None if state.stale => ("Stale", COLOR_STALE),
        None => return,
    };
    let caption = metrics.tier(SceneFontTier::Caption);
    nodes.push(SceneNode::Text(SceneText {
        // A content-sized C label aligned with BOTTOM_MID computes
        // `parent_w / 2 - label_w / 2`; a full-width text box computes
        // `(parent_w - text_w) / 2`. Those differ by one pixel when the text
        // width is odd, so use the 1..448 box that preserves LVGL's separate
        // integer halves.
        x: 1,
        // The C label is aligned BOTTOM_MID at y=-2*GRID. Its line box is
        // content-height, so the baseline is canvas_bottom - offset -
        // font.base_line; the line height cancels out.
        baseline_y: SCENE_CANVAS_HEIGHT - 2 * GRID - caption.base_line,
        w: SCENE_CANVAS_WIDTH - 1,
        align: SceneAlign::Center,
        font: SceneFont::Baked(SceneFontTier::Caption),
        color,
        running_color: None,
        value: SceneValue::Literal(text.to_string()),
        ellipsize: false,
    }));
}

fn finish_scene(revision: u32, nodes: Vec<SceneNode>) -> Scene {
    Scene {
        revision,
        background: COLOR_CANVAS,
        nodes,
    }
}

/// `timefmt_hhmm()`: `"%02d:%02d"`. The host renders it only to *measure* it —
/// the node carries a binding, so the device renders the reading itself.
fn time_text(now: NaiveDateTime) -> String {
    format!("{:02}:{:02}", now.hour(), now.minute())
}

/// One live clock hand in absolute canvas coordinates, plus the stroke
/// `digital_clock.c` gives it.
///
/// The pivot is translated out of the scale's local frame here. The endpoint
/// is deliberately not: the device recomputes it with LVGL's own trig table on
/// every binding refresh.
///
/// This helper reproduces a needle that the C face parents under its 112px
/// `lv_scale`. LVGL clamps that needle's length to half the scale box and the
/// parent clips its stroke; a scene line has neither behaviour. Byte parity
/// therefore requires `length + width / 2 < DIAL_BOX / 2`. Keep that
/// precondition if another scale-owned C needle is moved to this binding form.
fn hand_line(
    pivot: (i32, i32),
    length: i32,
    angle_binding: &str,
    width: i32,
    color: u32,
) -> SceneLine {
    debug_assert!(length + width / 2 < DIAL_BOX / 2);
    SceneLine {
        xs: Vec::new(),
        ys: Vec::new(),
        width,
        color,
        pivot_x: pivot.0,
        pivot_y: pivot.1,
        length,
        angle_binding: angle_binding.to_string(),
    }
}

/// Builds the whole `DigitalClock` face as a scene.
///
/// The node order is `digital_clock.c`'s creation order, which is its z-order:
/// the reading, the seconds, the date module and its label, the dial module,
/// the dial, then the two hands over it.
///
/// # `OBJ_STATE` in the OK state, and what that demands of a parity fixture
///
/// `OBJ_STATE` (`digital_clock.c:144-147`) is the shared state footer. In the
/// OK state `template_view.c:99` sets it to the empty string, which draws
/// nothing, so this compatibility entry point omits it. Stale/error callers
/// pass the result through [`with_scene_data_state`], which appends the shared
/// footer node without duplicating the six face builders.
///
/// **A fixture comparing this scene against the C face must drive the C side
/// through the state-update path** — `template_view.c`'s `update_data_state()`,
/// i.e. the full `template_view_show`/`template_view_patch` push — and not
/// merely leave `stale` and `error` unset. `digital_clock.c` creates that label
/// and *never* sets its text; `lv_label.c:764` assigns `LV_LABEL_DEFAULT_TEXT`
/// in the constructor, and `firmware/lv_conf.h`'s
/// `LV_WIDGETS_HAS_DEFAULT_VALUE 1` makes that default the literal string
/// `"Text"` (the simulator compiles the same `lv_conf.h`). So a harness that
/// calls `digital_clock_create()` and `digital_clock_tick()` directly renders
/// the word "Text" bottom-centre, in a region neither implementation looks
/// like it draws — a very expensive pixel-diff hunt for a fixture mistake.
pub fn build_digital_clock_scene(card: &ClockCard, metrics: &BakedFontMetrics) -> Scene {
    let mut nodes = Vec::with_capacity(8);

    // --- the reading. digital_clock.c:69-74.
    //
    // The tier is HERO because the C pins HERO; see number_font_tier's doc.
    // The width is deliberately generous: the C label is content-sized while a
    // scene text node is a fixed box, and a box measured exactly risks a
    // one-pixel clip if the device's measurement differs. LEFT alignment
    // starts at the box edge and LONG_MODE_CLIP suppresses wrapping, so extra
    // width costs nothing.
    let hero_baseline = TIME_Y + metrics.baseline_offset(SceneFontTier::Hero);
    nodes.push(SceneNode::Text(SceneText {
        x: MARGIN,
        baseline_y: hero_baseline,
        w: SCENE_CANVAS_WIDTH - MARGIN,
        align: SceneAlign::Left,
        font: SceneFont::Baked(SceneFontTier::Hero),
        color: COLOR_PRIMARY,
        running_color: None,
        value: SceneValue::Binding(TIME_BINDING.to_string()),
        ellipsize: false,
    }));

    // --- the seconds. digital_clock.c:80-84, positioned by :202-205.
    //
    // The C re-anchors this after every tick, because the hero is
    // content-sized: OUT_RIGHT_TOP of the hero, one 2*GRID gap along, with a y
    // offset of baseline_offset(HERO) - baseline_offset(DISPLAY). That offset
    // exists to put the two on one baseline, so in a baseline-addressed scene
    // it collapses to "the same baseline_y". The x has to be computed, and it
    // is safe to compute once because every digit in the HERO subset has the
    // same advance -- so "HH:mm" measures the same width at every minute of
    // the day. A test walks all 1440 of them.
    if card.show_seconds {
        let hero_width = metrics
            .measure(SceneFontTier::Hero, &time_text(card.local_now))
            .expect("a HH:mm reading is inside the HERO subset");
        let seconds_x = MARGIN + hero_width + SECONDS_GAP;
        nodes.push(SceneNode::Text(SceneText {
            x: seconds_x,
            baseline_y: hero_baseline,
            w: SCENE_CANVAS_WIDTH - seconds_x,
            align: SceneAlign::Left,
            font: SceneFont::Baked(SceneFontTier::Display),
            color: CLOCK_HUE,
            running_color: None,
            value: SceneValue::Binding(SECONDS_BINDING.to_string()),
            ellipsize: false,
        }));
    }

    // --- the date module and its one label. digital_clock.c:89-98.
    nodes.push(SceneNode::Rect(SceneRect {
        x: MARGIN,
        y: MODULE_Y,
        w: DATE_W,
        h: MODULE_H,
        radius: RADIUS_MODULE,
        fill: COLOR_SURFACE,
        opacity: u8::MAX,
        clip: None,
    }));
    // The date is the module's whole content -- no eyebrow names it -- so the
    // value centres in the surface.
    let value_line = metrics.tier(SceneFontTier::Body).line_height;
    let stack_top = (MODULE_H - value_line) / 2;
    nodes.push(SceneNode::Text(SceneText {
        // deskmate_label_box() is called with x = DESKMATE_MARGIN *inside* the
        // module, whose own origin is DESKMATE_MARGIN on the canvas.
        x: MARGIN + MARGIN,
        baseline_y: MODULE_Y + stack_top + metrics.baseline_offset(SceneFontTier::Body),
        // Exactly deskmate_label_box()'s width, not a generous one: this box is
        // fixed on both sides, and a wider one would ellipsize differently.
        w: DATE_W - 2 * MARGIN,
        align: SceneAlign::Left,
        font: SceneFont::Baked(SceneFontTier::Body),
        color: COLOR_PRIMARY,
        running_color: None,
        value: SceneValue::Binding(DATE_BINDING.to_string()),
        // deskmate_label_box() uses LV_LABEL_LONG_DOT.
        ellipsize: true,
    }));

    // --- the dial module and the dial. digital_clock.c:100-127.
    nodes.push(SceneNode::Rect(SceneRect {
        x: DIAL_X,
        y: MODULE_Y,
        w: DIAL_W,
        h: MODULE_H,
        radius: RADIUS_MODULE,
        fill: COLOR_SURFACE,
        opacity: u8::MAX,
        clip: None,
    }));
    let dial_origin = (
        DIAL_X + (DIAL_W - DIAL_BOX) / 2,
        MODULE_Y + (MODULE_H - DIAL_BOX) / 2,
    );
    nodes.push(SceneNode::Scale(SceneScale {
        x: dial_origin.0,
        y: dial_origin.1,
        box_size: DIAL_BOX,
        total_tick_count: DIAL_TOTAL_TICKS,
        major_tick_every: DIAL_MAJOR_TICK_EVERY,
        // Only the major ticks carry the face's hue; the minor ticks are the
        // fixed DESKMATE_COLOR_TERTIARY grey, which scene_view.c owns.
        major_tick_color: CLOCK_HUE,
    }));

    // --- the two hands. digital_clock.c:132-142, driven by :207-213.
    //
    let dial_pivot = (dial_origin.0 + DIAL_BOX / 2, dial_origin.1 + DIAL_BOX / 2);
    nodes.push(SceneNode::Line(hand_line(
        dial_pivot,
        HAND_HOUR_LEN,
        HOUR_ANGLE_BINDING,
        HAND_HOUR_WIDTH,
        COLOR_PRIMARY,
    )));
    nodes.push(SceneNode::Line(hand_line(
        dial_pivot,
        HAND_MINUTE_LEN,
        MINUTE_ANGLE_BINDING,
        HAND_MINUTE_WIDTH,
        CLOCK_HUE,
    )));

    finish_scene(card.revision, nodes)
}

/// Converts the centre of a child in `OBJ_FACE`'s local coordinate frame to
/// that child's absolute scene origin. LVGL centres each axis as
/// `parent / 2 - object / 2`; keeping those divisions independent matters for
/// odd-width children such as the 3px minor ticks.
fn analog_face_child_origin(w: i32, h: i32, local_center_y: i32) -> (i32, i32) {
    (
        SCENE_CANVAS_WIDTH / 2 - w / 2,
        SCENE_CANVAS_HEIGHT / 2 - ANALOG_FACE_DIAMETER / 2 + local_center_y - h / 2,
    )
}

fn analog_hand(length: i32, width: i32, color: u32, binding: &str) -> SceneNode {
    // make_hand() aligns the object to FACE centre with a -length/2 offset,
    // putting its bottom-centre pivot exactly on the hub. `width / 2` is also
    // the radius after LVGL's min(w, h) / 2 clamp, so the clamp is inert here.
    let local_center_y = ANALOG_FACE_RADIUS - length / 2;
    let (x, y) = analog_face_child_origin(width, length, local_center_y);
    SceneNode::RotRect(SceneRotRect {
        x,
        y,
        w: width,
        h: length,
        radius: width / 2,
        fill: color,
        pivot_x: width / 2,
        pivot_y: length,
        rotation: 0,
        rotation_binding: binding.to_string(),
        clip: None,
    })
}

fn analog_tick(index: i32) -> SceneNode {
    let major = index % 3 == 0;
    let (w, h) = if major {
        (ANALOG_TICK_MAJOR_WIDTH, ANALOG_TICK_MAJOR_LENGTH)
    } else {
        (ANALOG_TICK_MINOR_WIDTH, ANALOG_TICK_MINOR_LENGTH)
    };
    // LV_ALIGN_TOP_MID uses independent parent/object halves. Expressing the
    // local centre as h/2 lands the top edge at the face origin while keeping
    // the odd 3px minor width on LVGL's exact x coordinate.
    let (x, y) = analog_face_child_origin(w, h, h / 2);
    SceneNode::RotRect(SceneRotRect {
        x,
        y,
        w,
        h,
        radius: 0,
        fill: if index == 0 {
            CLOCK_HUE
        } else if major {
            COLOR_SECONDARY
        } else {
            COLOR_TERTIARY
        },
        pivot_x: w / 2,
        pivot_y: ANALOG_FACE_RADIUS,
        rotation: index * 300,
        rotation_binding: String::new(),
        // The C ticks are children of OBJ_FACE, so all twelve inherit its
        // clipping box. A scene display list is flat; carry that parent clip
        // explicitly so the four cardinal rotations cannot expose a pixel
        // just outside the 320px face.
        clip: Some(SceneClipRect {
            x: SCENE_CANVAS_WIDTH / 2 - ANALOG_FACE_DIAMETER / 2,
            y: SCENE_CANVAS_HEIGHT / 2 - ANALOG_FACE_DIAMETER / 2,
            w: ANALOG_FACE_DIAMETER,
            h: ANALOG_FACE_DIAMETER,
        }),
    })
}

/// Builds the live `AnalogClock` face as a scene.
///
/// All three hands use device-side rotation bindings, so the scene keeps time
/// without a host push. `show_seconds = false` drops the second-hand node,
/// matching `LV_OBJ_FLAG_HIDDEN` without moving anything else.
pub fn build_analog_clock_scene(card: &AnalogClockCard) -> Scene {
    let mut nodes = Vec::with_capacity(17);

    nodes.push(SceneNode::Arc(SceneArc {
        cx: SCENE_CANVAS_WIDTH / 2,
        cy: SCENE_CANVAS_HEIGHT / 2,
        r: ANALOG_FACE_RADIUS,
        start_deg: 0,
        end_deg: 360,
        width: ANALOG_CHAPTER_RING_WIDTH,
        color: COLOR_SURFACE,
        running_color: None,
        opacity: u8::MAX,
        rounded: false,
        end_binding: String::new(),
    }));

    // The twelve ticks are aligned at the face's top-middle, then rotated
    // around its centre by their external local pivot. Their rotations are
    // fixed; only the three hands below use live time bindings.
    nodes.extend((0..12).map(analog_tick));

    nodes.push(analog_hand(
        ANALOG_HOUR_LENGTH,
        ANALOG_HOUR_WIDTH,
        COLOR_PRIMARY,
        "time:hour",
    ));
    nodes.push(analog_hand(
        ANALOG_MINUTE_LENGTH,
        ANALOG_MINUTE_WIDTH,
        COLOR_PRIMARY,
        "time:minute",
    ));
    if card.show_seconds {
        nodes.push(analog_hand(
            ANALOG_SECOND_LENGTH,
            ANALOG_SECOND_WIDTH,
            CLOCK_HUE,
            "time:second",
        ));
    }

    let (hub_x, hub_y) =
        analog_face_child_origin(ANALOG_HUB_DIAMETER, ANALOG_HUB_DIAMETER, ANALOG_FACE_RADIUS);
    nodes.push(SceneNode::Rect(SceneRect {
        x: hub_x,
        y: hub_y,
        w: ANALOG_HUB_DIAMETER,
        h: ANALOG_HUB_DIAMETER,
        // The C requests LV_RADIUS_CIRCLE; LVGL clamps it to half the 16px
        // square before drawing, which is the value represented here.
        radius: ANALOG_HUB_DIAMETER / 2,
        fill: CLOCK_HUE,
        opacity: u8::MAX,
        clip: None,
    }));

    // `OBJ_STATE` is empty in the OK-state parity fixtures, so it has no
    // visible scene node.
    finish_scene(card.revision, nodes)
}

/// Adds one of `progress_ring.c`'s three fixed module stacks. The caption is
/// static; the value may be a live timer binding.
fn push_progress_module(
    nodes: &mut Vec<SceneNode>,
    y: i32,
    caption: &str,
    value: SceneValue,
    value_color: u32,
    running_color: Option<u32>,
    metrics: &BakedFontMetrics,
) {
    nodes.push(SceneNode::Rect(SceneRect {
        x: PROGRESS_CHIP_X,
        y,
        w: PROGRESS_CHIP_W,
        h: PROGRESS_CHIP_H,
        radius: RADIUS_MODULE,
        fill: COLOR_SURFACE,
        opacity: u8::MAX,
        clip: None,
    }));

    let eyebrow_line = metrics.tier(SceneFontTier::Caption).line_height;
    let value_line = metrics.tier(SceneFontTier::Body).line_height;
    let stack_top = (PROGRESS_CHIP_H - eyebrow_line - GRID - value_line) / 2;
    nodes.push(SceneNode::Label(SceneLabel {
        x: PROGRESS_CHIP_X + PROGRESS_CHIP_PAD,
        y: y + stack_top,
        horizontal_anchor: SceneLabelAnchor::Left,
        font: SceneFont::Baked(SceneFontTier::Caption),
        value: SceneValue::Literal(caption.to_string()),
        ink: COLOR_SECONDARY,
        fill: COLOR_CANVAS,
        fill_opacity: 0,
        radius: 0,
        pad_hor: 0,
        pad_ver: 0,
        letter_space: 3,
        hide_when_empty: false,
    }));
    nodes.push(SceneNode::Text(SceneText {
        x: PROGRESS_CHIP_X + PROGRESS_CHIP_PAD,
        baseline_y: y
            + stack_top
            + eyebrow_line
            + GRID
            + metrics.baseline_offset(SceneFontTier::Body),
        w: PROGRESS_CHIP_W - 2 * PROGRESS_CHIP_PAD,
        align: SceneAlign::Left,
        font: SceneFont::Baked(SceneFontTier::Body),
        color: value_color,
        running_color,
        value,
        ellipsize: true,
    }));
}

fn progress_timer_value(timer_active: bool, binding: &str, inactive: &str) -> SceneValue {
    if timer_active {
        SceneValue::Binding(binding.to_string())
    } else {
        SceneValue::Literal(inactive.to_string())
    }
}

/// Builds the whole `ProgressRing` face as a scene.
///
/// The track and indicator are two scene arcs because one `SceneArc` is one
/// stroke while the C `lv_arc` draws both parts. The track carries the palette
/// hue at `LV_OPA_20`, preserving LVGL's opacity-before-mask blend, while the
/// indicator and countdown remain live through `timer.permille` and
/// `timer.remaining:mm:ss`.
pub fn build_progress_ring_scene(card: &ProgressRingCard<'_>, metrics: &BakedFontMetrics) -> Scene {
    let timer_active = card.duration_seconds >= 1;
    let mut nodes = Vec::with_capacity(13);
    for (color, running_color, opacity, rounded, end_binding) in [
        (
            PROGRESS_RING_HUE,
            None,
            OPACITY_20_PERCENT,
            false,
            String::new(),
        ),
        (
            COLOR_TERTIARY,
            Some(PROGRESS_RING_HUE),
            u8::MAX,
            true,
            "timer.permille".to_string(),
        ),
    ] {
        nodes.push(SceneNode::Arc(SceneArc {
            cx: PROGRESS_RING_CENTER_X,
            cy: PROGRESS_RING_CENTER_Y,
            r: PROGRESS_RING_DIAMETER / 2,
            start_deg: 270,
            end_deg: 630,
            width: PROGRESS_RING_WIDTH,
            color,
            running_color,
            opacity,
            rounded,
            end_binding,
        }));
    }

    let label_line = metrics.tier(SceneFontTier::Caption).line_height;
    let time_line = metrics.tier(SceneFontTier::Display).line_height;
    let pair_top = PROGRESS_RING_CENTER_Y - (label_line + GRID + time_line) / 2;
    nodes.push(SceneNode::Text(SceneText {
        x: PROGRESS_RING_TEXT_X,
        baseline_y: pair_top + metrics.baseline_offset(SceneFontTier::Caption),
        w: PROGRESS_RING_TEXT_W,
        align: SceneAlign::Center,
        font: SceneFont::Baked(SceneFontTier::Caption),
        color: PROGRESS_RING_TINT,
        running_color: None,
        value: SceneValue::Literal(card.label.to_string()),
        ellipsize: true,
    }));
    nodes.push(SceneNode::Text(SceneText {
        x: PROGRESS_RING_TEXT_X,
        baseline_y: pair_top + label_line + GRID + metrics.baseline_offset(SceneFontTier::Display),
        w: PROGRESS_RING_TEXT_W,
        align: SceneAlign::Center,
        font: SceneFont::Baked(SceneFontTier::Display),
        color: COLOR_PRIMARY,
        running_color: None,
        value: progress_timer_value(timer_active, "timer.remaining:mm:ss", "00:00"),
        ellipsize: true,
    }));

    push_progress_module(
        &mut nodes,
        PROGRESS_CHIP_FIRST_Y,
        "TOTAL",
        progress_timer_value(timer_active, "timer.total:mm:ss", ""),
        COLOR_PRIMARY,
        None,
        metrics,
    );
    push_progress_module(
        &mut nodes,
        PROGRESS_CHIP_FIRST_Y + PROGRESS_CHIP_H + PROGRESS_CHIP_GAP,
        "ELAPSED",
        progress_timer_value(timer_active, "timer.elapsed:mm:ss", ""),
        PROGRESS_RING_TINT,
        None,
        metrics,
    );
    push_progress_module(
        &mut nodes,
        PROGRESS_CHIP_FIRST_Y + 2 * (PROGRESS_CHIP_H + PROGRESS_CHIP_GAP),
        "STATUS",
        progress_timer_value(timer_active, "timer.status", ""),
        COLOR_PRIMARY,
        Some(PROGRESS_RING_HUE),
        metrics,
    );

    finish_scene(card.revision, nodes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use protocol::{
        MAX_PAYLOAD_SIZE, Message, PushScene, SceneAlign, SceneFont, SceneLine, SceneNode,
        SceneRect, SceneScale, SceneText, SceneValue, decode_wire_frame, encode_message,
        validate_scene,
    };

    // ---------------------------------------------------------------- fixtures

    fn at(hour: u32, minute: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 8, 12)
            .expect("a real date")
            .and_hms_opt(hour, minute, 30)
            .expect("a real time")
    }

    fn scene_at(hour: u32, minute: u32, show_seconds: bool) -> Scene {
        build_digital_clock_scene(
            &ClockCard {
                revision: 7,
                show_seconds,
                local_now: at(hour, minute),
            },
            &BakedFontMetrics::SHIPPED,
        )
    }

    fn text(scene: &Scene, index: usize) -> &SceneText {
        match &scene.nodes[index] {
            SceneNode::Text(t) => t,
            other => panic!("node {index} is {other:?}, not a text node"),
        }
    }

    fn rect(scene: &Scene, index: usize) -> &SceneRect {
        match &scene.nodes[index] {
            SceneNode::Rect(r) => r,
            other => panic!("node {index} is {other:?}, not a rect node"),
        }
    }

    fn scale(scene: &Scene, index: usize) -> &SceneScale {
        match &scene.nodes[index] {
            SceneNode::Scale(s) => s,
            other => panic!("node {index} is {other:?}, not a scale node"),
        }
    }

    fn line(scene: &Scene, index: usize) -> &SceneLine {
        match &scene.nodes[index] {
            SceneNode::Line(l) => l,
            other => panic!("node {index} is {other:?}, not a line node"),
        }
    }

    #[test]
    fn hiding_analog_seconds_drops_only_the_second_hand() {
        let card = AnalogClockCard {
            revision: 7,
            show_seconds: true,
        };
        let with_seconds = build_analog_clock_scene(&card);
        let without_seconds = build_analog_clock_scene(&AnalogClockCard {
            show_seconds: false,
            ..card
        });

        let mut with_second_removed = with_seconds.clone();
        with_second_removed.nodes.retain(|node| {
            !matches!(
                node,
                SceneNode::RotRect(rect) if rect.rotation_binding == "time:second"
            )
        });
        assert_eq!(without_seconds, with_second_removed);
        assert_eq!(with_seconds.nodes.len(), without_seconds.nodes.len() + 1);
    }

    // ----------------------------------------------- the metrics' provenance

    /// A hardcoded baseline offset with no provenance is the single most
    /// likely cause of a failed parity gate, so this test reads the shipped
    /// baked font sources -- the exact bytes `lv_font_get_line_height()` and
    /// `font->base_line` return on the device -- and refuses any table that
    /// disagrees with them.
    #[test]
    fn the_baked_font_metrics_match_the_shipped_font_sources() {
        for (path, tier, expected) in [
            (
                "deskmate_font_18.c",
                SceneFontTier::Caption,
                BakedFontMetrics::SHIPPED.caption,
            ),
            (
                "deskmate_font_28.c",
                SceneFontTier::Body,
                BakedFontMetrics::SHIPPED.body,
            ),
            (
                "deskmate_font_56.c",
                SceneFontTier::Display,
                BakedFontMetrics::SHIPPED.display,
            ),
            (
                "deskmate_font_96.c",
                SceneFontTier::Hero,
                BakedFontMetrics::SHIPPED.hero,
            ),
        ] {
            let source = read_font_source(path);
            assert_eq!(
                expected.line_height,
                scalar(&source, ".line_height = "),
                "{path}: line_height"
            );
            assert_eq!(
                expected.base_line,
                scalar(&source, ".base_line = "),
                "{path}: base_line"
            );
            assert_eq!(
                expected,
                BakedFontMetrics::SHIPPED.tier(tier),
                "{path}: tier lookup"
            );

            // A face carries a numeric table here exactly when it is one of
            // the two digits-only subsets, which the baked source shows as a
            // sparse cmap. Without this, dropping a table would make the
            // advance assertions below vanish rather than fail.
            let is_subset = source.contains("LV_FONT_FMT_TXT_CMAP_SPARSE_TINY");
            assert_eq!(
                is_subset,
                expected.numeric.is_some(),
                "{path}: subset faces carry a numeric table and full-range faces do not"
            );
            let Some(numeric) = expected.numeric else {
                continue;
            };
            let advances = subset_advances(&source);
            for digit in '0'..='9' {
                assert_eq!(
                    Some(numeric.digit),
                    advances(digit),
                    "{path}: advance of '{digit}'"
                );
            }
            assert_eq!(Some(numeric.colon), advances(':'), "{path}: ':'");
            assert_eq!(Some(numeric.hyphen), advances('-'), "{path}: '-'");
            assert_eq!(Some(numeric.percent), advances('%'), "{path}: '%'");
            assert_eq!(Some(numeric.degree), advances('\u{b0}'), "{path}: degree");
        }
    }

    fn read_font_source(name: &str) -> String {
        let path = format!(
            "{}/../../../firmware/main/ui/fonts/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
    }

    fn scalar(source: &str, key: &str) -> i32 {
        let tail = source
            .split_once(key)
            .unwrap_or_else(|| panic!("{key} missing"))
            .1;
        let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().expect("an integer follows the key")
    }

    /// Rebuilds the codepoint -> whole-pixel advance map for a digits-only
    /// subset font, the way `lv_font_get_glyph_dsc_fmt_txt()` does:
    /// `(adv_w + (1 << 3)) >> 4` with no kerning, since `kern_dsc` is NULL in
    /// every baked face.
    fn subset_advances(source: &str) -> impl Fn(char) -> Option<i32> + '_ {
        let advances: Vec<i32> = source
            .match_indices(".adv_w = ")
            .map(|(at, key)| {
                let tail = &source[at + key.len()..];
                let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
                let raw: i32 = digits.parse().expect("adv_w is an integer");
                (raw + 8) >> 4
            })
            .collect();
        let range_start = scalar(source, ".range_start = ");
        // The indexing below reads glyph_dsc[list_index + 1]. That is only the
        // right glyph while glyph_id_start is 1, which a regenerated subset
        // need not preserve -- and taking it from a comment would turn this
        // provenance barrier into a rubber stamp comparing the wrong glyphs.
        assert_eq!(
            1,
            scalar(source, ".glyph_id_start = "),
            "the advance lookup below assumes glyph_id_start == 1"
        );
        let list = source
            .split_once("static const uint16_t unicode_list_0[] = {")
            .expect("a sparse cmap list")
            .1
            .split_once("};")
            .expect("a terminated list")
            .0;
        let codepoints: Vec<u32> = list
            .split(',')
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .map(|token| {
                let offset = u32::from_str_radix(
                    token.trim().trim_start_matches("0x").trim_end_matches('\n'),
                    16,
                )
                .expect("a hex offset");
                u32::try_from(range_start).expect("a non-negative range start") + offset
            })
            .collect();

        move |ch| {
            let index = codepoints.iter().position(|&c| c == ch as u32)?;
            // glyph_id_start is asserted to be 1 above, and glyph_dsc[0] is
            // the reserved entry, so the sparse list's Nth codepoint is
            // glyph_dsc[N + 1].
            advances.get(index + 1).copied()
        }
    }

    // ------------------------------------------------------ the scene itself

    #[test]
    fn the_digital_clock_scene_has_the_nodes_the_c_template_creates() {
        let scene = scene_at(10, 9, true);
        // digital_clock.c creates, in order: OBJ_TIME, OBJ_SECONDS, the date
        // module, OBJ_DATE, the dial module, OBJ_DIAL, OBJ_HAND_HOUR,
        // OBJ_HAND_MINUTE. OBJ_STATE draws nothing in the OK state.
        assert_eq!(8, scene.nodes.len());
        assert!(matches!(scene.nodes[0], SceneNode::Text(_)));
        assert!(matches!(scene.nodes[1], SceneNode::Text(_)));
        assert!(matches!(scene.nodes[2], SceneNode::Rect(_)));
        assert!(matches!(scene.nodes[3], SceneNode::Text(_)));
        assert!(matches!(scene.nodes[4], SceneNode::Rect(_)));
        assert!(matches!(scene.nodes[5], SceneNode::Scale(_)));
        assert!(matches!(scene.nodes[6], SceneNode::Line(_)));
        assert!(matches!(scene.nodes[7], SceneNode::Line(_)));
        assert_eq!(7, scene.revision);
        // DESKMATE_COLOR_CANVAS.
        assert_eq!(0x0000_0000, scene.background);
    }

    #[test]
    fn the_shared_footer_prioritises_error_and_treats_an_empty_error_as_absent() {
        let ok = scene_at(10, 9, true);
        let stale = with_scene_data_state(
            ok.clone(),
            SceneDataState {
                stale: true,
                error: Some(""),
            },
            &BakedFontMetrics::SHIPPED,
        );
        let error = with_scene_data_state(
            ok.clone(),
            SceneDataState {
                stale: true,
                error: Some("Sync failed"),
            },
            &BakedFontMetrics::SHIPPED,
        );

        assert_eq!(stale.nodes.len(), ok.nodes.len() + 1);
        let stale_footer = text(&stale, stale.nodes.len() - 1);
        assert_eq!(SceneValue::Literal("Stale".to_string()), stale_footer.value);
        assert_eq!(COLOR_STALE, stale_footer.color);
        assert_eq!(1, stale_footer.x);
        assert_eq!(SCENE_CANVAS_WIDTH - 1, stale_footer.w);
        assert_eq!(348, stale_footer.baseline_y);
        assert_eq!(SceneAlign::Center, stale_footer.align);
        assert_eq!(SceneFont::Baked(SceneFontTier::Caption), stale_footer.font);

        let error_footer = text(&error, error.nodes.len() - 1);
        assert_eq!(
            SceneValue::Literal("Sync failed".to_string()),
            error_footer.value
        );
        assert_eq!(COLOR_ERROR, error_footer.color);
    }

    #[test]
    fn the_hero_reading_sits_on_the_c_templates_baseline() {
        let scene = scene_at(10, 9, true);
        let hero = text(&scene, 0);
        // digital_clock.c:74 -- lv_obj_set_pos(OBJ_TIME, DESKMATE_MARGIN, TIME_Y)
        // with DESKMATE_MARGIN = 3 * 8 = 24 and TIME_Y = 8 * 8 = 64, plus
        // baseline_offset(HERO) = line_height 72 - base_line 1 = 71.
        assert_eq!(24, hero.x);
        assert_eq!(64 + 71, hero.baseline_y);
        assert_eq!(SceneFont::Baked(SceneFontTier::Hero), hero.font);
        assert_eq!(SceneAlign::Left, hero.align);
        // DESKMATE_COLOR_PRIMARY.
        assert_eq!(0x00f5_f5f7, hero.color);
        assert_eq!(SceneValue::Binding("time:HH:mm".to_string()), hero.value);
        assert!(!hero.ellipsize);
        // Generous by design: the C hero is content-sized at 279px, a scene
        // text node is a fixed box, and an exactly measured box risks a 1px
        // clip. 424 runs to the right canvas edge.
        assert_eq!(424, hero.w);
    }

    #[test]
    fn the_seconds_share_the_heros_baseline_and_sit_one_gap_right() {
        let scene = scene_at(10, 9, true);
        let hero = text(&scene, 0);
        let seconds = text(&scene, 1);
        // digital_clock.c:202 aligns OBJ_SECONDS OUT_RIGHT_TOP of the
        // content-sized hero with a 2 * DESKMATE_GRID gap and a y offset of
        // baseline_offset(HERO) - baseline_offset(DISPLAY), which puts both on
        // one baseline. "00:00" in HERO is 4 * 62 + 31 = 279px.
        assert_eq!(24 + 279 + 16, seconds.x);
        assert_eq!(hero.baseline_y, seconds.baseline_y);
        assert_eq!(SceneFont::Baked(SceneFontTier::Display), seconds.font);
        // The face's palette hue, digital_clock.c:83.
        assert_eq!(0x00ff_8f2e, seconds.color);
        assert_eq!(SceneValue::Binding("time:ss".to_string()), seconds.value);
        assert!(!seconds.ellipsize);
        assert_eq!(448 - (24 + 279 + 16), seconds.w);
    }

    #[test]
    fn the_two_modules_land_on_the_c_templates_grid() {
        let scene = scene_at(10, 9, true);
        // deskmate_module(root, DESKMATE_MARGIN, MODULE_Y, DATE_W, MODULE_H)
        // with MODULE_Y = 22 * 8, DATE_W = 28 * 8, MODULE_H = 17 * 8, and
        // DESKMATE_RADIUS_MODULE = 3 * 8.
        let date_module = rect(&scene, 2);
        assert_eq!(24, date_module.x);
        assert_eq!(176, date_module.y);
        assert_eq!(224, date_module.w);
        assert_eq!(136, date_module.h);
        assert_eq!(24, date_module.radius);
        // DESKMATE_COLOR_SURFACE at LV_OPA_COVER.
        assert_eq!(0x001a_1a1f, date_module.fill);
        assert_eq!(255, date_module.opacity);

        // deskmate_module(root, DIAL_X, MODULE_Y, DIAL_W, MODULE_H) with
        // DIAL_X = 33 * 8 and DIAL_W = 20 * 8.
        let dial_module = rect(&scene, 4);
        assert_eq!(264, dial_module.x);
        assert_eq!(176, dial_module.y);
        assert_eq!(160, dial_module.w);
        assert_eq!(136, dial_module.h);
        assert_eq!(24, dial_module.radius);
        assert_eq!(0x001a_1a1f, dial_module.fill);
    }

    #[test]
    fn the_date_box_reproduces_deskmate_label_box() {
        let scene = scene_at(10, 9, true);
        let date = text(&scene, 3);
        // deskmate_label_box(date_module, DESKMATE_MARGIN, stack_top,
        //                    DATE_W - 2 * DESKMATE_MARGIN, LEFT, PRIMARY, BODY)
        // where stack_top = (MODULE_H - line_height(BODY)) / 2 = (136 - 36) / 2
        // = 50, so the box top is 176 + 50 = 226 and the baseline is that plus
        // baseline_offset(BODY) = 36 - 7 = 29.
        assert_eq!(24 + 24, date.x);
        assert_eq!(226 + 29, date.baseline_y);
        assert_eq!(224 - 48, date.w);
        assert_eq!(SceneFont::Baked(SceneFontTier::Body), date.font);
        assert_eq!(SceneAlign::Left, date.align);
        assert_eq!(0x00f5_f5f7, date.color);
        // deskmate_label_box uses LV_LABEL_LONG_DOT.
        assert!(date.ellipsize);
        assert_eq!(SceneValue::Binding("date".to_string()), date.value);
    }

    #[test]
    fn the_dial_is_a_scale_node_at_the_module_centre() {
        let scene = scene_at(10, 9, true);
        let dial = scale(&scene, 5);
        // digital_clock.c:104-106 -- DIAL_BOX = 14 * 8 = 112, positioned at
        // ((DIAL_W - DIAL_BOX) / 2, (MODULE_H - DIAL_BOX) / 2) inside a module
        // whose own origin is (DIAL_X, MODULE_Y).
        assert_eq!(264 + 24, dial.x);
        assert_eq!(176 + 12, dial.y);
        assert_eq!(112, dial.box_size);
        assert_eq!(13, dial.total_tick_count);
        assert_eq!(3, dial.major_tick_every);
        assert_eq!(0x00ff_8f2e, dial.major_tick_color);
    }

    #[test]
    fn the_hands_are_bound_at_the_scales_canvas_space_pivot() {
        let scene = scene_at(10, 9, true);
        // lv_scale_set_line_needle_value writes points in the scale's own
        // frame: (box/2, box/2) and (box/2 + dx, box/2 + dy), with the line
        // aligned to the scale's top-left. The scale's top-left is (288, 188),
        // so the canvas-space centre is (288 + 56, 188 + 56) = (344, 244).
        //
        let hour = line(&scene, 6);
        assert!(hour.xs.is_empty());
        assert!(hour.ys.is_empty());
        assert_eq!(344, hour.pivot_x);
        assert_eq!(244, hour.pivot_y);
        assert_eq!(28, hour.length);
        assert_eq!("time:angle:hour", hour.angle_binding);
        assert_eq!(6, hour.width);
        assert_eq!(0x00f5_f5f7, hour.color);

        let minute = line(&scene, 7);
        assert!(minute.xs.is_empty());
        assert!(minute.ys.is_empty());
        assert_eq!(344, minute.pivot_x);
        assert_eq!(244, minute.pivot_y);
        assert_eq!(42, minute.length);
        assert_eq!("time:angle:minute", minute.angle_binding);
        assert_eq!(4, minute.width);
        assert_eq!(0x00ff_8f2e, minute.color);
    }

    #[test]
    fn changing_the_push_instant_does_not_change_the_hands() {
        let midnight = scene_at(0, 0, true);
        let quarter_past = scene_at(3, 15, true);
        assert_eq!(line(&midnight, 6), line(&quarter_past, 6));
        assert_eq!(line(&midnight, 7), line(&quarter_past, 7));
    }

    #[test]
    fn hiding_the_seconds_drops_the_node_rather_than_moving_anything() {
        let with = scene_at(10, 9, true);
        let without = scene_at(10, 9, false);
        assert_eq!(7, without.nodes.len());
        // The hero is left-anchored, so hiding the seconds moves nothing --
        // digital_clock.c:79 says exactly this.
        assert_eq!(with.nodes[0], without.nodes[0]);
        assert_eq!(with.nodes[2..], without.nodes[1..]);
        assert!(
            without
                .nodes
                .iter()
                .all(|node| !matches!(node, SceneNode::Text(t)
                    if t.value == SceneValue::Binding("time:ss".to_string())))
        );
    }

    #[test]
    fn every_digit_shares_one_advance_so_the_seconds_never_move() {
        // The C re-anchors the seconds after every tick because the hero is
        // content-sized; the scene pins one x at build time. That is only
        // sound because every digit in the HERO subset has the same advance,
        // so "HH:mm" measures 279px whatever the time.
        let reference = text(&scene_at(0, 0, true), 1).x;
        for hour in 0..24 {
            for minute in 0..60 {
                assert_eq!(
                    reference,
                    text(&scene_at(hour, minute, true), 1).x,
                    "{hour:02}:{minute:02}"
                );
            }
        }
    }

    // --------------------------------------------------- the ported subroutines

    #[test]
    fn the_numeric_test_accepts_exactly_the_subsets_range() {
        assert!(text_is_numeric("00:00"));
        assert!(text_is_numeric("-12%"));
        assert!(text_is_numeric("21\u{b0}"));
        assert!(!text_is_numeric(""));
        assert!(!text_is_numeric("12.5"));
        assert!(!text_is_numeric("Mon"));
    }

    #[test]
    fn the_tier_rule_walks_hero_then_display_then_body() {
        let metrics = &BakedFontMetrics::SHIPPED;
        // "00:00" is 279px in HERO and 162px in DISPLAY.
        assert_eq!(SceneFontTier::Hero, number_font_tier("00:00", 279, metrics));
        assert_eq!(
            SceneFontTier::Display,
            number_font_tier("00:00", 278, metrics)
        );
        assert_eq!(SceneFontTier::Body, number_font_tier("00:00", 161, metrics));
        // A non-numeric string never reaches a subset tier at all.
        assert_eq!(SceneFontTier::Body, number_font_tier("Mon", 448, metrics));
    }

    #[test]
    fn the_ported_tier_rule_agrees_with_the_heros_pin() {
        // digital_clock.c:70 pins DESKMATE_FONT_HERO rather than calling
        // deskmate_number_font, so the builder pins it too. This asserts the
        // ported rule would not have disagreed at the width available to the
        // reading, which is what makes the pin safe.
        assert_eq!(
            SceneFontTier::Hero,
            number_font_tier("00:00", 448 - 2 * 24, &BakedFontMetrics::SHIPPED)
        );
    }

    #[test]
    fn progress_ring_scene_uses_live_timer_bindings() {
        let scene = build_progress_ring_scene(
            &ProgressRingCard {
                revision: 1,
                label: "Pomodoro",
                duration_seconds: 1_500,
            },
            &BakedFontMetrics::SHIPPED,
        );
        let bindings: Vec<&str> = scene
            .nodes
            .iter()
            .filter_map(|node| match node {
                SceneNode::Text(text) => match &text.value {
                    SceneValue::Binding(binding) => Some(binding.as_str()),
                    SceneValue::Literal(_) => None,
                },
                _ => None,
            })
            .collect();
        assert_eq!(
            bindings,
            [
                "timer.remaining:mm:ss",
                "timer.total:mm:ss",
                "timer.elapsed:mm:ss",
                "timer.status",
            ]
        );
    }

    // ------------------------------------------------------- the wire budget

    #[test]
    fn the_scene_validates_and_fits_one_protocol_envelope() {
        let scene = scene_at(10, 9, true);
        validate_scene(&scene).expect("the builder emits a scene the device accepts");

        let wire = encode_message(
            1,
            &Message::PushScene(PushScene {
                card_id: "desk-clock".to_string(),
                revision: 7,
                scene,
            }),
        )
        .expect("a digital clock scene fits one envelope");
        let payload = decode_wire_frame(&wire)
            .expect("a well-formed frame")
            .payload;
        assert!(payload.len() <= MAX_PAYLOAD_SIZE);
        // Pinned so a node added here shows up as a budget change rather than
        // as a surprise at 2034. 304 of 2034 is 15% of one envelope, which is
        // the headroom that lets the plan refuse a second chunk protocol.
        assert_eq!(304, payload.len(), "encoded PushScene payload");
    }
}
