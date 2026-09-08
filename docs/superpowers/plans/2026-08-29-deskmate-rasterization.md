# Stage 4 — Rasterization fallback and SVG plugins

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development

> **STATUS (recorded 2026-09-09): SOFTWARE-COMPLETE 2026-09-01 and CONFIRMED ON HARDWARE
> 2026-09-06.** Its 15 open boxes are Task 7's hardware gates, and board-notes records
> that they PASSED — the `v2.0.0-raster1` OTA installed first-try and survived the
> rollback window, capabilities read 1003, and Task 6 Step 5's on-target
> `framebuffer_diff` returned **96 total / 10 excluded / 86 identical / 0 differing**
> (predicted 96/8/88; corrected by three test-harness-only fixes). Only the
> BUSY/OTA-owner variant, which needs a pending OTA in flight, is still owed.
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps
> use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve every card revision against the actual device that will draw it: send a
native scene when that device can render it, refuse a live card visibly when it cannot,
and otherwise render the card server-side with `resvg` into a volatile full-panel image.
The same fallback makes an SVG-authored plugin a supported, bounded manifest form.

**Architecture:** `app-core` owns one pure render-negotiation decision over a
`RenderRequirements` value: device capabilities, known/installable asset digests, scene
node support, and binding kinds. The existing `PluginHost` boundary remains the dependency
breaker (`plugin -> app-core` already exists, so `app-core -> plugin` cannot): the server
implementation compiles display-list plugins, evaluates SVG plugins, and is the only place
that calls `resvg`. A raster result is a canonical 448x368 `lv_image_header_t` plus RGB565
pixels, transferred through protocol-v1 `AssetBegin`/`Chunk`/`Commit` with the existing
volatile flag, then referenced by a one-image-node scene. The device holds at most the
active and incoming raster frames in PSRAM and never writes either to flash.

**Tech Stack:** Rust (`companion/`, `resvg`/`usvg`, the existing provider and runtime
seams), ESP-IDF 5.x/C11 with LVGL 9, protocol v1, TOML manifest v2, SVG.

**Spec:** `docs/superpowers/specs/2026-08-22-deskmate-plugin-scene-rendering-design.md`
(§1 volatile assets and GC, **§3 render negotiation**, §4 wire, §5 server pipeline and
security, §7 rollout stage 4, §8 open questions, §9 risks).

**Predecessor:** `docs/superpowers/plans/2026-08-28-deskmate-plugin-manifest.md`. Tasks
1-8b are delivered. **Task 9, its physical asset-path gate, is still unobserved.** This
plan depends on that gate and does not silently inherit it as passed; see Task 7 Phase A.

---

## Scope: what stage 3b left, and what this stage is

Stage 3b made the plugin path real: config -> provider -> raw snapshot -> `PluginHost` ->
`compile_scene_with_assets` -> `PushScene`, with registry-wide durable asset provisioning
before layout. It still assumes the connected device can draw every scene it produces.
The only capability policy in `push_active_scene` is a binary shortcut: with
`SceneRender`, push; without it, silently leave the legacy widget path alone. That is not
spec §3 and it cannot handle an unknown node, a missing asset, SVG, or raster fallback.

This stage replaces that shortcut with the complete per-**(scene, device, revision)**
decision. This table is normative and gets a table-driven test before any rasterizer is
written:

| Condition | Decision |
| --- | --- |
| Scenes supported, all node kinds supported, and every referenced asset present or installable | Native scene |
| The scene uses any **live** binding and that device cannot render it natively | **Refuse**, as that card's typed `CardErrorKind::SceneRefused` |
| Otherwise | Rasterize, upload one volatile image, then push a one-node scene |

`date`, every `time:*`, every `timer.*`, angle bindings, and a timer-driven style selector
are live. `field.*` is not: it changes only when the host supplies a new value, so the
server may resolve it while rasterizing. Unknown binding namespaces are refused rather
than guessed. Rasterizing a clock is never an approximation this product accepts: at a
30-second cadence `time:HH:mm` is wrong for up to half of every minute, and exact-minute
pushes still fail on one delayed packet. **A card that would freeze says so in its
editor.** The V1 playlists plan's validation-mislabeling fix is the governing precedent:
a real failure remains typed and is never presented as some other state. The repository
already has the right visible path in `app-core/src/runtime.rs`; this stage must use it,
not invent a parallel warning.

An SVG plugin has no device-native SVG node. Its native-support predicate is therefore
false; if it contains a live binding it is refused by the same table, and otherwise it is
rasterized. SVG is not a second renderer on the device.

## What this stage does not do

- **No headless browser, now or later in this plan.** `resvg` is the only rasterizer.
  Chromium/WebKit would turn the owner's homelab, beside unrelated services, into an
  arbitrary-code-execution host; spec §5 rejects that posture explicitly.
- **No public plugin uploads and no sandbox.** That is stage 5, after a separate risk
  review. Stage 4 still loads a curated filesystem registry.
- **No script, event handler, CSS network load, external image URL, filesystem URL, or
  system-font discovery from SVG.** An SVG template is bounded declarative input rendered
  from bytes the registry loaded; it gets no ambient authority.
- **No §8 draw-buffer spike.** Moving LVGL draw buffers to PSRAM is a separate experiment
  and a separate plan. §9 explains why folding a memory-layout experiment into a feature
  change is how this repository lost days twice.
- **No new device scene-node kind.** SVG and raster fallback normalize to the existing
  `SceneImage` node. The arc/line work below exposes fields already on `SceneArc` and
  `SceneLine`; it adds no wire node.
- **No silent contract edit to manifest v1.** `docs/plugins/manifest-v1.md` is frozen.
  `[source] root`, SVG templates, and authorable arc/line bindings are an explicit,
  additive manifest **v2**, while every existing v1 manifest keeps its current meaning.

---

## Global Constraints

- Protocol stays **v1, additive only**. No message id changes. Existing v1 payloads and
  fixtures keep their meaning.
- **One new capability bit is required:** bit 9, `VolatileAssets` (numeric `512`). Bit 5
  cannot honestly mean this: every deployed stage-3 image advertises `AssetTransfer` and
  `protocol_task.c` explicitly refuses `AssetBegin { volatile: true }` as unsupported;
  `app-core/src/asset_sync.rs` likewise hard-codes every transfer to `volatile: false`.
  Reusing bit 5 would make a conforming host mis-negotiate against real firmware.
  `PROTOCOL_CURRENT_CAPABILITIES` therefore moves from **491 to 1003**. Pin `512` and
  `1003` in `companion/crates/protocol/src/message.rs` tests and pin the matching firmware
  constants/numeric current value in `firmware/host_tests/test_protocol.c`. Bit 7 sat
  defined-but-dark for most of V2; defining a bit without advertising and testing it is
  not implementation.
- Bit 8 remains the coarse promise for every scene node that current firmware knows. A
  bit per node does not scale, and this stage adds no node kind. Negotiation still walks
  the node list explicitly so a future host can classify a node newer than one device.
- Manifest v1 remains frozen. Manifest v2 has an explicit contract discriminator;
  `version = "1.0.0"` remains the plugin's content version and must not be repurposed as
  the contract version.
- Rasterization cadence is `max(provider schedule, 30 seconds)`. **Thirty seconds is a
  named constant with the reason beside it:** below that, a raster card turns ordinary
  provider churn or repeated Refresh actions into panel/network churn. Initial activation
  may render immediately; later invalidations coalesce and the newest snapshot wins at
  the next eligible instant.
- The decoded raster asset is exactly 448x368: a 12-byte little-endian
  `lv_image_header_t` (`magic = 0x19`, `cf = RGB565 = 0x12`, `stride = width * 2`) followed
  by 164,864 little-endian RGB565 pixels. Use protocol canvas constants rather than
  restating 448/368 in implementation. The decoded size is **329,740 bytes**; assert the
  arithmetic in a test.
- Every input is untrusted and bounded before expensive work: manifest, source root,
  fetched JSON, SVG source, expanded SVG, font bytes, render wall-clock, pixel buffer,
  compressed transfer, decoded length, node count, digest count, and PSRAM slot count.
  Every bound is a named constant with a one-line reason and a one-past rejection test.
- **Firmware memory layout is the dominant risk, and it is historical, not theoretical.**
  Commit `3f2aa03` added about 105 bytes of static internal DRAM and broke every OTA
  download with every test green; the board bisect proved layout, not logic or capacity.
  Put volatile bytes and tables in PSRAM heap, add no file-scope buffers, record same-tree
  `.bss`, DIRAM `.text`, `.data`, IRAM, and DIRAM-total deltas, run
  `make -C firmware/host_tests sanitize`, and re-verify the OTA download on the physical
  board. **Green tests say nothing about whether OTA still works.**
- `AssetRelease.digests` is the complete **keep-set**, not a delete-list. A digest absent
  from it is marked DEAD. An empty keep-set means wipe everything, so the stage-3b guard
  that sends no release for an empty desired set remains load-bearing.
- Run both test invocations from `companion/`; neither covers the workspace alone:
  `cargo test --workspace --all-targets` **and** `cargo test --workspace --doc`.

---

## Six findings from stage 3b that shape these tasks

**1. The `PluginHost` trait object is an architecture boundary, not decoration.** The
`plugin` crate depends on `app-core`; reversing that edge creates a dependency cycle.
Negotiation belongs in `app-core`, while `resvg`, plugin manifests and registry files stay
server-side behind the injected host. The Tauri runtime still injects no server host; if
it ever needs a raster fallback, it reports the same typed refusal rather than linking a
server renderer into the app by accident.

**2. `AssetRelease` is a keep-set, and an empty registry was destructive.** Task 8b's
first finding came from a real bug: unconditional reconcile sent `digests: []`, which
means device-wide deletion. Rasterization adds a changing volatile digest to the same
ownership problem. There must be one union of durable plugin digests plus the currently
active volatile digest, and every failure-path test asserts the exact request sequence.

**3. Both curated endpoints are `example.invalid`, so the operator route is part of the
gate, not a fake second compiler.** Task 8b added `POST /v1/devices/{id}/scene` so the
committed fixtures could drive the real `ServerPluginHost` without depending on a third
party. Stage 4 moves that route onto the negotiated runtime path; it must not keep its
current direct `PushScene` bypass, or the only runnable hardware gate would skip the
policy this stage exists to prove.

**4. The provider returns a raw envelope and the manifest binds its inner value.** The
fixtures are `{"status":"ok","payload":{...}}`, production's
`parse_json_payload` returns that raw value, and the manifests address `data.current...`.
`crates/plugin/tests/aqi_fixture.rs:23-26` records the wrapper as deliberate. The fix is
the carried `[source] root = "payload"` debt from Task 8b's third finding: select the
root **before** `LastGood::complete`, so a missing root is a typed provider failure and a
previous good inner value remains last-good. Do not rewrite the manifests to
`data.payload.*`, flatten the hostile fixtures, or hard-code a universal envelope.

**5. A plugin golden is not the old parity gate.** There is no C implementation of an
SVG plugin or a rasterized scene to compare against. A checked image can prove that a
particular renderer version is deterministic and make review possible; it cannot prove
that the pixels are the intended ones. The strong evidence available here is split:
semantic/unit tests for the decision and translator, hostile-input tests for security,
pixel probes for simple known geometry, exact payload-to-device capture for the image
delivery path, and eyes on the real panel. No task calls a raster golden “parity.”

**6. Stage 3b's physical gate has not happened.** The plugin provider, SSRF guard, durable
asset transfer, runtime font, image node, `field.*`, and four-step GC teardown are wired
but unobserved on hardware. Task 7 begins on the predecessor image and pays that debt
before installing stage 4. A failure there invalidates the baseline and stops the stage-4
flash; it is not relabelled as a stage-4 defect.

### Carried evidence debt, and one source disagreement to settle rather than repeat

- **`[source] root`** originates in stage 3b Task 8b's third finding. Task 1 specifies and
  implements it as manifest v2 and tests the real raw fixture producer.
- **The arc/line binding gap** originates in stage 3b's compiler: `SceneArc.end_binding`
  and `SceneLine.angle_binding` are always emitted as `""`, because neither curated v1
  plugin needed them. Task 1 closes it in v2 using fields already present on the wire;
  this stage adds no node kind. Those bindings are also load-bearing refusal fixtures.
- **`timer.remaining`, `timer.pct`, numeric-tier step-down, and date truncation** were
  carried in `CLAUDE.md` as missing pixel evidence. The live tree is not perfectly aligned
  with that sentence: `lvgl-sim/src/cases.rs` already contains a synthetic arc whose
  `end_binding` is `timer.pct`, and `scene_parity.rs` already derives HERO/DISPLAY boundary
  rows for `BigNumberLabel`; `timer.remaining` is emitted by `ProgressRing`. Those tests
  still inject or construct their inputs, and no manifest emits `timer.pct`. Task 6 must
  inventory the exact rows and state the narrower truth before adding anything: coverage
  is not use, and injected input does not prove producer meaning. The date node is
  structurally `ellipsize: true`, but no located case forces the actual localized date
  string past its 176-pixel content box. That is a genuine missing case.

---

## Manifest v2, concretely

Manifest v1 has no contract-version field and means “top-level display-list nodes.” V2 is
explicit:

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

A v2 display-list plugin writes `[template] kind = "scene"` and keeps `[[nodes]]` /
`[[repeats]]`. An SVG template may use the existing restricted `{{ ... }}` expressions
only as the entire value of an XML text node or attribute. Results are XML-escaped before
`usvg` parses the final document. A binding-namespace expression is retained as a
requirement long enough for negotiation to apply the refuse rule; it is never baked into
pixels. Partial interpolation stays out: author adjacent `<tspan>` elements rather than a
second string-template language.

V1 remains accepted exactly as today with no `manifest_version`, no `[template]`, no
`[source].root`, fixed arc ends and fixed point-list lines. V2 adds:

- `[source].root`: a bounded dotted object path such as `payload`; missing/null/wrong-shape
  is a named provider error before `LastGood` is updated.
- `arc.end_binding = "{{ timer.pct }}"` or `"{{ timer.permille }}"`, alongside the
  declared start/end sweep the device scales.
- a bound `line` geometry carrying `pivot_x`, `pivot_y`, `length`, and
  `angle_binding = "{{ time:angle:minute }}"`, mutually exclusive with `points`.
- `[template] kind = "svg", file = "..."`, with `file` held to the same single-component
  and canonical-containment posture as an asset file.

---

## File Structure

**Create:**

- `docs/plugins/manifest-v2.md` — the additive authoring contract; v1 stays frozen.
- `companion/crates/app-core/src/render_negotiation.rs` — the pure decision table and
  binding/asset/node requirement analysis.
- `companion/crates/server/src/rasterizer.rs` — scene/SVG -> bounded `resvg` pixmap ->
  canonical RGB565 asset; no browser and no ambient resource loader.
- `companion/crates/server/tests/fixtures/svg-plugins/` — a static SVG plugin, a live-SVG
  refusal plugin, hostile SVG inputs, and real envelope fixtures for the hardware/admin
  path.
- `companion/crates/server/assets/Inter-Regular.ttf`, `Inter-SemiBold.ttf`, `OFL.txt` —
  deployable, licensed font bytes copied byte-for-byte from `tools/fonts/`; `resvg` loads
  these and plugin-declared font assets only, never host system fonts.
- `firmware/main/core/volatile_asset_store.h/.c` — a bounded caller-allocated directory
  over PSRAM-backed blobs, hardware-independent and host-tested.
- `firmware/host_tests/test_volatile_asset_store.c` — reserve/commit/release/corruption and
  two-slot swap tests.
- `companion/plugins/svg-aqi/` — one curated v2 SVG plugin using the existing hostile AQI
  envelope shape and `[source] root = "payload"`.

**Modify:**

- `companion/crates/plugin/src/manifest.rs`, `compile.rs`, and focused tests — versioned
  manifest parsing, source roots, SVG metadata, and the arc/line binding forms.
- `companion/crates/server/src/plugin_provider.rs`, `plugin_registry.rs`,
  `plugin_host.rs`, `plugin_refresher.rs`, `admin.rs` — root selection, bounded SVG load,
  raster execution, and one negotiated operator path.
- `companion/crates/app-core/src/runtime.rs`, `asset_sync.rs`, `scheduler.rs`, `state.rs` —
  render requirements, device inventory, volatile asset ownership, cadence and typed
  refusal.
- `companion/crates/protocol/src/message.rs`, `lib.rs`; `docs/protocol/v1.md` — bit 9 and,
  if RLE wins Task 3's measurement, additive optional `AssetBegin` encoding metadata.
- `firmware/main/core/protocol_message.h/.c`, `asset_transfer.h/.c`,
  `firmware/main/link/protocol_task.c`, `firmware/main/CMakeLists.txt` — capability,
  volatile transfer, resolver and release integration.
- `companion/crates/lvgl-sim/src/cases.rs`, focused tests, and
  `companion/crates/device/examples/framebuffer_diff.rs` — honest regression images,
  carried binding/date cases, and exact raster-payload delivery rows.
- `companion/apps/deskmate/tests/components.test.tsx` — the live-binding refusal is visible
  in that card's editor as `scene-refused`.
- `CLAUDE.md`, `docs/hardware/board-notes.md` — only when executing the close/hardware
  tasks, with observed and unobserved facts separated.

---

## Task ordering, and where the hardware cost sits

Tasks 1-3 are server/contract work. Task 4 is the only firmware feature wave; collect
every firmware need from Tasks 1-3 before landing it, so capability, PSRAM storage,
optional transfer decoding, resolver changes and release semantics ride **one image**.
Tasks 5-6 finish host integration and the software evidence.

**Task 7 is the only hardware session.** It has two phases in that same session:

1. on the existing predecessor image, execute stage 3b Task 9 and establish the durable
   asset/GC baseline;
2. only if Phase A passes, install the one stage-4 image and execute this stage's OTA,
   volatile-raster, refuse-rule, 30-second-floor and orientation observations.

This is one physical session and one stage-4 OTA cycle. It does not pretend stage 3b's
deferred gate already passed, and it does not mix an unexplained predecessor failure with
new firmware.

| Task | Kind | Hardware |
| --- | --- | --- |
| 1-3 | manifest, policy, server renderer | — |
| 4 | one firmware wave + host transfer changes | — |
| 5-6 | runtime integration and software evidence | — |
| 7 | **single session:** inherited stage-3b gate, one OTA, stage-4 observations | ✅ |
| 8 | docs / stage-5 risk-review handoff | — |

---

### Task 1: Manifest v2 — source roots, SVG, and the arc/line binding debt

**Files:**
- Create: `docs/plugins/manifest-v2.md`, `companion/plugins/svg-aqi/`, focused SVG/root
  fixtures
- Modify: `companion/crates/plugin/src/manifest.rs`, `compile.rs`, tests;
  `companion/crates/server/src/plugin_provider.rs`, `plugin_registry.rs`

**Interfaces:**
- Produces an explicit `ManifestVersion`, a v2 `Template::{Scene,Svg}`, and
  `Source::Json { url, refresh_minutes, root }` without changing any v1 parse result.
- Produces a registry-owned, bounded `Arc<str>` SVG source read once after canonical
  containment, and real `SceneArc.end_binding` / `SceneLine.angle_binding` values for v2
  scene manifests.

**Evidence model:** Parser tests prove contract/version/bounds; producer tests against the
raw committed HTTP fixture prove root selection. Compiler shape tests prove fields reach
the protocol model. None of those prove pixels; Task 6 owns pixel evidence.

- [x] **Step 1: Write the failing v1/v2 contract tests.** A v1 manifest containing
      `manifest_version`, `[source].root`, `[template]`, `arc.end_binding`, or a bound-line
      field is rejected, while each explicit v2 form parses. Reparse the two existing
      curated v1 manifests and assert their typed values are unchanged.
- [x] **Step 2: Write the failing producer test with the real raw AQI fixture.** Feed
      `{"status":"ok","payload":...}` through `PluginDataProvider`, not directly into
      the compiler, and assert the resulting snapshot value is the inner `payload` when
      `root = "payload"`. Then remove/replace/wrong-type `payload`: the failure is named,
      classified permanent for that response shape, and a previous inner value remains
      last-good/stale. This test exists because supplying the compiler its preferred
      shape proves nothing about the provider that produces it.
- [x] **Step 3: Run the focused tests and watch them fail.** From `companion/`:
      `cargo test -p plugin` and the named `server::plugin_provider` tests.
- [x] **Step 4: Define and test the bounds.** Reuse the expression path-segment ceiling
      rather than copying it for `root`; give SVG source and expanded-SVG byte caps named
      constants with one-line memory/parse reasons; validate `template.file` as one normal
      path component and re-check canonical containment after joining, so a symlink cannot
      escape the plugin directory.
- [x] **Step 5: Implement versioned parsing and root selection.** `manifest_version` is
      absent only for v1 and exactly `2` for v2. Select root after successful JSON parse
      and before `LastGood::complete`. There is no universal envelope fallback.
- [x] **Step 6: Implement the v2 template union.** `kind = "scene"` permits nodes/repeats
      and forbids `file`; `kind = "svg"` requires one file and forbids nodes/repeats.
      Load SVG once in the registry with a bounded read; do not re-read it per refresh.
- [x] **Step 7: Close the arc/line compiler gap.** Write failing tests first for a bound
      arc and bound line, plus both ambiguous forms (line has both `points` and pivot
      geometry; line has neither). Accept only protocol-valid binding tokens in their
      correct numeric/angle positions. `protocol::validate_scene` remains the final
      authority; do not restate its canvas/binding bounds.
- [x] **Step 8: Hostile inputs.** One-past root depth/length, missing root, scalar at an
      intermediate root, absolute/parent/subdirectory/symlink SVG paths, over-cap SVG,
      invalid UTF-8, both line geometries, neither line geometry, and a live-looking typo
      such as `timer.velocity`. Every case is a named error, never panic or silent literal.
- [x] **Step 9: Freeze `docs/plugins/manifest-v2.md`.** State explicitly that v1 is still
      frozen, that `version` is not the contract discriminator, and that SVG live bindings
      are requirements for negotiation rather than strings `resvg` may freeze.
- [x] **Step 10: Commit.** Landed as `334e99a feat: specify manifest v2 and add the resvg rasterizer core`, which also carries Task 3 Steps 1/4/5 (the two tasks were built in parallel on disjoint files and gated together).

---

### Task 2: The render-negotiation table, pure and per device

**Files:**
- Create: `companion/crates/app-core/src/render_negotiation.rs`
- Modify: `companion/crates/app-core/src/lib.rs`, `runtime.rs`, `state.rs`;
  `companion/crates/server/src/plugin_host.rs`
- Test: focused app-core table/property tests and runtime tests

**Interfaces:**
- Produces `RenderRequirements { native_source, node_kinds, asset_digests, bindings }`,
  `DeviceRenderProfile { capabilities, confirmed_assets, installable_assets }`, and
  `RenderDecision::{Native, RefuseLive, Rasterize}`.
- `PluginHost` produces either a display-list scene candidate or an SVG raster-only
  candidate; it does not decide policy independently.

**Evidence model:** This is a pure policy task. Exhaustive table tests prove the decision
for named inputs and property tests prove irrelevant revisions/devices do not share cached
answers. They do not prove that upload or rendering executes; Tasks 3-5 do.

- [x] **Step 1: Write the failing decision table verbatim from spec §3.** Cover native
      success, one unknown/unsupported node, one missing-but-installable asset, one
      missing-and-uninstallable asset, static SVG, live SVG, and a device without scene
      support. Assert the decision, not merely that some error occurred.
- [x] **Step 2: Write the failing binding classification tests.** `date`, `time:*`, every
      `timer.*`, bound arc/line geometry, and `running_color` driven by timer state are
      live; `field.*` and literals are not. An unrecognized namespace is an analysis error,
      never “probably static.”
- [x] **Step 3: Prove the cache key is `(card/scene, device, revision)`.** The same scene
      is native on a current device and rasterized/refused on an older profile; reconnect
      with changed capability bits recomputes; a new revision with a changed binding set
      recomputes; one device's confirmed digest never makes another device “present.”
- [x] **Step 4: Run and watch the tests fail.** `cargo test -p app-core render_negotiation`.
- [x] **Step 5: Implement requirement analysis by walking the actual protocol `Scene`.**
      Collect image/glyph/asset-font digests and every binding-bearing field. Keep node
      support explicit even though bit 8 covers all nine current node kinds. There is no
      new node bit because this stage adds no node kind.
- [x] **Step 6: Implement the pure decision and delete the binary shortcut.** The current
      `push_active_scene` branch that silently returns when `SceneRender` is absent must no
      longer decide policy. Do not add the legacy widget renderer as a fourth implicit row;
      stage 4 follows the approved table.
- [x] **Step 7: Use the existing typed visible path.** `RefuseLive` records
      `CardErrorKind::SceneRefused` against exactly that card, preserves the prior display,
      and clears only after that same card has an accepted render. The message says which
      live binding/device support caused the refusal and what upgrade/action fixes it.
- [x] **Step 8: Commit.** `feat: negotiate every scene against its device`

**Task 2 execution notes (2026-09-01).** Four recorded deviations/decisions:

1. **`crates/plugin` gained one public function outside this task's file list:**
   `device_binding_requirements(&str) -> BTreeSet<String>` in `compile.rs`, beside the
   private `looks_like_binding_namespace` it reuses. The alternative was duplicating the
   device-binding namespace decision in server code, which is exactly how two copies
   come to disagree (the repo's own icon-table precedent). Task 3's file list already
   anticipated a shared entry point in this crate.
2. **`server/src/admin.rs` was adapted, not redesigned:** the operator scene route now
   destructures `SceneCandidate` and refuses a raster-only (SVG) plugin with a typed
   message. That route pushes native scenes only; SVG goes through Task 5's executor.
3. **The `Rasterize` decision's interim behaviour is a typed refusal** naming the
   missing server-side rasterization, recorded through the same `SceneRefused` path.
   Task 5 replaces that arm with the real executor. Silence -- the deleted shortcut's
   behaviour -- was not an option, per the validation-mislabeling precedent.
4. **The old shortcut's side effect is intentionally gone:** a device without bit 8
   used to get any stale `SceneRefused` *cleared* and nothing recorded. Under the
   table, such a device now gets a fresh, accurate decision per push (refuse-live for
   clock/timer cards, interim rasterize-refusal for static ones), which is the plan's
   stated intent ("a card that would freeze says so in its editor").

`DeviceRenderProfile` construction at the runtime call site: `capabilities` =
`DeviceSnapshot::capability_bits()` (unknown bits survive), `confirmed_assets` = empty
(the runtime keeps no per-device ledger; content addressing at `AssetBegin` is the
inventory protocol), `installable_assets` = the plugin host's desired-asset digests,
fetched only when the scene references any digest. Positional bindings
(`time:hour`/`time:minute`/`time:second`, `time:angle:*`) are classified inside
`analyze_scene` because `binding_is_valid` correctly rejects them in value position;
`running_color: Some(_)` on arc/text registers the `running_color` live pseudo-token.
Verification: 19 table/classification/property tests in `render_negotiation.rs`, 3
push-path tests in `runtime.rs` (refuse-live recorded with binding+fix named, native
push clears a prior refusal, static card names the missing rasterizer), an SVG
raster-only candidate test in `plugin_host.rs`, 3 scanner tests in `compile.rs`.

---

### Task 3: The only rasterizer — `resvg` to canonical RGB565

**Files:**
- Create: `companion/crates/server/src/rasterizer.rs`, `companion/crates/server/assets/*`,
  SVG hostile/known-shape fixtures
- Modify: `companion/crates/server/Cargo.toml`, `plugin_host.rs`, `plugin_registry.rs`,
  `companion/crates/plugin/src/expr.rs` only if a shared bounded evaluation entry point is
  needed
- Test: rasterizer unit/property tests and server integration tests

**Interfaces:**
- Produces `RasterizedFrame { digest, bytes }`, where `bytes` is the decoded device-native
  LVGL image blob, `digest = SHA-256(bytes)`, and the fixed dimensions are implied by the
  protocol canvas constants and canonical LVGL image header.
- Accepts either a protocol `Scene` plus resolved non-live field inputs/assets, or one
  registry-owned v2 SVG template plus its provider snapshot.

**Evidence model:** Pixel probes against simple solid/alpha/geometry cases prove channel
packing and canvas placement. Structural tests prove translation and security posture.
Checked PNGs/hashes are regression artifacts for review, **not a C oracle and not
parity**. The device displaying the exact payload is a separate Task 6/7 claim.

- [x] **Step 1: Write the failing canonical-output tests.** A solid red 448x368 SVG yields
      a 12-byte little-endian header with the exact magic/format/dimensions/stride, exactly
      164,864 pixels and 329,740 total decoded bytes. Probe red, green, blue, black, white,
      and 50%-alpha-over-black pixels so RGB565 channel order and alpha compositing cannot
      be accidentally swapped and still pass a same-colour fixture.
- [x] **Step 2: Write the failing scene-translation tests.** Exercise every current scene
      node, clip, opacity, alignment, baked tier, asset font, glyph, image and non-live
      `field.*` value. Supply fields through the real `latest_fields` producer shape, not a
      pre-substituted scene. Hand a live binding to this layer and assert refusal: the
      rasterizer is defence in depth behind negotiation, not a place a frozen clock can
      sneak through.
- [x] **Step 3: Write the failing SVG-expression tests.** Evaluate exact text-node and
      attribute expressions against the real rooted fixture, XML-escape `&<>'"`, meter all
      expressions under one fuel budget, and reject partial interpolation. A live binding
      remains a `RenderRequirements` fact and is never replaced with a one-time value.
- [x] **Step 4: Write the hostile SVG corpus before adding `resvg`.** Oversized source and
      expansion, excessive XML depth/node count, script, event attributes, animation,
      `foreignObject`, external/data/file/http image references, external stylesheets,
      CSS `url()`, missing fonts and pathological dimensions all fail by name within the
      render budget. Configure `usvg` with no resource directory, no network callback, and
      a font database containing only the two committed Inter files plus this plugin's
      already-resolved font assets. Never call `load_system_fonts`.
- [x] **Step 5: Run and watch them fail.** `cargo test -p server rasterizer`.
- [x] **Step 6: Implement scene -> SVG and SVG-template evaluation, then call `resvg`.**
      There is one render engine. Do not shell out and do not add a browser crate. Load
      the committed fonts with `include_bytes!`; add tests pinning their SHA-256 and
      byte-equality to the licensed `tools/fonts/` sources so the deployable copies cannot
      drift silently.
- [x] **Step 7: Encode RGB565 explicitly.** Composite onto the scene's opaque background,
      quantize with one documented formula, write every `u16` little-endian, prepend the
      exact 12-byte header shape used by `framebuffer_diff.rs::asset_wire_bytes`, hash the
      decoded blob, and call `protocol::validate_scene` on the one-image-node scene.
- [x] **Step 8 (measurement half): Honour spec §4's RLE direction without hiding a wire change.** Measure RLE
      on the curated SVG frame and a hostile high-entropy frame. If it wins, add optional
      additive `AssetBegin` encoding/decoded-length keys gated by bit 9, round-trip tests,
      and a bounded pure-C streaming decoder in Task 4; raw remains the legal fallback
      when RLE expands. If the live wire cannot carry this safely within the existing
      frame/PSRAM bounds, record the measured result and amend this plan/spec explicitly
      before shipping raw-only — do not silently claim the spec's `RGB565 -> RLE` pipeline.
- [x] **Step 9: Add review images with honest labels.** One display-list fallback and the
      curated SVG plugin at fresh/stale/error/missing-data states and both logical
      orientations. The test name and failure text say “raster regression,” never
      “parity.” Dump-and-look at every new image before accepting it.
- [x] **Step 10: Commit.** Landed across `334e99a` (Steps 1/4/5) and the Task-3-remainder commit (Steps 2/3/6/7/8-measurement/9).

**Task 3 execution notes (2026-09-01).** The RLE measurement (Step 8) is decisive and
recorded here as the plan requires: encoding each RGB565 run as `(u16 count LE, u16
pixel LE)`, the curated `svg-aqi` frame compresses **329,728 -> 10,020 bytes (96.96%
smaller)** while a deterministic high-entropy frame expands to exactly **2x (659,456)**.
Raw must therefore remain the legal fallback, and RLE is worth carrying on the wire:
at ~2 KiB chunks over one-outstanding-request, a raw frame is ~161 round trips per
push against a 30-second cadence, an RLE frame ~5. **The wire half (additive
`AssetBegin` encoding/decoded-length keys gated by bit 9, plus the bounded pure-C
streaming decoder the original Step 8 assigned to Task 4) is a follow-up wave in this
stage, not silently dropped.** Other notes: the generated-SVG path may embed
module-generated data URIs (scene image nodes re-encoded as pixel-path SVG) because it
is trusted output rendered through the same hardened options -- plugin-authored input
still rejects every data URL; the rasterizer always draws the logical 448x368 canvas,
so orientation cannot change its bytes (asserted by a test rather than fabricated
orientation goldens); goldens under `crates/server/tests/raster-regression/` are
byte-pinned PNGs updated only via `UPDATE_RASTER_GOLDENS=1`, named and messaged
"raster regression", never "parity".

**RLE wire follow-up (2026-09-01).** The follow-up wave above is delivered.
`AssetBegin` keys **4** (`encoding`, absent = raw, `1` = RLE565) and **5**
(`decoded_length`, required iff non-raw) are additive under the existing bit 9. **Key 3
(`volatile`) is still ALWAYS emitted, false included** -- the implementing agent had
"fixed" it to omit-when-false per the canonical emission rule, and the controller
reverted that: every deployed firmware decoder had `REQUIRED_BIT(3)` in its mask, so an
omitting server would fail every durable AssetBegin against the fleet with
MISSING_FIELD. The new decoders tolerate absence (belt and braces), the encoders do not
exercise it, and `asset_begin.bin` is byte-identical to the deployed corpus. Only keys
4/5 follow the omit-default rule, because no deployed decoder knows them.
`total_length` remains the wire length, while digest and scene
references address the decoded canonical blob. `protocol::encode_rle565` preserves the
measured 10,020/659,456-byte pixel-stream results; app-core prepends the unchanged
12-byte header, selects RLE only when bit 9 is advertised and the complete encoded blob
is strictly smaller, and otherwise sends raw.

---

### Task 4: Volatile assets in PSRAM, plus the capability that tells the truth

**Files:**
- Create: `firmware/main/core/volatile_asset_store.h/.c`,
  `firmware/host_tests/test_volatile_asset_store.c`
- Modify: protocol Rust/C mirrors and tests; `firmware/main/core/asset_transfer.h/.c`,
  `firmware/main/link/protocol_task.c`, `firmware/main/CMakeLists.txt`;
  `companion/crates/app-core/src/asset_sync.rs`, `runtime.rs`;
  `companion/crates/server/src/plugin_registry.rs`; `docs/protocol/v1.md`

**Interfaces:**
- Bit 9 (`VolatileAssets`) means the device accepts and resolves volatile image assets;
  bit 5 continues to gate the four asset-transfer message types generally.
- A volatile store holds at most two committed/incoming frame allocations: the displayed
  frame and its atomic replacement. All storage comes from caller-provided PSRAM
  allocations; the pure core owns only bounded metadata passed by pointer.

**Evidence model:** Pure-C host tests and ASan prove bounds/lifetimes independent of
ESP-IDF. Exact host request-sequence tests prove tier flags and keep-set ownership. They
cannot prove PSRAM capability, LVGL pointer lifetime, memory layout, or OTA; Task 7 does.

- [x] **Step 1: Write the failing capability tests in both languages.** Rust pins
      `CAPABILITY_VOLATILE_ASSETS == 512` and `CURRENT_CAPABILITIES == 1003`; firmware pins
      the same bit and numeric `PROTOCOL_CURRENT_CAPABILITIES == 1003`, and asserts an
      advertised bit-9 build admits volatile `AssetBegin`. Run them and see 491 fail.
- [x] **Step 2: Write the failing two-slot store tests.** Begin/ordered chunks/commit/find,
      duplicate `AssetBegin` -> already-present, interrupted incoming frame frees only the
      incoming allocation, old active frame survives, a third live allocation is refused,
      and release frees exactly digests absent from the keep-set. Two slots is a named
      bound: one displayed plus one incoming is the minimum atomic swap and prevents
      unbounded 330-KiB PSRAM accumulation.
- [x] **Step 3: Hostile frame tests.** Volatile kind other than `Image`, zero/over-limit
      length, wrong digest, duplicate/out-of-order/overlapping chunk, incomplete commit,
      malformed 12-byte header, wrong format/dimensions/stride, decoded-length overflow,
      truncated/expanding RLE if selected, and allocation failure all refuse without
      losing the prior committed frame. Run under ASan.
- [x] **Step 4: Implement the pure store and PSRAM adapter.** Allocate with explicit
      `MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT`; store no frame bytes or slot arrays at file
      scope. A reboot naturally loses the directory. The asset resolver checks committed
      volatile digests before flash, and returns the same pointer/length/kind contract
      `scene_view` already consumes.
- [x] **Step 5: Integrate all transfer exits.** A second begin, bad chunk, disconnect,
      commit failure, release and shutdown each free the right incoming/committed buffer
      once. Durable transfers keep their current flash path. Never downgrade volatile to
      flash: that would spend partition endurance every 30 seconds.
- [x] **Step 6: Make `AssetRelease` own both tiers.** Keep-set = all desired durable plugin
      digests + the active volatile digest. Reduce the registry's durable ceiling from 32
      to `MAX_ASSET_DIGESTS - 1`, with the one-slot reason beside the constant; otherwise a
      valid 32-durable-asset catalog leaves no legal keep-set for one raster frame. Keep
      the empty-desired guard and add exact-sequence tests for empty registry, durable-only,
      raster-only, and their union.
- [x] **Step 7: Preserve the GC teardown safety sequence.** A release that can move/free
      bytes used by the active scene must load the standalone clock -> destroy scene ->
      reset font registry -> collect/compact/free -> rebuild from retained `scene_t`, on
      every exit. A release that removes only the old raster while the active scene reads
      the **kept new** volatile digest must not flap through the clock. Test the pure
      keep/use predicate; state plainly that the end-to-end sequence spans
      `link/protocol_task.c`, `link/asset_flash.c`, `ui/font_registry.c`, and
      `ui/scene_view.c`, with no automated test or simulator seam across all four.
- [x] **Step 8: Run firmware gates.** `make -C firmware/host_tests clean test` and
      `make -C firmware/host_tests sanitize`. Build before/after from the same tree and
      record `.bss`, DIRAM `.text`, `.data`, IRAM and DIRAM total. Any IRAM growth or
      unexplained file-scope static growth stops the task before hardware.
- [x] **Step 9: Commit.** `feat: hold raster frames as volatile PSRAM assets`

**Task 4 execution notes (2026-09-01).** Capability bit 9 (`VolatileAssets` = 512)
lands in both languages; `CURRENT_CAPABILITIES`/`PROTOCOL_CURRENT_CAPABILITIES` are
now **1003**, pinned by tests on both sides plus `protocol/tests/fixtures.rs`. The
volatile store is pure C11 (`core/volatile_asset_store.{h,c}`) with caller-provided
allocate/deallocate/SHA-256 callbacks; the PSRAM adapter in `link/protocol_task.c`
uses `MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT` and mbedTLS SHA-256. The store object
lives inside the PSRAM-allocated protocol context -- file-scope statics added by the
new C: **0 bytes**. `scene_reads_asset_bytes()` was replaced by the pure, host-tested
`volatile_asset_store_release_must_teardown()`: it preserves the old conservatism
(any durable asset use forces teardown, because flash compaction moves bytes) and
adds the one safe relaxation (a scene reading only the KEPT volatile digest skips
the clock flap, because a PSRAM allocation never moves). AssetBegin gains a
payload-aware second gate (`protocol_asset_begin_request_gate`): bit 5 still gates
the four message types, bit 9 additionally gates the volatile form. App-core gains
`AssetSync::transfer_volatile[_yielding]` and `compose_asset_keep_set` (durable +
active volatile, deduplicated, wire-ceiling-checked); the server registry's durable
ceiling is now `MAX_DURABLE_REGISTRY_ASSETS = MAX_ASSET_DIGESTS - 1` with an exact
boundary test, reserving one keep-set slot for the raster frame. **Size check on the
same tree (before -> after): DIRAM total 203,891 -> 203,891, `.bss` 87,128 -> 87,128,
`.data` 23,128 -> 23,128, DIRAM `.text` 93,635 -> 93,635, IRAM 16,384/16,384 with 0
remaining -> unchanged; only flash grew (code +2,396, rodata +144).** Internal RAM
layout is byte-identical, which is the best achievable posture against the OTA
layout hazard -- and per the standing rule, still not proof: Task 7's on-board OTA
download check remains mandatory. Deferred to Task 5: executor wiring and exact
request sequencing. Firmware has no normal protocol-task shutdown seam (infinite
task); store destruction is tested and used on task-start failure, and disconnect/
release paths abort incoming buffers.

The RLE follow-up adds `core/rle565.{h,c}` with caller-owned streaming state and the
`init`/`feed`/`finish` API. `volatile_asset_store_begin_encoded` allocates the promised
decoded length in PSRAM, copies the 12-byte header raw, streams arbitrary RLE chunk
boundaries into that allocation, and hashes the decoded bytes at commit; the original
`volatile_asset_store_begin` remains the raw wrapper. There are still zero new
file-scope statics, and the controller's same-tree size check confirms it: DIRAM total
203,891, `.bss` 87,128, `.data` 23,128, IRAM 16,384/16,384 all byte-identical to the
Task-4 build; flash code +956. The board OTA download check (Task 7) still stands.
Verification: both full host targets pass, including `test_rle565: OK (8 tests)` and
`test_volatile_asset_store: OK (21 tests)` under ASan/UBSan; Rust reports protocol
56 unit + 7 fixture tests, app-core 98 library tests, and server rasterizer 29 tests.

---

### Task 5: Execute negotiation, atomically, on the provider schedule

**Files:**
- Modify: `companion/crates/app-core/src/runtime.rs`, `asset_sync.rs`, `scheduler.rs`;
  `companion/crates/server/src/plugin_host.rs`, `plugin_refresher.rs`, `admin.rs`,
  `runtime_device.rs`; focused runtime/server tests
- Modify: `companion/apps/deskmate/tests/components.test.tsx`

**Interfaces:**
- Produces one event-driven render executor whose outcomes are native scene, visible
  refusal, or volatile upload + one-node scene.
- The operator fixture route injects a plugin snapshot into that executor; it no longer
  compiles and calls `RuntimeHandle::push_scene` around negotiation.

**Evidence model:** Fake-clock runtime tests prove cadence without sleeping. Exact hostile
device transcripts prove atomic ordering and failure cleanup. The component test proves
the typed error is shown in the editor. Hardware Task 7 proves real time and transport.

- [x] **Step 1: Write the failing exact-transcript tests.** Native asset case:
      install/confirm required assets before `PushScene`, no volatile transfer. Raster
      case: volatile `AssetBegin(true)` -> all chunks -> commit -> one-image `PushScene`
      -> `AssetRelease` containing durable + new volatile. Refuse-live case: no asset or
      scene frame at all, one `SceneRefused` on that card.
- [x] **Step 2: Write every failure transcript.** Fail begin/chunk/commit/push/release in
      turn. Before a successful new `PushScene`, the old volatile digest stays kept and
      displayed; after a successful push, release may drop the old digest. An incomplete
      pass never sends a keep-set that deletes desired bytes. `Busy` is retried; permanent
      device rejection becomes the same card's `SceneRefused`; link failure remains a link
      failure, not a card fault.
- [x] **Step 3: Write the failing fake-clock floor test.** Provider/manual invalidations at
      `t=0`, `t=5s`, `t=29.999s`, and `t=30s` produce raster pushes at `t=0` and no earlier
      than `t=30s`; the second frame contains the newest snapshot. A provider interval of
      2 minutes remains 2 minutes, while a test-only 5-second policy becomes 30 seconds.
      Native scenes remain event-driven and are not delayed by a raster-only floor.
- [x] **Step 4: Run and watch them fail.** `cargo test -p app-core` focused runtime tests
      and `cargo test -p server` focused admin/device transcript tests.
- [x] **Step 5: Retain per-device inventory honestly.** `AssetBegin.already_present` and a
      successful commit are the only digest observations; there is no inventory-list
      message. Durable confirmations may be re-checked on reconnect; volatile
      confirmations are connection/boot-epoch scoped and never inferred from flash stats.
- [x] **Step 6: Implement scheduling and coalescing.** `RASTER_MIN_INTERVAL = 30s` sits by
      its reason. Initial activation is immediate. Provider result, config/data-state
      change, playlist activation and operator injection mark a raster candidate dirty;
      one scheduler deadline wakes it, and repeated events replace the pending snapshot
      rather than queue frames.
- [x] **Step 7: Move the operator route onto the same path.** It may still accept committed
      fixture data for the hardware gate, but it writes the configured plugin card's
      cached snapshot and asks the runtime to render. There remains one plugin compiler,
      one negotiation table and one rasterizer. Reject a plugin/card mismatch and preserve
      the existing body-size/admin-auth bounds.
- [x] **Step 8: Prove the editor wording.** Extend the existing `scene-refused` component
      test with a live SVG/unsupported-device message and assert it appears inside that
      card's editor, not as “stale,” “last working settings,” or a global transport error.
      This is the V1 validation-mislabeling precedent made executable.
- [x] **Step 9: Commit.** `feat: deliver negotiated raster frames on a 30 second floor`

**Task 5 execution notes (2026-09-01).** The `PluginHost` boundary gained
`rasterize(&RasterRequest) -> RasterFrame` with app-core-owned request/frame types
(`DisplayList { scene, fields }` / `PluginSvg { plugin_id, snapshot, fields }`;
digest + `Arc<[u8]>`), keeping the dependency direction intact; the default trait
implementation preserves the no-renderer (Tauri) refusal, distinguishable from
"host present but rasterization failed". **App-core owns the one-image full-bleed
push** so revision minting, `validate_message` and every `PushScene` stay on one
runtime path; the server owns bytes only. `push_active_scene` now dispatches to
`execute_native_push` / `execute_raster_render` (controller extraction), whose
transcript is begin -> chunks -> commit -> validated one-image push -> keep-set
release, with the old volatile digest kept until the new push succeeds and no
release ever sent from an incomplete pass. Durable and connection-scoped volatile
confirmation ledgers now feed `DeviceRenderProfile.confirmed_assets`; volatile
confirmations reset on disconnect. `RASTER_MIN_INTERVAL` = 30 s lives in
`scheduler.rs` beside its round-trip rationale; one deadline coalesces
invalidations, newest snapshot wins, a 2-minute provider interval stays 2 minutes,
and native scenes stay event-driven. The operator plugin route injects the
snapshot into the configured runtime card (typed mismatch rejection), lets the
runtime mint the revision — closing the documented revision footgun on this path —
and renders through the same executor; `digital_clock` keeps its direct diagnostic
form. The component test asserts the refusal wording inside the card's editor
(`role="alert"`), never "stale"/"last working settings"/transport. Two stale
frontend fixture pins (schema 5 -> 6) predated this task and were fixed by the
controller in a separate commit. Verification on the combined tree: fmt, clippy
-D warnings, 899 workspace tests, doctests, 100 bun tests, all green; the server
suite's 11 sandbox failures reproduced as loopback-denial only and pass under the
controller (193/0).

---

### Task 6: Close the carried evidence gaps, without borrowing the old gate's confidence

**Files:**
- Modify: `companion/crates/lvgl-sim/src/cases.rs`, focused regression tests;
  `companion/crates/device/examples/framebuffer_diff.rs`;
  `companion/crates/app-core/tests/scene_parity.rs` only for the existing C-oracle rows,
  never to label raster output as parity
- Test: plugin/server integration tests using committed manifests, roots, SVG and assets

**Evidence model:** This task is explicitly an evidence inventory. It distinguishes
producer, formatter, renderer, transport and panel claims. A test that supplies a binding
input proves how that value is drawn and **never what it means**; three device-path defects
survived a whole stage in that hole.

- [x] **Step 1: Record the pre-change evidence row by row.** Name which existing case
      emits `timer.remaining`, which synthetic case draws `timer.pct`, which BigNumber
      cases cross HERO/DISPLAY/BODY, and whether any localized date actually overflows the
      date box. Resolve the disagreement noted under carried debt with file/row names and
      counts, not a blanket “covered/uncovered.”
- [x] **Step 2: Add the missing producer-to-pixel rows first and watch them fail for the
      intended reason.** A real v2 manifest emits `timer.pct` on an arc and
      `timer.remaining:mm:ss` on text; the provider/runtime produces the timer snapshot,
      rather than the render request injecting final percent/text. Configure the hardware
      row with the field registry that accepts timer inputs, not the plugin card's
      DigitalClock registry. Assert the producer's remaining percentage before comparing
      pixels, so a correctly drawn inverted value cannot pass.
- [x] **Step 3: Exercise tier choice on the new raster path.** Use values immediately
      either side of the metrics-derived HERO and DISPLAY widths and a non-numeric value.
      Assert the translated SVG selects the tier the real builder chose, then inspect the
      regression images. If the existing native rows already prove a boundary, retain and
      cite them rather than cloning them under a new name.
- [x] **Step 4: Force real date truncation.** Choose an actual localized date string whose
      measured BODY width exceeds the 176-pixel date content box, derive it through the
      real `date` producer with a non-zero UTC offset, and assert LVGL ellipsizes on the
      native scene. Add a separate raster regression for the same visible constraint;
      agreement between those two renderers is not required or called parity.
- [ ] **Step 5: Add the strongest raster delivery check available.** For a full-bleed
      one-image scene with no scaling/recolour, provision the real volatile payload,
      capture the device's logical framebuffer, and compare it byte-for-byte with the
      RGB565 pixel payload after its 12-byte header. This proves transfer/store/resolver/
      image-node delivery; it does **not** prove that `resvg` drew the right source.
- [x] **Step 6: Run the full host gates.** From `companion/`, separately:
      `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`;
      `cargo test --workspace --all-targets`; `cargo test --workspace --doc`. From repo
      root: `make -C firmware/host_tests clean test` and
      `make -C firmware/host_tests sanitize`.
- [x] **Step 7: Report the evidence honestly.** State exact regression-case and
      framebuffer-diff inventory counts, every exclusion with its reason, which tests pin
      their own inputs, and which claims remain hardware-only. Do not reuse the 132-row
      byte-exact headline for a raster image.
- [x] **Step 8: Commit.** `test: cover negotiated bindings and raster delivery` (Step 5,
      the on-device raster delivery byte-comparison, is the one open item and moves into
      Task 7's session where the captured framebuffer exists.)

**Task 6 execution notes (2026-09-01).** The Step 1 inventory is recorded in
`docs/scene/template-parity-ledger.md` ("Task 6 pre-change evidence inventory") and
resolved the standing disagreement: the older blanket claims ("timer.remaining/timer.pct
have no pixel coverage", "tier step-down never exercised", "measure() checked at exactly
one string width") were stale — `timer.remaining` had 10 producer-shaped lvgl-sim rows
plus 8 parity rows, `timer.pct` had exactly 2 synthetic rows, and BigNumber had 8
metrics-derived boundary rows — while the genuinely missing evidence was a manifest-v2
timer AUTHORING row and a produced date that actually overflows its box (the widest
produced string was 174 px against the 176 px box, so `ellipsize: true` had never been
exercised). Both now exist: `plugin-v2-timer--remaining-357-of-1000` (both orientations;
the semantic assertion pins REMAINING 35% before pixels, so a drawn elapsed-65% inversion
cannot pass; the hardware row uses the ProgressRing registry because plugin cards ride
DigitalClock's four fields on the wire) and `digital-clock--date-overflow` ("Wed, May 13"
at UTC+04:00 measures 178 px BODY, native LVGL renders "Wed, May…", goldens at both
orientations), plus `raster_svg_uses_the_real_builder_tier_across_both_numeric_boundaries`
(HERO 372/434 px, DISPLAY 396/432 px, non-numeric -> BODY, asserted against the real
builder's choice) and a separate raster regression pinning that resvg shows the full
date — an allowed renderer difference, not parity. The hardware framebuffer_diff
inventory moves from 92 total / 8 excluded / 84 included to **96 / 8 / 88** — unobserved
on hardware, a software prediction until Task 7 runs it. Two ledger corrections: the
quantization rationale cited r=195 where the shipped ProgressRing is r=120 (permille
still correct), and the plan's Task 6 text said `timer.pct` where the contract is
`timer.permille`.

---

### Task 7: GATE — one physical session, predecessor first, then stage 4

**Files:** `firmware/version.txt`, `docs/hardware/board-notes.md`

This is the one hardware session. Use `tools/hwcam/` for panel observations and quote
server/device timestamps and status values. Do not start Phase B if Phase A fails.

> **Prep note (2026-09-05, board away):** everything board-free is staged and recorded in
> `docs/hardware/2026-09-05-task7-session-runbook.md` — the post-cleanup server is
> redeployed from `f40958a` (registry 5/0 including the new
> `tools/hw-session-fixtures/svg-live-clock` refuse fixture), `v2.0.0-raster1` is built
> (sha `29f15a6f…`), staged in the catalog dir and verified through the tunnel but **not
> published** (env pin stays `v2.0.0-live2`), the diag variant is built, and
> `firmware/version.txt` moved to `v2.0.0-raster1`. Two prep findings: the server no
> longer builds from a `companion/`-only export (`include_bytes!` reaches
> `tools/fonts/`), and Phase A's BUSY/OTA-owner clause needs an OTA in flight, so the
> runbook defaults to observing it during Step 10 as a recorded deviation. No checkbox
> below is ticked by prep; every observation is still owed.

#### Phase A — pay stage 3b Task 9 on the predecessor image

- [ ] **Step 1: Redeploy the server and load the real curated registry.** Use a
      `git archive HEAD` export, set `DESKMATE_PLUGINS_DIR`, and confirm both v1 plugins
      plus the v2 SVG gate fixtures load with no unexpected `PluginLoadFailure`. A stale
      schema-v5 server rejects schema v6 saves, so deployment is part of the gate.
- [ ] **Step 2: Execute every still-open stage 3b Task 9 observation before flashing.**
      Confirm registry-wide durable digests, both curated v1 plugin cards at 90°/270°, a
      runtime glyph at a non-baked size with first-render timing, the image node through
      `board_lcd_rounder_cb`, and `field.title`. Use the committed inner payload through
      the operator route, as Task 8b specifies.
- [ ] **Step 3: Exercise the existing durable GC teardown.** Observe the standalone-clock
      handoff, scene destruction/font release, compaction with absent digests DEAD and
      retained digests kept, and scene rebuild. Force the documented `BUSY`/OTA-owner
      condition and confirm the scene is restored on the refusal path. Record pass/fail in
      board-notes. If any item fails, stop and amend stage 3b; do not flash stage 4.

#### Phase B — the one stage-4 image

- [ ] **Step 4: Build and record same-tree memory deltas.** Record `.bss`, DIRAM `.text`,
      `.data`, IRAM and DIRAM total before/after. Confirm volatile frame/table bytes are
      PSRAM heap, not statics. Publish one version, move `firmware/version.txt`, verify the
      public artifact's status/hash, and power-cycle once (owner action; USB unplug is not
      power loss on this battery-backed board).
- [ ] **Step 5: Verify the OTA download, not merely the boot.** Read
      `firmware_version`, `ota_state`, `last_ota_error`, link continuity and rollback-window
      survival. One transient retry of the identical image is permitted by the established
      weak-link precedent; a second identical failure is deterministic and requires a
      same-base board bisect. Green tests are not evidence here.
- [ ] **Step 6: Confirm capability truth.** The device and server report bit 9 by name,
      numeric capabilities **1003**, with no unknown bit. Send one volatile begin and
      confirm it is accepted; bit 5 alone in a host test profile still does not authorize
      volatile transfer.
- [ ] **Step 7: Observe native and raster decisions on real cards at both orientations.**
      A supported v1 display-list plugin stays native. The static SVG plugin produces a
      volatile 448x368 image and a one-node scene. Look at 90° and 270°; a host reversal or
      pre-flush capture cannot prove physical panel geometry or the rounder.
- [ ] **Step 8: Observe the refuse rule, typed and visible.** Activate the loaded SVG
      fixture containing `{{ time:HH:mm }}`. Confirm the server sends **no** raster asset
      and no frozen one-node scene, `CardErrorKind::SceneRefused` appears in that card's
      editor with the live-binding reason, and the panel preserves its prior valid content
      or standalone fallback rather than showing a stopped clock. This is a mandatory
      stage-4 rollout observation.
- [ ] **Step 9: Observe the 30-second floor with timestamps.** Through the negotiated
      operator injection path, submit visibly distinct static SVG snapshots at 0, 5 and
      10 seconds. Confirm one immediate volatile frame, no second frame before 30 seconds,
      then the newest snapshot at/after the floor; quote `AssetBegin`/`PushScene` timestamps
      and what appeared on the panel. This is the other mandatory rollout observation.
- [ ] **Step 10: Exercise volatile churn and release.** Run at least 20 raster revisions
      at the 30-second floor. Confirm decoded frames never touch the flash asset-used-byte
      counter, old absent volatile digests are freed, the kept current digest remains
      resolvable, no standalone-clock flash occurs on an ordinary old->new swap, PSRAM does
      not trend downward, and the panel shows no tear/artifact. Repeat one release while
      OTA owns the panel and observe the documented defer/rebuild behaviour.
- [x] **Step 11: Run the exact payload delivery row on hardware.** DONE on the board
      2026-09-06 (recorded in `docs/hardware/board-notes.md` under "B11 / Task 6 Step 5").
      `framebuffer_diff` = **96 total / 10 excluded / 86 identical / 0 differing / 0
      errored**, exit 0. The predicted 96/8/88 was corrected to 96/10/86 by three
      test-harness fidelity fixes the first hardware run surfaced (a wire-invalid
      `now_unix_seconds: 0` on the v2-timer rows; `plugin-aqi--empty`'s `field.title` `--`
      placeholder being unreachable on a device that registers its card's template fields,
      now excluded; and a heavy-image orientation-flip teardown race fixed by a
      `CONFIG_SETTLE`) — none a firmware or renderer defect, all test-only. The staged diag
      image was stale and was rebuilt fresh from HEAD; the release restore artifact rebuilt
      byte-identical to the shipping image. The raster row's honest claim stands: payload
      delivery, not source-render correctness.
- [ ] **Step 12: Record every observation and non-observation in
      `docs/hardware/board-notes.md`.** Keep Phase A and Phase B separate, include exact
      version/memory/timestamp values, and never promote a simulator golden into a panel
      observation.

---

### Task 8: Close stage 4 and hand stage 5 to its own risk review

**Files:** this plan, `CLAUDE.md`, `docs/hardware/board-notes.md`,
`docs/superpowers/plans/<date>-deskmate-plugin-upload-risk-review.md` (create)

- [ ] **Step 1 (software half DONE 2026-09-01; physical half open): Update `CLAUDE.md`.** Record the negotiation table, manifest v2, bit 9 /
      capabilities 1003, volatile/RLE result, exact evidence limits, memory deltas, and
      what the physical session did and did not observe.
- [ ] **Step 2 (software counts recorded in each task's execution notes; hardware numbers open): Mark this plan from actual results.** Fill exact regression/diff counts,
      transfer cadence, GC churn, memory and OTA observations. Do not mark Phase B passed
      if inherited Phase A failed or remained unobserved.
- [x] **Step 3: Write stage 5 as a risk-review plan, not implementation breadth.** Public
      uploads and sandboxing change the threat model and require explicit owner approval.
      Do not add upload endpoints, billing, arbitrary fonts or user content while merely
      planning the review.
- [ ] **Step 4: Commit.** `docs: close rasterization and plan the upload risk review`

---

## Exit criteria

1. The render decision is resolved per `(scene/card, device, revision)` from capability
   bits, node kinds, binding kinds and per-device asset knowledge, with exhaustive tests
   for all three spec §3 rows.
2. A live scene that cannot render natively produces that card's visible typed
   `CardErrorKind::SceneRefused`; no volatile asset or frozen scene is sent. The physical
   editor/panel observation is recorded.
3. A static unsupported scene and a static SVG plugin are rasterized only by `resvg` into
   the exact 12-byte-header + 448x368 RGB565 form, transferred volatile, and referenced by
   one existing image node.
4. Raster invalidations follow the provider schedule with a 30-second minimum, coalesce to
   the newest snapshot, and the physical timestamps demonstrate no second push before the
   floor.
5. Manifest v1 remains frozen and both existing manifests retain their meaning. Manifest
   v2 explicitly owns `[source] root`, `template.kind = "svg"`, and the arc/line binding
   forms; the raw AQI envelope reaches the compiler as its real inner producer value.
6. SVG cannot execute script or reach network/filesystem/system fonts; every named hostile
   input fails within the source/expansion/render bounds. No browser dependency or process
   exists.
7. Protocol remains v1 additive. Bit 9 is `512`, current capabilities are **1003**, and
   both numbers are pinned in Rust and firmware tests. A bit-5-only device is never assumed
   to accept volatile assets.
8. Volatile frames live only in PSRAM, at the two-slot active+incoming bound, survive an
   atomic swap, disappear on release/reboot, and do not increase flash asset usage during
   the 20-frame hardware run.
9. `AssetRelease` always carries the durable + active-volatile keep-set; the empty-registry
   wipe guard remains tested, the durable ceiling reserves one volatile digest, and the
   LVGL/font/compaction/rebuild sequence is observed in the single hardware session.
10. The carried evidence debt is stated accurately: producer vs injected-input coverage,
    actual numeric-tier boundary rows, a real overflowing date, and the manifest emitter
    for `timer.pct`/`timer.remaining`. Raster regression images are not called parity.
11. `cargo fmt --all --check`, clippy with `-D warnings`, **both**
    `cargo test --workspace --all-targets` and `cargo test --workspace --doc`, firmware
    host tests, and ASan pass.
12. Same-tree firmware memory deltas are recorded and the one stage-4 OTA download is
    re-verified on the physical board. Stage 3b Task 9 is observed first in the same
    session; neither gate is inferred from the other.

Stage 5 — public uploads and sandboxing — does not begin from this exit criterion. It
begins only after its separate risk review and explicit owner approval.

---

## Risks

**The refusal rule is easy to weaken into a frozen clock.** Any fallback helper that sees
“rasterizable” before it sees “live” can produce a face that looks correct at capture time
and lies seconds later. Keep the decision pure, table-driven and ahead of rendering; keep
the rasterizer's own live-binding rejection as defence in depth.

**Capability bit 5 already overclaims the volatile flag.** This is not hypothetical
compatibility reasoning: current firmware advertises 491 and rejects `volatile: true` in
`protocol_task.c`. Without bit 9, the first real fallback against an older scene build
would discover support by failing after policy had already chosen it.

**Memory layout remains the dominant risk.** The decoded frame is about 322 KiB and belongs
in PSRAM; that says nothing about the few bytes of metadata, function reachability or
linker movement the feature may introduce. About 105 bytes once killed OTA with every test
green. Measure same-tree and verify the download.

**Volatile lifetime and durable compaction now share one digest namespace.** Freeing the
old frame too early is a live LVGL pointer use-after-free; keeping every frame leaks PSRAM;
omitting a durable digest from `AssetRelease` deletes it; rebuilding after compaction from
a scene that names a released asset fails. Exact sequencing and the two-slot bound are the
only manageable answer.

**`resvg` is safer than a browser, not magically safe.** SVG is still recursive,
allocation-heavy input with resource-reference syntax. Source/expanded sizes, XML shape,
font set, dimensions and wall-clock all need bounds, and ambient resource resolution must
be absent rather than merely unused by the curated example.

**The raster evidence is necessarily weaker than the template oracle.** A golden can
freeze a bug. A simple pixel probe can miss complex typography. Device capture can prove
the exact bytes arrived and still say nothing about whether `resvg` interpreted the source
as intended. Keep the claims split and require panel review.

**The 30-second floor is stateful.** Reconnects, playlist changes, manual Refresh, stale
transitions and operator injection can each accidentally reset a naive timer and bypass or
starve the floor. The fake-clock matrix must cover each reset edge, and the newest pending
snapshot must win.

**Stage 3b's baseline is still unobserved.** If its durable font/image/GC path fails in
Task 7 Phase A, stage 4 cannot use that failure as evidence about volatile assets. Stop,
record it, and repair/amend the predecessor before flashing the new image.
