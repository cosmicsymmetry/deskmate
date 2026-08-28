//! TOML -> [`PluginManifest`], typed and bounded.
//!
//! Every manifest is untrusted input: it may arrive from a plugin author who
//! made a typo, or -- once uploads exist in a later stage -- from someone
//! actively hostile. This module's only job is turning manifest source into
//! a typed, bounded value or a named [`ManifestError`]; it never panics,
//! never allocates without a bound in front of it, and never blocks.
//!
//! # What this module does not do
//!
//! It does not parse the `{{ ... }}` expression syntax that appears inside a
//! `value` or `glyph` field. That string is stored as bounded, opaque text
//! -- length-checked, never interpreted -- because a later task owns the
//! expression grammar. It also does not resolve assets to digests (a later
//! task) or compile a manifest into a `Scene` (also a later task); it does
//! not validate node geometry against the 448x368 canvas, because
//! `protocol::validate_scene` is where that bound already lives and the
//! compiler that produces a real `Scene` is what calls it.

use std::collections::HashSet;
use std::fmt;

use serde::Deserialize;

// ---------------------------------------------------------------------------
// Bounds. Each protects a specific untrusted-input hazard; each has a test
// proving a value one past it is rejected by name.
// ---------------------------------------------------------------------------

/// Maximum size, in bytes, of a manifest's raw TOML source. Checked before
/// any parsing happens, so an oversized upload costs a length comparison
/// rather than a parse attempt.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;

/// Maximum number of `[[nodes]]` entries a manifest may declare. This is not
/// restated: it *is* `protocol::MAX_SCENE_NODES`, the wire's own node
/// ceiling, imported rather than copied so the two cannot drift apart.
pub const MAX_NODES: usize = protocol::MAX_SCENE_NODES;

/// Maximum number of `[[assets]]` entries per manifest.
pub const MAX_ASSETS: usize = 16;

/// Maximum number of glyph entries in a single `icon-font` asset.
pub const MAX_GLYPHS_PER_ICON_FONT: usize = 256;

/// Maximum byte length of an opaque expression source string -- the
/// `"{{ ... }}"` text carried by a `text` node's `value` or a `glyph`
/// node's `glyph` field. This is a lexical bound only: Task 1 does not
/// parse expression syntax, so it cannot bound anything about what the
/// expression *does*, only how long its unparsed source may be before it
/// reaches the parser that does.
pub const MAX_EXPR_SOURCE_LEN: usize = 256;

/// Lower bound on `source.refresh_minutes`. Zero would mean "refresh
/// continuously," which is a self-inflicted denial of service against the
/// plugin's own upstream and against the server's egress guard.
pub const MIN_REFRESH_MINUTES: u32 = 1;
/// Upper bound on `source.refresh_minutes`: one day.
pub const MAX_REFRESH_MINUTES: u32 = 24 * 60;

/// Maximum byte length of `name`.
pub const MAX_NAME_LEN: usize = 64;
/// Maximum byte length of `version`.
pub const MAX_VERSION_LEN: usize = 32;
/// Maximum byte length of an asset's `file` name.
pub const MAX_FILE_NAME_LEN: usize = 128;
/// Maximum byte length of `source.url`.
pub const MAX_URL_LEN: usize = 512;

/// The only scheme a plugin's data source URL may declare. This is the
/// lexical half of keeping a plugin off the server's own network; the
/// remaining SSRF surface -- private, loopback, and link-local
/// destinations, and the DNS-rebind window between check and connect -- is
/// a later task's egress guard, which runs at fetch time rather than parse
/// time because the destination address is not known until DNS resolves.
pub const ALLOWED_URL_SCHEME: &str = "https";

/// Minimum points a `line` node's fixed form may declare.
pub const MIN_LINE_POINTS: usize = 2;
/// Maximum points a `line` node's fixed form may declare. Reused from the
/// wire's own ceiling for the same reason `MAX_NODES` is.
pub const MAX_LINE_POINTS: usize = protocol::MAX_SCENE_LINE_POINTS;

/// Maximum nesting depth of TOML inline tables/arrays (`{`/`[`), checked by
/// a cheap linear pre-scan before any TOML parsing happens.
///
/// This exists because `toml`'s own parser is recursive, and its own
/// recursion limit is not a reliable backstop: a debug build overflows the
/// thread stack (an uncatchable `SIGABRT`, proved by
/// `a_deeply_nested_table_is_rejected_by_name_without_panicking`) at a nesting
/// depth several orders of magnitude below where the crate's error path
/// takes over in a release build. No manifest this format can express needs
/// more than a handful of levels (an icon-font's `glyphs` array of inline
/// tables is the deepest legitimate case, at 2), so this bound is generous
/// and still cheap insurance.
pub const MAX_TOML_NESTING_DEPTH: usize = 16;

const fn default_opacity() -> u8 {
    u8::MAX
}

// ---------------------------------------------------------------------------
// Errors: one variant per rejection reason.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    /// The raw manifest source exceeds [`MAX_MANIFEST_BYTES`].
    TooLarge { limit: usize, actual: usize },
    /// `parse_manifest_bytes` was given bytes that are not valid UTF-8.
    InvalidUtf8,
    /// TOML syntax error, an unknown field (`deny_unknown_fields`), a
    /// missing required field, an unknown `kind` tag, or any other
    /// structural mismatch the `toml`/`serde` layer reports. The message is
    /// preserved for diagnostics; it is not a bound this crate owns, so it
    /// is not itself a named reason.
    Toml(String),
    /// `nodes` exceeds [`MAX_NODES`].
    TooManyNodes { limit: usize, actual: usize },
    /// `assets` exceeds [`MAX_ASSETS`].
    TooManyAssets { limit: usize, actual: usize },
    /// An `icon-font` asset's `glyphs` exceeds [`MAX_GLYPHS_PER_ICON_FONT`].
    TooManyGlyphs {
        asset: String,
        limit: usize,
        actual: usize,
    },
    /// Two `[[assets]]` entries declare the same `file`.
    DuplicateAsset { file: String },
    /// A glyph's `codepoint` is not a valid Unicode scalar value: it falls
    /// in the UTF-16 surrogate range (`0xD800..=0xDFFF`) or exceeds
    /// `0x10FFFF`.
    InvalidGlyphCodepoint {
        asset: String,
        name: String,
        codepoint: u32,
    },
    /// `source.refresh_minutes` is outside
    /// `MIN_REFRESH_MINUTES..=MAX_REFRESH_MINUTES`.
    InvalidRefreshMinutes { value: u32 },
    /// `source.url` does not parse as a URL at all.
    InvalidUrl { message: String },
    /// `source.url`'s scheme is not [`ALLOWED_URL_SCHEME`] -- for example
    /// `file://`, which would let a manifest read the server's filesystem
    /// rather than fetch a remote resource.
    DisallowedUrlScheme { scheme: String },
    /// A `line` node's `points` count falls outside
    /// `MIN_LINE_POINTS..=MAX_LINE_POINTS`.
    InvalidPointCount {
        min: usize,
        max: usize,
        actual: usize,
    },
    /// The raw source has inline table/array nesting deeper than
    /// [`MAX_TOML_NESTING_DEPTH`], caught before the TOML parser ever runs.
    TooDeeplyNested { limit: usize },
    /// A bounded string field exceeded its limit. Covers `name`, `version`,
    /// an asset's `file`, `source.url`, a glyph's `name`, and a node's
    /// expression-source field (`value` or `glyph`) -- one reason, many
    /// fields, so `field` names which.
    StringTooLong {
        field: &'static str,
        limit: usize,
        actual: usize,
    },
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for ManifestError {}

// ---------------------------------------------------------------------------
// The manifest shape.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub source: Source,
    #[serde(default)]
    pub assets: Vec<Asset>,
    #[serde(default)]
    pub nodes: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Source {
    Json { url: String, refresh_minutes: u32 },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Asset {
    /// A plain TTF/OTF blob, sized per text node at render time.
    Font { file: String },
    /// A TTF/OTF blob plus a name-to-codepoint map; `icon(name)` resolves
    /// through this table.
    IconFont { file: String, glyphs: Vec<Glyph> },
    /// A source image, converted server-side to the device's native image
    /// format.
    Image { file: String },
}

impl Asset {
    fn file(&self) -> &str {
        match self {
            Self::Font { file } | Self::IconFont { file, .. } | Self::Image { file } => file,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Glyph {
    pub name: String,
    pub codepoint: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FontTier {
    Caption,
    Body,
    Display,
    Hero,
}

/// A `text` or `glyph` node selects its typeface either from the device's
/// baked tiers or from an asset named in `[[assets]]`. This mirrors
/// `protocol::SceneFont`'s two-variant shape but is not that type: it names
/// a manifest asset by filename, which only a later task's asset resolver
/// can turn into the digest the wire actually carries.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum Font {
    Tier { tier: FontTier },
    Asset { asset: String, pixel_size: i32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

/// A scene node, in the six-kind vocabulary the design spec names as
/// plugin-authorable (§2): `rect`, `arc`, `line`, `text`, `image`, `glyph`.
/// The wire has three more kinds (`scale`, `label`, `rotrect`) used only by
/// hand-written builder code; the manifest does not expose them.
///
/// Fields are typed and absolute, on the 448x368 canvas -- the manifest is
/// a display list, not a layout language. `value` (on `text`) and `glyph`
/// (on `glyph`) are the two fields that may carry `"{{ ... }}"` expression
/// source; every other field is a literal.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Node {
    Rect {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        radius: i32,
        fill: u32,
        #[serde(default = "default_opacity")]
        opacity: u8,
    },
    Arc {
        cx: i32,
        cy: i32,
        r: i32,
        start_deg: i32,
        end_deg: i32,
        width: i32,
        color: u32,
        #[serde(default)]
        caps: bool,
    },
    Line {
        points: Vec<Point>,
        width: i32,
        color: u32,
    },
    Text {
        x: i32,
        baseline_y: i32,
        w: i32,
        #[serde(default)]
        align: Align,
        font: Font,
        color: u32,
        value: String,
        #[serde(default)]
        ellipsize: bool,
    },
    Image {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        asset: String,
        #[serde(default)]
        recolor: bool,
        #[serde(default)]
        color: u32,
    },
    Glyph {
        x: i32,
        y: i32,
        font: Font,
        color: u32,
        glyph: String,
    },
}

// ---------------------------------------------------------------------------
// Validation.
// ---------------------------------------------------------------------------

/// Walks `source` byte-by-byte, tracking inline table/array nesting depth
/// while skipping over TOML string content (basic, literal, and
/// triple-quoted forms) and comments, so a `"{{ ... }}"` expression source
/// or a `#` comment does not itself count as nesting. Never panics: every
/// slice access is bounds-checked, and a malformed or unterminated string
/// simply scans to the end of input rather than indexing past it.
///
/// This is deliberately not a full TOML lexer -- it only needs to be
/// conservative about depth, not byte-perfect about syntax, because its one
/// job is standing between untrusted bytes and the real parser's recursion.
fn check_nesting_depth(source: &str) -> Result<(), ManifestError> {
    let bytes = source.as_bytes();
    let mut i = 0;
    let mut depth: usize = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'#' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'"' => {
                i = skip_string(bytes, i, b'"');
                continue;
            }
            b'\'' => {
                i = skip_string(bytes, i, b'\'');
                continue;
            }
            b'{' | b'[' => {
                depth += 1;
                if depth > MAX_TOML_NESTING_DEPTH {
                    return Err(ManifestError::TooDeeplyNested {
                        limit: MAX_TOML_NESTING_DEPTH,
                    });
                }
            }
            b'}' | b']' => {
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
        i += 1;
    }
    Ok(())
}

/// Skips a TOML string starting at `bytes[start]`, which must be `quote`.
/// Handles the triple-quoted multiline form and, for the single-quote
/// (basic string) form, backslash escapes. Returns the index just past the
/// string, or `bytes.len()` if it is unterminated.
fn skip_string(bytes: &[u8], start: usize, quote: u8) -> usize {
    let mut i = start + 1;
    let triple = bytes[i..].starts_with(&[quote, quote]);
    if triple {
        i += 2;
        let close = [quote, quote, quote];
        while i + 3 <= bytes.len() && bytes[i..i + 3] != close {
            i += 1;
        }
        return (i + 3).min(bytes.len());
    }
    while i < bytes.len() && bytes[i] != quote {
        if quote == b'"' && bytes[i] == b'\\' {
            i += 1;
        }
        i += 1;
    }
    (i + 1).min(bytes.len())
}

fn check_len(field: &'static str, value: &str, limit: usize) -> Result<(), ManifestError> {
    if value.len() > limit {
        Err(ManifestError::StringTooLong {
            field,
            limit,
            actual: value.len(),
        })
    } else {
        Ok(())
    }
}

impl PluginManifest {
    fn validate(&self) -> Result<(), ManifestError> {
        check_len("name", &self.name, MAX_NAME_LEN)?;
        check_len("version", &self.version, MAX_VERSION_LEN)?;
        self.source.validate()?;

        if self.assets.len() > MAX_ASSETS {
            return Err(ManifestError::TooManyAssets {
                limit: MAX_ASSETS,
                actual: self.assets.len(),
            });
        }
        let mut seen_files = HashSet::with_capacity(self.assets.len());
        for asset in &self.assets {
            let file = asset.file();
            check_len("asset.file", file, MAX_FILE_NAME_LEN)?;
            if !seen_files.insert(file) {
                return Err(ManifestError::DuplicateAsset {
                    file: file.to_string(),
                });
            }
            asset.validate()?;
        }

        if self.nodes.len() > MAX_NODES {
            return Err(ManifestError::TooManyNodes {
                limit: MAX_NODES,
                actual: self.nodes.len(),
            });
        }
        for node in &self.nodes {
            node.validate()?;
        }
        Ok(())
    }
}

impl Source {
    fn validate(&self) -> Result<(), ManifestError> {
        match self {
            Self::Json {
                url,
                refresh_minutes,
            } => {
                check_len("source.url", url, MAX_URL_LEN)?;
                if !(MIN_REFRESH_MINUTES..=MAX_REFRESH_MINUTES).contains(refresh_minutes) {
                    return Err(ManifestError::InvalidRefreshMinutes {
                        value: *refresh_minutes,
                    });
                }
                let parsed = url::Url::parse(url).map_err(|err| ManifestError::InvalidUrl {
                    message: err.to_string(),
                })?;
                if parsed.scheme() != ALLOWED_URL_SCHEME {
                    return Err(ManifestError::DisallowedUrlScheme {
                        scheme: parsed.scheme().to_string(),
                    });
                }
                Ok(())
            }
        }
    }
}

impl Asset {
    fn validate(&self) -> Result<(), ManifestError> {
        if let Self::IconFont { file, glyphs } = self {
            if glyphs.len() > MAX_GLYPHS_PER_ICON_FONT {
                return Err(ManifestError::TooManyGlyphs {
                    asset: file.clone(),
                    limit: MAX_GLYPHS_PER_ICON_FONT,
                    actual: glyphs.len(),
                });
            }
            for glyph in glyphs {
                check_len(
                    "glyph.name",
                    &glyph.name,
                    protocol::MAX_SCENE_GLYPH_NAME_LEN,
                )?;
                if char::from_u32(glyph.codepoint).is_none() {
                    return Err(ManifestError::InvalidGlyphCodepoint {
                        asset: file.clone(),
                        name: glyph.name.clone(),
                        codepoint: glyph.codepoint,
                    });
                }
            }
        }
        Ok(())
    }
}

impl Node {
    fn validate(&self) -> Result<(), ManifestError> {
        match self {
            Self::Text { value, .. } => check_len("node.value", value, MAX_EXPR_SOURCE_LEN),
            Self::Glyph { glyph, .. } => check_len("node.glyph", glyph, MAX_EXPR_SOURCE_LEN),
            Self::Line { points, .. } => {
                if (MIN_LINE_POINTS..=MAX_LINE_POINTS).contains(&points.len()) {
                    Ok(())
                } else {
                    Err(ManifestError::InvalidPointCount {
                        min: MIN_LINE_POINTS,
                        max: MAX_LINE_POINTS,
                        actual: points.len(),
                    })
                }
            }
            Self::Rect { .. } | Self::Arc { .. } | Self::Image { .. } => Ok(()),
        }
    }
}

// ---------------------------------------------------------------------------
// Entry points.
// ---------------------------------------------------------------------------

/// Parses and bounds a plugin manifest from TOML source.
///
/// # Errors
///
/// Returns a [`ManifestError`] naming the first rejection reason found: the
/// source is oversized, it is not valid TOML for this shape (including an
/// unknown field, since every table `#[serde(deny_unknown_fields)]`s), or it
/// violates one of this module's named bounds.
pub fn parse_manifest(source: &str) -> Result<PluginManifest, ManifestError> {
    if source.len() > MAX_MANIFEST_BYTES {
        return Err(ManifestError::TooLarge {
            limit: MAX_MANIFEST_BYTES,
            actual: source.len(),
        });
    }
    check_nesting_depth(source)?;
    let manifest: PluginManifest =
        toml::from_str(source).map_err(|err| ManifestError::Toml(err.to_string()))?;
    manifest.validate()?;
    Ok(manifest)
}

/// As [`parse_manifest`], but accepts raw bytes rather than a `&str`. A
/// manifest that reaches this crate started as bytes -- read from disk or
/// received over a network -- and nothing upstream has necessarily checked
/// that those bytes are valid UTF-8.
///
/// # Errors
///
/// Returns [`ManifestError::InvalidUtf8`] if `bytes` is not valid UTF-8,
/// checked before the size bound so a non-UTF-8 payload is rejected for the
/// right reason even when it is also oversized. Otherwise, see
/// [`parse_manifest`].
pub fn parse_manifest_bytes(bytes: &[u8]) -> Result<PluginManifest, ManifestError> {
    let source = std::str::from_utf8(bytes).map_err(|_| ManifestError::InvalidUtf8)?;
    parse_manifest(source)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_HEADER: &str = r#"
name = "aqi"
version = "1.0.0"

[source]
kind = "json"
url = "https://example.invalid/aqi.json"
refresh_minutes = 15

"#;

    const MINIMAL_TEXT_NODE: &str = r#"
[[nodes]]
kind = "text"
x = 24
baseline_y = 96
w = 400
align = "center"
font = { tier = "hero" }
color = 0xFFFFFF
value = "static"

"#;

    const CANONICAL_EXAMPLE: &str = r#"
name = "aqi"
version = "1.0.0"

[source]
kind = "json"
url = "https://example.invalid/aqi.json"
refresh_minutes = 15

[[assets]]
kind = "icon-font"
file = "weather-icons.ttf"
glyphs = [
  { name = "haze", codepoint = 0xE001 },
  { name = "clear", codepoint = 0xE002 },
]

[[nodes]]
kind = "text"
x = 24
baseline_y = 96
w = 400
align = "center"
font = { tier = "hero" }
color = 0xFFFFFF
value = "{{ data.aqi }}"

[[nodes]]
kind = "glyph"
x = 200
y = 180
font = { asset = "weather-icons.ttf", pixel_size = 64 }
color = 0x8AB4F8
glyph = "{{ icon(data.category) }}"
"#;

    fn node_block(nodes: impl Iterator<Item = &'static str>) -> String {
        nodes.collect()
    }

    #[test]
    fn the_canonical_example_parses() {
        let manifest = parse_manifest(CANONICAL_EXAMPLE).expect("canonical example must parse");
        assert_eq!(manifest.name, "aqi");
        assert_eq!(manifest.assets.len(), 1);
        assert_eq!(manifest.nodes.len(), 2);
    }

    #[test]
    fn a_manifest_over_the_node_ceiling_is_rejected_by_name() {
        let nodes = node_block((0..=MAX_NODES).map(|_| MINIMAL_TEXT_NODE));
        let err = parse_manifest(&format!("{MINIMAL_HEADER}{nodes}")).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::TooManyNodes {
                limit: MAX_NODES,
                ..
            }
        ));
    }

    #[test]
    fn a_manifest_at_the_node_ceiling_is_accepted() {
        let nodes = node_block((0..MAX_NODES).map(|_| MINIMAL_TEXT_NODE));
        parse_manifest(&format!("{MINIMAL_HEADER}{nodes}")).expect("exactly MAX_NODES must pass");
    }

    #[test]
    fn a_manifest_over_the_byte_ceiling_is_rejected_by_name() {
        let padding = "x".repeat(MAX_MANIFEST_BYTES + 1 - MINIMAL_HEADER.len());
        let source = format!("{MINIMAL_HEADER}# {padding}\nnodes = []\n");
        assert!(source.len() > MAX_MANIFEST_BYTES);
        let err = parse_manifest(&source).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::TooLarge {
                limit: MAX_MANIFEST_BYTES,
                ..
            }
        ));
    }

    #[test]
    fn a_ten_megabyte_manifest_is_rejected_without_parsing() {
        let huge = "x".repeat(10 * 1024 * 1024);
        let err = parse_manifest(&huge).unwrap_err();
        assert!(matches!(err, ManifestError::TooLarge { .. }));
    }

    #[test]
    fn too_many_assets_is_rejected_by_name() {
        use std::fmt::Write as _;

        let mut assets = String::new();
        for i in 0..=MAX_ASSETS {
            let _ = write!(
                assets,
                "[[assets]]\nkind = \"font\"\nfile = \"f{i}.ttf\"\n\n"
            );
        }
        let err = parse_manifest(&format!("{MINIMAL_HEADER}{assets}")).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::TooManyAssets {
                limit: MAX_ASSETS,
                ..
            }
        ));
    }

    #[test]
    fn duplicate_asset_file_names_are_rejected_by_name() {
        let source = format!(
            "{MINIMAL_HEADER}\
             [[assets]]\nkind = \"font\"\nfile = \"same.ttf\"\n\n\
             [[assets]]\nkind = \"font\"\nfile = \"same.ttf\"\n"
        );
        let err = parse_manifest(&source).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::DuplicateAsset { file } if file == "same.ttf"
        ));
    }

    #[test]
    fn too_many_glyphs_in_one_icon_font_is_rejected_by_name() {
        use std::fmt::Write as _;

        let mut glyphs = String::new();
        for i in 0..=MAX_GLYPHS_PER_ICON_FONT {
            if i > 0 {
                glyphs.push(',');
            }
            let _ = write!(glyphs, "{{ name = \"g{i}\", codepoint = 0xE{i:03X} }}");
        }
        let source = format!(
            "{MINIMAL_HEADER}\
             [[assets]]\nkind = \"icon-font\"\nfile = \"icons.ttf\"\nglyphs = [{glyphs}]\n"
        );
        let err = parse_manifest(&source).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::TooManyGlyphs {
                limit: MAX_GLYPHS_PER_ICON_FONT,
                ..
            }
        ));
    }

    #[test]
    fn a_surrogate_range_codepoint_is_rejected_by_name() {
        let source = format!(
            "{MINIMAL_HEADER}\
             [[assets]]\nkind = \"icon-font\"\nfile = \"icons.ttf\"\n\
             glyphs = [ {{ name = \"bad\", codepoint = 0xD800 }} ]\n"
        );
        let err = parse_manifest(&source).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::InvalidGlyphCodepoint {
                codepoint: 0xD800,
                ..
            }
        ));
    }

    #[test]
    fn a_codepoint_above_the_unicode_ceiling_is_rejected_by_name() {
        let source = format!(
            "{MINIMAL_HEADER}\
             [[assets]]\nkind = \"icon-font\"\nfile = \"icons.ttf\"\n\
             glyphs = [ {{ name = \"bad\", codepoint = 0x110000 }} ]\n"
        );
        let err = parse_manifest(&source).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::InvalidGlyphCodepoint {
                codepoint: 0x0011_0000,
                ..
            }
        ));
    }

    #[test]
    fn a_valid_codepoint_at_the_unicode_ceiling_is_accepted() {
        let source = format!(
            "{MINIMAL_HEADER}\
             [[assets]]\nkind = \"icon-font\"\nfile = \"icons.ttf\"\n\
             glyphs = [ {{ name = \"ok\", codepoint = 0x10FFFF }} ]\n"
        );
        parse_manifest(&source).expect("0x10FFFF is the last valid scalar value");
    }

    #[test]
    fn zero_refresh_minutes_is_rejected_by_name() {
        let source = r#"
name = "aqi"
version = "1.0.0"

[source]
kind = "json"
url = "https://example.invalid/aqi.json"
refresh_minutes = 0
"#;
        let err = parse_manifest(source).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::InvalidRefreshMinutes { value: 0 }
        ));
    }

    #[test]
    fn refresh_minutes_over_the_ceiling_is_rejected_by_name() {
        let source = format!(
            r#"
name = "aqi"
version = "1.0.0"

[source]
kind = "json"
url = "https://example.invalid/aqi.json"
refresh_minutes = {}
"#,
            MAX_REFRESH_MINUTES + 1
        );
        let err = parse_manifest(&source).unwrap_err();
        assert!(matches!(err, ManifestError::InvalidRefreshMinutes { .. }));
    }

    #[test]
    fn a_file_scheme_url_is_rejected_by_name() {
        let source = r#"
name = "aqi"
version = "1.0.0"

[source]
kind = "json"
url = "file:///etc/passwd"
refresh_minutes = 15
"#;
        let err = parse_manifest(source).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::DisallowedUrlScheme { scheme } if scheme == "file"
        ));
    }

    #[test]
    fn a_malformed_url_is_rejected_by_name() {
        let source = r#"
name = "aqi"
version = "1.0.0"

[source]
kind = "json"
url = "not a url"
refresh_minutes = 15
"#;
        let err = parse_manifest(source).unwrap_err();
        assert!(matches!(err, ManifestError::InvalidUrl { .. }));
    }

    #[test]
    fn non_utf8_bytes_are_rejected_by_name() {
        let bytes = [0x6e, 0x61, 0x6d, 0x65, 0xff, 0xfe, 0x00];
        let err = parse_manifest_bytes(&bytes).unwrap_err();
        assert!(matches!(err, ManifestError::InvalidUtf8));
    }

    #[test]
    fn an_unknown_top_level_field_is_rejected() {
        // `bogus` must appear before the `[source]` table header, or TOML
        // attributes it to `source` instead of the manifest root.
        let source = r#"
name = "aqi"
version = "1.0.0"
bogus = true

[source]
kind = "json"
url = "https://example.invalid/aqi.json"
refresh_minutes = 15
"#;
        let err = parse_manifest(source).unwrap_err();
        assert!(matches!(err, ManifestError::Toml(_)));
    }

    #[test]
    fn a_deeply_nested_table_is_rejected_by_name_without_panicking() {
        // Depth chosen to be far beyond MAX_TOML_NESTING_DEPTH: at this
        // depth, calling `toml::from_str` directly overflows the stack (an
        // uncatchable SIGABRT) in a debug build, well before the crate's own
        // recursion guard would return an error. The pre-scan must catch
        // this before the parser ever sees it.
        let depth = 5_000;
        let mut source = String::from("x = ");
        for _ in 0..depth {
            source.push_str("{a=");
        }
        source.push('1');
        for _ in 0..depth {
            source.push('}');
        }
        let err = parse_manifest(&source).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::TooDeeplyNested {
                limit: MAX_TOML_NESTING_DEPTH
            }
        ));
    }

    #[test]
    fn nesting_at_the_depth_ceiling_reaches_the_toml_parser() {
        let mut source = String::from("x = ");
        for _ in 0..MAX_TOML_NESTING_DEPTH {
            source.push_str("{a=");
        }
        source.push('1');
        for _ in 0..MAX_TOML_NESTING_DEPTH {
            source.push('}');
        }
        // Not a valid PluginManifest shape, but it must get past the depth
        // guard and fail as a structural TOML error instead.
        let err = parse_manifest(&source).unwrap_err();
        assert!(matches!(err, ManifestError::Toml(_)));
    }

    #[test]
    fn braces_inside_an_expression_string_do_not_count_as_nesting() {
        // "{{ data.aqi }}" must not be mistaken for four levels of nesting.
        parse_manifest(CANONICAL_EXAMPLE).expect("mustache braces inside a string are not nesting");
    }

    #[test]
    fn an_expression_source_over_the_length_ceiling_is_rejected_by_name() {
        let long_value = "a".repeat(MAX_EXPR_SOURCE_LEN + 1);
        let node = format!(
            "[[nodes]]\nkind = \"text\"\nx = 0\nbaseline_y = 0\nw = 10\n\
             font = {{ tier = \"body\" }}\ncolor = 0\nvalue = \"{long_value}\"\n"
        );
        let err = parse_manifest(&format!("{MINIMAL_HEADER}{node}")).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::StringTooLong {
                field: "node.value",
                ..
            }
        ));
    }

    #[test]
    fn a_glyph_expression_over_the_length_ceiling_is_rejected_by_name() {
        let long_glyph = "a".repeat(MAX_EXPR_SOURCE_LEN + 1);
        let node = format!(
            "[[nodes]]\nkind = \"glyph\"\nx = 0\ny = 0\n\
             font = {{ asset = \"icons.ttf\", pixel_size = 32 }}\ncolor = 0\n\
             glyph = \"{long_glyph}\"\n"
        );
        let err = parse_manifest(&format!("{MINIMAL_HEADER}{node}")).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::StringTooLong {
                field: "node.glyph",
                ..
            }
        ));
    }

    #[test]
    fn too_few_line_points_is_rejected_by_name() {
        let node =
            "[[nodes]]\nkind = \"line\"\npoints = [ { x = 0, y = 0 } ]\nwidth = 1\ncolor = 0\n";
        let err = parse_manifest(&format!("{MINIMAL_HEADER}{node}")).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::InvalidPointCount {
                min: MIN_LINE_POINTS,
                ..
            }
        ));
    }

    #[test]
    fn too_many_line_points_is_rejected_by_name() {
        let points: String = (0..=MAX_LINE_POINTS)
            .map(|i| format!("{{ x = {i}, y = {i} }}"))
            .collect::<Vec<_>>()
            .join(", ");
        let node =
            format!("[[nodes]]\nkind = \"line\"\npoints = [{points}]\nwidth = 1\ncolor = 0\n");
        let err = parse_manifest(&format!("{MINIMAL_HEADER}{node}")).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::InvalidPointCount {
                max: MAX_LINE_POINTS,
                ..
            }
        ));
    }

    #[test]
    fn a_name_over_the_length_ceiling_is_rejected_by_name() {
        let long_name = "a".repeat(MAX_NAME_LEN + 1);
        let source = format!(
            "name = \"{long_name}\"\nversion = \"1.0.0\"\n\n\
             [source]\nkind = \"json\"\nurl = \"https://example.invalid/x.json\"\n\
             refresh_minutes = 15\n"
        );
        let err = parse_manifest(&source).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::StringTooLong { field: "name", .. }
        ));
    }

    #[test]
    fn a_url_over_the_length_ceiling_is_rejected_by_name() {
        // Built to land at exactly MAX_URL_LEN + 1, not just "well past" the
        // bound, so this proves the edge rather than the region.
        let prefix = "https://example.invalid/";
        let long_path = "a".repeat(MAX_URL_LEN + 1 - prefix.len());
        let url = format!("{prefix}{long_path}");
        assert_eq!(url.len(), MAX_URL_LEN + 1);
        let source = format!(
            "{MINIMAL_HEADER_PREFIX}\nurl = \"{url}\"\n\
             refresh_minutes = 15\n",
            MINIMAL_HEADER_PREFIX =
                "name = \"aqi\"\nversion = \"1.0.0\"\n\n[source]\nkind = \"json\""
        );
        let err = parse_manifest(&source).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::StringTooLong {
                field: "source.url",
                limit: MAX_URL_LEN,
                actual
            } if actual == MAX_URL_LEN + 1
        ));
    }

    #[test]
    fn a_version_over_the_length_ceiling_is_rejected_by_name() {
        let long_version = "a".repeat(MAX_VERSION_LEN + 1);
        let source = format!(
            "name = \"aqi\"\nversion = \"{long_version}\"\n\n\
             [source]\nkind = \"json\"\nurl = \"https://example.invalid/x.json\"\n\
             refresh_minutes = 15\n"
        );
        let err = parse_manifest(&source).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::StringTooLong {
                field: "version",
                limit: MAX_VERSION_LEN,
                actual
            } if actual == MAX_VERSION_LEN + 1
        ));
    }

    #[test]
    fn an_asset_file_name_over_the_length_ceiling_is_rejected_by_name() {
        let long_file = "a".repeat(MAX_FILE_NAME_LEN + 1);
        let source =
            format!("{MINIMAL_HEADER}[[assets]]\nkind = \"font\"\nfile = \"{long_file}\"\n");
        let err = parse_manifest(&source).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::StringTooLong {
                field: "asset.file",
                limit: MAX_FILE_NAME_LEN,
                actual
            } if actual == MAX_FILE_NAME_LEN + 1
        ));
    }

    #[test]
    fn a_glyph_name_over_the_length_ceiling_is_rejected_by_name() {
        let long_name = "a".repeat(protocol::MAX_SCENE_GLYPH_NAME_LEN + 1);
        let source = format!(
            "{MINIMAL_HEADER}\
             [[assets]]\nkind = \"icon-font\"\nfile = \"icons.ttf\"\n\
             glyphs = [ {{ name = \"{long_name}\", codepoint = 0xE001 }} ]\n"
        );
        let err = parse_manifest(&source).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::StringTooLong {
                field: "glyph.name",
                limit,
                actual
            } if limit == protocol::MAX_SCENE_GLYPH_NAME_LEN
                && actual == protocol::MAX_SCENE_GLYPH_NAME_LEN + 1
        ));
    }
}
