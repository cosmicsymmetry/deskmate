//! Content-addresses plugin assets and resolves icon-font glyph names to
//! codepoints (stage 3b, spec §1).
//!
//! `[[assets]]` entries in a [`PluginManifest`] (Task 1) name a font,
//! icon-font, or image file by filename; nothing upstream of this module
//! reads the file's bytes. [`resolve_assets`] does exactly that: it reads
//! each asset's bytes from `base_dir`, content-addresses them by SHA-256 --
//! the same digest the wire's asset-transfer path and the device's
//! flash-resident asset store (spec §1) already key on
//! (`protocol::ASSET_DIGEST_LEN`) -- and builds the flat glyph-name ->
//! codepoint table `icon(name)` resolves through, using Task 2's own
//! [`crate::expr::build_icon_map`] rather than a second copy of that logic.
//!
//! # Why SHA-256 makes cross-plugin dedup free
//!
//! Content-addressing is deterministic by construction: the same bytes
//! always hash to the same digest, on any host, in any process, in any
//! order. Two different plugins whose manifests each name a font file with
//! byte-identical content therefore resolve to the same digest without
//! this module doing anything to notice -- the device's asset store (spec
//! §1, "ship once") and any server-side store key on that digest, never on
//! a manifest's asset name, which is only ever meaningful within its own
//! manifest.
//!
//! # What this module does not do
//!
//! It does not push bytes over the wire -- that is Task 7's
//! `server::asset_sync`, whose existing `resolve_assets` for schema-v5
//! config assets is this module's closest precedent, and whose established
//! per-asset byte ceiling (`app_core::MAX_ASSET_BYTES`) this module reuses
//! rather than inventing a second one for bytes destined for the same
//! physical flash-resident store. It does not rasterize or measure a font
//! -- that is `lvgl-sim`'s asset shim, Task 5. And it does not scan a
//! manifest's node expressions for every `icon(...)` call to pre-validate
//! every name a manifest could ever ask for: [`AssetSet::icon_codepoint`]
//! rejects an unresolvable name the moment something asks for one (Task
//! 3/7's compiler), which is early enough to satisfy spec §1's "the server
//! validates the name resolves" without this module needing its own copy
//! of the expression grammar.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use protocol::{ASSET_DIGEST_LEN, AssetKind};
use sha2::{Digest, Sha256};

use crate::expr::build_icon_map;
use crate::manifest::{Asset, PluginManifest};

/// Maximum byte length of one resolved asset file. Imported, not restated:
/// it *is* `app_core::MAX_ASSET_BYTES`, the per-asset ceiling the
/// schema-v5 (non-plugin) asset path already enforces for bytes destined
/// for the same physical asset store, so a plugin asset and a config asset
/// share one budget rather than drifting apart.
pub const MAX_ASSET_BYTES: u32 = app_core::MAX_ASSET_BYTES;

/// One manifest asset, resolved to the digest, byte length, and wire kind
/// a server-side transfer path needs to stream it (mirroring
/// `server::asset_sync::DesiredAsset`'s shape, without pulling the bytes
/// themselves into this crate).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedAsset {
    pub digest: [u8; ASSET_DIGEST_LEN],
    pub len: u32,
    pub kind: AssetKind,
}

/// The result of resolving every `[[assets]]` entry in one manifest: a
/// lookup from a manifest asset's `file` name to its [`ResolvedAsset`], and
/// the flat icon-name -> codepoint table every `icon-font` asset in the
/// manifest contributes to.
#[derive(Debug, Clone, Default)]
pub struct AssetSet {
    resolved: HashMap<String, ResolvedAsset>,
    icon_codepoints: HashMap<String, u32>,
}

impl AssetSet {
    /// Looks up a resolved asset by the manifest's `file` name -- the same
    /// string a `Font::Asset { asset, .. }` field names it by.
    pub fn get(&self, file: &str) -> Option<&ResolvedAsset> {
        self.resolved.get(file)
    }

    /// Resolves an `icon(name)` glyph name to its codepoint, across every
    /// `icon-font` asset in the manifest. Rejects a name no glyph defines
    /// -- spec §1's "the server validates the name resolves" -- rather
    /// than resolving to a fallback codepoint and letting a broken icon
    /// name draw a tofu box on the panel.
    pub fn icon_codepoint(&self, name: &str) -> Result<u32, AssetError> {
        self.icon_codepoints
            .get(name)
            .copied()
            .ok_or_else(|| AssetError::UnknownIconName {
                name: name.to_string(),
            })
    }

    /// Every resolved asset, `file` name paired with its [`ResolvedAsset`].
    /// A server-side transfer path uses this to build one desired-asset
    /// entry per manifest asset without re-deriving the manifest's asset
    /// list.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &ResolvedAsset)> {
        self.resolved
            .iter()
            .map(|(file, asset)| (file.as_str(), asset))
    }
}

/// A failure resolving one manifest's `[[assets]]` entries to bytes on
/// disk, or resolving an icon name through the result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetError {
    /// Reading `file` from `base_dir` failed -- missing file, permissions,
    /// or any other I/O error. The message is preserved for diagnostics;
    /// it is not itself a named bound this module owns.
    Io { file: String, message: String },
    /// An asset file is exactly zero bytes. A font, icon-font, or image
    /// blob with no content is always an authoring mistake, never
    /// legitimate: rejected here rather than shipped to fail (or, worse,
    /// succeed vacuously) at the device's font/image loader.
    Empty { file: String },
    /// An asset file exceeds [`MAX_ASSET_BYTES`].
    TooLarge {
        file: String,
        limit: u32,
        actual: usize,
    },
    /// Two `[[assets]] kind = "icon-font"` entries in the same manifest
    /// define the same glyph `name`. `icon(name)` has no syntax to say
    /// which font it means, so a collision is refused at resolve time
    /// rather than silently resolved by declaration order -- the same
    /// "fail at load, not draw a tofu box" reasoning as
    /// [`AssetError::UnknownIconName`].
    DuplicateIconName {
        name: String,
        first_asset: String,
        second_asset: String,
    },
    /// [`AssetSet::icon_codepoint`] was asked for a name no `[[assets]]`
    /// icon-font glyph defines.
    UnknownIconName { name: String },
}

impl std::fmt::Display for AssetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for AssetError {}

fn asset_file(asset: &Asset) -> &str {
    match asset {
        Asset::Font { file } | Asset::IconFont { file, .. } | Asset::Image { file } => file,
    }
}

fn wire_kind(asset: &Asset) -> AssetKind {
    match asset {
        Asset::Font { .. } => AssetKind::Font,
        Asset::IconFont { .. } => AssetKind::IconFont,
        Asset::Image { .. } => AssetKind::Image,
    }
}

/// Reads, content-addresses, and bounds every `[[assets]]` entry in
/// `manifest`, relative to `base_dir`, and builds the icon-name codepoint
/// table every `icon-font` entry contributes to.
///
/// Untrusted input: `manifest` may be attacker-controlled TOML (Task 1
/// bounds its *shape*, not its bytes) and the files it names may be
/// attacker-controlled bytes. Every length is bounded before it is used;
/// nothing here panics, hangs, or allocates without a cap in front of it.
pub fn resolve_assets(manifest: &PluginManifest, base_dir: &Path) -> Result<AssetSet, AssetError> {
    let mut resolved = HashMap::with_capacity(manifest.assets.len());
    let mut icon_codepoints = HashMap::new();
    let mut icon_name_owner: HashMap<String, String> = HashMap::new();

    for asset in &manifest.assets {
        let file = asset_file(asset);
        let path = base_dir.join(file);
        let bytes = fs::read(&path).map_err(|error| AssetError::Io {
            file: file.to_string(),
            message: error.to_string(),
        })?;

        if bytes.is_empty() {
            return Err(AssetError::Empty {
                file: file.to_string(),
            });
        }

        let len = u32::try_from(bytes.len()).map_err(|_| AssetError::TooLarge {
            file: file.to_string(),
            limit: MAX_ASSET_BYTES,
            actual: bytes.len(),
        })?;
        if len > MAX_ASSET_BYTES {
            return Err(AssetError::TooLarge {
                file: file.to_string(),
                limit: MAX_ASSET_BYTES,
                actual: bytes.len(),
            });
        }

        let digest: [u8; ASSET_DIGEST_LEN] = Sha256::digest(&bytes).into();
        resolved.insert(
            file.to_string(),
            ResolvedAsset {
                digest,
                len,
                kind: wire_kind(asset),
            },
        );

        if let Asset::IconFont { glyphs, .. } = asset {
            for (name, codepoint) in build_icon_map(glyphs) {
                if let Some(owner) = icon_name_owner.get(&name) {
                    return Err(AssetError::DuplicateIconName {
                        name,
                        first_asset: owner.clone(),
                        second_asset: file.to_string(),
                    });
                }
                icon_name_owner.insert(name.clone(), file.to_string());
                icon_codepoints.insert(name, codepoint);
            }
        }
    }

    Ok(AssetSet {
        resolved,
        icon_codepoints,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::manifest::{Glyph, Source};

    fn minimal_manifest(assets: Vec<Asset>) -> PluginManifest {
        PluginManifest {
            name: "test".to_string(),
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

    // -- Step 1: the load-bearing determinism test. Content-addressing is
    // only worth anything if the same bytes always produce the same
    // digest. --
    #[test]
    fn resolving_the_same_asset_bytes_twice_produces_the_same_digest() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("font.ttf"), b"deterministic font bytes").expect("write fixture");
        let manifest = minimal_manifest(vec![Asset::Font {
            file: "font.ttf".to_string(),
        }]);

        let first = resolve_assets(&manifest, dir.path()).expect("resolve once");
        let second = resolve_assets(&manifest, dir.path()).expect("resolve again");

        assert_eq!(
            first.get("font.ttf").expect("resolved once").digest,
            second.get("font.ttf").expect("resolved again").digest,
        );
    }

    #[test]
    fn two_manifests_referencing_byte_identical_files_produce_the_same_digest() {
        let dir_a = tempfile::tempdir().expect("tempdir a");
        let dir_b = tempfile::tempdir().expect("tempdir b");
        fs::write(dir_a.path().join("weather.ttf"), b"shared font payload").expect("write a");
        fs::write(dir_b.path().join("icons.ttf"), b"shared font payload").expect("write b");

        let manifest_a = minimal_manifest(vec![Asset::Font {
            file: "weather.ttf".to_string(),
        }]);
        let manifest_b = minimal_manifest(vec![Asset::Font {
            file: "icons.ttf".to_string(),
        }]);

        let resolved_a = resolve_assets(&manifest_a, dir_a.path()).expect("resolve a");
        let resolved_b = resolve_assets(&manifest_b, dir_b.path()).expect("resolve b");

        assert_eq!(
            resolved_a.get("weather.ttf").expect("a").digest,
            resolved_b.get("icons.ttf").expect("b").digest,
            "two plugins shipping byte-identical fonts must ship the same digest, per §1's \
             \"ship once\""
        );
    }

    #[test]
    fn a_font_asset_resolves_by_name_with_its_digest_length_and_kind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bytes: &[u8] = b"a small font blob";
        fs::write(dir.path().join("font.ttf"), bytes).expect("write fixture");
        let manifest = minimal_manifest(vec![Asset::Font {
            file: "font.ttf".to_string(),
        }]);

        let resolved = resolve_assets(&manifest, dir.path()).expect("resolve");
        let asset = resolved.get("font.ttf").expect("resolved");

        assert_eq!(asset.kind, AssetKind::Font);
        assert_eq!(asset.len, u32::try_from(bytes.len()).expect("fits"));
        let expected_digest: [u8; ASSET_DIGEST_LEN] = Sha256::digest(bytes).into();
        assert_eq!(asset.digest, expected_digest);
    }

    #[test]
    fn an_image_asset_resolves_with_image_kind() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("logo.bin"), b"image bytes").expect("write fixture");
        let manifest = minimal_manifest(vec![Asset::Image {
            file: "logo.bin".to_string(),
        }]);

        let resolved = resolve_assets(&manifest, dir.path()).expect("resolve");

        assert_eq!(
            resolved.get("logo.bin").expect("resolved").kind,
            AssetKind::Image
        );
    }

    #[test]
    fn an_icon_font_asset_resolves_with_icon_font_kind_and_its_glyph_names_resolve() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("weather-icons.ttf"), b"icon font bytes").expect("write fixture");
        let manifest = minimal_manifest(vec![Asset::IconFont {
            file: "weather-icons.ttf".to_string(),
            glyphs: vec![
                Glyph {
                    name: "haze".to_string(),
                    codepoint: 0xE001,
                },
                Glyph {
                    name: "clear".to_string(),
                    codepoint: 0xE002,
                },
            ],
        }]);

        let resolved = resolve_assets(&manifest, dir.path()).expect("resolve");

        assert_eq!(
            resolved.get("weather-icons.ttf").expect("resolved").kind,
            AssetKind::IconFont
        );
        assert_eq!(resolved.icon_codepoint("haze"), Ok(0xE001));
        assert_eq!(resolved.icon_codepoint("clear"), Ok(0xE002));
    }

    // -- Step 4: boundary cases. --

    #[test]
    fn a_zero_byte_asset_file_is_rejected_as_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("empty.ttf"), b"").expect("write fixture");
        let manifest = minimal_manifest(vec![Asset::Font {
            file: "empty.ttf".to_string(),
        }]);

        let error = resolve_assets(&manifest, dir.path()).expect_err("must be rejected");

        assert_eq!(
            error,
            AssetError::Empty {
                file: "empty.ttf".to_string()
            }
        );
    }

    #[test]
    fn an_asset_file_exactly_at_the_byte_ceiling_resolves() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bytes = vec![0xAB_u8; MAX_ASSET_BYTES as usize];
        fs::write(dir.path().join("max.ttf"), &bytes).expect("write fixture");
        let manifest = minimal_manifest(vec![Asset::Font {
            file: "max.ttf".to_string(),
        }]);

        let resolved = resolve_assets(&manifest, dir.path()).expect("must resolve at the ceiling");

        assert_eq!(
            resolved.get("max.ttf").expect("resolved").len,
            MAX_ASSET_BYTES
        );
    }

    #[test]
    fn an_asset_file_one_byte_over_the_ceiling_is_rejected_by_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bytes = vec![0xAB_u8; MAX_ASSET_BYTES as usize + 1];
        fs::write(dir.path().join("toobig.ttf"), &bytes).expect("write fixture");
        let manifest = minimal_manifest(vec![Asset::Font {
            file: "toobig.ttf".to_string(),
        }]);

        let error = resolve_assets(&manifest, dir.path()).expect_err("must be rejected");

        assert_eq!(
            error,
            AssetError::TooLarge {
                file: "toobig.ttf".to_string(),
                limit: MAX_ASSET_BYTES,
                actual: MAX_ASSET_BYTES as usize + 1,
            }
        );
    }

    #[test]
    fn a_missing_asset_file_is_rejected_as_an_io_error_not_a_panic() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manifest = minimal_manifest(vec![Asset::Font {
            file: "does-not-exist.ttf".to_string(),
        }]);

        let error = resolve_assets(&manifest, dir.path()).expect_err("must be rejected");

        assert!(matches!(error, AssetError::Io { file, .. } if file == "does-not-exist.ttf"));
    }

    #[test]
    fn two_icon_fonts_defining_the_same_glyph_name_is_rejected_as_duplicate() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("a.ttf"), b"font a bytes").expect("write a");
        fs::write(dir.path().join("b.ttf"), b"font b bytes").expect("write b");
        let manifest = minimal_manifest(vec![
            Asset::IconFont {
                file: "a.ttf".to_string(),
                glyphs: vec![Glyph {
                    name: "cloud".to_string(),
                    codepoint: 0xE010,
                }],
            },
            Asset::IconFont {
                file: "b.ttf".to_string(),
                glyphs: vec![Glyph {
                    name: "cloud".to_string(),
                    codepoint: 0xE020,
                }],
            },
        ]);

        let error = resolve_assets(&manifest, dir.path()).expect_err("must be rejected");

        assert_eq!(
            error,
            AssetError::DuplicateIconName {
                name: "cloud".to_string(),
                first_asset: "a.ttf".to_string(),
                second_asset: "b.ttf".to_string(),
            }
        );
    }

    #[test]
    fn an_icon_name_no_glyph_defines_is_rejected_as_unknown() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("icons.ttf"), b"icon font bytes").expect("write fixture");
        let manifest = minimal_manifest(vec![Asset::IconFont {
            file: "icons.ttf".to_string(),
            glyphs: vec![Glyph {
                name: "cloud".to_string(),
                codepoint: 0xE010,
            }],
        }]);

        let resolved = resolve_assets(&manifest, dir.path()).expect("resolve");
        let error = resolved
            .icon_codepoint("does-not-exist")
            .expect_err("must be rejected");

        assert_eq!(
            error,
            AssetError::UnknownIconName {
                name: "does-not-exist".to_string()
            }
        );
    }

    #[test]
    fn an_empty_manifest_resolves_to_an_empty_asset_set() {
        let manifest = minimal_manifest(Vec::new());
        let dir = tempfile::tempdir().expect("tempdir");

        let resolved = resolve_assets(&manifest, dir.path()).expect("resolve");

        assert!(resolved.get("anything").is_none());
        assert_eq!(
            resolved.icon_codepoint("anything"),
            Err(AssetError::UnknownIconName {
                name: "anything".to_string()
            })
        );
        assert_eq!(resolved.iter().count(), 0);
    }
}
