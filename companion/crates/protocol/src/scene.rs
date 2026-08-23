//! The host half of the scene wire format.
//!
//! This mirrors `firmware/main/core/scene_model.h` and the wire shape
//! documented at the top of `firmware/main/core/scene_decode.c`, key for key
//! and bound for bound. Where the two can disagree, they are caught by the
//! shared fixture corpus in `protocol/fixtures/v1/`: the Rust encoder writes
//! `push_scene.bin` and `firmware/host_tests/test_protocol.c` decodes it,
//! re-encodes it in C and `memcmp`s the result against the file.
//!
//! # Canonical emission rule
//!
//! Required keys are always emitted; an **optional** key is emitted only when
//! its value differs from the default the device's decoder would supply. Both
//! implementations follow it, and the byte-for-byte fixture round-trip is why
//! they must. It is not tidiness: it is what keeps a 24-node scene inside the
//! 2034-byte payload, since a baked-font text node that spelled out an unused
//! 32-byte digest would cost 34 wasted bytes and 24 of them would nearly fill
//! the envelope.
//!
//! # Where this model is narrower than the device's
//!
//! `scene_value_t` and `scene_font_ref_t` are C structs carrying every field
//! of every variant, with a `kind` selecting which ones matter. [`SceneValue`]
//! and [`SceneFont`] are Rust enums, so they simply cannot carry the half the
//! kind does not select. That makes this side unable to *emit* a value whose
//! unused half is populated, which is the right direction: the device tolerates
//! one, and nothing should send one.

use crate::cbor::{Decoder, Encoder};
use crate::message::{ASSET_DIGEST_LEN, MessageError};

pub const SCENE_CANVAS_WIDTH: i32 = 448;
pub const SCENE_CANVAS_HEIGHT: i32 = 368;
/// Must match firmware's `SCENE_MAX_NODES`.
pub const MAX_SCENE_NODES: usize = 24;
/// Must match firmware's `SCENE_MAX_TEXT_BYTES`.
pub const MAX_SCENE_TEXT_LEN: usize = 128;
/// Must match firmware's `SCENE_MAX_BINDING`.
pub const MAX_SCENE_BINDING_LEN: usize = 48;
/// Must match firmware's `SCENE_MAX_LINE_POINTS`.
pub const MAX_SCENE_LINE_POINTS: usize = 8;
/// Must match firmware's `SCENE_MAX_GLYPH_NAME`.
pub const MAX_SCENE_GLYPH_NAME_LEN: usize = 32;
/// Must match firmware's `SCENE_SCALE_MAX_TOTAL_TICKS`.
pub const MAX_SCENE_SCALE_TICKS: u32 = 361;

const MIN_ASSET_FONT_PIXEL_SIZE: i32 = 8;
const MAX_ASSET_FONT_PIXEL_SIZE: i32 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SceneFontTier {
    Caption = 1,
    Body = 2,
    Display = 3,
    Hero = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneFont {
    Baked(SceneFontTier),
    Asset {
        digest: [u8; ASSET_DIGEST_LEN],
        pixel_size: i32,
    },
}

impl Default for SceneFont {
    fn default() -> Self {
        Self::Baked(SceneFontTier::Body)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum SceneAlign {
    /// The wire default, and deliberately **1** rather than 0: a text node
    /// that omits the align key must not decode to a value the device's
    /// validator rejects.
    #[default]
    Left = 1,
    Center = 2,
    Right = 3,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneValue {
    Literal(String),
    Binding(String),
}

impl Default for SceneValue {
    fn default() -> Self {
        Self::Literal(String::new())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub radius: i32,
    pub fill: u32,
    pub opacity: u8,
}

impl Default for SceneRect {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            radius: 0,
            fill: 0,
            // An omitted opacity means opaque; 0 would draw nothing.
            opacity: u8::MAX,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SceneArc {
    pub cx: i32,
    pub cy: i32,
    pub r: i32,
    /// Absolute canvas angles in LVGL's convention: 0 is 3 o'clock, angles
    /// increase clockwise, and a full turn is a nonzero exact multiple of 360
    /// (`start_deg = 270, end_deg = 630`), never `start_deg == end_deg`.
    pub start_deg: i32,
    pub end_deg: i32,
    pub width: i32,
    pub color: u32,
    pub rounded: bool,
    /// Scales the declared sweep rather than replacing it. Empty means none.
    pub end_binding: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SceneLine {
    /// Both arrays must be the same length, in `2..=MAX_SCENE_LINE_POINTS`.
    pub xs: Vec<i32>,
    pub ys: Vec<i32>,
    pub width: i32,
    pub color: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SceneText {
    pub x: i32,
    /// The type's baseline, not the box top.
    pub baseline_y: i32,
    pub w: i32,
    pub align: SceneAlign,
    pub font: SceneFont,
    pub color: u32,
    pub value: SceneValue,
    pub ellipsize: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SceneImage {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub digest: [u8; ASSET_DIGEST_LEN],
    pub recolor: bool,
    pub color: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SceneGlyph {
    pub x: i32,
    pub baseline_y: i32,
    pub size: i32,
    pub digest: [u8; ASSET_DIGEST_LEN],
    pub name: String,
    pub color: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SceneScale {
    pub x: i32,
    pub y: i32,
    /// One side length: every scale drawn so far is square.
    pub box_size: i32,
    pub total_tick_count: u32,
    pub major_tick_every: u32,
    pub major_tick_color: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneNode {
    Rect(SceneRect),
    Arc(SceneArc),
    Line(SceneLine),
    Text(SceneText),
    Image(SceneImage),
    Glyph(SceneGlyph),
    Scale(SceneScale),
}

impl SceneNode {
    const fn wire_kind(&self) -> u64 {
        match self {
            Self::Rect(_) => 1,
            Self::Arc(_) => 2,
            Self::Line(_) => 3,
            Self::Text(_) => 4,
            Self::Image(_) => 5,
            Self::Glyph(_) => 6,
            Self::Scale(_) => 7,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Scene {
    pub revision: u32,
    pub background: u32,
    pub nodes: Vec<SceneNode>,
}

// ---------------------------------------------------------------------------
// Bindings.
//
// A mirror of firmware's scene_binding_parse(), which the device's decoder
// applies while decoding so a scene naming a binding it cannot evaluate is
// refused whole rather than drawn empty. Mirrored here so this side cannot
// emit one either -- otherwise the first sign of the problem would be the
// device refusing a scene the host believed was valid.
// ---------------------------------------------------------------------------

fn time_format_is_valid(argument: &str) -> bool {
    !argument.is_empty()
        && argument.len() <= MAX_SCENE_BINDING_LEN
        // A whitelist, never a printf format string: these characters reach a
        // formatter on the device.
        && argument
            .chars()
            .all(|c| matches!(c, 'H' | 'h' | 'M' | 'm' | 'S' | 's' | ':'))
}

#[must_use]
pub fn binding_is_valid(text: &str) -> bool {
    if text.len() > MAX_SCENE_BINDING_LEN {
        return false;
    }
    // `timer.pct` is an exact-match token checked before the `timer.remaining:`
    // prefix they share a stem with, so "timer.pctXYZ" cannot match it.
    if text == "timer.pct" {
        return true;
    }
    if let Some(argument) = text.strip_prefix("timer.remaining:") {
        return time_format_is_valid(argument);
    }
    if let Some(argument) = text.strip_prefix("time:") {
        return time_format_is_valid(argument);
    }
    if let Some(argument) = text.strip_prefix("field.") {
        return !argument.is_empty() && argument.len() <= MAX_SCENE_BINDING_LEN;
    }
    false
}

// ---------------------------------------------------------------------------
// Validation -- a mirror of scene_model_validate(), plus the length caps the
// device's decoder owns rather than its model.
// ---------------------------------------------------------------------------

fn rect_within_canvas(x: i32, y: i32, w: i32, h: i32) -> bool {
    // Written as `x > LIMIT - extent` rather than `x + extent > LIMIT` for the
    // same reason the C does: a huge coordinate must not wrap before the
    // comparison.
    x >= 0
        && y >= 0
        && w >= 0
        && h >= 0
        && w <= SCENE_CANVAS_WIDTH
        && h <= SCENE_CANVAS_HEIGHT
        && x <= SCENE_CANVAS_WIDTH - w
        && y <= SCENE_CANVAS_HEIGHT - h
}

fn baseline_within_canvas(baseline_y: i32) -> bool {
    (0..=SCENE_CANVAS_HEIGHT).contains(&baseline_y)
}

fn validate_font(font: &SceneFont) -> Result<(), MessageError> {
    match font {
        SceneFont::Baked(_) => Ok(()),
        SceneFont::Asset { pixel_size, .. } => {
            if (MIN_ASSET_FONT_PIXEL_SIZE..=MAX_ASSET_FONT_PIXEL_SIZE).contains(pixel_size) {
                Ok(())
            } else {
                Err(MessageError::InvalidValue("scene font pixel size"))
            }
        }
    }
}

fn validate_value(value: &SceneValue) -> Result<(), MessageError> {
    match value {
        SceneValue::Literal(literal) => {
            if literal.len() > MAX_SCENE_TEXT_LEN {
                return Err(MessageError::InvalidValue("scene text literal"));
            }
            Ok(())
        }
        SceneValue::Binding(binding) => {
            if binding_is_valid(binding) {
                Ok(())
            } else {
                Err(MessageError::InvalidValue("scene binding"))
            }
        }
    }
}

fn validate_node(node: &SceneNode) -> Result<(), MessageError> {
    match node {
        SceneNode::Rect(rect) => {
            if !rect_within_canvas(rect.x, rect.y, rect.w, rect.h) {
                return Err(MessageError::InvalidValue("scene rect geometry"));
            }
        }
        SceneNode::Arc(arc) => {
            if arc.r < 0
                || arc.cx < 0
                || arc.cy < 0
                || arc.width < 0
                || arc.r > SCENE_CANVAS_WIDTH
                || arc.r > SCENE_CANVAS_HEIGHT
                || arc.cx < arc.r
                || arc.cy < arc.r
                || arc.cx > SCENE_CANVAS_WIDTH - arc.r
                || arc.cy > SCENE_CANVAS_HEIGHT - arc.r
            {
                return Err(MessageError::InvalidValue("scene arc geometry"));
            }
            if !arc.end_binding.is_empty() && !binding_is_valid(&arc.end_binding) {
                return Err(MessageError::InvalidValue("scene binding"));
            }
        }
        SceneNode::Line(line) => {
            if line.xs.len() != line.ys.len() {
                return Err(MessageError::InvalidValue("scene line point arrays"));
            }
            if line.xs.len() < 2 || line.xs.len() > MAX_SCENE_LINE_POINTS {
                return Err(MessageError::InvalidValue("scene line point count"));
            }
            if line.width < 0
                || line
                    .xs
                    .iter()
                    .any(|x| !(0..=SCENE_CANVAS_WIDTH).contains(x))
                || line
                    .ys
                    .iter()
                    .any(|y| !(0..=SCENE_CANVAS_HEIGHT).contains(y))
            {
                return Err(MessageError::InvalidValue("scene line geometry"));
            }
        }
        SceneNode::Text(text) => {
            if text.x < 0
                || text.w < 0
                || text.w > SCENE_CANVAS_WIDTH
                || text.x > SCENE_CANVAS_WIDTH - text.w
                || !baseline_within_canvas(text.baseline_y)
            {
                return Err(MessageError::InvalidValue("scene text geometry"));
            }
            validate_font(&text.font)?;
            validate_value(&text.value)?;
        }
        SceneNode::Image(image) => {
            if !rect_within_canvas(image.x, image.y, image.w, image.h) {
                return Err(MessageError::InvalidValue("scene image geometry"));
            }
        }
        SceneNode::Glyph(glyph) => {
            if glyph.x < 0
                || glyph.size < 0
                || glyph.size > SCENE_CANVAS_WIDTH
                || glyph.size > SCENE_CANVAS_HEIGHT
                || glyph.x > SCENE_CANVAS_WIDTH - glyph.size
                || !baseline_within_canvas(glyph.baseline_y)
            {
                return Err(MessageError::InvalidValue("scene glyph geometry"));
            }
            if glyph.name.len() > MAX_SCENE_GLYPH_NAME_LEN {
                return Err(MessageError::InvalidValue("scene glyph name"));
            }
        }
        SceneNode::Scale(scale) => {
            if !rect_within_canvas(scale.x, scale.y, scale.box_size, scale.box_size) {
                return Err(MessageError::InvalidValue("scene scale geometry"));
            }
            // Bounded because LVGL redraws every tick on each invalidation
            // with no cap of its own.
            if scale.total_tick_count < 2 || scale.total_tick_count > MAX_SCENE_SCALE_TICKS {
                return Err(MessageError::InvalidValue("scene scale tick count"));
            }
            if scale.major_tick_every < 1 || scale.major_tick_every > scale.total_tick_count {
                return Err(MessageError::InvalidValue("scene scale major tick"));
            }
        }
    }
    Ok(())
}

/// Checks a scene against every bound the device enforces.
///
/// # Errors
///
/// Returns [`MessageError::InvalidValue`] naming the first bound violated.
pub fn validate_scene(scene: &Scene) -> Result<(), MessageError> {
    if scene.nodes.len() > MAX_SCENE_NODES {
        return Err(MessageError::InvalidValue("scene node count"));
    }
    for node in &scene.nodes {
        validate_node(node)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Encoding.
// ---------------------------------------------------------------------------

fn encode_font(encoder: &mut Encoder, font: &SceneFont) {
    match font {
        SceneFont::Baked(tier) => {
            encoder.map(2);
            encoder.unsigned(0);
            encoder.unsigned(1); // SCENE_FONT_BAKED
            encoder.unsigned(1);
            encoder.unsigned(*tier as u64);
        }
        SceneFont::Asset { digest, pixel_size } => {
            encoder.map(3);
            encoder.unsigned(0);
            encoder.unsigned(2); // SCENE_FONT_ASSET
            encoder.unsigned(2);
            encoder.bytes(digest);
            encoder.unsigned(3);
            encoder.signed(i64::from(*pixel_size));
        }
    }
}

fn encode_value(encoder: &mut Encoder, value: &SceneValue) {
    match value {
        SceneValue::Literal(literal) => {
            encoder.map(1 + usize::from(!literal.is_empty()));
            encoder.unsigned(0);
            encoder.unsigned(1); // SCENE_VALUE_LITERAL
            if !literal.is_empty() {
                encoder.unsigned(1);
                encoder.text(literal);
            }
        }
        SceneValue::Binding(binding) => {
            encoder.map(1 + usize::from(!binding.is_empty()));
            encoder.unsigned(0);
            encoder.unsigned(2); // SCENE_VALUE_BINDING
            if !binding.is_empty() {
                encoder.unsigned(2);
                encoder.text(binding);
            }
        }
    }
}

fn encode_points(encoder: &mut Encoder, key: u64, points: &[i32]) {
    encoder.unsigned(key);
    encoder.array(points.len());
    for point in points {
        encoder.signed(i64::from(*point));
    }
}

/// One arm per node kind, each a flat run of key/value pairs. Splitting it
/// per kind would scatter the canonical emission rule across seven functions
/// that must stay in step; keeping it in one place is what makes the rule
/// readable as a whole.
#[allow(clippy::too_many_lines)]
fn encode_node_payload(encoder: &mut Encoder, node: &SceneNode) {
    match node {
        SceneNode::Rect(rect) => {
            encoder.map(
                4 + usize::from(rect.radius != 0)
                    + usize::from(rect.fill != 0)
                    + usize::from(rect.opacity != u8::MAX),
            );
            for (key, value) in [(0, rect.x), (1, rect.y), (2, rect.w), (3, rect.h)] {
                encoder.unsigned(key);
                encoder.signed(i64::from(value));
            }
            if rect.radius != 0 {
                encoder.unsigned(4);
                encoder.signed(i64::from(rect.radius));
            }
            if rect.fill != 0 {
                encoder.unsigned(5);
                encoder.unsigned(u64::from(rect.fill));
            }
            if rect.opacity != u8::MAX {
                encoder.unsigned(6);
                encoder.unsigned(u64::from(rect.opacity));
            }
        }
        SceneNode::Arc(arc) => {
            encoder.map(
                6 + usize::from(arc.color != 0)
                    + usize::from(arc.rounded)
                    + usize::from(!arc.end_binding.is_empty()),
            );
            for (key, value) in [
                (0, arc.cx),
                (1, arc.cy),
                (2, arc.r),
                (3, arc.start_deg),
                (4, arc.end_deg),
                (5, arc.width),
            ] {
                encoder.unsigned(key);
                encoder.signed(i64::from(value));
            }
            if arc.color != 0 {
                encoder.unsigned(6);
                encoder.unsigned(u64::from(arc.color));
            }
            if arc.rounded {
                encoder.unsigned(7);
                encoder.boolean(true);
            }
            if !arc.end_binding.is_empty() {
                encoder.unsigned(8);
                encoder.text(&arc.end_binding);
            }
        }
        SceneNode::Line(line) => {
            encoder.map(3 + usize::from(line.color != 0));
            encode_points(encoder, 0, &line.xs);
            encode_points(encoder, 1, &line.ys);
            encoder.unsigned(2);
            encoder.signed(i64::from(line.width));
            if line.color != 0 {
                encoder.unsigned(3);
                encoder.unsigned(u64::from(line.color));
            }
        }
        SceneNode::Text(text) => {
            encoder.map(
                5 + usize::from(text.align != SceneAlign::Left)
                    + usize::from(text.color != 0)
                    + usize::from(text.ellipsize),
            );
            for (key, value) in [(0, text.x), (1, text.baseline_y), (2, text.w)] {
                encoder.unsigned(key);
                encoder.signed(i64::from(value));
            }
            if text.align != SceneAlign::Left {
                encoder.unsigned(3);
                encoder.unsigned(text.align as u64);
            }
            encoder.unsigned(4);
            encode_font(encoder, &text.font);
            if text.color != 0 {
                encoder.unsigned(5);
                encoder.unsigned(u64::from(text.color));
            }
            encoder.unsigned(6);
            encode_value(encoder, &text.value);
            if text.ellipsize {
                encoder.unsigned(7);
                encoder.boolean(true);
            }
        }
        SceneNode::Image(image) => {
            encoder.map(5 + usize::from(image.recolor) + usize::from(image.color != 0));
            for (key, value) in [(0, image.x), (1, image.y), (2, image.w), (3, image.h)] {
                encoder.unsigned(key);
                encoder.signed(i64::from(value));
            }
            encoder.unsigned(4);
            encoder.bytes(&image.digest);
            if image.recolor {
                encoder.unsigned(5);
                encoder.boolean(true);
            }
            if image.color != 0 {
                encoder.unsigned(6);
                encoder.unsigned(u64::from(image.color));
            }
        }
        SceneNode::Glyph(glyph) => {
            encoder.map(5 + usize::from(glyph.color != 0));
            for (key, value) in [(0, glyph.x), (1, glyph.baseline_y), (2, glyph.size)] {
                encoder.unsigned(key);
                encoder.signed(i64::from(value));
            }
            encoder.unsigned(3);
            encoder.bytes(&glyph.digest);
            encoder.unsigned(4);
            encoder.text(&glyph.name);
            if glyph.color != 0 {
                encoder.unsigned(5);
                encoder.unsigned(u64::from(glyph.color));
            }
        }
        SceneNode::Scale(scale) => {
            encoder.map(5 + usize::from(scale.major_tick_color != 0));
            for (key, value) in [(0, scale.x), (1, scale.y), (2, scale.box_size)] {
                encoder.unsigned(key);
                encoder.signed(i64::from(value));
            }
            encoder.unsigned(3);
            encoder.unsigned(u64::from(scale.total_tick_count));
            encoder.unsigned(4);
            encoder.unsigned(u64::from(scale.major_tick_every));
            if scale.major_tick_color != 0 {
                encoder.unsigned(5);
                encoder.unsigned(u64::from(scale.major_tick_color));
            }
        }
    }
}

pub(crate) fn encode_scene(encoder: &mut Encoder, scene: &Scene) {
    encoder.map(3);
    encoder.unsigned(0);
    encoder.unsigned(u64::from(scene.revision));
    encoder.unsigned(1);
    encoder.unsigned(u64::from(scene.background));
    encoder.unsigned(2);
    encoder.array(scene.nodes.len());
    for node in &scene.nodes {
        encoder.map(2);
        encoder.unsigned(0);
        encoder.unsigned(node.wire_kind());
        encoder.unsigned(1);
        encode_node_payload(encoder, node);
    }
}

// ---------------------------------------------------------------------------
// Decoding.
// ---------------------------------------------------------------------------

fn next_key(decoder: &mut Decoder<'_>, previous: &mut Option<u64>) -> Result<u64, MessageError> {
    let key = decoder.unsigned()?;
    if previous.is_some_and(|last| key <= last) {
        return Err(MessageError::DuplicateOrUnsortedKey);
    }
    *previous = Some(key);
    Ok(key)
}

fn read_i32(decoder: &mut Decoder<'_>, name: &'static str) -> Result<i32, MessageError> {
    i32::try_from(decoder.signed()?).map_err(|_| MessageError::InvalidValue(name))
}

fn read_u32(decoder: &mut Decoder<'_>, name: &'static str) -> Result<u32, MessageError> {
    u32::try_from(decoder.unsigned()?).map_err(|_| MessageError::InvalidValue(name))
}

fn read_digest(decoder: &mut Decoder<'_>) -> Result<[u8; ASSET_DIGEST_LEN], MessageError> {
    <[u8; ASSET_DIGEST_LEN]>::try_from(decoder.bytes()?)
        .map_err(|_| MessageError::InvalidValue("scene digest"))
}

fn read_bounded_text(
    decoder: &mut Decoder<'_>,
    max: usize,
    name: &'static str,
) -> Result<String, MessageError> {
    let text = decoder.text()?;
    if text.len() > max {
        return Err(MessageError::InvalidValue(name));
    }
    Ok(text.to_owned())
}

fn decode_font(decoder: &mut Decoder<'_>) -> Result<SceneFont, MessageError> {
    let len = decoder.map_len()?;
    let mut previous = None;
    let mut kind = None;
    let mut tier = 0u32;
    let mut digest = None;
    let mut pixel_size = 0i32;
    for _ in 0..len {
        match next_key(decoder, &mut previous)? {
            0 => kind = Some(read_u32(decoder, "scene font kind")?),
            1 => tier = read_u32(decoder, "scene font tier")?,
            2 => digest = Some(read_digest(decoder)?),
            3 => pixel_size = read_i32(decoder, "scene font pixel size")?,
            _ => decoder.skip()?,
        }
    }
    match kind.ok_or(MessageError::MissingField(0))? {
        1 => Ok(SceneFont::Baked(match tier {
            1 => SceneFontTier::Caption,
            2 => SceneFontTier::Body,
            3 => SceneFontTier::Display,
            4 => SceneFontTier::Hero,
            _ => return Err(MessageError::InvalidValue("scene font tier")),
        })),
        2 => Ok(SceneFont::Asset {
            // Required for an asset font specifically: an all-zero digest
            // would validate and then miss at draw time.
            digest: digest.ok_or(MessageError::MissingField(2))?,
            pixel_size,
        }),
        _ => Err(MessageError::InvalidValue("scene font kind")),
    }
}

fn decode_value(decoder: &mut Decoder<'_>) -> Result<SceneValue, MessageError> {
    let len = decoder.map_len()?;
    let mut previous = None;
    let mut kind = None;
    let mut literal = String::new();
    let mut binding = String::new();
    for _ in 0..len {
        match next_key(decoder, &mut previous)? {
            0 => kind = Some(read_u32(decoder, "scene value kind")?),
            1 => {
                literal = read_bounded_text(decoder, MAX_SCENE_TEXT_LEN, "scene text literal")?;
            }
            2 => {
                binding = read_bounded_text(decoder, MAX_SCENE_BINDING_LEN, "scene binding")?;
            }
            _ => decoder.skip()?,
        }
    }
    match kind.ok_or(MessageError::MissingField(0))? {
        1 => Ok(SceneValue::Literal(literal)),
        2 => Ok(SceneValue::Binding(binding)),
        _ => Err(MessageError::InvalidValue("scene value kind")),
    }
}

fn decode_points(decoder: &mut Decoder<'_>) -> Result<Vec<i32>, MessageError> {
    let count = decoder.array_len()?;
    // Bounded before a single element is read.
    if count > MAX_SCENE_LINE_POINTS {
        return Err(MessageError::InvalidValue("scene line point count"));
    }
    let mut points = Vec::with_capacity(count);
    for _ in 0..count {
        points.push(read_i32(decoder, "scene line point")?);
    }
    Ok(points)
}

/// Arms are keyed by `(node kind, wire key)` so this file's decoder reads in
/// the same order as the wire-shape table at the top of
/// `firmware/main/core/scene_decode.c`, one line per key per kind.
///
/// `match_same_arms` is allowed deliberately: several kinds share a slot --
/// ARC's `rounded`, TEXT's `ellipsize` and IMAGE's `recolor` are all the
/// node's single boolean, and IMAGE and GLYPH both carry one digest -- and
/// merging those patterns would move a key away from the kind it belongs to,
/// which is exactly the correspondence this layout exists to keep.
#[allow(clippy::too_many_lines, clippy::match_same_arms)]
fn decode_node_payload(decoder: &mut Decoder<'_>, kind: u32) -> Result<SceneNode, MessageError> {
    let len = decoder.map_len()?;
    let mut previous = None;
    let mut present = 0u32;
    // One flat set of slots for every node kind, so this mirrors the C
    // switch-per-key shape rather than building a parallel struct per kind.
    let mut ints = [0i32; 6];
    let mut colors = [0u32; 2];
    let mut flags = [false; 2];
    let mut opacity = u8::MAX;
    let mut align = SceneAlign::Left;
    let mut font = None;
    let mut value = None;
    let mut digest = None;
    let mut text = String::new();
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut ticks = [0u32; 2];

    for _ in 0..len {
        let key = next_key(decoder, &mut previous)?;
        if key < 32 {
            present |= 1 << key;
        }
        // Only the arms whose key pattern is 0..=5 index `ints` with this,
        // so it cannot saturate; the fallback keeps the conversion total
        // rather than panicking on a 32-bit target.
        let slot = usize::try_from(key).unwrap_or(usize::MAX);
        match (kind, key) {
            // RECT {0: x, 1: y, 2: w, 3: h, 4: radius, 5: fill, 6: opacity}
            (1, 0..=4) => ints[slot] = read_i32(decoder, "scene rect field")?,
            (1, 5) => colors[0] = read_u32(decoder, "scene rect fill")?,
            (1, 6) => {
                opacity = u8::try_from(decoder.unsigned()?)
                    .map_err(|_| MessageError::InvalidValue("scene rect opacity"))?;
            }
            // ARC {0: cx, 1: cy, 2: r, 3: start, 4: end, 5: width, 6: color,
            //      7: rounded, 8: end_binding}
            (2, 0..=5) => ints[slot] = read_i32(decoder, "scene arc field")?,
            (2, 6) => colors[0] = read_u32(decoder, "scene arc color")?,
            (2, 7) => flags[0] = decoder.boolean()?, // rounded
            (2, 8) => {
                text = read_bounded_text(decoder, MAX_SCENE_BINDING_LEN, "scene binding")?;
            }
            // LINE {0: [x...], 1: [y...], 2: width, 3: color}
            (3, 0) => xs = decode_points(decoder)?,
            (3, 1) => ys = decode_points(decoder)?,
            (3, 2) => ints[0] = read_i32(decoder, "scene line width")?,
            (3, 3) => colors[0] = read_u32(decoder, "scene line color")?,
            // TEXT {0: x, 1: baseline_y, 2: w, 3: align, 4: font, 5: color,
            //       6: value, 7: ellipsize}
            (4, 0..=2) => ints[slot] = read_i32(decoder, "scene text field")?,
            (4, 3) => {
                align = match read_u32(decoder, "scene text align")? {
                    1 => SceneAlign::Left,
                    2 => SceneAlign::Center,
                    3 => SceneAlign::Right,
                    _ => return Err(MessageError::InvalidValue("scene text align")),
                };
            }
            (4, 4) => font = Some(decode_font(decoder)?),
            (4, 5) => colors[0] = read_u32(decoder, "scene text color")?,
            (4, 6) => value = Some(decode_value(decoder)?),
            (4, 7) => flags[0] = decoder.boolean()?, // ellipsize
            // IMAGE {0: x, 1: y, 2: w, 3: h, 4: digest, 5: recolor, 6: color}
            (5, 0..=3) => ints[slot] = read_i32(decoder, "scene image field")?,
            (5, 4) => digest = Some(read_digest(decoder)?),
            (5, 5) => flags[0] = decoder.boolean()?, // recolor
            (5, 6) => colors[0] = read_u32(decoder, "scene image color")?,
            // GLYPH {0: x, 1: baseline_y, 2: size, 3: digest, 4: name, 5: color}
            (6, 0..=2) => ints[slot] = read_i32(decoder, "scene glyph field")?,
            (6, 3) => digest = Some(read_digest(decoder)?),
            (6, 4) => {
                text = read_bounded_text(decoder, MAX_SCENE_GLYPH_NAME_LEN, "scene glyph name")?;
            }
            (6, 5) => colors[0] = read_u32(decoder, "scene glyph color")?,
            // SCALE {0: x, 1: y, 2: box, 3: total_ticks, 4: every, 5: color}
            (7, 0..=2) => ints[slot] = read_i32(decoder, "scene scale field")?,
            (7, 3) => ticks[0] = read_u32(decoder, "scene scale tick count")?,
            (7, 4) => ticks[1] = read_u32(decoder, "scene scale major tick")?,
            (7, 5) => colors[0] = read_u32(decoder, "scene scale color")?,
            _ => decoder.skip()?,
        }
    }

    let require = |mask: u32| -> Result<(), MessageError> {
        if present & mask == mask {
            Ok(())
        } else {
            Err(MessageError::MissingField(u64::from(
                (!present & mask).trailing_zeros(),
            )))
        }
    };

    match kind {
        1 => {
            require(0x0f)?;
            Ok(SceneNode::Rect(SceneRect {
                x: ints[0],
                y: ints[1],
                w: ints[2],
                h: ints[3],
                radius: ints[4],
                fill: colors[0],
                opacity,
            }))
        }
        2 => {
            require(0x3f)?;
            Ok(SceneNode::Arc(SceneArc {
                cx: ints[0],
                cy: ints[1],
                r: ints[2],
                start_deg: ints[3],
                end_deg: ints[4],
                width: ints[5],
                color: colors[0],
                rounded: flags[0],
                end_binding: text,
            }))
        }
        3 => {
            require(0x07)?;
            // Two arrays of different lengths describe no polyline at all.
            if xs.len() != ys.len() {
                return Err(MessageError::InvalidValue("scene line point arrays"));
            }
            Ok(SceneNode::Line(SceneLine {
                xs,
                ys,
                width: ints[0],
                color: colors[0],
            }))
        }
        4 => {
            require(0x57)?;
            Ok(SceneNode::Text(SceneText {
                x: ints[0],
                baseline_y: ints[1],
                w: ints[2],
                align,
                font: font.ok_or(MessageError::MissingField(4))?,
                color: colors[0],
                value: value.ok_or(MessageError::MissingField(6))?,
                ellipsize: flags[0],
            }))
        }
        5 => {
            require(0x1f)?;
            Ok(SceneNode::Image(SceneImage {
                x: ints[0],
                y: ints[1],
                w: ints[2],
                h: ints[3],
                digest: digest.ok_or(MessageError::MissingField(4))?,
                recolor: flags[0],
                color: colors[0],
            }))
        }
        6 => {
            require(0x1f)?;
            Ok(SceneNode::Glyph(SceneGlyph {
                x: ints[0],
                baseline_y: ints[1],
                size: ints[2],
                digest: digest.ok_or(MessageError::MissingField(3))?,
                name: text,
                color: colors[0],
            }))
        }
        7 => {
            require(0x1f)?;
            Ok(SceneNode::Scale(SceneScale {
                x: ints[0],
                y: ints[1],
                box_size: ints[2],
                total_tick_count: ticks[0],
                major_tick_every: ticks[1],
                major_tick_color: colors[0],
            }))
        }
        _ => Err(MessageError::InvalidValue("scene node kind")),
    }
}

fn decode_node(decoder: &mut Decoder<'_>) -> Result<SceneNode, MessageError> {
    let len = decoder.map_len()?;
    let mut previous = None;
    let mut kind = None;
    let mut node = None;
    for _ in 0..len {
        match next_key(decoder, &mut previous)? {
            0 => kind = Some(read_u32(decoder, "scene node kind")?),
            // Keys are strictly increasing, so key 0 has already set the kind
            // by the time the payload arrives.
            1 => {
                let kind = kind.ok_or(MessageError::MissingField(0))?;
                node = Some(decode_node_payload(decoder, kind)?);
            }
            _ => decoder.skip()?,
        }
    }
    node.ok_or(MessageError::MissingField(1))
}

pub(crate) fn decode_scene(decoder: &mut Decoder<'_>) -> Result<Scene, MessageError> {
    let len = decoder.map_len()?;
    let mut previous = None;
    let mut revision = None;
    let mut background = None;
    let mut nodes = None;
    for _ in 0..len {
        match next_key(decoder, &mut previous)? {
            0 => revision = Some(read_u32(decoder, "scene revision")?),
            1 => background = Some(read_u32(decoder, "scene background")?),
            2 => {
                let count = decoder.array_len()?;
                // Bounded before a single node is read, and rejected rather
                // than truncated to the first 24.
                if count > MAX_SCENE_NODES {
                    return Err(MessageError::InvalidValue("scene node count"));
                }
                let mut list = Vec::with_capacity(count);
                for _ in 0..count {
                    list.push(decode_node(decoder)?);
                }
                nodes = Some(list);
            }
            _ => decoder.skip()?,
        }
    }
    let scene = Scene {
        revision: revision.ok_or(MessageError::MissingField(0))?,
        background: background.ok_or(MessageError::MissingField(1))?,
        nodes: nodes.ok_or(MessageError::MissingField(2))?,
    };
    validate_scene(&scene)?;
    Ok(scene)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_grammar_matches_the_device() {
        assert!(binding_is_valid("timer.pct"));
        assert!(!binding_is_valid("timer.pctXYZ"));
        assert!(binding_is_valid("time:HH:mm"));
        assert!(binding_is_valid("timer.remaining:mm:ss"));
        assert!(binding_is_valid("field.temp"));
        assert!(!binding_is_valid("time:"));
        assert!(!binding_is_valid("time:%s"));
        assert!(!binding_is_valid("field."));
        assert!(!binding_is_valid("whatever"));
        assert!(!binding_is_valid(
            &"field."
                .chars()
                .chain("x".repeat(64).chars())
                .collect::<String>()
        ));
    }

    /// Writes `{0: revision, 1: background, 2: <array header>}` and then
    /// whatever `body` appends, so a test can hand the decoder an array
    /// header that LIES about how much follows it.
    fn scene_with_node_array(claimed: usize, body: impl FnOnce(&mut Encoder)) -> Vec<u8> {
        let mut encoder = Encoder::new();
        encoder.map(3);
        encoder.unsigned(0);
        encoder.unsigned(1);
        encoder.unsigned(1);
        encoder.unsigned(0);
        encoder.unsigned(2);
        encoder.array(claimed);
        body(&mut encoder);
        encoder.into_bytes()
    }

    fn minimal_rect_node(encoder: &mut Encoder) {
        encoder.map(2);
        encoder.unsigned(0);
        encoder.unsigned(1); // RECT
        encoder.unsigned(1);
        encoder.map(4);
        for (key, value) in [(0, 4), (1, 5), (2, 10), (3, 11)] {
            encoder.unsigned(key);
            encoder.unsigned(value);
        }
    }

    /// The validate()-masking trap, on the side with no sanitizer to catch
    /// what the assertion misses.
    ///
    /// `decode_scene`'s pre-allocation bound and `validate_scene` return the
    /// identical error for a count over the cap, so a payload of 25 real
    /// nodes cannot tell them apart: delete the pre-check and the test stays
    /// green. What is lost is not correctness of the verdict but WHEN it is
    /// reached -- `Decoder::array_len` returns the raw host-supplied count
    /// with no relation to the bytes remaining, so a five-byte
    /// `0x9a FFFFFFFF` header would reach `Vec::with_capacity(4_294_967_295)`
    /// before anything rejected it.
    ///
    /// The discriminating input is a header that LIES: it claims 1000 nodes
    /// and carries one. Correct code rejects the count before reading a
    /// single node; code without the pre-check reads until the bytes run out
    /// and fails with a CBOR error instead. Asserting the exact error is
    /// therefore a real barrier.
    #[test]
    fn a_node_array_header_is_bounded_before_anything_is_read() {
        let bytes = scene_with_node_array(1000, minimal_rect_node);
        let mut decoder = Decoder::new(&bytes);
        assert_eq!(
            decode_scene(&mut decoder),
            Err(MessageError::InvalidValue("scene node count"))
        );
    }

    /// The same trap and the same fix for a LINE's coordinate arrays, whose
    /// pre-check is masked by `validate_node`'s point-count bound.
    #[test]
    fn a_line_point_array_header_is_bounded_before_anything_is_read() {
        let bytes = scene_with_node_array(1, |encoder| {
            encoder.map(2);
            encoder.unsigned(0);
            encoder.unsigned(3); // LINE
            encoder.unsigned(1);
            encoder.map(3);
            encoder.unsigned(0);
            encoder.array(1000); // claims 1000 points ...
            encoder.signed(1); // ... and carries one.
            encoder.unsigned(1);
            encoder.array(1);
            encoder.signed(1);
            encoder.unsigned(2);
            encoder.signed(4);
        });
        let mut decoder = Decoder::new(&bytes);
        assert_eq!(
            decode_scene(&mut decoder),
            Err(MessageError::InvalidValue("scene line point count"))
        );
    }

    #[test]
    fn a_full_turn_is_a_multiple_of_360_not_a_degenerate_arc() {
        // Not behaviour this module implements -- it is the convention the
        // device's renderer reads, pinned here so a host author meets it.
        let arc = SceneArc {
            cx: 100,
            cy: 100,
            r: 50,
            start_deg: 270,
            end_deg: 630,
            width: 8,
            ..SceneArc::default()
        };
        assert!(validate_node(&SceneNode::Arc(arc)).is_ok());
    }
}
