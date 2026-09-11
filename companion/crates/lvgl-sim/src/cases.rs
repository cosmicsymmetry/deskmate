//! The render case table: every firmware template x both mount orientations x
//! a field matrix chosen to exercise each template's visually distinct
//! states. `tests/golden.rs` pins the landscape outputs under `tests/golden/`;
//! flipped rows remain available to the physical harness. These rows now drive
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

use std::sync::{Arc, LazyLock};

use crate::SimOrientation;
use protocol::AssetKind;

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

/// 2025-08-13 11:35:00 UTC — the "widest-looking" of the tabular pair.
pub const TABULAR_1135: i64 = 1_755_084_900;
/// 2025-08-13 00:00:00 UTC — the "narrowest-looking" of the tabular pair.
pub const TABULAR_0000: i64 = 1_755_043_200;

/// The shipping-face rows as device-pushable scenes.
///
/// This is both the golden table (`tests/golden.rs` pins the landscape rows
/// under `tests/golden/`) and the hardware-facing half of the framebuffer
/// matrix that `framebuffer_diff` drives.
///
/// It used to be an adapter over a second table of C-template requests, which
/// the retired reference oracle rendered so a parity gate could compare the
/// two. There is one renderer now, so the field matrix is expressed directly
/// as scene-builder inputs. The row NAMES are unchanged on purpose:
/// `framebuffer_diff`'s exclusions are keyed by them, and renaming a row would
/// silently make an exclusion branch unreachable.
#[allow(clippy::too_many_lines)] // one explicit row per shipped face case
pub fn face_scene_cases() -> Vec<(String, SceneRenderRequest)> {
    use app_core::scene_build::{
        AnalogClockCard, BakedFontMetrics, ClockCard, ProgressRingCard, SceneDataState,
        build_analog_clock_scene, build_digital_clock_scene, build_progress_ring_scene,
        with_scene_data_state,
    };

    /// 2025-08-12 12:00:00 UTC.
    const NOW: i64 = 1_755_000_000;
    /// -> 16:00 local.
    const OFFSET: i16 = 240;
    /// The analog brief's pinned instant. `typical` and `no-seconds` share it
    /// on purpose: together they pin that the hour and minute hands do not
    /// depend on `show_seconds`, which is the deterministic form of the
    /// 2026-08-11 no-seconds defect.
    const ANALOG_INSTANT: i64 = 1_755_081_480;
    /// 2025-08-13 00:00:00 UTC.
    const MIDNIGHT_INSTANT: i64 = 1_755_043_200;
    const DURATION_SECONDS: i64 = 1500;

    let metrics = &BakedFontMetrics::SHIPPED;

    let clock = |show_seconds: bool, now_unix_seconds: i64, utc_offset_minutes: i16| {
        let local_seconds = now_unix_seconds + i64::from(utc_offset_minutes) * 60;
        let local_now = chrono::DateTime::from_timestamp(local_seconds, 0)
            .expect("golden clock instant is representable")
            .naive_utc();
        build_digital_clock_scene(
            &ClockCard {
                revision: 1,
                show_seconds,
                local_now,
            },
            metrics,
        )
    };
    let analog = |show_seconds: bool| {
        build_analog_clock_scene(&AnalogClockCard {
            revision: 1,
            show_seconds,
        })
    };
    let ring = |remaining_seconds: i64, running: bool| {
        let scene = build_progress_ring_scene(
            &ProgressRingCard {
                revision: 1,
                label: "Pomodoro",
                duration_seconds: DURATION_SECONDS,
            },
            metrics,
        );
        let to_milliseconds = |seconds: i64| {
            u32::try_from(seconds * 1_000).expect("progress-ring fixture is inside u32")
        };
        (
            scene,
            Some(SceneTimer {
                total_ms: to_milliseconds(DURATION_SECONDS),
                remaining_ms: to_milliseconds(remaining_seconds),
                running,
            }),
        )
    };

    let rows: Vec<(&'static str, Scene, Option<SceneTimer>, i64, i16)> = vec![
        (
            "digital-clock--typical",
            clock(true, NOW, OFFSET),
            None,
            NOW,
            OFFSET,
        ),
        (
            "digital-clock--no-seconds",
            clock(false, NOW, OFFSET),
            None,
            NOW,
            OFFSET,
        ),
        // Tabular-figure pair. Both instants render an HH:MM with no repeated
        // digit shape in common, at the same font and the same box, so their
        // lit column spans must be identical -- proportional figures would
        // shift the second frame. `tests/tabular.rs` asserts that equality in
        // pixels; these goldens pin the frames it asserts over.
        (
            "digital-clock--tabular-1135",
            clock(false, TABULAR_1135, 0),
            None,
            TABULAR_1135,
            0,
        ),
        (
            "digital-clock--tabular-0000",
            clock(false, TABULAR_0000, 0),
            None,
            TABULAR_0000,
            0,
        ),
        (
            "analog-clock--typical",
            analog(true),
            None,
            ANALOG_INSTANT,
            0,
        ),
        (
            "analog-clock--no-seconds",
            analog(false),
            None,
            ANALOG_INSTANT,
            0,
        ),
        (
            "analog-clock--midnight",
            analog(true),
            None,
            MIDNIGHT_INSTANT,
            0,
        ),
    ];

    // A running ring mid-countdown is the only pixel coverage of the running
    // arc-indicator hue: `running` drives the indicator and status-text
    // colours, and at a zero-length arc the indicator is not drawn at all. It
    // is golden-only -- `framebuffer_diff` excludes it by name, because the
    // device keeps ticking a running timer through `scene_view_tick_bindings`
    // while this simulator's fake tick is fixed.
    let ring_rows: Vec<(&'static str, (Scene, Option<SceneTimer>))> = vec![
        ("progress-ring--running-mid-countdown", ring(900, true)),
        // The hardware-comparable half of the row above: the same partial arc
        // with the ring stopped, so both sides render the pinned value.
        ("progress-ring--paused-mid-countdown", ring(900, false)),
        // The only running ring hardware can be compared on, and so the only
        // on-device coverage of the running palette -- here the status text,
        // since a zero-length arc draws no indicator.
        ("progress-ring--running-at-zero", ring(0, true)),
        ("progress-ring--finished", ring(0, false)),
        (
            "progress-ring--never-started",
            ring(DURATION_SECONDS, false),
        ),
    ];

    let mut cases = Vec::new();
    let all = rows.into_iter().chain(
        ring_rows
            .into_iter()
            .map(|(name, (scene, timer))| (name, scene, timer, NOW, 0)),
    );
    for (name, scene, timer, now_unix_seconds, utc_offset_minutes) in all {
        let scene = with_scene_data_state(
            scene,
            SceneDataState {
                stale: false,
                error: None,
            },
            metrics,
        );
        for (orientation_slug, orientation) in orientations() {
            cases.push((
                format!("{name}--{orientation_slug}"),
                SceneRenderRequest {
                    scene: scene.clone(),
                    assets: Vec::new(),
                    utc_offset_minutes,
                    now_unix_seconds,
                    timer,
                    fields: Vec::new(),
                    orientation,
                },
            ));
        }
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

/// SHA-256 of the canonical 8,204-byte [`SCENE_IMAGE_BYTES`] blob.
pub const SCENE_IMAGE_DIGEST: [u8; 32] = [
    0xdd, 0xb7, 0x3d, 0xdf, 0x7a, 0xad, 0x5e, 0x8f, 0xd6, 0x7a, 0x8a, 0x6c, 0xbe, 0x93, 0x56, 0x61,
    0x22, 0xee, 0x2b, 0x20, 0x36, 0x74, 0x29, 0x4e, 0x06, 0xf8, 0xa9, 0x32, 0x1b, 0x54, 0x96, 0x51,
];

/// The synthetic image's side, in pixels. Square, and the same size the
/// `image` nodes below declare, so nothing is scaled — except the one node
/// that deliberately declares a smaller box to show the documented clipping.
const SCENE_IMAGE_SIDE: u32 = 64;

/// A 64x64 RGB565 test image: a 4px white frame around four coloured
/// quadrants. Asymmetric on both axes on purpose so the flipped hardware row
/// cannot be confused with the landscape placement; a solid image would prove
/// nothing about placement.
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

/// Builds the LVGL binary image blob once: its 12-byte little-endian header
/// followed by the raw RGB565 pixel bytes. This is the same canonical form a
/// asset resolver hashes and the device receives.
static SCENE_IMAGE_BYTES: LazyLock<Arc<[u8]>> = LazyLock::new(|| {
    const MAGIC: u32 = 0x19;
    const COLOR_FORMAT_RGB565: u32 = 0x12;
    let pixels = quadrant_image_pixels();
    let stride = SCENE_IMAGE_SIDE * 2;
    let word0 = MAGIC | (COLOR_FORMAT_RGB565 << 8);
    let word1 = SCENE_IMAGE_SIDE | (SCENE_IMAGE_SIDE << 16);
    let word2 = stride;
    let mut bytes = Vec::with_capacity(12 + pixels.len() * 2);
    bytes.extend_from_slice(&word0.to_le_bytes());
    bytes.extend_from_slice(&word1.to_le_bytes());
    bytes.extend_from_slice(&word2.to_le_bytes());
    for pixel in pixels {
        bytes.extend_from_slice(&pixel.to_le_bytes());
    }
    bytes.into()
});

/// The synthetic image asset, as a scene request lists it.
fn image_asset() -> SceneAsset {
    SceneAsset {
        digest: SCENE_IMAGE_DIGEST,
        kind: AssetKind::Image,
        bytes: Arc::clone(&SCENE_IMAGE_BYTES),
    }
}

/// The runtime font asset, reused from the Task 12 golden rather than
/// vendored twice. Its subset is digits, colon, space and `A`-`Z` (see
/// `crate::assets`), which is what the glyph case's codepoints are chosen
/// from.
fn font_asset() -> SceneAsset {
    SceneAsset {
        digest: crate::assets::INTER_SUBSET_SHA256,
        kind: AssetKind::Font,
        bytes: Arc::from(crate::assets::INTER_SUBSET_TTF),
    }
}

/// One 72px runtime-font `SceneText` golden, deliberately outside the four
/// baked sizes. This is the normal asset-backed scene path used by shipping
/// firmware; the retired raw 0x7D probe and its special host renderer are no
/// longer needed. The baseline centres Inter's 87px line box vertically.
pub fn asset_font_scene_cases() -> Vec<(String, SceneRenderRequest)> {
    vec![(
        "asset-font--72px-digits--landscape".to_string(),
        SceneRenderRequest {
            scene: Scene {
                revision: 1,
                background: CANVAS,
                nodes: vec![SceneNode::Text(SceneText {
                    x: 1,
                    baseline_y: 211,
                    w: SCENE_CANVAS_WIDTH - 1,
                    align: SceneAlign::Center,
                    font: SceneFont::Asset {
                        digest: crate::assets::INTER_SUBSET_SHA256,
                        pixel_size: 72,
                    },
                    color: PRIMARY,
                    running_color: None,
                    value: literal("12:34"),
                    ellipsize: false,
                })],
            },
            assets: vec![font_asset()],
            utc_offset_minutes: 0,
            now_unix_seconds: SCENE_NOW,
            timer: None,
            fields: Vec::new(),
            orientation: SimOrientation::Landscape,
        },
    )]
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
        // No upstream source has reported `absent`, so this renders the device's
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
/// A separate table from [`face_scene_cases`]: one case per scene node kind,
/// rather than per shipped card face. The physical harness drives both with
/// `PushScene`, the same message shipping firmware renders.
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
// Native date-overflow evidence.
// ---------------------------------------------------------------------------

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
    let local_seconds =
        DATE_OVERFLOW_NOW_UNIX_SECONDS + i64::from(DATE_OVERFLOW_UTC_OFFSET_MINUTES) * 60;
    let local_now = chrono::DateTime::from_timestamp(local_seconds, 0)
        .expect("the date-overflow instant is representable")
        .naive_utc();
    let scene = app_core::build_digital_clock_scene(
        &app_core::ClockCard {
            revision: 1,
            show_seconds: false,
            local_now,
        },
        &app_core::BakedFontMetrics::SHIPPED,
    );

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
                    fields: Vec::new(),
                    orientation,
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod scene_image_tests {
    use sha2::{Digest, Sha256};

    use super::*;

    #[test]
    fn synthetic_image_digest_matches_canonical_blob() {
        let computed: [u8; 32] = Sha256::digest(&*SCENE_IMAGE_BYTES).into();
        assert_eq!(computed, SCENE_IMAGE_DIGEST);
    }
}
