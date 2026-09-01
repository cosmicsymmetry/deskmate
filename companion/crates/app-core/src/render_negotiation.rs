//! Spec §3's render negotiation: one pure decision per (scene, device, revision).
//!
//! Stage 4 replaces `push_active_scene`'s binary shortcut -- "has bit 8, push;
//! lacks it, silently do nothing" -- with the approved three-row table:
//!
//! | Condition | Decision |
//! | --- | --- |
//! | Scenes supported, all node kinds supported, every referenced asset present or installable | [`RenderDecision::Native`] |
//! | The scene uses any **live** binding and that device cannot render it natively | [`RenderDecision::RefuseLive`] |
//! | Otherwise | [`RenderDecision::Rasterize`] |
//!
//! Everything here is pure policy over value types: no device I/O, no cache,
//! no shared state. The plan's cache-key requirement -- (card/scene, device,
//! revision) -- is met by construction, because every call recomputes from the
//! requirements and profile it is handed. A test cannot find a stale answer
//! because there is nowhere for one to live.
//!
//! # Live versus static
//!
//! A binding is **live** when the device re-evaluates it between pushes, so a
//! server-rasterized frame of it would be a photograph of a moving subject:
//! `date`, every `time:*` form, every `timer.*` form, the positional
//! rotation/angle bindings on hands, and a timer-driven `running_color` style
//! selector. `field.*` is **static** -- it changes only when the host supplies
//! a new value, so the server may resolve it while rasterizing. Rasterizing a
//! clock is never an approximation this product accepts: at a 30-second
//! cadence `time:HH:mm` is wrong for up to half of every minute. A card that
//! would freeze says so in its editor instead.
//!
//! An unrecognized binding is an analysis **error**, never "probably static":
//! guessing static is exactly the path by which a future live namespace would
//! silently freeze.

use std::collections::BTreeSet;

use protocol::{
    ASSET_DIGEST_LEN, CAPABILITY_SCENE_RENDER, Scene, SceneFont, SceneNode, SceneValue,
    binding_is_valid,
};

/// A content address, as both sides of the asset wire use it.
pub type AssetDigest = [u8; ASSET_DIGEST_LEN];

/// The pseudo-binding recorded when a node carries a timer-driven
/// `running_color`. It is a style selector, not a value binding -- there is no
/// wire string to collect -- but it is every bit as live: the device flips the
/// colour when the timer starts or stops, between pushes. The token can never
/// collide with a real binding because `binding_is_valid` rejects it, and
/// analysis only ever *inserts* it; nothing parses it back.
pub const RUNNING_COLOR_REQUIREMENT: &str = "running_color";

/// Which renderer a candidate was authored for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeSource {
    /// A protocol display-list scene the device may draw natively.
    DisplayList,
    /// An SVG template. No device node kind can draw SVG, so its
    /// native-support predicate is false by construction; only rasterization
    /// can produce pixels for it.
    RasterOnly,
}

/// The nine scene node kinds, mirrored from [`protocol::SceneNode`] so
/// support stays an explicit per-kind question. Bit 8 currently covers all
/// nine -- this stage adds no node kind and there is no new node bit -- but
/// the table would be the place a finer-grained capability lands, and keeping
/// it forces that future change to be a visible policy edit rather than an
/// implicit "bit 8 means everything forever".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SceneNodeKind {
    Rect,
    Arc,
    Line,
    Text,
    Image,
    Glyph,
    Scale,
    Label,
    RotRect,
}

impl SceneNodeKind {
    fn of(node: &SceneNode) -> Self {
        match node {
            SceneNode::Rect(_) => Self::Rect,
            SceneNode::Arc(_) => Self::Arc,
            SceneNode::Line(_) => Self::Line,
            SceneNode::Text(_) => Self::Text,
            SceneNode::Image(_) => Self::Image,
            SceneNode::Glyph(_) => Self::Glyph,
            SceneNode::Scale(_) => Self::Scale,
            SceneNode::Label(_) => Self::Label,
            SceneNode::RotRect(_) => Self::RotRect,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Rect => "rect",
            Self::Arc => "arc",
            Self::Line => "line",
            Self::Text => "text",
            Self::Image => "image",
            Self::Glyph => "glyph",
            Self::Scale => "scale",
            Self::Label => "label",
            Self::RotRect => "rotated rect",
        }
    }
}

/// True when `capabilities` lets the device draw `kind` natively.
///
/// Deliberately a per-kind match rather than one bit test, for the reason on
/// [`SceneNodeKind`]: a future node kind gated by its own bit changes this
/// table, visibly, instead of inheriting bit 8 by accident.
#[must_use]
pub fn node_kind_supported(kind: SceneNodeKind, capabilities: u64) -> bool {
    let scene_render = capabilities & CAPABILITY_SCENE_RENDER != 0;
    match kind {
        SceneNodeKind::Rect
        | SceneNodeKind::Arc
        | SceneNodeKind::Line
        | SceneNodeKind::Text
        | SceneNodeKind::Image
        | SceneNodeKind::Glyph
        | SceneNodeKind::Scale
        | SceneNodeKind::Label
        | SceneNodeKind::RotRect => scene_render,
    }
}

/// Whether a frozen raster of a binding would be correct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingClass {
    /// The device recomputes it between pushes; rasterizing it freezes it.
    Live,
    /// It changes only when the host supplies a new value, so the server may
    /// resolve it while rasterizing.
    Static,
}

/// The bindings a candidate requires, split by [`BindingClass`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BindingRequirements {
    pub live: BTreeSet<String>,
    /// `field.*` and nothing else today. Kept as the full token so a refusal
    /// or a rasterizer can name exactly what it resolved.
    pub resolvable: BTreeSet<String>,
}

/// Everything negotiation needs to know about one render candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderRequirements {
    pub native_source: NativeSource,
    pub node_kinds: BTreeSet<SceneNodeKind>,
    pub asset_digests: BTreeSet<AssetDigest>,
    pub bindings: BindingRequirements,
}

/// Everything negotiation needs to know about one device, at one connection.
///
/// `capabilities` is the raw advertised bit set
/// ([`crate::DeviceSnapshot::capability_bits`]), not the parsed enum list, so
/// unknown future bits survive into the decision. `confirmed_assets` is what
/// this device has acknowledged holding; `installable_assets` is what the
/// host holds bytes for and can transfer before the scene. The two sets are
/// per device on purpose -- one device's confirmed digest says nothing about
/// another's flash.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceRenderProfile {
    pub capabilities: u64,
    pub confirmed_assets: BTreeSet<AssetDigest>,
    pub installable_assets: BTreeSet<AssetDigest>,
}

/// The three rows of spec §3's table. There is no fourth row: the legacy
/// widget renderer is not an implicit fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderDecision {
    /// Push the display-list scene as-is.
    Native,
    /// The card would freeze if rasterized; refuse visibly instead. The
    /// reason names the live requirements and the missing device support, and
    /// what action fixes it -- it becomes that card's
    /// [`crate::state::CardErrorKind::SceneRefused`] message verbatim.
    RefuseLive { reason: String },
    /// Render server-side into one volatile image and push a one-node scene.
    Rasterize,
}

/// A candidate whose requirements cannot even be stated. Distinct from
/// [`RenderDecision::RefuseLive`]: a refusal is a valid scene meeting a
/// limited device, an analysis error is a scene naming a binding outside the
/// closed vocabulary -- a bug or a hostile input, not a compatibility fact.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RequirementsError {
    #[error(
        "scene binding {binding:?} is not in the closed protocol vocabulary; refusing to guess whether a rasterized copy of it would stay correct"
    )]
    UnknownBinding { binding: String },
}

/// Classifies one **value-position** binding token (`SceneValue::Binding`,
/// `SceneArc::end_binding`). Positional rotation/angle tokens have narrower
/// vocabularies and are classified inside [`analyze_scene`], because
/// `binding_is_valid` -- correctly -- rejects them here.
pub fn classify_binding(text: &str) -> Result<BindingClass, RequirementsError> {
    if !binding_is_valid(text) {
        // Includes near-misses like `timer.velocity`: inside a live-looking
        // namespace but not a member of the closed set. Refusing is the whole
        // point -- see the module doc.
        return Err(RequirementsError::UnknownBinding {
            binding: text.to_owned(),
        });
    }
    if text.starts_with("field.") {
        return Ok(BindingClass::Static);
    }
    // Everything else `binding_is_valid` accepts is device-recomputed:
    // `date`, `time:*`, `timer.*`.
    Ok(BindingClass::Live)
}

/// Walks the actual protocol [`Scene`] and states what rendering it requires:
/// every node kind, every referenced asset digest (image, glyph, asset font),
/// and every binding-bearing field, classified.
pub fn analyze_scene(scene: &Scene) -> Result<RenderRequirements, RequirementsError> {
    let mut requirements = RenderRequirements {
        native_source: NativeSource::DisplayList,
        node_kinds: BTreeSet::new(),
        asset_digests: BTreeSet::new(),
        bindings: BindingRequirements::default(),
    };

    for node in &scene.nodes {
        requirements.node_kinds.insert(SceneNodeKind::of(node));
        match node {
            SceneNode::Rect(_) | SceneNode::Scale(_) => {}
            SceneNode::Arc(arc) => {
                if !arc.end_binding.is_empty() {
                    classify_into(&arc.end_binding, &mut requirements.bindings)?;
                }
                if arc.running_color.is_some() {
                    requirements
                        .bindings
                        .live
                        .insert(RUNNING_COLOR_REQUIREMENT.to_owned());
                }
            }
            SceneNode::Line(line) => {
                // The bound-hand form. Its two legal tokens are clock angles,
                // both live; anything else is outside the closed set.
                if !line.angle_binding.is_empty() {
                    classify_positional(
                        &line.angle_binding,
                        &["time:angle:hour", "time:angle:minute"],
                        &mut requirements.bindings,
                    )?;
                }
            }
            SceneNode::Text(text) => {
                classify_value(&text.value, &mut requirements.bindings)?;
                collect_font(&text.font, &mut requirements.asset_digests);
                if text.running_color.is_some() {
                    requirements
                        .bindings
                        .live
                        .insert(RUNNING_COLOR_REQUIREMENT.to_owned());
                }
            }
            SceneNode::Image(image) => {
                requirements.asset_digests.insert(image.digest);
            }
            SceneNode::Glyph(glyph) => {
                requirements.asset_digests.insert(glyph.digest);
            }
            SceneNode::Label(label) => {
                classify_value(&label.value, &mut requirements.bindings)?;
                collect_font(&label.font, &mut requirements.asset_digests);
            }
            SceneNode::RotRect(rect) => {
                if !rect.rotation_binding.is_empty() {
                    classify_positional(
                        &rect.rotation_binding,
                        &["time:hour", "time:minute", "time:second"],
                        &mut requirements.bindings,
                    )?;
                }
            }
        }
    }

    Ok(requirements)
}

/// States the requirements of a raster-only (SVG) candidate from the
/// device-binding tokens its template names. The node-kind and asset sets are
/// empty on purpose: nothing about the *device* is required to draw an image
/// it will never draw natively -- the volatile frame's own transfer is Task
/// 4/5's concern, not a per-scene digest reference.
pub fn analyze_raster_only(
    bindings: impl IntoIterator<Item = String>,
) -> Result<RenderRequirements, RequirementsError> {
    let mut requirements = RenderRequirements {
        native_source: NativeSource::RasterOnly,
        node_kinds: BTreeSet::new(),
        asset_digests: BTreeSet::new(),
        bindings: BindingRequirements::default(),
    };
    for binding in bindings {
        classify_into(&binding, &mut requirements.bindings)?;
    }
    Ok(requirements)
}

fn classify_value(
    value: &SceneValue,
    bindings: &mut BindingRequirements,
) -> Result<(), RequirementsError> {
    match value {
        SceneValue::Literal(_) => Ok(()),
        SceneValue::Binding(binding) => classify_into(binding, bindings),
    }
}

fn classify_into(
    binding: &str,
    bindings: &mut BindingRequirements,
) -> Result<(), RequirementsError> {
    match classify_binding(binding)? {
        BindingClass::Live => bindings.live.insert(binding.to_owned()),
        BindingClass::Static => bindings.resolvable.insert(binding.to_owned()),
    };
    Ok(())
}

/// Positional bindings (hand rotation, hand angle) have their own two- or
/// three-token vocabularies that `binding_is_valid` rejects. Every member is
/// a clock fact, so every member is live; anything else is the same
/// closed-set error as everywhere else. `validate_scene` normally refuses
/// such a scene before analysis ever sees it -- this is defence in depth, not
/// the primary gate.
fn classify_positional(
    binding: &str,
    allowed: &[&str],
    bindings: &mut BindingRequirements,
) -> Result<(), RequirementsError> {
    if !allowed.contains(&binding) {
        return Err(RequirementsError::UnknownBinding {
            binding: binding.to_owned(),
        });
    }
    bindings.live.insert(binding.to_owned());
    Ok(())
}

/// The decision. Pure: same inputs, same answer, nothing cached.
#[must_use]
pub fn negotiate(
    requirements: &RenderRequirements,
    profile: &DeviceRenderProfile,
) -> RenderDecision {
    // Row 1: everything the candidate needs, this device natively has.
    let mut missing: Vec<String> = Vec::new();
    match requirements.native_source {
        NativeSource::RasterOnly => {
            missing.push("the card is an SVG template, which no device draws natively".to_owned());
        }
        NativeSource::DisplayList => {
            if profile.capabilities & CAPABILITY_SCENE_RENDER == 0 {
                missing
                    .push("the display does not advertise declarative scene rendering".to_owned());
            } else {
                for kind in &requirements.node_kinds {
                    if !node_kind_supported(*kind, profile.capabilities) {
                        missing.push(format!("the display cannot draw {} nodes", kind.name()));
                    }
                }
            }
            for digest in &requirements.asset_digests {
                let present = profile.confirmed_assets.contains(digest)
                    || profile.installable_assets.contains(digest);
                if !present {
                    missing.push(format!(
                        "asset sha256:{} is neither on the display nor available from this host",
                        hex(digest)
                    ));
                }
            }
        }
    }
    if missing.is_empty() {
        return RenderDecision::Native;
    }

    // Row 2: a live requirement that cannot be met natively is a card that
    // would freeze. Say so, visibly, rather than shipping a wrong clock.
    if !requirements.bindings.live.is_empty() {
        let live: Vec<&str> = requirements
            .bindings
            .live
            .iter()
            .map(String::as_str)
            .collect();
        return RenderDecision::RefuseLive {
            reason: format!(
                "this card needs the display to keep {} moving between pushes, but {}; a \
                 server-rendered image of it would freeze, so it is refused instead -- update \
                 the display's firmware (or supply the missing asset) to show this card",
                live.join(", "),
                missing.join("; ")
            ),
        };
    }

    // Row 3: nothing live, so a server-rendered frame is exactly as correct
    // as a native one.
    RenderDecision::Rasterize
}

fn collect_font(font: &SceneFont, digests: &mut BTreeSet<AssetDigest>) {
    if let SceneFont::Asset { digest, .. } = font {
        digests.insert(*digest);
    }
}

fn hex(digest: &AssetDigest) -> String {
    digest
        .iter()
        .fold(String::with_capacity(digest.len() * 2), |mut out, byte| {
            use std::fmt::Write;
            let _ = write!(out, "{byte:02x}");
            out
        })
}

#[cfg(test)]
mod tests {
    use protocol::{
        CURRENT_CAPABILITIES, SceneArc, SceneFontTier, SceneGlyph, SceneImage, SceneLabel,
        SceneLine, SceneRect, SceneRotRect, SceneScale, SceneText,
    };

    use super::*;

    fn digest(fill: u8) -> AssetDigest {
        [fill; ASSET_DIGEST_LEN]
    }

    fn scene(nodes: Vec<SceneNode>) -> Scene {
        Scene {
            revision: 7,
            background: 0,
            nodes,
        }
    }

    fn text_binding(binding: &str) -> SceneNode {
        SceneNode::Text(SceneText {
            value: SceneValue::Binding(binding.to_owned()),
            ..SceneText::default()
        })
    }

    fn current_profile() -> DeviceRenderProfile {
        DeviceRenderProfile {
            capabilities: CURRENT_CAPABILITIES,
            confirmed_assets: BTreeSet::new(),
            installable_assets: BTreeSet::new(),
        }
    }

    fn legacy_profile() -> DeviceRenderProfile {
        DeviceRenderProfile {
            capabilities: CURRENT_CAPABILITIES & !CAPABILITY_SCENE_RENDER,
            confirmed_assets: BTreeSet::new(),
            installable_assets: BTreeSet::new(),
        }
    }

    // -- Step 1: the decision table, verbatim from spec §3 -------------------

    #[test]
    fn a_supported_scene_with_no_assets_is_native() {
        let requirements = analyze_scene(&scene(vec![
            SceneNode::Rect(SceneRect::default()),
            text_binding("time:HH:mm"),
        ]))
        .unwrap();

        assert_eq!(
            negotiate(&requirements, &current_profile()),
            RenderDecision::Native
        );
    }

    #[test]
    fn an_unsupported_node_kind_blocks_native() {
        // No profile can lack support for one of the nine kinds while keeping
        // bit 8 today, so exercise the per-kind predicate directly: it is the
        // seam a finer-grained capability would move through.
        for kind in [
            SceneNodeKind::Rect,
            SceneNodeKind::Arc,
            SceneNodeKind::Line,
            SceneNodeKind::Text,
            SceneNodeKind::Image,
            SceneNodeKind::Glyph,
            SceneNodeKind::Scale,
            SceneNodeKind::Label,
            SceneNodeKind::RotRect,
        ] {
            assert!(node_kind_supported(kind, CURRENT_CAPABILITIES));
            assert!(!node_kind_supported(
                kind,
                CURRENT_CAPABILITIES & !CAPABILITY_SCENE_RENDER
            ));
        }
    }

    #[test]
    fn a_missing_but_installable_asset_is_still_native() {
        let requirements = analyze_scene(&scene(vec![SceneNode::Image(SceneImage {
            digest: digest(0xAA),
            ..SceneImage::default()
        })]))
        .unwrap();
        let mut profile = current_profile();
        profile.installable_assets.insert(digest(0xAA));

        assert_eq!(negotiate(&requirements, &profile), RenderDecision::Native);
    }

    #[test]
    fn a_missing_uninstallable_asset_rasterizes_a_static_scene() {
        let requirements = analyze_scene(&scene(vec![SceneNode::Image(SceneImage {
            digest: digest(0xAA),
            ..SceneImage::default()
        })]))
        .unwrap();

        // Neither confirmed nor installable: not native. Nothing live: row 3.
        assert_eq!(
            negotiate(&requirements, &current_profile()),
            RenderDecision::Rasterize
        );
    }

    #[test]
    fn a_missing_uninstallable_asset_refuses_a_live_scene() {
        let requirements = analyze_scene(&scene(vec![
            SceneNode::Image(SceneImage {
                digest: digest(0xAA),
                ..SceneImage::default()
            }),
            text_binding("time:HH:mm"),
        ]))
        .unwrap();

        let RenderDecision::RefuseLive { reason } = negotiate(&requirements, &current_profile())
        else {
            panic!("a live scene with a missing asset must refuse, not rasterize");
        };
        assert!(
            reason.contains("time:HH:mm"),
            "names the live binding: {reason}"
        );
        assert!(
            reason.contains(&hex(&digest(0xAA))),
            "names the asset: {reason}"
        );
    }

    #[test]
    fn a_static_svg_rasterizes() {
        let requirements = analyze_raster_only(vec!["field.title".to_owned()]).unwrap();

        // Even on a fully current device: there is no native SVG node.
        assert_eq!(
            negotiate(&requirements, &current_profile()),
            RenderDecision::Rasterize
        );
    }

    #[test]
    fn a_live_svg_is_refused_on_every_device() {
        let requirements = analyze_raster_only(vec!["time:HH:mm".to_owned()]).unwrap();

        for profile in [current_profile(), legacy_profile()] {
            let RenderDecision::RefuseLive { reason } = negotiate(&requirements, &profile) else {
                panic!("a live SVG must refuse");
            };
            assert!(reason.contains("time:HH:mm"));
            assert!(reason.contains("SVG"));
        }
    }

    #[test]
    fn a_device_without_scene_support_refuses_live_and_rasterizes_static() {
        let live = analyze_scene(&scene(vec![text_binding("time:HH:mm")])).unwrap();
        let RenderDecision::RefuseLive { reason } = negotiate(&live, &legacy_profile()) else {
            panic!("live scene on a legacy device must refuse");
        };
        assert!(reason.contains("declarative scene rendering"));
        assert!(reason.contains("firmware"), "says what fixes it: {reason}");

        let static_scene = analyze_scene(&scene(vec![
            SceneNode::Rect(SceneRect::default()),
            text_binding("field.title"),
        ]))
        .unwrap();
        assert_eq!(
            negotiate(&static_scene, &legacy_profile()),
            RenderDecision::Rasterize
        );
    }

    // -- Step 2: binding classification --------------------------------------

    #[test]
    fn live_bindings_classify_live() {
        for binding in [
            "date",
            "time:HH:mm",
            "time:HH:mm:SS",
            "timer.pct",
            "timer.permille",
            "timer.status",
            "timer.remaining:MM:SS",
            "timer.elapsed:MM:SS",
            "timer.total:MM:SS",
        ] {
            assert_eq!(
                classify_binding(binding),
                Ok(BindingClass::Live),
                "{binding} must be live"
            );
        }
    }

    #[test]
    fn field_bindings_and_literals_are_static() {
        assert_eq!(classify_binding("field.title"), Ok(BindingClass::Static));
        assert_eq!(classify_binding("field.status"), Ok(BindingClass::Static));

        // Literals never reach classification at all: analysis skips them.
        let requirements = analyze_scene(&scene(vec![SceneNode::Text(SceneText {
            value: SceneValue::Literal("AQI".to_owned()),
            ..SceneText::default()
        })]))
        .unwrap();
        assert!(requirements.bindings.live.is_empty());
        assert!(requirements.bindings.resolvable.is_empty());
    }

    #[test]
    fn positional_hand_bindings_are_live() {
        let requirements = analyze_scene(&scene(vec![
            SceneNode::RotRect(SceneRotRect {
                rotation_binding: "time:minute".to_owned(),
                ..SceneRotRect::default()
            }),
            SceneNode::Line(SceneLine {
                angle_binding: "time:angle:hour".to_owned(),
                pivot_x: 224,
                pivot_y: 184,
                length: 80,
                ..SceneLine::default()
            }),
        ]))
        .unwrap();

        assert!(requirements.bindings.live.contains("time:minute"));
        assert!(requirements.bindings.live.contains("time:angle:hour"));
    }

    #[test]
    fn a_bound_arc_end_is_live() {
        let requirements = analyze_scene(&scene(vec![SceneNode::Arc(SceneArc {
            end_binding: "timer.permille".to_owned(),
            ..SceneArc::default()
        })]))
        .unwrap();

        assert!(requirements.bindings.live.contains("timer.permille"));
    }

    #[test]
    fn running_color_is_a_live_requirement_without_a_wire_binding() {
        // The subtle row: a style selector flipped by timer state. The value
        // binding may be static or absent entirely -- the node is live anyway.
        for node in [
            SceneNode::Arc(SceneArc {
                running_color: Some(0x00FF_0000),
                ..SceneArc::default()
            }),
            SceneNode::Text(SceneText {
                running_color: Some(0x00FF_0000),
                value: SceneValue::Literal("25:00".to_owned()),
                ..SceneText::default()
            }),
        ] {
            let requirements = analyze_scene(&scene(vec![node])).unwrap();
            assert!(
                requirements
                    .bindings
                    .live
                    .contains(RUNNING_COLOR_REQUIREMENT),
                "a timer-driven running_color must register live"
            );
        }
    }

    #[test]
    fn an_unknown_namespace_is_an_error_not_probably_static() {
        for binding in ["weather.temp", "timer.velocity", "now", "field.", "time:XY"] {
            let error = classify_binding(binding).unwrap_err();
            assert_eq!(
                error,
                RequirementsError::UnknownBinding {
                    binding: binding.to_owned()
                }
            );
        }

        // And through full analysis, in every binding position.
        let in_text = analyze_scene(&scene(vec![text_binding("timer.velocity")]));
        assert!(in_text.is_err());
        let in_rotation = analyze_scene(&scene(vec![SceneNode::RotRect(SceneRotRect {
            rotation_binding: "time:sidereal".to_owned(),
            ..SceneRotRect::default()
        })]));
        assert!(in_rotation.is_err());
        let in_svg = analyze_raster_only(vec!["timer.velocity".to_owned()]);
        assert!(in_svg.is_err());
    }

    // -- Step 3: no shared answers across devices or revisions ----------------

    #[test]
    fn the_same_scene_decides_differently_per_device_profile() {
        let requirements = analyze_scene(&scene(vec![text_binding("time:HH:mm")])).unwrap();

        assert_eq!(
            negotiate(&requirements, &current_profile()),
            RenderDecision::Native
        );
        assert!(matches!(
            negotiate(&requirements, &legacy_profile()),
            RenderDecision::RefuseLive { .. }
        ));
    }

    #[test]
    fn changed_capability_bits_change_the_answer_for_the_same_inputs() {
        // A reconnect that drops bit 8 must recompute -- nothing is cached, so
        // the only way this fails is a future cache keyed too coarsely.
        let requirements = analyze_scene(&scene(vec![text_binding("field.title")])).unwrap();
        let before = negotiate(&requirements, &current_profile());
        let after = negotiate(&requirements, &legacy_profile());

        assert_eq!(before, RenderDecision::Native);
        assert_eq!(after, RenderDecision::Rasterize);
    }

    #[test]
    fn one_devices_confirmed_digest_never_makes_anothers_present() {
        let requirements = analyze_scene(&scene(vec![SceneNode::Glyph(SceneGlyph {
            digest: digest(0x5C),
            ..SceneGlyph::default()
        })]))
        .unwrap();

        let mut device_a = current_profile();
        device_a.confirmed_assets.insert(digest(0x5C));
        let device_b = current_profile();

        assert_eq!(negotiate(&requirements, &device_a), RenderDecision::Native);
        assert_eq!(
            negotiate(&requirements, &device_b),
            RenderDecision::Rasterize
        );
    }

    #[test]
    fn a_changed_binding_set_changes_the_decision_at_the_same_device() {
        // A new revision that adds a live binding to a previously static card
        // must be renegotiated: on a legacy device the answer flips from
        // Rasterize to RefuseLive.
        let old = analyze_scene(&scene(vec![text_binding("field.title")])).unwrap();
        let new = analyze_scene(&scene(vec![
            text_binding("field.title"),
            text_binding("time:HH:mm"),
        ]))
        .unwrap();

        assert_eq!(
            negotiate(&old, &legacy_profile()),
            RenderDecision::Rasterize
        );
        assert!(matches!(
            negotiate(&new, &legacy_profile()),
            RenderDecision::RefuseLive { .. }
        ));
    }

    // -- Step 5: requirement analysis walks the real scene --------------------

    #[test]
    fn analysis_collects_every_node_kind_and_asset_digest() {
        let requirements = analyze_scene(&scene(vec![
            SceneNode::Rect(SceneRect::default()),
            SceneNode::Scale(SceneScale::default()),
            SceneNode::Image(SceneImage {
                digest: digest(0x01),
                ..SceneImage::default()
            }),
            SceneNode::Glyph(SceneGlyph {
                digest: digest(0x02),
                ..SceneGlyph::default()
            }),
            SceneNode::Text(SceneText {
                font: SceneFont::Asset {
                    digest: digest(0x03),
                    pixel_size: 72,
                },
                value: SceneValue::Literal("42".to_owned()),
                ..SceneText::default()
            }),
            SceneNode::Label(SceneLabel {
                font: SceneFont::Baked(SceneFontTier::Caption),
                value: SceneValue::Literal("AQI".to_owned()),
                ..SceneLabel::default()
            }),
        ]))
        .unwrap();

        assert_eq!(
            requirements.node_kinds,
            BTreeSet::from([
                SceneNodeKind::Rect,
                SceneNodeKind::Scale,
                SceneNodeKind::Image,
                SceneNodeKind::Glyph,
                SceneNodeKind::Text,
                SceneNodeKind::Label,
            ])
        );
        // A baked-tier font contributes no digest; the three assets do.
        assert_eq!(
            requirements.asset_digests,
            BTreeSet::from([digest(0x01), digest(0x02), digest(0x03)])
        );
        assert_eq!(requirements.native_source, NativeSource::DisplayList);
    }
}
