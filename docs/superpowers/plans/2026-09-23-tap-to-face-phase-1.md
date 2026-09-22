# Tap to face, phase 1 — implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A tap on a picture card reaches the face that draws it, and the face answers with a new frame — proved on `dev-0005` with Hacker News paging and weather flipping.

**Architecture:** The runtime lowers every picture card's wire tap action to `StartPause`, so the flashed firmware reports taps it currently swallows. A new `CardTapSink` port carries the tap out of `app-core` (which never learns what a face is) into the server, which routes it to that card's existing refresher task. The refresher calls the faces package with `{state, event}` and gets back `{png, state}`; the frame takes the picture card's existing asset path. No schema bump, no wire change, no firmware image.

**Tech Stack:** Rust (tokio, serde) for `crates/app-core` and `crates/server`; TypeScript on Bun for `companion/faces`; React for `companion/apps/deskmate`.

**Spec:** `docs/superpowers/specs/2026-09-23-deskmate-tap-to-face-design.md` — read it first; it holds the reasoning this plan implements and the rejected alternatives.

## Global Constraints

- **Worktree** `/Users/rodion/dev/deskmate-c1`, branch `track-c1-tap-to-face`. All paths below are relative to it.
- **Cross no expensive boundary.** `CURRENT_SCHEMA_VERSION` stays 10, no protocol message/key/error code/capability bit changes, and **nothing under `firmware/` is edited by this plan**. If a task seems to need one, stop and raise it.
- **`cargo` is not on the Bash tool's PATH**: `export PATH="$HOME/.cargo/bin:$PATH"`. Never pipe a cargo invocation into `tail` — in zsh the exit status becomes `tail`'s and a failure reads as a pass. Redirect to a file and check `$?`.
- **`data_cards.rs` names no face kind**, and `app-core` knows nothing about faces. Adding a face must stay "a file in `faces/src/faces/` plus `deploy.sh --faces-only`".
- **Golden SVGs are byte-exact.** Every coordinate reaching a document goes through `kit/svg.ts`'s `fixed()`, never `toFixed`. A deliberate design change is `bun run dump --update`, reviewed as a diff.
- **`textWidth` does not account for `letter-spacing`.** Any tracked run (every eyebrow) uses `trackedWidth`/`fitTracked`.
- **Face state cap:** 16 KB (`16 * 1024` bytes) of encoded JSON per source. **Tap coalescing cap:** 32. **Temporary-view window:** 10 minutes.
- **Commit style:** conventional prefixes (`feat:`, `fix:`, `test:`, `docs:`, `chore:`). Commit messages end with:
  ```
  Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_018JRf5xcZToBYGR9o66hKwN
  ```
- **Full gate suite before handoff** (from `companion/`): `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace --all-targets`; `cargo test --workspace --doc`; in `apps/deskmate`: `bun test && bun run check && bun run format:check && bun run build`; in `faces`: `bun test && bun run check && bun run lint && bun run format:check`. Warm run ≈ 30 s; a cold one ≈ 1 minute. Subagents cannot run the workspace gates (they time out) — the controller runs them.

## File structure

| File | Responsibility | Task |
|---|---|---|
| `companion/faces/src/face.ts` | `RenderContext` type: `state`, `event`, and what a face returns | 1 |
| `companion/faces/src/main.ts` | The seam: parse `{kind, settings, state, event}`, emit `{png, state}` | 1 |
| `companion/faces/src/registry.ts` | unchanged shape; faces now may declare `tap` | 1 |
| `companion/crates/server/src/data_cards/faces_package.rs` | `render` sends state/event, parses the envelope, sniffs a legacy PNG; `describe` reads `tap` | 2, 4 |
| `companion/crates/server/src/data_cards/face_state.rs` (new) | The per-source state store: load, get, put, remove, atomic write, 16 KB cap | 3 |
| `companion/crates/server/src/data_cards/worker.rs` | Refresher loop selects over interval and taps; passes state in, stores state out | 3, 6 |
| `companion/crates/server/src/data_cards.rs` | `tap` on the descriptor; tap routing; per-source `TapSignal` beside each task | 4, 6 |
| `companion/crates/app-core/src/runtime/mod.rs` | `CardTapSink`, the new `Tap` arm, the diagnostic counter | 5 |
| `companion/crates/app-core/src/config.rs` | `wire_config` lowers every picture to `StartPause` | 7 |
| `docs/config/v10.md` | One sentence: what a picture's `tap_action` means now | 7 |
| `companion/faces/src/faces/hackernews.ts` | Paging from state, page indicator | 8 |
| `companion/faces/src/faces/weather.ts` | The coming-days view | 9 |
| `companion/apps/deskmate/src/lib/types.ts`, `components/CardEditor.tsx` | The one hint line | 10 |
| `docs/hardware/board-notes.md` | What was observed on `dev-0005` | 11 |

---

### Task 1: The faces seam takes state and an event

**Files:**
- Modify: `companion/faces/src/face.ts`
- Modify: `companion/faces/src/main.ts`
- Test: `companion/faces/test/main.test.ts`

**Interfaces:**
- Produces: `RenderContext { state: unknown; event?: { taps: number; point: { x: number; y: number } | null } }`; `FaceDefinition.render(settings, now, context) => Promise<string | { svg: string; state?: unknown }>`; `FaceDefinition.tap?: string`; stdout envelope `{ "png": "<base64>", "state"?: unknown }`.

Every existing face keeps `render(settings, now)` working: the third argument is optional and returning a bare SVG string means "state unchanged".

- [ ] **Step 1: Write the failing tests** in `companion/faces/test/main.test.ts`, following the file's existing style for invoking the seam:

```ts
test("render passes state and the event to the face and returns an envelope", async () => {
  const seen: unknown[] = [];
  const result = await renderRequest(
    { kind: "probe", settings: {}, state: { page: 2 }, event: { taps: 3, point: null } },
    { kind: "probe", label: "Probe", fields: [], tap: "Tap me.",
      render: (_settings, _now, context) => { seen.push(context); return { svg: SQUARE_SVG, state: { page: 5 } }; } },
  );
  expect(seen).toEqual([{ state: { page: 2 }, event: { taps: 3, point: null } }]);
  expect(result.state).toEqual({ page: 5 });
  expect(Buffer.from(result.png, "base64").subarray(0, 4)).toEqual(Buffer.from([0x89, 0x50, 0x4e, 0x47]));
});

test("a face that returns a bare SVG leaves the stored state alone", async () => {
  const result = await renderRequest(
    { kind: "probe", settings: {}, state: { page: 2 } },
    { kind: "probe", label: "Probe", fields: [], render: async () => SQUARE_SVG },
  );
  expect("state" in result).toBe(false);
});

test("a render with no state and no event is what every existing face already gets", async () => {
  const seen: unknown[] = [];
  await renderRequest({ kind: "probe", settings: {} }, {
    kind: "probe", label: "Probe", fields: [],
    render: (_settings, _now, context) => { seen.push(context); return SQUARE_SVG; },
  });
  expect(seen).toEqual([{ state: undefined, event: undefined }]);
});

test("describe carries a face's tap sentence and omits it otherwise", () => {
  const catalog = JSON.parse(describeCatalog([
    { kind: "a", label: "A", fields: [], tap: "Tap the panel for more.", render: async () => SQUARE_SVG },
    { kind: "b", label: "B", fields: [], render: async () => SQUARE_SVG },
  ]));
  expect(catalog[0].tap).toBe("Tap the panel for more.");
  expect("tap" in catalog[1]).toBe(false);
});
```

`renderRequest` and `describeCatalog` do not exist yet — export them from `main.ts` so the seam is testable without spawning a process (`main.ts`'s `render`/`describe` become thin wrappers over them). `SQUARE_SVG` is a minimal 448x368 SVG; reuse the existing helper in the test file if there is one.

- [ ] **Step 2: Run the tests and watch them fail**

```sh
cd companion/faces && bun test test/main.test.ts
```
Expected: FAIL — `renderRequest` is not exported.

- [ ] **Step 3: Implement**

In `face.ts`:

```ts
/** What a face is told beyond its settings: where it left off, and why it is drawing now. */
export interface RenderContext {
  /** Whatever this face returned as `state` last time, or undefined. */
  state?: unknown;
  /**
   * Absent for a scheduled refresh. `taps` is how many taps this render answers --
   * they coalesce, so three quick taps can arrive as one render with taps: 3.
   * `point` is always null until the wire carries one (C2); a face that wants to
   * hit-test gets the point in the same 448x368 space it drew in.
   */
  event?: { taps: number; point: { x: number; y: number } | null };
}

/** A face returns a document, or a document plus the state it wants back next time. */
export type RenderResult = string | { svg: string; state?: unknown };

export interface FaceDefinition {
  kind: string;
  label: string;
  fields: FieldSpec[];
  /** What the window tells the owner a tap does. A face without it ignores taps. */
  tap?: string;
  render(settings: Settings, now: Date, context?: RenderContext): Promise<RenderResult> | RenderResult;
}
```

In `main.ts`, replace the body of `render` with `renderRequest`, and `describe` with `describeCatalog`:

```ts
export function describeCatalog(faces: readonly FaceDefinition[] = FACES): string {
  return JSON.stringify(
    faces.map(({ kind, label, fields, tap }) => (tap === undefined ? { kind, label, fields } : { kind, label, fields, tap })),
    null,
    2,
  );
}

export async function renderRequest(
  request: { kind?: unknown; settings?: unknown; state?: unknown; event?: unknown },
  face?: FaceDefinition,
  now: Date = new Date(),
): Promise<{ png: string; state?: unknown }> {
  const definition = face ?? (typeof request.kind === "string" ? faceOfKind(request.kind) : undefined);
  if (definition === undefined) {
    throw new ConfigurationError(`this faces package has no face of kind ${JSON.stringify(request.kind)}`);
  }
  const values = typeof request.settings === "object" && request.settings !== null ? (request.settings as Settings) : {};
  const result = await definition.render(values, now, { state: request.state, event: tapEvent(request.event) });
  const svg = typeof result === "string" ? result : result.svg;
  const png = Buffer.from(pngFromSvg(svg)).toString("base64");
  // A bare SVG means "leave the stored state alone"; an explicit null clears it.
  return typeof result === "string" || !("state" in result) ? { png } : { png, state: result.state };
}
```

`tapEvent` validates the server's event defensively — `taps` a finite integer clamped to `1..=32`, `point` passed through only as two finite numbers — and returns `undefined` for anything else. The stdout write in `main()` becomes `process.stdout.write(JSON.stringify(await renderRequest(JSON.parse(await Bun.stdin.text()))))`. Keep the rule that nothing but this reaches stdout, and keep the exit codes exactly as they are.

- [ ] **Step 4: Run the tests and the suite**

```sh
cd companion/faces && bun test && bun run check && bun run lint && bun run format:check
```
Expected: PASS, including the 19 existing goldens unchanged — no face's geometry moved.

- [ ] **Step 5: Commit**

```sh
git add companion/faces/src/face.ts companion/faces/src/main.ts companion/faces/test/main.test.ts
git commit -m "feat(faces): render takes state and a tap event, and answers with an envelope"
```

---

### Task 2: The server speaks the envelope, and still accepts a bare PNG

**Files:**
- Modify: `companion/crates/server/src/data_cards/faces_package.rs`
- Modify: `companion/crates/server/src/data_cards/worker.rs` (call site only)
- Modify: `companion/crates/server/tests/support/fake-faces.sh`
- Test: `companion/crates/server/src/data_cards/faces_package.rs` (`#[cfg(test)]` at the bottom, as now)

**Interfaces:**
- Consumes: Task 1's envelope.
- Produces:
```rust
pub(crate) struct RenderRequest<'a> {
    pub(crate) kind: &'a str,
    pub(crate) settings: &'a BTreeMap<String, serde_json::Value>,
    pub(crate) state: Option<&'a serde_json::Value>,
    pub(crate) taps: u32, // 0 means a scheduled refresh
}
pub(crate) struct Rendered { pub(crate) png: Vec<u8>, pub(crate) state: Option<Option<serde_json::Value>> }
pub(crate) fn render(command: &FaceCommand, request: RenderRequest<'_>) -> Result<Rendered, FaceRenderError>;
```
`Rendered.state`: outer `None` = the package said nothing, keep what is stored; `Some(None)` = clear it; `Some(Some(v))` = store `v`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_legacy_bare_png_is_still_a_frame_and_leaves_state_alone() {
    let rendered = render(&fake(), request("weather", &settings(""), None, 0)).expect("a frame");
    assert_eq!(&rendered.png[..4], b"\x89PNG");
    assert!(rendered.state.is_none());
}

#[test]
fn an_envelope_carries_the_frame_and_the_new_state() {
    let rendered = render(&fake(), request("weather", &settings("answer-with-an-envelope"), None, 0))
        .expect("a frame");
    assert_eq!(&rendered.png[..4], b"\x89PNG");
    assert_eq!(rendered.state, Some(Some(serde_json::json!({ "page": 3 }))));
}

#[test]
fn an_envelope_may_clear_the_state() {
    let rendered = render(&fake(), request("weather", &settings("answer-with-a-null-state"), None, 0))
        .expect("a frame");
    assert_eq!(rendered.state, Some(None));
}

#[test]
fn the_request_carries_state_and_the_tap_count_to_the_package() {
    // fake-faces.sh echoes its request to stderr for this setting and exits 1.
    let error = render(&fake(), request("weather", &settings("echo-the-request"), Some(&serde_json::json!({"page": 2})), 3))
        .expect_err("the fake refuses on purpose");
    let FaceRenderError::Transient(message) = error else { panic!("expected transient") };
    assert!(message.contains("\"taps\":3"), "{message}");
    assert!(message.contains("\"page\":2"), "{message}");
}

#[test]
fn a_scheduled_refresh_sends_no_event_at_all() {
    let error = render(&fake(), request("weather", &settings("echo-the-request"), None, 0))
        .expect_err("the fake refuses on purpose");
    let FaceRenderError::Transient(message) = error else { panic!("expected transient") };
    assert!(!message.contains("event"), "{message}");
}

#[test]
fn an_envelope_that_is_not_json_and_not_a_png_is_transient() {
    let error = render(&fake(), request("weather", &settings("answer-with-garbage"), None, 0))
        .expect_err("not a frame");
    assert!(matches!(error, FaceRenderError::Transient(_)));
}
```

Add a `request(kind, settings, state, taps)` test helper building a `RenderRequest`, and extend `fake-faces.sh` with the three new cases (note it matches on words *in the request*, so the settings map carries them):

```sh
*answer-with-an-envelope*) printf '{"png":"%s","state":{"page":3}}' "$(base64 < "$here/fake-face.png" | tr -d '\n')" ;;
*answer-with-a-null-state*) printf '{"png":"%s","state":null}' "$(base64 < "$here/fake-face.png" | tr -d '\n')" ;;
*echo-the-request*) echo "$request" >&2; exit 1 ;;
```

- [ ] **Step 2: Run them and watch them fail**

```sh
cd companion && export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p server data_cards::faces_package > /tmp/t.log 2>&1; echo $?; tail -30 /tmp/t.log
```
Expected: FAIL to compile — `RenderRequest` does not exist.

- [ ] **Step 3: Implement**

```rust
pub(crate) fn render(
    command: &FaceCommand,
    request: RenderRequest<'_>,
) -> Result<Rendered, FaceRenderError> {
    let mut body = serde_json::json!({ "kind": request.kind, "settings": request.settings });
    if let Some(state) = request.state {
        body["state"] = state.clone();
    }
    if request.taps > 0 {
        // `point` is null until the wire carries one. Sending the key now means C2
        // changes the firmware and the server, and no face and no contract.
        body["event"] = serde_json::json!({ "taps": request.taps, "point": serde_json::Value::Null });
    }
    let finished = run(command, "render", body.to_string().as_bytes(), MAX_PNG_BYTES, RENDER_TIMEOUT)
        .map_err(FaceRenderError::Transient)?;
    match finished.code {
        Some(0) if finished.stdout.is_empty() => Err(FaceRenderError::Transient(
            "the faces package exited cleanly without a frame".into(),
        )),
        Some(0) => decode(finished.stdout),
        Some(EXIT_CONFIGURATION) => Err(FaceRenderError::Configuration(message_of(&finished))),
        _ => Err(FaceRenderError::Transient(message_of(&finished))),
    }
}

/// The two forms of a successful `render`.
///
/// The server and the faces package deploy independently -- `deploy.sh --faces-only`
/// ships one without the other -- so an older package answering with a bare PNG must
/// keep working. A PNG's first four bytes are its signature and can never open a JSON
/// document, which makes the two unambiguous.
fn decode(stdout: Vec<u8>) -> Result<Rendered, FaceRenderError> {
    if stdout.starts_with(b"\x89PNG") {
        return Ok(Rendered { png: stdout, state: None });
    }
    #[derive(serde::Deserialize)]
    struct Envelope {
        png: String,
        #[serde(default, deserialize_with = "serde_with_explicit_null")]
        state: Option<Option<serde_json::Value>>,
    }
    let envelope: Envelope = serde_json::from_slice(&stdout)
        .map_err(|error| FaceRenderError::Transient(format!("the faces package answered with neither a PNG nor an envelope: {error}")))?;
    let png = base64_decode(&envelope.png)
        .map_err(|error| FaceRenderError::Transient(format!("the envelope's frame is not base64: {error}")))?;
    Ok(Rendered { png, state: envelope.state })
}
```

Two notes for the implementer:

- Distinguishing "key absent" from `"state": null` is the one subtlety. `Option<Option<T>>` with `#[serde(default)]` alone collapses them; use `deserialize_with` on a `Value` (absent → `None`, `Value::Null` → `Some(None)`, otherwise `Some(Some(v))`). A test above pins each case, so a collapse fails loudly. Remember `value["key"].is_null()` is true for a MISSING key — assert with `value.get("key") == Some(&Value::Null)` if you check JSON directly anywhere.
- For base64, use the crate already in the workspace if there is one (`rg '^base64' companion/Cargo.lock`); otherwise add `base64` to `crates/server/Cargo.toml` — it is a small, ubiquitous dependency and the alternative is hand-rolling a decoder.

Update `worker.rs`'s `render_frame` to build a `RenderRequest` with `state: None, taps: 0` for now; Task 3 and Task 6 fill them in.

- [ ] **Step 4: Run them and watch them pass**

```sh
cd companion && cargo test -p server data_cards > /tmp/t.log 2>&1; echo $?; tail -20 /tmp/t.log
```
Expected: PASS, and the existing `data_cards` tests still pass — the fake's default branch still answers with a bare PNG, which is the legacy path.

- [ ] **Step 5: Commit**

```sh
git add companion/crates/server/src/data_cards companion/crates/server/tests/support/fake-faces.sh
git commit -m "feat(server): render sends state and taps, and accepts an envelope or a bare PNG"
```

---

### Task 3: The face state store

**Files:**
- Create: `companion/crates/server/src/data_cards/face_state.rs`
- Modify: `companion/crates/server/src/data_cards.rs` (module declaration, path derivation, removal hook)
- Modify: `companion/crates/server/src/data_cards/worker.rs` (read before render, write after)
- Test: in `face_state.rs` under `#[cfg(test)]`

**Interfaces:**
- Produces:
```rust
pub(super) struct FaceStateStore { /* path + Mutex<BTreeMap<String, serde_json::Value>> */ }
impl FaceStateStore {
    pub(super) fn load(path: PathBuf) -> Self;             // a missing or corrupt file is "no state"
    pub(super) fn get(&self, source_id: &str) -> Option<serde_json::Value>;
    pub(super) fn put(&self, source_id: &str, state: Option<serde_json::Value>);
    pub(super) fn remove(&self, source_id: &str);
}
pub(super) const MAX_FACE_STATE_BYTES: usize = 16 * 1024;
```
`put` persists the whole file atomically and never returns an error to the caller: a frame was already published, and failing to remember a page number must not fail a refresh. It logs instead.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_missing_file_is_no_state_rather_than_an_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FaceStateStore::load(dir.path().join("face-state.json"));
    assert_eq!(store.get("hn"), None);
}

#[test]
fn a_corrupt_file_is_no_state_and_is_overwritten_by_the_next_put() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("face-state.json");
    std::fs::write(&path, b"{not json").expect("write");
    let store = FaceStateStore::load(path.clone());
    assert_eq!(store.get("hn"), None);
    store.put("hn", Some(serde_json::json!({ "page": 1 })));
    let reloaded = FaceStateStore::load(path);
    assert_eq!(reloaded.get("hn"), Some(serde_json::json!({ "page": 1 })));
}

#[test]
fn state_survives_a_reload_and_a_none_clears_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("face-state.json");
    let store = FaceStateStore::load(path.clone());
    store.put("hn", Some(serde_json::json!({ "page": 3 })));
    assert_eq!(FaceStateStore::load(path.clone()).get("hn"), Some(serde_json::json!({ "page": 3 })));
    store.put("hn", None);
    assert_eq!(FaceStateStore::load(path).get("hn"), None);
}

#[test]
fn state_larger_than_the_cap_is_refused_and_the_previous_state_survives() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("face-state.json");
    let store = FaceStateStore::load(path);
    store.put("hn", Some(serde_json::json!({ "page": 3 })));
    let huge = serde_json::json!({ "junk": "x".repeat(MAX_FACE_STATE_BYTES) });
    store.put("hn", Some(huge));
    assert_eq!(store.get("hn"), Some(serde_json::json!({ "page": 3 })));
}

#[test]
fn removing_a_source_forgets_its_state_and_keeps_its_neighbours() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("face-state.json");
    let store = FaceStateStore::load(path);
    store.put("hn", Some(serde_json::json!({ "page": 3 })));
    store.put("weather", Some(serde_json::json!({ "view": "days" })));
    store.remove("hn");
    assert_eq!(store.get("hn"), None);
    assert_eq!(store.get("weather"), Some(serde_json::json!({ "view": "days" })));
}

#[test]
fn an_unchanged_state_does_not_rewrite_the_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("face-state.json");
    let store = FaceStateStore::load(path.clone());
    store.put("hn", Some(serde_json::json!({ "page": 3 })));
    let first = std::fs::metadata(&path).expect("metadata").modified().expect("mtime");
    store.put("hn", Some(serde_json::json!({ "page": 3 })));
    assert_eq!(std::fs::metadata(&path).expect("metadata").modified().expect("mtime"), first);
}
```

- [ ] **Step 2: Run them and watch them fail**

```sh
cd companion && cargo test -p server face_state > /tmp/t.log 2>&1; echo $?; tail -20 /tmp/t.log
```
Expected: FAIL — the module does not exist.

- [ ] **Step 3: Implement**

The file is `{"version": 1, "sources": { "<source_id>": <state> }}`. Write to `<path>.tmp` then `std::fs::rename`, the way `persist_specs` in `data_cards.rs` already writes specs — read it and match its approach rather than inventing a second one. The cap is checked on `serde_json::to_vec(&state)` length before insertion; over it, log at `warn!` with `target: "server::data_cards"` naming the source and the size, and keep the stored value. Remember: a `tracing` macro takes its target from the enclosing module, so pass `target:` explicitly.

In `data_cards.rs`, derive the path from the spec path (`spec_path.with_file_name("face-state.json")`), own the store beside the specs, and call `remove` from `remove_face`. In `worker.rs`, read `state` before the render and apply `Rendered.state` after a successful frame.

- [ ] **Step 4: Run them and watch them pass**

```sh
cd companion && cargo test -p server face_state > /tmp/t.log 2>&1; echo $?; tail -20 /tmp/t.log
```
Expected: PASS.

- [ ] **Step 5: Commit**

```sh
git add companion/crates/server/src/data_cards.rs companion/crates/server/src/data_cards/
git commit -m "feat(server): faces keep a small per-source state beside their specs"
```

---

### Task 4: `tap` reaches the window's contract

**Files:**
- Modify: `companion/crates/server/src/data_cards/faces_package.rs` (`CatalogFace`)
- Modify: `companion/crates/server/src/data_cards.rs` (`FaceDescriptor`, `describe_face`)
- Modify: `companion/crates/server/src/app_api/contract.rs` (the contract test fixture)
- Test: `data_cards.rs` tests, `app_api/contract.rs`

**Interfaces:**
- Produces: `CatalogFace.tap: Option<String>`, `FaceDescriptor.tap: Option<String>` (serialized as `tap`), and `data_cards::face_takes_taps(state, source_id) -> bool` used by Task 6.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn a_faces_tap_sentence_reaches_the_descriptor_and_absence_means_no_taps() {
    let state = ServerState::in_memory();
    data_cards::set_faces(&state, faces_package::fake());
    let descriptors = creatable_faces(&state);
    let headlines = descriptors.iter().find(|face| face.kind == "headlines").expect("headlines");
    assert_eq!(headlines.tap.as_deref(), Some("Tap the panel for the next stories."));
    let weather = descriptors.iter().find(|face| face.kind == "weather").expect("weather");
    assert_eq!(weather.tap, None);
}
```

Add `"tap": "Tap the panel for the next stories."` to the `headlines` entry in `fake-faces.sh`'s `describe` output (and to no other), so one fake face takes taps and the rest do not.

- [ ] **Step 2: Run it and watch it fail**

```sh
cd companion && cargo test -p server tap_sentence > /tmp/t.log 2>&1; echo $?; tail -20 /tmp/t.log
```
Expected: FAIL — no `tap` field.

- [ ] **Step 3: Implement.** Add `#[serde(default)] pub(crate) tap: Option<String>` to `CatalogFace`, carry it through `describe_face` into `FaceDescriptor`, and add `#[serde(skip_serializing_if = "Option::is_none")]` so a face without one adds no key. Add `face_takes_taps`, which resolves a `source_id` to its spec and asks the catalog whether that kind has a `tap`.

- [ ] **Step 4: Run the test and the contract suite**

```sh
cd companion && cargo test -p server 'app_api::contract' data_cards > /tmp/t.log 2>&1; echo $?; tail -20 /tmp/t.log
```
Expected: PASS. If the contract test enumerates descriptor fields for the TypeScript mirror, update `apps/deskmate/src/lib/types.contract.ts`'s counterpart in Task 10 — note the failure now and fix it there.

- [ ] **Step 5: Commit**

```sh
git add companion/crates/server/src companion/crates/server/tests/support/fake-faces.sh
git commit -m "feat(server): a face may declare what a tap does, and the descriptor carries it"
```

---

### Task 5: `app-core` reports a picture tap through a port

**Files:**
- Modify: `companion/crates/app-core/src/runtime/mod.rs`
- Test: `companion/crates/app-core/tests/runtime/` — put the new cases beside the existing device-event tests (find them with `rg -n "EventKind::Navigation" companion/crates/app-core/tests`)

**Interfaces:**
- Produces:
```rust
pub trait CardTapSink: Send + Sync {
    /// Called on the runtime worker thread. MUST NOT block.
    fn tapped(&self, card_id: &str, source_id: &str);
}
impl RuntimeHandle {
    pub fn start_with_ports(
        config: AppConfig,
        device: Box<dyn RuntimeDevice>,
        options: RuntimeOptions,
        image_source_host: Option<Box<dyn ImageSourceHost>>,
        tap_sink: Option<Arc<dyn CardTapSink>>,
    ) -> Result<Self, RuntimeError>;
}
```
`start` and `start_with_image_source_host` stay, delegating with `None`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_tap_on_a_picture_card_reaches_the_sink_with_its_source() {
    let sink = Arc::new(RecordingTapSink::default());
    let harness = harness_with_tap_sink(config_with_picture("usage", "claude-limits"), Arc::clone(&sink));
    harness.device_emits(tap_event("usage"));
    harness.settle();
    assert_eq!(sink.taken(), vec![("usage".to_owned(), "claude-limits".to_owned())]);
}

#[test]
fn a_tap_on_a_pomodoro_still_toggles_its_timer_and_never_reaches_the_sink() {
    let sink = Arc::new(RecordingTapSink::default());
    let harness = harness_with_tap_sink(config_with_pomodoro("focus"), Arc::clone(&sink));
    harness.device_emits(tap_event("focus"));
    harness.settle();
    assert!(sink.taken().is_empty());
    assert!(harness.snapshot().timer_running("focus"));
}

#[test]
fn a_tap_naming_a_card_that_does_not_exist_is_counted_and_dropped() {
    let sink = Arc::new(RecordingTapSink::default());
    let harness = harness_with_tap_sink(config_with_picture("usage", "claude-limits"), Arc::clone(&sink));
    harness.device_emits(tap_event("ghost"));
    harness.settle();
    assert!(sink.taken().is_empty());
    assert_eq!(harness.diagnostics().taps_dropped, 1);
}

#[test]
fn a_runtime_with_no_sink_drops_a_picture_tap_without_panicking() {
    let harness = harness(config_with_picture("usage", "claude-limits"));
    harness.device_emits(tap_event("usage"));
    harness.settle();
    assert!(harness.is_running());
}
```

`RecordingTapSink` is a `Mutex<Vec<(String, String)>>` implementing `CardTapSink`. Use the existing test harness helpers in that directory rather than writing a new one — `rg -n "fn harness" companion/crates/app-core/tests` shows the pattern, including how a fake device emits an event and how tests pace the runtime (`RuntimeOptions` are settable per test; do not wait on production pacing).

- [ ] **Step 2: Run them and watch them fail**

```sh
cd companion && cargo test -p app-core tap > /tmp/t.log 2>&1; echo $?; tail -30 /tmp/t.log
```
Expected: FAIL to compile — `CardTapSink` does not exist.

- [ ] **Step 3: Implement.** Add the trait and the constructor, store the sink in `WorkerState`, add the diagnostic counter beside the existing ones (mirror `RuntimeDiagnosticCounters`'s existing field, its snapshot field, and its doc comment style), and add the arm in `drain_device_events`:

```rust
(EventKind::Tap, EventAction::StartPause | EventAction::Reset) => {
    // Every picture card is lowered to StartPause so the device reports taps at
    // all (see config.rs's wire_config). What the tap MEANS is the host's
    // business: a pomodoro's tap drives its timer, a picture's tap goes to
    // whatever draws it. This file must not learn what that is.
    match picture_source(&state.config, &received.event.card_id) {
        Some(source_id) => match &state.tap_sink {
            Some(sink) => sink.tapped(&received.event.card_id, &source_id),
            None => diagnostics.taps_dropped.fetch_add(1, Ordering::Relaxed),
        },
        None => { /* existing pomodoro dispatch, unchanged */ }
    }
}
```

`picture_source` returns the `source_id` when the named card exists and is a `CardSettings::Picture`. A tap naming no card at all is counted in `taps_dropped` — the device is untrusted input.

- [ ] **Step 4: Run them and watch them pass**

```sh
cd companion && cargo test -p app-core > /tmp/t.log 2>&1; echo $?; tail -20 /tmp/t.log
```
Expected: PASS.

- [ ] **Step 5: Mutation probe.** Delete the `None => diagnostics...` line, re-run, confirm a test fails; restore it. Then change `picture_source` to return `Some` for every card kind, re-run, confirm the pomodoro test fails; restore. A green suite that survives either edit is not testing this.

- [ ] **Step 6: Commit**

```sh
git add companion/crates/app-core
git commit -m "feat(app-core): a tap on a picture card leaves the runtime through a sink"
```

---

### Task 6: The server routes a tap to the card's refresher

**Files:**
- Modify: `companion/crates/server/src/data_cards.rs` (the `TapSignal` beside each task, `tap`)
- Modify: `companion/crates/server/src/data_cards/worker.rs` (select over interval and taps)
- Modify: wherever the server starts the runtime (`rg -n "start_with_image_source_host" companion/crates/server/src`)
- Test: `data_cards.rs` tests

**Interfaces:**
- Consumes: Tasks 2-5.
- Produces: `data_cards::tapped(state: &ServerState, source_id: &str)`, and a `ServerTapSink` implementing `app_core::CardTapSink` that calls it without blocking.

```rust
/// Taps waiting for a refresher, as a count rather than a queue.
///
/// Three quick taps must page three times, so they accumulate; but a render takes
/// as long as it takes, so they must not queue up renders. One further render
/// carrying the accumulated count is exactly right.
struct TapSignal { pending: AtomicU32, notify: tokio::sync::Notify }
impl TapSignal {
    fn tap(&self) { /* saturating add, capped at MAX_COALESCED_TAPS = 32, then notify_one */ }
    fn take(&self) -> u32 { self.pending.swap(0, Ordering::AcqRel) }
}
```

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn a_tap_on_a_tappable_face_renders_at_once_with_a_tap_count() {
    // fake-faces.sh's "headlines" declares a tap sentence; its render echoes the
    // request when asked, which is how this asserts what reached the package.
    let state = server_with_face("hn", "headlines").await;
    data_cards::tapped(&state, "hn");
    let request = next_render_request(&state).await;
    assert_eq!(request["event"]["taps"], serde_json::json!(1));
}

#[tokio::test]
async fn taps_arriving_during_a_render_coalesce_into_one_further_render() {
    let state = server_with_slow_face("hn", "headlines").await;
    data_cards::tapped(&state, "hn");
    data_cards::tapped(&state, "hn");
    data_cards::tapped(&state, "hn");
    let counts = render_request_counts(&state, 2).await;
    assert_eq!(counts.iter().sum::<u64>(), 3, "every tap is answered, in at most two renders: {counts:?}");
}

#[tokio::test]
async fn a_tap_on_a_face_that_declares_no_tap_is_dropped() {
    let state = server_with_face("sky", "weather").await;
    let before = renders_so_far(&state);
    data_cards::tapped(&state, "sky");
    tokio::task::yield_now().await;
    assert_eq!(renders_so_far(&state), before);
}

#[tokio::test]
async fn a_tap_on_a_source_with_no_spec_is_dropped_without_starting_a_task() {
    let state = server_with_face("hn", "headlines").await;
    data_cards::tapped(&state, "not-a-face"); // an external producer's source
    tokio::task::yield_now().await;
    assert!(!state.data_cards_has_task_for_test("not-a-face"));
}

#[tokio::test]
async fn a_tapped_render_carries_the_stored_state_and_stores_what_comes_back() {
    let state = server_with_face("hn", "headlines").await;
    data_cards::tapped(&state, "hn");
    settle(&state).await;
    assert_eq!(face_state_of(&state, "hn"), Some(serde_json::json!({ "page": 3 })));
    data_cards::tapped(&state, "hn");
    let request = next_render_request(&state).await;
    assert_eq!(request["state"], serde_json::json!({ "page": 3 }));
}

#[tokio::test]
async fn a_tap_reschedules_the_interval_rather_than_refreshing_a_second_later() {
    // A card tapped one second before its interval elapses must not render twice.
    let state = server_with_face_refreshing_every(60, "hn", "headlines").await;
    data_cards::tapped(&state, "hn");
    settle(&state).await;
    assert_eq!(renders_so_far(&state), 2, "the startup render and the tap, and nothing else");
}
```

The helpers (`server_with_face`, `next_render_request`, `renders_so_far`) belong beside the existing `data_cards` tests, which already build a `ServerState` with `fake_faces()`. Have the fake write each request to a file under a directory passed in an env var so a test can read what the package was sent; extend `fake-faces.sh` accordingly and keep it POSIX `sh`.

- [ ] **Step 2: Run them and watch them fail**

```sh
cd companion && cargo test -p server data_cards > /tmp/t.log 2>&1; echo $?; tail -30 /tmp/t.log
```
Expected: FAIL — `tapped` does not exist.

- [ ] **Step 3: Implement.** Keep a `TapSignal` per running refresher, created with the task and dropped with it. `tapped` looks up the signal, checks `face_takes_taps`, and calls `signal.tap()`; every miss bumps a counter and logs at `debug!`. In `refresh_loop`, replace `tokio::time::sleep(wait).await` with:

```rust
let taps = tokio::select! {
    () = tokio::time::sleep(wait) => 0,
    () = signal.notified() => signal.take().max(1),
};
```

and pass `taps` into the next `refresh_once`, which forwards it to `RenderRequest`. Because the loop's next wait is computed after the render, a tap naturally reschedules the interval. `ServerTapSink::tapped` does `state.clone()` plus `tokio::spawn`/`Handle::spawn` of `data_cards::tapped` — never blocking work on the caller's thread, which is the runtime worker.

Wire the sink in where the server starts the runtime, switching that call to `start_with_ports`.

- [ ] **Step 4: Run them and watch them pass**

```sh
cd companion && cargo test -p server > /tmp/t.log 2>&1; echo $?; tail -20 /tmp/t.log
```
Expected: PASS.

- [ ] **Step 5: Mutation probe.** Replace `signal.take().max(1)` with `1`, re-run, confirm the coalescing test fails; restore. Remove the `face_takes_taps` check, re-run, confirm the "declares no tap" test fails; restore.

- [ ] **Step 6: Commit**

```sh
git add companion/crates/server
git commit -m "feat(server): a tap wakes its card's refresher, coalescing taps into one render"
```

---

### Task 7: Every picture card reports taps

**Files:**
- Modify: `companion/crates/app-core/src/config.rs:1048` (`wire_config`)
- Modify: `docs/config/v10.md` (the `tap_action` lowering paragraph)
- Test: `companion/crates/app-core/src/config.rs` tests, `companion/crates/server/src/app_api/contract.rs`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_picture_card_reports_taps_whatever_its_document_says() {
    let config = config_with_picture_tap_action(WidgetTapAction::None);
    let compiled = config.compile(1).expect("valid");
    assert_eq!(compiled.layout.cards[0].tap_action, TapAction::StartPause);
}

#[test]
fn a_clock_still_reports_nothing_and_a_pomodoro_keeps_its_own_action() {
    let compiled = config_with_clock_and_pomodoro().compile(1).expect("valid");
    assert_eq!(compiled.layout.cards[0].tap_action, TapAction::None);
    assert_eq!(compiled.layout.cards[1].tap_action, TapAction::StartPause);
}

#[test]
fn a_picture_cards_document_value_is_untouched_by_lowering() {
    // The document still says what the DEVICE does locally, which for a picture is
    // nothing. Lowering is a host decision and must not rewrite what was saved.
    let config = config_with_picture_tap_action(WidgetTapAction::None);
    let saved = serde_json::to_value(&config).expect("serialize");
    assert_eq!(saved["cards"][0]["tap_action"]["kind"], "none");
}
```

- [ ] **Step 2: Run them and watch them fail**

```sh
cd companion && cargo test -p app-core wire_config picture > /tmp/t.log 2>&1; echo $?; tail -20 /tmp/t.log
```
Expected: FAIL — a picture lowers to `TapAction::None` today.

- [ ] **Step 3: Implement.** In `wire_config`, return `TapAction::StartPause` for `CardSettings::Picture` before consulting `tap_action()`, with a comment naming the spec and the two firmware lines that make it safe (`carousel.c:68`, `scene_view.c:1385`) plus the `v2.md` sentence that sanctions it. Keep every other kind's lowering exactly as it is.

- [ ] **Step 4: Run the workspace tests.** Some existing tests assert a picture's wire config; update them to the new expectation and check each one is still testing what it meant to.

```sh
cd companion && cargo test --workspace --all-targets > /tmp/t.log 2>&1; echo $?; tail -30 /tmp/t.log
```
Expected: PASS.

- [ ] **Step 5: Update `docs/config/v10.md`.** In the `tap_action` paragraph, after the sentence about `requires-capability`, add:

```markdown
A picture card's `tap_action` describes what the *device* does by itself, which is
nothing, and it stays `none`. The host lowers every picture card to the wire's
`StartPause` regardless, because that is the only way protocol v2 reports a tap at all;
what the tap then means is the host's decision, and for a server-rendered face it is a
re-render. See `docs/superpowers/specs/2026-09-23-deskmate-tap-to-face-design.md`. No
stored file, validation rule or wire byte changes because of this paragraph.
```

- [ ] **Step 6: Commit**

```sh
git add companion/crates docs/config/v10.md
git commit -m "feat(app-core): every picture card is lowered so the device reports its taps"
```

---

### Task 8: Hacker News pages on a tap

**Files:**
- Modify: `companion/faces/src/faces/hackernews.ts`
- Modify: `companion/faces/src/cases.ts`, `companion/faces/test/golden/` (new goldens)
- Test: `companion/faces/test/faces.test.ts`

**Interfaces:**
- Produces: state `{ "stories": Story[], "page": number, "tappedAt": string | null }` (`tappedAt` an ISO instant), and `tap: "Tap the panel for the next stories."` on the face definition.

- [ ] **Step 1: Write the failing tests**

```ts
test("a tap moves to the next page without fetching", async () => {
  const fetched: string[] = [];
  const result = await hackernews.render({}, NOW, {
    state: { stories: twentyStories(), page: 0, tappedAt: null },
    event: { taps: 1, point: null },
  });
  expect(fetched).toEqual([]);           // the fetch helper records into `fetched`
  expect(pageOf(result)).toBe(1);
  expect(svgOf(result)).toContain(twentyStories()[4].title);
});

test("three coalesced taps move three pages", async () => {
  const result = await hackernews.render({}, NOW, {
    state: { stories: twentyStories(), page: 0, tappedAt: null },
    event: { taps: 3, point: null },
  });
  expect(pageOf(result)).toBe(3);
});

test("paging wraps at the end", async () => {
  const result = await hackernews.render({}, NOW, {
    state: { stories: twentyStories(), page: 4, tappedAt: null },
    event: { taps: 1, point: null },
  });
  expect(pageOf(result)).toBe(0);
});

test("a scheduled refresh within ten minutes of a tap keeps the page", async () => {
  const result = await hackernews.render({}, NOW, {
    state: { stories: twentyStories(), page: 2, tappedAt: minutesBefore(NOW, 9) },
  });
  expect(pageOf(result)).toBe(2);
});

test("a scheduled refresh more than ten minutes after a tap returns to the front", async () => {
  const result = await hackernews.render({}, NOW, {
    state: { stories: twentyStories(), page: 2, tappedAt: minutesBefore(NOW, 11) },
  });
  expect(pageOf(result)).toBe(0);
});

test("a tap on a face with no state fetches, as a first render does", async () => {
  // Nothing to page through yet: the tap must not draw an empty panel.
  const result = await hackernews.render({}, NOW, { state: undefined, event: { taps: 1, point: null } });
  expect(svgOf(result)).toContain(FIXTURE_FRONT_PAGE[0].title);
});

test("a page with fewer than four stories still draws", async () => {
  const result = await hackernews.render({}, NOW, {
    state: { stories: twentyStories().slice(0, 17), page: 4, tappedAt: null },
  });
  expect(svgOf(result)).toContain(twentyStories()[16].title);
});
```

`twentyStories()` comes from a **captured** front page, not invented titles — the trap says a short top story once broke the lone-lead planner and every fixture name was short. Capture with:

```sh
curl -s https://hacker-news.firebaseio.com/v0/topstories.json | head -c 400
```
and fetch the first 20 items, or reuse the existing captured fixture in `cases.ts` if it holds enough stories.

- [ ] **Step 2: Run them and watch them fail**

```sh
cd companion/faces && bun test test/faces.test.ts
```
Expected: FAIL — `render` takes no context.

- [ ] **Step 3: Implement.** `FETCHED` becomes `MAX_STORIES * 5 + 2`; a scheduled refresh (no `event`) fetches as now and keeps up to 20 stories in state; a tap renders from state without fetching, advancing `page` by `event.taps` modulo the number of pages. State goes back out on every render. The resting page is 0, and the 10-minute rule uses `now` against `tappedAt`.

Add the page indicator to the eyebrow. It is a visible change to an approved-looking face, so keep it small and use `trackedWidth`/`fitTracked` for the tracked run.

- [ ] **Step 4: Run the tests and add the goldens.** Add one case per page state to `cases.ts` (at least: page 0 with the captured front page, page 2, and a last page with a short tail), then:

```sh
cd companion/faces && bun run dump out/ && bun test && bun run check && bun run lint && bun run format:check
```
Expected: PASS, with the 19 pre-existing goldens byte-identical — only the new cases are new files.

- [ ] **Step 5: Owner review gate.** Show the owner `out/*.desk.png` for the Hacker News cases (0.4x is roughly the panel's physical size, and type that reads at 448px can be illegible from a chair). **Do not proceed past this step without the owner's approval of the page indicator.**

- [ ] **Step 6: Commit**

```sh
git add companion/faces
git commit -m "feat(faces): a tap pages Hacker News, from state and without a fetch"
```

---

### Task 9: Weather flips to the coming days

**Files:**
- Modify: `companion/faces/src/faces/weather.ts`
- Modify: `companion/faces/src/cases.ts`, `companion/faces/test/golden/`
- Test: `companion/faces/test/faces.test.ts`

**Interfaces:**
- Produces: state `{ "view": "now" | "days", "tappedAt": string | null }`, and `tap: "Tap the panel for the coming days."`

- [ ] **Step 1: Write the failing tests**

```ts
test("a tap flips to the coming days and another flips back", async () => {
  const days = await weather.render(DUBAI, NOW, { state: { view: "now", tappedAt: null }, event: { taps: 1, point: null } });
  expect(viewOf(days)).toBe("days");
  const back = await weather.render(DUBAI, NOW, { state: { view: "days", tappedAt: NOW.toISOString() }, event: { taps: 1, point: null } });
  expect(viewOf(back)).toBe("now");
});

test("an even number of coalesced taps lands where it started", async () => {
  const result = await weather.render(DUBAI, NOW, { state: { view: "now", tappedAt: null }, event: { taps: 2, point: null } });
  expect(viewOf(result)).toBe("now");
});

test("a scheduled refresh more than ten minutes after a tap returns to now", async () => {
  const result = await weather.render(DUBAI, NOW, { state: { view: "days", tappedAt: minutesBefore(NOW, 11) } });
  expect(viewOf(result)).toBe("now");
});

test("the current-conditions view is byte-identical to the approved design", async () => {
  // The approved weather face must not move a pixel because a second view exists.
  const result = await weather.render(DUBAI, NOW, { state: { view: "now", tappedAt: null } });
  expect(svgOf(result)).toBe(readFileSync(`${import.meta.dir}/golden/weather-dubai.svg`, "utf8"));
});
```

(Use the real existing golden's name — check `ls companion/faces/test/golden/`.)

- [ ] **Step 2: Run them and watch them fail**

```sh
cd companion/faces && bun test test/faces.test.ts
```
Expected: FAIL.

- [ ] **Step 3: Implement.** `forecast_days` moves from 2 to 5 in the existing Open-Meteo request; parse `daily.time`, `weather_code`, `temperature_2m_max/min` into a `DailyStep[]`. Draw the days view as its own renderer beside the current one, sharing the eyebrow and the kit helpers. The current view's drawing code is untouched — the byte-identical test above is what proves it.

- [ ] **Step 4: Run the tests, add goldens**

```sh
cd companion/faces && bun run dump out/ && bun test && bun run check && bun run lint && bun run format:check
```
Expected: PASS, every pre-existing golden byte-identical.

- [ ] **Step 5: Owner review gate.** Show the owner the new `weather-*-days.desk.png` files. **The forecast layout ships only with the owner's approval.**

- [ ] **Step 6: Commit**

```sh
git add companion/faces
git commit -m "feat(faces): a tap flips weather between now and the coming days"
```

---

### Task 10: The window says what a tap does

**Files:**
- Modify: `companion/apps/deskmate/src/lib/types.ts` (`FaceDescriptor`)
- Modify: `companion/apps/deskmate/src/lib/types.contract.ts` if it mirrors the Rust contract
- Modify: `companion/apps/deskmate/src/components/CardEditor.tsx` (~line 299, the `source-settings__fields` fieldset)
- Modify: `companion/apps/deskmate/src/dev/mockBackend.ts` (one mock face declares a tap)
- Modify: `companion/apps/deskmate/src/styles.css`
- Test: `companion/apps/deskmate/tests/cardEditor.test.tsx`

- [ ] **Step 1: Write the failing test**

```tsx
test("a tappable face tells the owner what a tap does, and others say nothing", async () => {
  renderEditor(sourceWithFace({ kind: "headlines", label: "Hacker News", fields: [], tap: "Tap the panel for the next stories." }));
  expect(await screen.findByText("Tap the panel for the next stories.")).toBeTruthy();
  cleanup();
  renderEditor(sourceWithFace({ kind: "weather", label: "Weather", fields: [] }));
  expect(screen.queryByText(/Tap the panel/)).toBeNull();
});
```

Follow the file's existing helpers rather than inventing `renderEditor`/`sourceWithFace` if equivalents already exist.

- [ ] **Step 2: Run it and watch it fail**

```sh
cd companion/apps/deskmate && bun test tests/cardEditor.test.tsx
```
Expected: FAIL — no such text.

- [ ] **Step 3: Implement.** Add `tap?: string` to `FaceDescriptor`, and render it under the fieldset's fields as one quiet line of helper text — per `DESIGN.md`, no label above a heading, no card inside a card. Add the mock so the dev harness shows it too, but **only for a face that really declares one** — a harness must not offer what the shipped app does not have.

- [ ] **Step 4: Run the frontend suite**

```sh
cd companion/apps/deskmate && bun test && bun run check && bun run format:check && bun run build
```
Expected: PASS. Remember the DOM suite cannot see a stylesheet, so a CSS mistake survives it — check the rendered page in Chrome in Task 11.

- [ ] **Step 5: Commit**

```sh
git add companion/apps/deskmate
git commit -m "feat(app): the editor says what a tap does on a face that takes one"
```

---

### Task 11: Gates, deploy, and the board

**Files:**
- Modify: `docs/hardware/board-notes.md`
- Modify: `docs/roadmap.md` (the board's C1 line)

- [ ] **Step 1: Run the full gate suite** from `companion/`, each as its own line, redirecting to a file and checking `$?`:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo test --workspace --doc
(cd apps/deskmate && bun test && bun run check && bun run format:check && bun run build)
(cd faces && bun test && bun run check && bun run lint && bun run format:check)
```
Expected: all green. Nothing under `firmware/` changed, so the firmware gates are not owed — confirm with `git diff --stat main -- firmware/` printing nothing.

- [ ] **Step 2: Deploy.** `companion/crates/server/deploy/deploy.sh` (the faces ship as a directory and their suite runs on the VM). The server and the faces both changed, so deploy both. A no-op deploy is ~4 s and a real one ~23 s.

- [ ] **Step 3: Drive the window in Chrome** against the live server: open a face card's editor, confirm the tap line reads correctly and nothing in the layout broke. The mock harness cannot see anything that needs the server, and the DOM suite cannot see the stylesheet — this is the honest check.

- [ ] **Step 4: Verify on `dev-0005`.** The board is normally powered off; power it on. Do not run long retry batteries — take what the board shows, once, and record it. Observe and write down:
  - a redeploy against the live board produces **no `StaleRevision`** (every picture card's wire config changed once);
  - a tap on a Hacker News card **pages the panel**, at rotation 90 and at 270;
  - a tap on a weather card **flips to the coming days**, and again **flips back**;
  - the **tap-to-redraw latency**, wall clock, for Hacker News (paging from state, so no fetch) — the number the spec says is unmeasured;
  - a tap on a clock or pomodoro card behaves exactly as before.

- [ ] **Step 5: Write the board notes.** Append a dated entry to `docs/hardware/board-notes.md` with exactly what was observed, in those words, and what was not. A frame in the server's store is not the panel; say which was checked. If anything failed, record the failure — do not retry into a green.

- [ ] **Step 6: Update the roadmap board** — C1's line to what is true, and whether phase 2 is unblocked.

- [ ] **Step 7: Commit, open the PR, confirm CI**

```sh
git add docs
git commit -m "docs: tap to face observed on dev-0005"
git push -u origin track-c1-tap-to-face
gh pr create --fill
gh run list --branch track-c1-tap-to-face
```
A skipped job is grey, not red: CI was silently skipping the companion gates for four weeks once. Confirm the companion job actually **ran**.

---

## Self-review

- **Spec coverage.** Lowering → Task 7. The sink → Task 5. Routing and coalescing → Task 6. The seam and both compatibility directions → Tasks 1-2. State storage and its caps → Task 3. `tap` in the descriptor → Task 4. The window → Task 10. The two faces and the 10-minute rule → Tasks 8-9. Testing, deploy and hardware → Task 11. Phase 2 (the stream deck) is deliberately **not** in this plan: the spec says it starts only after phase 1 is observed on the board, and its plan is written at this plan's exit.
- **Known soft spots**, flagged rather than hidden: Task 5's counter field name (`taps_dropped`) and Task 6's helper names must match whatever the existing files already use — the implementer reads the neighbours first. Task 2's base64 dependency may or may not already be in the workspace.
- **Ticking.** If you execute this plan, tick as you go. An unticked box here means not done, which is *not* true of older plans in this repo.
