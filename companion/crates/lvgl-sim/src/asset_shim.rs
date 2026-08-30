//! Task 5 (stage 3b): §6's parity obligation.
//!
//! `docs/superpowers/specs/2026-08-22-deskmate-plugin-scene-rendering-design.md`
//! §6 states it precisely: "Today the simulator compiles the firmware's own
//! font C files, so both hosts rasterize identically by construction. Once
//! fonts become runtime assets that stops being automatic: the simulator
//! must resolve the **same digest to the same bytes** as the device." A
//! scene names a font asset by digest (`protocol::SceneFont::Asset {
//! digest, pixel_size }`), and nothing on the host resolves a bare digest
//! by itself -- something has to hold the bytes a digest names and hand
//! them back on request. [`AssetShim`] is that something.
//!
//! # One store, not two
//!
//! `AssetShim::from_asset_set` builds its digest -> bytes index directly
//! from a [`plugin::AssetSet`] (Task 4): it re-keys `AssetSet`'s already-
//! resolved `(file, ResolvedAsset)` pairs by `ResolvedAsset::digest`
//! instead of by the manifest's `file` name, and it clones the `Arc<[u8]>`
//! `ResolvedAsset` already carries -- an `Arc` clone bumps a refcount, not
//! a byte. There is no second read of an asset file anywhere in this
//! module, and no second hash: `AssetSet` already did both, from the one
//! bounded read `plugin::assets::read_bounded` performs, which is exactly
//! "the same content-addressed store the server pushes from" this task's
//! brief asks for -- a server-side transfer path (`server::asset_sync`,
//! Task 7) is expected to source its own transferable bytes from the same
//! `ResolvedAsset::bytes`, so both sides of the wire end up naming a digest
//! from the identical `Arc<[u8]>` allocation this crate's caller resolved.
//!
//! # The failure mode this type exists to close off
//!
//! An unknown digest -- one no manifest asset resolved to -- must be a
//! named [`AssetShimError`], never `Ok` with some other face's bytes
//! quietly substituted in. A silent fallback would make every later font
//! golden (Task 8) measure the wrong bytes while reporting agreement; see
//! `tests::an_unknown_digest_is_an_error_not_a_silent_fallback` for the
//! test that pins this, and
//! `tests/asset_shim_parity.rs` for the render-level proof that the bytes
//! this shim hands back are the bytes that actually get drawn.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use plugin::AssetSet;
use protocol::ASSET_DIGEST_LEN;

/// A failure resolving a digest through an [`AssetShim`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetShimError {
    /// No asset [`AssetShim::from_asset_set`] indexed resolves to this
    /// digest. The caller must treat this as a hard failure -- there is no
    /// correct face to fall back to, and drawing one anyway would draw the
    /// wrong bytes while looking like success.
    UnknownDigest { digest: [u8; ASSET_DIGEST_LEN] },
}

impl fmt::Display for AssetShimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssetShimError::UnknownDigest { digest } => {
                write!(
                    f,
                    "asset digest {digest:02x?} is not resolved by this AssetShim"
                )
            }
        }
    }
}

impl std::error::Error for AssetShimError {}

/// The simulator's digest -> bytes resolver for plugin assets. Built once
/// from a [`plugin::AssetSet`] (see the module doc for why that is the
/// whole store, not a copy of it), then queried by
/// [`protocol::SceneFont::Asset`]'s `digest` field however many times a
/// render needs.
#[derive(Debug, Clone, Default)]
pub struct AssetShim {
    by_digest: HashMap<[u8; ASSET_DIGEST_LEN], Arc<[u8]>>,
}

impl AssetShim {
    /// Indexes every asset `set` already resolved, by digest. Two manifest
    /// assets that resolved to the same digest (byte-identical content,
    /// per `plugin::assets`' own "ship once" guarantee) collapse to one
    /// entry here, which is correct: a digest names content, not a
    /// particular manifest's asset.
    pub fn from_asset_set(set: &AssetSet) -> Self {
        let by_digest = set
            .iter()
            .map(|(_, resolved)| (resolved.digest, resolved.bytes.clone()))
            .collect();
        Self { by_digest }
    }

    /// Resolves `digest` to its bytes, or a named [`AssetShimError`] --
    /// never a silent fallback. See this module's doc on why that is the
    /// one failure mode this type exists to prevent.
    pub fn resolve(&self, digest: &[u8; ASSET_DIGEST_LEN]) -> Result<Arc<[u8]>, AssetShimError> {
        self.by_digest
            .get(digest)
            .cloned()
            .ok_or(AssetShimError::UnknownDigest { digest: *digest })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use plugin::{Asset, ManifestVersion, PluginManifest, Source, Template, resolve_assets};

    use super::*;

    /// A v1-shaped manifest: this shim predates manifest v2 and its assets
    /// are the v1 `[[assets]]` form, so it pins the v1 contract values
    /// explicitly rather than tracking whatever v2 adds later.
    fn minimal_manifest(assets: Vec<Asset>) -> PluginManifest {
        PluginManifest {
            manifest_version: ManifestVersion::V1,
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            source: Source::Json {
                url: "https://example.invalid/x.json".to_string(),
                refresh_minutes: 15,
                root: None,
            },
            template: Template::Scene,
            assets,
            nodes: Vec::new(),
            repeats: Vec::new(),
        }
    }

    // -- Step 1: the failing tests, written and watched fail before
    // `AssetShim` existed (the crate did not compile: `asset_shim` had no
    // module to import). --

    #[test]
    fn a_registered_digest_resolves_to_its_asset_set_bytes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bytes: &[u8] = b"the exact bytes this digest must resolve to";
        fs::write(dir.path().join("font.ttf"), bytes).expect("write fixture");
        let manifest = minimal_manifest(vec![Asset::Font {
            file: "font.ttf".to_string(),
        }]);
        let set = resolve_assets(&manifest, dir.path()).expect("resolve");
        let resolved = set.get("font.ttf").expect("resolved");

        let shim = AssetShim::from_asset_set(&set);
        let got = shim
            .resolve(&resolved.digest)
            .expect("a registered digest must resolve");

        assert_eq!(&*got, bytes);
    }

    #[test]
    fn an_unknown_digest_is_an_error_not_a_silent_fallback() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("font.ttf"), b"some registered font").expect("write fixture");
        let manifest = minimal_manifest(vec![Asset::Font {
            file: "font.ttf".to_string(),
        }]);
        let set = resolve_assets(&manifest, dir.path()).expect("resolve");
        let shim = AssetShim::from_asset_set(&set);

        // A digest that was never resolved into `set` at all -- not a typo
        // of a real one, a value that provably matches nothing.
        let unknown_digest = [0xAB_u8; ASSET_DIGEST_LEN];
        let err = shim.resolve(&unknown_digest).unwrap_err();

        assert_eq!(
            err,
            AssetShimError::UnknownDigest {
                digest: unknown_digest
            },
            "an unknown digest must fail as the named UnknownDigest error, never Ok(_) with \
             some other asset's bytes silently substituted in"
        );
    }

    #[test]
    fn an_empty_shim_resolves_nothing() {
        let shim = AssetShim::default();
        let digest = [0x11_u8; ASSET_DIGEST_LEN];

        assert_eq!(
            shim.resolve(&digest).unwrap_err(),
            AssetShimError::UnknownDigest { digest }
        );
    }

    #[test]
    fn two_assets_resolving_to_the_same_digest_both_resolve_through_it() {
        // Byte-identical content under two different manifest file names
        // hashes to the same digest (plugin::assets's own "ship once"
        // guarantee) -- confirm the shim's index collapses them to the one
        // entry a digest-keyed lookup implies, rather than losing one.
        let dir = tempfile::tempdir().expect("tempdir");
        let bytes: &[u8] = b"shared content, two names";
        fs::write(dir.path().join("a.ttf"), bytes).expect("write a");
        fs::write(dir.path().join("b.ttf"), bytes).expect("write b");
        let manifest = minimal_manifest(vec![
            Asset::Font {
                file: "a.ttf".to_string(),
            },
            Asset::Font {
                file: "b.ttf".to_string(),
            },
        ]);
        let set = resolve_assets(&manifest, dir.path()).expect("resolve");
        let digest = set.get("a.ttf").expect("a").digest;
        assert_eq!(digest, set.get("b.ttf").expect("b").digest);

        let shim = AssetShim::from_asset_set(&set);
        assert_eq!(&*shim.resolve(&digest).expect("resolves"), bytes);
    }

    #[test]
    fn resolving_shares_the_asset_sets_own_allocation_not_a_copy() {
        // `Arc::ptr_eq` proves the shim's bytes are the SAME allocation
        // `AssetSet` already resolved, not a byte-for-byte copy of it --
        // the module doc's "one store, not two" claim, checked rather than
        // just asserted.
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("font.ttf"), b"identity, not equality").expect("write fixture");
        let manifest = minimal_manifest(vec![Asset::Font {
            file: "font.ttf".to_string(),
        }]);
        let set = resolve_assets(&manifest, dir.path()).expect("resolve");
        let resolved = set.get("font.ttf").expect("resolved");

        let shim = AssetShim::from_asset_set(&set);
        let got = shim.resolve(&resolved.digest).expect("resolves");

        assert!(
            Arc::ptr_eq(&got, &resolved.bytes),
            "AssetShim::resolve must hand back the AssetSet's own Arc<[u8]>, not a copy"
        );
    }
}
