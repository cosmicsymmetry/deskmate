//! Parses and bounds the declarative plugin manifest (stage 3b).
//!
//! A plugin manifest is a TOML document describing a card's face as a
//! display list: a data source, a set of named assets, and a list of scene
//! nodes. This crate's first job -- and this module's only job -- is turning
//! untrusted manifest bytes into a typed, bounded [`manifest::PluginManifest`]
//! or a named [`manifest::ManifestError`]. It does not parse the `{{ ... }}`
//! expression syntax inside `value`/`glyph` fields (that is a later task's
//! restricted expression language), compile a manifest into a `Scene`, or
//! resolve assets to content-addressed digests.

mod manifest;

pub use manifest::{
    Align, Asset, Font, FontTier, Glyph, ManifestError, Node, PluginManifest, Source,
    parse_manifest, parse_manifest_bytes,
};
