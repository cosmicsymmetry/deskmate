//! `digital_clock.c`'s layout, moved to the host as a scene.
//!
//! Stage 2a's parity gate renders the shipped `DigitalClock` C template and the
//! scene this module emits through the same simulator and asserts the two
//! framebuffers are **byte-identical**. Every constant here is therefore a
//! *port* of `firmware/main/ui/templates/digital_clock.c`, not a re-derivation:
//! read that file before changing a number here, and change the number there
//! first if the layout is genuinely meant to move.

use chrono::NaiveDateTime;
use protocol::{Scene, SceneFontTier};

// -------------------------------------------------------------------------
// Stubs. Replaced by the implementation commit.
// -------------------------------------------------------------------------

/// The card-level inputs `digital_clock.c` draws from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockCard {
    /// Echoed as the scene's own revision.
    pub revision: u32,
    pub show_seconds: bool,
    /// The local wall-clock instant the dial's hands and the date module are
    /// drawn for. The reading itself is a device-side `time:` binding, so it
    /// ticks between pushes; the hands and the date do not.
    pub local_now: NaiveDateTime,
}

/// Whole-pixel advance widths for the glyph classes the digits-only
/// `DISPLAY`/`HERO` subsets carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumericAdvances {
    pub digit: i32,
    pub colon: i32,
    pub hyphen: i32,
    pub percent: i32,
    pub degree: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TierMetrics {
    pub line_height: i32,
    pub base_line: i32,
    pub numeric: Option<NumericAdvances>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BakedFontMetrics {
    pub caption: TierMetrics,
    pub body: TierMetrics,
    pub display: TierMetrics,
    pub hero: TierMetrics,
}

impl BakedFontMetrics {
    pub const SHIPPED: Self = Self {
        caption: TierMetrics {
            line_height: 0,
            base_line: 0,
            numeric: None,
        },
        body: TierMetrics {
            line_height: 0,
            base_line: 0,
            numeric: None,
        },
        display: TierMetrics {
            line_height: 0,
            base_line: 0,
            numeric: None,
        },
        hero: TierMetrics {
            line_height: 0,
            base_line: 0,
            numeric: None,
        },
    };

    pub fn tier(&self, _tier: SceneFontTier) -> TierMetrics {
        self.caption
    }

    pub fn baseline_offset(&self, _tier: SceneFontTier) -> i32 {
        0
    }

    pub fn measure(&self, _tier: SceneFontTier, _text: &str) -> Option<i32> {
        None
    }
}

pub fn text_is_numeric(_text: &str) -> bool {
    false
}

pub fn number_font_tier(
    _text: &str,
    _max_width: i32,
    _metrics: &BakedFontMetrics,
) -> SceneFontTier {
    SceneFontTier::Body
}

pub fn build_digital_clock_scene(_card: &ClockCard, _metrics: &BakedFontMetrics) -> Scene {
    Scene::default()
}

fn trigo_sin(_angle: i32) -> i32 {
    0
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
            // glyph_id_start is 1 in every baked subset, and glyph_dsc[0] is
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
        // The C normalises with while-loops, so out-of-turn angles wrap.
        assert_eq!(trigo_sin(30), trigo_sin(390));
        assert_eq!(trigo_sin(330), trigo_sin(-30));
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
        let payload = decode_wire_frame(&wire).expect("a well-formed frame").payload;
        assert!(payload.len() <= MAX_PAYLOAD_SIZE);
        // Pinned so a node added here shows up as a budget change rather than
        // as a surprise at 2034.
        assert_eq!(0, payload.len(), "encoded PushScene payload");
    }
}
