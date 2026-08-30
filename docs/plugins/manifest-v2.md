# Deskmate plugin manifest v2

Status: frozen. Manifest v2 is an additive contract beside the frozen
[`manifest-v1.md`](manifest-v1.md); it does not revise v1. A manifest with no
`manifest_version` is v1. A manifest with `manifest_version = 2` is v2. No other value
is accepted.

`version = "1.0.0"` remains the plugin content version. It is not and never becomes the
manifest-contract discriminator.

## Top-level shape

```toml
manifest_version = 2
name = "svg-aqi"
version = "1.0.0"

[source]
kind = "json"
url = "https://example.invalid/aqi.json"
refresh_minutes = 15
root = "payload"

[template]
kind = "svg"
file = "face.svg"
```

The v1 `name`, content `version`, source URL/refresh bounds, assets, expression language,
node-count budget, repeat budget, stale/error footer, and asset resolution rules remain
unchanged. V2 adds only the explicit discriminator, source roots, the template union, and
the two authorable binding positions below.

Every v2 manifest must contain exactly one `[template]`. V1 must contain none.

## `[source].root`

`root` is optional in v2 and forbidden in v1. When absent, the successfully parsed raw
JSON value is the provider value, exactly as in v1. When present, it is a dotted path of
object keys, such as `payload` or `response.data`.

- The string is at most 256 UTF-8 bytes (`MAX_SOURCE_ROOT_LEN`).
- It has at most `expr::MAX_PATH_SEGMENTS` segments; the expression-path ceiling is
  reused rather than copied.
- Empty paths and empty segments are invalid.
- Every intermediate value must be an object. The selected leaf must be an object or an
  array, not null or a scalar.

Root selection is a provider operation. It happens after successful JSON parsing and
before `LastGood::complete`. A missing root, null root, or wrong-shaped path is a named,
permanent failure for that response shape. If a previous rooted response succeeded, its
inner value remains last-good and the failed refresh is stale. There is no implicit
`payload` fallback or any other universal envelope convention.

## `[template]`

The union has exactly two forms.

### Scene

```toml
[template]
kind = "scene"

[[nodes]]
# the v1 node vocabulary, plus the binding positions below
```

`scene` permits top-level `[[nodes]]` and `[[repeats]]` and forbids `file`.

V2 retains v1's fixed arc and point-list line forms and adds:

```toml
[[nodes]]
kind = "arc"
cx = 224
cy = 184
r = 120
start_deg = 270
end_deg = 630
end_binding = "{{ timer.permille }}" # timer.pct is also accepted
width = 12
color = 0xff8f2e

[[nodes]]
kind = "line"
pivot_x = 224
pivot_y = 184
length = 120
angle_binding = "{{ time:angle:minute }}" # or time:angle:hour
width = 4
color = 0xf5f5f7
```

An arc end accepts only `timer.pct` or `timer.permille`. A bound line accepts only
`time:angle:hour` or `time:angle:minute`. These are existing protocol-v1 binding tokens;
v2 adds no vocabulary and no wire node. A line supplies either `points` or all four bound
fields (`pivot_x`, `pivot_y`, `length`, `angle_binding`), never both and never neither.
The protocol's `validate_scene`/`validate_message` checks remain authoritative for canvas,
length, and wire bounds.

A token that looks live but is not in the closed protocol vocabulary, such as
`timer.velocity`, is a named compile error. It is never evaluated into a one-time literal.
Likewise, a valid token in the wrong position (`timer.status` on an arc, for example) is
rejected.

### SVG

```toml
[template]
kind = "svg"
file = "face.svg"
```

`svg` requires exactly one `file` and forbids `[[nodes]]` and `[[repeats]]`. `file` must
be one `std::path::Component::Normal` filename directly beside `manifest.toml`: absolute,
parent, current-directory, and subdirectory paths are rejected. The registry joins and
then canonicalizes both the plugin directory and template path and checks containment
again, so an in-directory symlink cannot escape. This is the same two-layer posture as
asset files.

The registry reads the SVG once at startup, validates UTF-8, bounds it to 256 KiB
(`MAX_SVG_SOURCE_BYTES`) for retained-memory/XML-parse cost, and holds one `Arc<str>` for
the registry lifetime. It is not re-read per provider refresh. Expanded SVG is separately
bounded to 512 KiB (`MAX_EXPANDED_SVG_BYTES`) because expression substitution can grow the
document before XML parsing.

An SVG may use the existing restricted `{{ ... }}` expressions only as the complete
value of an XML text node or attribute. Evaluated values are XML-escaped before parsing.
Partial interpolation is not part of the contract; use adjacent `<tspan>` elements.
Scripts, event handlers, animation, external URLs/files, ambient system fonts, and active
content remain forbidden.

A live binding inside SVG remains a live render requirement for negotiation. It is never
turned into text for `resvg` to freeze into pixels. If the target cannot render that live
requirement natively, the card is refused under the stage-4 negotiation rule.

## V1 compatibility

The two shipped v1 manifests remain byte-for-byte unchanged:

- `companion/plugins/aqi/manifest.toml`
- `companion/plugins/agenda/manifest.toml`

They parse with synthesized typed defaults `ManifestVersion::V1`, `Template::Scene`, and
`Source::Json { root: None, .. }`; every field present in the frozen v1 typed shape retains
the same value. A v1 manifest that supplies `manifest_version`, `[source].root`,
`[template]`, `arc.end_binding`, or bound-line geometry is rejected rather than silently
changing meaning.
