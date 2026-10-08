//! Host-side scene builders for Deskmate's card faces: `DigitalClock`,
//! `AnalogClock` and `ProgressRing`.
//!
//! Layout is pinned by `crates/lvgl-sim/tests/golden/`, which renders these
//! scenes through the real firmware scene interpreter.
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
//! The digital face combines canvas coordinates, module-relative geometry,
//! and scale-local hand pivots. A scene has no nesting — every node is a
//! sibling under one full-canvas container — so this module resolves each
//! chain to absolute canvas coordinates. `hand_line()` is where that matters
//! most.

use protocol::{
    SCENE_CANVAS_HEIGHT, SCENE_CANVAS_WIDTH, Scene, SceneAlign, SceneArc, SceneClipRect, SceneFont,
    SceneFontTier, SceneLabel, SceneLabelAnchor, SceneLine, SceneNode, SceneRect, SceneRotRect,
    SceneScale, SceneText, SceneValue,
};

// ---------------------------------------------------------------------------
// Shared design-system constants.
// ---------------------------------------------------------------------------

const METRICS: &BakedFontMetrics = &BakedFontMetrics::SHIPPED;

const GRID: i32 = 8;
const MARGIN: i32 = 3 * GRID;
const RADIUS_MODULE: i32 = 3 * GRID;

const COLOR_CANVAS: u32 = 0x0000_0000;
const COLOR_PRIMARY: u32 = 0x00f5_f5f7;
const COLOR_TERTIARY: u32 = 0x005c_5c66;
const COLOR_SURFACE: u32 = 0x001a_1a1f;
/// Reserved for the stale-picture footer.
const COLOR_STALE: u32 = 0x00f2_c94c;
/// LVGL's `LV_OPA_20` constant.
const OPACITY_20_PERCENT: u8 = 51;
/// Both clock faces share this accent hue.
const CLOCK_HUE: u32 = 0x00ff_8f2e;
const PROGRESS_RING_HUE: u32 = 0x00ff_5a3d;
const PROGRESS_RING_TINT: u32 = 0x00ff_9a85;
const COLOR_SECONDARY: u32 = 0x009a_9aa5;

// ---------------------------------------------------------------------------
// Analog-clock geometry.
// ---------------------------------------------------------------------------

const ANALOG_FACE_DIAMETER: i32 = SCENE_CANVAS_HEIGHT - 2 * MARGIN;
const ANALOG_FACE_RADIUS: i32 = ANALOG_FACE_DIAMETER / 2;
const ANALOG_CHAPTER_RING_WIDTH: i32 = 4 * GRID;
const ANALOG_TICK_MAJOR_LENGTH: i32 = 2 * GRID;
const ANALOG_TICK_MINOR_LENGTH: i32 = GRID;
const ANALOG_TICK_MAJOR_WIDTH: i32 = 4;
const ANALOG_TICK_MINOR_WIDTH: i32 = 3;
const ANALOG_HOUR_LENGTH: i32 = 13 * GRID;
const ANALOG_MINUTE_LENGTH: i32 = 18 * GRID;
const ANALOG_SECOND_LENGTH: i32 = 19 * GRID;
const ANALOG_HOUR_WIDTH: i32 = 6;
const ANALOG_MINUTE_WIDTH: i32 = 4;
const ANALOG_SECOND_WIDTH: i32 = 2;
const ANALOG_HUB_DIAMETER: i32 = 2 * GRID;

// ---------------------------------------------------------------------------
// Progress-ring geometry.
// ---------------------------------------------------------------------------

const PROGRESS_RING_DIAMETER: i32 = 30 * GRID;
const PROGRESS_RING_X: i32 = 2 * GRID;
const PROGRESS_RING_Y: i32 = 8 * GRID;
const PROGRESS_RING_CENTER_X: i32 = PROGRESS_RING_X + PROGRESS_RING_DIAMETER / 2;
const PROGRESS_RING_CENTER_Y: i32 = PROGRESS_RING_Y + PROGRESS_RING_DIAMETER / 2;
const PROGRESS_RING_WIDTH: i32 = 26;
const PROGRESS_CHIP_X: i32 = 34 * GRID;
const PROGRESS_CHIP_W: i32 = 19 * GRID;
const PROGRESS_CHIP_H: i32 = 10 * GRID;
const PROGRESS_CHIP_GAP: i32 = 2 * GRID;
const PROGRESS_CHIP_FIRST_Y: i32 = 6 * GRID;
const PROGRESS_CHIP_PAD: i32 = 2 * GRID;
const PROGRESS_RING_TEXT_W: i32 = 25 * GRID;
const PROGRESS_RING_TEXT_X: i32 = PROGRESS_RING_CENTER_X - PROGRESS_RING_TEXT_W / 2;

// ---------------------------------------------------------------------------
// Digital-clock geometry.
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
const HAND_HOUR_WIDTH: i32 = 6;
const HAND_MINUTE_WIDTH: i32 = 4;
/// One tick per hour, with the thirteenth landing back on twelve.
const DIAL_TOTAL_TICKS: u32 = 13;
/// Every third tick is major.
const DIAL_MAJOR_TICK_EVERY: u32 = 3;
/// Gap between the hero reading and its seconds suffix.
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
struct NumericAdvances {
    digit: i32,
    colon: i32,
    hyphen: i32,
    percent: i32,
    degree: i32,
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
pub(crate) struct TierMetrics {
    /// `lv_font_get_line_height(font)`.
    pub(crate) line_height: i32,
    /// `font->base_line`, measured up from the bottom of the line box.
    pub(crate) base_line: i32,
    /// `None` for a full-range tier, whose advances this table deliberately
    /// does not reproduce. A partial table that looked complete would be
    /// worse than no table.
    numeric: Option<NumericAdvances>,
}

/// The four faces `tools/genfonts.sh` bakes, addressed by scene font tier.
///
/// The numbers are transcribed from `firmware/main/ui/fonts/deskmate_font_*.c`
/// — the same bytes the device links — and a test re-reads those files and
/// fails on any disagreement. That test is the whole provenance argument: a
/// baseline offset invented here would be invisible to every other check and
/// would surface only as a failed pixel diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BakedFontMetrics {
    caption: TierMetrics,
    body: TierMetrics,
    display: TierMetrics,
    hero: TierMetrics,
}

impl BakedFontMetrics {
    /// The metrics of the faces this repository ships.
    pub(crate) const SHIPPED: Self = Self {
        // deskmate_font_18.c — full range, so no numeric table.
        caption: TierMetrics {
            line_height: 22,
            base_line: 4,
            numeric: None,
        },
        // deskmate_font_28.c — full range, like Caption, so no numeric table.
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

    pub(crate) fn tier(&self, tier: SceneFontTier) -> TierMetrics {
        match tier {
            SceneFontTier::Caption => self.caption,
            SceneFontTier::Body => self.body,
            SceneFontTier::Display => self.display,
            SceneFontTier::Hero => self.hero,
        }
    }

    /// Distance from a label box's top edge to the baseline of the type in it.
    ///
    /// `scene_view.c`'s `baseline_box_top()` is the exact inverse, so a text
    /// node's `baseline_y` must be `box_top + baseline_offset(tier)`.
    pub(crate) fn baseline_offset(&self, tier: SceneFontTier) -> i32 {
        let metrics = self.tier(tier);
        metrics.line_height - metrics.base_line
    }

    /// `lv_text_get_size()`'s width for one line of `text` at `tier`, with
    /// `letter_space` 0.
    ///
    /// Returns `None` when the tier carries no numeric table, or when `text`
    /// steps outside the subset — the two cases where this side must not
    /// pretend to know what the device would measure.
    fn measure(&self, tier: SceneFontTier, text: &str) -> Option<i32> {
        let numeric = self.tier(tier).numeric?;
        // lv_text_get_width sums the advances and trims one trailing
        // letter_space; at letter_space 0 that is a plain sum.
        text.chars().try_fold(0, |width, character| {
            numeric.advance(character).map(|advance| width + advance)
        })
    }
}

// ---------------------------------------------------------------------------
// The builder.
// ---------------------------------------------------------------------------

/// Inputs needed to build a digital-clock scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockCard {
    /// Echoed as the scene's own revision.
    pub revision: u32,
    /// False drops the seconds node without moving anything else because the
    /// hero is left-anchored.
    pub show_seconds: bool,
}

/// Inputs needed to build an analog-clock scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalogClockCard {
    /// Echoed as the scene's own revision.
    pub revision: u32,
    /// False drops the second hand without moving any other part of the face.
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
    pub label: &'a str,
    /// Selects the duration-zero initial state. For a positive duration,
    /// TOTAL, ELAPSED, STATUS, and the countdown are live bindings; for a
    /// non-positive duration they retain their constructed text.
    pub duration_seconds: i64,
}

/// Adds the host-owned stale marker to a picture face.
///
/// Staleness is not a device binding: the host rebuilds the scene when the
/// source's freshness changes. A fresh picture keeps its single image node.
pub(crate) fn with_stale_footer(mut scene: Scene, stale: bool) -> Scene {
    if !stale {
        return scene;
    }
    let caption = METRICS.tier(SceneFontTier::Caption);
    scene.nodes.push(SceneNode::Text(SceneText {
        // A content-sized bottom-centred label computes `parent_w / 2 -
        // label_w / 2`; a full-width text box computes
        // `(parent_w - text_w) / 2`. Those differ by one pixel when the text
        // width is odd, so use the 1..448 box that preserves LVGL's separate
        // integer halves.
        x: 1,
        // The footer sits 2*GRID above the bottom. Its line box is
        // content-height, so line height cancels out of the baseline.
        baseline_y: SCENE_CANVAS_HEIGHT - 2 * GRID - caption.base_line,
        w: SCENE_CANVAS_WIDTH - 1,
        align: SceneAlign::Center,
        font: SceneFont::Baked(SceneFontTier::Caption),
        color: COLOR_STALE,
        running_color: None,
        value: SceneValue::Literal("Stale".to_string()),
        ellipsize: false,
    }));
    scene
}

fn finish_scene(revision: u32, nodes: Vec<SceneNode>) -> Scene {
    Scene {
        revision,
        background: COLOR_CANVAS,
        nodes,
    }
}

/// One live clock hand in absolute canvas coordinates.
///
/// The pivot is translated out of the scale's local frame here. The endpoint
/// is deliberately not: the device recomputes it with LVGL's own trig table on
/// every binding refresh.
///
/// LVGL clamps a scale needle's length to half the scale box and clips its
/// stroke; a scene line has neither behavior. Pixel parity therefore requires
/// `length + width / 2 < DIAL_BOX / 2`.
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
/// Node order is z-order: reading, seconds, date module and label, dial module,
/// dial, then the two hands. Clock faces have no footer node.
pub fn build_digital_clock_scene(card: &ClockCard) -> Scene {
    let mut nodes = Vec::with_capacity(8);

    // The width is deliberately generous: the visible reading is
    // content-sized while a scene text node is a fixed box, and an exactly
    // measured box risks a
    // one-pixel clip if the device's measurement differs. LEFT alignment
    // starts at the box edge and LONG_MODE_CLIP suppresses wrapping, so extra
    // width costs nothing.
    let hero_baseline = TIME_Y + METRICS.baseline_offset(SceneFontTier::Hero);
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

    // The hero is content-sized, so seconds sit one 2*GRID gap to its right,
    // with a y
    // offset of baseline_offset(HERO) - baseline_offset(DISPLAY). That offset
    // exists to put the two on one baseline, so in a baseline-addressed scene
    // it collapses to "the same baseline_y". The x has to be computed, and it
    // is safe to compute once because every digit in the HERO subset has the
    // same advance -- so "HH:mm" measures the same width at every minute of
    // the day. The font provenance test pins every digit advance.
    if card.show_seconds {
        let hero_width = METRICS
            .measure(SceneFontTier::Hero, "00:00")
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

    // The date module and its one label.
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
    let value_line = METRICS.tier(SceneFontTier::Body).line_height;
    let stack_top = (MODULE_H - value_line) / 2;
    nodes.push(SceneNode::Text(SceneText {
        // The label has one margin inside a module that itself starts at the
        // canvas margin.
        x: MARGIN + MARGIN,
        baseline_y: MODULE_Y + stack_top + METRICS.baseline_offset(SceneFontTier::Body),
        // This box is fixed on both sides; a wider one would ellipsize
        // differently.
        w: DATE_W - 2 * MARGIN,
        align: SceneAlign::Left,
        font: SceneFont::Baked(SceneFontTier::Body),
        color: COLOR_PRIMARY,
        running_color: None,
        value: SceneValue::Binding(DATE_BINDING.to_string()),
        // The bounded label ellipsizes overflowing dates.
        ellipsize: true,
    }));

    // The dial module and dial.
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
        // fixed tertiary grey, which scene_view.c owns.
        major_tick_color: CLOCK_HUE,
    }));

    // The two hands use live angle bindings.
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

/// Converts the centre of a child in the clock face's local coordinate frame to
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
        // The twelve ticks share the face's clipping box. A scene display
        // list is flat, so carry that parent clip
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
        // A circular radius clamps to half the 16px square.
        radius: ANALOG_HUB_DIAMETER / 2,
        fill: CLOCK_HUE,
        opacity: u8::MAX,
        clip: None,
    }));

    // The OK state has no visible footer node.
    finish_scene(card.revision, nodes)
}

/// Adds one of the progress ring's three fixed module stacks. The caption is
/// static; the value may be a live timer binding.
fn push_progress_module(
    nodes: &mut Vec<SceneNode>,
    y: i32,
    caption: &str,
    value: SceneValue,
    value_color: u32,
    running_color: Option<u32>,
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

    let eyebrow_line = METRICS.tier(SceneFontTier::Caption).line_height;
    let value_line = METRICS.tier(SceneFontTier::Body).line_height;
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
            + METRICS.baseline_offset(SceneFontTier::Body),
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
/// stroke, so track and indicator are separate nodes. The track carries the
/// palette hue at `LV_OPA_20`, preserving LVGL's opacity-before-mask blend, while the
/// indicator and countdown remain live through `timer.permille` and
/// `timer.remaining:mm:ss`.
pub fn build_progress_ring_scene(card: &ProgressRingCard<'_>) -> Scene {
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

    let label_line = METRICS.tier(SceneFontTier::Caption).line_height;
    let time_line = METRICS.tier(SceneFontTier::Display).line_height;
    let pair_top = PROGRESS_RING_CENTER_Y - (label_line + GRID + time_line) / 2;
    nodes.push(SceneNode::Text(SceneText {
        x: PROGRESS_RING_TEXT_X,
        baseline_y: pair_top + METRICS.baseline_offset(SceneFontTier::Caption),
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
        baseline_y: pair_top + label_line + GRID + METRICS.baseline_offset(SceneFontTier::Display),
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
    );
    push_progress_module(
        &mut nodes,
        PROGRESS_CHIP_FIRST_Y + PROGRESS_CHIP_H + PROGRESS_CHIP_GAP,
        "ELAPSED",
        progress_timer_value(timer_active, "timer.elapsed:mm:ss", ""),
        PROGRESS_RING_TINT,
        None,
    );
    push_progress_module(
        &mut nodes,
        PROGRESS_CHIP_FIRST_Y + 2 * (PROGRESS_CHIP_H + PROGRESS_CHIP_GAP),
        "STATUS",
        progress_timer_value(timer_active, "timer.status", ""),
        COLOR_PRIMARY,
        Some(PROGRESS_RING_HUE),
    );

    finish_scene(card.revision, nodes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::{
        MAX_PAYLOAD_SIZE, Message, PushScene, SceneAlign, SceneFont, SceneLine, SceneNode,
        SceneRect, SceneScale, SceneText, SceneValue, decode_wire_frame, encode_message,
        validate_scene,
    };

    // ---------------------------------------------------------------- fixtures

    fn scene(show_seconds: bool) -> Scene {
        build_digital_clock_scene(&ClockCard {
            revision: 7,
            show_seconds,
        })
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
    /// likely cause of text landing off its baseline on the panel (and of a
    /// framebuffer-matrix mismatch), so this test reads the shipped baked font
    /// sources -- the exact bytes `lv_font_get_line_height()` and
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
    fn the_digital_clock_scene_has_the_expected_z_order() {
        let scene = scene(true);
        // Reading, seconds, date module and label, dial module and scale, then
        // hour and minute hands. The OK state draws no footer.
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
        assert_eq!(0x0000_0000, scene.background);
    }

    #[test]
    fn the_stale_footer_is_one_yellow_caption_line() {
        let ok = scene(true);
        let stale = with_stale_footer(ok.clone(), true);
        assert_eq!(with_stale_footer(ok.clone(), false), ok);

        assert_eq!(stale.nodes.len(), ok.nodes.len() + 1);
        let stale_footer = text(&stale, stale.nodes.len() - 1);
        assert_eq!(SceneValue::Literal("Stale".to_string()), stale_footer.value);
        assert_eq!(COLOR_STALE, stale_footer.color);
        assert_eq!(1, stale_footer.x);
        assert_eq!(SCENE_CANVAS_WIDTH - 1, stale_footer.w);
        assert_eq!(348, stale_footer.baseline_y);
        assert_eq!(SceneAlign::Center, stale_footer.align);
        assert_eq!(SceneFont::Baked(SceneFontTier::Caption), stale_footer.font);
    }

    #[test]
    fn the_hero_reading_uses_the_shipped_baseline() {
        let scene = scene(true);
        let hero = text(&scene, 0);
        // MARGIN = 3 * 8 = 24 and TIME_Y = 8 * 8 = 64, plus
        // baseline_offset(HERO) = line_height 72 - base_line 1 = 71.
        assert_eq!(24, hero.x);
        assert_eq!(64 + 71, hero.baseline_y);
        assert_eq!(SceneFont::Baked(SceneFontTier::Hero), hero.font);
        assert_eq!(SceneAlign::Left, hero.align);
        assert_eq!(0x00f5_f5f7, hero.color);
        assert_eq!(SceneValue::Binding("time:HH:mm".to_string()), hero.value);
        assert!(!hero.ellipsize);
        // Generous by design: the reading is 279px while the scene text node
        // is a fixed box. 424 runs to the right canvas edge and avoids a
        // one-pixel clip if measurement changes.
        assert_eq!(424, hero.w);
    }

    #[test]
    fn the_seconds_share_the_heros_baseline_and_sit_one_gap_right() {
        let scene = scene(true);
        let hero = text(&scene, 0);
        let seconds = text(&scene, 1);
        // Seconds follow the content-sized hero with a 2 * GRID gap and a y offset of
        // baseline_offset(HERO) - baseline_offset(DISPLAY), which puts both on
        // one baseline. "00:00" in HERO is 4 * 62 + 31 = 279px.
        assert_eq!(MARGIN + 279 + SECONDS_GAP, seconds.x);
        assert_eq!(hero.baseline_y, seconds.baseline_y);
        assert_eq!(SceneFont::Baked(SceneFontTier::Display), seconds.font);
        // The face's accent hue.
        assert_eq!(0x00ff_8f2e, seconds.color);
        assert_eq!(SceneValue::Binding("time:ss".to_string()), seconds.value);
        assert!(!seconds.ellipsize);
        assert_eq!(448 - (24 + 279 + 16), seconds.w);
    }

    #[test]
    fn the_two_modules_land_on_the_design_grid() {
        let scene = scene(true);
        // MODULE_Y = 22 * 8, DATE_W = 28 * 8, MODULE_H = 17 * 8, and
        // RADIUS_MODULE = 3 * 8.
        let date_module = rect(&scene, 2);
        assert_eq!(24, date_module.x);
        assert_eq!(176, date_module.y);
        assert_eq!(224, date_module.w);
        assert_eq!(136, date_module.h);
        assert_eq!(24, date_module.radius);
        // Opaque surface color.
        assert_eq!(0x001a_1a1f, date_module.fill);
        assert_eq!(255, date_module.opacity);

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
    fn the_date_box_is_centered_inside_its_module() {
        let scene = scene(true);
        let date = text(&scene, 3);
        // stack_top = (MODULE_H - line_height(BODY)) / 2 = (136 - 36) / 2
        // = 50, so the box top is 176 + 50 = 226 and the baseline is that plus
        // baseline_offset(BODY) = 36 - 7 = 29.
        assert_eq!(24 + 24, date.x);
        assert_eq!(226 + 29, date.baseline_y);
        assert_eq!(224 - 48, date.w);
        assert_eq!(SceneFont::Baked(SceneFontTier::Body), date.font);
        assert_eq!(SceneAlign::Left, date.align);
        assert_eq!(0x00f5_f5f7, date.color);
        // Long dates are ellipsized within the module.
        assert!(date.ellipsize);
        assert_eq!(SceneValue::Binding("date".to_string()), date.value);
    }

    #[test]
    fn the_dial_is_a_scale_node_at_the_module_centre() {
        let scene = scene(true);
        let dial = scale(&scene, 5);
        // DIAL_BOX = 14 * 8 = 112, centered inside the dial module whose
        // origin is (DIAL_X, MODULE_Y).
        assert_eq!(264 + 24, dial.x);
        assert_eq!(176 + 12, dial.y);
        assert_eq!(112, dial.box_size);
        assert_eq!(13, dial.total_tick_count);
        assert_eq!(3, dial.major_tick_every);
        assert_eq!(0x00ff_8f2e, dial.major_tick_color);
    }

    #[test]
    fn the_hands_are_bound_at_the_scales_canvas_space_pivot() {
        let scene = scene(true);
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
    fn hiding_the_seconds_drops_the_node_rather_than_moving_anything() {
        let with = scene(true);
        let without = scene(false);
        assert_eq!(7, without.nodes.len());
        // The hero is left-anchored, so hiding the seconds moves nothing.
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

    // ------------------------------------------------------- shipped metrics

    #[test]
    fn numeric_metrics_cover_the_baked_subset() {
        let metrics = &BakedFontMetrics::SHIPPED;
        assert!(metrics.measure(SceneFontTier::Hero, "00:00").is_some());
        assert!(metrics.measure(SceneFontTier::Hero, "-12%").is_some());
        assert!(metrics.measure(SceneFontTier::Hero, "21\u{b0}").is_some());
        assert!(metrics.measure(SceneFontTier::Hero, "12.5").is_none());
        assert!(metrics.measure(SceneFontTier::Hero, "Mon").is_none());
    }

    #[test]
    fn hero_and_display_metrics_preserve_expected_widths() {
        let metrics = &BakedFontMetrics::SHIPPED;
        assert_eq!(Some(279), metrics.measure(SceneFontTier::Hero, "00:00"));
        assert_eq!(Some(162), metrics.measure(SceneFontTier::Display, "00:00"));
    }

    #[test]
    fn the_hero_reading_fits_its_available_width() {
        let width = BakedFontMetrics::SHIPPED
            .measure(SceneFontTier::Hero, "00:00")
            .unwrap();
        assert!(width <= SCENE_CANVAS_WIDTH - 2 * MARGIN);
    }

    #[test]
    fn progress_ring_without_a_duration_keeps_the_offline_preview_values() {
        let scene = build_progress_ring_scene(&ProgressRingCard {
            revision: 1,
            label: "Pomodoro",
            duration_seconds: 0,
        });

        assert_eq!(text(&scene, 3).value, SceneValue::Literal("00:00".into()));
        for index in [6, 9, 12] {
            assert_eq!(
                text(&scene, index).value,
                SceneValue::Literal(String::new())
            );
        }
    }

    #[test]
    fn progress_ring_scene_uses_live_timer_bindings() {
        let scene = build_progress_ring_scene(&ProgressRingCard {
            revision: 1,
            label: "Pomodoro",
            duration_seconds: 1_500,
        });
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
        let scene = scene(true);
        validate_scene(&scene).expect("the builder emits a scene the device accepts");

        let wire = encode_message(
            1,
            &Message::PushScene(PushScene {
                tap_views: Vec::new(),
                tap_wrap: true,
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
