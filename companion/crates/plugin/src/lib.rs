//! Parses and bounds the declarative plugin manifest (stage 3b).
//!
//! A plugin manifest is a TOML document describing a card's face as a
//! display list: a data source, a set of named assets, and a list of scene
//! nodes. This crate turns untrusted manifest bytes into a typed, bounded
//! [`manifest::PluginManifest`] or a named [`manifest::ManifestError`];
//! parses and evaluates the `{{ ... }}` restricted expression syntax inside
//! `value`/`glyph` fields; and compiles a manifest plus a fetched provider
//! snapshot into a wire `Scene` via [`compile::compile_scene`]. It does not
//! yet resolve assets to content-addressed digests -- see `compile.rs`'s
//! module doc for what a manifest needing one compiles to today.

mod assets;
mod compile;
mod expr;
mod manifest;

pub use assets::{AssetError, AssetSet, MAX_ASSET_BYTES, ResolvedAsset, resolve_assets};
pub use compile::{CompileError, MAX_REPEAT_ITEMS, compile_scene};
pub use expr::{
    EvalContext, EvalValue, Expr, ExprError, FUEL_BUDGET, Fuel, MAX_DEPTH, MAX_OUTPUT_LEN,
    MAX_PATH_SEGMENTS, MAX_ROUND_PLACES, MAX_SOURCE_LEN, build_icon_map,
};
pub use manifest::{
    Align, Asset, Font, FontTier, Glyph, MAX_REPEAT_GROUPS, MAX_REPEAT_SOURCE_LEN, ManifestError,
    Node, PluginManifest, Point, Repeat, Source, parse_manifest, parse_manifest_bytes,
};
