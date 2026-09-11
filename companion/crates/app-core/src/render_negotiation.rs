//! The native-scene gate for one freshly built face and one connected device.
//!
//! Manifest rasterization used to make this a three-way policy decision. With
//! picture cards already represented as native image nodes, only one question
//! remains: can this exact device draw the scene after installing any frame the
//! image-source host owns? The answer is recomputed for every push; there is no
//! cross-device or cross-revision cache where a confirmed digest could leak.

use std::collections::BTreeSet;

use protocol::{ASSET_DIGEST_LEN, CAPABILITY_SCENE_RENDER, Scene, SceneFont, SceneNode};

pub type AssetDigest = [u8; ASSET_DIGEST_LEN];

/// The nine scene node kinds stay explicit even though bit 8 currently gates
/// all of them. A future node-specific capability must change this match rather
/// than inheriting broad support by accident.
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderRequirements {
    pub node_kinds: BTreeSet<SceneNodeKind>,
    pub asset_digests: BTreeSet<AssetDigest>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceRenderProfile {
    pub capabilities: u64,
    pub confirmed_assets: BTreeSet<AssetDigest>,
    pub installable_assets: BTreeSet<AssetDigest>,
}

#[must_use]
pub fn analyze_scene(scene: &Scene) -> RenderRequirements {
    let mut requirements = RenderRequirements {
        node_kinds: BTreeSet::new(),
        asset_digests: BTreeSet::new(),
    };
    for node in &scene.nodes {
        requirements.node_kinds.insert(SceneNodeKind::of(node));
        match node {
            SceneNode::Image(image) => {
                requirements.asset_digests.insert(image.digest);
            }
            SceneNode::Glyph(glyph) => {
                requirements.asset_digests.insert(glyph.digest);
            }
            SceneNode::Text(text) => collect_font(&text.font, &mut requirements.asset_digests),
            SceneNode::Label(label) => collect_font(&label.font, &mut requirements.asset_digests),
            SceneNode::Rect(_)
            | SceneNode::Arc(_)
            | SceneNode::Line(_)
            | SceneNode::Scale(_)
            | SceneNode::RotRect(_) => {}
        }
    }
    requirements
}

/// Refuses any scene the connected device cannot draw, including a picture
/// whose digest is neither confirmed on that device nor installable from the
/// current image-source host.
pub fn validate_native_scene(
    requirements: &RenderRequirements,
    profile: &DeviceRenderProfile,
) -> Result<(), String> {
    if profile.capabilities & CAPABILITY_SCENE_RENDER == 0 {
        return Err(
            "the display does not advertise declarative scene rendering; update its firmware"
                .into(),
        );
    }
    if let Some(kind) = requirements
        .node_kinds
        .iter()
        .find(|kind| !node_kind_supported(**kind, profile.capabilities))
    {
        return Err(format!("the display cannot draw {} nodes", kind.name()));
    }
    if let Some(digest) = requirements.asset_digests.iter().find(|digest| {
        !profile.confirmed_assets.contains(*digest) && !profile.installable_assets.contains(*digest)
    }) {
        return Err(format!(
            "asset sha256:{} is neither on the display nor available from this host",
            hex(digest)
        ));
    }
    Ok(())
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
        CURRENT_CAPABILITIES, SceneFontTier, SceneGlyph, SceneImage, SceneLabel, SceneRect,
        SceneText, SceneValue,
    };

    use super::*;

    fn digest(fill: u8) -> AssetDigest {
        [fill; ASSET_DIGEST_LEN]
    }

    #[test]
    fn analysis_collects_node_kinds_and_asset_digests() {
        let scene = Scene {
            revision: 7,
            background: 0,
            nodes: vec![
                SceneNode::Rect(SceneRect::default()),
                SceneNode::Image(SceneImage {
                    digest: digest(1),
                    ..SceneImage::default()
                }),
                SceneNode::Glyph(SceneGlyph {
                    digest: digest(2),
                    ..SceneGlyph::default()
                }),
                SceneNode::Text(SceneText {
                    font: SceneFont::Asset {
                        digest: digest(3),
                        pixel_size: 72,
                    },
                    value: SceneValue::Literal("42".into()),
                    ..SceneText::default()
                }),
                SceneNode::Label(SceneLabel {
                    font: SceneFont::Baked(SceneFontTier::Caption),
                    ..SceneLabel::default()
                }),
            ],
        };
        let requirements = analyze_scene(&scene);
        assert_eq!(
            requirements.node_kinds,
            BTreeSet::from([
                SceneNodeKind::Rect,
                SceneNodeKind::Image,
                SceneNodeKind::Glyph,
                SceneNodeKind::Text,
                SceneNodeKind::Label,
            ])
        );
        assert_eq!(
            requirements.asset_digests,
            BTreeSet::from([digest(1), digest(2), digest(3)])
        );
    }

    #[test]
    fn a_picture_digest_must_be_confirmed_or_installable() {
        let requirements = RenderRequirements {
            node_kinds: BTreeSet::from([SceneNodeKind::Image]),
            asset_digests: BTreeSet::from([digest(9)]),
        };
        let mut profile = DeviceRenderProfile {
            capabilities: CURRENT_CAPABILITIES,
            ..DeviceRenderProfile::default()
        };
        assert!(validate_native_scene(&requirements, &profile).is_err());
        profile.installable_assets.insert(digest(9));
        assert_eq!(validate_native_scene(&requirements, &profile), Ok(()));
        profile.installable_assets.clear();
        profile.confirmed_assets.insert(digest(9));
        assert_eq!(validate_native_scene(&requirements, &profile), Ok(()));
    }

    #[test]
    fn a_device_without_scene_support_is_refused() {
        let requirements = RenderRequirements {
            node_kinds: BTreeSet::from([SceneNodeKind::Rect]),
            asset_digests: BTreeSet::new(),
        };
        let error = validate_native_scene(&requirements, &DeviceRenderProfile::default())
            .expect_err("legacy devices cannot accept a scene");
        assert!(error.contains("declarative scene rendering"));
    }
}
