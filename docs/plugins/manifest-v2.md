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
display_name = "Air quality (SVG)"
description = "EPA index for a location, drawn from an SVG template"
summary = "{{ data.current.aqi }}"

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
unchanged. V2 adds only the explicit discriminator, source roots, the template union, the
two authorable binding positions below, and the three optional presentation keys.

Every v2 manifest must contain exactly one `[template]`. V1 must contain none.

## Presentation keys (`display_name`, `description`, `summary`)

Three optional top-level keys, added 2026-09-07 for plugin-card parity in the companion
app (`docs/superpowers/specs/2026-09-06-deskmate-plugin-card-parity-design.md` §3). All
three are v2-only: a v1 manifest carrying any of them is rejected by name
(`ManifestError::V2FieldInV1`). Absence is always tolerated. `name` stays the identity
key and is never repurposed. `RawPluginManifest` remains `deny_unknown_fields`, so an
unknown top-level key is still a rejection, not a silent no-op.

| Key | Type | Bound | Meaning |
|---|---|---|---|
| `display_name` | string | 1..=64 bytes (`MAX_DISPLAY_NAME_LEN`, the card `title` bound) | What the card **is**, on every surface: "Air quality". Absent → the plugin id. |
| `description` | string | 1..=160 bytes (`MAX_DESCRIPTION_LEN`) | The add-menu's second line: "EPA index for a location". Absent → "Plugin · <version>". |
| `summary` | expression string | source ≤ 256 bytes (`MAX_EXPR_SOURCE_LEN`, as a text node's `value`); evaluated output truncated to 32 bytes on a char boundary (`MAX_SUMMARY_LEN`) | The tile's live value: `"{{ data.current.aqi }}"`. Evaluated on every refresh by `plugin::evaluate_summary` and published as the card's `hero` field. Absent → the tile shows "—". |

An empty `display_name` or `description` is `ManifestError::EmptyString`, not a fallback
request; omit the key to fall back.

`summary` uses the same whole-value `{{ ... }}` rule, grammar and six functions as a
text node's `value`, with one difference: it is **data expressions only**. A device
binding (`time:*`, `timer.*`, `date`, `field.*`) anywhere in the expression outside a
string literal is `ManifestError::SummaryUsesDeviceBinding` at parse time, because the
summary is evaluated on the server at refresh time, where none of those resolve. A
provider path that merely shares a name, such as `data.date` or `data.events[0].date`,
is fine. Evaluation rules: a literal is returned as written; an expression that
evaluates to a missing value (an absent field, a JSON `null`, a type mismatch) yields no
summary rather than an empty string, so the tile falls back to "—"; a partial
interpolation or an expression that fails to parse or evaluate is a named `SummaryError`,
which the server logs once per plugin and otherwise treats as "no summary"; `icon()` has
no glyph table here and yields no summary. A stale or error snapshot is evaluated like
any other, because it still carries the last-good value.

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

The four curated plugins (`aqi`, `agenda`, `claude-limits`, `svg-aqi`) are all manifest
v2 as of 2026-09-07 and declare every presentation key. The v1 contract keeps real
coverage through two byte-exact copies of the `aqi` and `agenda` manifests as they
shipped under v1, frozen at:

- `companion/crates/plugin/tests/fixtures/manifest_v1_aqi.toml`
- `companion/crates/plugin/tests/fixtures/manifest_v1_agenda.toml`

They parse with synthesized typed defaults `ManifestVersion::V1`, `Template::Scene`,
`Source::Json { root: None, .. }`, and `display_name`/`description`/`summary` all
`None`; every field present in the frozen v1 typed shape retains the same value. A v1
manifest that supplies `manifest_version`, `[source].root`, `[template]`,
`arc.end_binding`, bound-line geometry, `display_name`, `description`, or `summary` is
rejected rather than silently changing meaning.
