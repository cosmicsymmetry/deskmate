//! Task 8: the two curated plugins compile end to end -- real TOML on disk
//! under `companion/plugins/{aqi,agenda}/`, real committed assets, and a
//! captured-in-spirit fixture (aqi's reused from Task 2, agenda's new for
//! this task) standing in for the provider payload -- not a shape built to
//! flatter the compiler.
//!
//! This is the one place in the repo that proves `compile_scene_with_assets`
//! (Task 8's addition to `compile.rs`, closing the "does not yet resolve
//! assets" gap `compile_scene` alone still has) actually resolves an `image`
//! node and a `glyph` node against real, on-disk, content-addressed bytes --
//! not just the synthetic in-memory fixtures `compile.rs`'s own unit tests
//! build.

use std::path::{Path, PathBuf};

use plugin::{
    CompileError, compile_scene, compile_scene_with_assets, parse_manifest, resolve_assets,
};
use protocol::{SceneNode, SceneValue};
use providers::ProviderSnapshot;

fn plugins_dir() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is `companion/crates/plugin`; the two curated
    // plugins live at `companion/plugins/{aqi,agenda}`, two levels up and
    // back down -- not under this crate, since they are content shipped to
    // devices via the server, not test fixtures of this crate.
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins")
}

fn payload_from_envelope(raw: &str) -> serde_json::Value {
    let root: serde_json::Value = serde_json::from_str(raw).expect("fixture is valid JSON");
    assert_eq!(root["status"], "ok");
    root["payload"].clone()
}

fn snapshot(value: serde_json::Value) -> ProviderSnapshot<serde_json::Value> {
    ProviderSnapshot {
        value,
        refreshed_at: None,
        age: None,
        stale: false,
        error: None,
    }
}

fn metrics() -> app_core::BakedFontMetrics {
    app_core::BakedFontMetrics::SHIPPED
}

fn text_value(node: &SceneNode) -> &SceneValue {
    let SceneNode::Text(text) = node else {
        panic!("expected a Text node, got {node:?}");
    };
    &text.value
}

fn literal(node: &SceneNode) -> &str {
    match text_value(node) {
        SceneValue::Literal(text) => text,
        SceneValue::Binding(text) => panic!("expected a literal, got binding {text:?}"),
    }
}

// ---------------------------------------------------------------------------
// aqi: the icon-font path and `field.*`.
// ---------------------------------------------------------------------------

const AQI_FIXTURE: &str = include_str!("fixtures/aqi_response.json");

#[test]
fn the_aqi_manifest_parses_and_bounds_clean() {
    let dir = plugins_dir().join("aqi");
    let source = std::fs::read_to_string(dir.join("manifest.toml")).expect("read aqi manifest");
    let manifest = parse_manifest(&source).expect("aqi manifest must parse and bound clean");
    assert_eq!(manifest.name, "aqi");
    assert_eq!(manifest.assets.len(), 1);
}

#[test]
fn the_aqi_manifest_compiles_against_its_real_assets_and_captured_fixture() {
    let dir = plugins_dir().join("aqi");
    let source = std::fs::read_to_string(dir.join("manifest.toml")).expect("read aqi manifest");
    let manifest = parse_manifest(&source).expect("aqi manifest must parse");
    let assets = resolve_assets(&manifest, &dir).expect("aqi assets must resolve");
    let data = payload_from_envelope(AQI_FIXTURE);

    let scene = compile_scene_with_assets(&manifest, &snapshot(data), &metrics(), 1, &assets)
        .expect("aqi manifest must compile against its real fixture and assets");

    // Hero: the numeric AQI reading, unrounded because it is already whole.
    assert_eq!(literal(&scene.nodes[2]), "42");
    // The icon-font path: a real glyph node, resolved against the real
    // committed `icons.ttf` digest, drawing the letter `icon()` maps
    // "moderate" to.
    let SceneNode::Glyph(glyph) = &scene.nodes[1] else {
        panic!("node 1 is not Glyph: {:?}", scene.nodes[1]);
    };
    assert_eq!(glyph.name, "M");
    assert_eq!(glyph.digest, assets.get("icons.ttf").unwrap().digest);
    // `field.*`: compiles to a device-side binding, not a literal -- this
    // manifest is what gives the mechanism its first pixel coverage.
    assert_eq!(
        text_value(&scene.nodes[6]),
        &SceneValue::Binding("field.title".to_string())
    );
}

// ---------------------------------------------------------------------------
// agenda: the repeat form, a real truncation case, and an image asset.
// ---------------------------------------------------------------------------

const AGENDA_FIXTURE: &str = include_str!("fixtures/agenda_response.json");

#[test]
fn the_agenda_manifest_parses_and_bounds_clean() {
    let dir = plugins_dir().join("agenda");
    let source = std::fs::read_to_string(dir.join("manifest.toml")).expect("read agenda manifest");
    let manifest = parse_manifest(&source).expect("agenda manifest must parse and bound clean");
    assert_eq!(manifest.name, "agenda");
    assert_eq!(manifest.repeats.len(), 1);
}

#[test]
fn the_agenda_manifest_compiles_against_its_real_assets_and_captured_fixture() {
    let dir = plugins_dir().join("agenda");
    let source = std::fs::read_to_string(dir.join("manifest.toml")).expect("read agenda manifest");
    let manifest = parse_manifest(&source).expect("agenda manifest must parse");
    let assets = resolve_assets(&manifest, &dir).expect("agenda assets must resolve");
    let data = payload_from_envelope(AGENDA_FIXTURE);

    let scene = compile_scene_with_assets(&manifest, &snapshot(data), &metrics(), 1, &assets)
        .expect("agenda manifest must compile against its real fixture and assets");

    // header text + image badge + 5 rows * 2 nodes = 12. The shared
    // stale/error footer `with_scene_data_state` appends nothing at all
    // for a fresh, error-free snapshot (`push_state_footer`'s `None => return`
    // arm), so this fixture's fresh state means no 13th node.
    assert_eq!(scene.nodes.len(), 12);

    // The image path: a real image node, resolved against the real
    // committed `badge.rgb565` digest.
    let SceneNode::Image(image) = &scene.nodes[1] else {
        panic!("node 1 is not Image: {:?}", scene.nodes[1]);
    };
    assert_eq!(image.digest, assets.get("badge.rgb565").unwrap().digest);

    // The repeat form, capped at 5 even though the fixture's `events` array
    // holds 6 -- the sixth event's title must never appear anywhere.
    let all_literals: Vec<&str> = scene
        .nodes
        .iter()
        .filter_map(|node| match node {
            SceneNode::Text(text) => match &text.value {
                SceneValue::Literal(value) => Some(value.as_str()),
                SceneValue::Binding(_) => None,
            },
            _ => None,
        })
        .collect();
    assert!(
        !all_literals.iter().any(|text| text.contains("sixth item")),
        "the sixth event must be dropped by the MAX_REPEAT_ITEMS cap, found in {all_literals:?}"
    );

    // Row 0's time and truncated title (evt-1: a 92-character title,
    // truncate()'d to 40 characters -- the real truncation case).
    assert_eq!(literal(&scene.nodes[2]), "09:00");
    assert_eq!(
        literal(&scene.nodes[3]),
        "Standup with the whole platform team to "
    );
    assert_eq!(literal(&scene.nodes[3]).chars().count(), 40);

    // Row 2 (evt-3) is entirely null in the fixture -- both `default()`
    // fallbacks must render, not panic or propagate a missing value.
    assert_eq!(literal(&scene.nodes[6]), "--:--");
    assert_eq!(literal(&scene.nodes[7]), "(untitled)");
}

// ---------------------------------------------------------------------------
// Both plugins: `compile_scene` (no assets) fails closed the way any
// manifest using an asset-bearing node must, until a caller resolves one.
// ---------------------------------------------------------------------------

#[test]
fn neither_curated_plugin_compiles_through_the_no_assets_entry_point() {
    for plugin_name in ["aqi", "agenda"] {
        let dir = plugins_dir().join(plugin_name);
        let source = std::fs::read_to_string(dir.join("manifest.toml"))
            .unwrap_or_else(|error| panic!("read {plugin_name} manifest: {error}"));
        let manifest = parse_manifest(&source)
            .unwrap_or_else(|error| panic!("{plugin_name} manifest must parse: {error}"));
        let data = payload_from_envelope(if plugin_name == "aqi" {
            AQI_FIXTURE
        } else {
            AGENDA_FIXTURE
        });
        let err = compile_scene(&manifest, &snapshot(data), &metrics(), 1)
            .expect_err("an asset-bearing manifest must not compile with no assets resolved");
        assert!(
            matches!(err, CompileError::AssetNotResolved { .. }),
            "{plugin_name}: expected AssetNotResolved, got {err:?}"
        );
    }
}
