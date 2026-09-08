//! Minimal plugin directories built in a tempdir, for tests that must assert
//! how a manifest field is *threaded* rather than what a curated plugin
//! happens to say today. Editing `plugins/aqi/manifest.toml` must never move
//! one of these assertions.

use std::sync::Arc;

use crate::plugin_registry::PluginRegistry;

/// A v1 document: no `manifest_version`, and none of v2's three optional keys.
pub(crate) const V1_MANIFEST: &str = r#"
name = "fixture"
version = "0.1.0"

[source]
kind = "json"
url = "https://example.invalid/fixture.json"
refresh_minutes = 7

[[nodes]]
kind = "text"
x = 24
baseline_y = 56
w = 300
align = "left"
font = { tier = "caption" }
color = 0x5C5C66
value = "{{ data.current.aqi }}"
"#;

/// A v2 document declaring all three optional keys.
pub(crate) const V2_NAMED_MANIFEST: &str = r#"
manifest_version = 2
name = "fixture"
version = "0.1.0"
display_name = "Fixture plugin"
description = "Names and cadence, threaded from the manifest"
summary = "{{ data.current.aqi }}"

[source]
kind = "json"
url = "https://example.invalid/fixture.json"
refresh_minutes = 7

[template]
kind = "scene"

[[nodes]]
kind = "text"
x = 24
baseline_y = 56
w = 300
align = "left"
font = { tier = "caption" }
color = 0x5C5C66
value = "{{ data.current.aqi }}"
"#;

/// A v2 document declaring none of them: absence is tolerated, per §3.
pub(crate) const V2_UNNAMED_MANIFEST: &str = r#"
manifest_version = 2
name = "fixture"
version = "0.1.0"

[source]
kind = "json"
url = "https://example.invalid/fixture.json"
refresh_minutes = 7

[template]
kind = "scene"

[[nodes]]
kind = "text"
x = 24
baseline_y = 56
w = 300
align = "left"
font = { tier = "caption" }
color = 0x5C5C66
value = "{{ data.current.aqi }}"
"#;

/// Loads a one-plugin registry whose id is `fixture`. The returned tempdir
/// must outlive the registry: dropping it deletes the directory.
pub(crate) fn fixture_registry(manifest: &str) -> (Arc<PluginRegistry>, tempfile::TempDir) {
    let base = tempfile::tempdir().expect("plugin fixture base directory");
    std::fs::create_dir(base.path().join("fixture")).expect("plugin fixture directory");
    std::fs::write(base.path().join("fixture/manifest.toml"), manifest)
        .expect("write fixture manifest");
    let (registry, failures) =
        PluginRegistry::load(base.path()).expect("load the fixture registry");
    assert!(failures.is_empty(), "unexpected failures: {failures:?}");
    (Arc::new(registry), base)
}
