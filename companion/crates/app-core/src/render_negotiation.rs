//! The native-scene gate for one freshly built face and one connected device.
//!
//! The gate answers one question: can this exact device draw the scene after
//! installing any frame the image-source host owns? The answer is recomputed
//! for every push; there is no cross-device or cross-revision cache where a
//! confirmed digest could leak.

use std::collections::BTreeSet;

use protocol::{ASSET_DIGEST_LEN, CAPABILITY_SCENE_RENDER, Scene, SceneFont, SceneNode};

pub type AssetDigest = [u8; ASSET_DIGEST_LEN];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderRequirements {
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
        asset_digests: BTreeSet::new(),
    };
    for node in &scene.nodes {
        // A new SceneNode variant must be classified here; bit 8 gates
        // PushScene as a whole today.
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
    if let Some(digest) = requirements.asset_digests.iter().find(|digest| {
        !profile.confirmed_assets.contains(*digest) && !profile.installable_assets.contains(*digest)
    }) {
        return Err(format!(
            "asset sha256:{} is neither on the display nor available from this host",
            protocol::digest_hex(digest)
        ));
    }
    Ok(())
}

fn collect_font(font: &SceneFont, digests: &mut BTreeSet<AssetDigest>) {
    if let SceneFont::Asset { digest, .. } = font {
        digests.insert(*digest);
    }
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
    fn analysis_collects_asset_digests() {
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
            requirements.asset_digests,
            BTreeSet::from([digest(1), digest(2), digest(3)])
        );
    }

    #[test]
    fn a_picture_digest_must_be_confirmed_or_installable() {
        let requirements = RenderRequirements {
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
            asset_digests: BTreeSet::new(),
        };
        let error = validate_native_scene(&requirements, &DeviceRenderProfile::default())
            .expect_err("legacy devices cannot accept a scene");
        assert!(error.contains("declarative scene rendering"));
    }
}
