//! Task 5 (stage 3b), brief Step 4: proves §6's parity obligation is
//! discharged, not just declared.
//!
//! `stb_truetype` is deterministic, so identical bytes at an identical
//! pixel size produce identical glyphs — the obligation is about byte
//! *provenance*, not the rasterizer (the brief's own words). So this test
//! keeps the rasterizer fixed (`Simulator::render_asset_font_png`,
//! `font_registry_acquire`, `stb_truetype`, exactly as `tests/asset_font.rs`
//! already exercises) and varies only *where the bytes came from*:
//!
//! - The "baked" half is `tests/golden/asset-font/asset-font--72px-digits--*.png`,
//!   already committed by `tests/asset_font.rs` (Task 12) from
//!   `lvgl_sim::assets::INTER_SUBSET_TTF` — bytes baked directly into this
//!   crate's binary via `include_bytes!`, the same way the six retired C
//!   templates baked their fonts into firmware.
//! - The "supplied as an asset" half writes that *same* TTF content to a
//!   fresh temp directory, names it from a plugin manifest, resolves it
//!   through `plugin::resolve_assets` (a real file read + SHA-256, Task 4),
//!   and looks the resulting digest up through `AssetShim::resolve` (Task
//!   5) — the full path a curated plugin's font takes, with nothing
//!   hand-supplied.
//!
//! If the plugin asset pipeline ever hands back the wrong bytes for a
//! digest — truncated, re-encoded, or substituted for another asset — this
//! render diverges from the committed golden. Comparing against an
//! already-committed golden (rather than rendering the "baked" half again
//! in this same process) is deliberate: `csrc/sim_shim.c`'s asset store
//! registers a digest idempotently (see `sim_asset_register`'s doc), so a
//! second `render_asset_font_png` call for the *same* digest in the same
//! process would silently reuse whatever bytes were registered first and
//! could never observe a wrong second registration. Reading the golden
//! from disk instead means the only render in this process is the one
//! sourced through `AssetShim`, and it is the first-ever registration of
//! that digest in this fresh test binary — nothing to fall back to but the
//! bytes `AssetShim::resolve` actually returns.

use lvgl_sim::asset_shim::AssetShim;
use lvgl_sim::assets::{INTER_SUBSET_SHA256, INTER_SUBSET_TTF};
use lvgl_sim::cases::AssetFontCase;
use lvgl_sim::{SimOrientation, Simulator};
use plugin::{Asset, PluginManifest, Source, resolve_assets};

fn minimal_manifest(assets: Vec<Asset>) -> PluginManifest {
    PluginManifest {
        name: "parity-check".to_string(),
        version: "1.0.0".to_string(),
        source: Source::Json {
            url: "https://example.invalid/x.json".to_string(),
            refresh_minutes: 15,
        },
        assets,
        nodes: Vec::new(),
        repeats: Vec::new(),
    }
}

fn golden_bytes(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/asset-font")
        .join(format!("{name}.png"));
    std::fs::read(&path)
        .unwrap_or_else(|error| panic!("read committed golden {}: {error}", path.display()))
}

fn render_via_plugin_asset_pipeline(orientation: SimOrientation) -> Vec<u8> {
    let dir = tempfile::tempdir().expect("tempdir");
    // The identical bytes `tests/asset_font.rs` baked its golden from,
    // written to disk as a plugin would ship them, under a name that has
    // nothing to do with the constant it came from -- content addressing
    // must not care what the file is called.
    std::fs::write(dir.path().join("inter-subset.ttf"), INTER_SUBSET_TTF).expect("write fixture");
    let manifest = minimal_manifest(vec![Asset::Font {
        file: "inter-subset.ttf".to_string(),
    }]);

    let set = resolve_assets(&manifest, dir.path()).expect("resolve plugin asset");
    let resolved = set.get("inter-subset.ttf").expect("resolved");

    // Sanity check, not the obligation itself: if this fails, the fixture
    // bytes and the committed constant have already drifted apart, and the
    // render comparison below would be comparing two different faces for
    // the wrong reason.
    assert_eq!(
        resolved.digest, INTER_SUBSET_SHA256,
        "the fixture written to disk must hash to the same digest as the baked-in constant"
    );

    let shim = AssetShim::from_asset_set(&set);
    let bytes = shim.resolve(&resolved.digest).expect(
        "the digest resolve_assets just produced must resolve through the shim that indexed it",
    );
    // `AssetFontCase::ttf_bytes` is `&'static [u8]`: every other case in
    // this crate supplies compile-time-embedded bytes. Leaking this test's
    // owned `Vec<u8>` for the remaining life of the process is the
    // standard way to get that lifetime for bytes that only exist at test
    // time, and it costs one allocation never freed in a short-lived test
    // binary.
    let bytes: &'static [u8] = bytes.to_vec().leak();

    let mut simulator = Simulator::new().expect("simulator");
    simulator
        .render_asset_font_png(&AssetFontCase {
            digest: resolved.digest,
            ttf_bytes: bytes,
            pixel_size: 72,
            text: "12:34".to_string(),
            orientation,
        })
        .expect("render bytes resolved through the plugin asset pipeline")
}

#[test]
fn a_font_resolved_through_the_plugin_asset_pipeline_matches_the_baked_in_reference_landscape() {
    let via_pipeline = render_via_plugin_asset_pipeline(SimOrientation::Landscape);
    let baked = golden_bytes("asset-font--72px-digits--landscape");

    assert_eq!(
        via_pipeline, baked,
        "bytes resolved through plugin::resolve_assets + AssetShim must render pixel-for-pixel \
         identically to the reference baked directly into this crate -- any difference means the \
         wrong bytes arrived somewhere on the plugin asset path"
    );
}

#[test]
fn a_font_resolved_through_the_plugin_asset_pipeline_matches_the_baked_in_reference_flipped() {
    let via_pipeline = render_via_plugin_asset_pipeline(SimOrientation::LandscapeFlipped);
    let baked = golden_bytes("asset-font--72px-digits--flipped");

    assert_eq!(
        via_pipeline, baked,
        "bytes resolved through plugin::resolve_assets + AssetShim must render pixel-for-pixel \
         identically to the reference baked directly into this crate -- any difference means the \
         wrong bytes arrived somewhere on the plugin asset path"
    );
}
