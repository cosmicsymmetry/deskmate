//! Rasterizing a server-authored face into the frame a picture producer would
//! have pushed.
//!
//! # Why this is not a second renderer
//!
//! `CLAUDE.md` forbids adding a second renderer to check the first, and that
//! rule is intact here. There is still exactly one *scene* renderer. What this
//! module produces is a **frame**: the same canonical RGB565 blob
//! [`crate::image_ingest::canonical_frame_from_png`] produces from a producer's
//! PNG, delivered behind the same one-`Image`-node scene
//! (`frame_face_scene`) a picture card already uses. The server is simply the
//! producer.
//!
//! The alternative was to compose these faces as scenes. It does not fit:
//! `protocol::MAX_SCENE_NODES` is 24 (a fixed `scene_node_t nodes[24]` array in
//! `firmware/main/core/scene_model.h`, so raising it is a firmware change and
//! owes an on-board OTA re-verification), `SceneLine` carries at most 8 points,
//! and only the Caption and Body baked tiers contain letters at all -- Display
//! and Hero are digits-only subsets of `0-9 : - % °`. A layered weather
//! illustration with a six-column hourly strip is not expressible in that
//! budget.
//!
//! # Determinism
//!
//! The font database holds the two bundled Inter faces and nothing else, and
//! `resvg`'s `system-fonts` feature is off in `Cargo.toml`. A face therefore
//! rasterizes to identical bytes on the owner's Mac, in CI, and on docker-vm,
//! which is what makes a golden-image test meaningful rather than a report on
//! what fonts the runner happened to have installed.

use std::sync::{Arc, OnceLock};

use protocol::{SCENE_CANVAS_HEIGHT, SCENE_CANVAS_WIDTH};
use resvg::tiny_skia::{Color, Pixmap, Transform};
use resvg::usvg::fontdb;
use sha2::{Digest, Sha256};

use crate::image_ingest::{CanonicalFrame, encode_rgb565};

/// The two faces the panel's own baked tiers are subset from, so a rastered
/// face and a scene-native one are the same typeface.
///
/// The bundled fonts are tracked build inputs inside this crate. The deploy
/// exports `companion/` plus `firmware/`; `tools/` is outside that payload, so
/// an `include_bytes!` path into it would fail on the build VM.
const INTER_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");
pub(crate) const FONT_FAMILY: &str = "Inter";

/// A face that could not be rasterized. Every variant is a bug in this
/// repository's own SVG authoring rather than anything a feed can cause, so
/// these are logged and surfaced as a card error, never returned to a caller
/// who could act on them.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum FaceRenderError {
    #[error("the authored SVG is malformed: {message}")]
    MalformedSvg { message: String },
    #[error("the rasterizer could not allocate the {width}x{height} canvas")]
    CanvasUnavailable { width: u32, height: u32 },
}

fn font_database() -> &'static Arc<fontdb::Database> {
    static DATABASE: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    DATABASE.get_or_init(|| {
        let mut database = fontdb::Database::new();
        database.load_font_data(INTER_REGULAR.to_vec());
        database.load_font_data(INTER_SEMIBOLD.to_vec());
        database.set_sans_serif_family(FONT_FAMILY);
        Arc::new(database)
    })
}

fn parse_options() -> resvg::usvg::Options<'static> {
    let mut options = resvg::usvg::Options {
        // The authored SVG is written in device pixels, so one user unit is one
        // pixel and no DPI scaling may creep in.
        dpi: 96.0,
        font_family: FONT_FAMILY.to_owned(),
        ..resvg::usvg::Options::default()
    };
    options.fontdb = Arc::clone(font_database());
    options
}

/// Rasterizes one authored face into the canonical frame the asset store and
/// the wire already carry.
///
/// The canvas is filled opaque black first: the panel's ground is black in both
/// schemes (`DESIGN.md`), and it matches the ingest path's own
/// composite-over-black rule, so a face that leaves a region unpainted looks
/// the same whichever producer drew it.
pub(crate) fn frame_from_svg(svg: &str) -> Result<CanonicalFrame, FaceRenderError> {
    let width = u32::try_from(SCENE_CANVAS_WIDTH).expect("fixed canvas");
    let height = u32::try_from(SCENE_CANVAS_HEIGHT).expect("fixed canvas");
    let pixmap = pixmap_from_svg(svg)?;
    let bytes = encode_rgb565(width, height, &pixmap);
    let digest: [u8; 32] = Sha256::digest(&bytes).into();
    Ok(CanonicalFrame { digest, bytes })
}

/// The rasterized face before it is packed down to RGB565.
///
/// Split out because the golden-image harness writes these as PNGs for a human
/// to look at, and reviewing a design is the one job the packed frame is bad
/// at: RGB565 quantization is invisible on a panel and distracting in a
/// side-by-side on a Mac display.
pub(crate) fn pixmap_from_svg(svg: &str) -> Result<Pixmap, FaceRenderError> {
    let width = u32::try_from(SCENE_CANVAS_WIDTH).expect("fixed canvas");
    let height = u32::try_from(SCENE_CANVAS_HEIGHT).expect("fixed canvas");

    let tree = resvg::usvg::Tree::from_str(svg, &parse_options()).map_err(|error| {
        FaceRenderError::MalformedSvg {
            message: error.to_string(),
        }
    })?;

    let mut pixmap =
        Pixmap::new(width, height).ok_or(FaceRenderError::CanvasUnavailable { width, height })?;
    pixmap.fill(Color::BLACK);
    resvg::render(&tree, Transform::identity(), &mut pixmap.as_mut());
    Ok(pixmap)
}

/// Exact advance width of one run of text in the bundled Inter faces.
///
/// The faces wrap and fit their own text, and a fixed panel gives an
/// overestimate nowhere to go: a headline measured 5% narrow than it draws runs
/// off the canvas with no scrollbar to reveal it. So this measures rather than
/// estimates -- it lays the run out through the same `usvg` text engine that
/// will draw it and reads the resulting bounding box.
///
/// Returns 0.0 for empty text, and for text whose glyphs all fail to resolve
/// (which cannot happen with Inter's coverage, but must not panic if it did).
pub(crate) fn text_width(text: &str, size: f64, weight: u16) -> f64 {
    if text.trim().is_empty() {
        return 0.0;
    }
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}"><text x="0" y="{baseline}" font-family="{FONT_FAMILY}" font-size="{size}" font-weight="{weight}">{escaped}</text></svg>"#,
        // A generous measuring canvas: usvg clips no text, but the bbox of a
        // run wider than the viewport is still reported in full.
        width = 20_000,
        height = (size * 4.0).ceil(),
        baseline = (size * 2.0).ceil(),
        escaped = crate::faces::svg::escape(text),
    );
    let Ok(tree) = resvg::usvg::Tree::from_str(&svg, &parse_options()) else {
        return 0.0;
    };
    f64::from(tree.root().abs_layer_bounding_box().width())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368"><rect x="0" y="0" width="448" height="368" fill="#000000"/></svg>"##;

    #[test]
    fn an_authored_face_rasterizes_to_a_canonical_frame() {
        let pixmap = pixmap_from_svg(MINIMAL).expect("the minimal face renders");
        assert_eq!(
            pixmap.width(),
            u32::try_from(SCENE_CANVAS_WIDTH).expect("fixed canvas")
        );
        assert_eq!(
            pixmap.height(),
            u32::try_from(SCENE_CANVAS_HEIGHT).expect("fixed canvas")
        );
        let frame = frame_from_svg(MINIMAL).expect("the minimal face renders");
        // 12-byte LVGL header plus two bytes per pixel: byte-identical in shape
        // to what the PNG ingest path produces, because it is the same encoder.
        let expected = 12 + (448 * 368 * 2);
        assert_eq!(frame.bytes.len(), expected);
    }

    #[test]
    fn an_all_black_face_matches_the_ingest_paths_bytes_for_the_same_picture() {
        // The two producers must agree: a black face drawn here and a black PNG
        // pushed by an external producer are the same asset, same digest. This
        // is what makes "the server is just another producer" true rather than
        // merely intended.
        let frame = frame_from_svg(MINIMAL).expect("the minimal face renders");
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 448, 368);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("png header");
            writer
                .write_image_data(&vec![0u8; 448 * 368 * 3])
                .expect("png body");
        }
        let ingested =
            crate::image_ingest::canonical_frame_from_png(&png).expect("the black PNG is accepted");
        assert_eq!(frame.bytes, ingested.bytes);
        assert_eq!(frame.digest, ingested.digest);
    }

    #[test]
    fn malformed_svg_is_a_typed_refusal_rather_than_a_panic() {
        let error = frame_from_svg("<svg><unclosed").expect_err("malformed SVG is refused");
        assert!(matches!(error, FaceRenderError::MalformedSvg { .. }));
    }

    #[test]
    fn text_measurement_grows_with_length_and_size_and_is_zero_for_blank() {
        assert!(text_width("   ", 24.0, 400).abs() < f64::EPSILON);
        let short = text_width("Solana", 24.0, 600);
        let long = text_width("Solana network fees", 24.0, 600);
        assert!(short > 0.0, "a resolved run has a positive width");
        assert!(long > short, "more text is wider: {long} vs {short}");
        let bigger = text_width("Solana", 48.0, 600);
        assert!(
            bigger > short * 1.5,
            "doubling the size roughly doubles the width: {bigger} vs {short}"
        );
    }

    #[test]
    fn measurement_does_not_depend_on_installed_system_fonts() {
        // The whole determinism argument in this module's header rests on the
        // font database holding exactly the two bundled faces.
        let database = font_database();
        assert_eq!(database.len(), 2, "only the bundled Inter faces are loaded");
        assert!(
            database
                .faces()
                .all(|face| face.families.iter().any(|(name, _)| name == FONT_FAMILY)),
            "every loaded face is Inter"
        );
    }
}
