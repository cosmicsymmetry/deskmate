//! `digital_clock.c`'s layout, moved to the host as a scene.
//!
//! Stage 2a's parity gate renders the shipped `DigitalClock` C template and the
//! scene this module emits through the same simulator and asserts the two
//! framebuffers are **byte-identical**. Every constant here is therefore a
//! *port* of `firmware/main/ui/templates/digital_clock.c`, not a re-derivation:
//! read that file before changing a number here, and change the number there
//! first if the layout is genuinely meant to move.
//!
//! # What the host decides, and what the device still decides
//!
//! The host does all layout. It does **not** do all evaluation: the reading and
//! the seconds are `time:` bindings the device re-renders on its own tick, so a
//! scene pushed once keeps telling the time. The dial's hands and the date
//! module are not bindable — a `SCENE_NODE_SCALE` carries no needle and there is
//! no date binding — so those two are drawn for [`ClockCard::local_now`] and go
//! stale until the host pushes again. A host that wants a live dial pushes once
//! a minute.
//!
//! # Three coordinate systems, and the one translation that matters
//!
//! `digital_clock.c` nests its objects: the hero and the modules sit on a
//! full-canvas, style-stripped root; the date label sits inside the date
//! module; the dial sits inside the dial module; and **the two hands are
//! children of the `lv_scale`**, which positions them from its own box centre.
//! A scene has no nesting — every node is a sibling under one full-canvas
//! container at the canvas origin — so this module resolves each chain to
//! absolute canvas coordinates. `hand_points()` is where that matters most; see
//! its comment.

use chrono::{Datelike, NaiveDateTime, Timelike};
use protocol::{
    SCENE_CANVAS_HEIGHT, SCENE_CANVAS_WIDTH, Scene, SceneAlign, SceneFont, SceneFontTier,
    SceneLabel, SceneLabelAnchor, SceneLine, SceneNode, SceneRect, SceneScale, SceneText,
    SceneValue,
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
/// `DESKMATE_RADIUS_CHIP`.
const RADIUS_CHIP: i32 = 2 * GRID;
/// `DESKMATE_CHIP_HEIGHT`.
const CHIP_HEIGHT: i32 = 4 * GRID;

/// `DESKMATE_COLOR_CANVAS`.
const COLOR_CANVAS: u32 = 0x0000_0000;
/// `DESKMATE_COLOR_PRIMARY`.
const COLOR_PRIMARY: u32 = 0x00f5_f5f7;
/// `DESKMATE_COLOR_SURFACE`.
const COLOR_SURFACE: u32 = 0x001a_1a1f;
/// `deskmate_palette(PROTOCOL_TEMPLATE_DIGITAL_CLOCK).hue`. Both clock faces
/// share one identity, so this is also the analog clock's hue.
const CLOCK_HUE: u32 = 0x00ff_8f2e;
/// `deskmate_palette(PROTOCOL_TEMPLATE_BIG_NUMBER_LABEL).hue`.
const BIG_NUMBER_HUE: u32 = 0x008b_6cff;
/// `deskmate_palette(PROTOCOL_TEMPLATE_BIG_NUMBER_LABEL).tint`.
const BIG_NUMBER_TINT: u32 = 0x00c0_b0ff;
/// `deskmate_palette(PROTOCOL_TEMPLATE_BIG_NUMBER_LABEL).ink`.
const BIG_NUMBER_INK: u32 = 0x000f_0726;

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
/// `DIAL_RANGE` — a 12-hour dial in minutes, so one scale positions both hands.
const DIAL_RANGE: i32 = 720;
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
/// `lv_scale_set_angle_range(OBJ_DIAL, 360)`. Fixed in `scene_view.c` too, as
/// `SCENE_SCALE_ANGLE_RANGE`, so it is not a wire field.
const DIAL_ANGLE_RANGE: i32 = 360;
/// `lv_scale_set_rotation(OBJ_DIAL, 270)` — twelve o'clock. Fixed in
/// `scene_view.c` as `SCENE_SCALE_ROTATION`.
const DIAL_ROTATION: i32 = 270;
/// The gap `digital_clock.c:203` puts between the hero and the seconds.
const SECONDS_GAP: i32 = 2 * GRID;

/// The reading, evaluated on the device so it ticks between pushes.
const TIME_BINDING: &str = "time:HH:mm";
/// The superior figure, on the hero's baseline.
const SECONDS_BINDING: &str = "time:ss";

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
// LVGL's fixed-point trigonometry, for the hands.
// ---------------------------------------------------------------------------

/// `lv_math.c`'s `sin0_90_table`, transcribed. The hands' endpoints are
/// whatever `lv_scale_set_line_needle_value()` computes, and it computes them
/// from this table, so a floating-point sine would land a pixel elsewhere.
const SIN_0_90: [i32; 91] = [
    0, 572, 1144, 1715, 2286, 2856, 3425, 3993, 4560, 5126, 5690, 6252, 6813, 7371, 7927, 8481,
    9032, 9580, 10126, 10668, 11207, 11743, 12275, 12803, 13328, 13848, 14365, 14876, 15384, 15886,
    16384, 16877, 17364, 17847, 18324, 18795, 19261, 19720, 20174, 20622, 21063, 21498, 21926,
    22348, 22763, 23170, 23571, 23965, 24351, 24730, 25102, 25466, 25822, 26170, 26510, 26842,
    27166, 27482, 27789, 28088, 28378, 28660, 28932, 29197, 29452, 29698, 29935, 30163, 30382,
    30592, 30792, 30983, 31164, 31336, 31499, 31651, 31795, 31928, 32052, 32166, 32270, 32365,
    32449, 32524, 32588, 32643, 32688, 32723, 32748, 32763, 32768,
];

/// `LV_TRIGO_SHIFT`.
const TRIGO_SHIFT: u32 = 15;

/// `lv_trigo_sin()`. The C normalises with `while` loops; `rem_euclid` is the
/// same map for every input.
fn trigo_sin(angle: i32) -> i32 {
    let normalised = angle.rem_euclid(360);
    let index = match normalised {
        0..90 => normalised,
        90..180 => 180 - normalised,
        180..270 => normalised - 180,
        _ => 360 - normalised,
    };
    let magnitude = SIN_0_90[usize::try_from(index).expect("the index is in 0..=90")];
    let signed = if normalised >= 180 {
        -magnitude
    } else {
        magnitude
    };
    // The C saturates +-32767 to +-32768. No entry in the table is 32767, so
    // this is inert today; it is kept because the table is transcribed and a
    // regenerated one could reach it.
    match signed {
        32767 => 32768,
        -32767 => -32768,
        other => other,
    }
}

/// `lv_trigo_cos()`.
fn trigo_cos(angle: i32) -> i32 {
    trigo_sin(angle + 90)
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
    /// The local wall-clock instant the dial's hands and the date module are
    /// drawn for. The reading itself is a device-side `time:` binding, so it
    /// ticks between pushes; the hands and the date do not.
    pub local_now: NaiveDateTime,
}

/// The card-level inputs `big_number_label.c` draws from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BigNumberCard<'a> {
    /// Echoed as the scene's own revision.
    pub revision: u32,
    /// `big_number_label_patch()`'s `title` field.
    pub title: &'a str,
    /// `big_number_label_patch()`'s `value` field.
    pub value: &'a str,
    /// `big_number_label_patch()`'s `label` field.
    pub label: &'a str,
}

/// `timefmt.c`'s `DOW`, which `digital_clock_tick()` indexes with
/// `(tm_wday + 6) % 7` — i.e. Monday-based.
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
/// `timefmt.c`'s `MON`.
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `timefmt_date()`: `"%s, %s %d"`, with an unpadded day.
fn date_text(now: NaiveDateTime) -> String {
    let weekday = WEEKDAYS[usize::try_from(now.weekday().num_days_from_monday())
        .expect("chrono returns 0..=6 days from Monday")];
    let month = MONTHS[usize::try_from(now.month0()).expect("chrono returns a month0 of 0..=11")];
    format!("{weekday}, {month} {}", now.day())
}

/// `timefmt_hhmm()`: `"%02d:%02d"`. The host renders it only to *measure* it —
/// the node carries a binding, so the device renders the reading itself.
fn time_text(now: NaiveDateTime) -> String {
    format!("{:02}:{:02}", now.hour(), now.minute())
}

/// One clock hand: its two endpoints in absolute canvas coordinates, plus
/// the stroke `digital_clock.c` gives it.
///
/// **This is the translation the C does not have to do.** In
/// `digital_clock.c:132` the hands are `lv_line` children of the `lv_scale`,
/// and `lv_scale_set_line_needle_value()` writes their points in the *scale's*
/// frame — `(box/2, box/2)` for the pivot — after aligning the line to the
/// scale's top-left. A scene `LINE` node is in absolute canvas coordinates
/// with its object at the canvas origin (`scene_view.c`'s `build_line()` ends
/// with `lv_obj_set_pos(object, 0, 0)`), and `scene_scale_t`'s own header says
/// "whatever emits their endpoints must derive that centre itself". So the
/// scale's top-left is added to every point.
///
/// The rest is `lv_scale_set_line_needle_value()` line for line: the needle
/// length is clamped to half the box, the value is mapped onto the angle range
/// with C's truncating division, and the offsets are fixed-point sine and
/// cosine of `rotation + angle`, arithmetic-shifted down by `LV_TRIGO_SHIFT`.
fn hand_line(origin: (i32, i32), length: i32, value: i32, width: i32, color: u32) -> SceneLine {
    // The clamp is a faithful port, but note the parity precondition it hides:
    // at actual_length == half_box the C hand touches the scale's own box edge,
    // and the C hand is a *child of the scale*, so LVGL clips its rounded cap
    // to that 112px box -- while this scene's hand is a child of the
    // full-canvas container and is not clipped. The two agree only while
    // `length + width / 2 < half_box`. DigitalClock is comfortably inside it
    // (42 + 2 = 44 < 56); a future face reusing this helper may not be.
    let half_box = DIAL_BOX / 2;
    let actual_length = if length >= half_box {
        half_box
    } else if length >= 0 {
        length
    } else if length + half_box < 0 {
        0
    } else {
        half_box + length
    };

    // scale->range_min is 0 and scale->range_max is DIAL_RANGE.
    let angle = if value < 0 {
        0
    } else if value > DIAL_RANGE {
        DIAL_ANGLE_RANGE
    } else {
        DIAL_ANGLE_RANGE * value / DIAL_RANGE
    };

    let offset_x = (actual_length * trigo_cos(DIAL_ROTATION + angle)) >> TRIGO_SHIFT;
    let offset_y = (actual_length * trigo_sin(DIAL_ROTATION + angle)) >> TRIGO_SHIFT;

    let pivot_x = origin.0 + half_box;
    let pivot_y = origin.1 + half_box;
    SceneLine {
        xs: vec![pivot_x, pivot_x + offset_x],
        ys: vec![pivot_y, pivot_y + offset_y],
        width,
        color,
    }
}

/// Builds the whole `DigitalClock` face as a scene.
///
/// The node order is `digital_clock.c`'s creation order, which is its z-order:
/// the reading, the seconds, the date module and its label, the dial module,
/// the dial, then the two hands over it.
///
/// # `OBJ_STATE` has no node, and what that demands of a parity fixture
///
/// `OBJ_STATE` (`digital_clock.c:144-147`) is the shared state footer. In the
/// OK state `template_view.c:99` sets it to the empty string, which draws
/// nothing, so the scene omits it.
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
        // No binding renders a date, so the host formats it. It goes stale at
        // local midnight until the next push.
        value: SceneValue::Literal(date_text(card.local_now)),
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
    // A 12-hour dial in minutes, so one scale positions both: the hour hand at
    // h*60+m and the minute hand at m*12.
    let hour = i32::try_from(card.local_now.hour()).expect("chrono returns an hour of 0..=23");
    let minute = i32::try_from(card.local_now.minute()).expect("chrono returns a minute of 0..=59");
    nodes.push(SceneNode::Line(hand_line(
        dial_origin,
        HAND_HOUR_LEN,
        (hour % 12) * 60 + minute,
        HAND_HOUR_WIDTH,
        COLOR_PRIMARY,
    )));
    nodes.push(SceneNode::Line(hand_line(
        dial_origin,
        HAND_MINUTE_LEN,
        minute * 12,
        HAND_MINUTE_WIDTH,
        CLOCK_HUE,
    )));

    Scene {
        revision: card.revision,
        background: COLOR_CANVAS,
        nodes,
    }
}

/// Builds the whole `BigNumberLabel` face as a scene.
///
/// The node order is `big_number_label.c`'s creation order: the title chip,
/// the value, then the label pill. `OBJ_STATE` is empty in the OK state and
/// therefore has no node.
pub fn build_big_number_label_scene(card: &BigNumberCard<'_>, metrics: &BakedFontMetrics) -> Scene {
    /// `big_number_label.c`'s `CONTENT_WIDTH`.
    const CONTENT_WIDTH: i32 = SCENE_CANVAS_WIDTH - 2 * MARGIN;

    let value = if card.value.is_empty() {
        "--"
    } else {
        card.value
    };
    let value_tier = number_font_tier(value, CONTENT_WIDTH, metrics);
    let label_tier = if value_tier == SceneFontTier::Body {
        SceneFontTier::Caption
    } else {
        SceneFontTier::Body
    };
    let value_line = metrics.tier(value_tier).line_height;
    let label_line = metrics.tier(label_tier).line_height;

    // `set_value_text()` centres the value, then moves it up by half the
    // label gap and rendered label height (including its vertical padding).
    // LVGL halves the parent and object independently; that is one pixel
    // lower than `(parent - object) / 2` for Display's odd 43px line height.
    let value_top =
        SCENE_CANVAS_HEIGHT / 2 - value_line / 2 + (-(2 * GRID + label_line + GRID) / 2);
    let value_baseline = value_top + metrics.baseline_offset(value_tier);
    let label_top = value_top + value_line + 2 * GRID;

    let nodes = vec![
        // `deskmate_chip()` at `(DESKMATE_MARGIN, 2 * DESKMATE_GRID)`.
        SceneNode::Label(SceneLabel {
            x: MARGIN,
            y: 2 * GRID,
            horizontal_anchor: SceneLabelAnchor::Left,
            font: SceneFont::Baked(SceneFontTier::Caption),
            value: SceneValue::Literal(card.title.to_string()),
            ink: BIG_NUMBER_INK,
            fill: BIG_NUMBER_HUE,
            fill_opacity: u8::MAX,
            radius: RADIUS_CHIP,
            pad_hor: 2 * GRID,
            pad_ver: (CHIP_HEIGHT - metrics.tier(SceneFontTier::Caption).line_height) / 2,
            letter_space: 1,
            hide_when_empty: true,
        }),
        SceneNode::Text(SceneText {
            x: MARGIN,
            baseline_y: value_baseline,
            w: CONTENT_WIDTH,
            align: SceneAlign::Center,
            font: SceneFont::Baked(value_tier),
            color: COLOR_PRIMARY,
            value: SceneValue::Literal(value.to_string()),
            ellipsize: true,
        }),
        // Hand-built in `big_number_label.c`, not a `deskmate_chip()`: it has
        // zero letter spacing and fixed `DESKMATE_GRID / 2` vertical padding.
        SceneNode::Label(SceneLabel {
            x: SCENE_CANVAS_WIDTH / 2,
            y: label_top,
            horizontal_anchor: SceneLabelAnchor::Center,
            font: SceneFont::Baked(label_tier),
            value: SceneValue::Literal(card.label.to_string()),
            ink: BIG_NUMBER_TINT,
            fill: COLOR_SURFACE,
            fill_opacity: u8::MAX,
            radius: RADIUS_CHIP,
            pad_hor: 2 * GRID,
            pad_ver: GRID / 2,
            letter_space: 0,
            hide_when_empty: true,
        }),
    ];

    Scene {
        revision: card.revision,
        background: COLOR_CANVAS,
        nodes,
    }
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
        // timefmt_date renders "%s, %s %d" with a Monday-based weekday table.
        // 2026-08-12 is a Wednesday.
        assert_eq!(SceneValue::Literal("Wed, Aug 12".to_string()), date.value);
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
    fn the_hands_are_translated_out_of_the_scales_local_frame() {
        let scene = scene_at(10, 9, true);
        // lv_scale_set_line_needle_value writes points in the scale's own
        // frame: (box/2, box/2) and (box/2 + dx, box/2 + dy), with the line
        // aligned to the scale's top-left. The scale's top-left is (288, 188),
        // so the canvas-space centre is (288 + 56, 188 + 56) = (344, 244).
        //
        // Hour hand: value = (10 % 12) * 60 + 9 = 609, angle = 360 * 609 / 720
        // = 304 (truncating), so the trig angle is 270 + 304 = 574.
        //   dx = (28 * lv_trigo_cos(574)) >> 15 = -24
        //   dy = (28 * lv_trigo_sin(574)) >> 15 = -16
        let hour = line(&scene, 6);
        assert_eq!(vec![344, 344 - 24], hour.xs);
        assert_eq!(vec![244, 244 - 16], hour.ys);
        assert_eq!(6, hour.width);
        assert_eq!(0x00f5_f5f7, hour.color);

        // Minute hand: value = 9 * 12 = 108, angle = 54, trig angle 324.
        //   dx = (42 * lv_trigo_cos(324)) >> 15 = 33
        //   dy = (42 * lv_trigo_sin(324)) >> 15 = -25
        let minute = line(&scene, 7);
        assert_eq!(vec![344, 344 + 33], minute.xs);
        assert_eq!(vec![244, 244 - 25], minute.ys);
        assert_eq!(4, minute.width);
        assert_eq!(0x00ff_8f2e, minute.color);
    }

    #[test]
    fn midnight_points_both_hands_straight_up() {
        let scene = scene_at(0, 0, true);
        // Both values are 0, so the angle is 0 and the trig angle is the
        // scale's rotation, 270 -- twelve o'clock.
        assert_eq!(vec![244, 244 - 28], line(&scene, 6).ys);
        assert_eq!(vec![344, 344], line(&scene, 6).xs);
        assert_eq!(vec![244, 244 - 42], line(&scene, 7).ys);
        assert_eq!(vec![344, 344], line(&scene, 7).xs);
    }

    #[test]
    fn a_quarter_past_three_points_the_minute_hand_at_three_oclock() {
        let scene = scene_at(3, 15, true);
        assert_eq!(vec![344, 344 + 42], line(&scene, 7).xs);
        assert_eq!(vec![244, 244], line(&scene, 7).ys);
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
    fn the_trig_port_reproduces_lvgls_table() {
        assert_eq!(0, trigo_sin(0));
        assert_eq!(16384, trigo_sin(30));
        assert_eq!(32768, trigo_sin(90));
        assert_eq!(0, trigo_sin(180));
        assert_eq!(-32768, trigo_sin(270));
        assert_eq!(-16384, trigo_sin(330));
        // Odd angles too: the round ones alone would not catch a transcription
        // slip in the middle of the table.
        assert_eq!(572, trigo_sin(1));
        assert_eq!(22763, trigo_sin(44));
        assert_eq!(32763, trigo_sin(89));
        assert_eq!(32763, trigo_sin(91));
        assert_eq!(-32763, trigo_sin(269));
        // The C normalises with while-loops, so out-of-turn angles wrap.
        assert_eq!(trigo_sin(30), trigo_sin(390));
        assert_eq!(trigo_sin(330), trigo_sin(-30));
    }

    /// `lv_math.c`'s table is `round(sin(d) * 32768)` at every whole degree, so
    /// the whole transcription can be checked without pasting it twice. A
    /// float sine is *not* what the builder may use at runtime -- the device
    /// reads the table, and only the table is guaranteed to round the way the
    /// device rounds -- but it is a fine oracle for the transcription.
    #[test]
    fn the_transcribed_sine_table_has_no_typo() {
        for (degrees, &entry) in SIN_0_90.iter().enumerate() {
            let degrees = u16::try_from(degrees).expect("the table is 91 entries long");
            let expected = (f64::from(degrees).to_radians().sin() * 32768.0).round();
            assert!(
                (expected - f64::from(entry)).abs() < f64::EPSILON,
                "sin0_90_table[{degrees}] is {entry}, want {expected}"
            );
        }
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
        // as a surprise at 2034. 283 of 2034 is 14% of one envelope, which is
        // the headroom that lets the plan refuse a second chunk protocol.
        assert_eq!(283, payload.len(), "encoded PushScene payload");
    }
}
