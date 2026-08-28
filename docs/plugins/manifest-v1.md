# Deskmate plugin manifest v1

Status: frozen. A plugin manifest is a TOML document that describes one card's face as
a declarative display list: a data source, a set of named assets, and a list of scene
nodes. The server fetches the source, evaluates the manifest's `{{ ... }}` expressions
against the fetched data, and compiles the result directly to a `Scene` — the same wire
shape `PushScene` (protocol v1, message 19) already carries for every other card. There
is no `DisplayTemplate` selection for a plugin card (`docs/config/v6.md`); the manifest
**is** the layout.

This document is the authoring contract: every bound below is enforced, by name, in
`companion/crates/plugin/src/{manifest,expr,compile,assets}.rs`, and violating one is a
parse-time, compile-time, or asset-resolve-time error — never a silent clamp or a panic.
The two curated plugins that ship today, `companion/plugins/aqi/manifest.toml` and
`companion/plugins/agenda/manifest.toml`, are worked examples of everything in this
document; read them alongside it.

## Top-level shape

```toml
name = "aqi"
version = "1.0.0"

[source]
kind = "json"
url = "https://example.invalid/aqi.json"
refresh_minutes = 15

[[assets]]
kind = "icon-font"
file = "icons.ttf"
glyphs = [ { name = "moderate", codepoint = 0x4D } ]

[[nodes]]
kind = "text"
# ...

[[repeats]]
source = "events"
dx = 0
dy = 40
[[repeats.nodes]]
kind = "text"
# ...
```

| Field      | Type                    | Required | Bound |
| ---------- | ----------------------- | -------- | ----- |
| `name`     | string                  | yes      | ≤ 64 bytes (`MAX_NAME_LEN`) |
| `version`  | string                  | yes      | ≤ 32 bytes (`MAX_VERSION_LEN`) |
| `source`   | table, `kind = "json"`  | yes      | see below |
| `assets`   | array of tables         | no (default `[]`) | ≤ 16 entries (`MAX_ASSETS`) |
| `nodes`    | array of tables         | no (default `[]`) | ≤ 24 total, counting every repeat's expansion plus one reserved slot for the shared stale/error footer (`MAX_NODES` = `protocol::MAX_SCENE_NODES`) |
| `repeats`  | array of tables         | no (default `[]`) | ≤ 2 groups (`MAX_REPEAT_GROUPS`) |

The raw TOML source itself is capped at 64 KiB (`MAX_MANIFEST_BYTES`), checked before any
parsing happens, and a nesting pre-scan rejects a manifest whose inline tables/arrays
(`{`/`[`) nest past 16 levels (`MAX_TOML_NESTING_DEPTH`) before the real TOML parser —
which is recursive — ever sees the bytes. Every table `#[serde(deny_unknown_fields)]`s:
an unrecognized field anywhere, at any nesting level, is a parse error, not a silently
ignored typo.

## `[source]`

The only source kind today is `json`:

```toml
[source]
kind = "json"
url = "https://..."
refresh_minutes = 15
```

- `url` must parse as a URL and its scheme must be exactly `https` (`ALLOWED_URL_SCHEME`)
  — a `file://` URL, for instance, is rejected at parse time rather than being allowed to
  read the server's filesystem. This is the lexical half of keeping a plugin off the
  server's own network; the remaining SSRF surface (private/loopback/link-local
  destinations, and the DNS-rebind window between check and connect) is the server's
  egress guard (`companion/crates/server/src/egress.rs`), which runs at fetch time
  because the destination address is not known until DNS resolves. `url` itself is
  bounded to 512 bytes (`MAX_URL_LEN`).
- `refresh_minutes` must be in `1..=1440` (`MIN_REFRESH_MINUTES..=MAX_REFRESH_MINUTES`).
  Zero would mean "refresh continuously," a denial-of-service against the plugin's own
  upstream and the server's egress guard.

## `[[assets]]`

Three kinds, each naming a `file` that must be a single plain filename directly inside
the manifest's own directory -- **not** a path. `file` is validated against
`std::path::Component`, not hand-rolled string matching: it is refused if it is absolute,
if any component is `..`, or if it has more than one component at all (so
`"assets/icons.ttf"` is refused exactly like `"../icons.ttf"` is -- a subdirectory is not
"relative to the manifest's own directory" any more loosely than a parent-directory walk
is, and allowing one but not the other would make the rule harder to audit for no
security benefit). This is deliberately the strictest reading and the reason the two
curated plugins below keep every asset file flat, beside their `manifest.toml`, rather
than under an `assets/` subdirectory:

```toml
[[assets]]
kind = "font"
file = "body.ttf"

[[assets]]
kind = "icon-font"
file = "icons.ttf"
glyphs = [
  { name = "moderate", codepoint = 0x4D },
  { name = "hazardous", codepoint = 0x48 },
]

[[assets]]
kind = "image"
file = "badge.rgb565"
```

- **`font`** — a plain TTF/OTF, sized per node at render time via `pixel_size`.
- **`icon-font`** — a TTF/OTF plus a `glyphs` array of `{ name, codepoint }` pairs. The
  `icon(name)` expression function (below) resolves a name through this table to a
  character; `glyphs` is capped at 256 entries (`MAX_GLYPHS_PER_ICON_FONT`) and each
  `codepoint` must be a valid Unicode scalar value (not in the UTF-16 surrogate range
  `0xD800..=0xDFFF`, not above `0x10FFFF`).
- **`image`** — a source image. **There is no image-decoding or RGB565-conversion
  pipeline in this crate or the server as of v1** — the committed file must already be
  the device-native blob: a 12-byte `lv_image_header_t` (magic `0x19`, `cf =
  LV_COLOR_FORMAT_RGB565` = `0x12`, little-endian packed fields) directly followed by raw
  host-endian RGB565 pixels, exactly as `sim_build_rgb565_image`
  (`companion/crates/lvgl-sim/csrc/sim_shim.c`) builds it and as the device's own asset
  store expects to receive it. `companion/plugins/agenda/manifest.toml`'s own doc comment
  and `companion/plugins/agenda/badge.rgb565` are the worked example. A real
  image-conversion pipeline is future work, not a v1 promise.

Two `[[assets]]` entries may not declare the same `file`. Every `file` is bounded to 128
bytes (`MAX_FILE_NAME_LEN`), and the resolved bytes of any one asset file are bounded to
1 MiB (`MAX_ASSET_BYTES`, which *is* `protocol::MAX_ASSET_TOTAL_LENGTH`, the wire's own
`AssetBegin.total_length` ceiling — imported, not restated). An asset is content
addressed: `plugin::resolve_assets` hashes the file's exact bytes with SHA-256 to get the
digest a compiled scene's `image`/`glyph` node, or a `font.asset` reference, carries on
the wire (`SceneImage.digest`, `SceneGlyph.digest`, `SceneFont::Asset.digest`). Two
manifests that ship byte-identical asset content resolve to the same digest — "ship
once" — which is why `companion/plugins/aqi/icons.ttf` is deliberately the same
committed bytes as `companion/crates/lvgl-sim/assets/Inter-subset.ttf` rather than a
second, separately-hashed copy of the same font (see the "No real icon artwork" section
below).

## `[[nodes]]` — the six-kind vocabulary

A manifest's display list is a flat sequence of nodes, drawn in order, on the 448×368
canvas (`protocol::SCENE_CANVAS_WIDTH`/`SCENE_CANVAS_HEIGHT`). This is a **display list,
not a layout language**: every field is a literal number or color except the two that
may carry `"{{ ... }}"` expression source (`text.value`, `glyph.glyph`) — there is no
flow, no auto-sizing, and no interpolation of a literal and an expression in the same
field (`"{{ x }} suffix"` is invalid; author two nodes instead — see
`companion/plugins/aqi/manifest.toml`'s AQI-value/"AQI"-label pair for the pattern).

The manifest exposes six of the wire's nine `scene_node_kind_t` kinds. `scale`, `label`,
and `rotrect` exist on the wire but are drawn only by hand-written builder code
(`app_core::scene_build`), not authorable from a manifest.

| `kind`   | Fields | Notes |
| -------- | ------ | ----- |
| `rect`   | `x, y, w, h, radius, fill, opacity` (opacity defaults to 255) | |
| `arc`    | `cx, cy, r, start_deg, end_deg, width, color, caps` (`caps` defaults `false`) | `end_deg` is always a literal here — see "The arc/line binding gap" below |
| `line`   | `points` (2–8 `{x,y}` pairs, `MIN_LINE_POINTS..=MAX_LINE_POINTS`), `width, color` | the endpoint/angle binding is likewise unavailable — same section |
| `text`   | `x, baseline_y, w, align, font, color, value, ellipsize` (`align` defaults `left`, `ellipsize` defaults `false`) | baseline-anchored, not box-top-anchored; `value` may be `"{{ ... }}"` |
| `image`  | `x, y, w, h, asset, recolor, color` (`recolor`/`color` default `false`/`0`) | `asset` names a `[[assets]] kind = "image"` file |
| `glyph`  | `x, y, font, color, glyph` | `font` must be `{ asset = "...", pixel_size = N }`, never `{ tier = "..." }` — a baked tier has no icon glyphs; `glyph` may be `"{{ ... }}"` but must resolve to a **literal**, never a binding (`SceneGlyph.name` is a plain wire string, with no shape for a device-side binding the way `SceneText.value` has) |

`font` (on `text` and `glyph`) is one of:

```toml
font = { tier = "hero" }                              # caption | body | display | hero
font = { asset = "icons.ttf", pixel_size = 64 }
```

## The expression language

`"{{ ... }}"` on `text.value` or `glyph.glyph` is either one of a **closed set of
device-side bindings** or a **restricted expression evaluated once, now, against the
fetched data, and baked into a literal**. A string that merely *looks* like an attempt at
the closed set (starts with `time:`, `timer.`, `field.`, or is exactly `date`) but does
not match it exactly is refused at compile time rather than silently falling through to
the expression evaluator.

### The closed binding set

Compiles to `SceneValue::Binding`, which the device keeps resolving locally — against
its own clock, its own locally-ticked timer snapshot, or the most recent `PushData` —
without a scene recompile:

- `date`
- `time:<strftime-like format>` — the wall clock, e.g. `time:HH:mm`
- `timer.pct`, `timer.permille`, `timer.status`, `timer.remaining:<fmt>`,
  `timer.elapsed:<fmt>`, `timer.total:<fmt>` — a card's `ProgressRing`-shaped timer
  snapshot
- `field.<name>` — a value carried by `PushData` for this card's widget id, evaluated
  device-side through the same registry the six retired C templates used
  (`firmware/main/core/template_fields.c`). **This is the one binding this repository's
  own notes describe as having no builder and no pixel coverage anywhere** before this
  plugin pair; see "field.\*: reach and its real limit" below for exactly what it can and
  cannot name today.

### Restricted expressions

Everything else inside `{{ }}` is parsed as one of:

- **Field access**: `data.a.b.c`, array index `data.rows[0]`, up to 16 total
  `.field`/`[index]` steps (`MAX_PATH_SEGMENTS`). A missing key, a JSON `null`, an
  out-of-range index, or an object/array where a scalar was expected all evaluate to
  `Missing` — never a panic or a parse-time refusal, because a missing provider field is
  an ordinary, expected outcome, not a manifest error.
- **String literals**: `"like this"`.
- **Six functions**: `upper(s)`, `lower(s)`, `round(x, places)` (`places` ≤ 12,
  `MAX_ROUND_PLACES`), `truncate(s, n)` (bounds to at most `n` characters, char-boundary
  safe), `default(a, b)` (returns `a` unless it is `Missing`, else `b`), `icon(name)`
  (resolves `name` through every `[[assets]] kind = "icon-font"` glyph table the manifest
  declares; a name no glyph defines is `Missing`, not an error).
- **The `?:` (elvis) operator**: `a ?: b`, "prefer `a` if present."
- Nested calls/parens up to 16 deep (`MAX_DEPTH`), enforced at parse time.

A number-typed provider field never silently coerces from a string that merely *looks*
numeric (`"42"` stays `Text`, and `round("42", 0)` is `Missing`, not `42`) — the AQI
fixture's `current.legacy_index` field exists specifically to pin this. Evaluation is
metered by a shared fuel budget (`FUEL_BUDGET` = 10,000 AST nodes, one budget per
manifest compile, not per node) so a legal-looking but repeated expression cannot hang
compilation, and every constructed text value is bounded to 4,096 bytes
(`MAX_OUTPUT_LEN`) — rejected, not silently shortened, since silent truncation here would
be indistinguishable from `truncate(s, n)`'s own deliberate one.

A node's compiled literal is separately bounded to `protocol::MAX_SCENE_TEXT_LEN` (128
bytes) — the wire's own per-text-node ceiling — which is a tighter bound than the
expression evaluator's own 4,096-byte ceiling above it.

### `field.*`: reach and its real limit

`field.*` reads whatever `PushData` most recently carried for this card's widget id,
through the field registry the device's `ApplyConfig` selected for it — but **every
plugin card's `WidgetConfig.template` is `TemplateKind::DigitalClock` on the wire, always
and deliberately** (`companion/crates/app-core/src/config.rs`'s `wire_config`, a decision
predating this task and not reopened by it — a plugin renders from a host-pushed scene,
not any built-in C template, so which byte `template` carries is otherwise inert). That
means the only field names `field.*` can ever legally resolve, for **any** plugin card,
are the four `s_digital_clock_fields` registers
(`firmware/main/core/template_fields.c`): `title`, `show_seconds`, `stale`, `error`. A
plugin does not get a field registry of its own to "bring" — there is no wire mechanism
for one. `companion/plugins/aqi/manifest.toml` binds `field.title`, deliberately, as the
one name in that set that is not already claimed by the shared stale/error footer
mechanism below.

No production code path pushes `PushData` for a plugin card as of v1 — `field.*` on a
real device today renders the "--" placeholder until something does. This manifest pair
is what gives the binding its first real pixel coverage (both a live value, via
`companion/crates/lvgl-sim/src/cases.rs`'s `plugin_scene_cases`, and the placeholder, via
its `empty` state), not a claim that it is wired end to end in production.

### The arc/line binding gap

`compile.rs` compiles every `arc` node's `end_deg` and every `line` node's angle/pivot
fields as fixed literals (`SceneArc.end_binding` and `SceneLine.angle_binding` are always
empty strings) — there is no way to author a ticking arc or a rotating clock hand from a
manifest today. This was deferred deliberately by an earlier task in this stage.
**Neither curated plugin needs it**: `aqi` uses no `arc`/`line` node at all, and
`agenda`'s only geometry beyond text is its `image` badge. If a future plugin needs a
live arc or hand, that is new compiler work, not something this contract already
supports.

## `[[repeats]]` — the one repeat form

```toml
[[repeats]]
source = "events"
dx = 0
dy = 40

[[repeats.nodes]]
kind = "text"
x = 24
baseline_y = 140
w = 80
font = { tier = "body" }
color = 0x5C5C66
value = "{{ default(data.events[item].time, \"--:--\") }}"
```

- `source` (≤ 64 bytes, `MAX_REPEAT_SOURCE_LEN`) is a plain dotted path (no array-index
  syntax) into the fetched data, naming the array to iterate. A missing path, or an
  explicit JSON `null` at the end of it, means "no items yet" — zero repetitions, not an
  error. A present value that is neither an array nor `null` is a real authoring error
  (`RepeatSourceNotArray`).
- The repetition count is `min(array length, MAX_REPEAT_ITEMS)` — **5**, however long the
  fetched array actually is (`companion/plugins/agenda`'s fixture ships six events
  specifically so its golden matrix proves the sixth never reaches the panel).
- `dx`/`dy` offset repetition `n`'s every node by `n * dx, n * dy` from that node's own
  authored position.
- Inside a repeat's template nodes, the bare identifier `item` is textually substituted
  with the 0-based repetition index *before* the expression is parsed — `data.events[item].time`
  becomes `data.events[2].time` for the third repetition. This is why a repeated node
  writes a full indexed path rather than some other "current row" syntax: it reuses the
  same array-index grammar every other expression already has. A longer identifier that
  merely contains "item" (`items`), a trailing path segment literally named `item`
  (`field.item`, `data.item`), and any occurrence inside a string literal are all left
  alone by the substitution.
- At most 2 repeat groups per manifest (`MAX_REPEAT_GROUPS`); a repeat group's own
  template contributes to the shared 24-node ceiling once expanded — `MAX_NODES` counts
  the *expanded* total, not the authored template.

## The shared stale/error footer

Every compiled scene gets a shared bottom-of-canvas footer appended automatically
(`app_core::scene_build::with_scene_data_state`, reused rather than reimplemented): if
the fetch that produced this scene errored, the footer shows that error text (bounded to
`protocol::MAX_SCENE_TEXT_LEN` by truncation, not refusal — a display footer summarizing
a fault is not authored content); otherwise, if the data is merely stale, it shows
"Stale"; otherwise it appends nothing at all. A manifest never authors this footer
itself — its one reserved node slot is why `MAX_NODES` checks `authored + 1`, not just
`authored`.

## Worked examples: the two curated plugins

- **`companion/plugins/aqi/manifest.toml`** — a JSON object source (not a list). A
  numeric hero (`{{ data.current.aqi }}`), an uppercased category label
  (`{{ upper(data.current.category) }}`), a secondary PM2.5 reading with a `default()`
  fallback for a `null` reading, the `icon-font` path (a `glyph` node drawing
  `{{ icon(data.current.category) }}` from a real uploaded font asset), and the
  `field.title` binding.
- **`companion/plugins/agenda/manifest.toml`** — a JSON list source. One `[[repeats]]`
  group over `events`, capped at 5 rows even though the fixture ships 6; a real
  truncation case (`truncate(default(data.events[item].title, "(untitled)"), 40)` on a
  92-character title, with `ellipsize = true` so the device's own width-based LVGL
  long-mode has something to do on top of the expression-level cut); an `image` node
  drawing a hand-built RGB565 badge asset; and two rows exercising a fully-`null` fixture
  event (`evt-3`) to prove `default()` fallbacks render rather than panic or propagate.

Both compile against captured-in-spirit fixtures with real nesting, JSON `null`s in
several positions, a numeric-looking string field, and sibling fields nothing reads —
`companion/crates/plugin/tests/fixtures/aqi_response.json` (reused from an earlier task
in this stage, not recaptured) and `.../agenda_response.json` (new for this task) — never
a shape hand-built to flatter the compiler. `companion/crates/plugin/tests/curated_plugins.rs`
compiles both against those fixtures and their real, on-disk, committed assets; nothing
about either plugin's face is asserted only against an in-memory synthetic case.

## No real icon artwork (a known, honest gap)

There is no icon library, font-authoring tool, or network access available to this
stage to produce real glyph shapes. `aqi`'s icon-font asset is the same already-committed
`Inter-subset.ttf` test font `lvgl-sim` ships (byte-identical, same SHA-256 digest — see
"`[[assets]]`" above), with the six EPA AQI categories mapped to a single capital letter
each (`good` → `G`, `moderate` → `M`, and so on). This exercises the real wire path end
to end — an uploaded font asset, a `glyph` node resolving a name to a codepoint through
it, the device drawing that glyph — with real, already-verified-identical bytes, rather
than leaving the icon-font path untested for want of artwork, or inventing new binary
content whose only purpose would be to look like an icon. Swapping in a real icon font
later is a content change to `companion/plugins/aqi/icons.ttf`, not a shape
change to the manifest or the compiler.

## Compiling a manifest: `compile_scene` vs. `compile_scene_with_assets`

`plugin::compile_scene(manifest, snapshot, metrics, revision)` compiles against an
always-empty asset set — any `image`/`glyph` node, or a `text`/`glyph` node naming an
asset font, fails closed with `CompileError::AssetNotResolved`. This is deliberate and
is what every pre-existing caller and test in this crate already depends on.
`plugin::compile_scene_with_assets(manifest, snapshot, metrics, revision, &assets)` is
the one that actually resolves an `image` node, a `glyph` node, or a `font.asset`
reference against a real `plugin::AssetSet` (built by `plugin::resolve_assets`),
kind-checking each reference along the way — an `image` node naming a `font`/`icon-font`
asset is refused as `CompileError::AssetKindMismatch { expected: "image" }`, not silently
drawn as if it were an image, and vice versa. Both curated plugins in this repository
need `compile_scene_with_assets`; a manifest with no `image`/`glyph` node and no asset
font at all — which neither of these two is — would compile identically through either
entry point.
