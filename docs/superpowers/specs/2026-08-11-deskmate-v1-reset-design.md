# Deskmate V1 Reset — Design

> **HISTORICAL DESIGN.** The desktop-companion references describe the V1 architecture
> at the time. The Tauri companion was deleted on 2026-09-18; this is not a description
> of the current companion.

**Date:** 2026-08-11
**Status:** Approved section-by-section in brainstorming; supersedes the M4 plan's
remaining scope (see §7).
**Prior contract:** `docs/superpowers/specs/2026-08-03-deskmate-design.md` remains the
reference for everything this document does not amend.

## 0. Why this reset

Three defects surfaced during M4 use, and evaluating them led to a product-shape
decision rather than a patch list:

1. The settings preview is a hand-written React lookalike of the firmware's LVGL
   templates. It has drifted visibly (gold-gradient clock the panel never renders)
   because nothing connects the two implementations.
2. The authoring UX — a per-card `presence` tri-state feeding one implicit rotation —
   is not discoverable. The user wants a TRMNL-style playlist model.
3. The built-in widgets read as LVGL demos: built-in Montserrat at a few fixed sizes,
   per-template magic offsets, no visual system.

Evaluating fixes surfaced a long-term product architecture (§1) that the old roadmap
did not anticipate. The roadmap is re-cut around it (§7); this document specifies the
first stage, **V1: Local Deskmate**.

## 1. Long-term product architecture (decided, not all built now)

### 1.1 Device contract

The device supports two card classes:

- **Native templates** — live, device-rendered faces (clock, pomodoro, calendar,
  weather, RSS/JSON rows, big-number, icon-badge). They tick locally
  (`template_view.c`'s tick timer) and keep working with no host attached.
- **Image cards** *(V4)* — 448×368 PNGs produced by community plugins, refreshed on
  the plugin's interval. Static by nature; anything live must be a native template.

Pixel-shipping-only was rejected: a device that only displays frames cannot advance
its own clock (a ticking second hand would need ~322 KB/s of frames forever), and the
three most-developed widgets are live. TRMNL's all-image model works because e-ink
refreshes every ~15 minutes; this product's AMOLED does not.

### 1.2 Host ownership

Exactly one host owns authoritative device state (config, time, provider data,
interrupts, replay) at any moment — M3's single-owner invariant, preserved. **Which**
host is a setup-time pairing choice:

- **Free tier:** the user's Mac owns the device over USB. Local, no account. This is
  M3's runtime essentially unchanged.
- **Paid tier** *(V3+)*: the device joins WiFi and the Deskmate server owns it; the
  Mac app demotes to a settings client that writes through the server. The user's Mac
  can be off.

Switching tiers is an explicit re-pairing. Never two owners at once.

### 1.3 Community plugins

Plugins are authored as HTML/CSS and rendered to 448×368 PNGs by **one headless
Chromium on the server only** *(V4)*. Server-only rendering was chosen over
local-webview rendering to eliminate engine divergence (WKWebView vs WebView2 vs
Chromium) by construction. Consequences:

- Plugins are inherently a paid-tier feature: they require the service. Built-ins
  stay free, local, and standalone forever.
- Plugins carry their own type inside the PNG, so the device font system only serves
  the built-in native templates. This removed the justification for pulling the
  asset/font-push subsystem (old M4 Task 6) forward; it returns in V4 as the
  plugin-image cache.

### 1.4 Standalone

Standalone survives as a requirement. The built-in tier (whatever must render with no
host ever attached: standalone clock, connection status, errors) is baked into the
app image, including its typeface (§5). Battery is orthogonal to standalone — a
wall-powered WiFi device with the PCF85063A backup RTC is standalone with no cell.

### 1.5 Roadmap decomposition

| Stage | Scope | Depends on |
|---|---|---|
| **V1 — Local Deskmate** (this spec) | Partition table, baked typeface, pixel-exact preview harness, built-in widget redesign, playlist authoring model | — |
| **V2 — Networked device** | WiFi, TLS, provisioning, pairing, host handoff, OTA update mechanism | V1 |
| **V3 — Server host** | `app-core`/`providers` deployed server-side, device pairing, config storage, accounts | V2 |
| **V4 — Plugin platform** | HTML plugin contract, headless-Chromium renderer, image cards, asset cache, billing | V3 |

Each of V2–V4 gets its own brainstorm → spec → plan cycle at the preceding stage's
exit. This spec binds V1 only; §1 records the decided direction so V1's irreversible
choices (§4) are made network-ready.

## 2. V1 Section 1 — the pixel-exact preview harness

### 2.1 Principle

The preview stops being a second implementation. The same
`firmware/main/ui/templates/*.c`, `template_view.c`, and `firmware/main/core/*.c`
sources compile against desktop LVGL into a headless 448×368 RGB565 framebuffer on
the host. The app displays those pixels. Drift becomes inexpressible, not detected.

### 2.2 Single LVGL configuration

The firmware currently has **no `lv_conf.h`** (`CONFIG_LV_CONF_SKIP=y`; all LVGL
config lives in Kconfig/sdkconfig). That is inverted:

- Check in `firmware/lv_conf.h` as the single source of LVGL configuration.
- Set `CONFIG_LV_CONF_SKIP=n`; both the ESP-IDF build and the host build include the
  same file. Not a copy, not a generated mirror — one file, two include paths.
- Fallback only if `esp_lvgl_port` proves to require Kconfig symbols: generate the
  host header from `sdkconfig` at build time, using LVGL's `lv_conf_kconfig.h` as the
  mapping reference. This is worse (a generator can go stale) and is used only if
  forced; the spec's default is the shared file.

### 2.3 Time seam

New `firmware/main/core/clock_source.{h,c}`: `int64_t clock_source_now(void)`
defaulting to `time(NULL)`, with a test/harness override setter. Templates call it
instead of `time(NULL)`. Lives in `core/` (ESP-IDF-free, host-tested as plain C, per
the engineering constraints).

This also converts the open `show_seconds`-false analog-clock defect (wrong time
observed 2026-08-11, undiagnosed, timezone hypothesis untested) into a deterministic
host test asserting hand angles for a fixed instant and UTC offset. That test is part
of V1's scope; the board-notes item closes on its evidence plus a repeated physical
check, or stays open with a diagnosis.

### 2.4 The `lvgl-sim` crate

`companion/crates/lvgl-sim/`: a `build.rs` (via `cc`) compiles LVGL, the firmware
`core/` sources, `ui/templates/*.c`, and `template_view.c`. No firmware source is
copied or reimplemented.

```
RenderRequest { template, fields[], utc_offset_minutes, now_unix_ms, orientation }
    → template_view_show()            (firmware's own entry point, unchanged)
    → LVGL software render, LV_COLOR_DEPTH=16, lv_display_set_rotation for 90°/270°
    → 448×368 RGB565 buffer → PNG
```

Orientation uses LVGL's own rotation path — the same code the firmware runs — not a
CSS transform.

### 2.5 Runtime integration and bounds

- LVGL is not thread-safe: the renderer is owned by one thread inside the existing
  single-owner runtime. No new ownership domain.
- Renders trigger on card selection, on field-data change, and at 1 Hz while a live
  template (clock/ring) is selected. Requests coalesce; at most one render in flight
  (same pattern as provider jobs in `runtime.rs`).
- The webview receives PNG bytes over typed IPC and gains no new privileges.
- `DevicePreview.tsx` and the preview portion of `styles.css` are **deleted**: the
  hand-written faces, `ICON_GLYPH`, `WEATHER_ICONS` (as a preview concern), and the
  CSS clock all go. The `<img>`-based preview replaces them.

## 3. V1 Section 2 — enforcing exactness

### 3.1 Claim boundary

The harness reproduces the device's **framebuffer** for a given
`(template, fields, instant, orientation)`. It does not reproduce photons, and it
renders whole frames — so defects in *partial* flushing (the CO5300 even-pixel
rounding path, `board_lcd_rounder_cb`) are explicitly outside its reach. A widget
that draws outside its invalidated region can look correct in the harness and leave
stale pixels on the panel. The existing physical checks remain responsible for that
class.

### 3.2 Enforcement layers

1. **Golden frames in CI.** Every template × both orientations × a field matrix
   (empty, typical, maximal, malformed, unicode, truncation boundary) rendered and
   byte-compared against committed PNGs (~70 images, 1–2 MB in git). Any pixel-moving
   edit to a template, `lv_conf.h`, or LVGL fails CI. Visual changes land as
   reviewable image diffs.
2. **Configuration canary.** A test asserts `LV_CONF_SKIP` is off and that a canary
   define from `lv_conf.h` is visible to the ESP-IDF build, so a future `menuconfig`
   run cannot silently revert the firmware to Kconfig while the harness reads the
   file.
3. **Dev-only framebuffer capture** (accepted). A development-build-only diagnostic
   dumps the device's LVGL draw buffer (~322 KB) over the wire, enabling pixel-diff
   of the physical panel against the harness for the golden cases. Not present in
   release builds, not part of the release wire contract. Diagnostic output must not
   contaminate the machine-protocol stream (existing constraint); the dump is framed
   as a proper protocol message in dev builds, not log text.

### 3.3 Error handling

No fallback renderer exists, deliberately.

| Failure | Behaviour |
|---|---|
| Harness fails to compile | CI fails hard; no degraded preview mode |
| Render fails at runtime | Explicit "preview unavailable" state; never stale or invented frames |
| Config names an unknown template | Preview shows exactly the device's fallback for the same input |

### 3.4 Test tiers after V1

| Tier | Role |
|---|---|
| Plain-C model tests (existing 9 suites) | Unchanged |
| `lvgl-sim` golden frames | New visual-regression net |
| Rust workspace | Render pipeline, coalescing, bounds, playlists |
| Frontend | Reduced: no faces left to test |
| Physical | Framebuffer diff + existing partial-flush/orientation checks |

## 4. V1 Section 3 — playlists, schema v4, migration

### 4.1 Shape

```rust
AppConfig {
    schema_version: 4,
    preferences: AppPreferences,
    cards: Vec<CardSettings>,        // `presence` removed; `alert` stays per-card
    playlists: Vec<Playlist>,
    active_playlist_id: String,
}

Playlist {
    id: String,                      // stable, internal
    name: String,                    // user-facing, 1..48 chars
    advance: CarouselAdvance,        // Manual | Timed { default_dwell_seconds }
    entries: Vec<PlaylistEntry>,
}

PlaylistEntry { card_id: String, dwell_seconds: Option<u32> }
```

- `advance` is **per-playlist** (the global `carousel` setting is removed): pacing is
  a property of the loop.
- `alert` stays **on the card**, never the playlist: alerts are global interrupts and
  fire regardless of which playlist is active.
- `presence` ceases to exist. In-rotation = card is in a playlist; alert-only = card
  has an alert and is outside the active playlist; off = in the library, unused.

### 4.2 Bounds

- Library: **1..=8 cards** (unchanged from v3). The wire caps compiled widgets at 8,
  and the compiled set is active-playlist entries plus alert-capable cards outside
  it; at 8 the cap is satisfied by construction with no user-visible counting rule.
  Raising the wire bound is V2+ work.
- Playlists: 1..=8, unique ids, unique names. Entries: 1..=8, each referencing an
  existing card, no card twice in the same playlist. `active_playlist_id` must
  resolve. Existing numeric bounds (`dwell_seconds`, `default_dwell_seconds`,
  `lead_minutes`, `hold`) unchanged.
- All new tagged types use the hand-written validating `Deserialize` pattern
  (`strict_tagged_enum`) — serde's `deny_unknown_fields` remains a no-op on
  internally tagged enums.

### 4.3 Compilation and switching

Only the active playlist lowers to screens, in entry order, screen id derived from
card id as in v3. Alert-capable cards outside the active playlist compile to
screenless widgets (v3's `alert-only` path). `SizeClass::Full` stays pinned; the wire
contract does not move — `ApplyConfig` fixtures must re-encode byte-identically.

Switching the active playlist is an ordinary config apply: validate → compile →
persist atomically → replace runtime state → queue device replay. No new wire
message. Device-side playlist switching (gesture) is out of scope for V1.

### 4.4 Migration v3 → v4

- Cards keep ids and all settings except `presence`.
- Synthesize one playlist, name "My playlist", `advance` taken from the old global
  `carousel`, entries = the `InRotation` cards in array order with their
  `dwell_seconds`. It becomes active.
- `AlertOnly` cards → library, outside the playlist, alert intact (behaviour
  identical).
- `Off` cards → library with `alert` forced to `None`. This is the single lossy
  point: v3 allowed an `Off` card to carry a never-firing alert; preserving it would
  make muted cards start alerting after upgrade. The migration is behaviour-lossless
  and configuration-lossy exactly here, by design.
- v0/v1/v2 documents migrate directly to v4 (no in-memory chaining), reusing the v3
  migration's card derivation and then the v3→v4 rules. `future-v4.json` fixture is
  renamed `future-v5.json`.

### 4.5 Carried defect fixed in this section

The validation-fallback defect (a config failing validation falls back to built-in
defaults, pushes them to the device, and labels them "your last working settings")
is in this code path and is fixed here: keep the genuine last-good config, push
nothing, surface a typed error naming the offending playlist/card. Validation issues
are keyed by stable id (card id, or playlist id + entry index), preserving the
Task 2B reordering fix.

`AppSnapshot` gains `playlists` + `active_playlist_id`; `types.contract.ts` is
regenerated from the Rust fixture printer with the byte-for-byte sync test.

The second carried defect — an alert firing while the device is unpowered is lost on
reconnect (observed once, undiagnosed) — is **not** fixed by this spec; it remains
open and travels to the V1 plan as an investigation item.

### 4.6 Settings UX

The M4 Task 8 remainder is reshaped around playlists: a card library plus named
playlists, one visibly active, entries reorderable with per-entry dwell; the
filmstrip binds to the active playlist. Alert configuration stays on the card editor.
First-run guidance, stable ids, unsaved drafts, keyboard/pointer parity, visible
focus, and reduced-motion behaviour carry over as requirements from the M3/M4
settings work.

**Amended 2026-09-06 (owner direction; plan
`docs/superpowers/plans/2026-09-06-deskmate-one-loop.md`).** The settings UX no longer
exposes a card library or named playlists. The window has one loop: the complication
tile grid in loop order, reordered in place, with an add-card slot whose menu enrols the
new card at once; the per-card editor edits the card's dwell; the ring's head carries
the pacing control. Schema v4/v5/v6's `playlists[]` and `active_playlist_id` are
untouched — the document keeps exactly one playlist, the app never creates another, and
extra playlists in older files round-trip unchanged. The reasons, in the owner's words:
adding a card and then placing it was a step with no purpose in a one-playlist world,
and a growing plugin registry must never reshape the window. §4.1–4.5 remain the
schema contract.

## 5. V1 Section 4 — partition table and baked typeface

### 5.1 Partition table (irreversible)

`firmware/partitions.csv`, replacing the default `SINGLE_APP` table:

```
nvs        64K
otadata     8K
phy_init    4K
ota_0       4M      # image today: 0.77 MB; V2 adds WiFi+TLS (est. 1.5–2.5 MB)
ota_1       4M
assets      6M      # deliberately dead space in V1; claimed for V4
coredump   64K
```

- 4 MB slots remove doubt for the WiFi-era image; the cost (2 MB less asset space) is
  negligible against plugin PNGs of tens of KB.
- **No factory recovery slot.** Recovery is USB reflash until devices ship to third
  parties. Recorded consequence: adding a factory slot later requires repartitioning
  (physical reflash).
- The `assets` region gets no filesystem, no mount, and no code in V1. Claiming the
  region is the irreversible act; its format is a V4 decision.
- V1 boots from `ota_0` via the OTA-aware bootloader. No update mechanism ships in
  V1 (V2 scope), but every device flashed from here on has the layout it needs.

### 5.2 Baked typeface — mechanism and bounds (face chosen in the redesign loop)

- Pipeline: `lv_font_conv` output checked in as `firmware/main/ui/fonts/*.c`,
  generated by a committed `tools/genfonts.sh`, never hand-edited. The host harness
  compiles the same files, so preview type is exact.
- Four sizes (≈18/28/56/96 px — caption, body, secondary display, hero; exact values
  tuned in the redesign, count capped at four).
- Subset: ASCII + the **Latin-1 Supplement block (U+00A0–U+00FF)** + the punctuation
  the built-ins emit (`—`, curly quotes); the block subsumes the previously explicit
  `°` and `·`. Out-of-subset glyphs render LVGL's fallback box; a golden test pins
  that appearance. *Latin-1 Supplement amended 2026-08-13 by user direction after
  design review: real calendar feeds send accented Latin row titles ("Café",
  "Zürich") and every one of them was rendering as a box. The widening costs the
  28px tier one pixel of line height, because Latin-1's accented capitals reach
  higher than any ASCII glyph.*
- Hero sizes: digits + `:` `-` `°` `%` only, **tabular figures** (a ticking clock
  must not shimmer as digit widths change).
- Budget: ≤ 200 KB total across all font files, asserted by a build-time size check
  (estimated 100–150 KB).
- Built-in Montserrat is disabled in `lv_conf.h` once all templates migrate, so no
  template can silently keep using it.
- Face criteria: OFL or equivalent license recorded in-repo; strong tabular
  numerals; legible at 18 px at 328 ppi; distinct from Montserrat. Candidates for
  the harness loop: Inter, Space Grotesk, IBM Plex Sans, plus optionally one display
  face for the hero tier if a pairing earns its flash.

## 6. V1 Section 5 — widget redesign

### 6.1 Process

Edit template `.c` → render via `lvgl-sim` → inspect real pixels → iterate. No
flashing inside the loop; the board appears at the acceptance gate. Every accepted
change lands as a golden-frame image diff. Aesthetic execution runs under the
frontend-design skill at implementation time; this spec pins direction and
acceptance.

### 6.2 System direction (fixed by this spec)

**Amended 2026-08-13 by user direction after design review.** The original
direction here was austerity: one accent hue system-wide, hierarchy from type
alone, no boxes or borders. Executed faithfully, it produced six faces the user
rejected as basic — "the pomodoro timer is very basic, the calendar looks bad,
it's just text basically, the clock is very basic too". A three-direction design
spike followed (complication / instrument / editorial); the user chose the
complication language, and it replaces the austerity clauses below. The
true-black canvas, the grid, the four type tiers and the reserved semantic
colours are unchanged.

- **True-black canvas:** `#101020` → `#000000` (AMOLED pixels off; free contrast; the
  panel edge dissolves into the bezel).
- **Per-face identity hue.** Each template kind owns a hue, returned by
  `deskmate_palette()` in three roles: `hue` (gauges, filled chips, key secondary
  figures), `tint` (the hue held back, for type on the black canvas), `ink`
  (near-black drawn from the hue, for type set *on* a hue fill). The point is a
  swipeable carousel: the hue says which card you are on before the reading is
  parsed. Both clock faces share one palette — they are the same reading in two
  notations. Semantic colors stay reserved (amber `#f2c94c` stale, red `#ff6b6b`
  error) and appear **only** in the shared state footer; no face hue may be
  confusable with either, which is why the clock hue sits at 28° rather than
  nearer stale-gold's 45°.
- **Dark surface modules.** `DESKMATE_COLOR_SURFACE` (`#1a1a1f`) cards group
  related data — a date module, a status module, one module per list entry. Dark
  enough that the AMOLED still reads the canvas as off; a face may not cover the
  canvas in surface (a full 320px dial plate was built and rejected on exactly
  this ground).
- **Gauges carry weight.** Arcs are fat (≥ 24px) with rounded caps, and a gauge's
  track is its own hue held back rather than a neutral grey, so a partly-run gauge
  reads as one object with a spent part.
- **Chips.** A face's title is a filled pill in its hue; so is any live token
  (the weather badge). An empty chip hides rather than collapsing to a blob.
  Clock faces are unlabelled as of the 2026-08-17 change; see
  `docs/superpowers/specs/2026-08-17-deskmate-clock-title-removal-design.md`.
- **Hierarchy from the four type sizes,** supported by — not replaced by — the
  surface and hue above.
- **8 px spacing grid** with named constants in `template_internal.h`, replacing
  per-template magic offsets. Baseline arithmetic derived from font metrics is
  exempt: it is measured, not chosen.
- **One state (stale/error) footer treatment** defined at the shared
  `template_view.c` layer and inherited by all templates, at `-2 * DESKMATE_GRID`.
- **Shared grammar, not six dialects.** The module/chip/eyebrow/label primitives
  live in `templates/template_style.c`; faces compose from them.

### 6.3 Per-face intent (headline; detail belongs to the redesign loop)

| Template | Redesign intent |
|---|---|
| Digital clock | Hero tabular numerals (96), date as quiet caption |
| Analog clock | Tick marks + designed hands; `show_seconds` honored and host-tested |
| Progress ring | Accent arc, hero countdown centered, state word as caption |
| Row list | Tabular time column, clear primary/secondary contrast per row |
| Big-number-label | Value to hero tier; title/label to caption tier |
| Icon-badge-text | Icon sized to the type grid, badge as accent, value as hero |

### 6.4 Acceptance

1. Full golden set (template × orientation × field matrix) approved by the user as
   images.
2. Physical board: both orientations, malformed/maximal data, byte-flat heap,
   `ui_queue_high_water` within bound (same bar as M4 Task 3), run once at the end.
3. Dev framebuffer-diff agrees with the harness on the golden cases, within the
   §3.1 partial-flush caveat.
4. Board-notes entry with observed results only; no unobserved claims.

## 7. Effect on the existing roadmap and M4 plan

The M4 plan (`2026-08-05-deskmate-m4-v1-completion.md`) stops being the active plan.
Disposition of its tasks:

| M4 task | Disposition |
|---|---|
| 1, 2, 2B, 3, 4 (complete) | Stand as delivered; their evidence records are unchanged. Cards become playlist entries; providers run in whichever host owns the device. |
| 5 — host tap actions | Reconsidered at V1 planning: "open URL/app on host" is coherent only for the Mac-owned tier. Not in V1's committed scope. |
| 6 — asset/font push | Deferred to V4 (plugin-image cache). Justification removed by server-rendered plugins carrying their own type. Replaced in V1 by the baked typeface. |
| 7 — production identity + USB firmware update | Reshaped: partition table (with OTA slots) lands in V1; the update mechanism and identity work move to V2. |
| 8 — settings experience | Reshaped around playlists (§4.6). Asset/update settings follow their features to V4/V2. |
| 9 — packaging/hardening | Deferred to V1's exit; rewritten against V1 scope at planning time. |
| 10 — exit gate | Replaced by V1's own exit gate, defined in the V1 plan. |

Open items that travel forward unchanged (still not resolved, must not be described
as resolved): the unpowered-alert loss on reconnect; `unknown_field_count` not
observable on hardware. The validation-fallback mislabeling defect is resolved by
§4.5. The `show_seconds` analog defect gains a deterministic test path via §2.3.

The roadmap document is updated in the same change to reflect V1–V4 and to mark M4
superseded (tasks 1–4 delivered, remainder redistributed). Per the working
agreement, explicit user direction (this reset) wins over repository documents, and
the affected spec/plan documents are updated rather than left to diverge.

## 8. Out of scope for V1

WiFi, TLS, provisioning, pairing, server anything, accounts, billing, plugin
contract, image cards, asset transfer, OTA update mechanism, device-side playlist
switching, raising the 8-widget wire cap, portrait orientations, dashboards/status
strip (still cancelled), and any new wire message. The protocol does not move in V1.
