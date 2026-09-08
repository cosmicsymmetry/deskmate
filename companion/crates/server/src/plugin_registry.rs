//! Startup registry for the curated plugins the server can render.
//!
//! A plugin directory is independent of every other one: malformed TOML,
//! missing files, or an asset-resolution failure is attached to that plugin
//! and does not stop healthy plugins from loading. Registry-wide resource
//! ceilings are different: exceeding them makes the catalog impossible to
//! reconcile with one device and therefore fails the load as a whole.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use app_core::MAX_PLUGIN_ID_LEN;
use plugin::{
    AssetError, AssetSet, MAX_SVG_SOURCE_BYTES, ManifestError, PluginManifest, ResolvedAsset,
    parse_manifest, resolve_assets,
};
use protocol::{ASSET_DIGEST_LEN, MAX_ASSET_DIGESTS};

/// 32 bounds startup filesystem work while leaving 16x headroom over today's curated catalog.
pub const MAX_PLUGINS: usize = 32;

/// The registry's durable-asset ceiling: one below the wire's
/// [`MAX_ASSET_DIGESTS`], because stage 4's `AssetRelease` keep-set must
/// carry every desired durable digest *plus* the one active volatile raster
/// frame. A registry allowed to fill all 32 slots would leave no legal
/// keep-set for that frame, and the release would silently have to drop a
/// desired asset instead.
pub const MAX_DURABLE_REGISTRY_ASSETS: usize = MAX_ASSET_DIGESTS - 1;

/// Every curated plugin the server can render, loaded once at startup.
#[derive(Debug)]
pub struct PluginRegistry {
    plugins: BTreeMap<String, LoadedPlugin>,
}

/// One validated manifest and the assets resolved from its own directory.
#[derive(Debug, Clone)]
pub struct LoadedPlugin {
    pub id: String,
    pub manifest: PluginManifest,
    pub assets: AssetSet,
    /// Registry-owned SVG bytes, loaded and validated once at startup.
    pub svg_source: Option<Arc<str>>,
}

/// One plugin directory that could not be loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginLoadFailure {
    pub id: String,
    pub error: PluginLoadError,
}

/// Why one plugin directory could not join the registry.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PluginLoadError {
    #[error("plugin identifier {plugin_id:?} is invalid: {source}")]
    InvalidIdentifier {
        plugin_id: String,
        #[source]
        source: PluginIdentifierError,
    },
    #[error("cannot read plugin manifest {path:?}: {message}")]
    ManifestRead { path: PathBuf, message: String },
    #[error("plugin manifest is invalid: {0}")]
    Manifest(#[source] ManifestError),
    #[error("plugin directory {directory_id:?} contains manifest for {manifest_name:?}")]
    ManifestNameMismatch {
        directory_id: String,
        manifest_name: String,
    },
    #[error("plugin assets could not be resolved: {0}")]
    Assets(#[source] AssetError),
    #[error("SVG template {path:?} resolves outside plugin directory {base_dir:?}")]
    TemplateOutsideDirectory { path: PathBuf, base_dir: PathBuf },
    #[error("cannot read SVG template {path:?}: {message}")]
    TemplateRead { path: PathBuf, message: String },
    #[error("SVG template {path:?} is {actual} bytes; the limit is {limit}")]
    TemplateTooLarge {
        path: PathBuf,
        limit: usize,
        actual: usize,
    },
    #[error("SVG template {path:?} is not valid UTF-8")]
    TemplateInvalidUtf8 { path: PathBuf },
}

/// Why a would-be plugin id is unsafe to use as a filesystem component.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PluginIdentifierError {
    #[error("the identifier is empty")]
    Empty,
    #[error("the identifier is {actual} UTF-8 bytes; the limit is {limit}")]
    TooLong { limit: usize, actual: usize },
    #[error("character {character:?} at byte {index} is not lowercase ASCII, a digit, '-' or '_'")]
    InvalidCharacter { index: usize, character: char },
    #[error("the directory name is not valid UTF-8")]
    InvalidUtf8,
}

/// A registry-wide failure, rather than a problem isolated to one plugin.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PluginRegistryError {
    #[error("cannot read plugin base directory {path:?}: {message}")]
    ReadBaseDirectory { path: PathBuf, message: String },
    #[error("cannot inspect an entry in plugin base directory {path:?}: {message}")]
    ReadDirectoryEntry { path: PathBuf, message: String },
    #[error("plugin catalog contains at least {actual} directories; the limit is {limit}")]
    TooManyPlugins { limit: usize, actual: usize },
    #[error(
        "plugin catalog contains {actual} unique assets; the device can retain at most {limit} digests"
    )]
    TooManyUniqueAssets { limit: usize, actual: usize },
}

impl PluginRegistry {
    /// Loads every non-hidden plugin directory directly under `base_dir`.
    ///
    /// Per-plugin parse and asset failures are returned beside the usable
    /// registry. Failure to enumerate the directory, exceeding
    /// [`MAX_PLUGINS`], or exceeding [`MAX_ASSET_DIGESTS`] is registry-wide
    /// and returned as [`PluginRegistryError`].
    pub fn load(base_dir: &Path) -> Result<(Self, Vec<PluginLoadFailure>), PluginRegistryError> {
        let entries =
            fs::read_dir(base_dir).map_err(|error| PluginRegistryError::ReadBaseDirectory {
                path: base_dir.to_path_buf(),
                message: error.to_string(),
            })?;

        let mut candidates = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| PluginRegistryError::ReadDirectoryEntry {
                path: base_dir.to_path_buf(),
                message: error.to_string(),
            })?;
            let file_name = entry.file_name();
            if file_name.to_string_lossy().starts_with('.') {
                continue;
            }
            let file_type =
                entry
                    .file_type()
                    .map_err(|error| PluginRegistryError::ReadDirectoryEntry {
                        path: entry.path(),
                        message: error.to_string(),
                    })?;
            if !file_type.is_dir() {
                continue;
            }

            candidates.push(match file_name.into_string() {
                Ok(id) => PluginCandidate::Named(id),
                Err(id) => PluginCandidate::InvalidUtf8(id.to_string_lossy().into_owned()),
            });
            if candidates.len() > MAX_PLUGINS {
                return Err(PluginRegistryError::TooManyPlugins {
                    limit: MAX_PLUGINS,
                    actual: candidates.len(),
                });
            }
        }

        candidates.sort_by(|left, right| left.id().cmp(right.id()));

        let mut plugins = BTreeMap::new();
        let mut failures = Vec::new();
        for candidate in candidates {
            match candidate {
                PluginCandidate::Named(id) => match load_plugin(base_dir, &id) {
                    Ok(plugin) => {
                        plugins.insert(id, plugin);
                    }
                    Err(error) => failures.push(PluginLoadFailure { id, error }),
                },
                PluginCandidate::InvalidUtf8(id) => failures.push(PluginLoadFailure {
                    error: PluginLoadError::InvalidIdentifier {
                        plugin_id: id.clone(),
                        source: PluginIdentifierError::InvalidUtf8,
                    },
                    id,
                }),
            }
        }

        let registry = Self { plugins };
        let unique_assets = registry.all_assets().len();
        if unique_assets > MAX_DURABLE_REGISTRY_ASSETS {
            return Err(PluginRegistryError::TooManyUniqueAssets {
                limit: MAX_DURABLE_REGISTRY_ASSETS,
                actual: unique_assets,
            });
        }

        Ok((registry, failures))
    }

    /// A registry with no plugins, for a deployment that configures none.
    ///
    /// This exists so "no plugins" costs no filesystem access. Building it by
    /// loading an empty temporary directory would make a server whose temp
    /// directory is unwritable panic at startup over a state that needs no
    /// disk at all.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            plugins: BTreeMap::new(),
        }
    }

    /// Returns a loaded plugin by its directory/manifest id.
    pub fn get(&self, plugin_id: &str) -> Option<&LoadedPlugin> {
        self.plugins.get(plugin_id)
    }

    /// Returns one borrowed asset per digest, sorted by digest.
    ///
    /// The returned values are the original [`ResolvedAsset`] instances, so
    /// their digest and `Arc<[u8]>` retain `plugin::resolve_assets`'s
    /// single-read provenance. No bytes are re-read, re-hashed, or copied.
    pub fn all_assets(&self) -> Vec<&ResolvedAsset> {
        let mut by_digest: BTreeMap<[u8; ASSET_DIGEST_LEN], &ResolvedAsset> = BTreeMap::new();

        for plugin in self.plugins.values() {
            let mut named_assets: Vec<_> = plugin.assets.iter().collect();
            named_assets.sort_by(|left, right| left.0.cmp(right.0));
            for (_, asset) in named_assets {
                by_digest.entry(asset.digest).or_insert(asset);
            }
        }

        by_digest.into_values().collect()
    }

    /// Number of usable plugins in the registry.
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    /// Whether no plugin loaded successfully.
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Loaded plugin ids in deterministic lexicographic order.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.plugins.keys().map(String::as_str)
    }
}

enum PluginCandidate {
    Named(String),
    InvalidUtf8(String),
}

impl PluginCandidate {
    fn id(&self) -> &str {
        match self {
            Self::Named(id) | Self::InvalidUtf8(id) => id,
        }
    }
}

fn validate_plugin_id(plugin_id: &str) -> Result<(), PluginIdentifierError> {
    if plugin_id.trim().is_empty() {
        return Err(PluginIdentifierError::Empty);
    }
    if plugin_id.len() > MAX_PLUGIN_ID_LEN {
        return Err(PluginIdentifierError::TooLong {
            limit: MAX_PLUGIN_ID_LEN,
            actual: plugin_id.len(),
        });
    }
    if let Some((index, character)) = plugin_id.char_indices().find(|(_, character)| {
        !character.is_ascii_lowercase()
            && !character.is_ascii_digit()
            && !matches!(character, '-' | '_')
    }) {
        return Err(PluginIdentifierError::InvalidCharacter { index, character });
    }
    Ok(())
}

/// Runs `load` only after `plugin_id` has passed the path-component guard.
/// Keeping the filesystem operation behind this closure lets the hostile-id
/// test prove it was never invoked, instead of merely inferring that fact
/// from an eventual I/O error.
fn with_validated_plugin_id<F>(plugin_id: &str, load: F) -> Result<LoadedPlugin, PluginLoadError>
where
    F: FnOnce(&str) -> Result<LoadedPlugin, PluginLoadError>,
{
    validate_plugin_id(plugin_id).map_err(|source| PluginLoadError::InvalidIdentifier {
        plugin_id: plugin_id.to_string(),
        source,
    })?;
    load(plugin_id)
}

fn load_plugin(base_dir: &Path, plugin_id: &str) -> Result<LoadedPlugin, PluginLoadError> {
    with_validated_plugin_id(plugin_id, |validated_id| {
        let plugin_dir = base_dir.join(validated_id);
        let manifest_path = plugin_dir.join("manifest.toml");
        let source =
            fs::read_to_string(&manifest_path).map_err(|error| PluginLoadError::ManifestRead {
                path: manifest_path,
                message: error.to_string(),
            })?;
        let manifest = parse_manifest(&source).map_err(PluginLoadError::Manifest)?;
        if manifest.name != validated_id {
            return Err(PluginLoadError::ManifestNameMismatch {
                directory_id: validated_id.to_string(),
                manifest_name: manifest.name,
            });
        }
        let assets = resolve_assets(&manifest, &plugin_dir).map_err(PluginLoadError::Assets)?;
        let svg_source = manifest
            .svg_template_file()
            .map(|file| {
                let path = ensure_within_plugin_dir(&plugin_dir, file)?;
                read_bounded_svg(&path)
            })
            .transpose()?;

        Ok(LoadedPlugin {
            id: validated_id.to_string(),
            manifest,
            assets,
            svg_source,
        })
    })
}

fn ensure_within_plugin_dir(base_dir: &Path, file: &str) -> Result<PathBuf, PluginLoadError> {
    let canonical_base =
        fs::canonicalize(base_dir).map_err(|error| PluginLoadError::TemplateRead {
            path: base_dir.to_path_buf(),
            message: error.to_string(),
        })?;
    let joined = base_dir.join(file);
    let canonical_path =
        fs::canonicalize(&joined).map_err(|error| PluginLoadError::TemplateRead {
            path: joined.clone(),
            message: error.to_string(),
        })?;
    if !canonical_path.starts_with(&canonical_base) {
        return Err(PluginLoadError::TemplateOutsideDirectory {
            path: canonical_path,
            base_dir: canonical_base,
        });
    }
    Ok(canonical_path)
}

fn read_bounded_svg(path: &Path) -> Result<Arc<str>, PluginLoadError> {
    let limit = MAX_SVG_SOURCE_BYTES;
    let file = File::open(path).map_err(|error| PluginLoadError::TemplateRead {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    let take_limit = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    let mut bytes = Vec::with_capacity(limit.min(8 * 1024));
    file.take(take_limit)
        .read_to_end(&mut bytes)
        .map_err(|error| PluginLoadError::TemplateRead {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    if bytes.len() > limit {
        return Err(PluginLoadError::TemplateTooLarge {
            path: path.to_path_buf(),
            limit,
            actual: bytes.len(),
        });
    }
    let source = String::from_utf8(bytes).map_err(|_| PluginLoadError::TemplateInvalidUtf8 {
        path: path.to_path_buf(),
    })?;
    Ok(Arc::from(source))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::fmt::Write as _;

    use super::*;

    fn curated_plugins_dir() -> PathBuf {
        // `CARGO_MANIFEST_DIR` is `companion/crates/server`; curated
        // plugins are repo content under `companion/plugins`, matching the
        // convention in `plugin/tests/curated_plugins.rs`.
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins")
    }

    fn copy_curated_plugin(destination: &Path, plugin_id: &str) {
        let source = curated_plugins_dir().join(plugin_id);
        let target = destination.join(plugin_id);
        fs::create_dir(&target).expect("create copied plugin directory");
        for entry in fs::read_dir(source).expect("read curated plugin directory") {
            let entry = entry.expect("read curated plugin entry");
            if entry.file_type().expect("read curated file type").is_file() {
                fs::copy(entry.path(), target.join(entry.file_name()))
                    .expect("copy curated plugin file");
            }
        }
    }

    fn write_image_plugin(base_dir: &Path, plugin_id: &str, assets: &[(String, Vec<u8>)]) {
        let plugin_dir = base_dir.join(plugin_id);
        fs::create_dir(&plugin_dir).expect("create synthetic plugin directory");
        let mut manifest = format!(
            "name = \"{plugin_id}\"\n\
             version = \"1.0.0\"\n\n\
             [source]\n\
             kind = \"json\"\n\
             url = \"https://example.invalid/{plugin_id}.json\"\n\
             refresh_minutes = 15\n"
        );
        for (file, bytes) in assets {
            write!(
                manifest,
                "\n[[assets]]\nkind = \"image\"\nfile = \"{file}\"\n"
            )
            .expect("write manifest asset entry");
            fs::write(plugin_dir.join(file), bytes).expect("write synthetic asset");
        }
        fs::write(plugin_dir.join("manifest.toml"), manifest).expect("write manifest");
    }

    fn write_svg_plugin(base_dir: &Path, plugin_id: &str, file: &str, bytes: &[u8]) -> PathBuf {
        let plugin_dir = base_dir.join(plugin_id);
        fs::create_dir(&plugin_dir).expect("create SVG plugin directory");
        let manifest = format!(
            "manifest_version = 2\n\
             name = \"{plugin_id}\"\n\
             version = \"1.0.0\"\n\n\
             [source]\n\
             kind = \"json\"\n\
             url = \"https://example.invalid/{plugin_id}.json\"\n\
             refresh_minutes = 15\n\n\
             [template]\n\
             kind = \"svg\"\n\
             file = \"{file}\"\n"
        );
        fs::write(plugin_dir.join("manifest.toml"), manifest).expect("write SVG manifest");
        let svg_path = plugin_dir.join(file);
        fs::write(&svg_path, bytes).expect("write SVG template");
        svg_path
    }

    #[test]
    fn real_curated_plugins_load_with_their_real_assets() {
        let (registry, failures) =
            PluginRegistry::load(&curated_plugins_dir()).expect("load curated plugins");

        assert!(failures.is_empty(), "unexpected failures: {failures:?}");
        assert_eq!(registry.len(), 4);
        assert!(!registry.is_empty());
        assert_eq!(
            registry.ids().collect::<Vec<_>>(),
            ["agenda", "aqi", "claude-limits", "svg-aqi"]
        );
        assert!(
            registry
                .get("aqi")
                .expect("aqi loaded")
                .assets
                .get("icons.ttf")
                .is_some()
        );
        assert!(
            registry
                .get("agenda")
                .expect("agenda loaded")
                .assets
                .get("badge.rgb565")
                .is_some()
        );
        let svg = registry.get("svg-aqi").expect("SVG plugin loaded");
        let retained = svg.svg_source.as_ref().expect("SVG source retained");
        assert!(retained.contains("{{ data.current.aqi }}"));
        let cloned = svg.clone();
        assert!(
            Arc::ptr_eq(
                retained,
                cloned
                    .svg_source
                    .as_ref()
                    .expect("clone retains SVG source")
            ),
            "cloning a loaded plugin must retain the registry's one Arc allocation"
        );
    }

    #[test]
    fn svg_source_one_byte_over_its_cap_is_a_named_per_plugin_failure() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_svg_plugin(
            temp.path(),
            "oversized",
            "face.svg",
            &vec![b'x'; PluginManifest::MAX_SVG_SOURCE_BYTES + 1],
        );

        let (registry, failures) = PluginRegistry::load(temp.path()).expect("load registry");

        assert!(registry.is_empty());
        assert!(matches!(
            failures.as_slice(),
            [PluginLoadFailure {
                error: PluginLoadError::TemplateTooLarge {
                    limit: PluginManifest::MAX_SVG_SOURCE_BYTES,
                    actual,
                    ..
                },
                ..
            }] if *actual == PluginManifest::MAX_SVG_SOURCE_BYTES + 1
        ));
    }

    #[test]
    fn invalid_utf8_svg_is_a_named_per_plugin_failure() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_svg_plugin(temp.path(), "bad-utf8", "face.svg", &[0xff, 0xfe]);

        let (registry, failures) = PluginRegistry::load(temp.path()).expect("load registry");

        assert!(registry.is_empty());
        assert!(matches!(
            failures.as_slice(),
            [PluginLoadFailure {
                error: PluginLoadError::TemplateInvalidUtf8 { .. },
                ..
            }]
        ));
    }

    #[cfg(unix)]
    #[test]
    fn svg_symlink_escaping_the_plugin_directory_is_caught_by_canonical_containment() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("tempdir");
        let outside = tempfile::NamedTempFile::new().expect("outside SVG file");
        fs::write(outside.path(), b"<svg/>").expect("write outside SVG");
        let plugin_dir = temp.path().join("escape");
        fs::create_dir(&plugin_dir).expect("create escaping plugin");
        fs::write(
            plugin_dir.join("manifest.toml"),
            "manifest_version=2\nname=\"escape\"\nversion=\"1.0.0\"\n\
             [source]\nkind=\"json\"\nurl=\"https://example.invalid/x\"\nrefresh_minutes=15\n\
             [template]\nkind=\"svg\"\nfile=\"face.svg\"\n",
        )
        .expect("write escaping manifest");
        symlink(outside.path(), plugin_dir.join("face.svg")).expect("create escaping symlink");

        let (registry, failures) = PluginRegistry::load(temp.path()).expect("load registry");

        assert!(registry.is_empty());
        assert!(matches!(
            failures.as_slice(),
            [PluginLoadFailure {
                error: PluginLoadError::TemplateOutsideDirectory { .. },
                ..
            }]
        ));
    }

    #[test]
    fn get_returns_the_named_manifest_or_none() {
        let (registry, failures) =
            PluginRegistry::load(&curated_plugins_dir()).expect("load curated plugins");
        assert!(failures.is_empty());

        assert_eq!(
            registry.get("aqi").expect("aqi loaded").manifest.name,
            "aqi"
        );
        assert!(registry.get("nope").is_none());
    }

    #[test]
    fn hostile_plugin_ids_are_rejected_before_the_filesystem_loader_runs() {
        let overlong = "a".repeat(MAX_PLUGIN_ID_LEN + 1);
        let hostile_ids = [
            "..".to_string(),
            "../../etc".to_string(),
            "/etc/passwd".to_string(),
            "a/b".to_string(),
            String::new(),
            overlong,
            "AQI".to_string(),
        ];

        for plugin_id in hostile_ids {
            let filesystem_loader_ran = Cell::new(false);
            let error = with_validated_plugin_id(&plugin_id, |_| {
                filesystem_loader_ran.set(true);
                unreachable!("hostile id reached the filesystem loader")
            })
            .expect_err("hostile plugin id must be rejected");

            let PluginLoadError::InvalidIdentifier {
                plugin_id: rejected_id,
                source,
            } = error
            else {
                panic!("{plugin_id:?}: wrong error: {error:?}");
            };
            assert_eq!(rejected_id, plugin_id);
            if plugin_id.is_empty() {
                assert_eq!(source, PluginIdentifierError::Empty);
            } else if plugin_id.len() > MAX_PLUGIN_ID_LEN {
                assert_eq!(
                    source,
                    PluginIdentifierError::TooLong {
                        limit: MAX_PLUGIN_ID_LEN,
                        actual: plugin_id.len(),
                    }
                );
            } else {
                assert!(
                    matches!(source, PluginIdentifierError::InvalidCharacter { .. }),
                    "{plugin_id:?}: wrong identifier error: {source:?}"
                );
            }
            assert!(
                !filesystem_loader_ran.get(),
                "{plugin_id:?}: filesystem loader unexpectedly ran"
            );
        }
    }

    #[test]
    fn malformed_manifest_is_reported_without_blocking_good_plugins() {
        let temp = tempfile::tempdir().expect("tempdir");
        copy_curated_plugin(temp.path(), "aqi");
        copy_curated_plugin(temp.path(), "agenda");
        let broken = temp.path().join("broken");
        fs::create_dir(&broken).expect("create broken plugin");
        fs::write(broken.join("manifest.toml"), "this is not = [valid TOML")
            .expect("write malformed manifest");

        let (registry, failures) = PluginRegistry::load(temp.path()).expect("load registry");

        assert_eq!(registry.ids().collect::<Vec<_>>(), ["agenda", "aqi"]);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].id, "broken");
        assert!(matches!(
            failures[0].error,
            PluginLoadError::Manifest(ManifestError::Toml(_))
        ));
    }

    #[test]
    fn missing_manifest_is_a_per_plugin_failure() {
        let temp = tempfile::tempdir().expect("tempdir");
        copy_curated_plugin(temp.path(), "aqi");
        fs::create_dir(temp.path().join("missing")).expect("create plugin without manifest");

        let (registry, failures) = PluginRegistry::load(temp.path()).expect("load registry");

        assert_eq!(registry.ids().collect::<Vec<_>>(), ["aqi"]);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].id, "missing");
        assert!(matches!(
            failures[0].error,
            PluginLoadError::ManifestRead { .. }
        ));
    }

    #[test]
    fn asset_resolution_failure_is_reported_without_blocking_good_plugins() {
        let temp = tempfile::tempdir().expect("tempdir");
        copy_curated_plugin(temp.path(), "aqi");
        write_image_plugin(
            temp.path(),
            "broken-assets",
            &[("missing.bin".to_string(), vec![1])],
        );
        fs::remove_file(temp.path().join("broken-assets/missing.bin"))
            .expect("remove declared asset");

        let (registry, failures) = PluginRegistry::load(temp.path()).expect("load registry");

        assert_eq!(registry.ids().collect::<Vec<_>>(), ["aqi"]);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].id, "broken-assets");
        assert!(matches!(
            failures[0].error,
            PluginLoadError::Assets(AssetError::Io { .. })
        ));
    }

    #[test]
    fn hidden_and_non_directory_entries_are_skipped_quietly() {
        let temp = tempfile::tempdir().expect("tempdir");
        copy_curated_plugin(temp.path(), "aqi");
        fs::write(temp.path().join(".DS_Store"), b"stray metadata").expect("write dotfile");
        fs::create_dir(temp.path().join(".hidden-plugin")).expect("create hidden directory");
        fs::write(temp.path().join("README"), b"not a plugin").expect("write regular file");

        let (registry, failures) = PluginRegistry::load(temp.path()).expect("load registry");

        assert_eq!(registry.ids().collect::<Vec<_>>(), ["aqi"]);
        assert!(failures.is_empty());
    }

    #[test]
    fn manifest_name_must_match_its_directory_id() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_image_plugin(
            temp.path(),
            "directory-name",
            &[("image.bin".to_string(), vec![1])],
        );
        let manifest_path = temp.path().join("directory-name/manifest.toml");
        let manifest = fs::read_to_string(&manifest_path).expect("read manifest");
        fs::write(
            manifest_path,
            manifest.replacen("name = \"directory-name\"", "name = \"other-name\"", 1),
        )
        .expect("rewrite manifest name");

        let (registry, failures) = PluginRegistry::load(temp.path()).expect("load registry");

        assert!(registry.is_empty());
        assert!(matches!(
            failures.as_slice(),
            [PluginLoadFailure {
                error: PluginLoadError::ManifestNameMismatch { .. },
                ..
            }]
        ));
    }

    #[test]
    fn all_assets_are_deduplicated_by_digest() {
        let temp = tempfile::tempdir().expect("tempdir");
        let shared = vec![1, 2, 3, 4];
        write_image_plugin(
            temp.path(),
            "alpha",
            &[("alpha.bin".to_string(), shared.clone())],
        );
        write_image_plugin(temp.path(), "beta", &[("beta.bin".to_string(), shared)]);

        let (registry, failures) = PluginRegistry::load(temp.path()).expect("load registry");

        assert!(failures.is_empty());
        assert_eq!(registry.len(), 2);
        assert_eq!(registry.all_assets().len(), 1);
    }

    #[test]
    fn all_assets_have_the_same_exact_digest_order_across_independent_loads() {
        let (first, first_failures) =
            PluginRegistry::load(&curated_plugins_dir()).expect("first load");
        let (second, second_failures) =
            PluginRegistry::load(&curated_plugins_dir()).expect("second load");
        assert!(first_failures.is_empty());
        assert!(second_failures.is_empty());

        let mut expected = vec![
            first
                .get("aqi")
                .expect("aqi loaded")
                .assets
                .get("icons.ttf")
                .expect("aqi asset")
                .digest,
            first
                .get("agenda")
                .expect("agenda loaded")
                .assets
                .get("badge.rgb565")
                .expect("agenda asset")
                .digest,
        ];
        expected.sort_unstable();

        let first_order: Vec<_> = first
            .all_assets()
            .iter()
            .map(|asset| asset.digest)
            .collect();
        let second_order: Vec<_> = second
            .all_assets()
            .iter()
            .map(|asset| asset.digest)
            .collect();
        assert_eq!(first_order, expected);
        assert_eq!(second_order, expected);
    }

    #[test]
    fn more_plugin_directories_than_the_startup_cap_is_rejected() {
        let temp = tempfile::tempdir().expect("tempdir");
        for index in 0..=MAX_PLUGINS {
            fs::create_dir(temp.path().join(format!("plugin-{index:02}")))
                .expect("create plugin directory");
        }

        let error = PluginRegistry::load(temp.path()).expect_err("catalog must be bounded");

        assert_eq!(
            error,
            PluginRegistryError::TooManyPlugins {
                limit: MAX_PLUGINS,
                actual: MAX_PLUGINS + 1,
            }
        );
    }

    #[test]
    fn exactly_the_durable_ceiling_loads_and_one_more_is_rejected() {
        // The precise boundary: 31 unique assets is the largest legal
        // registry, because the wire's 32-digest keep-set must also carry
        // stage 4's one active volatile raster digest.
        for (count, expect_ok) in [
            (MAX_DURABLE_REGISTRY_ASSETS, true),
            (MAX_DURABLE_REGISTRY_ASSETS + 1, false),
        ] {
            let temp = tempfile::tempdir().expect("tempdir");
            let mut byte = 1u8;
            let mut remaining = count;
            let mut plugin_index = 0usize;
            while remaining > 0 {
                let batch = remaining.min(plugin::MAX_ASSETS);
                let mut assets = Vec::new();
                for asset_index in 0..batch {
                    assets.push((format!("asset-{asset_index:02}.bin"), vec![byte]));
                    byte = byte.checked_add(1).expect("test digest seed fits in u8");
                }
                write_image_plugin(temp.path(), &format!("plugin-{plugin_index}"), &assets);
                plugin_index += 1;
                remaining -= batch;
            }

            let result = PluginRegistry::load(temp.path());
            if expect_ok {
                let (registry, failures) = result.expect("the ceiling itself must load");
                assert!(failures.is_empty(), "unexpected failures: {failures:?}");
                assert_eq!(registry.all_assets().len(), MAX_DURABLE_REGISTRY_ASSETS);
            } else {
                assert_eq!(
                    result.expect_err("one past the ceiling must be refused"),
                    PluginRegistryError::TooManyUniqueAssets {
                        limit: MAX_DURABLE_REGISTRY_ASSETS,
                        actual: MAX_DURABLE_REGISTRY_ASSETS + 1,
                    }
                );
            }
        }
    }

    #[test]
    fn more_unique_assets_than_the_durable_ceiling_is_rejected() {
        let temp = tempfile::tempdir().expect("tempdir");
        let assets_per_plugin = MAX_ASSET_DIGESTS / 3 + 1;
        let mut byte = 1u8;
        for plugin_index in 0..3 {
            let mut assets = Vec::new();
            for asset_index in 0..assets_per_plugin {
                assets.push((format!("asset-{asset_index:02}.bin"), vec![byte]));
                byte = byte.checked_add(1).expect("test digest seed fits in u8");
            }
            write_image_plugin(temp.path(), &format!("plugin-{plugin_index}"), &assets);
        }
        let expected = assets_per_plugin * 3;
        assert!(expected > MAX_DURABLE_REGISTRY_ASSETS);

        let error = PluginRegistry::load(temp.path()).expect_err("asset union must be bounded");

        assert_eq!(
            error,
            PluginRegistryError::TooManyUniqueAssets {
                limit: MAX_DURABLE_REGISTRY_ASSETS,
                actual: expected,
            }
        );
    }
}
