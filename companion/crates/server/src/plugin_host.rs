//! Server-side implementation of app-core's plugin rendering boundary.

use std::sync::Arc;

use app_core::{
    BakedFontMetrics, DesiredAsset, PluginHost, RasterFrame, RasterRequest, SceneCandidate,
};
use providers::ProviderSnapshot;

use crate::plugin_provider::{
    FailureClass, PluginFailure, classify_plugin_failure, within_render_wall_clock_budget,
};
use crate::plugin_registry::PluginRegistry;

/// Supplies loaded plugin assets and compiles plugin scenes for one runtime.
pub struct ServerPluginHost {
    registry: Arc<PluginRegistry>,
}

impl ServerPluginHost {
    #[must_use]
    pub fn new(registry: Arc<PluginRegistry>) -> Self {
        Self { registry }
    }
}

impl PluginHost for ServerPluginHost {
    fn desired_assets(&mut self) -> Vec<DesiredAsset> {
        self.registry
            .all_assets()
            .into_iter()
            .map(|asset| DesiredAsset {
                digest: asset.digest,
                kind: asset.kind,
                // Keep the exact allocation that resolve_assets hashed. Re-reading or
                // rebuilding this buffer would break its single-read provenance.
                bytes: Arc::clone(&asset.bytes),
            })
            .collect()
    }

    fn render_scene(
        &mut self,
        plugin_id: &str,
        snapshot: &ProviderSnapshot<serde_json::Value>,
        revision: u32,
    ) -> Result<SceneCandidate, String> {
        let loaded = self
            .registry
            .get(plugin_id)
            .ok_or_else(|| format!("unknown plugin id {plugin_id:?}"))?;

        // An SVG template is a raster-only candidate: the host states which
        // device-binding tokens the template names and nothing more.
        // Negotiation classifies them and decides -- this boundary never
        // applies spec §3's table itself, or there would be two copies of it
        // free to disagree.
        if let Some(svg) = &loaded.svg_source {
            return Ok(SceneCandidate::RasterOnly {
                bindings: plugin::device_binding_requirements(svg),
            });
        }

        match within_render_wall_clock_budget(|| {
            plugin::compile_scene_with_assets(
                &loaded.manifest,
                snapshot,
                &BakedFontMetrics::SHIPPED,
                revision,
                &loaded.assets,
            )
        }) {
            Ok(Ok(scene)) => Ok(SceneCandidate::DisplayList(scene)),
            Ok(Err(error)) => Err(classified_failure_message(
                plugin_id,
                &PluginFailure::Compile(&error),
                &error,
            )),
            Err(error) => Err(classified_failure_message(
                plugin_id,
                &PluginFailure::RenderTimedOut,
                &error,
            )),
        }
    }

    fn rasterize(&mut self, request: &RasterRequest) -> Result<RasterFrame, String> {
        let frame = match request {
            RasterRequest::DisplayList { scene, fields } => {
                let assets = self
                    .registry
                    .all_assets()
                    .into_iter()
                    .map(|asset| (asset.digest, Arc::clone(&asset.bytes)))
                    .collect();
                crate::rasterizer::rasterize_scene(scene, fields, &assets)
            }
            RasterRequest::PluginSvg {
                plugin_id,
                snapshot,
                fields,
            } => {
                let loaded = self
                    .registry
                    .get(plugin_id)
                    .ok_or_else(|| format!("unknown plugin id {plugin_id:?}"))?;
                let template = loaded.svg_source.as_deref().ok_or_else(|| {
                    format!("plugin {plugin_id:?} does not own an SVG raster template")
                })?;
                let font_assets = loaded
                    .assets
                    .iter()
                    .filter(|(_, asset)| {
                        matches!(
                            asset.kind,
                            protocol::AssetKind::Font | protocol::AssetKind::IconFont
                        )
                    })
                    .map(|(_, asset)| (asset.digest, Arc::clone(&asset.bytes)))
                    .collect();
                crate::rasterizer::rasterize_plugin_svg(
                    template,
                    &snapshot.value,
                    fields,
                    &font_assets,
                    app_core::SceneDataState {
                        stale: snapshot.stale,
                        error: snapshot.error.as_deref(),
                    },
                )
            }
        }
        .map_err(|error| format!("server rasterization failed: {error}"))?;

        Ok(RasterFrame {
            digest: frame.digest,
            bytes: Arc::from(frame.bytes),
        })
    }
}

fn classified_failure_message(
    plugin_id: &str,
    failure: &PluginFailure<'_>,
    detail: &dyn std::fmt::Display,
) -> String {
    let class = match classify_plugin_failure(failure) {
        FailureClass::Transient => "transient",
        FailureClass::Permanent => "permanent",
    };
    format!("{class} plugin render failure for {plugin_id:?}: {detail}")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use chrono::Utc;
    use protocol::{Message, PushScene};
    use sha2::Digest as _;

    use super::*;

    const AQI_FIXTURE: &str = include_str!("../../plugin/tests/fixtures/aqi_response.json");

    fn curated_plugins_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins")
    }

    fn curated_registry() -> Arc<PluginRegistry> {
        let (registry, failures) =
            PluginRegistry::load(&curated_plugins_dir()).expect("load curated plugins");
        assert!(failures.is_empty(), "unexpected failures: {failures:?}");
        Arc::new(registry)
    }

    fn aqi_snapshot() -> ProviderSnapshot<serde_json::Value> {
        let envelope: serde_json::Value =
            serde_json::from_str(AQI_FIXTURE).expect("parse AQI fixture");
        ProviderSnapshot {
            // The manifest is authored against the fixture's payload object. The
            // production provider's raw-envelope divergence is pinned separately in
            // plugin_refresher's tests and reported at handoff.
            value: envelope["payload"].clone(),
            refreshed_at: Some(Utc::now()),
            age: Some(std::time::Duration::ZERO),
            stale: false,
            error: None,
        }
    }

    fn raw_aqi_envelope_snapshot() -> ProviderSnapshot<serde_json::Value> {
        ProviderSnapshot {
            value: serde_json::from_str(AQI_FIXTURE).expect("parse AQI fixture"),
            refreshed_at: Some(Utc::now()),
            age: Some(std::time::Duration::ZERO),
            stale: false,
            error: None,
        }
    }

    #[test]
    fn real_aqi_plugin_compiles_to_a_protocol_valid_scene() {
        let mut host = ServerPluginHost::new(curated_registry());
        let candidate = host
            .render_scene("aqi", &aqi_snapshot(), 41)
            .expect("compile AQI scene");
        let SceneCandidate::DisplayList(scene) = candidate else {
            panic!("a display-list plugin must produce a native candidate");
        };

        protocol::validate_message(&Message::PushScene(PushScene {
            card_id: "air-quality".into(),
            revision: 41,
            scene,
        }))
        .expect("compiled scene must satisfy the real wire validator");
    }

    #[test]
    fn production_raw_aqi_envelope_renders_missing_data_instead_of_its_payload() {
        let mut host = ServerPluginHost::new(curated_registry());

        let candidate = host
            .render_scene("aqi", &raw_aqi_envelope_snapshot(), 42)
            .expect("missing expression paths currently degrade to empty text");
        let SceneCandidate::DisplayList(scene) = candidate else {
            panic!("a display-list plugin must produce a native candidate");
        };

        let protocol::SceneNode::Text(category) = &scene.nodes[0] else {
            panic!("AQI category node changed kind");
        };
        let protocol::SceneNode::Text(hero) = &scene.nodes[2] else {
            panic!("AQI hero node changed kind");
        };
        assert_eq!(category.value, protocol::SceneValue::Literal(String::new()));
        assert_eq!(hero.value, protocol::SceneValue::Literal(String::new()));
    }

    #[test]
    fn an_svg_plugin_is_a_raster_only_candidate_not_a_compile_error() {
        let mut host = ServerPluginHost::new(curated_registry());

        let candidate = host
            .render_scene("svg-aqi", &aqi_snapshot(), 43)
            .expect("an SVG template must produce a candidate, not an error");

        // The curated svg-aqi face binds only provider data (`data.*`), so it
        // carries no device-binding requirement: negotiation will rasterize
        // it rather than refuse it.
        assert_eq!(
            candidate,
            SceneCandidate::RasterOnly {
                bindings: std::collections::BTreeSet::new()
            }
        );
    }

    #[test]
    fn plugin_host_boundary_rasterizes_svg_to_an_app_core_owned_canonical_frame() {
        let mut host = ServerPluginHost::new(curated_registry());

        let frame = host
            .rasterize(&RasterRequest::PluginSvg {
                plugin_id: "svg-aqi".into(),
                snapshot: aqi_snapshot(),
                fields: vec![],
            })
            .expect("rasterize curated SVG through the app-core boundary");

        assert_eq!(
            frame.bytes.len(),
            protocol::VOLATILE_IMAGE_DECODED_LENGTH as usize
        );
        assert_eq!(sha2::Sha256::digest(&frame.bytes).as_slice(), frame.digest);
    }

    #[test]
    fn svg_data_state_goldens_run_through_server_plugin_host_rasterize() {
        let mut host = ServerPluginHost::new(curated_registry());
        let mut stale = aqi_snapshot();
        stale.stale = true;
        let mut error = aqi_snapshot();
        error.stale = true;
        error.error = Some("Fetch failed".into());
        let mut missing = aqi_snapshot();
        missing.value = serde_json::json!({});

        for (name, snapshot) in [
            ("svg-aqi-fresh", aqi_snapshot()),
            ("svg-aqi-stale", stale),
            ("svg-aqi-error", error),
            ("svg-aqi-missing-data", missing),
        ] {
            let frame = host
                .rasterize(&RasterRequest::PluginSvg {
                    plugin_id: "svg-aqi".into(),
                    snapshot,
                    fields: vec![],
                })
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            crate::rasterizer::tests::assert_raster_regression(
                name,
                &crate::rasterizer::RasterizedFrame {
                    digest: frame.digest,
                    bytes: frame.bytes.to_vec(),
                },
            );
        }
    }

    fn replace_font_name(bytes: &mut [u8], from: &[u8], to: &[u8]) {
        assert_eq!(from.len(), to.len());
        let mut offset = 0;
        while let Some(index) = bytes[offset..]
            .windows(from.len())
            .position(|window| window == from)
        {
            let start = offset + index;
            bytes[start..start + to.len()].copy_from_slice(to);
            offset = start + to.len();
        }
    }

    fn inter_with_family(family: &str) -> Vec<u8> {
        assert_eq!(family.len(), 5);
        let mut bytes = include_bytes!("../../../../tools/fonts/Inter-Regular.ttf").to_vec();
        replace_font_name(&mut bytes, b"Inter", family.as_bytes());
        let from = [0, b'I', 0, b'n', 0, b't', 0, b'e', 0, b'r'];
        let mut to = Vec::with_capacity(10);
        for byte in family.bytes() {
            to.extend_from_slice(&[0, byte]);
        }
        replace_font_name(&mut bytes, &from, &to);
        bytes
    }

    fn write_svg_plugin(
        root: &Path,
        id: &str,
        asset_kind: &str,
        asset_file: &str,
        asset_bytes: &[u8],
        family: &str,
    ) {
        let directory = root.join(id);
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join(asset_file), asset_bytes).unwrap();
        fs::write(
            directory.join("face.svg"),
            format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368"><text x="24" y="100" font-family="{family}" font-size="40">A</text></svg>"#
            ),
        )
        .unwrap();
        fs::write(
            directory.join("manifest.toml"),
            format!(
                r#"manifest_version = 2
name = "{id}"
version = "1.0.0"

[source]
kind = "json"
url = "https://example.invalid/data.json"
refresh_minutes = 15

[[assets]]
kind = "{asset_kind}"
file = "{asset_file}"

[template]
kind = "svg"
file = "face.svg"
"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn server_plugin_host_rasterize_loads_declared_fonts_but_not_image_assets() {
        let temp = tempfile::tempdir().unwrap();
        write_svg_plugin(
            temp.path(),
            "declared-font",
            "font",
            "font.ttf",
            &inter_with_family("Other"),
            "Other",
        );
        write_svg_plugin(
            temp.path(),
            "image-not-font",
            "image",
            "image.ttf",
            &inter_with_family("Image"),
            "Image",
        );
        let (registry, failures) = PluginRegistry::load(temp.path()).unwrap();
        assert!(failures.is_empty(), "unexpected failures: {failures:?}");
        let mut host = ServerPluginHost::new(Arc::new(registry));
        let request = |plugin_id: &str| RasterRequest::PluginSvg {
            plugin_id: plugin_id.into(),
            snapshot: ProviderSnapshot {
                value: serde_json::json!({}),
                refreshed_at: Some(Utc::now()),
                age: Some(std::time::Duration::ZERO),
                stale: false,
                error: None,
            },
            fields: vec![],
        };

        host.rasterize(&request("declared-font"))
            .expect("the declared font's embedded family must reach resvg");
        let error = host
            .rasterize(&request("image-not-font"))
            .expect_err("an image asset must not expand the font allowlist");
        assert!(error.contains("font family") && error.contains("Image"));
    }

    #[test]
    fn unknown_plugin_id_is_an_error_not_a_panic() {
        let mut host = ServerPluginHost::new(curated_registry());

        let error = host
            .render_scene("not-installed", &aqi_snapshot(), 1)
            .expect_err("unknown plugin must be refused");

        assert!(error.contains("unknown plugin id") && error.contains("not-installed"));
    }

    #[test]
    fn desired_assets_preserve_resolved_digests_lengths_and_allocations() {
        let registry = curated_registry();
        let resolved = registry.all_assets();
        let expected: Vec<_> = resolved
            .iter()
            .map(|asset| (asset.digest, asset.bytes.len(), Arc::clone(&asset.bytes)))
            .collect();
        let mut host = ServerPluginHost::new(Arc::clone(&registry));

        let desired = host.desired_assets();

        assert_eq!(desired.len(), expected.len());
        for (desired, (resolved_digest, resolved_len, resolved_bytes)) in
            desired.iter().zip(expected)
        {
            assert_eq!(desired.digest, resolved_digest);
            assert_eq!(desired.bytes.len(), resolved_len);
            assert!(
                Arc::ptr_eq(&desired.bytes, &resolved_bytes),
                "asset bytes were rebuilt instead of cloning resolve_assets' Arc"
            );
        }
    }

    #[test]
    fn render_failure_messages_preserve_transient_and_permanent_classes() {
        let timeout = crate::plugin_provider::PluginCapError::RenderWallClockExceeded {
            limit: std::time::Duration::from_millis(250),
            elapsed: std::time::Duration::from_millis(251),
        };
        let transient = classified_failure_message("aqi", &PluginFailure::RenderTimedOut, &timeout);
        assert!(transient.starts_with("transient plugin render failure"));

        let compile_error = plugin::CompileError::Expression(plugin::ExprError::Empty);
        let permanent = classified_failure_message(
            "aqi",
            &PluginFailure::Compile(&compile_error),
            &compile_error,
        );
        assert!(permanent.starts_with("permanent plugin render failure"));
    }
}
