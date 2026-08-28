# Stage 3b — Plugin manifest and curated plugins

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps
> use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A declarative plugin manifest, evaluated entirely on the server, that turns
fetched data into a scene — proved end to end by two curated first-party plugins that ship
their own font and icon glyphs.

**Architecture:** A new `plugin` crate parses a TOML manifest into a validated, bounded
`PluginManifest`. A restricted expression language evaluates the manifest's node list
against a `ProviderSnapshot`, producing a `Scene` through the same `scene_build` primitives
the six retired templates use. Assets named by the manifest are content-addressed, pushed
through the existing `asset_transfer` path, and referenced by digest. The device is
unchanged: it evaluates the same closed binding set stage 3a froze.

**Tech Stack:** Rust (`companion/`), TOML manifests, `serde_json` for provider payloads,
the existing `providers::Provider` trait, `lvgl-sim` for goldens, `stb_truetype` via LVGL's
`tiny_ttf` on the device.

**Spec:** `docs/superpowers/specs/2026-08-22-deskmate-plugin-scene-rendering-design.md`
(§1 asset store, §2 scene format, §5 server pipeline and security, §6 parity, §7 rollout
stage 3).

**Ledger:** `docs/scene/template-parity-ledger.md` — read its "Stage 3b entry decision"
section first. It names the two things this stage inherits and must not lose.

---

## Scope: what stage 3a left, and what this stage is

Spec §7 names stage 3 as "Plugin manifest, curated plugins, clock and pomodoro as scenes".
The last of those was **stage 3a**, which retired all six hand-written C templates. What
remains is the authoring model: a plugin that is not one of the six faces.

This is a **server-and-authoring** stage. The device already has everything it needs:
`SceneFont::Asset` is on the wire, `firmware/main/core/asset_store.c` and
`asset_transfer.c` exist, and `ui/scene_view.c` resolves an asset font through a refcounted
`font_registry` with a release callback. **The expected firmware delta is zero.** Verify
that claim with `idf.py size` rather than assuming it; if it turns out non-zero, Task 9's
gate becomes a layout gate as well as an asset gate, and the cost of this stage goes up by
one OTA cycle.

## What this stage does not do

- **No scripting runtime.** §5 is explicit: the manifest contains no code. The expression
  language has no loops except one bounded repeat, no recursion, no user-defined
  functions, and no way to reach the filesystem, the network, or the clock.
- **No public uploads and no sandbox.** Stage 5, with its own risk review. This stage
  ships the format and a curated set, which is what §5 says removes the largest risk.
- **No rasterization and no SVG.** Stage 4. `resvg` is not added here.
- **No new device binding.** The vocabulary froze at stage 3a: nine tokens plus
  `running_color`. See Global Constraints.
- **No §8 draw-buffer spike.** It is a memory-layout change; §9 says why folding one into
  a feature change is how this repository lost days twice.

---

## Global Constraints

- Protocol stays **v1, additive only**. `PROTOCOL_CURRENT_CAPABILITIES` stays **491**.
- Config schema moves **v5 -> v6** for the plugin card kind, with **lossless v0..v5
  migration** and compatibility fixtures, exactly as every prior schema move.
  **Redeploy the server whenever the schema moves** — the app and the server carry
  independent copies of `CURRENT_SCHEMA_VERSION`, and a stale server rejects every save
  with `schema version 6 is not supported; expected 5`.
- **The binding vocabulary is closed and this stage adds nothing to it.** If a plugin
  wants a tenth token, the answer is a host-side rebuild-and-push. §2's rule is "anything
  computed happens on the server", and the pressure to add just one more is precisely how
  a closed set becomes an expression language. A task that finds itself wanting one must
  stop and re-read the ledger's entry decision.
- **Every expression evaluation is bounded**: a fuel counter, a depth cap, an output-size
  cap, and a repeat cap. A manifest is untrusted input the moment stage 5 exists, and
  writing the caps later means writing them under pressure.
- **Provider payloads are untrusted bytes**, like everything else this protocol touches.
  Bound lengths and counts, reject malformed input, and never let a fetch failure become a
  panic or an unbounded allocation.
- **A transient provider failure is not a permanent card fault.** Stage 3a's Task 7 shipped
  that defect and it was invisible to every test in the sandbox: a `Busy` was recorded as
  a permanent fault on the card. `providers::LastGood` and the companion's per-card `stale`
  flag already express the correct behaviour; use them.
- Run both test invocations from `companion/`; neither alone covers the workspace:
  `cargo test --workspace --all-targets` and `cargo test --workspace --doc`.
- If any firmware source changes, `make -C firmware/host_tests sanitize` is not optional,
  same-tree `.bss`/DIRAM/`.data`/IRAM deltas are recorded, and the OTA download is
  re-verified on the board.

---

## Three findings from stage 3a that shape these tasks

**1. The byte-exact gate ends here, and nothing inherits its name.** Stages 2a through 3a
were carried by one exceptionally strong claim: a host-built scene reproduces the shipped C
template byte-identically, because `lvgl-sim` compiles the same C. **A plugin scene has no C
counterpart.** There is nothing to be byte-identical to. Any test that calls itself parity
in this stage is comparing the simulator against itself, which proves determinism and
nothing else. The evidence model here is different by necessity and the plan says so
explicitly in each task: bounded-input property tests for the evaluator, simulator goldens
for pixels, and `framebuffer_diff` for device-versus-simulator agreement. Do not let a
green golden be read as the gate stage 2b had.

**2. `field.*` has never drawn a pixel.** It is the one binding with no builder, therefore
no coverage, and it is the plugin data path. A plugin that binds it is exercising an
untested arm of `scene_binding.c` on the device. Task 8's `framebuffer_diff` rows are the
first thing that closes this, and that is why they are not optional garnish.

**3. A gate that supplies a binding's input proves how a value is drawn, never what it
means.** Three device-path defects lived in that hole for a whole stage. Every task here
that evaluates something must test the **producer**, not only the formatter: the
expression evaluator against real provider payloads, not against a hand-built `Value` that
already has the shape the code expects.

---

## The manifest, concretely

One example, because every task below refers to it.

```toml
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
```

Two properties of that shape are load-bearing:

- **Node fields are typed and absolute.** The manifest is a display list, not a layout
  language. `x`, `baseline_y`, `w` are integers on the 448x368 canvas; the server does all
  layout, per §2. A field that accepts an expression accepts it only where a *value* goes.
- **`value` may evaluate to a device binding.** `value = "{{ time:HH:mm }}"` compiles to
  `SceneValue::Binding`, not to a literal. That is how a plugin card keeps time with the
  link down. Which forms are bindings is fixed by the closed set, and the compiler rejects
  anything else in that position rather than falling back to a literal.

---

## File Structure

**Create:**

- `companion/crates/plugin/Cargo.toml`, `src/lib.rs` — the crate root.
- `companion/crates/plugin/src/manifest.rs` — TOML -> `PluginManifest`, bounded and typed.
- `companion/crates/plugin/src/expr.rs` — the restricted expression language: parse and
  evaluate, with fuel.
- `companion/crates/plugin/src/compile.rs` — `PluginManifest` + `ProviderSnapshot` ->
  `Scene`.
- `companion/crates/plugin/src/assets.rs` — manifest assets -> content-addressed digests
  and the icon-font name map.
- `companion/crates/plugin/tests/hostile_manifest.rs` — untrusted-input corpus.
- `companion/crates/lvgl-sim/src/asset_shim.rs` — §6's parity obligation: the simulator
  resolves a digest to the same bytes the server pushes.
- `companion/plugins/aqi/` and `companion/plugins/agenda/` — the two curated plugins.
- `docs/plugins/manifest-v1.md` — the manifest contract, frozen like `docs/config/v5.md`.

**Modify:**

- `companion/crates/app-core/src/config.rs` — schema v6 plugin card kind, v5 migration.
- `companion/crates/server/src/` — plugin registry, provider wiring, egress guard.
- `companion/crates/lvgl-sim/src/cases.rs` — plugin golden cases.
- `companion/crates/device/examples/framebuffer_diff.rs` — plugin rows that provision
  real assets; the inventory assertion moves off 76/64/12.
- `docs/config/v6.md` (create, from v5), `CLAUDE.md`, `docs/hardware/board-notes.md`.

---

## Task ordering, and where the hardware cost sits

Tasks 1-8 are host-only and carry no hardware cost. Task 9 is the single hardware gate, and
it is single **because** the firmware is expected to be unchanged: there is no image to
build, so the asset provisioning, the panel observation and the GC teardown all ride one
session. If Task 1 discovers a firmware need, defer it — collect every firmware need until
Task 8 and land them in one image, exactly as stage 2b did to turn four needs into one OTA.

---

### Task 1: The manifest, parsed and bounded

**Files:**
- Create: `companion/crates/plugin/Cargo.toml`, `companion/crates/plugin/src/lib.rs`,
  `companion/crates/plugin/src/manifest.rs`
- Test: `companion/crates/plugin/src/manifest.rs` (unit), and
  `companion/crates/plugin/tests/hostile_manifest.rs`

**Interfaces:**
- Produces: `PluginManifest { name, version, source, assets, nodes }`,
  `parse_manifest(&str) -> Result<PluginManifest, ManifestError>`, and a `ManifestError`
  enum with one variant per rejection reason. Tasks 2, 3, 4 and 7 all consume these.

- [ ] **Step 1: Write the failing bounds tests.** Every limit gets a test that a value one
      past it is rejected with the *named* error, not a generic one.

```rust
#[test]
fn a_manifest_over_the_node_ceiling_is_rejected_by_name() {
    let nodes = (0..MAX_NODES + 1).map(|_| MINIMAL_TEXT_NODE).collect::<String>();
    let err = parse_manifest(&format!("{MINIMAL_HEADER}{nodes}")).unwrap_err();
    assert!(matches!(err, ManifestError::TooManyNodes { limit: MAX_NODES, .. }));
}
```

- [ ] **Step 2: Run them and watch them fail** — `cargo test -p plugin`.
- [ ] **Step 3: Define the bounds as named constants**, each with a one-line comment
      saying what it protects. `MAX_NODES` must not exceed the wire's own node ceiling;
      **call `protocol::validate_message`'s bound rather than restating it** — the
      protocol is where the wire's limits live, and a second copy drifts.
- [ ] **Step 4: Implement the parser** with `serde` + `toml`, rejecting unknown fields
      (`#[serde(deny_unknown_fields)]`) so a typo is an error rather than a silent default.
- [ ] **Step 5: Run the tests and the hostile corpus.** The corpus includes: a 10 MB
      manifest, deeply nested tables, duplicate asset names, a glyph codepoint in the
      surrogate range, a codepoint above `0x10FFFF`, `refresh_minutes = 0`, a `url` with a
      `file://` scheme, and non-UTF-8 bytes.
- [ ] **Step 6: Commit.** `feat: parse and bound the plugin manifest`

---

### Task 2: The restricted expression language

**Files:**
- Create: `companion/crates/plugin/src/expr.rs`
- Test: same file, plus property tests

**Interfaces:**
- Produces: `Expr::parse(&str) -> Result<Expr, ExprError>` and
  `Expr::eval(&self, ctx: &EvalContext, fuel: &mut Fuel) -> Result<EvalValue, ExprError>`,
  where `EvalValue` is `Text(String)`, `Number(f64)`, `Bool(bool)` or `Missing`.

This is the task where a restricted language becomes a scripting runtime if nobody is
watching. The grammar is deliberately small: field access (`data.a.b`), array index
(`data.rows[0]`), a fixed function set, and `?:` for presence. **No user-defined names, no
assignment, no loops** — the one repeat form lives in the manifest, not the expression.

- [ ] **Step 1: Write the failing termination test first.** This is the property that
      matters most and it is cheapest to pin before any evaluation exists.

```rust
#[test]
fn evaluation_always_terminates_within_its_fuel() {
    let nested = "upper(".repeat(MAX_DEPTH + 1)
        + "data.a"
        + &")".repeat(MAX_DEPTH + 1);
    assert!(matches!(Expr::parse(&nested), Err(ExprError::TooDeep { .. })));

    // Within the depth cap, fuel is the second bound: a legal expression
    // repeated across many nodes must still terminate against one budget.
    let mut fuel = Fuel::new(FUEL_BUDGET);
    let expr = Expr::parse("truncate(upper(data.a), 64)").unwrap();
    let err = loop {
        if let Err(err) = expr.eval(&EvalContext::empty(), &mut fuel) {
            break err;
        }
    };
    assert!(matches!(err, ExprError::OutOfFuel));
}
```

- [ ] **Step 2: Run it and watch it fail.**
- [ ] **Step 3: Implement the parser** as a small recursive-descent parser with an explicit
      depth counter, so depth is refused at parse time and fuel bounds evaluation.
- [ ] **Step 4: Implement the function set**, and keep it to what the two curated plugins
      genuinely need: `upper`, `lower`, `round(x, places)`, `truncate(s, n)`,
      `default(x, fallback)`, and `icon(name)` which resolves through the manifest's
      icon-font map. Each function is total: it returns `Missing` rather than panicking, and
      it never allocates beyond the output cap.
- [ ] **Step 5: Test the producer, not the formatter.** Evaluate against the *real* JSON a
      curated plugin's endpoint returns — a fixture file captured once and committed — not
      against a hand-built `Value` that already has the shape the code expects. This is
      finding 3 above; three device defects lived in exactly that gap.
- [ ] **Step 6: Hostile inputs.** A missing key, a null, an array where an object is
      expected, a number where text is expected, a 10 MB string, and a NaN. None may panic;
      each has a named error or evaluates to `Missing`.
- [ ] **Step 7: Commit.** `feat: add the bounded plugin expression language`

---

### Task 3: Compile a manifest and a snapshot into a Scene

**Files:**
- Create: `companion/crates/plugin/src/compile.rs`
- Modify: nothing in `app-core` yet
- Test: same file

**Interfaces:**
- Consumes: `PluginManifest` (Task 1), `Expr` (Task 2), `BakedFontMetrics` and the
  `scene_build` node constructors from `app-core`.
- Produces:
  `compile_scene(&PluginManifest, &ProviderSnapshot, &BakedFontMetrics, revision: u32) -> Result<Scene, CompileError>`.

- [ ] **Step 1: Write the failing binding test.** The load-bearing case is that a value
      expression in the closed set compiles to a *binding*, not a literal — that is what
      keeps a plugin card alive with the link down.

```rust
#[test]
fn a_closed_set_expression_compiles_to_a_binding_not_a_literal() {
    let scene = compile_scene(&manifest_with_value("{{ time:HH:mm }}"), &snapshot(),
                              &metrics(), 1).unwrap();
    assert_eq!(SceneValue::Binding("time:HH:mm".into()), text_node(&scene, 0).value);
}

#[test]
fn an_expression_outside_the_closed_set_is_refused_in_binding_position() {
    let err = compile_scene(&manifest_with_value("{{ timer.velocity }}"), &snapshot(),
                            &metrics(), 1).unwrap_err();
    assert!(matches!(err, CompileError::UnknownBinding { .. }));
}
```

      The second test is the ratchet's enforcement. Without it the compiler silently
      degrades an unknown binding to a literal and the card looks right until it stops
      ticking.

- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement the compiler.** Reuse `app-core`'s existing node constructors so
      there is one representation of a scene node, not two. Text measurement goes through
      `BakedFontMetrics::measure()` for baked tiers; an asset font's measurement is Task 5's
      problem and this task must not guess at it.
- [ ] **Step 4: Validate before returning.** Call `protocol::validate_scene` on the
      compiled scene under `#[cfg(debug_assertions)]`, the same way `with_scene_data_state`
      does. The wire's bounds live in the protocol; do not restate them here.
- [ ] **Step 5: Implement the one repeat form** — a node group repeated over a bounded
      array, capped, with the index available as `item`. `RowList`'s shape is what this
      exists for.
- [ ] **Step 6: Run the tests.** Include a snapshot in every `ProviderSnapshot` state:
      fresh, last-good/stale, and error. The stale and error states must reach
      `push_state_footer`'s equivalent — a plugin card that goes stale must say so, the
      same way the six faces do.
- [ ] **Step 7: Commit.** `feat: compile a plugin manifest into a scene`

---

### Task 4: Assets — digests, and the icon-font name map

**Files:**
- Create: `companion/crates/plugin/src/assets.rs`
- Test: same file

**Interfaces:**
- Produces: `resolve_assets(&PluginManifest, &Path) -> Result<AssetSet, AssetError>`,
  where `AssetSet` maps a manifest asset name to its SHA-256 digest, byte length, and kind,
  and resolves an icon name to a codepoint.

- [ ] **Step 1: Write the failing determinism test.** Content-addressing is only worth
      anything if the same bytes produce the same digest across runs and hosts.
- [ ] **Step 2: Run it and watch it fail.**
- [ ] **Step 3: Implement digesting and the name map**, rejecting a name that no glyph
      defines — §1 says "the server validates the name resolves", and a plugin that ships a
      broken icon name should fail at load, not draw a tofu box on the panel.
- [ ] **Step 4: Test the boundaries** — a zero-byte file, a file larger than the asset
      partition's blob region, a duplicate digest across two plugins (which must ship once,
      per §1), and a name colliding across two icon fonts.
- [ ] **Step 5: Commit.** `feat: content-address plugin assets and resolve icon names`

---

### Task 5: The simulator asset shim — §6's parity obligation

**Files:**
- Create: `companion/crates/lvgl-sim/src/asset_shim.rs`
- Modify: `companion/crates/lvgl-sim/src/lib.rs`, `build.rs` if a source list changes
- Test: same file

This task exists because §6 says this design **adds** a parity obligation rather than
removing one. Today both hosts rasterize identically by construction, because the simulator
compiles the firmware's own font C files. A runtime font ends that: the simulator must
resolve the **same digest to the same bytes** as the device, or every golden in Task 8 is
measuring two different fonts and reporting agreement.

**Do this before any font golden exists.** A golden written against an unshimmed simulator
will pass, and it will mean nothing.

- [ ] **Step 1: Write the failing test** that the shim resolves a digest to bytes and that
      an unknown digest is an error rather than a silent fallback to a baked face. A silent
      fallback is the failure mode that makes this whole task pointless.
- [ ] **Step 2: Run it and watch it fail.**
- [ ] **Step 3: Implement the shim** reading from the same content-addressed store the
      server pushes from, so there is one set of bytes and no copy to drift.
- [ ] **Step 4: Prove the obligation is discharged** — render one string at one size
      through a baked tier and through the same face supplied as an asset, and assert the
      glyph bitmaps agree. `stb_truetype` is deterministic; the obligation is about byte
      provenance, not the rasterizer, so this test is about *which bytes arrived*.
- [ ] **Step 5: Commit.** `test: resolve asset fonts identically on both hosts`

---

### Task 6: Schema v6 — the plugin card kind

**Files:**
- Modify: `companion/crates/app-core/src/config.rs`
- Create: `docs/config/v6.md`, compatibility fixtures
- Test: migration tests alongside the existing v0..v5 corpus

**Interfaces:**
- Produces: a `plugin` card kind carrying a plugin id and its per-card settings.

- [ ] **Step 1: Write the failing lossless-migration test** for v5 -> v6 across the full
      existing fixture corpus, asserting round-trip equality of everything a v5 config could
      express.
- [ ] **Step 2: Run it and watch it fail.**
- [ ] **Step 3: Implement the kind and the migration.**
- [ ] **Step 4: Bump `CURRENT_SCHEMA_VERSION` in both places** — the app and the server
      carry independent copies, and a stale server rejects every save. Add the redeploy to
      Task 9's checklist rather than discovering it on the board.
- [ ] **Step 5: Freeze the contract** in `docs/config/v6.md`, written from v5.
- [ ] **Step 6: Commit.** `feat: add the plugin card kind as schema v6`

---

### Task 7: The server — registry, providers, and the egress guard

**Files:**
- Modify: `companion/crates/server/src/`
- Test: server integration tests

**A note the implementer needs:** integration tests here bind loopback listeners. If you
are running in a sandbox that denies that, **say so in your report rather than reporting
green** — this has produced three rounds of invisible defects in this project. Name the
tests you could not run.

- [ ] **Step 1: Write the failing SSRF tests first.** §5's allowlist is the security
      boundary of this entire stage, and the render host is the owner's homelab beside
      unrelated services.

```rust
#[test]
fn the_egress_guard_denies_private_and_metadata_destinations() {
    for url in ["http://127.0.0.1/", "http://10.0.0.1/", "http://169.254.169.254/",
                "http://[::1]/", "http://192.168.8.20/"] {
        assert!(matches!(egress_guard(url), Err(EgressError::Denied { .. })), "{url}");
    }
}
```

- [ ] **Step 2: Run them and watch them fail.**
- [ ] **Step 3: Implement the guard**: deny RFC1918, loopback, link-local and
      `169.254.169.254`; **resolve then pin** the address actually connected to, so a DNS
      rebind between check and connect cannot slip past; cap body size, wall-clock, and
      redirect count.
- [ ] **Step 4: Wire plugins to `providers::Provider`.** §5 is explicit that plugin data
      sources implement the existing trait, so `LastGood`, `RefreshPolicy` and categorised
      `ProviderError` apply with no new machinery.
- [ ] **Step 5: Classify errors correctly, and test the classification.** A transient
      failure surfaces as `stale` on the card; only a permanent one is a card fault. Stage
      3a shipped the opposite and no test caught it.
- [ ] **Step 6: Per-plugin caps** on refresh rate, CPU and render wall-clock, per §5.
- [ ] **Step 7: Commit.** `feat: run curated plugins server-side behind an egress guard`

---

### Task 8: Two curated plugins, end to end

**Files:**
- Create: `companion/plugins/aqi/`, `companion/plugins/agenda/`
- Modify: `companion/crates/lvgl-sim/src/cases.rs`,
  `companion/crates/device/examples/framebuffer_diff.rs`
- Create: `docs/plugins/manifest-v1.md`

Two plugins, chosen to cover different surface: **aqi** is a JSON source with a numeric
hero and an icon glyph, so it exercises the icon-font path and `field.*`. **agenda** is a
list with the bounded repeat and a real truncation case, so it exercises the repeat form
and text measurement at a boundary.

- [ ] **Step 1: Write the two manifests and commit a captured payload fixture for each.**
- [ ] **Step 2: Add golden cases** to `cases.rs` covering, for each plugin: fresh, stale,
      error, an empty/missing-data state, and both orientations.
- [ ] **Step 3: Add `framebuffer_diff` rows that provision real assets.** This is the
      first time the device's asset path is exercised by a test. It closes the eight
      synthetic rows currently excluded as unrepresentable — `exclusion_reason()` in
      `framebuffer_diff.rs` gives three reasons for them: an RGB565 image asset, a runtime
      font asset, and a `field.status` binding no built-in template field registry accepts.
      A plugin provisions the first two and brings its own field registry for the third.
      Report the actual per-reason counts rather than assuming the split. Update the
      inventory assertion, which currently pins `total=76 identical=64 excluded=12`, and
      **state the new numbers in the task report** so a future reader diffing against
      `CLAUDE.md` is not misled the way the 58/54/4 invariant misled this stage.
- [ ] **Step 4: Bind `field.*` in at least one plugin, deliberately.** It is the one
      binding with no builder and therefore no pixel coverage; a plugin is the first thing
      that can give it any.
- [ ] **Step 5: Freeze the manifest contract** in `docs/plugins/manifest-v1.md`.
- [ ] **Step 6: Run the full gate set** — fmt, clippy, both test invocations, and the
      firmware host tests if any firmware source changed.
- [ ] **Step 7: Commit.** `feat: ship the aqi and agenda curated plugins`

---

### Task 9: GATE — the asset path on the physical board

**Files:** `docs/hardware/board-notes.md`

The single hardware session. Its shape depends on Task 1's finding: if the firmware is
genuinely unchanged, this is an **asset gate**, not a layout gate, and there is no image to
build. If any firmware source changed, add the OTA download check and the same-tree memory
deltas, and expect to have bundled every firmware need into one image.

- [ ] **Step 1: Redeploy the server** from a `git archive HEAD` export — the schema moved
      to v6, and a server built before it rejects every save.
- [ ] **Step 2: Provision the two plugins' assets** and confirm the digests the device
      reports match the ones the server computed.
- [ ] **Step 3: Observe both plugin cards on the panel at both orientations.** Real 270°
      geometry is provable only by looking at the panel — no host gate and no `0x7E`
      capture can substitute, because both carry the same `flipped(A) == flipped(B)`
      blindness.
- [ ] **Step 4: Observe a glyph at a size no baked tier provides**, which is the whole
      point of a runtime font. Watch for the first-render hitch §9 predicts; §6's
      glyph-cache-miss timing gate at 96 px **becomes owed here**, because this is the
      first stage where a font is a runtime asset.
- [ ] **Step 5: Observe `board_lcd_rounder_cb` at both orientations with an image node
      present.** This is the second of §6's two gates that stage 3a explicitly deferred,
      and this stage is where it comes due.
- [ ] **Step 6: Exercise the asset-GC teardown** — release, compact, rebuild the retained
      scene — and observe the `BUSY`/OTA-owner interaction. It spans four LVGL/ESP-IDF-bound
      files with no simulator seam and has **no automated test**; this is its first real
      exercise, and the four on-device observations specified at the end of the scene
      renderer plan's Task 10 apply here.
- [ ] **Step 7: Record every observation in `docs/hardware/board-notes.md`**, including
      anything that did **not** pass. Never record an observation that was not made.

---

### Task 10: Close the stage and hand off to stage 4

**Files:** this plan, `CLAUDE.md`, `docs/scene/template-parity-ledger.md`,
`docs/superpowers/plans/<date>-deskmate-rasterization.md` (create)

- [ ] **Step 1: Update `CLAUDE.md`** — the manifest contract, the schema at v6, what the
      hardware session did and did not observe.
- [ ] **Step 2: Update the ledger's "Stage 3b entry decision"** to say whether `field.*`
      and the asset path held up, since that section named both as inherited risks.
- [ ] **Step 3: Write the stage 4 plan** (rasterization fallback and SVG plugins) from
      what this stage taught, per the working agreement.
- [ ] **Step 4: Commit.** `docs: close stage 3b and plan rasterization`

---

## Exit criteria

1. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and
   **both** `cargo test --workspace --all-targets` and `cargo test --workspace --doc` pass.
2. `make -C firmware/host_tests clean test` and `make -C firmware/host_tests sanitize`
   pass. If any firmware source changed, same-tree memory deltas are recorded and the OTA
   download is verified on the board.
3. A manifest cannot execute code, cannot reach a private address, and cannot evaluate
   without terminating — each proved by a test that fails if the guard is removed.
4. `protocol::validate_scene` accepts every scene the compiler emits, and the compiler
   **refuses** an unknown binding in binding position rather than degrading it to a literal.
5. §6's parity obligation is discharged: the simulator resolves a digest to the same bytes
   as the device, proved before any font golden was written.
6. Schema v6 migrates v0..v5 losslessly, with fixtures, and both copies of
   `CURRENT_SCHEMA_VERSION` moved.
7. Both curated plugins render at both orientations on the physical panel, with a runtime
   glyph at a non-baked size.
8. The asset-GC teardown has been exercised on hardware and the result recorded — pass or
   fail.
9. `framebuffer_diff`'s inventory has moved off 76/64/12, the eight asset exclusions are
   closed, and the new numbers are stated in both the task report and `CLAUDE.md`.

Stage 4 — rasterization fallback and SVG plugins — is planned at this plan's exit.

---

## Risks

**A restricted language becomes a scripting runtime one convenience at a time.** Every
addition is individually reasonable and the sum is an evaluator on the owner's homelab.
The function set in Task 2 is the ceiling; a plugin that needs more is a plugin that needs
a server-side provider, which is the seam that already exists for exactly this.

**The strongest gate this project has had does not apply here.** Stages 2a through 3a were
carried by byte-exact reproduction of hand-written C. Plugin scenes have no oracle, so the
evidence is weaker by necessity. The danger is not the weaker evidence — it is calling it
by the old name and inheriting the old confidence. Each task states its own evidence model
for that reason.

**The asset path has never run.** `asset_store.c`, `asset_transfer.c`, the font registry,
and the GC teardown all exist, are host-tested where they can be, and have drawn nothing on
a panel. Task 9 is the first time any of it is real, and the GC teardown in particular
spans four LVGL/ESP-IDF-bound files with no simulator seam.

**Memory layout, if the firmware turns out not to be unchanged.** The expectation is a zero
delta, and an expectation is not a measurement. `idf.py size` decides it. If it is
non-zero, this stage inherits the full layout hazard: ~105 bytes once broke OTA downloads
outright with every test green, and green tests say nothing about whether OTA still works.

**IRAM is reported 100% full.** Nothing in this stage should reach it — the expression
language and the compiler are host-side — but `tiny_ttf` running for real on the device is
new execution on a board with no IRAM headroom.
