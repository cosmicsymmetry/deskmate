//! Bounded SVG rasterization into the device's decoded LVGL RGB565 image format.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use resvg::tiny_skia::{Color, Pixmap, Transform};
use roxmltree::{Document, Node, NodeId};
use sha2::{Digest, Sha256};
use thiserror::Error;

use protocol::{SCENE_CANVAS_HEIGHT, SCENE_CANVAS_WIDTH};

const LVGL_IMAGE_HEADER_BYTES: usize = 12;
const LVGL_IMAGE_MAGIC: u32 = 0x19;
const LVGL_COLOR_FORMAT_RGB565: u32 = 0x12;
const INTER_REGULAR: &[u8] = include_bytes!("../../../../tools/fonts/Inter-Regular.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../../../../tools/fonts/Inter-SemiBold.ttf");
const ALLOWED_FONT_FAMILY: &str = "Inter";

/// SVG source is capped before XML parsing so one plugin cannot consume server memory.
pub const MAX_SVG_SOURCE_BYTES: usize = 256 * 1024;
/// XML nesting is capped to keep all tree walks and renderer recursion shallow.
pub const MAX_XML_DEPTH: usize = 64;
/// Element count is capped before renderer allocation to bound document complexity.
pub const MAX_XML_ELEMENT_NODES: usize = 4_096;
/// Referenced `<use>` trees are metered separately because a small DOM can expand greatly.
pub const MAX_EXPANDED_SVG_NODES: usize = 8_192;
/// Root dimensions are bounded even though output is normalized to the fixed device canvas.
pub const MAX_SVG_DIMENSION: u32 = 4_096;
/// Matches the provider's render deadline so validation and raster work share one budget.
pub const MAX_RENDER_WALL_CLOCK: Duration = Duration::from_millis(250);

/// The decoded image bytes that can be uploaded directly as an LVGL image asset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasterizedFrame {
    pub digest: [u8; 32],
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// A named, fail-closed reason an SVG was not rasterized.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RasterizeError {
    #[error("SVG source exceeds the {max_bytes}-byte limit")]
    SourceTooLarge { max_bytes: usize },
    #[error("SVG source is not valid UTF-8")]
    InvalidUtf8,
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
    #[error("the renderer could not allocate the fixed canvas")]
    CanvasAllocation,
    #[error("SVG validation or rendering exceeded the {millis} ms budget")]
    RenderDeadlineExceeded { millis: u128 },
}

/// Rasterizes one SVG source string onto the fixed, true-black device canvas.
pub fn rasterize_svg(svg: &str) -> Result<RasterizedFrame, RasterizeError> {
    rasterize_svg_bytes(svg.as_bytes())
}

fn rasterize_svg_bytes(bytes: &[u8]) -> Result<RasterizedFrame, RasterizeError> {
    let deadline = RenderDeadline::start();
    if bytes.len() > MAX_SVG_SOURCE_BYTES {
        return Err(RasterizeError::SourceTooLarge {
            max_bytes: MAX_SVG_SOURCE_BYTES,
        });
    }
    let svg = std::str::from_utf8(bytes).map_err(|_| RasterizeError::InvalidUtf8)?;
    preflight(svg)?;
    deadline.check()?;

    let options = renderer_options();

    let tree = resvg::usvg::Tree::from_str(svg, &options)
        .map_err(|error| RasterizeError::SvgParse(error.to_string()))?;
    deadline.check()?;

    let width = canvas_width();
    let height = canvas_height();
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

    let bytes = encode_rgb565(width, height, &pixmap);
    deadline.check()?;
    let digest = Sha256::digest(&bytes).into();
    Ok(RasterizedFrame {
        digest,
        bytes,
        width,
        height,
    })
}

fn renderer_options() -> resvg::usvg::Options<'static> {
    // usvg's default string resolver reads relative/absolute filesystem paths even
    // when `resources_dir` is None, while its data resolver expands embedded images.
    // Replace both callbacks with deny-all functions; preflight's named errors remain
    // the user-facing decision, and these callbacks are defense in depth.
    let mut options = resvg::usvg::Options {
        resources_dir: None,
        style_sheet: None,
        image_href_resolver: resvg::usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        font_family: ALLOWED_FONT_FAMILY.to_owned(),
        ..Default::default()
    };
    // This database starts empty. Loading these reviewed bytes directly is deliberate:
    // never call `load_system_fonts`, which would grant ambient host filesystem access.
    // usvg's default FontResolver queries only this supplied database.
    options.fontdb_mut().load_font_data(INTER_REGULAR.to_vec());
    options.fontdb_mut().load_font_data(INTER_SEMIBOLD.to_vec());
    options
}

struct RenderDeadline(Instant);

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

fn canvas_width() -> u32 {
    u32::try_from(SCENE_CANVAS_WIDTH).expect("protocol canvas width is positive")
}

fn canvas_height() -> u32 {
    u32::try_from(SCENE_CANVAS_HEIGHT).expect("protocol canvas height is positive")
}

fn preflight(svg: &str) -> Result<Document<'_>, RasterizeError> {
    let lowercase = svg.to_ascii_lowercase();
    if lowercase.contains("<!doctype") || lowercase.contains("<!entity") {
        return Err(RasterizeError::EntityExpansionForbidden);
    }
    if lowercase.contains("<?xml-stylesheet") || lowercase.contains("@import") {
        return Err(RasterizeError::ExternalStylesheetForbidden);
    }
    // All CSS url() forms are rejected, including local paint servers. This is more
    // restrictive than SVG itself, but it makes authority review syntactic and prevents
    // escaped/redirected URL forms from relying on a renderer default.
    if lowercase.contains("url(") {
        return Err(RasterizeError::CssUrlForbidden);
    }

    let document = Document::parse(svg).map_err(|_| RasterizeError::MalformedXml)?;
    let root = document.root_element();
    if root.tag_name().name() != "svg" {
        return Err(RasterizeError::MalformedXml);
    }
    validate_dimensions(root)?;

    let mut element_count = 0_usize;
    let mut ids = HashMap::new();
    for node in document.descendants().filter(Node::is_element) {
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
        validate_element(node)?;
    }
    validate_use_expansion(&document, &ids)?;
    Ok(document)
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

fn validate_element(node: Node<'_, '_>) -> Result<(), RasterizeError> {
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
            validate_reference(attribute.value())?;
        }
    }

    if name == "text" {
        validate_font_family(node)?;
    }
    Ok(())
}

fn validate_reference(value: &str) -> Result<(), RasterizeError> {
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
        return Err(RasterizeError::DataReferenceForbidden);
    }
    // A relative URL is a filesystem request if a resource directory ever appears.
    Err(RasterizeError::FileReferenceForbidden)
}

fn validate_font_family(node: Node<'_, '_>) -> Result<(), RasterizeError> {
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
        family
            .trim()
            .trim_matches(['\'', '"'])
            .eq_ignore_ascii_case(ALLOWED_FONT_FAMILY)
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

fn validate_use_expansion(
    document: &Document<'_>,
    ids: &HashMap<&str, Node<'_, '_>>,
) -> Result<(), RasterizeError> {
    let mut expanded = 0_usize;
    let mut memo = HashMap::new();
    for use_node in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "use")
    {
        let Some(target) = local_reference_target(use_node, ids) else {
            continue;
        };
        let cost = expanded_subtree_cost(target, ids, &mut memo, &mut HashSet::new())?;
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
) -> Result<usize, RasterizeError> {
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
        cost = cost.saturating_add(expanded_subtree_cost(child, ids, memo, visiting)?);
        if child.tag_name().name() == "use"
            && let Some(target) = local_reference_target(child, ids)
        {
            cost = cost.saturating_add(expanded_subtree_cost(target, ids, memo, visiting)?);
        }
        if cost > MAX_EXPANDED_SVG_NODES {
            break;
        }
    }
    visiting.remove(&node.id());
    memo.insert(node.id(), cost);
    Ok(cost)
}

fn encode_rgb565(width: u32, height: u32, pixmap: &Pixmap) -> Vec<u8> {
    let pixel_count = usize::try_from(width * height).expect("fixed canvas fits usize");
    let mut bytes = Vec::with_capacity(LVGL_IMAGE_HEADER_BYTES + pixel_count * 2);
    let stride = width * 2;
    let word0 = LVGL_IMAGE_MAGIC | (LVGL_COLOR_FORMAT_RGB565 << 8);
    let word1 = (width & 0xffff) | ((height & 0xffff) << 16);
    let word2 = stride & 0xffff;
    bytes.extend_from_slice(&word0.to_le_bytes());
    bytes.extend_from_slice(&word1.to_le_bytes());
    bytes.extend_from_slice(&word2.to_le_bytes());
    for pixel in pixmap.pixels() {
        bytes.extend_from_slice(
            &pack_rgb565(pixel.red(), pixel.green(), pixel.blue()).to_le_bytes(),
        );
    }
    bytes
}

fn pack_rgb565(red: u8, green: u8, blue: u8) -> u16 {
    // Round each 8-bit channel to the nearest representable endpoint-inclusive value:
    // (channel * max + 127) / 255. Exact half steps round upward.
    let red = (u16::from(red) * 31 + 127) / 255;
    let green = (u16::from(green) * 63 + 127) / 255;
    let blue = (u16::from(blue) * 31 + 127) / 255;
    (red << 11) | (green << 5) | blue
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Instant;

    use protocol::{SCENE_CANVAS_HEIGHT, SCENE_CANVAS_WIDTH};
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
        assert_eq!(frame.width, width);
        assert_eq!(frame.height, height);
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
    fn usvg_options_have_no_ambient_image_or_font_authority() {
        let options = renderer_options();
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
}
