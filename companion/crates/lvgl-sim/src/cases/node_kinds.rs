use std::sync::{Arc, LazyLock};

use super::orientations;
use crate::SimOrientation;

// One scene case per `scene_node_kind_t`, at both orientations.

use protocol::{
    AssetKind, SCENE_CANVAS_WIDTH, Scene, SceneAlign, SceneArc, SceneClipRect, SceneFont,
    SceneFontTier, SceneGlyph, SceneImage, SceneLabel, SceneLabelAnchor, SceneLine, SceneNode,
    SceneRect, SceneRotRect, SceneScale, SceneText, SceneValue,
};

use crate::scene::{SceneAsset, SceneRenderRequest, SceneTimer};

/// `app-core/src/scene_build.rs`'s palette, so a scene golden reads as a plausible
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
/// face cases already use, so a scene clock and a `DigitalClock` golden
/// can be read side by side.
const SCENE_NOW: i64 = 1_755_000_000;
const SCENE_OFFSET: i16 = 240;

/// SHA-256 of the canonical 8,204-byte [`SCENE_IMAGE_BYTES`] blob.
const SCENE_IMAGE_DIGEST: [u8; 32] = [
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

/// The runtime font asset shared by the asset-font and glyph cases. Its subset
/// is digits, colon, space and `A`-`Z` (see `crate::assets`), which is what the
/// glyph case's codepoints are chosen from.
fn font_asset() -> SceneAsset {
    SceneAsset {
        digest: crate::assets::INTER_SUBSET_SHA256,
        kind: AssetKind::Font,
        bytes: Arc::from(crate::assets::INTER_SUBSET_TTF),
    }
}

/// One 72px runtime-font `SceneText` golden, deliberately outside the four
/// baked sizes. It exercises the same asset-backed scene path as shipping
/// firmware. The baseline centres Inter's 87px line box vertically.
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

/// Appends one scene case at both orientations with a stable name:
/// `scene-{kind_slug}--{orientation_slug}`.
fn scene_case(
    cases: &mut Vec<(String, SceneRenderRequest)>,
    kind_slug: &str,
    nodes: &[SceneNode],
    assets: &[SceneAsset],
    timer: Option<SceneTimer>,
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
            // scene builders make their pills and dots.
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
/// a `time:` binding, a `date` binding that resolves and a `timer.status` one
/// that cannot (this case supplies no timer) — each on a drawn baseline rule.
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
            value: binding("date"),
            ellipsize: false,
        }),
        // This case pushes no timer, so the binding has nothing to resolve
        // against and renders the device's "--" placeholder rather than
        // nothing and rather than failing.
        SceneNode::Text(SceneText {
            x: 288,
            baseline_y: 314,
            w: 136,
            align: SceneAlign::Right,
            font: SceneFont::Baked(SceneFontTier::Caption),
            color: PINK,
            running_color: None,
            value: binding("timer.status"),
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

/// `scale`: a synthetic dial in a 112px box with 13 ticks and
/// every third major, so the majors land at twelve, three, six and nine —
/// with two ordinary line nodes standing in for its hands, because a scale
/// node carries no needle. A second, denser dial pins the tick machinery at
/// a minute scale rather than an hour one.
///
/// The hands are drawn from the scale's own centre, `(x + box/2, y + box/2)`,
/// which is the translation `app-core/src/scene_build.rs` uses: LVGL
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
            value: binding("date"),
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
/// A separate table from [`crate::cases::face_scene_cases`]: one case per scene node kind,
/// rather than per shipped card face. The physical harness drives both with
/// `PushScene`, the same message shipping firmware renders.
pub fn scene_cases() -> Vec<(String, SceneRenderRequest)> {
    let mut cases = Vec::new();
    scene_case(&mut cases, "rect", &scene_rect_nodes(), &[], None);
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
    );
    scene_case(&mut cases, "line", &scene_line_nodes(), &[], None);
    scene_case(&mut cases, "text", &scene_text_nodes(), &[], None);
    scene_case(
        &mut cases,
        "image",
        &scene_image_nodes(),
        &[image_asset()],
        None,
    );
    scene_case(
        &mut cases,
        "glyph",
        &scene_glyph_nodes(),
        &[font_asset()],
        None,
    );
    scene_case(&mut cases, "scale", &scene_scale_nodes(), &[], None);
    scene_case(&mut cases, "label", &scene_label_nodes(), &[], None);
    scene_case(&mut cases, "rot-rect", &scene_rot_rect_nodes(), &[], None);
    cases
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
