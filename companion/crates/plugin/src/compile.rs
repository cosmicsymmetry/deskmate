//! Compiles a [`PluginManifest`] and a fetched [`providers::ProviderSnapshot`]
//! into a [`Scene`] the existing wire path carries.
//!
//! This module owns exactly three decisions manifest.rs and expr.rs both
//! deliberately left open:
//!
//! - **Binding vs. literal.** A `"{{ ... }}"` field is stripped of its
//!   delimiters and checked against [`protocol::binding_is_valid`] -- the
//!   device's own closed set, mirrored (and enforced) in `protocol::scene`.
//!   A match compiles to [`SceneValue::Binding`], which the device keeps
//!   ticking locally with the link down. Anything that merely *looks* like an
//!   attempt at that closed set (`timer.`, `time:`, `field.`, or the exact
//!   token `date`) but does not match it is refused as [`CompileError::UnknownBinding`]
//!   rather than falling through to the expression evaluator -- see
//!   `looks_like_binding_namespace`'s doc for why that refusal has to happen
//!   before `Expr::parse` ever runs. Everything else is a restricted
//!   expression, evaluated once, now, against the snapshot's data, and baked
//!   into a [`SceneValue::Literal`].
//! - **The one repeat form.** A `[[repeats]]` block (`manifest::Repeat`) names
//!   a fetched array by a dotted path, and its template node group is
//!   compiled once per element, capped at [`MAX_REPEAT_ITEMS`], with the
//!   0-based loop index substituted for the bare token `item` in every
//!   templated expression before it is parsed. `RowList`'s five-row shape is
//!   the precedent this generalises.
//! - **The shared stale/error footer.** `providers::ProviderSnapshot::stale`
//!   and `::error` are threaded straight into
//!   `app_core::scene_build::SceneDataState`, so a plugin card announces a
//!   data problem exactly the way the six retired templates did --
//!   `with_scene_data_state` is reused, not re-implemented.
//!
//! # Assets
//!
//! An `image` node, a `glyph` node, or a `text`/`glyph` node whose font names
//! an asset rather than a baked tier needs a content-addressed digest to put
//! on the wire. [`compile_scene`] resolves none of these itself -- it always
//! compiles against an empty [`AssetSet`], so any manifest that uses one of
//! these node shapes fails with [`CompileError::AssetNotResolved`] through
//! that entry point. [`compile_scene_with_assets`] is the real one: it takes
//! the [`AssetSet`] `assets::resolve_assets` already built for the manifest
//! and resolves every asset-bearing node against it, kind-checking each
//! reference (an `image` node naming a `font` asset is refused, not silently
//! drawn as if it were an image) rather than trusting the manifest author's
//! `kind` tag to match how a node uses it.

use protocol::{
    AssetKind, Scene, SceneAlign, SceneArc, SceneFont, SceneFontTier, SceneGlyph, SceneImage,
    SceneLine, SceneNode, SceneRect, SceneText, SceneValue,
};

use app_core::{BakedFontMetrics, SceneDataState, text_is_numeric, with_scene_data_state};

use crate::assets::AssetSet;
use crate::expr::{EvalContext, Expr, ExprError, FUEL_BUDGET, Fuel};
use crate::manifest::{Align, Font, FontTier, Node, PluginManifest, Point, Repeat};

/// Maximum repetitions the one repeat form expands to, however long the
/// fetched array actually is. Mirrors `RowListCard`'s five-row precedent
/// (`app_core::scene_build::build_row_list_scene`), which is the shape this
/// form generalises.
pub const MAX_REPEAT_ITEMS: usize = 5;

/// The scene canvas background this compiler emits. Not a manifest field --
/// every retired C template and every `scene_build` builder draws the same
/// true-black ground (`scene_build`'s private `COLOR_CANVAS`), and a plugin
/// face follows the same convention rather than inventing a second one.
const PLUGIN_CANVAS_BACKGROUND: u32 = 0x0000_0000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileError {
    /// A `{{ ... }}` value named something in the device-binding namespace
    /// (`timer.*`, `time:*`, `field.*`, `date`) that is not, in fact, one of
    /// the closed bindings the firmware evaluates locally
    /// (`protocol::binding_is_valid`). Refused rather than degraded to a
    /// literal: a degraded binding renders correctly once and then never
    /// ticks again, a defect no pixel test catches.
    UnknownBinding { text: String },
    /// A restricted expression (anything outside the binding namespace)
    /// failed to parse or evaluate. Wraps the specific `expr::ExprError`,
    /// which includes `OutputTooLong` for a provider value too large for
    /// `expr`'s own, larger bound (`expr::MAX_OUTPUT_LEN`, 4096 bytes) --
    /// that stays a named, fatal error because it can only be reached
    /// through `truncate(s, n)`, an explicit function an author called with
    /// too large an `n`, which is an authoring mistake worth surfacing
    /// rather than papering over. Contrast with a literal value's own,
    /// smaller wire bound (`protocol::MAX_SCENE_TEXT_LEN`), which this
    /// module truncates rather than refuses -- see `bound_literal`.
    Expression(ExprError),
    /// A node named an asset (by `file`) that the manifest's own
    /// `[[assets]]` table never declares, or that `compile_scene`'s empty
    /// default [`AssetSet`] was never given the chance to resolve --
    /// `compile_scene_with_assets` is the entry point that can actually
    /// resolve one.
    AssetNotResolved { asset: String },
    /// A node named an asset that the manifest's `[[assets]]` table does
    /// declare, but under a `kind` this node cannot use it as -- an `image`
    /// node naming a `font`/`icon-font` asset, or a `text`/`glyph` node's
    /// `font.asset` naming an `image` asset. `expected` is a short label for
    /// what the node needed (`"image"` or `"font"`).
    AssetKindMismatch {
        asset: String,
        expected: &'static str,
    },
    /// A `glyph` node's `glyph` field compiled to a device-side binding
    /// (`{{ field.foo }}`) rather than a literal. `SceneGlyph::name` is a
    /// plain `String` on the wire, not a `SceneValue` -- there is no way to
    /// carry a binding through it, unlike a `text` node's `value`.
    GlyphBindingNotSupported { text: String },
    /// A `glyph` node's `font` named a baked tier
    /// (`Font::Tier { .. }`) rather than an asset. Baked tiers have no icon
    /// glyphs; a `glyph` node only ever draws from an uploaded icon font.
    GlyphFontMustBeAsset,
    /// A `[[repeats]]` block's `source` resolved to a JSON value that is
    /// neither an array nor absent/null -- a real author error (the wrong
    /// field name), distinct from "no items yet", which is not an error.
    RepeatSourceNotArray { source: String },
    /// The manifest's own static shape -- top-level nodes, every repeat
    /// group's capped expansion, and one reserved slot for the shared
    /// stale/error footer -- exceeds `protocol::MAX_SCENE_NODES`. Computed
    /// here because it depends on the repeat cap and node counts together,
    /// which only this compiler has both of.
    TooManyNodes { limit: usize, actual: usize },
    /// The compiled scene fails `protocol::validate_scene`'s own bounds --
    /// most likely out-of-canvas node geometry authored directly in the
    /// manifest, which nothing upstream of this check validates. Checked
    /// unconditionally, not only in debug builds: unlike app-core's static
    /// Rust scene builders, plugin geometry arrives from an untrusted TOML
    /// manifest at *runtime*, so a release server must not compile an
    /// invalid manifest to `Ok` and let the device reject the whole scene
    /// with no named host-side error.
    Invalid(protocol::MessageError),
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for CompileError {}

// ---------------------------------------------------------------------------
// Binding-position detection.
// ---------------------------------------------------------------------------

/// True when `text` names the device-binding namespace -- the prefixes and
/// exact token `protocol::binding_is_valid` itself recognises
/// (`firmware/main/core/scene_binding.c`, mirrored in
/// `protocol::scene::binding_is_valid`) -- whether or not it is actually a
/// *valid* member of that closed set.
///
/// This exists because `protocol::binding_is_valid` alone cannot distinguish
/// "not a binding, evaluate as an expression" from "a mistyped binding" --
/// both return `false`. Without this check, a mistyped binding like
/// `timer.velocity` would fall through to `Expr::parse`, which (correctly,
/// since `Expr`'s only path root is `data`) rejects it too, but with the
/// wrong, unrelated error (`UnknownIdentifier`) instead of the one that
/// names the real problem. This function's whole job is making that
/// refusal explicit and correctly named, not relying on `Expr`'s grammar to
/// incidentally reject the same input for an unrelated reason -- a future,
/// more permissive `Expr` grammar could stop rejecting it at all, silently
/// turning a mistyped binding into a baked-forever literal.
fn looks_like_binding_namespace(text: &str) -> bool {
    text == "date"
        || text.starts_with("timer.")
        || text.starts_with("time:")
        || text.starts_with("field.")
}

/// Strips a `"{{ ... }}"` wrapper, returning the trimmed inner source.
/// `None` means `source` is a plain literal, used verbatim.
fn extract_expression(source: &str) -> Option<&str> {
    let trimmed = source.trim();
    trimmed.strip_prefix("{{")?.strip_suffix("}}")
}

/// Replaces every occurrence of the bare identifier token `item` in `source`
/// with `index`'s decimal form, leaving everything else alone: a longer
/// identifier that merely contains "item" (such as `items`), a trailing path
/// segment named `item` (such as `field.item` or `data.item`, which names a
/// provider or binding field literally called "item"), and any occurrence
/// inside a `"..."` string literal (such as `{{ "item" }}`). This is a plain
/// text substitution performed *before* `Expr::parse` ever sees the source,
/// which is what lets a repeated node write `data.rows[item].label` using
/// `Expr`'s existing, unmodified `[digit+]` array-index grammar: by the time
/// it parses, `item` is already `"2"`.
fn substitute_item_token(source: &str, index: usize) -> String {
    fn is_ident_byte(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'_'
    }

    let mut out = String::with_capacity(source.len());
    let bytes = source.as_bytes();
    let mut i = 0;
    let mut in_string = false;
    while i < bytes.len() {
        if in_string {
            // Mirror `Expr::parse_string`'s escape handling just enough to
            // find the closing quote without treating an escaped `\"` as
            // one, and without ever substituting inside literal text.
            if bytes[i] == b'\\' {
                out.push('\\');
                i += 1;
                if i < bytes.len() {
                    let ch = source[i..]
                        .chars()
                        .next()
                        .expect("i is a valid char boundary within source");
                    out.push(ch);
                    i += ch.len_utf8();
                }
                continue;
            }
            if bytes[i] == b'"' {
                in_string = false;
                out.push('"');
                i += 1;
                continue;
            }
            let ch = source[i..]
                .chars()
                .next()
                .expect("i is a valid char boundary within source");
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }

        if bytes[i] == b'"' {
            in_string = true;
            out.push('"');
            i += 1;
            continue;
        }

        if source[i..].starts_with("item") {
            // A preceding `.` means `item` is a trailing path segment (a
            // provider/binding field literally named "item"), not the
            // standalone loop-index token -- do not substitute it.
            let before_ok = i == 0 || (!is_ident_byte(bytes[i - 1]) && bytes[i - 1] != b'.');
            let after = i + 4;
            let after_ok = after >= bytes.len() || !is_ident_byte(bytes[after]);
            if before_ok && after_ok {
                out.push_str(&index.to_string());
                i = after;
                continue;
            }
        }
        // Bounds-checked, char-boundary-safe advance for the non-match case,
        // including multi-byte UTF-8 -- this scan must never panic on
        // untrusted (bounded, but arbitrary) manifest text.
        let ch = source[i..]
            .chars()
            .next()
            .expect("i is a valid char boundary within source");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Compiles one `value`/`glyph` field's raw manifest source (which may or
/// may not be a `"{{ ... }}"` expression) into a [`SceneValue`].
///
/// `item_index`, when `Some`, is the current repetition's 0-based loop
/// index: it is substituted for the bare token `item` in the expression
/// source (see [`substitute_item_token`]) before anything else happens, so a
/// binding-namespace check or an `Expr::parse` never sees the literal word
/// `item`.
fn compile_value_source(
    source: &str,
    ctx: &EvalContext<'_>,
    fuel: &mut Fuel,
    item_index: Option<usize>,
) -> Result<SceneValue, CompileError> {
    let Some(inner) = extract_expression(source) else {
        return Ok(bound_literal(source));
    };
    let mut trimmed = inner.trim().to_string();
    if let Some(index) = item_index {
        trimmed = substitute_item_token(&trimmed, index);
    }

    if protocol::binding_is_valid(&trimmed) {
        return Ok(SceneValue::Binding(trimmed));
    }
    if looks_like_binding_namespace(&trimmed) {
        return Err(CompileError::UnknownBinding { text: trimmed });
    }

    let expr = Expr::parse(&trimmed).map_err(CompileError::Expression)?;
    let value = expr.eval(ctx, fuel).map_err(CompileError::Expression)?;
    // `EvalValue`'s `Display` impl renders `Missing` (an absent field, a JSON
    // `null`, a type mismatch) as an empty string; nothing here treats that
    // as an error, since a missing plugin data field is an ordinary, expected
    // outcome, not a card fault (that is what the stale/error footer is for).
    Ok(bound_literal(&value.to_string()))
}

/// Bounds a face node's literal to `protocol::MAX_SCENE_TEXT_LEN` bytes by
/// truncating at a valid UTF-8 char boundary, rather than refusing to
/// compile the scene.
///
/// This used to be a fatal `CompileError::ValueTooLong`, refusing the
/// *whole scene* over one field a step this ordinary: an authored heading a
/// few characters too long, or (found in the final whole-stage review) an
/// entirely ordinary provider string -- an API title or description just
/// over 128 bytes -- blanking the card forever, since
/// `plugin_provider::classify_plugin_failure` rules a `CompileError`
/// permanent. That is the same class of defect `NonNumericTextForTier` was
/// fixed as earlier in this stage, and the fix is the same shape: degrade
/// gracefully instead of refusing. It is deliberately the *display* rule,
/// not `expr`'s rule -- see `CompileError::Expression`'s doc for why
/// `expr::MAX_OUTPUT_LEN`/`OutputTooLong` stays a named, fatal error.
/// Shares its char-boundary walk-back with `bound_footer_text`, the other
/// place text is trimmed for display rather than refused.
fn bound_literal(text: &str) -> SceneValue {
    SceneValue::Literal(truncate_to_scene_text_len(text))
}

/// Truncates `text` to `protocol::MAX_SCENE_TEXT_LEN` bytes at a valid
/// UTF-8 char boundary. Shared by [`bound_literal`] (a face node's literal)
/// and `bound_footer_text` (the shared stale/error footer) -- the one place
/// this walk-back is written.
fn truncate_to_scene_text_len(text: &str) -> String {
    if text.len() <= protocol::MAX_SCENE_TEXT_LEN {
        return text.to_string();
    }
    let mut end = protocol::MAX_SCENE_TEXT_LEN;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

// ---------------------------------------------------------------------------
// Small, total, field-by-field conversions.
// ---------------------------------------------------------------------------

const fn scene_font_tier(tier: FontTier) -> SceneFontTier {
    match tier {
        FontTier::Caption => SceneFontTier::Caption,
        FontTier::Body => SceneFontTier::Body,
        FontTier::Display => SceneFontTier::Display,
        FontTier::Hero => SceneFontTier::Hero,
    }
}

const fn scene_align(align: Align) -> SceneAlign {
    match align {
        Align::Left => SceneAlign::Left,
        Align::Center => SceneAlign::Center,
        Align::Right => SceneAlign::Right,
    }
}

/// Resolves a manifest asset `file` name to its digest, refusing both an
/// undeclared name (`AssetNotResolved`) and a resolved one that is the wrong
/// `kind` to be used as a font (`AssetKindMismatch`) -- a plain `Font` asset
/// or an `IconFont` asset both work here, since both name a TTF the device
/// loads through the same `font_registry_acquire` path; only `Image` is
/// refused.
fn resolve_font_asset(
    assets: &AssetSet,
    asset: &str,
) -> Result<[u8; protocol::ASSET_DIGEST_LEN], CompileError> {
    let resolved = assets
        .get(asset)
        .ok_or_else(|| CompileError::AssetNotResolved {
            asset: asset.to_string(),
        })?;
    match resolved.kind {
        AssetKind::Font | AssetKind::IconFont => Ok(resolved.digest),
        AssetKind::Image => Err(CompileError::AssetKindMismatch {
            asset: asset.to_string(),
            expected: "font",
        }),
    }
}

/// Resolves a manifest `Font` to a wire `SceneFont` against `assets`. The
/// baked-tier variant needs no resolution at all; `Font::Asset` resolves
/// through [`resolve_font_asset`].
fn resolve_scene_font(font: &Font, assets: &AssetSet) -> Result<SceneFont, CompileError> {
    match font {
        Font::Tier { tier } => Ok(SceneFont::Baked(scene_font_tier(*tier))),
        Font::Asset { asset, pixel_size } => Ok(SceneFont::Asset {
            digest: resolve_font_asset(assets, asset)?,
            pixel_size: *pixel_size,
        }),
    }
}

/// Steps a Display/Hero literal's *authored* tier down to `Body` when it
/// cannot render `text` -- never up to a larger tier.
///
/// `app_core::number_font_tier` always tries `Hero` before `Display`: it
/// was written for callers that dynamically pick the biggest tier that
/// fits (`digital_clock.c` deliberately does *not* call it, precisely
/// because it pins `Hero` rather than letting it float). A plugin manifest
/// author, by contrast, names one specific tier -- an authoring decision,
/// not a hint -- so this only ever tries `tier` itself, falling back to
/// `Body` (the one baked tier with no digits-only restriction) when `text`
/// is not numeric or does not fit `max_width` at that tier.
fn capped_number_font_tier(
    tier: SceneFontTier,
    text: &str,
    max_width: i32,
    metrics: &BakedFontMetrics,
) -> SceneFontTier {
    if text_is_numeric(text)
        && metrics
            .measure(tier, text)
            .is_some_and(|width| width <= max_width)
    {
        tier
    } else {
        SceneFontTier::Body
    }
}

// ---------------------------------------------------------------------------
// Per-repetition geometry offset.
// ---------------------------------------------------------------------------

/// Applies a repeat block's `(dx, dy)` offset to a cloned template node's
/// geometry fields, leaving every non-geometry field untouched. Written as
/// an explicit per-kind match, not a generic "shift the first two i32
/// fields" trick, so a future node-kind addition is a compile error here
/// instead of a silently wrong offset.
#[allow(clippy::too_many_lines)]
fn offset_node(node: Node, dx: i32, dy: i32) -> Node {
    match node {
        Node::Rect {
            x,
            y,
            w,
            h,
            radius,
            fill,
            opacity,
        } => Node::Rect {
            x: x + dx,
            y: y + dy,
            w,
            h,
            radius,
            fill,
            opacity,
        },
        Node::Arc {
            cx,
            cy,
            r,
            start_deg,
            end_deg,
            width,
            color,
            caps,
        } => Node::Arc {
            cx: cx + dx,
            cy: cy + dy,
            r,
            start_deg,
            end_deg,
            width,
            color,
            caps,
        },
        Node::Line {
            points,
            width,
            color,
        } => Node::Line {
            points: points
                .into_iter()
                .map(|p| Point {
                    x: p.x + dx,
                    y: p.y + dy,
                })
                .collect(),
            width,
            color,
        },
        Node::Text {
            x,
            baseline_y,
            w,
            align,
            font,
            color,
            value,
            ellipsize,
        } => Node::Text {
            x: x + dx,
            baseline_y: baseline_y + dy,
            w,
            align,
            font,
            color,
            value,
            ellipsize,
        },
        Node::Image {
            x,
            y,
            w,
            h,
            asset,
            recolor,
            color,
        } => Node::Image {
            x: x + dx,
            y: y + dy,
            w,
            h,
            asset,
            recolor,
            color,
        },
        Node::Glyph {
            x,
            y,
            font,
            color,
            glyph,
        } => Node::Glyph {
            x: x + dx,
            y: y + dy,
            font,
            color,
            glyph,
        },
    }
}

// ---------------------------------------------------------------------------
// Node compilation.
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_lines)]
fn compile_node(
    node: &Node,
    ctx: &EvalContext<'_>,
    fuel: &mut Fuel,
    metrics: &BakedFontMetrics,
    item_index: Option<usize>,
    assets: &AssetSet,
) -> Result<SceneNode, CompileError> {
    match node {
        Node::Rect {
            x,
            y,
            w,
            h,
            radius,
            fill,
            opacity,
        } => Ok(SceneNode::Rect(SceneRect {
            x: *x,
            y: *y,
            w: *w,
            h: *h,
            radius: *radius,
            fill: *fill,
            opacity: *opacity,
            clip: None,
        })),
        Node::Arc {
            cx,
            cy,
            r,
            start_deg,
            end_deg,
            width,
            color,
            caps,
        } => Ok(SceneNode::Arc(SceneArc {
            cx: *cx,
            cy: *cy,
            r: *r,
            start_deg: *start_deg,
            end_deg: *end_deg,
            width: *width,
            color: *color,
            running_color: None,
            opacity: u8::MAX,
            rounded: *caps,
            end_binding: String::new(),
        })),
        Node::Line {
            points,
            width,
            color,
        } => {
            let xs = points.iter().map(|p| p.x).collect();
            let ys = points.iter().map(|p| p.y).collect();
            Ok(SceneNode::Line(SceneLine {
                xs,
                ys,
                width: *width,
                color: *color,
                pivot_x: 0,
                pivot_y: 0,
                length: 0,
                angle_binding: String::new(),
            }))
        }
        Node::Text {
            x,
            baseline_y,
            w,
            align,
            font,
            color,
            value,
            ellipsize,
        } => {
            let scene_font = resolve_scene_font(font, assets)?;
            let scene_value = compile_value_source(value, ctx, fuel, item_index)?;
            // A literal on a digits-only baked tier (`Display`/`Hero`) that
            // is not, in fact, digits-only content (a transient provider
            // value like "42.5", "N/A", or "12 km" is entirely realistic)
            // must not fail the whole scene -- a transient data value is
            // not a permanent card fault. `capped_number_font_tier` steps
            // the *authored* tier down to `Body` when it cannot render the
            // text -- never up. `app_core::number_font_tier` cannot be
            // reused directly here: it always tries `Hero` before
            // `Display`, which silently promoted an authored `Display` node
            // to `Hero` whenever the text also happened to fit Hero's
            // width. A manifest's tier is an authoring decision, not a
            // hint.
            let scene_font =
                if let SceneFont::Baked(tier @ (SceneFontTier::Display | SceneFontTier::Hero)) =
                    scene_font
                    && let SceneValue::Literal(text) = &scene_value
                {
                    SceneFont::Baked(capped_number_font_tier(tier, text, *w, metrics))
                } else {
                    scene_font
                };
            Ok(SceneNode::Text(SceneText {
                x: *x,
                baseline_y: *baseline_y,
                w: *w,
                align: scene_align(*align),
                font: scene_font,
                color: *color,
                running_color: None,
                value: scene_value,
                ellipsize: *ellipsize,
            }))
        }
        Node::Image {
            x,
            y,
            w,
            h,
            asset,
            recolor,
            color,
        } => {
            let resolved = assets
                .get(asset)
                .ok_or_else(|| CompileError::AssetNotResolved {
                    asset: asset.clone(),
                })?;
            if resolved.kind != AssetKind::Image {
                return Err(CompileError::AssetKindMismatch {
                    asset: asset.clone(),
                    expected: "image",
                });
            }
            Ok(SceneNode::Image(SceneImage {
                x: *x,
                y: *y,
                w: *w,
                h: *h,
                digest: resolved.digest,
                recolor: *recolor,
                color: *color,
            }))
        }
        Node::Glyph {
            x,
            y,
            font,
            color,
            glyph,
        } => {
            let Font::Asset { asset, pixel_size } = font else {
                return Err(CompileError::GlyphFontMustBeAsset);
            };
            let digest = resolve_font_asset(assets, asset)?;
            // `SceneGlyph::name` is a plain `String`, not a `SceneValue` --
            // there is no wire shape for a device-side glyph binding, so a
            // `{{ field.foo }}`-style result is refused rather than
            // silently downgraded to its binding text.
            let name = match compile_value_source(glyph, ctx, fuel, item_index)? {
                SceneValue::Literal(text) => text,
                SceneValue::Binding(text) => {
                    return Err(CompileError::GlyphBindingNotSupported { text });
                }
            };
            Ok(SceneNode::Glyph(SceneGlyph {
                x: *x,
                baseline_y: *y,
                size: *pixel_size,
                digest,
                name,
                color: *color,
            }))
        }
    }
}

// ---------------------------------------------------------------------------
// The repeat form.
// ---------------------------------------------------------------------------

/// Resolves `path` (a plain `a.b.c` dotted chain, no array-index syntax --
/// this only needs to *find* the array a repeat block iterates, not walk
/// into one) against `data` and returns the array's length. A missing field
/// anywhere along the chain, or an explicit JSON `null` at the end, means
/// "no items yet" (`Ok(0)`), not an error -- an empty list is an ordinary,
/// common state. A present value that is neither an array nor `null` is a
/// real author error: the field name is probably wrong.
fn repeat_array_len(data: &serde_json::Value, path: &str) -> Result<usize, CompileError> {
    let mut current = data;
    for segment in path.split('.') {
        match current.get(segment) {
            Some(next) => current = next,
            None => return Ok(0),
        }
    }
    match current {
        serde_json::Value::Array(items) => Ok(items.len()),
        serde_json::Value::Null => Ok(0),
        _ => Err(CompileError::RepeatSourceNotArray {
            source: path.to_string(),
        }),
    }
}

fn compile_repeat(
    repeat: &Repeat,
    data: &serde_json::Value,
    ctx: &EvalContext<'_>,
    fuel: &mut Fuel,
    metrics: &BakedFontMetrics,
    out: &mut Vec<SceneNode>,
    assets: &AssetSet,
) -> Result<(), CompileError> {
    let available = repeat_array_len(data, &repeat.source)?;
    let count = available.min(MAX_REPEAT_ITEMS);
    for index in 0..count {
        let index_i32 = i32::try_from(index).unwrap_or(i32::MAX);
        let dx = repeat.dx.saturating_mul(index_i32);
        let dy = repeat.dy.saturating_mul(index_i32);
        for template in &repeat.nodes {
            let offset = offset_node(template.clone(), dx, dy);
            out.push(compile_node(
                &offset,
                ctx,
                fuel,
                metrics,
                Some(index),
                assets,
            )?);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Entry point.
// ---------------------------------------------------------------------------

/// Compiles a manifest and a fetched provider snapshot into a [`Scene`],
/// against an empty [`AssetSet`] -- so any `image` node, `glyph` node, or
/// asset-font reference fails with [`CompileError::AssetNotResolved`]. Kept
/// as the no-assets entry point every existing caller and test already
/// depends on; [`compile_scene_with_assets`] is the one that can actually
/// resolve an asset-bearing manifest, and this function is defined in terms
/// of it rather than duplicating its body.
///
/// # Errors
///
/// See [`CompileError`]'s variants.
pub fn compile_scene(
    manifest: &PluginManifest,
    snapshot: &providers::ProviderSnapshot<serde_json::Value>,
    metrics: &BakedFontMetrics,
    revision: u32,
) -> Result<Scene, CompileError> {
    compile_scene_with_assets(manifest, snapshot, metrics, revision, &AssetSet::default())
}

/// As [`compile_scene`], but resolves `image`/`glyph` nodes and asset-font
/// references against `assets` -- the [`AssetSet`] `assets::resolve_assets`
/// built for this same manifest. `snapshot.value` is evaluated against
/// unconditionally, whether or not the snapshot is fresh -- a stale/error
/// snapshot still carries the last-good value
/// (`providers::LastGood::complete`'s contract), so a plugin card keeps
/// showing its last-good reading under the shared stale/error footer, the
/// same way the six retired templates did.
///
/// # Errors
///
/// See [`CompileError`]'s variants.
pub fn compile_scene_with_assets(
    manifest: &PluginManifest,
    snapshot: &providers::ProviderSnapshot<serde_json::Value>,
    metrics: &BakedFontMetrics,
    revision: u32,
    assets: &AssetSet,
) -> Result<Scene, CompileError> {
    // Final whole-stage review finding 2: the `icon(name)` lookup table
    // comes from `assets` itself -- the same `icon_codepoints()` table
    // `AssetSet::icon_codepoint` resolves a single name against -- rather
    // than a second table recomputed from the manifest's raw text. There is
    // now exactly one place this table is built (`assets::resolve_assets`),
    // so the two can never disagree about a duplicate name: `resolve_assets`
    // already rejects one before `assets` can exist to be passed in here.
    let ctx = EvalContext::new(&snapshot.value, assets.icon_codepoints());
    let mut fuel = Fuel::new(FUEL_BUDGET);
    let mut nodes = Vec::with_capacity(manifest.nodes.len());

    for node in &manifest.nodes {
        nodes.push(compile_node(node, &ctx, &mut fuel, metrics, None, assets)?);
    }

    for repeat in &manifest.repeats {
        compile_repeat(
            repeat,
            &snapshot.value,
            &ctx,
            &mut fuel,
            metrics,
            &mut nodes,
            assets,
        )?;
    }

    // Reserve one slot for the shared stale/error footer `with_scene_data_state`
    // may append below, so a manifest that fits today does not blow the wire's
    // node ceiling the moment its provider goes stale -- the same reservation
    // `build_row_list_scene`'s five-row shape makes against this exact ceiling.
    let reserved_total = nodes.len() + 1;
    if reserved_total > protocol::MAX_SCENE_NODES {
        return Err(CompileError::TooManyNodes {
            limit: protocol::MAX_SCENE_NODES,
            actual: reserved_total,
        });
    }

    let scene = Scene {
        revision,
        background: PLUGIN_CANVAS_BACKGROUND,
        nodes,
    };
    // Validate the manifest-authored scene -- before the shared footer is
    // appended, and unconditionally, not only in debug builds. Unlike
    // app-core's static Rust scene builders (where `with_scene_data_state`'s
    // debug-only assertion below is sufficient), plugin geometry arrives
    // from an untrusted TOML manifest at *runtime*: a release build must not
    // compile an out-of-canvas manifest to `Ok` and let the device reject
    // the whole scene with no named host-side error. Checking here, before
    // the footer, rather than after, also avoids a spurious panic: in a
    // debug build, `with_scene_data_state`'s own assertion would otherwise
    // fire on the same already-invalid geometry before this function ever
    // gets to return the named `Err`. The footer itself needs no separate
    // check -- its geometry is fixed and its text is bounded below, so it
    // cannot turn an already-valid scene invalid, and the node-count
    // reservation above already accounts for its one extra node.
    protocol::validate_scene(&scene).map_err(CompileError::Invalid)?;
    // Bound the footer's error text the same way `bound_literal` bounds
    // every face node's literal. `providers::LastGood` truncates to a
    // tighter 96-byte bound, but `compile_scene` accepts any
    // `providers::ProviderSnapshot`, so an error that did not pass through
    // `LastGood` must still be bounded here -- truncation, not refusal, is
    // correct for a display footer, the same way the retired C templates
    // truncated an oversized error.
    let bounded_error = snapshot.error.as_deref().map(bound_footer_text);
    let state = SceneDataState {
        stale: snapshot.stale,
        error: bounded_error.as_deref(),
    };
    Ok(with_scene_data_state(scene, state, metrics))
}

/// Bounds `text` to `protocol::MAX_SCENE_TEXT_LEN` bytes, truncating at a
/// valid UTF-8 char boundary. Used for the shared stale/error footer's
/// text -- `bound_literal` guards every face node's literal the same way,
/// for the same reason: this is a display footer summarizing a fault, not
/// authored content, and the retired C templates truncated an oversized
/// error the same way. Shares its walk-back with `bound_literal` via
/// [`truncate_to_scene_text_len`].
fn bound_footer_text(text: &str) -> String {
    truncate_to_scene_text_len(text)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{
        Align as ManifestAlign, Font as ManifestFont, FontTier as ManifestFontTier, Source,
    };

    fn metrics() -> BakedFontMetrics {
        BakedFontMetrics::SHIPPED
    }

    fn snapshot() -> providers::ProviderSnapshot<serde_json::Value> {
        providers::ProviderSnapshot {
            value: serde_json::json!({ "aqi": 42, "rows": [] }),
            refreshed_at: None,
            age: None,
            stale: false,
            error: None,
        }
    }

    fn stale_snapshot() -> providers::ProviderSnapshot<serde_json::Value> {
        providers::ProviderSnapshot {
            stale: true,
            ..snapshot()
        }
    }

    fn error_snapshot(message: &str) -> providers::ProviderSnapshot<serde_json::Value> {
        providers::ProviderSnapshot {
            stale: true,
            error: Some(message.to_string()),
            ..snapshot()
        }
    }

    fn text_node_manifest(value: &str) -> PluginManifest {
        PluginManifest {
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            source: Source::Json {
                url: "https://example.invalid/x.json".to_string(),
                refresh_minutes: 15,
            },
            assets: Vec::new(),
            nodes: vec![Node::Text {
                x: 24,
                baseline_y: 96,
                w: 400,
                align: ManifestAlign::Center,
                font: ManifestFont::Tier {
                    tier: ManifestFontTier::Body,
                },
                color: 0x00FF_FFFF,
                value: value.to_string(),
                ellipsize: false,
            }],
            repeats: Vec::new(),
        }
    }

    fn manifest_with_value(value: &str) -> PluginManifest {
        text_node_manifest(value)
    }

    fn text_node(scene: &Scene, index: usize) -> &SceneText {
        match &scene.nodes[index] {
            SceneNode::Text(text) => text,
            other => panic!("node {index} is not Text: {other:?}"),
        }
    }

    // -- Step 1: the load-bearing binding tests, verbatim from the brief. --

    #[test]
    fn a_closed_set_expression_compiles_to_a_binding_not_a_literal() {
        let scene = compile_scene(
            &manifest_with_value("{{ time:HH:mm }}"),
            &snapshot(),
            &metrics(),
            1,
        )
        .unwrap();
        assert_eq!(
            SceneValue::Binding("time:HH:mm".into()),
            text_node(&scene, 0).value
        );
    }

    #[test]
    fn an_expression_outside_the_closed_set_is_refused_in_binding_position() {
        let err = compile_scene(
            &manifest_with_value("{{ timer.velocity }}"),
            &snapshot(),
            &metrics(),
            1,
        )
        .unwrap_err();
        assert!(matches!(err, CompileError::UnknownBinding { .. }));
    }

    // -- More of the closed set, and its neighbouring "looks like but isn't". --

    #[test]
    fn date_is_a_valid_binding() {
        let scene = compile_scene(
            &manifest_with_value("{{ date }}"),
            &snapshot(),
            &metrics(),
            1,
        )
        .unwrap();
        assert_eq!(
            SceneValue::Binding("date".into()),
            text_node(&scene, 0).value
        );
    }

    #[test]
    fn timer_permille_is_a_valid_binding() {
        let scene = compile_scene(
            &manifest_with_value("{{ timer.permille }}"),
            &snapshot(),
            &metrics(),
            1,
        )
        .unwrap();
        assert_eq!(
            SceneValue::Binding("timer.permille".into()),
            text_node(&scene, 0).value
        );
    }

    #[test]
    fn field_dot_name_is_a_valid_binding() {
        let scene = compile_scene(
            &manifest_with_value("{{ field.status }}"),
            &snapshot(),
            &metrics(),
            1,
        )
        .unwrap();
        assert_eq!(
            SceneValue::Binding("field.status".into()),
            text_node(&scene, 0).value
        );
    }

    #[test]
    fn a_mistyped_time_format_is_refused_as_unknown_binding_not_an_expression_error() {
        // "time:XYZ" is in the binding namespace (starts with "time:") but "X"
        // and "Y" aren't in the whitelisted format alphabet -- this must be
        // named as an unknown binding, not fall through to Expr and produce an
        // unrelated parse error.
        let err = compile_scene(
            &manifest_with_value("{{ time:XYZ }}"),
            &snapshot(),
            &metrics(),
            1,
        )
        .unwrap_err();
        assert!(matches!(err, CompileError::UnknownBinding { .. }));
    }

    #[test]
    fn a_field_binding_with_no_name_is_refused_as_unknown_binding() {
        let err = compile_scene(
            &manifest_with_value("{{ field. }}"),
            &snapshot(),
            &metrics(),
            1,
        )
        .unwrap_err();
        assert!(matches!(err, CompileError::UnknownBinding { .. }));
    }

    // -- Plain literals and real expressions, outside the binding namespace. --

    #[test]
    fn a_plain_literal_value_compiles_unchanged() {
        let scene =
            compile_scene(&manifest_with_value("static"), &snapshot(), &metrics(), 1).unwrap();
        assert_eq!(
            SceneValue::Literal("static".to_string()),
            text_node(&scene, 0).value
        );
    }

    #[test]
    fn a_data_expression_evaluates_to_a_literal_at_compile_time() {
        let scene = compile_scene(
            &manifest_with_value("{{ data.aqi }}"),
            &snapshot(),
            &metrics(),
            1,
        )
        .unwrap();
        assert_eq!(
            SceneValue::Literal("42".to_string()),
            text_node(&scene, 0).value
        );
    }

    #[test]
    fn a_syntax_error_in_a_real_expression_is_a_named_expression_error() {
        let err = compile_scene(
            &manifest_with_value("{{ upper(data.aqi }}"),
            &snapshot(),
            &metrics(),
            1,
        )
        .unwrap_err();
        assert!(matches!(err, CompileError::Expression(_)));
    }

    #[test]
    fn a_provider_value_over_the_wire_bound_truncates_instead_of_failing_the_scene() {
        // Final whole-stage review finding 3: an ordinary long API title or
        // description must not blank the card forever. 129 bytes is exactly
        // one past `protocol::MAX_SCENE_TEXT_LEN` (128) -- the smallest
        // input that exercises the bound at all.
        let long_value = "x".repeat(protocol::MAX_SCENE_TEXT_LEN + 1);
        assert_eq!(long_value.len(), protocol::MAX_SCENE_TEXT_LEN + 1);
        let snapshot = providers::ProviderSnapshot {
            value: serde_json::json!({ "big": long_value }),
            refreshed_at: None,
            age: None,
            stale: false,
            error: None,
        };
        let scene = compile_scene(
            &manifest_with_value("{{ data.big }}"),
            &snapshot,
            &metrics(),
            1,
        )
        .expect("a long provider value must truncate, not fail the scene");

        let expected = "x".repeat(protocol::MAX_SCENE_TEXT_LEN);
        assert_eq!(expected.len(), protocol::MAX_SCENE_TEXT_LEN);
        assert_eq!(
            SceneValue::Literal(expected),
            text_node(&scene, 0).value,
            "the literal must be truncated to exactly MAX_SCENE_TEXT_LEN bytes at a char \
             boundary, not refused"
        );
    }

    #[test]
    fn expr_output_too_long_propagates_as_a_named_expression_error() {
        // A provider value past expr's own MAX_OUTPUT_LEN (4096) must surface
        // as ExprError::OutputTooLong, not be silently truncated or treated as
        // Missing -- see Task 2's controller ruling.
        let huge = "x".repeat(crate::expr::MAX_OUTPUT_LEN + 1);
        let snapshot = providers::ProviderSnapshot {
            value: serde_json::json!({ "huge": huge }),
            refreshed_at: None,
            age: None,
            stale: false,
            error: None,
        };
        let err = compile_scene(
            &manifest_with_value("{{ data.huge }}"),
            &snapshot,
            &metrics(),
            1,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            CompileError::Expression(ExprError::OutputTooLong { .. })
        ));
    }

    // -- Step 6: every ProviderSnapshot state reaches the shared footer. --

    #[test]
    fn a_fresh_snapshot_has_no_state_footer() {
        let scene =
            compile_scene(&manifest_with_value("static"), &snapshot(), &metrics(), 1).unwrap();
        assert_eq!(scene.nodes.len(), 1, "no footer node in the OK state");
    }

    #[test]
    fn a_stale_snapshot_reaches_the_shared_stale_footer() {
        let scene = compile_scene(
            &manifest_with_value("static"),
            &stale_snapshot(),
            &metrics(),
            1,
        )
        .unwrap();
        assert_eq!(scene.nodes.len(), 2, "stale footer node appended");
        let footer = text_node(&scene, 1);
        assert_eq!(footer.value, SceneValue::Literal("Stale".to_string()));
    }

    #[test]
    fn an_error_snapshot_reaches_the_shared_error_footer() {
        let scene = compile_scene(
            &manifest_with_value("static"),
            &error_snapshot("fetch failed"),
            &metrics(),
            1,
        )
        .unwrap();
        assert_eq!(scene.nodes.len(), 2, "error footer node appended");
        let footer = text_node(&scene, 1);
        assert_eq!(
            footer.value,
            SceneValue::Literal("fetch failed".to_string())
        );
    }

    #[test]
    fn an_error_snapshot_still_evaluates_the_last_good_value_under_the_footer() {
        // The card keeps showing its last-good reading; only the footer
        // announces the fault, exactly as the six retired C templates did.
        let scene = compile_scene(
            &manifest_with_value("{{ data.aqi }}"),
            &error_snapshot("fetch failed"),
            &metrics(),
            1,
        )
        .unwrap();
        assert_eq!(
            SceneValue::Literal("42".to_string()),
            text_node(&scene, 0).value
        );
        assert_eq!(scene.nodes.len(), 2);
    }

    // -- Non-authorable-yet node kinds. --

    #[test]
    fn an_image_node_is_refused_as_asset_not_resolved() {
        let manifest = PluginManifest {
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            source: Source::Json {
                url: "https://example.invalid/x.json".to_string(),
                refresh_minutes: 15,
            },
            assets: Vec::new(),
            nodes: vec![Node::Image {
                x: 0,
                y: 0,
                w: 10,
                h: 10,
                asset: "logo.png".to_string(),
                recolor: false,
                color: 0,
            }],
            repeats: Vec::new(),
        };
        let err = compile_scene(&manifest, &snapshot(), &metrics(), 1).unwrap_err();
        assert!(matches!(
            err,
            CompileError::AssetNotResolved { asset } if asset == "logo.png"
        ));
    }

    #[test]
    fn a_glyph_node_is_refused_as_asset_not_resolved() {
        let manifest = PluginManifest {
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            source: Source::Json {
                url: "https://example.invalid/x.json".to_string(),
                refresh_minutes: 15,
            },
            assets: Vec::new(),
            nodes: vec![Node::Glyph {
                x: 0,
                y: 0,
                font: ManifestFont::Asset {
                    asset: "icons.ttf".to_string(),
                    pixel_size: 32,
                },
                color: 0,
                glyph: "{{ icon(data.category) }}".to_string(),
            }],
            repeats: Vec::new(),
        };
        let err = compile_scene(&manifest, &snapshot(), &metrics(), 1).unwrap_err();
        assert!(matches!(
            err,
            CompileError::AssetNotResolved { asset } if asset == "icons.ttf"
        ));
    }

    #[test]
    fn a_text_node_naming_an_asset_font_is_refused_as_asset_not_resolved() {
        let manifest = PluginManifest {
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            source: Source::Json {
                url: "https://example.invalid/x.json".to_string(),
                refresh_minutes: 15,
            },
            assets: Vec::new(),
            nodes: vec![Node::Text {
                x: 0,
                baseline_y: 40,
                w: 200,
                align: ManifestAlign::Left,
                font: ManifestFont::Asset {
                    asset: "Inter.ttf".to_string(),
                    pixel_size: 24,
                },
                color: 0,
                value: "static".to_string(),
                ellipsize: false,
            }],
            repeats: Vec::new(),
        };
        let err = compile_scene(&manifest, &snapshot(), &metrics(), 1).unwrap_err();
        assert!(matches!(
            err,
            CompileError::AssetNotResolved { asset } if asset == "Inter.ttf"
        ));
    }

    // -- Numeric-tier literal step-down (controller ruling: a transient
    // -- non-numeric provider value must not fail the whole scene). --

    fn tiered_text_manifest(tier: ManifestFontTier, value: &str) -> PluginManifest {
        PluginManifest {
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            source: Source::Json {
                url: "https://example.invalid/x.json".to_string(),
                refresh_minutes: 15,
            },
            assets: Vec::new(),
            nodes: vec![Node::Text {
                x: 0,
                baseline_y: 96,
                w: 400,
                align: ManifestAlign::Left,
                font: ManifestFont::Tier { tier },
                color: 0,
                value: value.to_string(),
                ellipsize: false,
            }],
            repeats: Vec::new(),
        }
    }

    fn hero_text_manifest(value: &str) -> PluginManifest {
        tiered_text_manifest(ManifestFontTier::Hero, value)
    }

    #[test]
    fn a_non_numeric_literal_on_the_hero_tier_compiles_and_steps_down_to_body() {
        // "42.5" (a decimal reading), "N/A" (a common provider sentinel),
        // and "12 km" (units) are all realistic transient provider values --
        // none is digits-only-subset content, so all three must land on
        // Body rather than fail the scene.
        for value in ["42.5", "N/A", "12 km"] {
            let manifest = hero_text_manifest(value);
            let scene = compile_scene(&manifest, &snapshot(), &metrics(), 1)
                .unwrap_or_else(|err| panic!("{value:?} must compile, got {err:?}"));
            let node = text_node(&scene, 0);
            assert_eq!(
                node.font,
                SceneFont::Baked(SceneFontTier::Body),
                "{value:?} must step down to Body"
            );
            assert_eq!(node.value, SceneValue::Literal(value.to_string()));
        }
    }

    #[test]
    fn a_numeric_literal_on_the_hero_tier_is_accepted_and_keeps_the_hero_tier() {
        let manifest = PluginManifest {
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            source: Source::Json {
                url: "https://example.invalid/x.json".to_string(),
                refresh_minutes: 15,
            },
            assets: Vec::new(),
            nodes: vec![Node::Text {
                x: 0,
                baseline_y: 96,
                w: 400,
                align: ManifestAlign::Left,
                font: ManifestFont::Tier {
                    tier: ManifestFontTier::Hero,
                },
                color: 0,
                value: "{{ data.aqi }}".to_string(),
                ellipsize: false,
            }],
            repeats: Vec::new(),
        };
        let scene = compile_scene(&manifest, &snapshot(), &metrics(), 1).unwrap();
        let node = text_node(&scene, 0);
        assert_eq!(SceneValue::Literal("42".to_string()), node.value);
        assert_eq!(node.font, SceneFont::Baked(SceneFontTier::Hero));
    }

    // -- The tier cap (round-2 review fix): the compiler must never promote
    // -- an authored tier upward, only ever step it down to Body. --

    #[test]
    fn an_authored_display_tier_with_a_short_numeric_value_stays_display() {
        // Regression: `app_core::number_font_tier` tries `Hero` before
        // `Display`, and "5" fits Hero's width too -- calling it directly
        // silently promoted this node to Hero. This must stay `Display`.
        let manifest = tiered_text_manifest(ManifestFontTier::Display, "5");
        let scene = compile_scene(&manifest, &snapshot(), &metrics(), 1).unwrap();
        let node = text_node(&scene, 0);
        assert_eq!(node.font, SceneFont::Baked(SceneFontTier::Display));
        assert_eq!(node.value, SceneValue::Literal("5".to_string()));
    }

    #[test]
    fn an_authored_display_tier_with_a_non_numeric_value_steps_down_to_body() {
        let manifest = tiered_text_manifest(ManifestFontTier::Display, "N/A");
        let scene = compile_scene(&manifest, &snapshot(), &metrics(), 1).unwrap();
        let node = text_node(&scene, 0);
        assert_eq!(node.font, SceneFont::Baked(SceneFontTier::Body));
        assert_eq!(node.value, SceneValue::Literal("N/A".to_string()));
    }

    #[test]
    fn an_authored_hero_tier_with_a_short_numeric_value_stays_hero() {
        let manifest = tiered_text_manifest(ManifestFontTier::Hero, "5");
        let scene = compile_scene(&manifest, &snapshot(), &metrics(), 1).unwrap();
        let node = text_node(&scene, 0);
        assert_eq!(node.font, SceneFont::Baked(SceneFontTier::Hero));
        assert_eq!(node.value, SceneValue::Literal("5".to_string()));
    }

    // -- The repeat form. --

    #[allow(clippy::needless_pass_by_value)]
    fn repeat_manifest(
        rows: serde_json::Value,
    ) -> (
        PluginManifest,
        providers::ProviderSnapshot<serde_json::Value>,
    ) {
        let manifest = PluginManifest {
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            source: Source::Json {
                url: "https://example.invalid/x.json".to_string(),
                refresh_minutes: 15,
            },
            assets: Vec::new(),
            nodes: Vec::new(),
            repeats: vec![Repeat {
                source: "rows".to_string(),
                dx: 0,
                dy: 40,
                nodes: vec![Node::Text {
                    x: 10,
                    baseline_y: 20,
                    w: 200,
                    align: ManifestAlign::Left,
                    font: ManifestFont::Tier {
                        tier: ManifestFontTier::Body,
                    },
                    color: 0,
                    value: "{{ data.rows[item].label }}".to_string(),
                    ellipsize: false,
                }],
            }],
        };
        let snapshot = providers::ProviderSnapshot {
            value: serde_json::json!({ "rows": rows }),
            refreshed_at: None,
            age: None,
            stale: false,
            error: None,
        };
        (manifest, snapshot)
    }

    #[test]
    fn a_repeat_group_expands_once_per_array_element() {
        let (manifest, snapshot) = repeat_manifest(serde_json::json!([
            { "label": "a" },
            { "label": "b" },
            { "label": "c" }
        ]));
        let scene = compile_scene(&manifest, &snapshot, &metrics(), 1).unwrap();
        assert_eq!(scene.nodes.len(), 3);
        assert_eq!(
            text_node(&scene, 0).value,
            SceneValue::Literal("a".to_string())
        );
        assert_eq!(
            text_node(&scene, 1).value,
            SceneValue::Literal("b".to_string())
        );
        assert_eq!(
            text_node(&scene, 2).value,
            SceneValue::Literal("c".to_string())
        );
    }

    #[test]
    fn a_repeat_group_offsets_each_repetition_by_dy() {
        let (manifest, snapshot) = repeat_manifest(serde_json::json!([
            { "label": "a" },
            { "label": "b" }
        ]));
        let scene = compile_scene(&manifest, &snapshot, &metrics(), 1).unwrap();
        assert_eq!(text_node(&scene, 0).baseline_y, 20);
        assert_eq!(text_node(&scene, 1).baseline_y, 60);
    }

    #[test]
    fn a_repeat_group_is_capped_at_max_repeat_items() {
        let rows: Vec<serde_json::Value> = (0..MAX_REPEAT_ITEMS + 5)
            .map(|i| serde_json::json!({ "label": i.to_string() }))
            .collect();
        let (manifest, snapshot) = repeat_manifest(serde_json::Value::Array(rows));
        let scene = compile_scene(&manifest, &snapshot, &metrics(), 1).unwrap();
        assert_eq!(scene.nodes.len(), MAX_REPEAT_ITEMS);
    }

    #[test]
    fn a_missing_repeat_source_expands_to_zero_rows_not_an_error() {
        let manifest = PluginManifest {
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            source: Source::Json {
                url: "https://example.invalid/x.json".to_string(),
                refresh_minutes: 15,
            },
            assets: Vec::new(),
            nodes: Vec::new(),
            repeats: vec![Repeat {
                source: "missing_field".to_string(),
                dx: 0,
                dy: 0,
                nodes: vec![Node::Rect {
                    x: 0,
                    y: 0,
                    w: 1,
                    h: 1,
                    radius: 0,
                    fill: 0,
                    opacity: 255,
                }],
            }],
        };
        let scene = compile_scene(&manifest, &snapshot(), &metrics(), 1).unwrap();
        assert!(scene.nodes.is_empty());
    }

    #[test]
    fn a_repeat_source_naming_a_non_array_field_is_refused() {
        let (manifest, _) = repeat_manifest(serde_json::json!([]));
        let mut manifest = manifest;
        manifest.repeats[0].source = "not_an_array".to_string();
        let snapshot = providers::ProviderSnapshot {
            value: serde_json::json!({ "not_an_array": "oops" }),
            refreshed_at: None,
            age: None,
            stale: false,
            error: None,
        };
        let err = compile_scene(&manifest, &snapshot, &metrics(), 1).unwrap_err();
        assert!(matches!(
            err,
            CompileError::RepeatSourceNotArray { source } if source == "not_an_array"
        ));
    }

    // -- The node-count budget, including the reserved footer slot. --

    #[test]
    fn a_manifest_that_exactly_fills_the_node_budget_with_the_footer_reserved_compiles() {
        let nodes: Vec<Node> = (0..protocol::MAX_SCENE_NODES - 1)
            .map(|_| Node::Rect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
                radius: 0,
                fill: 0,
                opacity: 255,
            })
            .collect();
        let manifest = PluginManifest {
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            source: Source::Json {
                url: "https://example.invalid/x.json".to_string(),
                refresh_minutes: 15,
            },
            assets: Vec::new(),
            nodes,
            repeats: Vec::new(),
        };
        compile_scene(&manifest, &snapshot(), &metrics(), 1)
            .expect("MAX_SCENE_NODES - 1 face nodes plus the reserved footer slot must fit");
    }

    #[test]
    fn a_manifest_one_node_past_the_reserved_budget_is_refused_by_name() {
        let nodes: Vec<Node> = (0..protocol::MAX_SCENE_NODES)
            .map(|_| Node::Rect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
                radius: 0,
                fill: 0,
                opacity: 255,
            })
            .collect();
        let manifest = PluginManifest {
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            source: Source::Json {
                url: "https://example.invalid/x.json".to_string(),
                refresh_minutes: 15,
            },
            assets: Vec::new(),
            nodes,
            repeats: Vec::new(),
        };
        let err = compile_scene(&manifest, &snapshot(), &metrics(), 1).unwrap_err();
        assert!(matches!(
            err,
            CompileError::TooManyNodes {
                limit: n,
                actual
            } if n == protocol::MAX_SCENE_NODES && actual == protocol::MAX_SCENE_NODES + 1
        ));
    }

    // -- `item` substitution does not corrupt an unrelated identifier. --

    #[test]
    fn item_substitution_does_not_corrupt_a_longer_identifier() {
        assert_eq!(substitute_item_token("data.items[0]", 3), "data.items[0]");
        assert_eq!(substitute_item_token("data.rows[item]", 3), "data.rows[3]");
        assert_eq!(
            substitute_item_token("truncate(data.rows[item].label, item)", 7),
            "truncate(data.rows[7].label, 7)"
        );
    }

    #[test]
    fn item_substitution_leaves_a_trailing_path_segment_named_item_alone() {
        // `field.item`/`data.item` name a provider or binding field literally
        // called "item" -- a real field name, not the loop-index token.
        assert_eq!(substitute_item_token("field.item", 3), "field.item");
        assert_eq!(substitute_item_token("data.item", 3), "data.item");
    }

    #[test]
    fn item_substitution_does_not_touch_a_string_literal() {
        assert_eq!(substitute_item_token("\"item\"", 3), "\"item\"");
        assert_eq!(
            substitute_item_token("default(\"item\", item)", 5),
            "default(\"item\", 5)"
        );
    }

    #[test]
    fn item_substitution_still_replaces_a_bare_item_token() {
        assert_eq!(substitute_item_token("item", 3), "3");
    }

    // -- The same three cases, through the real compile pipeline
    // -- (`compile_value_source` with a repeat's `item_index`), so the fix
    // -- is proven where the corruption actually happened, not only in the
    // -- string-substitution helper. --

    #[test]
    fn a_binding_naming_a_field_literally_called_item_survives_repeat_compilation() {
        let data = serde_json::json!({});
        let ctx = EvalContext::with_data(&data);
        let mut fuel = Fuel::new(FUEL_BUDGET);
        let value = compile_value_source("{{ field.item }}", &ctx, &mut fuel, Some(2)).unwrap();
        assert_eq!(value, SceneValue::Binding("field.item".to_string()));
    }

    #[test]
    fn a_quoted_string_literal_item_survives_repeat_compilation() {
        let data = serde_json::json!({});
        let ctx = EvalContext::with_data(&data);
        let mut fuel = Fuel::new(FUEL_BUDGET);
        let value = compile_value_source(r#"{{ "item" }}"#, &ctx, &mut fuel, Some(2)).unwrap();
        assert_eq!(value, SceneValue::Literal("item".to_string()));
    }

    #[test]
    fn a_bare_item_token_still_substitutes_through_repeat_compilation() {
        let data = serde_json::json!({});
        let ctx = EvalContext::with_data(&data);
        let mut fuel = Fuel::new(FUEL_BUDGET);
        let value = compile_value_source("{{ item }}", &ctx, &mut fuel, Some(4)).unwrap();
        assert_eq!(value, SceneValue::Literal("4".to_string()));
    }

    // -- Unconditional whole-scene validation (finding 2): out-of-canvas
    // -- manifest geometry is refused with a named error in a normal test,
    // -- not only under `#[cfg(debug_assertions)]`. --

    #[test]
    fn out_of_canvas_node_geometry_is_refused_as_invalid() {
        let manifest = PluginManifest {
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            source: Source::Json {
                url: "https://example.invalid/x.json".to_string(),
                refresh_minutes: 15,
            },
            assets: Vec::new(),
            nodes: vec![Node::Rect {
                x: 1000,
                y: 0,
                w: 10,
                h: 10,
                radius: 0,
                fill: 0,
                opacity: 255,
            }],
            repeats: Vec::new(),
        };
        let err = compile_scene(&manifest, &snapshot(), &metrics(), 1).unwrap_err();
        assert!(
            matches!(err, CompileError::Invalid(_)),
            "expected CompileError::Invalid, got {err:?}"
        );
    }

    // -- The state footer's error text is bounded (finding 3). --

    #[test]
    fn an_over_long_provider_error_yields_a_valid_scene_with_a_bounded_footer() {
        let over_long = "e".repeat(protocol::MAX_SCENE_TEXT_LEN + 40);
        let scene = compile_scene(
            &manifest_with_value("static"),
            &error_snapshot(&over_long),
            &metrics(),
            1,
        )
        .expect("an over-long provider error must still yield a valid scene");
        let footer = text_node(&scene, 1);
        let SceneValue::Literal(text) = &footer.value else {
            panic!("footer value is not a literal: {:?}", footer.value);
        };
        assert_eq!(text.len(), protocol::MAX_SCENE_TEXT_LEN);
        assert_eq!(*text, "e".repeat(protocol::MAX_SCENE_TEXT_LEN));
        // The whole scene, footer included, still passes the wire's own bounds.
        protocol::validate_scene(&scene).expect("bounded footer must validate");
    }

    // -- Step 4 (Task 8): asset resolution -- `compile_scene_with_assets`
    // actually resolving `image`/`glyph` nodes and asset fonts, which
    // `compile_scene` (against an always-empty `AssetSet`) never exercises. --

    use crate::assets::resolve_assets;
    use crate::manifest::{Asset, Glyph};

    fn manifest_with(assets: Vec<Asset>, nodes: Vec<Node>) -> PluginManifest {
        PluginManifest {
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            source: Source::Json {
                url: "https://example.invalid/x.json".to_string(),
                refresh_minutes: 15,
            },
            assets,
            nodes,
            repeats: Vec::new(),
        }
    }

    fn image_node(asset: &str) -> Node {
        Node::Image {
            x: 10,
            y: 10,
            w: 32,
            h: 32,
            asset: asset.to_string(),
            recolor: false,
            color: 0,
        }
    }

    #[test]
    fn an_image_node_resolves_against_a_real_asset_set() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("badge.bin"), b"pretend RGB565 blob")
            .expect("write fixture");
        let manifest = manifest_with(
            vec![Asset::Image {
                file: "badge.bin".to_string(),
            }],
            vec![image_node("badge.bin")],
        );
        let assets = resolve_assets(&manifest, dir.path()).expect("resolve");

        let scene =
            compile_scene_with_assets(&manifest, &snapshot(), &metrics(), 1, &assets).unwrap();

        let SceneNode::Image(image) = &scene.nodes[0] else {
            panic!("node 0 is not Image: {:?}", scene.nodes[0]);
        };
        assert_eq!(image.digest, assets.get("badge.bin").unwrap().digest);
        assert_eq!(image.w, 32);
        assert_eq!(image.h, 32);
    }

    #[test]
    fn an_image_node_the_manifest_never_declares_is_asset_not_resolved() {
        let manifest = manifest_with(Vec::new(), vec![image_node("missing.bin")]);
        let err =
            compile_scene_with_assets(&manifest, &snapshot(), &metrics(), 1, &AssetSet::default())
                .unwrap_err();
        assert_eq!(
            err,
            CompileError::AssetNotResolved {
                asset: "missing.bin".to_string()
            }
        );
    }

    #[test]
    fn an_image_node_naming_a_font_asset_is_a_kind_mismatch() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("font.ttf"),
            b"not actually a font, bytes suffice",
        )
        .expect("write fixture");
        let manifest = manifest_with(
            vec![Asset::Font {
                file: "font.ttf".to_string(),
            }],
            vec![image_node("font.ttf")],
        );
        let assets = resolve_assets(&manifest, dir.path()).expect("resolve");

        let err =
            compile_scene_with_assets(&manifest, &snapshot(), &metrics(), 1, &assets).unwrap_err();
        assert_eq!(
            err,
            CompileError::AssetKindMismatch {
                asset: "font.ttf".to_string(),
                expected: "image"
            }
        );
    }

    fn glyph_node(asset: &str, glyph: &str) -> Node {
        Node::Glyph {
            x: 10,
            y: 40,
            font: Font::Asset {
                asset: asset.to_string(),
                pixel_size: 32,
            },
            color: 0x00ff_ffff,
            glyph: glyph.to_string(),
        }
    }

    fn icon_font_manifest(nodes: Vec<Node>) -> PluginManifest {
        manifest_with(
            vec![Asset::IconFont {
                file: "icons.ttf".to_string(),
                glyphs: vec![Glyph {
                    name: "warn".to_string(),
                    codepoint: 0x41, // 'A' -- see this test module's asset-resolution
                                     // note on why a plain letter stands in for real
                                     // icon artwork.
                }],
            }],
            nodes,
        )
    }

    #[test]
    fn a_glyph_node_resolves_against_a_real_icon_font_asset() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("icons.ttf"), b"pretend TTF bytes").expect("write fixture");
        let manifest = icon_font_manifest(vec![glyph_node("icons.ttf", r#"{{ icon("warn") }}"#)]);
        let assets = resolve_assets(&manifest, dir.path()).expect("resolve");

        let scene =
            compile_scene_with_assets(&manifest, &snapshot(), &metrics(), 1, &assets).unwrap();

        let SceneNode::Glyph(glyph) = &scene.nodes[0] else {
            panic!("node 0 is not Glyph: {:?}", scene.nodes[0]);
        };
        assert_eq!(glyph.digest, assets.get("icons.ttf").unwrap().digest);
        assert_eq!(glyph.name, "A");
        assert_eq!(glyph.size, 32);
        assert_eq!(glyph.baseline_y, 40);
    }

    // -- Final whole-stage review finding 2: the production path
    // (`compile_scene_with_assets`), not `AssetSet::icon_codepoint` called
    // directly, is what must be exercised here -- that is the entry point
    // every real caller (`curated_plugins.rs`, `server::plugin_provider`)
    // actually uses. --

    #[test]
    fn an_icon_name_no_registered_glyph_defines_compiles_to_an_empty_glyph_not_a_compile_error() {
        // Documents the stated decision: an unresolvable icon name stays
        // non-fatal at draw time, the same way any other missing/mismatched
        // runtime value does elsewhere in this compiler (see
        // `compile_value_source`'s doc on `Missing`) -- `expr::call_icon`'s
        // own contract (`icon_with_an_unknown_name_is_missing`) already
        // decides this at the expression layer, and the production path
        // must not disagree with it now that both draw from the SAME table
        // (`AssetSet::icon_codepoints`).
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("icons.ttf"), b"pretend TTF bytes").expect("write fixture");
        let manifest = icon_font_manifest(vec![glyph_node(
            "icons.ttf",
            r#"{{ icon("does-not-exist") }}"#,
        )]);
        let assets = resolve_assets(&manifest, dir.path()).expect("resolve");

        let scene = compile_scene_with_assets(&manifest, &snapshot(), &metrics(), 1, &assets)
            .expect("an unknown icon name must not fail the scene");

        let SceneNode::Glyph(glyph) = &scene.nodes[0] else {
            panic!("node 0 is not Glyph: {:?}", scene.nodes[0]);
        };
        assert_eq!(
            glyph.name, "",
            "an unresolved icon name must evaluate to Missing (an empty glyph name), not a \
             fallback codepoint"
        );
    }

    #[test]
    fn duplicate_icon_names_across_two_icon_fonts_never_reach_compile_because_resolve_assets_rejects_first()
     {
        // The other half of finding 2: this proves the two entry points
        // cannot disagree about a duplicate, because `resolve_assets` --
        // the one place `AssetSet::icon_codepoints()` is built -- rejects
        // the manifest before `compile_scene_with_assets` can ever run.
        // There is no second, independent duplicate check left in
        // `compile.rs` to potentially disagree with this one.
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.ttf"), b"font a bytes").expect("write a");
        std::fs::write(dir.path().join("b.ttf"), b"font b bytes").expect("write b");
        let manifest = manifest_with(
            vec![
                Asset::IconFont {
                    file: "a.ttf".to_string(),
                    glyphs: vec![Glyph {
                        name: "warn".to_string(),
                        codepoint: 0x41,
                    }],
                },
                Asset::IconFont {
                    file: "b.ttf".to_string(),
                    glyphs: vec![Glyph {
                        name: "warn".to_string(),
                        codepoint: 0x42,
                    }],
                },
            ],
            vec![glyph_node("a.ttf", r#"{{ icon("warn") }}"#)],
        );

        let error = resolve_assets(&manifest, dir.path()).expect_err("must be rejected");

        assert_eq!(
            error,
            crate::assets::AssetError::DuplicateIconName {
                name: "warn".to_string(),
                first_asset: "a.ttf".to_string(),
                second_asset: "b.ttf".to_string(),
            }
        );
    }

    #[test]
    fn a_glyph_node_naming_an_image_asset_is_a_kind_mismatch() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("badge.bin"), b"pretend RGB565 blob")
            .expect("write fixture");
        let manifest = manifest_with(
            vec![Asset::Image {
                file: "badge.bin".to_string(),
            }],
            vec![glyph_node("badge.bin", r#"{{ icon("warn") }}"#)],
        );
        let assets = resolve_assets(&manifest, dir.path()).expect("resolve");

        let err =
            compile_scene_with_assets(&manifest, &snapshot(), &metrics(), 1, &assets).unwrap_err();
        assert_eq!(
            err,
            CompileError::AssetKindMismatch {
                asset: "badge.bin".to_string(),
                expected: "font"
            }
        );
    }

    #[test]
    fn a_glyph_node_with_a_baked_tier_font_is_refused() {
        let manifest = manifest_with(
            Vec::new(),
            vec![Node::Glyph {
                x: 10,
                y: 40,
                font: Font::Tier {
                    tier: FontTier::Body,
                },
                color: 0,
                glyph: "static".to_string(),
            }],
        );
        let err =
            compile_scene_with_assets(&manifest, &snapshot(), &metrics(), 1, &AssetSet::default())
                .unwrap_err();
        assert_eq!(err, CompileError::GlyphFontMustBeAsset);
    }

    #[test]
    fn a_glyph_node_bound_to_a_device_binding_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("icons.ttf"), b"pretend TTF bytes").expect("write fixture");
        let manifest = icon_font_manifest(vec![glyph_node("icons.ttf", "{{ field.icon }}")]);
        let assets = resolve_assets(&manifest, dir.path()).expect("resolve");

        let err =
            compile_scene_with_assets(&manifest, &snapshot(), &metrics(), 1, &assets).unwrap_err();
        assert_eq!(
            err,
            CompileError::GlyphBindingNotSupported {
                text: "field.icon".to_string()
            }
        );
    }

    #[test]
    fn a_text_node_can_use_an_asset_font() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("font.ttf"), b"pretend TTF bytes").expect("write fixture");
        let manifest = manifest_with(
            vec![Asset::Font {
                file: "font.ttf".to_string(),
            }],
            vec![Node::Text {
                x: 24,
                baseline_y: 96,
                w: 400,
                align: ManifestAlign::Left,
                font: ManifestFont::Asset {
                    asset: "font.ttf".to_string(),
                    pixel_size: 24,
                },
                color: 0x00ff_ffff,
                value: "static".to_string(),
                ellipsize: false,
            }],
        );
        let assets = resolve_assets(&manifest, dir.path()).expect("resolve");

        let scene =
            compile_scene_with_assets(&manifest, &snapshot(), &metrics(), 1, &assets).unwrap();

        let SceneFont::Asset { digest, pixel_size } = text_node(&scene, 0).font else {
            panic!(
                "expected an asset font, got {:?}",
                text_node(&scene, 0).font
            );
        };
        assert_eq!(digest, assets.get("font.ttf").unwrap().digest);
        assert_eq!(pixel_size, 24);
    }

    #[test]
    fn compile_scene_without_assets_still_refuses_an_image_node_by_name() {
        // `compile_scene` (not `_with_assets`) is the entry point every
        // existing caller uses; pin that it still fails closed rather than
        // silently accepting an asset-bearing manifest it cannot resolve.
        let manifest = manifest_with(Vec::new(), vec![image_node("badge.bin")]);
        let err = compile_scene(&manifest, &snapshot(), &metrics(), 1).unwrap_err();
        assert_eq!(
            err,
            CompileError::AssetNotResolved {
                asset: "badge.bin".to_string()
            }
        );
    }
}
