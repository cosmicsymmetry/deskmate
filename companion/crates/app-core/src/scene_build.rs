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
    SCENE_CANVAS_HEIGHT, SCENE_CANVAS_WIDTH, Scene, SceneAlign, SceneArc, SceneFont, SceneFontTier,
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
/// `DESKMATE_COLOR_TERTIARY`.
const COLOR_TERTIARY: u32 = 0x005c_5c66;
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
/// `deskmate_palette(PROTOCOL_TEMPLATE_ROW_LIST).hue`.
const ROW_LIST_HUE: u32 = 0x002f_d9c0;
/// `deskmate_palette(PROTOCOL_TEMPLATE_ROW_LIST).ink`.
const ROW_LIST_INK: u32 = 0x0004_211d;
/// `deskmate_palette(PROTOCOL_TEMPLATE_ICON_BADGE_TEXT).hue`.
const ICON_BADGE_HUE: u32 = 0x0035_b6f5;
/// `deskmate_palette(PROTOCOL_TEMPLATE_ICON_BADGE_TEXT).tint`.
const ICON_BADGE_TINT: u32 = 0x0094_d8fa;
/// `deskmate_palette(PROTOCOL_TEMPLATE_ICON_BADGE_TEXT).ink`.
const ICON_BADGE_INK: u32 = 0x0004_1a24;
/// `deskmate_palette(PROTOCOL_TEMPLATE_PROGRESS_RING).hue`.
const PROGRESS_RING_HUE: u32 = 0x00ff_5a3d;
/// `deskmate_palette(PROTOCOL_TEMPLATE_PROGRESS_RING).tint`.
const PROGRESS_RING_TINT: u32 = 0x00ff_9a85;
/// The RGB565 quantisation produced when LVGL draws `PROGRESS_RING_HUE` at
/// `LV_OPA_20` over the black canvas. It matches every fully covered track
/// pixel. It cannot match the anti-aliased annulus edge: the C composes
/// `mask * LV_OPA_20`, while an opaque scene arc composes the mask alone. A
/// fully exposed track therefore differs at 592 pixels until `SceneArc`
/// carries per-arc opacity.
const PROGRESS_RING_TRACK: u32 = 0x0028_1008;
/// `DESKMATE_COLOR_SECONDARY`.
const COLOR_SECONDARY: u32 = 0x009a_9aa5;

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
/// `CLOCK_SECONDS_MAX`.
const PROGRESS_CLOCK_SECONDS_MAX: i64 = 86_400;

// ---------------------------------------------------------------------------
// row_list.c's own geometry. Ported, not re-derived.
// ---------------------------------------------------------------------------

/// `ROW_FIRST_Y`.
const ROW_FIRST_Y: i32 = 7 * GRID;
/// `ROW_PITCH`.
const ROW_PITCH: i32 = 7 * GRID;
/// `ROW_HEIGHT`.
const ROW_HEIGHT: i32 = 6 * GRID;
/// `ROW_X`.
const ROW_X: i32 = MARGIN;
/// `ROW_WIDTH`.
const ROW_WIDTH: i32 = SCENE_CANVAS_WIDTH - 2 * MARGIN;
/// `BAR_X`.
const ROW_BAR_X: i32 = GRID;
/// `BAR_WIDTH`.
const ROW_BAR_WIDTH: i32 = 4;
/// `BAR_HEIGHT`.
const ROW_BAR_HEIGHT: i32 = 4 * GRID;
/// `TIME_X`.
const ROW_TIME_X: i32 = 3 * GRID;
/// `TIME_WIDTH`.
const ROW_TIME_WIDTH: i32 = 11 * GRID;
/// `TITLE_X`.
const ROW_TITLE_X: i32 = 16 * GRID;
/// `TITLE_WIDTH`.
const ROW_TITLE_WIDTH: i32 = ROW_WIDTH - ROW_TITLE_X - 2 * GRID;
/// `COUNT_BOX`.
const ROW_COUNT_BOX: i32 = 4 * GRID;
/// `COUNT_X`.
const ROW_COUNT_X: i32 = SCENE_CANVAS_WIDTH - MARGIN - ROW_COUNT_BOX;
/// `lv_obj_set_style_border_width(OBJ_COUNT, 2, 0)`.
const ROW_COUNT_BORDER_WIDTH: i32 = 2;

// ---------------------------------------------------------------------------
// icon_badge_text.c's own geometry. Ported, not re-derived.
// ---------------------------------------------------------------------------

/// `ICON_BOX` in both `icon_badge_text.c` and `weather_icon.c`.
const ICON_BOX: i32 = 120;
/// `ICON_TILE`.
const ICON_TILE: i32 = 19 * GRID;
/// `ICON_INSET`.
const ICON_INSET: i32 = (ICON_TILE - ICON_BOX) / 2;
/// `TEXT_LEFT`.
const ICON_TEXT_LEFT: i32 = MARGIN + ICON_TILE + 2 * GRID;
/// `TEXT_WIDTH`.
const ICON_TEXT_WIDTH: i32 = 28 * GRID;

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

/// The card-level inputs `icon_badge_text.c` draws from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IconBadgeCard<'a> {
    /// Echoed as the scene's own revision.
    pub revision: u32,
    /// `icon_badge_text_patch()`'s `title` field.
    pub title: &'a str,
    /// `icon_badge_text_patch()`'s `icon` field.
    pub icon: &'a str,
    /// `icon_badge_text_patch()`'s `badge` field.
    pub badge: &'a str,
    /// `icon_badge_text_patch()`'s `value` field.
    pub value: &'a str,
    /// `icon_badge_text_patch()`'s `label` field.
    pub label: &'a str,
}

/// The card-level inputs `progress_ring.c` draws from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgressRingCard<'a> {
    /// Echoed as the scene's own revision.
    pub revision: u32,
    /// `progress_ring_patch()`'s `label` field.
    pub label: &'a str,
    /// `progress_ring_patch()`'s `duration_seconds` field.
    pub duration_seconds: i64,
    /// `progress_ring_patch()`'s `remaining_seconds` field.
    pub remaining_seconds: i64,
    /// `progress_ring_patch()`'s `running` field.
    pub running: bool,
}

/// The card-level inputs `row_list.c` draws from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowListCard<'a> {
    /// Echoed as the scene's own revision.
    pub revision: u32,
    /// `row_list_patch()`'s `title` field.
    pub title: &'a str,
    /// `row_list_patch()`'s `row0_title` field.
    pub row0_title: &'a str,
    /// `row_list_patch()`'s `row0_time` field.
    pub row0_time: &'a str,
    /// `row_list_patch()`'s `row1_title` field.
    pub row1_title: &'a str,
    /// `row_list_patch()`'s `row1_time` field.
    pub row1_time: &'a str,
    /// `row_list_patch()`'s `row2_title` field.
    pub row2_title: &'a str,
    /// `row_list_patch()`'s `row2_time` field.
    pub row2_time: &'a str,
    /// `row_list_patch()`'s `row3_title` field.
    pub row3_title: &'a str,
    /// `row_list_patch()`'s `row3_time` field.
    pub row3_time: &'a str,
    /// `row_list_patch()`'s `row4_title` field.
    pub row4_title: &'a str,
    /// `row_list_patch()`'s `row4_time` field.
    pub row4_time: &'a str,
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

/// Converts `LV_ALIGN_CENTER` plus an `(x, y)` offset inside a container to
/// the absolute top-left a scene rectangle needs.
///
/// Keep the two halves separate. LVGL computes `parent / 2 - object / 2`,
/// which differs from `(parent - object) / 2` by one pixel when both sizes are
/// odd.
fn centered_rect_origin(
    container_origin: (i32, i32),
    container_size: (i32, i32),
    own_size: (i32, i32),
    offset: (i32, i32),
) -> (i32, i32) {
    (
        container_origin.0 + container_size.0 / 2 + offset.0 - own_size.0 / 2,
        container_origin.1 + container_size.1 / 2 + offset.1 - own_size.1 / 2,
    )
}

/// `weather_icon.c`'s `disc()` in scene form.
fn disc(
    nodes: &mut Vec<SceneNode>,
    container_origin: (i32, i32),
    color: u32,
    diameter: i32,
    x: i32,
    y: i32,
) {
    let (x, y) = centered_rect_origin(
        container_origin,
        (ICON_BOX, ICON_BOX),
        (diameter, diameter),
        (x, y),
    );
    nodes.push(SceneNode::Rect(SceneRect {
        x,
        y,
        w: diameter,
        h: diameter,
        radius: diameter / 2,
        fill: color,
        opacity: u8::MAX,
    }));
}

/// `weather_icon.c`'s `bar()` in scene form.
fn bar(
    nodes: &mut Vec<SceneNode>,
    container_origin: (i32, i32),
    color: u32,
    w: i32,
    h: i32,
    x: i32,
    y: i32,
) {
    let (x, y) = centered_rect_origin(container_origin, (ICON_BOX, ICON_BOX), (w, h), (x, y));
    nodes.push(SceneNode::Rect(SceneRect {
        x,
        y,
        w,
        h,
        radius: h / 2,
        fill: color,
        opacity: u8::MAX,
    }));
}

/// `weather_icon.c`'s `draw_sun()`.
fn draw_sun(
    nodes: &mut Vec<SceneNode>,
    container_origin: (i32, i32),
    color: u32,
    dx: i32,
    dy: i32,
    diameter: i32,
) {
    disc(nodes, container_origin, color, diameter, dx, dy);
}

/// `weather_icon.c`'s `draw_moon()`.
fn draw_moon(
    nodes: &mut Vec<SceneNode>,
    container_origin: (i32, i32),
    color: u32,
    background: u32,
    dx: i32,
    dy: i32,
) {
    disc(nodes, container_origin, color, 56, dx, dy);
    disc(nodes, container_origin, background, 48, dx + 16, dy - 8);
}

/// `weather_icon.c`'s `draw_cloud()`.
fn draw_cloud(nodes: &mut Vec<SceneNode>, container_origin: (i32, i32), color: u32, dy: i32) {
    disc(nodes, container_origin, color, 44, -18, dy);
    disc(nodes, container_origin, color, 56, 4, dy - 8);
    disc(nodes, container_origin, color, 40, 26, dy + 2);
    bar(nodes, container_origin, color, 88, 26, 2, dy + 12);
}

/// `weather_icon.c`'s `CLOUD_SUN` case.
fn draw_cloud_sun(nodes: &mut Vec<SceneNode>, container_origin: (i32, i32), color: u32) {
    draw_sun(nodes, container_origin, color, -26, -30, 44);
    draw_cloud(nodes, container_origin, color, 6);
}

/// `weather_icon.c`'s `CLOUD_MOON` case.
fn draw_cloud_moon(
    nodes: &mut Vec<SceneNode>,
    container_origin: (i32, i32),
    color: u32,
    background: u32,
) {
    draw_moon(nodes, container_origin, color, background, -26, -30);
    draw_cloud(nodes, container_origin, color, 6);
}

/// `weather_icon.c`'s `draw_drops()`.
fn draw_drops(
    nodes: &mut Vec<SceneNode>,
    container_origin: (i32, i32),
    color: u32,
    count: usize,
    length: i32,
) {
    for x in [-24, 0, 24].into_iter().take(count) {
        bar(nodes, container_origin, color, 6, length, x, 40);
    }
}

/// `weather_icon.c`'s RAIN case.
fn draw_rain(nodes: &mut Vec<SceneNode>, container_origin: (i32, i32), color: u32) {
    draw_cloud(nodes, container_origin, color, -12);
    draw_drops(nodes, container_origin, color, 3, 22);
}

/// `weather_icon.c`'s DRIZZLE case.
fn draw_drizzle(nodes: &mut Vec<SceneNode>, container_origin: (i32, i32), color: u32) {
    draw_cloud(nodes, container_origin, color, -12);
    draw_drops(nodes, container_origin, color, 2, 12);
}

/// `weather_icon.c`'s SNOW case.
fn draw_snow(nodes: &mut Vec<SceneNode>, container_origin: (i32, i32), color: u32) {
    draw_cloud(nodes, container_origin, color, -12);
    disc(nodes, container_origin, color, 10, -24, 42);
    disc(nodes, container_origin, color, 10, 0, 46);
    disc(nodes, container_origin, color, 10, 24, 42);
}

/// `weather_icon.c`'s STORM case.
fn draw_storm(nodes: &mut Vec<SceneNode>, container_origin: (i32, i32), color: u32) {
    draw_cloud(nodes, container_origin, color, -12);
    // This extends two pixels below the 120px icon container. The C parent
    // clips those pixels; flat scene nodes currently have no clip region.
    bar(nodes, container_origin, color, 10, 40, 0, 42);
}

/// `weather_icon.c`'s FOG case.
fn draw_fog(nodes: &mut Vec<SceneNode>, container_origin: (i32, i32), color: u32) {
    draw_cloud(nodes, container_origin, color, -16);
    bar(nodes, container_origin, color, 84, 8, -4, 30);
    bar(nodes, container_origin, color, 68, 8, 6, 46);
}

/// `weather_icon.c`'s UNKNOWN case.
fn draw_unknown(
    nodes: &mut Vec<SceneNode>,
    container_origin: (i32, i32),
    color: u32,
    background: u32,
) {
    disc(nodes, container_origin, color, 72, 0, 0);
    disc(nodes, container_origin, background, 52, 0, 0);
}

/// Draws `weather_icon_render()`'s artworks in their original creation order.
fn push_weather_icon_nodes(
    nodes: &mut Vec<SceneNode>,
    icon: &str,
    container_origin: (i32, i32),
    color: u32,
    background: u32,
) {
    match icon {
        "sun" => draw_sun(nodes, container_origin, color, 0, 0, 72),
        "moon" => draw_moon(nodes, container_origin, color, background, 0, 0),
        "cloud" => draw_cloud(nodes, container_origin, color, 0),
        "cloud-sun" => draw_cloud_sun(nodes, container_origin, color),
        "cloud-moon" => draw_cloud_moon(nodes, container_origin, color, background),
        "rain" => draw_rain(nodes, container_origin, color),
        "drizzle" => draw_drizzle(nodes, container_origin, color),
        "snow" => draw_snow(nodes, container_origin, color),
        "storm" => draw_storm(nodes, container_origin, color),
        "fog" => draw_fog(nodes, container_origin, color),
        // `weather_icon_from_name()` maps an unrecognised name to UNKNOWN.
        _ => draw_unknown(nodes, container_origin, color, background),
    }
}

/// Builds the whole `IconBadgeText` face as a scene.
///
/// `background` is the color physically behind the icon artwork. The shipped
/// call site passes `DESKMATE_COLOR_SURFACE`, because the icon container is a
/// child of the surface tile. MOON and UNKNOWN use it to punch out an inner
/// disc; it must not be replaced with the scene's black canvas color.
pub fn build_icon_badge_text_scene(
    card: &IconBadgeCard<'_>,
    background: u32,
    metrics: &BakedFontMetrics,
) -> Scene {
    let value = if card.value.is_empty() {
        "--"
    } else {
        card.value
    };
    let value_tier = number_font_tier(value, ICON_TEXT_WIDTH, metrics);
    let label_tier = if value_tier == SceneFontTier::Body {
        SceneFontTier::Caption
    } else {
        SceneFontTier::Body
    };
    let value_line = metrics.tier(value_tier).line_height;
    let label_line = metrics.tier(label_tier).line_height;

    // `set_value_text()` uses LV_ALIGN_LEFT_MID. Preserve LVGL's independent
    // parent/object halves instead of simplifying them to one subtraction.
    let value_top = SCENE_CANVAS_HEIGHT / 2 - value_line / 2 + (-(2 * GRID + label_line) / 2);
    let label_top = value_top + value_line + 2 * GRID;

    let tile_y = SCENE_CANVAS_HEIGHT / 2 - ICON_TILE / 2;
    let icon_origin = (MARGIN + ICON_INSET, tile_y + ICON_INSET);
    let mut nodes = Vec::with_capacity(11);

    nodes.push(SceneNode::Label(SceneLabel {
        x: MARGIN,
        y: 2 * GRID,
        horizontal_anchor: SceneLabelAnchor::Left,
        font: SceneFont::Baked(SceneFontTier::Caption),
        value: SceneValue::Literal(card.title.to_string()),
        ink: ICON_BADGE_INK,
        fill: ICON_BADGE_HUE,
        fill_opacity: u8::MAX,
        radius: RADIUS_CHIP,
        pad_hor: 2 * GRID,
        pad_ver: (CHIP_HEIGHT - metrics.tier(SceneFontTier::Caption).line_height) / 2,
        letter_space: 1,
        hide_when_empty: true,
    }));
    nodes.push(SceneNode::Label(SceneLabel {
        x: SCENE_CANVAS_WIDTH - MARGIN,
        y: 2 * GRID,
        horizontal_anchor: SceneLabelAnchor::Right,
        font: SceneFont::Baked(SceneFontTier::Caption),
        value: SceneValue::Literal(card.badge.to_string()),
        ink: ICON_BADGE_INK,
        fill: ICON_BADGE_HUE,
        fill_opacity: u8::MAX,
        radius: RADIUS_CHIP,
        pad_hor: 2 * GRID,
        pad_ver: (CHIP_HEIGHT - metrics.tier(SceneFontTier::Caption).line_height) / 2,
        letter_space: 1,
        hide_when_empty: true,
    }));
    nodes.push(SceneNode::Rect(SceneRect {
        x: MARGIN,
        y: tile_y,
        w: ICON_TILE,
        h: ICON_TILE,
        radius: RADIUS_MODULE,
        fill: COLOR_SURFACE,
        opacity: u8::MAX,
    }));
    push_weather_icon_nodes(
        &mut nodes,
        card.icon,
        icon_origin,
        ICON_BADGE_HUE,
        background,
    );
    nodes.push(SceneNode::Text(SceneText {
        x: ICON_TEXT_LEFT,
        baseline_y: value_top + metrics.baseline_offset(value_tier),
        w: ICON_TEXT_WIDTH,
        align: SceneAlign::Left,
        font: SceneFont::Baked(value_tier),
        color: COLOR_PRIMARY,
        value: SceneValue::Literal(value.to_string()),
        ellipsize: true,
    }));
    // Unlike BigNumberLabel, IconBadgeText's secondary label is a plain
    // fixed-width LVGL label: no surface fill, padding, radius, or tracking.
    nodes.push(SceneNode::Text(SceneText {
        x: ICON_TEXT_LEFT,
        baseline_y: label_top + metrics.baseline_offset(label_tier),
        w: ICON_TEXT_WIDTH,
        align: SceneAlign::Left,
        font: SceneFont::Baked(label_tier),
        color: ICON_BADGE_TINT,
        value: SceneValue::Literal(card.label.to_string()),
        ellipsize: true,
    }));

    // `OBJ_STATE` is cleared by `template_view.c` in these OK-state parity
    // fixtures, so it has no visible scene node.
    Scene {
        revision: card.revision,
        background: COLOR_CANVAS,
        nodes,
    }
}

/// Converts a row child's coordinates from its module-local frame to the
/// scene's absolute canvas frame. `row_list.c` parents the bar and two labels
/// to the row module; scene nodes are all siblings at the canvas origin.
fn row_child_origin(row_y: i32, local_x: i32, local_y: i32) -> (i32, i32) {
    (ROW_X + local_x, row_y + local_y)
}

fn push_row_nodes(
    nodes: &mut Vec<SceneNode>,
    row: usize,
    title: &str,
    time: &str,
    metrics: &BakedFontMetrics,
) {
    if title.is_empty() {
        return;
    }

    let row = i32::try_from(row).expect("the row-list has exactly five rows");
    let row_y = ROW_FIRST_Y + row * ROW_PITCH;
    nodes.push(SceneNode::Rect(SceneRect {
        x: ROW_X,
        y: row_y,
        w: ROW_WIDTH,
        h: ROW_HEIGHT,
        // `row_list_create()` overrides `deskmate_module()`'s radius.
        radius: 2 * GRID,
        fill: COLOR_SURFACE,
        opacity: u8::MAX,
    }));

    let (bar_x, bar_y) = row_child_origin(row_y, ROW_BAR_X, (ROW_HEIGHT - ROW_BAR_HEIGHT) / 2);
    nodes.push(SceneNode::Rect(SceneRect {
        x: bar_x,
        y: bar_y,
        w: ROW_BAR_WIDTH,
        h: ROW_BAR_HEIGHT,
        radius: ROW_BAR_WIDTH / 2,
        fill: ROW_LIST_HUE,
        opacity: u8::MAX,
    }));

    let text_y = (ROW_HEIGHT - metrics.tier(SceneFontTier::Body).line_height) / 2;
    let (time_x, text_top) = row_child_origin(row_y, ROW_TIME_X, text_y);
    nodes.push(SceneNode::Text(SceneText {
        x: time_x,
        baseline_y: text_top + metrics.baseline_offset(SceneFontTier::Body),
        w: ROW_TIME_WIDTH,
        align: SceneAlign::Left,
        font: SceneFont::Baked(SceneFontTier::Body),
        color: ROW_LIST_HUE,
        value: SceneValue::Literal(time.to_string()),
        ellipsize: true,
    }));

    let (title_x, _) = row_child_origin(row_y, ROW_TITLE_X, text_y);
    nodes.push(SceneNode::Text(SceneText {
        x: title_x,
        baseline_y: text_top + metrics.baseline_offset(SceneFontTier::Body),
        w: ROW_TITLE_WIDTH,
        align: SceneAlign::Left,
        font: SceneFont::Baked(SceneFontTier::Body),
        color: COLOR_PRIMARY,
        value: SceneValue::Literal(title.to_string()),
        ellipsize: true,
    }));
}

/// Builds the whole `RowList` face as a scene.
///
/// The count border is deliberately represented by a nominal full-turn arc:
/// the C draws a 2px circular `lv_obj` border, while the existing scene model
/// has no border field. The parity gate is the authority on whether LVGL's arc
/// and border drawing paths produce the same pixels; the radius and width here
/// are the C object's unadjusted `COUNT_BOX / 2` and border width, not tuned.
pub fn build_row_list_scene(card: &RowListCard<'_>, metrics: &BakedFontMetrics) -> Scene {
    let rows = [
        (card.row0_title, card.row0_time),
        (card.row1_title, card.row1_time),
        (card.row2_title, card.row2_time),
        (card.row3_title, card.row3_time),
        (card.row4_title, card.row4_time),
    ];
    let shown = rows.iter().filter(|(title, _)| !title.is_empty()).count();
    let mut nodes = Vec::with_capacity(23);

    nodes.push(SceneNode::Label(SceneLabel {
        x: MARGIN,
        y: 2 * GRID,
        horizontal_anchor: SceneLabelAnchor::Left,
        font: SceneFont::Baked(SceneFontTier::Caption),
        value: SceneValue::Literal(card.title.to_string()),
        ink: ROW_LIST_INK,
        fill: ROW_LIST_HUE,
        fill_opacity: u8::MAX,
        radius: RADIUS_CHIP,
        pad_hor: 2 * GRID,
        pad_ver: (CHIP_HEIGHT - metrics.tier(SceneFontTier::Caption).line_height) / 2,
        letter_space: 1,
        hide_when_empty: true,
    }));

    if shown > 0 {
        nodes.push(SceneNode::Arc(SceneArc {
            cx: ROW_COUNT_X + ROW_COUNT_BOX / 2,
            cy: 2 * GRID + ROW_COUNT_BOX / 2,
            r: ROW_COUNT_BOX / 2,
            start_deg: 270,
            end_deg: 630,
            width: ROW_COUNT_BORDER_WIDTH,
            color: ROW_LIST_HUE,
            rounded: false,
            end_binding: String::new(),
        }));

        let count_pad_top = (ROW_COUNT_BOX - metrics.tier(SceneFontTier::Caption).line_height) / 2;
        nodes.push(SceneNode::Text(SceneText {
            x: ROW_COUNT_X,
            baseline_y: 2 * GRID
                + ROW_COUNT_BORDER_WIDTH
                + count_pad_top
                + metrics.baseline_offset(SceneFontTier::Caption),
            w: ROW_COUNT_BOX,
            align: SceneAlign::Center,
            font: SceneFont::Baked(SceneFontTier::Caption),
            color: ROW_LIST_HUE,
            value: SceneValue::Literal(shown.to_string()),
            ellipsize: false,
        }));

        for (row, (title, time)) in rows.into_iter().enumerate() {
            push_row_nodes(&mut nodes, row, title, time, metrics);
        }
    } else {
        nodes.push(SceneNode::Rect(SceneRect {
            x: ROW_X,
            y: ROW_FIRST_Y,
            w: ROW_WIDTH,
            h: ROW_HEIGHT,
            radius: 2 * GRID,
            fill: COLOR_SURFACE,
            opacity: u8::MAX,
        }));
        let text_y = (ROW_HEIGHT - metrics.tier(SceneFontTier::Body).line_height) / 2;
        let (text_x, text_top) = row_child_origin(ROW_FIRST_Y, ROW_TIME_X, text_y);
        nodes.push(SceneNode::Text(SceneText {
            x: text_x,
            baseline_y: text_top + metrics.baseline_offset(SceneFontTier::Body),
            w: ROW_WIDTH - 2 * ROW_TIME_X,
            align: SceneAlign::Left,
            font: SceneFont::Baked(SceneFontTier::Body),
            color: COLOR_TERTIARY,
            value: SceneValue::Literal("Nothing to show".to_string()),
            ellipsize: true,
        }));
    }

    Scene {
        revision: card.revision,
        background: COLOR_CANVAS,
        nodes,
    }
}

/// `progress_ring.c`'s `format_clock()`, including its `[0, 86400]` clamp and
/// total-minutes representation rather than an hours field.
fn format_progress_clock(seconds: i64) -> String {
    let seconds = seconds.clamp(0, PROGRESS_CLOCK_SECONDS_MAX);
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

/// `progress_ring.c`'s `status_word()`.
fn progress_status_word(
    running: bool,
    duration_seconds: i64,
    remaining_seconds: i64,
) -> &'static str {
    if remaining_seconds <= 0 {
        "Done"
    } else if running {
        "Running"
    } else if duration_seconds > 0 && remaining_seconds >= duration_seconds {
        "Ready"
    } else {
        "Paused"
    }
}

fn push_progress_module(
    nodes: &mut Vec<SceneNode>,
    y: i32,
    caption: &str,
    value: String,
    value_color: u32,
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
        value: SceneValue::Literal(value),
        ellipsize: true,
    }));
}

/// Builds the whole `ProgressRing` face as a scene.
///
/// The track and indicator are two scene arcs because `SceneArc` is one
/// opaque stroke while the C `lv_arc` draws both parts. The indicator and
/// countdown remain live through `timer.pct` and `timer.remaining:mm:ss`.
/// The track's solid colour is the exact fully-covered RGB565 result, but its
/// anti-aliased edge remains a recorded parity gap until the arc node exposes
/// the opacity LVGL applies before its mask blend.
pub fn build_progress_ring_scene(card: &ProgressRingCard<'_>, metrics: &BakedFontMetrics) -> Scene {
    // Mirror progress_ring_patch()'s defensive bounds even though the field
    // registry already applies them on device. Preview and simulator callers
    // bypass that registry, and the scene builder must agree with both paths.
    let duration_seconds = card.duration_seconds.clamp(0, PROGRESS_CLOCK_SECONDS_MAX);
    let remaining_seconds = card
        .remaining_seconds
        .clamp(0, PROGRESS_CLOCK_SECONDS_MAX)
        .min(duration_seconds);

    let mut nodes = Vec::with_capacity(13);
    for (color, rounded, end_binding) in [
        (PROGRESS_RING_TRACK, false, String::new()),
        (
            if card.running {
                PROGRESS_RING_HUE
            } else {
                COLOR_TERTIARY
            },
            true,
            "timer.pct".to_string(),
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
        value: SceneValue::Binding("timer.remaining:mm:ss".to_string()),
        ellipsize: true,
    }));

    // Live-update gap (Task 7): SceneValue has no conditional or arithmetic
    // form, so STATUS, TOTAL and ELAPSED are literals from the build instant.
    // They reproduce this frame but cannot follow the timer between pushes.
    let module_values = if duration_seconds >= 1 {
        (
            format_progress_clock(duration_seconds),
            format_progress_clock(duration_seconds - remaining_seconds),
            progress_status_word(card.running, duration_seconds, remaining_seconds).to_string(),
        )
    } else {
        (String::new(), String::new(), String::new())
    };
    push_progress_module(
        &mut nodes,
        PROGRESS_CHIP_FIRST_Y,
        "TOTAL",
        module_values.0,
        COLOR_PRIMARY,
        metrics,
    );
    push_progress_module(
        &mut nodes,
        PROGRESS_CHIP_FIRST_Y + PROGRESS_CHIP_H + PROGRESS_CHIP_GAP,
        "ELAPSED",
        module_values.1,
        PROGRESS_RING_TINT,
        metrics,
    );
    push_progress_module(
        &mut nodes,
        PROGRESS_CHIP_FIRST_Y + 2 * (PROGRESS_CHIP_H + PROGRESS_CHIP_GAP),
        "STATUS",
        module_values.2,
        if card.running {
            PROGRESS_RING_HUE
        } else {
            COLOR_PRIMARY
        },
        metrics,
    );

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

    fn progress_module_text_nodes(scene: &Scene) -> Vec<SceneNode> {
        scene
            .nodes
            .iter()
            .filter(|node| match node {
                SceneNode::Label(label) => label.x == PROGRESS_CHIP_X + PROGRESS_CHIP_PAD,
                SceneNode::Text(text) => text.x == PROGRESS_CHIP_X + PROGRESS_CHIP_PAD,
                _ => false,
            })
            .cloned()
            .collect()
    }

    fn progress_module_text_values(scene: &Scene) -> Vec<&str> {
        scene
            .nodes
            .iter()
            .filter_map(|node| match node {
                SceneNode::Label(label) if label.x == PROGRESS_CHIP_X + PROGRESS_CHIP_PAD => {
                    match &label.value {
                        SceneValue::Literal(value) => Some(value.as_str()),
                        SceneValue::Binding(_) => panic!("a progress module label must be literal"),
                    }
                }
                SceneNode::Text(text) if text.x == PROGRESS_CHIP_X + PROGRESS_CHIP_PAD => {
                    match &text.value {
                        SceneValue::Literal(value) => Some(value.as_str()),
                        SceneValue::Binding(_) => panic!("a progress module value must be literal"),
                    }
                }
                _ => None,
            })
            .collect()
    }

    /// Passing is the defect this test pins, and it pins it by showing a
    /// divergence rather than by restating that the builder is a pure
    /// function of its card.
    ///
    /// A running timer pushed once at `t` is still on the panel at `t + 90s`,
    /// because a scene ticks on the device without a re-push -- that is the
    /// whole point of the bindings. The ring and the countdown do follow, via
    /// `timer.pct` and `timer.remaining:mm:ss`. The three module chips cannot:
    /// `SceneValue` has no conditional and no arithmetic, so `status_word()`'s
    /// choice and the ELAPSED subtraction are frozen at the build instant.
    ///
    /// So the assertion is two-sided. The scene's chips are unchanged after 90
    /// seconds, and the C face -- whose `refresh_modules()` recomputes them
    /// every tick from `current_remaining_ms()` -- would by then be reading
    /// something else. The second half is what makes this a proof: without it
    /// the test would hold just as well for a card whose text was genuinely
    /// still correct.
    ///
    /// Task 7 must close this before stage 3 can retire the C template.
    #[test]
    fn progress_ring_scene_status_text_does_not_advance_with_time() {
        const DURATION: i64 = 1_500;
        const REMAINING_AT_T: i64 = 90;
        const ELAPSED_SECONDS: i64 = 90;

        // `Running` with 90s left at `t`; `Done` with 0s left 90 seconds later.
        // The state transition is deliberate: a stale word is easiest to
        // dismiss as a rounding artifact and hardest to dismiss as a wrong
        // word.
        let card = ProgressRingCard {
            revision: 7,
            label: "Pomodoro",
            duration_seconds: DURATION,
            remaining_seconds: REMAINING_AT_T,
            running: true,
        };
        let pushed_at_t = build_progress_ring_scene(&card, &BakedFontMetrics::SHIPPED);

        // What the device still shows 90 seconds later, having received
        // nothing further: the same scene, so the same literals.
        let still_on_the_panel = build_progress_ring_scene(&card, &BakedFontMetrics::SHIPPED);
        assert_eq!(
            progress_module_text_nodes(&pushed_at_t),
            progress_module_text_nodes(&still_on_the_panel),
            "a scene carries literals, so nothing about the chips can have moved"
        );

        // What `progress_ring.c` would be showing at that moment, computed
        // through the very helpers the builder used -- so this cannot drift
        // from the C independently of the builder.
        let remaining_at_t_plus_90 = REMAINING_AT_T - ELAPSED_SECONDS;
        let live = [
            format_progress_clock(DURATION),
            format_progress_clock(DURATION - remaining_at_t_plus_90),
            progress_status_word(card.running, DURATION, remaining_at_t_plus_90).to_string(),
        ];
        let frozen = progress_module_text_values(&pushed_at_t);

        // TOTAL is the one chip that is honestly constant.
        assert_eq!(frozen[1], live[0], "TOTAL does not depend on the clock");
        assert_ne!(
            frozen[3], live[1],
            "ELAPSED is frozen at the build instant: the panel reads {} where the C reads {}",
            frozen[3], live[1]
        );
        assert_ne!(
            frozen[5], live[2],
            "STATUS is frozen at the build instant: the panel reads {} where the C reads {}",
            frozen[5], live[2]
        );
    }

    #[test]
    fn progress_ring_scene_clamps_duration_and_remaining_like_the_c_patch() {
        let above_max = build_progress_ring_scene(
            &ProgressRingCard {
                revision: 1,
                label: "Pomodoro",
                duration_seconds: 90_000,
                remaining_seconds: 100_000,
                running: false,
            },
            &BakedFontMetrics::SHIPPED,
        );
        assert_eq!(
            progress_module_text_values(&above_max),
            ["TOTAL", "1440:00", "ELAPSED", "00:00", "STATUS", "Ready"]
        );

        let below_zero = build_progress_ring_scene(
            &ProgressRingCard {
                revision: 1,
                label: "Pomodoro",
                duration_seconds: 100,
                remaining_seconds: -1,
                running: false,
            },
            &BakedFontMetrics::SHIPPED,
        );
        assert_eq!(
            progress_module_text_values(&below_zero),
            ["TOTAL", "01:40", "ELAPSED", "01:40", "STATUS", "Done"]
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
        // as a surprise at 2034. 283 of 2034 is 14% of one envelope, which is
        // the headroom that lets the plan refuse a second chunk protocol.
        assert_eq!(283, payload.len(), "encoded PushScene payload");
    }
}
