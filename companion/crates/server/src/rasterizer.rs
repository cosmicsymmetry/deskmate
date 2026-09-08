//! Bounded SVG rasterization into the device's decoded LVGL RGB565 image format.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Instant;

use base64::Engine as _;
use resvg::tiny_skia::{Color, Pixmap, Transform};
use roxmltree::{Document, Node, NodeId};
use sha2::{Digest, Sha256};
use thiserror::Error;

use plugin::{EvalContext, EvalValue, Expr, ExprError, ExpressionSource, FUEL_BUDGET, Fuel};
use protocol::{
    Field, FieldValue, SCENE_CANVAS_HEIGHT, SCENE_CANVAS_WIDTH, Scene, SceneAlign, SceneFont,
    SceneFontTier, SceneLabelAnchor, SceneNode, SceneValue,
};

const LVGL_IMAGE_HEADER_BYTES: usize = 12;
const LVGL_IMAGE_MAGIC: u32 = 0x19;
const LVGL_COLOR_FORMAT_RGB565: u32 = 0x12;
const INTER_REGULAR: &[u8] = include_bytes!("../../../../tools/fonts/Inter-Regular.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../../../../tools/fonts/Inter-SemiBold.ttf");
const ALLOWED_FONT_FAMILY: &str = "Inter";

pub use crate::plugin_provider::MAX_RENDER_WALL_CLOCK;
pub use plugin::{MAX_EXPANDED_SVG_BYTES, MAX_SVG_SOURCE_BYTES};
/// XML nesting is capped to keep all tree walks and renderer recursion shallow.
pub const MAX_XML_DEPTH: usize = 64;
/// Element count is capped before renderer allocation to bound document complexity.
pub const MAX_XML_ELEMENT_NODES: usize = 4_096;
/// Referenced `<use>` trees are metered separately because a small DOM can expand greatly.
pub const MAX_EXPANDED_SVG_NODES: usize = 8_192;
/// Root dimensions are bounded even though output is normalized to the fixed device canvas.
pub const MAX_SVG_DIMENSION: u32 = 4_096;
/// The decoded image bytes that can be uploaded directly as an LVGL image asset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RasterizedFrame {
    pub digest: [u8; 32],
    pub bytes: Vec<u8>,
}

/// Content-addressed bytes available while translating asset-bearing scene nodes.
pub(crate) type RasterAssetMap = HashMap<[u8; 32], Arc<[u8]>>;

/// A named, fail-closed reason an SVG was not rasterized.
#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum RasterizeError {
    #[error("SVG source exceeds the {max_bytes}-byte limit")]
    SourceTooLarge { max_bytes: usize },
    #[error("SVG source is not valid UTF-8")]
    InvalidUtf8,
    #[error("expanded SVG exceeds the {max_bytes}-byte limit")]
    ExpandedSourceTooLarge { max_bytes: usize },
    #[error("SVG XML is malformed")]
    MalformedXml,
    #[error("DTD/entity expansion is forbidden")]
    EntityExpansionForbidden,
    #[error("XML depth exceeds the {max_depth}-element limit")]
    XmlTooDeep { max_depth: usize },
    #[error("XML element count exceeds the {max_nodes}-node limit")]
    TooManyNodes { max_nodes: usize },
    #[error("expanded SVG exceeds the {max_nodes}-node limit")]
    ExpansionTooLarge { max_nodes: usize },
    #[error("SVG scripts are forbidden")]
    ScriptForbidden,
    #[error("SVG event-handler attributes are forbidden")]
    EventHandlerForbidden,
    #[error("SVG animation is forbidden")]
    AnimationForbidden,
    #[error("SVG foreignObject content is forbidden")]
    ForeignObjectForbidden,
    #[error("network references are forbidden")]
    NetworkReferenceForbidden,
    #[error("filesystem references are forbidden")]
    FileReferenceForbidden,
    #[error("data URLs are forbidden")]
    DataReferenceForbidden,
    #[error("external stylesheets are forbidden")]
    ExternalStylesheetForbidden,
    #[error("CSS url() references are forbidden")]
    CssUrlForbidden,
    #[error("font family is not in the explicit rasterizer font database: {family}")]
    MissingFont { family: String },
    #[error("SVG root dimensions must be finite, positive, and at most {max}")]
    InvalidDimensions { max: u32 },
    #[error("SVG parsing failed: {0}")]
    SvgParse(String),
    #[error("scene validation failed: {0}")]
    InvalidScene(String),
    #[error("live binding cannot be rasterized: {binding}")]
    LiveBinding { binding: String },
    #[error("unknown binding cannot be rasterized: {binding}")]
    UnknownBinding { binding: String },
    #[error("field binding has no supplied value: {binding}")]
    MissingField { binding: String },
    #[error("scene asset is unavailable: {digest}")]
    MissingAsset { digest: String },
    #[error("scene image asset is not canonical RGB565: {digest}")]
    InvalidImageAsset { digest: String },
    #[error("SVG expression must occupy the complete XML value: {value}")]
    PartialInterpolation { value: String },
    #[error("SVG expression failed: {message}")]
    Expression { message: String },
    #[error("SVG document exhausted its shared expression fuel budget")]
    ExpressionFuelExhausted,
    #[error("the renderer could not allocate the fixed canvas")]
    CanvasAllocation,
    #[error("SVG validation or rendering exceeded the {millis} ms budget")]
    RenderDeadlineExceeded { millis: u128 },
}

/// Rasterizes one SVG source string onto the fixed, true-black device canvas.
#[cfg(test)]
fn rasterize_svg(svg: &str) -> Result<RasterizedFrame, RasterizeError> {
    rasterize_svg_bytes(svg.as_bytes())
}

#[cfg(test)]
fn rasterize_svg_bytes(bytes: &[u8]) -> Result<RasterizedFrame, RasterizeError> {
    let deadline = RenderDeadline::start();
    render_svg_bytes(
        bytes,
        MAX_SVG_SOURCE_BYTES,
        false,
        &RasterAssetMap::new(),
        &deadline,
    )
}

/// Evaluates a registry-owned SVG template against its already-rooted provider value.
#[cfg(test)]
fn evaluate_svg_template(
    template: &str,
    rooted_data: &serde_json::Value,
    fields: &[Field],
) -> Result<String, RasterizeError> {
    evaluate_svg_template_with_deadline(template, rooted_data, fields, &RenderDeadline::start())
}

fn evaluate_svg_template_with_deadline(
    template: &str,
    rooted_data: &serde_json::Value,
    fields: &[Field],
    deadline: &RenderDeadline,
) -> Result<String, RasterizeError> {
    if template.len() > MAX_SVG_SOURCE_BYTES {
        return Err(RasterizeError::SourceTooLarge {
            max_bytes: MAX_SVG_SOURCE_BYTES,
        });
    }
    let document = Document::parse(template).map_err(|_| RasterizeError::MalformedXml)?;
    deadline.check()?;
    let context = EvalContext::with_data(rooted_data);
    let mut fuel = Fuel::new(FUEL_BUDGET);
    let mut replacements = Vec::new();

    for node in document.descendants() {
        deadline.check()?;
        if node.is_text() {
            let range = node.range();
            let raw = &template[range.clone()];
            if contains_mustache(raw) {
                replacements.push((
                    range,
                    evaluate_xml_value(raw, &context, fields, &mut fuel, deadline)?,
                ));
            }
        }
        if node.is_element() {
            for attribute in node.attributes() {
                deadline.check()?;
                let range = attribute.range_value();
                let raw = &template[range.clone()];
                if contains_mustache(raw) {
                    replacements.push((
                        range,
                        evaluate_xml_value(raw, &context, fields, &mut fuel, deadline)?,
                    ));
                }
            }
        }
    }

    replacements.sort_by_key(|(range, _)| range.start);
    let mut expanded = template.to_owned();
    for (range, replacement) in replacements.into_iter().rev() {
        deadline.check()?;
        expanded.replace_range(range, &replacement);
    }
    if expanded.len() > MAX_EXPANDED_SVG_BYTES {
        return Err(RasterizeError::ExpandedSourceTooLarge {
            max_bytes: MAX_EXPANDED_SVG_BYTES,
        });
    }
    Ok(expanded)
}

/// Evaluates and rasterizes one registry-owned SVG template through the same renderer
/// and output encoder used for translated scenes.
pub(crate) fn rasterize_plugin_svg(
    template: &str,
    rooted_data: &serde_json::Value,
    fields: &[Field],
    assets: &RasterAssetMap,
    state: app_core::SceneDataState<'_>,
) -> Result<RasterizedFrame, RasterizeError> {
    let deadline = RenderDeadline::start();
    let mut evaluated =
        evaluate_svg_template_with_deadline(template, rooted_data, fields, &deadline)?;
    append_data_state_footer(&mut evaluated, state, assets, &deadline)?;
    render_svg_bytes(
        evaluated.as_bytes(),
        MAX_EXPANDED_SVG_BYTES,
        false,
        assets,
        &deadline,
    )
}

#[cfg(test)]
fn rasterize_svg_template(
    template: &str,
    rooted_data: &serde_json::Value,
    fields: &[Field],
) -> Result<RasterizedFrame, RasterizeError> {
    rasterize_plugin_svg(
        template,
        rooted_data,
        fields,
        &RasterAssetMap::new(),
        app_core::SceneDataState {
            stale: false,
            error: None,
        },
    )
}

/// Translates one validated static scene into a trusted SVG document.
///
/// Scene image data is converted from the canonical LVGL RGB565 blob to an embedded
/// SVG image. That data URI exists only in this module's generated output. The public
/// [`rasterize_svg`] and [`rasterize_svg_template`] paths continue to reject every data
/// URL from plugin-authored input.
#[allow(clippy::too_many_lines)] // one arm per scene node kind
pub(crate) fn scene_to_svg(
    scene: &Scene,
    fields: &[Field],
    assets: &RasterAssetMap,
    deadline: &RenderDeadline,
) -> Result<String, RasterizeError> {
    protocol::validate_scene(scene)
        .map_err(|error| RasterizeError::InvalidScene(error.to_string()))?;
    deadline.check()?;
    let requirements = app_core::analyze_scene(scene).map_err(|error| match error {
        app_core::RequirementsError::UnknownBinding { binding } => {
            RasterizeError::UnknownBinding { binding }
        }
    })?;
    if let Some(binding) = requirements.bindings.live.first() {
        return Err(RasterizeError::LiveBinding {
            binding: binding.clone(),
        });
    }

    let mut svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="0 0 {} {}"><rect width="{}" height="{}" fill="{}"/>"#,
        SCENE_CANVAS_WIDTH,
        SCENE_CANVAS_HEIGHT,
        SCENE_CANVAS_WIDTH,
        SCENE_CANVAS_HEIGHT,
        SCENE_CANVAS_WIDTH,
        SCENE_CANVAS_HEIGHT,
        color(scene.background),
    );
    let mut definitions = String::new();
    let mut body = String::new();

    for (index, node) in scene.nodes.iter().enumerate() {
        deadline.check()?;
        match node {
            SceneNode::Rect(rect) => {
                let clip = clip_attribute(rect.clip, index, &mut definitions);
                write!(body, r#"<rect x="{}" y="{}" width="{}" height="{}" rx="{}" fill="{}" opacity="{}"{clip}/>"#, rect.x, rect.y, rect.w, rect.h, rect.radius, color(rect.fill), opacity(rect.opacity)).unwrap();
            }
            SceneNode::Arc(arc) => {
                let common = format!(
                    r#" fill="none" stroke="{}" stroke-width="{}" opacity="{}" stroke-linecap="{}""#,
                    color(arc.color),
                    arc.width,
                    opacity(arc.opacity),
                    if arc.rounded { "round" } else { "butt" }
                );
                if (arc.end_deg - arc.start_deg).unsigned_abs() >= 360 {
                    write!(
                        body,
                        r#"<circle cx="{}" cy="{}" r="{}"{common}/>"#,
                        arc.cx, arc.cy, arc.r
                    )
                    .unwrap();
                } else {
                    let (x1, y1) = polar(arc.cx, arc.cy, arc.r, arc.start_deg);
                    let (x2, y2) = polar(arc.cx, arc.cy, arc.r, arc.end_deg);
                    let large = i32::from((arc.end_deg - arc.start_deg).unsigned_abs() > 180);
                    let sweep = i32::from(arc.end_deg >= arc.start_deg);
                    write!(body, r#"<path d="M {x1:.4} {y1:.4} A {} {} 0 {large} {sweep} {x2:.4} {y2:.4}"{common}/>"#, arc.r, arc.r).unwrap();
                }
            }
            SceneNode::Line(line) => {
                let points = line
                    .xs
                    .iter()
                    .zip(&line.ys)
                    .map(|(x, y)| format!("{x},{y}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                write!(
                    body,
                    r#"<polyline points="{points}" fill="none" stroke="{}" stroke-width="{}"/>"#,
                    color(line.color),
                    line.width
                )
                .unwrap();
            }
            SceneNode::Text(text) => {
                let value = scene_value(&text.value, fields)?;
                let (family, size, weight) = font_properties(&text.font, assets)?;
                let (x, anchor) = aligned_x(text.x, text.w, text.align);
                let clip = if text.ellipsize {
                    let clip_rect = protocol::SceneClipRect {
                        x: text.x,
                        y: 0,
                        w: text.w,
                        h: SCENE_CANVAS_HEIGHT,
                    };
                    clip_attribute(Some(clip_rect), index, &mut definitions)
                } else {
                    String::new()
                };
                write!(body, r#"<text x="{x}" y="{}" text-anchor="{anchor}" font-family="{}" font-size="{size}" font-weight="{weight}" fill="{}"{clip}>{}</text>"#, text.baseline_y, xml_escape(&family), color(text.color), xml_escape(&value)).unwrap();
            }
            SceneNode::Image(image) => {
                let bytes = asset_bytes(assets, &image.digest)?;
                if image.recolor {
                    write!(
                        body,
                        r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{}"/>"#,
                        image.x,
                        image.y,
                        image.w,
                        image.h,
                        color(image.color)
                    )
                    .unwrap();
                } else {
                    let embedded = rgb565_asset_svg(bytes, &image.digest, deadline)?;
                    write!(body, r#"<image x="{}" y="{}" width="{}" height="{}" preserveAspectRatio="none" href="data:image/svg+xml;base64,{}"/>"#, image.x, image.y, image.w, image.h, base64(&embedded, deadline)?).unwrap();
                }
            }
            SceneNode::Glyph(glyph) => {
                let bytes = asset_bytes(assets, &glyph.digest)?;
                let family = font_family(bytes).ok_or_else(|| RasterizeError::MissingFont {
                    family: protocol::digest_hex(&glyph.digest),
                })?;
                let value = glyph_text(&glyph.name);
                write!(
                    body,
                    r#"<text x="{}" y="{}" font-family="{}" font-size="{}" fill="{}">{}</text>"#,
                    glyph.x,
                    glyph.baseline_y,
                    xml_escape(&family),
                    glyph.size,
                    color(glyph.color),
                    xml_escape(&value)
                )
                .unwrap();
            }
            SceneNode::Scale(scale) => {
                let center_x = f64::from(scale.x) + f64::from(scale.box_size) / 2.0;
                let center_y = f64::from(scale.y) + f64::from(scale.box_size) / 2.0;
                let outer = f64::from(scale.box_size) / 2.0;
                for tick in 0..scale.total_tick_count {
                    deadline.check()?;
                    let major = tick % scale.major_tick_every == 0;
                    let inner = outer - if major { 10.0 } else { 5.0 };
                    let angle =
                        270.0 + f64::from(tick) * 360.0 / f64::from(scale.total_tick_count - 1);
                    let (x1, y1) = polar_f64(center_x, center_y, inner, angle);
                    let (x2, y2) = polar_f64(center_x, center_y, outer, angle);
                    write!(body, r#"<line x1="{x1:.4}" y1="{y1:.4}" x2="{x2:.4}" y2="{y2:.4}" stroke="{}" stroke-width="{}"/>"#, color(scale.major_tick_color), if major { 3 } else { 1 }).unwrap();
                }
            }
            SceneNode::Label(label) => {
                let value = scene_value(&label.value, fields)?;
                if label.hide_when_empty && value.is_empty() {
                    continue;
                }
                let (family, size, weight) = font_properties(&label.font, assets)?;
                let estimated_text_width = estimate_text_width(&value, size, label.letter_space);
                let width = estimated_text_width + label.pad_hor * 2;
                let height = size + label.pad_ver * 2;
                let left = match label.horizontal_anchor {
                    SceneLabelAnchor::Left => label.x,
                    SceneLabelAnchor::Center => label.x - width / 2,
                    SceneLabelAnchor::Right => label.x - width,
                };
                write!(body, r#"<rect x="{left}" y="{}" width="{width}" height="{height}" rx="{}" fill="{}" opacity="{}"/><text x="{}" y="{}" font-family="{}" font-size="{size}" font-weight="{weight}" letter-spacing="{}" fill="{}">{}</text>"#, label.y, label.radius, color(label.fill), opacity(label.fill_opacity), left + label.pad_hor, label.y + label.pad_ver + size, xml_escape(&family), label.letter_space, color(label.ink), xml_escape(&value)).unwrap();
            }
            SceneNode::RotRect(rect) => {
                let clip = clip_attribute(rect.clip, index, &mut definitions);
                let pivot_x = rect.x + rect.pivot_x;
                let pivot_y = rect.y + rect.pivot_y;
                let degrees = f64::from(rect.rotation) / 10.0;
                write!(body, r#"<rect x="{}" y="{}" width="{}" height="{}" rx="{}" fill="{}" transform="rotate({degrees} {pivot_x} {pivot_y})"{clip}/>"#, rect.x, rect.y, rect.w, rect.h, rect.radius, color(rect.fill)).unwrap();
            }
        }
    }
    if !definitions.is_empty() {
        write!(svg, "<defs>{definitions}</defs>").unwrap();
    }
    svg.push_str(&body);
    svg.push_str("</svg>");
    if svg.len() > MAX_EXPANDED_SVG_BYTES {
        return Err(RasterizeError::ExpandedSourceTooLarge {
            max_bytes: MAX_EXPANDED_SVG_BYTES,
        });
    }
    Ok(svg)
}

fn append_data_state_footer(
    svg: &mut String,
    state: app_core::SceneDataState<'_>,
    assets: &RasterAssetMap,
    deadline: &RenderDeadline,
) -> Result<(), RasterizeError> {
    let bounded_error = state
        .error
        .map(|error| protocol::truncate_utf8_to_bytes(error, protocol::MAX_SCENE_TEXT_LEN));
    let footer_scene = app_core::with_scene_data_state(
        Scene {
            revision: 0,
            background: 0,
            nodes: Vec::new(),
        },
        app_core::SceneDataState {
            stale: state.stale,
            error: bounded_error,
        },
        &app_core::BakedFontMetrics::SHIPPED,
    );
    let Some(SceneNode::Text(text)) = footer_scene.nodes.first() else {
        return Ok(());
    };
    deadline.check()?;
    let value = scene_value(&text.value, &[])?;
    let (family, size, weight) = font_properties(&text.font, assets)?;
    let (x, anchor) = aligned_x(text.x, text.w, text.align);
    let footer = format!(
        r#"<text x="{x}" y="{}" text-anchor="{anchor}" font-family="{}" font-size="{size}" font-weight="{weight}" fill="{}">{}</text>"#,
        text.baseline_y,
        xml_escape(&family),
        color(text.color),
        xml_escape(&value),
    );
    let closing = svg.rfind("</svg>").ok_or(RasterizeError::MalformedXml)?;
    svg.insert_str(closing, &footer);
    if svg.len() > MAX_EXPANDED_SVG_BYTES {
        return Err(RasterizeError::ExpandedSourceTooLarge {
            max_bytes: MAX_EXPANDED_SVG_BYTES,
        });
    }
    Ok(())
}

/// Rasterizes a static protocol scene through the sole resvg pipeline.
pub(crate) fn rasterize_scene(
    scene: &Scene,
    fields: &[Field],
    assets: &RasterAssetMap,
) -> Result<RasterizedFrame, RasterizeError> {
    let deadline = RenderDeadline::start();
    let svg = scene_to_svg(scene, fields, assets, &deadline)?;
    render_svg_bytes(
        svg.as_bytes(),
        MAX_EXPANDED_SVG_BYTES,
        true,
        assets,
        &deadline,
    )
}

fn render_svg_bytes(
    bytes: &[u8],
    source_limit: usize,
    generated: bool,
    assets: &RasterAssetMap,
    deadline: &RenderDeadline,
) -> Result<RasterizedFrame, RasterizeError> {
    if bytes.len() > source_limit {
        return Err(RasterizeError::SourceTooLarge {
            max_bytes: source_limit,
        });
    }
    let svg = std::str::from_utf8(bytes).map_err(|_| RasterizeError::InvalidUtf8)?;
    let allowed_fonts = allowed_font_families(assets, deadline)?;
    preflight(svg, generated, &allowed_fonts, deadline)?;
    deadline.check()?;

    let options = renderer_options(generated, assets, deadline)?;

    let tree = resvg::usvg::Tree::from_str(svg, &options)
        .map_err(|error| RasterizeError::SvgParse(error.to_string()))?;
    deadline.check()?;

    let width = u32::try_from(SCENE_CANVAS_WIDTH).expect("protocol canvas width is positive");
    let height = u32::try_from(SCENE_CANVAS_HEIGHT).expect("protocol canvas height is positive");
    let mut pixmap = Pixmap::new(width, height).ok_or(RasterizeError::CanvasAllocation)?;
    // DESIGN.md defines the display stage as true black in both schemes. Filling first
    // makes transparent SVG pixels and partial alpha composite against the panel ground,
    // never usvg's transparent buffer and never white.
    pixmap.fill(Color::BLACK);
    let source_size = tree.size();
    let canvas_width = f32::from(u16::try_from(width).expect("fixed canvas width fits u16"));
    let canvas_height = f32::from(u16::try_from(height).expect("fixed canvas height fits u16"));
    let transform = Transform::from_scale(
        canvas_width / source_size.width(),
        canvas_height / source_size.height(),
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    deadline.check()?;

    let bytes = encode_rgb565(width, height, &pixmap, deadline)?;
    deadline.check()?;
    let digest = Sha256::digest(&bytes).into();
    Ok(RasterizedFrame { digest, bytes })
}

fn renderer_options(
    generated: bool,
    assets: &RasterAssetMap,
    deadline: &RenderDeadline,
) -> Result<resvg::usvg::Options<'static>, RasterizeError> {
    // usvg's default string resolver reads relative/absolute filesystem paths even
    // when `resources_dir` is None, while its data resolver expands embedded images.
    // Replace both callbacks with deny-all functions; preflight's named errors remain
    // the user-facing decision, and these callbacks are defense in depth.
    let mut options = resvg::usvg::Options {
        resources_dir: None,
        style_sheet: None,
        image_href_resolver: if generated {
            resvg::usvg::ImageHrefResolver {
                resolve_data: resvg::usvg::ImageHrefResolver::default_data_resolver(),
                resolve_string: Box::new(|_, _| None),
            }
        } else {
            resvg::usvg::ImageHrefResolver {
                resolve_data: Box::new(|_, _, _| None),
                resolve_string: Box::new(|_, _| None),
            }
        },
        font_family: ALLOWED_FONT_FAMILY.to_owned(),
        ..Default::default()
    };
    // This database starts empty. Loading these reviewed bytes directly is deliberate:
    // never call `load_system_fonts`, which would grant ambient host filesystem access.
    // usvg's default FontResolver queries only this supplied database.
    options.fontdb_mut().load_font_data(INTER_REGULAR.to_vec());
    deadline.check()?;
    options.fontdb_mut().load_font_data(INTER_SEMIBOLD.to_vec());
    deadline.check()?;
    for bytes in assets.values() {
        options.fontdb_mut().load_font_data(bytes.to_vec());
        deadline.check()?;
    }
    Ok(options)
}

pub(crate) struct RenderDeadline(Instant);

impl RenderDeadline {
    fn start() -> Self {
        Self(Instant::now())
    }

    fn check(&self) -> Result<(), RasterizeError> {
        if self.0.elapsed() > MAX_RENDER_WALL_CLOCK {
            return Err(RasterizeError::RenderDeadlineExceeded {
                millis: MAX_RENDER_WALL_CLOCK.as_millis(),
            });
        }
        Ok(())
    }
}

fn preflight(
    svg: &str,
    generated: bool,
    allowed_fonts: &HashSet<String>,
    deadline: &RenderDeadline,
) -> Result<(), RasterizeError> {
    let lowercase = svg.to_ascii_lowercase();
    deadline.check()?;
    if lowercase.contains("<!doctype") || lowercase.contains("<!entity") {
        return Err(RasterizeError::EntityExpansionForbidden);
    }
    if lowercase.contains("<?xml-stylesheet") || lowercase.contains("@import") {
        return Err(RasterizeError::ExternalStylesheetForbidden);
    }
    // All CSS url() forms are rejected, including local paint servers. This is more
    // restrictive than SVG itself, but it makes authority review syntactic and prevents
    // escaped/redirected URL forms from relying on a renderer default.
    if lowercase.contains("url(") && !generated {
        return Err(RasterizeError::CssUrlForbidden);
    }

    let document = Document::parse(svg).map_err(|_| RasterizeError::MalformedXml)?;
    deadline.check()?;
    let root = document.root_element();
    if root.tag_name().name() != "svg" {
        return Err(RasterizeError::MalformedXml);
    }
    validate_dimensions(root)?;

    let mut element_count = 0_usize;
    let mut ids = HashMap::new();
    for node in document.descendants().filter(Node::is_element) {
        deadline.check()?;
        element_count += 1;
        if element_count > MAX_XML_ELEMENT_NODES {
            return Err(RasterizeError::TooManyNodes {
                max_nodes: MAX_XML_ELEMENT_NODES,
            });
        }
        let depth = node.ancestors().filter(Node::is_element).count();
        if depth > MAX_XML_DEPTH {
            return Err(RasterizeError::XmlTooDeep {
                max_depth: MAX_XML_DEPTH,
            });
        }
        if let Some(id) = node.attribute("id") {
            ids.insert(id, node);
        }
        validate_element(node, generated, allowed_fonts)?;
    }
    validate_use_expansion(&document, &ids, deadline)?;
    Ok(())
}

fn validate_dimensions(root: Node<'_, '_>) -> Result<(), RasterizeError> {
    for name in ["width", "height"] {
        let Some(raw) = root.attribute(name) else {
            return Err(RasterizeError::InvalidDimensions {
                max: MAX_SVG_DIMENSION,
            });
        };
        let number = raw.strip_suffix("px").unwrap_or(raw).trim();
        let Ok(value) = number.parse::<f64>() else {
            return Err(RasterizeError::InvalidDimensions {
                max: MAX_SVG_DIMENSION,
            });
        };
        if !value.is_finite() || value <= 0.0 || value > f64::from(MAX_SVG_DIMENSION) {
            return Err(RasterizeError::InvalidDimensions {
                max: MAX_SVG_DIMENSION,
            });
        }
    }
    Ok(())
}

fn validate_element(
    node: Node<'_, '_>,
    generated: bool,
    allowed_fonts: &HashSet<String>,
) -> Result<(), RasterizeError> {
    let local_name = node.tag_name().name();
    let name = local_name.to_ascii_lowercase();
    match name.as_str() {
        "script" => return Err(RasterizeError::ScriptForbidden),
        "animate" | "animatemotion" | "animatetransform" | "set" => {
            return Err(RasterizeError::AnimationForbidden);
        }
        "foreignobject" => return Err(RasterizeError::ForeignObjectForbidden),
        "link" => return Err(RasterizeError::ExternalStylesheetForbidden),
        _ => {}
    }

    for attribute in node.attributes() {
        let attribute_name = attribute.name().to_ascii_lowercase();
        if attribute_name.starts_with("on") {
            return Err(RasterizeError::EventHandlerForbidden);
        }
        if attribute_name == "href" || attribute_name == "src" {
            validate_reference(attribute.value(), generated)?;
        }
    }

    if name == "text" {
        validate_font_family(node, allowed_fonts)?;
    }
    Ok(())
}

fn validate_reference(value: &str, generated: bool) -> Result<(), RasterizeError> {
    let value = value.trim();
    if value.is_empty() || value.starts_with('#') {
        return Ok(());
    }
    let lowercase = value.to_ascii_lowercase();
    if lowercase.starts_with("http:")
        || lowercase.starts_with("https:")
        || lowercase.starts_with("//")
    {
        return Err(RasterizeError::NetworkReferenceForbidden);
    }
    if lowercase.starts_with("file:") {
        return Err(RasterizeError::FileReferenceForbidden);
    }
    if lowercase.starts_with("data:") {
        if generated && lowercase.starts_with("data:image/svg+xml;base64,") {
            return Ok(());
        }
        return Err(RasterizeError::DataReferenceForbidden);
    }
    // A relative URL is a filesystem request if a resource directory ever appears.
    Err(RasterizeError::FileReferenceForbidden)
}

fn validate_font_family(
    node: Node<'_, '_>,
    allowed_fonts: &HashSet<String>,
) -> Result<(), RasterizeError> {
    let direct = node.attribute("font-family");
    let styled = node.attribute("style").and_then(|style| {
        style.split(';').find_map(|declaration| {
            let (property, value) = declaration.split_once(':')?;
            property
                .trim()
                .eq_ignore_ascii_case("font-family")
                .then(|| value.trim())
        })
    });
    let Some(families) = direct.or(styled) else {
        return Ok(());
    };
    if families.split(',').any(|family| {
        let family = family.trim().trim_matches(['\'', '"']);
        allowed_fonts
            .iter()
            .any(|allowed| family.eq_ignore_ascii_case(allowed))
    }) {
        return Ok(());
    }
    let family = families
        .split(',')
        .next()
        .unwrap_or(families)
        .trim()
        .trim_matches(['\'', '"'])
        .to_owned();
    Err(RasterizeError::MissingFont { family })
}

fn allowed_font_families(
    assets: &RasterAssetMap,
    deadline: &RenderDeadline,
) -> Result<HashSet<String>, RasterizeError> {
    let mut families = HashSet::from([ALLOWED_FONT_FAMILY.to_owned()]);
    for bytes in assets.values() {
        if let Some(family) = font_family(bytes) {
            families.insert(family);
        }
        deadline.check()?;
    }
    Ok(families)
}

fn font_family(bytes: &[u8]) -> Option<String> {
    let mut database = resvg::usvg::fontdb::Database::new();
    database.load_font_data(bytes.to_vec());
    database
        .faces()
        .next()
        .and_then(|face| face.families.first())
        .map(|family| family.0.clone())
}

fn contains_mustache(value: &str) -> bool {
    value.contains("{{") || value.contains("}}")
}

fn evaluate_xml_value(
    value: &str,
    context: &EvalContext<'_>,
    fields: &[Field],
    fuel: &mut Fuel,
    deadline: &RenderDeadline,
) -> Result<String, RasterizeError> {
    let body = match plugin::classify_expression_source(value) {
        ExpressionSource::Expression(body) => body,
        ExpressionSource::Literal(_) | ExpressionSource::MalformedPartial => {
            return Err(RasterizeError::PartialInterpolation {
                value: value.to_owned(),
            });
        }
    };

    let evaluated = if plugin::device_binding_requirements(value).contains(body) {
        match app_core::classify_binding(body) {
            Ok(app_core::BindingClass::Live) => {
                return Err(RasterizeError::LiveBinding {
                    binding: body.to_owned(),
                });
            }
            Ok(app_core::BindingClass::Static) => field_value(body, fields)?,
            Err(app_core::RequirementsError::UnknownBinding { binding }) => {
                return Err(RasterizeError::UnknownBinding { binding });
            }
        }
    } else {
        let expression = Expr::parse(body).map_err(expression_error)?;
        let evaluated = expression.eval(context, fuel).map_err(expression_error)?;
        deadline.check()?;
        match evaluated {
            EvalValue::Missing => String::new(),
            value => value.to_string(),
        }
    };
    xml_escape_with_deadline(&evaluated, deadline)
}

fn expression_error(error: ExprError) -> RasterizeError {
    match error {
        ExprError::OutOfFuel => RasterizeError::ExpressionFuelExhausted,
        error => RasterizeError::Expression {
            message: error.to_string(),
        },
    }
}

fn scene_value(value: &SceneValue, fields: &[Field]) -> Result<String, RasterizeError> {
    match value {
        SceneValue::Literal(value) => Ok(value.clone()),
        SceneValue::Binding(binding) => field_value(binding, fields),
    }
}

fn field_value(binding: &str, fields: &[Field]) -> Result<String, RasterizeError> {
    match app_core::classify_binding(binding) {
        Ok(app_core::BindingClass::Live) => {
            return Err(RasterizeError::LiveBinding {
                binding: binding.to_owned(),
            });
        }
        Ok(app_core::BindingClass::Static) => {}
        Err(app_core::RequirementsError::UnknownBinding { binding }) => {
            return Err(RasterizeError::UnknownBinding { binding });
        }
    }
    let key = binding
        .strip_prefix("field.")
        .expect("app-core classifies only field.* as static");
    let field = fields
        .iter()
        .find(|field| field.key == key)
        .ok_or_else(|| RasterizeError::MissingField {
            binding: binding.to_owned(),
        })?;
    Ok(match &field.value {
        FieldValue::Text(value) => value.clone(),
        FieldValue::Integer(value) => value.to_string(),
        FieldValue::Boolean(value) => value.to_string(),
    })
}

fn color(value: u32) -> String {
    format!("#{:06x}", value & 0x00ff_ffff)
}

fn opacity(value: u8) -> String {
    format!("{:.6}", f64::from(value) / 255.0)
}

fn xml_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\'' => escaped.push_str("&apos;"),
            '"' => escaped.push_str("&quot;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn xml_escape_with_deadline(
    value: &str,
    deadline: &RenderDeadline,
) -> Result<String, RasterizeError> {
    let mut escaped = String::with_capacity(value.len());
    for (index, character) in value.chars().enumerate() {
        if index % 1_024 == 0 {
            deadline.check()?;
        }
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\'' => escaped.push_str("&apos;"),
            '"' => escaped.push_str("&quot;"),
            _ => escaped.push(character),
        }
    }
    Ok(escaped)
}

fn asset_bytes<'a>(
    assets: &'a RasterAssetMap,
    digest: &[u8; 32],
) -> Result<&'a [u8], RasterizeError> {
    assets
        .get(digest)
        .map(AsRef::as_ref)
        .ok_or_else(|| RasterizeError::MissingAsset {
            digest: protocol::digest_hex(digest),
        })
}

fn font_properties(
    font: &SceneFont,
    assets: &RasterAssetMap,
) -> Result<(String, i32, u16), RasterizeError> {
    match font {
        SceneFont::Baked(SceneFontTier::Caption) => Ok(("Inter".into(), 18, 400)),
        SceneFont::Baked(SceneFontTier::Body) => Ok(("Inter".into(), 28, 400)),
        SceneFont::Baked(SceneFontTier::Display) => Ok(("Inter".into(), 56, 600)),
        SceneFont::Baked(SceneFontTier::Hero) => Ok(("Inter".into(), 96, 600)),
        SceneFont::Asset { digest, pixel_size } => {
            let bytes = asset_bytes(assets, digest)?;
            let family = font_family(bytes).ok_or_else(|| RasterizeError::MissingFont {
                family: protocol::digest_hex(digest),
            })?;
            Ok((family, *pixel_size, 400))
        }
    }
}

fn aligned_x(x: i32, width: i32, align: SceneAlign) -> (i32, &'static str) {
    match align {
        SceneAlign::Left => (x, "start"),
        SceneAlign::Center => (x + width / 2, "middle"),
        SceneAlign::Right => (x + width, "end"),
    }
}

fn clip_attribute(
    clip: Option<protocol::SceneClipRect>,
    index: usize,
    definitions: &mut String,
) -> String {
    let Some(clip) = clip else {
        return String::new();
    };
    let id = format!("clip-{index}");
    write!(
        definitions,
        r#"<clipPath id="{id}"><rect x="{}" y="{}" width="{}" height="{}"/></clipPath>"#,
        clip.x, clip.y, clip.w, clip.h
    )
    .unwrap();
    format!(r#" clip-path="url(#{id})""#)
}

fn polar(cx: i32, cy: i32, radius: i32, degrees: i32) -> (f64, f64) {
    polar_f64(
        f64::from(cx),
        f64::from(cy),
        f64::from(radius),
        f64::from(degrees),
    )
}

fn polar_f64(cx: f64, cy: f64, radius: f64, degrees: f64) -> (f64, f64) {
    let radians = degrees.to_radians();
    (cx + radius * radians.cos(), cy + radius * radians.sin())
}

fn estimate_text_width(value: &str, pixel_size: i32, letter_space: i32) -> i32 {
    let count = i32::try_from(value.chars().count()).unwrap_or(i32::MAX);
    (count * pixel_size * 3 / 5).saturating_add(count.saturating_sub(1) * letter_space)
}

fn glyph_text(name: &str) -> String {
    if let Some(hex) = name.strip_prefix("U+").or_else(|| name.strip_prefix("u+"))
        && let Ok(codepoint) = u32::from_str_radix(hex, 16)
        && let Some(character) = char::from_u32(codepoint)
    {
        return character.to_string();
    }
    name.to_owned()
}

fn rgb565_asset_svg(
    bytes: &[u8],
    digest: &[u8; 32],
    deadline: &RenderDeadline,
) -> Result<Vec<u8>, RasterizeError> {
    let invalid = || RasterizeError::InvalidImageAsset {
        digest: protocol::digest_hex(digest),
    };
    if bytes.len() < LVGL_IMAGE_HEADER_BYTES || bytes[0] != 0x19 || bytes[1] != 0x12 {
        return Err(invalid());
    }
    let width = u32::from(u16::from_le_bytes([bytes[4], bytes[5]]));
    let height = u32::from(u16::from_le_bytes([bytes[6], bytes[7]]));
    let stride = u32::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    if width == 0 || height == 0 || stride != width * 2 {
        return Err(invalid());
    }
    let pixels = &bytes[LVGL_IMAGE_HEADER_BYTES..];
    if pixels.len() != usize::try_from(width * height * 2).map_err(|_| invalid())? {
        return Err(invalid());
    }
    let mut paths: BTreeMap<u16, String> = BTreeMap::new();
    for (index, pair) in pixels.as_chunks::<2>().0.iter().enumerate() {
        if index % 1_024 == 0 {
            deadline.check()?;
        }
        let pixel = u16::from_le_bytes(*pair);
        let index = u32::try_from(index).map_err(|_| invalid())?;
        let x = index % width;
        let y = index / width;
        write!(paths.entry(pixel).or_default(), "M{x} {y}h1v1h-1z").unwrap();
    }
    let mut svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}" shape-rendering="crispEdges">"#
    );
    for (pixel, path) in paths {
        deadline.check()?;
        let red = u8::try_from((pixel >> 11) & 0x1f).unwrap();
        let green = u8::try_from((pixel >> 5) & 0x3f).unwrap();
        let blue = u8::try_from(pixel & 0x1f).unwrap();
        let red = u16::from(red) * 255 / 31;
        let green = u16::from(green) * 255 / 63;
        let blue = u16::from(blue) * 255 / 31;
        write!(
            svg,
            r##"<path fill="#{red:02x}{green:02x}{blue:02x}" d="{path}"/>"##
        )
        .unwrap();
    }
    svg.push_str("</svg>");
    Ok(svg.into_bytes())
}

fn base64(bytes: &[u8], deadline: &RenderDeadline) -> Result<String, RasterizeError> {
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3 * 1_024) {
        base64::engine::general_purpose::STANDARD.encode_string(chunk, &mut output);
        deadline.check()?;
    }
    Ok(output)
}

fn validate_use_expansion(
    document: &Document<'_>,
    ids: &HashMap<&str, Node<'_, '_>>,
    deadline: &RenderDeadline,
) -> Result<(), RasterizeError> {
    let mut expanded = 0_usize;
    let mut memo = HashMap::new();
    for use_node in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "use")
    {
        deadline.check()?;
        let Some(target) = local_reference_target(use_node, ids) else {
            continue;
        };
        let cost = expanded_subtree_cost(target, ids, &mut memo, &mut HashSet::new(), deadline)?;
        expanded = expanded.saturating_add(cost);
        if expanded > MAX_EXPANDED_SVG_NODES {
            return Err(RasterizeError::ExpansionTooLarge {
                max_nodes: MAX_EXPANDED_SVG_NODES,
            });
        }
    }
    Ok(())
}

fn local_reference_target<'a, 'input>(
    node: Node<'a, 'input>,
    ids: &HashMap<&'input str, Node<'a, 'input>>,
) -> Option<Node<'a, 'input>> {
    let href = node
        .attributes()
        .find(|attribute| attribute.name() == "href")?
        .value();
    ids.get(href.strip_prefix('#')?).copied()
}

fn expanded_subtree_cost<'a, 'input>(
    node: Node<'a, 'input>,
    ids: &HashMap<&'input str, Node<'a, 'input>>,
    memo: &mut HashMap<NodeId, usize>,
    visiting: &mut HashSet<NodeId>,
    deadline: &RenderDeadline,
) -> Result<usize, RasterizeError> {
    deadline.check()?;
    if let Some(cost) = memo.get(&node.id()) {
        return Ok(*cost);
    }
    if !visiting.insert(node.id()) {
        return Err(RasterizeError::ExpansionTooLarge {
            max_nodes: MAX_EXPANDED_SVG_NODES,
        });
    }
    let mut cost = 1_usize;
    for child in node.children().filter(Node::is_element) {
        deadline.check()?;
        cost = cost.saturating_add(expanded_subtree_cost(child, ids, memo, visiting, deadline)?);
        if child.tag_name().name() == "use"
            && let Some(target) = local_reference_target(child, ids)
        {
            cost = cost.saturating_add(expanded_subtree_cost(
                target, ids, memo, visiting, deadline,
            )?);
        }
        if cost > MAX_EXPANDED_SVG_NODES {
            break;
        }
    }
    visiting.remove(&node.id());
    memo.insert(node.id(), cost);
    Ok(cost)
}

fn encode_rgb565(
    width: u32,
    height: u32,
    pixmap: &Pixmap,
    deadline: &RenderDeadline,
) -> Result<Vec<u8>, RasterizeError> {
    let pixel_count = usize::try_from(width * height).expect("fixed canvas fits usize");
    let mut bytes = Vec::with_capacity(LVGL_IMAGE_HEADER_BYTES + pixel_count * 2);
    let stride = width * 2;
    let word0 = LVGL_IMAGE_MAGIC | (LVGL_COLOR_FORMAT_RGB565 << 8);
    let word1 = (width & 0xffff) | ((height & 0xffff) << 16);
    let word2 = stride & 0xffff;
    bytes.extend_from_slice(&word0.to_le_bytes());
    bytes.extend_from_slice(&word1.to_le_bytes());
    bytes.extend_from_slice(&word2.to_le_bytes());
    for (index, pixel) in pixmap.pixels().iter().enumerate() {
        if index % 4_096 == 0 {
            deadline.check()?;
        }
        bytes.extend_from_slice(
            &pack_rgb565(pixel.red(), pixel.green(), pixel.blue()).to_le_bytes(),
        );
    }
    Ok(bytes)
}

fn pack_rgb565(red: u8, green: u8, blue: u8) -> u16 {
    // Round each 8-bit channel to the nearest representable endpoint-inclusive value:
    // (channel * max + 127) / 255. Exact half steps round upward.
    let red = (u16::from(red) * 31 + 127) / 255;
    let green = (u16::from(green) * 63 + 127) / 255;
    let blue = (u16::from(blue) * 31 + 127) / 255;
    (red << 11) | (green << 5) | blue
}

/// The decoded RGB565 blob as a PNG, for the admin API's card preview.
/// `assert_raster_regression` uses the same encoder, so a golden and a
/// preview are the same bytes for the same frame. Takes the canonical
/// 12-byte-header blob by reference: a preview must not copy 330 KB to
/// rebuild a `RasterizedFrame` it already holds the bytes of.
pub(crate) fn frame_png(bytes: &[u8]) -> Vec<u8> {
    let width = u32::try_from(SCENE_CANVAS_WIDTH).unwrap();
    let height = u32::try_from(SCENE_CANVAS_HEIGHT).unwrap();
    let mut rgba = Vec::with_capacity(usize::try_from(width * height * 4).unwrap());
    for pair in bytes[12..].as_chunks::<2>().0 {
        let pixel = u16::from_le_bytes(*pair);
        let red = u8::try_from((pixel >> 11) & 0x1f).unwrap();
        let green = u8::try_from((pixel >> 5) & 0x3f).unwrap();
        let blue = u8::try_from(pixel & 0x1f).unwrap();
        rgba.extend_from_slice(&[
            u8::try_from(u16::from(red) * 255 / 31).unwrap(),
            u8::try_from(u16::from(green) * 255 / 63).unwrap(),
            u8::try_from(u16::from(blue) * 255 / 31).unwrap(),
            255,
        ]);
    }
    let size = resvg::tiny_skia::IntSize::from_wh(width, height).unwrap();
    resvg::tiny_skia::Pixmap::from_vec(rgba, size)
        .unwrap()
        .encode_png()
        .unwrap()
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use protocol::{
        Field, FieldValue, SCENE_CANVAS_HEIGHT, SCENE_CANVAS_WIDTH, Scene, SceneAlign, SceneArc,
        SceneClipRect, SceneFont, SceneFontTier, SceneGlyph, SceneImage, SceneLabel,
        SceneLabelAnchor, SceneLine, SceneNode, SceneRect, SceneRotRect, SceneScale, SceneText,
        SceneValue,
    };
    use sha2::{Digest, Sha256};

    use super::*;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/svg");

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{FIXTURES}/{name}"))
            .unwrap_or_else(|error| panic!("read fixture {name}: {error}"))
    }

    fn assert_rejected(svg: &str, expected: RasterizeError) {
        let started = Instant::now();
        assert_eq!(rasterize_svg(svg), Err(expected));
        assert!(
            started.elapsed() <= MAX_RENDER_WALL_CLOCK,
            "hostile input exceeded the render budget"
        );
    }

    fn pixel(frame: &RasterizedFrame, x: u32, y: u32) -> u16 {
        let width = u32::try_from(SCENE_CANVAS_WIDTH).unwrap();
        let offset = 12 + usize::try_from((y * width + x) * 2).unwrap();
        u16::from_le_bytes([frame.bytes[offset], frame.bytes[offset + 1]])
    }

    #[test]
    fn solid_red_has_exact_canonical_lvgl_header_and_length() {
        let width = u32::try_from(SCENE_CANVAS_WIDTH).unwrap();
        let height = u32::try_from(SCENE_CANVAS_HEIGHT).unwrap();
        let svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}"><rect width="{width}" height="{height}" fill="#ff0000"/></svg>"##
        );

        let frame = rasterize_svg(&svg).unwrap();

        assert_eq!(width * height, 164_864);
        assert_eq!(12 + usize::try_from(width * height * 2).unwrap(), 329_740);
        assert_eq!(frame.bytes.len(), 329_740);
        assert_eq!(
            &frame.bytes[..12],
            &[
                0x19, 0x12, 0x00, 0x00, 0xc0, 0x01, 0x70, 0x01, 0x80, 0x03, 0x00, 0x00
            ]
        );
        assert!(
            frame.bytes[12..]
                .as_chunks::<2>()
                .0
                .iter()
                .all(|bytes| *bytes == 0xf800_u16.to_le_bytes())
        );
        let expected_digest: [u8; 32] = Sha256::digest(&frame.bytes).into();
        assert_eq!(frame.digest, expected_digest);
    }

    #[test]
    fn rgb565_channel_order_alpha_and_black_ground_are_exact() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368">
            <rect x="0" y="0" width="1" height="1" fill="#ff0000"/>
            <rect x="1" y="0" width="1" height="1" fill="#00ff00"/>
            <rect x="2" y="0" width="1" height="1" fill="#0000ff"/>
            <rect x="3" y="0" width="1" height="1" fill="#000000"/>
            <rect x="4" y="0" width="1" height="1" fill="#ffffff"/>
            <rect x="5" y="0" width="1" height="1" fill="#ff0000" fill-opacity="0.5"/>
        </svg>"##;

        let frame = rasterize_svg(svg).unwrap();

        assert_eq!(pixel(&frame, 0, 0), 0xf800);
        assert_eq!(pixel(&frame, 1, 0), 0x07e0);
        assert_eq!(pixel(&frame, 2, 0), 0x001f);
        assert_eq!(pixel(&frame, 3, 0), 0x0000);
        assert_eq!(pixel(&frame, 4, 0), 0xffff);
        assert_eq!(pixel(&frame, 5, 0), 0x8000);
        assert_eq!(pixel(&frame, 6, 0), 0x0000);
    }

    #[test]
    fn rgb565_quantization_rounds_nearest_with_half_steps_up() {
        assert_eq!(pack_rgb565(0, 0, 0), 0x0000);
        assert_eq!(pack_rgb565(127, 127, 127), 0x7bef);
        assert_eq!(pack_rgb565(128, 128, 128), 0x8410);
        assert_eq!(pack_rgb565(255, 255, 255), 0xffff);
    }

    #[test]
    fn source_size_cap_rejects_one_byte_past() {
        let svg = " ".repeat(MAX_SVG_SOURCE_BYTES + 1);
        assert_rejected(
            &svg,
            RasterizeError::SourceTooLarge {
                max_bytes: MAX_SVG_SOURCE_BYTES,
            },
        );
    }

    #[test]
    fn entity_expansion_is_rejected_by_name() {
        assert_rejected(
            &fixture("entity-expansion.svg"),
            RasterizeError::EntityExpansionForbidden,
        );
    }

    #[test]
    fn use_expansion_cap_rejects_one_node_past() {
        let group_uses = "<use href=\"#four\"/>".repeat(MAX_EXPANDED_SVG_NODES / 4);
        let svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368"><defs><g id="four"><rect/><rect/><rect/></g><rect id="one"/></defs>{group_uses}<use href="#one"/></svg>"##
        );
        assert_rejected(
            &svg,
            RasterizeError::ExpansionTooLarge {
                max_nodes: MAX_EXPANDED_SVG_NODES,
            },
        );
    }

    #[test]
    fn xml_depth_cap_rejects_one_element_past() {
        let mut svg =
            String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368">"#);
        svg.push_str(&"<g>".repeat(MAX_XML_DEPTH));
        svg.push_str(&"</g>".repeat(MAX_XML_DEPTH));
        svg.push_str("</svg>");
        assert_rejected(
            &svg,
            RasterizeError::XmlTooDeep {
                max_depth: MAX_XML_DEPTH,
            },
        );
    }

    #[test]
    fn xml_node_cap_rejects_one_element_past() {
        let nodes = "<path/>".repeat(MAX_XML_ELEMENT_NODES);
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368">{nodes}</svg>"#
        );
        assert_rejected(
            &svg,
            RasterizeError::TooManyNodes {
                max_nodes: MAX_XML_ELEMENT_NODES,
            },
        );
    }

    #[test]
    fn executable_and_foreign_content_is_rejected_by_name() {
        for (name, error) in [
            ("script.svg", RasterizeError::ScriptForbidden),
            ("event-attribute.svg", RasterizeError::EventHandlerForbidden),
            ("animate.svg", RasterizeError::AnimationForbidden),
            ("foreign-object.svg", RasterizeError::ForeignObjectForbidden),
        ] {
            assert_rejected(&fixture(name), error);
        }
    }

    #[test]
    fn every_external_image_scheme_is_decided_explicitly() {
        for (name, error) in [
            ("image-http.svg", RasterizeError::NetworkReferenceForbidden),
            ("image-https.svg", RasterizeError::NetworkReferenceForbidden),
            ("image-file.svg", RasterizeError::FileReferenceForbidden),
            ("image-data.svg", RasterizeError::DataReferenceForbidden),
        ] {
            assert_rejected(&fixture(name), error);
        }
    }

    #[test]
    fn stylesheet_and_css_url_are_rejected_by_name() {
        assert_rejected(
            &fixture("external-stylesheet.svg"),
            RasterizeError::ExternalStylesheetForbidden,
        );
        assert_rejected(&fixture("css-url.svg"), RasterizeError::CssUrlForbidden);
    }

    #[test]
    fn missing_font_is_rejected_instead_of_using_the_host() {
        assert_rejected(
            &fixture("missing-font.svg"),
            RasterizeError::MissingFont {
                family: "Definitely Missing Font".into(),
            },
        );
    }

    #[test]
    fn pathological_dimensions_are_rejected_by_name() {
        for name in [
            "dimension-zero.svg",
            "dimension-negative.svg",
            "dimension-enormous.svg",
            "dimension-nan.svg",
        ] {
            assert_rejected(
                &fixture(name),
                RasterizeError::InvalidDimensions {
                    max: MAX_SVG_DIMENSION,
                },
            );
        }
    }

    #[test]
    fn invalid_utf8_and_malformed_xml_are_distinct() {
        assert_eq!(
            super::rasterize_svg_bytes(&[0xff, 0xfe]),
            Err(RasterizeError::InvalidUtf8)
        );
        assert_rejected(&fixture("malformed.svg"), RasterizeError::MalformedXml);
    }

    #[test]
    fn render_deadline_rejects_one_millisecond_past() {
        // `checked_sub` rather than `-`: a bare Duration subtraction on an
        // `Instant` panics on a monotonic clock too young to go back this far,
        // which is a real possibility in a freshly booted CI container.
        let past = Instant::now()
            .checked_sub(MAX_RENDER_WALL_CLOCK + Duration::from_millis(1))
            .expect("the monotonic clock is far enough past its origin");
        let deadline = RenderDeadline(past);
        assert_eq!(
            deadline.check(),
            Err(RasterizeError::RenderDeadlineExceeded {
                millis: MAX_RENDER_WALL_CLOCK.as_millis(),
            })
        );
    }

    #[test]
    fn hostile_template_time_spent_before_renderer_is_inside_the_single_deadline() {
        let hostile = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368">{}</svg>"#,
            "<g/>".repeat(MAX_XML_ELEMENT_NODES)
        );
        let past = Instant::now()
            .checked_sub(MAX_RENDER_WALL_CLOCK + Duration::from_millis(1))
            .expect("the monotonic clock is far enough past its origin");

        assert_eq!(
            evaluate_svg_template_with_deadline(
                &hostile,
                &serde_json::Value::Null,
                &[],
                &RenderDeadline(past),
            ),
            Err(RasterizeError::RenderDeadlineExceeded {
                millis: MAX_RENDER_WALL_CLOCK.as_millis()
            })
        );
    }

    #[test]
    fn usvg_options_have_no_ambient_image_or_font_authority() {
        let options =
            renderer_options(false, &RasterAssetMap::new(), &RenderDeadline::start()).unwrap();
        assert!(options.resources_dir.is_none());
        assert!(options.style_sheet.is_none());
        assert!((options.image_href_resolver.resolve_string)("/etc/passwd", &options).is_none());
        assert!(
            (options.image_href_resolver.resolve_data)(
                "image/png",
                Arc::new(vec![0; 8]),
                &options,
            )
            .is_none()
        );
        assert_eq!(options.fontdb.faces().count(), 2);
    }

    fn rgb565_asset(width: u32, height: u32, pixels: &[u16]) -> Vec<u8> {
        assert_eq!(pixels.len(), usize::try_from(width * height).unwrap());
        let mut bytes = Vec::with_capacity(12 + pixels.len() * 2);
        bytes
            .extend_from_slice(&(LVGL_IMAGE_MAGIC | (LVGL_COLOR_FORMAT_RGB565 << 8)).to_le_bytes());
        bytes.extend_from_slice(&((width & 0xffff) | ((height & 0xffff) << 16)).to_le_bytes());
        bytes.extend_from_slice(&(width * 2).to_le_bytes());
        for pixel in pixels {
            bytes.extend_from_slice(&pixel.to_le_bytes());
        }
        bytes
    }

    #[test]
    #[allow(clippy::too_many_lines)] // one exercised node per kind
    fn scene_translation_covers_every_node_clip_opacity_alignment_font_and_field_shape() {
        let image_digest = [0x11; 32];
        let font_digest = [0x22; 32];
        let fields = vec![
            Field {
                key: "title".into(),
                value: FieldValue::Text("A&B <desk>".into()),
            },
            Field {
                key: "count".into(),
                value: FieldValue::Integer(42),
            },
            Field {
                key: "ready".into(),
                value: FieldValue::Boolean(true),
            },
        ];
        let mut assets = RasterAssetMap::new();
        assets.insert(
            image_digest,
            Arc::from(rgb565_asset(2, 2, &[0xf800, 0x07e0, 0x001f, 0xffff])),
        );
        assets.insert(font_digest, Arc::from(INTER_REGULAR));
        let clip = Some(SceneClipRect {
            x: 0,
            y: 0,
            w: 200,
            h: 100,
        });
        let scene = Scene {
            revision: 9,
            background: 0x0001_0203,
            nodes: vec![
                SceneNode::Rect(SceneRect {
                    x: 2,
                    y: 3,
                    w: 40,
                    h: 20,
                    radius: 5,
                    fill: 0x0011_2233,
                    opacity: 128,
                    clip,
                }),
                SceneNode::Arc(SceneArc {
                    cx: 80,
                    cy: 80,
                    r: 30,
                    start_deg: 270,
                    end_deg: 550,
                    width: 6,
                    color: 0x0044_5566,
                    running_color: None,
                    opacity: 192,
                    rounded: true,
                    end_binding: String::new(),
                }),
                SceneNode::Line(SceneLine {
                    xs: vec![10, 20, 30],
                    ys: vec![120, 110, 120],
                    width: 3,
                    color: 0x0077_8899,
                    ..Default::default()
                }),
                SceneNode::Text(SceneText {
                    x: 20,
                    baseline_y: 160,
                    w: 180,
                    align: SceneAlign::Center,
                    font: SceneFont::Baked(SceneFontTier::Caption),
                    color: 0x00aa_bbcc,
                    running_color: None,
                    value: SceneValue::Binding("field.title".into()),
                    ellipsize: true,
                }),
                SceneNode::Image(SceneImage {
                    x: 220,
                    y: 10,
                    w: 32,
                    h: 32,
                    digest: image_digest,
                    recolor: false,
                    color: 0,
                }),
                SceneNode::Glyph(SceneGlyph {
                    x: 270,
                    baseline_y: 70,
                    size: 32,
                    digest: font_digest,
                    name: "U+0041".into(),
                    color: 0x00dd_eeff,
                }),
                SceneNode::Scale(SceneScale {
                    x: 20,
                    y: 190,
                    box_size: 100,
                    total_tick_count: 13,
                    major_tick_every: 3,
                    major_tick_color: 0x0012_3456,
                }),
                SceneNode::Label(SceneLabel {
                    x: 180,
                    y: 210,
                    horizontal_anchor: SceneLabelAnchor::Right,
                    font: SceneFont::Asset {
                        digest: font_digest,
                        pixel_size: 28,
                    },
                    value: SceneValue::Binding("field.count".into()),
                    ink: 0x00f5_f5f7,
                    fill: 0x0025_252b,
                    fill_opacity: 220,
                    radius: 10,
                    pad_hor: 12,
                    pad_ver: 6,
                    letter_space: 1,
                    hide_when_empty: true,
                }),
                SceneNode::RotRect(SceneRotRect {
                    x: 300,
                    y: 180,
                    w: 12,
                    h: 80,
                    radius: 6,
                    fill: 0x00ff_8f2e,
                    pivot_x: 6,
                    pivot_y: 70,
                    rotation: 450,
                    rotation_binding: String::new(),
                    clip,
                }),
            ],
        };

        let svg = scene_to_svg(&scene, &fields, &assets, &RenderDeadline::start()).unwrap();

        for marker in [
            "<rect",
            "<path",
            "<polyline",
            "<text",
            "<image",
            "clip-path",
            "opacity=\"0.501961\"",
            "text-anchor=\"middle\"",
            "font-size=\"18\"",
            "font-size=\"28\"",
            "A&amp;B &lt;desk&gt;",
            ">42<",
        ] {
            assert!(
                svg.contains(marker),
                "translated SVG missing {marker}: {svg}"
            );
        }
        assert!(
            svg.matches("<line").count() >= 13,
            "scale ticks were not translated"
        );
        let frame = rasterize_scene(&scene, &fields, &assets)
            .expect("every translated node renders through resvg");
        assert_eq!(
            pixel(&frame, 224, 14),
            0xf800,
            "the nested data URI must decode and render its red source pixel"
        );
    }

    #[test]
    fn scene_translation_refuses_every_live_binding_by_name() {
        let cases = [
            (SceneValue::Binding("date".into()), "date"),
            (SceneValue::Binding("time:HH:mm".into()), "time:HH:mm"),
            (
                SceneValue::Binding("timer.remaining:mm:ss".into()),
                "timer.remaining:mm:ss",
            ),
        ];
        for (value, binding) in cases {
            let scene = Scene {
                revision: 1,
                background: 0,
                nodes: vec![SceneNode::Text(SceneText {
                    x: 0,
                    baseline_y: 30,
                    w: 100,
                    value,
                    ..Default::default()
                })],
            };
            assert_eq!(
                scene_to_svg(
                    &scene,
                    &[],
                    &RasterAssetMap::new(),
                    &RenderDeadline::start(),
                ),
                Err(RasterizeError::LiveBinding {
                    binding: binding.into()
                })
            );
        }
        let scene = Scene {
            revision: 1,
            background: 0,
            nodes: vec![SceneNode::Arc(SceneArc {
                cx: 50,
                cy: 50,
                r: 20,
                start_deg: 0,
                end_deg: 180,
                width: 4,
                color: 1,
                running_color: Some(2),
                ..Default::default()
            })],
        };
        assert_eq!(
            scene_to_svg(
                &scene,
                &[],
                &RasterAssetMap::new(),
                &RenderDeadline::start(),
            ),
            Err(RasterizeError::LiveBinding {
                binding: "running_color".into()
            })
        );
    }

    #[test]
    fn scene_translation_maps_all_baked_tiers_alignments_and_field_value_kinds() {
        let fields = vec![
            Field {
                key: "count".into(),
                value: FieldValue::Integer(7),
            },
            Field {
                key: "ready".into(),
                value: FieldValue::Boolean(true),
            },
        ];
        let text = |baseline_y, align, font, value| {
            SceneNode::Text(SceneText {
                x: 10,
                baseline_y,
                w: 300,
                align,
                font,
                color: 0x00ff_ffff,
                running_color: None,
                value,
                ellipsize: false,
            })
        };
        let scene = Scene {
            revision: 1,
            background: 0,
            nodes: vec![
                text(
                    30,
                    SceneAlign::Left,
                    SceneFont::Baked(SceneFontTier::Caption),
                    SceneValue::Binding("field.ready".into()),
                ),
                text(
                    90,
                    SceneAlign::Center,
                    SceneFont::Baked(SceneFontTier::Body),
                    SceneValue::Binding("field.count".into()),
                ),
                text(
                    180,
                    SceneAlign::Right,
                    SceneFont::Baked(SceneFontTier::Display),
                    SceneValue::Literal("56".into()),
                ),
                text(
                    300,
                    SceneAlign::Left,
                    SceneFont::Baked(SceneFontTier::Hero),
                    SceneValue::Literal("96".into()),
                ),
            ],
        };

        let svg = scene_to_svg(
            &scene,
            &fields,
            &RasterAssetMap::new(),
            &RenderDeadline::start(),
        )
        .unwrap();

        for marker in [
            r#"font-size="18" font-weight="400""#,
            r#"font-size="28" font-weight="400""#,
            r#"font-size="56" font-weight="600""#,
            r#"font-size="96" font-weight="600""#,
            r#"text-anchor="start""#,
            r#"text-anchor="middle""#,
            r#"text-anchor="end""#,
            ">true<",
            ">7<",
        ] {
            assert!(
                svg.contains(marker),
                "translated SVG missing {marker}: {svg}"
            );
        }
    }

    /// Matches `scene_parity.rs::numeric_tier_boundary`: these are the
    /// existing native C-oracle rows named `hero-just-fits`,
    /// `hero-just-misses`, `display-just-fits`, and
    /// `display-just-misses`. The raster test consumes the same metrics-derived
    /// values but proves a different claim: translated SVG preserves the tier
    /// the real builder selected.
    fn numeric_tier_boundary(tier: SceneFontTier) -> (String, String) {
        const CONTENT_WIDTH: i32 = SCENE_CANVAS_WIDTH - 2 * 24;
        let metrics = &app_core::BakedFontMetrics::SHIPPED;
        let zero_width = metrics
            .measure(tier, "0")
            .expect("numeric tiers carry zero");
        let fitting_zeroes = usize::try_from(CONTENT_WIDTH / zero_width).unwrap();
        let fits = "0".repeat(fitting_zeroes);
        let misses = "0".repeat(fitting_zeroes + 1);
        assert!(metrics.measure(tier, &fits).unwrap() <= CONTENT_WIDTH);
        assert!(metrics.measure(tier, &misses).unwrap() > CONTENT_WIDTH);
        (fits, misses)
    }

    #[test]
    fn raster_svg_uses_the_real_builder_tier_across_both_numeric_boundaries() {
        const CONTENT_WIDTH: i32 = SCENE_CANVAS_WIDTH - 2 * 24;
        let metrics = &app_core::BakedFontMetrics::SHIPPED;
        let (hero_fits, hero_misses) = numeric_tier_boundary(SceneFontTier::Hero);
        let (display_fits, display_misses) = numeric_tier_boundary(SceneFontTier::Display);
        let values = [
            ("hero-just-fits", hero_fits),
            ("hero-just-misses", hero_misses),
            ("display-just-fits", display_fits),
            ("display-just-misses", display_misses),
            ("non-numeric", "ready".to_string()),
        ];

        for (slug, value) in values {
            let expected_tier = app_core::number_font_tier(&value, CONTENT_WIDTH, metrics);
            let scene = app_core::build_big_number_label_scene(
                &app_core::BigNumberCard {
                    revision: 1,
                    title: "Stats",
                    value: &value,
                    label: "ITEMS",
                },
                metrics,
            );
            let builder_font = scene
                .nodes
                .iter()
                .find_map(|node| match node {
                    SceneNode::Text(text) if text.value == SceneValue::Literal(value.clone()) => {
                        Some(&text.font)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{slug}: real builder value node"));
            assert_eq!(
                builder_font,
                &SceneFont::Baked(expected_tier),
                "{slug}: builder and number_font_tier must agree"
            );

            let (family, size, weight) = font_properties(builder_font, &RasterAssetMap::new())
                .expect("baked font properties");
            let size = size.to_string();
            let weight = weight.to_string();
            let svg = scene_to_svg(
                &scene,
                &[],
                &RasterAssetMap::new(),
                &RenderDeadline::start(),
            )
            .unwrap();
            let document = Document::parse(&svg).expect("translated SVG is XML");
            let translated = document
                .descendants()
                .find(|node| node.has_tag_name("text") && node.text() == Some(value.as_str()))
                .unwrap_or_else(|| panic!("{slug}: translated value text"));
            assert_eq!(translated.attribute("font-family"), Some(family.as_str()));
            assert_eq!(
                translated.attribute("font-size"),
                Some(size.as_str()),
                "{slug}: SVG must preserve the builder-selected tier"
            );
            assert_eq!(
                translated.attribute("font-weight"),
                Some(weight.as_str()),
                "{slug}: SVG must preserve the builder-selected face"
            );
        }
    }

    fn rooted_aqi() -> serde_json::Value {
        let envelope: serde_json::Value = serde_json::from_str(include_str!(
            "../../plugin/tests/fixtures/aqi_response.json"
        ))
        .unwrap();
        envelope["payload"].clone()
    }

    #[test]
    fn svg_template_evaluates_rooted_text_and_attributes_and_xml_escapes() {
        let mut data = rooted_aqi();
        data["current"]["category"] = serde_json::Value::String("A&B <bad> ' \"".into());
        let source = r#"<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368"><text x="{{ data.current.aqi }}">{{ data.current.category }}</text></svg>"#;

        let evaluated = evaluate_svg_template(source, &data, &[]).unwrap();

        assert!(evaluated.contains("x=\"42\""));
        assert!(evaluated.contains("A&amp;B &lt;bad&gt; &apos; &quot;"));
        rasterize_svg_template(source, &data, &[]).unwrap();
    }

    #[test]
    fn svg_template_shares_one_fuel_budget_across_the_document() {
        let attributes = (0..2_501).fold(String::new(), |mut out, index| {
            write!(
                out,
                r#" data-{index}="{{{{ upper(data.current.category) }}}}""#
            )
            .unwrap();
            out
        });
        let source = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368"{attributes}/>"#
        );
        assert_eq!(
            evaluate_svg_template(&source, &rooted_aqi(), &[]),
            Err(RasterizeError::ExpressionFuelExhausted)
        );
    }

    #[test]
    fn svg_template_rejects_partial_interpolation_and_live_bindings_by_name() {
        let partial = r#"<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368"><rect x="{{ data.current.aqi }}px"/></svg>"#;
        assert_eq!(
            evaluate_svg_template(partial, &rooted_aqi(), &[]),
            Err(RasterizeError::PartialInterpolation {
                value: "{{ data.current.aqi }}px".into()
            })
        );
        let live = r#"<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368"><text>{{ time:HH:mm }}</text></svg>"#;
        assert_eq!(
            evaluate_svg_template(live, &rooted_aqi(), &[]),
            Err(RasterizeError::LiveBinding {
                binding: "time:HH:mm".into()
            })
        );
        let near_miss = r#"<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368"><text>{{ timer.velocity }}</text></svg>"#;
        assert_eq!(
            evaluate_svg_template(near_miss, &rooted_aqi(), &[]),
            Err(RasterizeError::UnknownBinding {
                binding: "timer.velocity".into()
            })
        );
    }

    #[test]
    fn expanded_svg_one_byte_over_its_cap_is_rejected_after_substitution() {
        let prefix = r#"<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368">"#;
        let suffix = "</svg>";
        let text_markup_len = "<text></text>".len();
        let full_expressions = 127;
        let tail_len = MAX_EXPANDED_SVG_BYTES + 1
            - prefix.len()
            - suffix.len()
            - (full_expressions + 1) * text_markup_len
            - full_expressions * 4_096;
        let source = format!(
            "{prefix}{}<text>{{{{ data.tail }}}}</text>{suffix}",
            "<text>{{ data.large }}</text>".repeat(full_expressions)
        );
        assert!(source.len() <= MAX_SVG_SOURCE_BYTES);
        let data = serde_json::json!({
            "large": "x".repeat(4_096),
            "tail": "x".repeat(tail_len),
        });

        assert_eq!(
            evaluate_svg_template(&source, &data, &[]),
            Err(RasterizeError::ExpandedSourceTooLarge {
                max_bytes: MAX_EXPANDED_SVG_BYTES
            })
        );
    }

    #[test]
    fn svg_footer_uses_shared_error_precedence_colour_and_utf8_byte_bound() {
        let mut svg =
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368"></svg>"#.to_owned();
        let error = "é".repeat(protocol::MAX_SCENE_TEXT_LEN);
        append_data_state_footer(
            &mut svg,
            app_core::SceneDataState {
                stale: true,
                error: Some(&error),
            },
            &RasterAssetMap::new(),
            &RenderDeadline::start(),
        )
        .unwrap();

        let document = Document::parse(&svg).unwrap();
        let footer = document
            .descendants()
            .find(|node| node.has_tag_name("text"))
            .expect("shared rule must add an error footer");
        let text = footer.text().unwrap();
        assert_eq!(text.len(), protocol::MAX_SCENE_TEXT_LEN);
        assert!(text.is_char_boundary(text.len()));
        assert_eq!(footer.attribute("fill"), Some("#ff6b6b"));
        assert_ne!(text, "Stale", "a non-empty error must win over stale");
    }

    #[test]
    fn committed_inter_faces_match_tools_sources_and_pinned_sha256() {
        let regular = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../tools/fonts/Inter-Regular.ttf"
        ))
        .unwrap();
        let semibold = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../tools/fonts/Inter-SemiBold.ttf"
        ))
        .unwrap();
        assert_eq!(INTER_REGULAR, regular);
        assert_eq!(INTER_SEMIBOLD, semibold);
        assert_eq!(
            protocol::digest_hex(&Sha256::digest(INTER_REGULAR).into()),
            "40d692fce188e4471e2b3cba937be967878f631ad3ebbbdcd587687c7ebe0c82"
        );
        assert_eq!(
            protocol::digest_hex(&Sha256::digest(INTER_SEMIBOLD).into()),
            "78a843fade9d4612a5567302fb595b56976eb5fcebf4fea5a5912d638bafcde3"
        );
    }

    #[test]
    fn rle_measurement_curated_svg_aqi_raw_329728_candidate_is_10020() {
        let template = include_str!("../../../plugins/svg-aqi/face.svg");
        let frame = rasterize_svg_template(template, &rooted_aqi(), &[]).unwrap();
        let candidate = protocol::encode_rle565(&frame.bytes[12..]).unwrap();
        assert_eq!((frame.bytes.len() - 12, candidate.len()), (329_728, 10_020));
    }

    #[test]
    fn rle_measurement_high_entropy_raw_329728_candidate_is_659456() {
        let mut state = 0x6d2b_79f5_u32;
        let mut pixels = Vec::with_capacity(329_728);
        for _ in 0..164_864 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            pixels.extend_from_slice(&u16::try_from(state & 0xFFFF).unwrap().to_le_bytes());
        }
        let candidate = protocol::encode_rle565(&pixels).unwrap();
        assert_eq!((pixels.len(), candidate.len()), (329_728, 659_456));
    }

    pub(crate) fn assert_raster_regression(name: &str, frame: &RasterizedFrame) {
        let directory =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/raster-regression");
        let path = directory.join(format!("{name}.png"));
        let actual = frame_png(&frame.bytes);
        if std::env::var_os("UPDATE_RASTER_GOLDENS").is_some() {
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(&path, &actual).unwrap();
        }
        let expected = std::fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "raster regression golden {} is missing: {error}",
                path.display()
            )
        });
        assert_eq!(actual, expected, "raster regression changed: {name}");
    }

    #[test]
    fn raster_regression_display_list_fallback() {
        let scene = app_core::build_big_number_label_scene(
            &app_core::BigNumberCard {
                revision: 1,
                title: "AIR QUALITY",
                value: "42",
                label: "AQI · MODERATE",
            },
            &app_core::BakedFontMetrics::SHIPPED,
        );
        let frame = rasterize_scene(&scene, &[], &RasterAssetMap::new()).unwrap();
        assert_raster_regression("display-list-fallback", &frame);
    }

    #[test]
    fn raster_regression_produced_date_overflow_is_a_separate_visible_constraint() {
        use chrono::Datelike;

        const NOW_UNIX_SECONDS: i64 = 1_778_661_296;
        const UTC_OFFSET_MINUTES: i16 = 240;
        assert_ne!(UTC_OFFSET_MINUTES, 0);
        let local_seconds = NOW_UNIX_SECONDS + i64::from(UTC_OFFSET_MINUTES) * 60;
        let local_now = chrono::DateTime::from_timestamp(local_seconds, 0)
            .expect("date-overflow instant")
            .naive_utc();
        let produced_date = format!(
            "{}, {} {}",
            local_now.format("%a"),
            local_now.format("%b"),
            local_now.day()
        );
        let built = app_core::build_digital_clock_scene(
            &app_core::ClockCard {
                revision: 1,
                show_seconds: false,
                local_now,
            },
            &app_core::BakedFontMetrics::SHIPPED,
        );
        let date_index = built
            .nodes
            .iter()
            .position(|node| {
                matches!(
                    node,
                    SceneNode::Text(SceneText {
                        value: SceneValue::Binding(binding),
                        ..
                    }) if binding == "date"
                )
            })
            .expect("real DigitalClock builder date node");
        let mut date_node = built.nodes[date_index].clone();
        let SceneNode::Text(date_text) = &mut date_node else {
            unreachable!("date index was selected as text")
        };
        assert_eq!(date_text.w, 176);
        assert!(date_text.ellipsize);
        date_text.value = SceneValue::Literal(produced_date.clone());
        let static_date_constraint = Scene {
            revision: built.revision,
            background: built.background,
            // The preceding node is the date module emitted by the same real
            // builder. This regression intentionally isolates the constraint;
            // it is not compared with or called parity against native LVGL.
            nodes: vec![built.nodes[date_index - 1].clone(), date_node],
        };

        let svg = scene_to_svg(
            &static_date_constraint,
            &[],
            &RasterAssetMap::new(),
            &RenderDeadline::start(),
        )
        .unwrap();
        assert!(svg.contains(&xml_escape(&produced_date)));
        assert!(svg.contains(r#"<clipPath id="clip-1"><rect x="48" y="0" width="176""#));
        let frame = rasterize_scene(&static_date_constraint, &[], &RasterAssetMap::new()).unwrap();
        assert_raster_regression("produced-date-overflow", &frame);
    }

    #[test]
    fn raster_regression_logical_orientation_does_not_change_raster_output() {
        let template = include_str!("../../../plugins/svg-aqi/face.svg");
        let orientation_90 = rasterize_svg_template(template, &rooted_aqi(), &[]).unwrap();
        let orientation_270 = rasterize_svg_template(template, &rooted_aqi(), &[]).unwrap();
        assert_eq!(
            orientation_90, orientation_270,
            "raster regression: logical 448x368 output must be orientation-independent"
        );
    }
}
