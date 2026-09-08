//! Parses and bounds the declarative plugin manifest (stage 3b).
//!
//! A plugin manifest is a TOML document describing a card's face as a
//! display list: a data source, a set of named assets, and a list of scene
//! nodes. This crate turns untrusted manifest bytes into a typed, bounded
//! [`manifest::PluginManifest`] or a named [`manifest::ManifestError`];
//! parses and evaluates the `{{ ... }}` restricted expression syntax inside
//! `value`/`glyph` fields; resolves `[[assets]]` entries to content-addressed
//! digests via [`assets::resolve_assets`]; and compiles a manifest plus a
//! fetched provider snapshot into a wire `Scene` via
//! [`compile::compile_scene`] (against an always-empty [`assets::AssetSet`],
//! so an asset-bearing manifest fails closed) or
//! [`compile::compile_scene_with_assets`] (which resolves `image`/`glyph`
//! nodes and asset fonts against a real one). The v2 `summary` key is
//! evaluated separately by [`summary::evaluate_summary`] into the bounded
//! headline a card's tile shows.

mod assets;
mod compile;
mod expr;
mod manifest;
mod summary;

pub use assets::{AssetError, AssetSet, MAX_ASSET_BYTES, ResolvedAsset, resolve_assets};
pub use compile::{
    CompileError, ExpressionSource, MAX_REPEAT_ITEMS, classify_expression_source, compile_scene,
    compile_scene_with_assets, device_binding_requirements,
};
pub use expr::{
    EvalContext, EvalValue, Expr, ExprError, FUEL_BUDGET, Fuel, MAX_DEPTH, MAX_OUTPUT_LEN,
    MAX_PATH_SEGMENTS, MAX_ROUND_PLACES, MAX_SOURCE_LEN, build_icon_map,
};
pub use manifest::{
    Align, Asset, Font, FontTier, Glyph, MAX_ASSETS, MAX_DESCRIPTION_LEN, MAX_DISPLAY_NAME_LEN,
    MAX_EXPANDED_SVG_BYTES, MAX_REFRESH_MINUTES, MAX_REPEAT_GROUPS, MAX_REPEAT_SOURCE_LEN,
    MAX_SOURCE_ROOT_LEN, MAX_SVG_SOURCE_BYTES, MIN_REFRESH_MINUTES, ManifestError, ManifestVersion,
    Node, PluginManifest, Point, Repeat, Source, Template, parse_manifest, parse_manifest_bytes,
};
pub use summary::{MAX_SUMMARY_LEN, SummaryError, evaluate_summary};
