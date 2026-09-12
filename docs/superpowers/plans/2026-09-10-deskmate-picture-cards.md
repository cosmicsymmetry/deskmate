# Picture Cards Implementation Plan

> **STATUS (2026-09-10): ALL ELEVEN TASKS ARE IMPLEMENTED AND MERGED on
> `feat/picture-cards`. The checkboxes below were NOT ticked during execution —
> do not read an unticked `- [ ]` here as outstanding work.** All four workspace
> gates pass (`fmt`, `clippy --workspace --all-targets -D warnings`,
> `test --workspace --all-targets` at **1035 passed / 0 failed**, and
> `test --workspace --doc`), plus the frontend at **149 passed / 0 failed** and a
> clean `bun run build`.
>
> **What is genuinely still owed, and nothing else:**
> 1. A real-pointer check of the add-card menu **in the actual app**, not the
>    Chrome harness. The harness cannot see the WKWebView focus behaviour that
>    once made no card of any kind addable, and neither can an `AXPress`-driven
>    check.
> 2. The server redeploy, in the order §12 gives — **binary first**, because it
>    compiles its own `CURRENT_SCHEMA_VERSION` and until it moves every v7 save is
>    refused.
> 3. The `claude-limits` producer change, which lives in the TRMNL repository and
>    is the owner's to commit.
> 4. Task 11 Step 5's three hardware observations.
>
> Task 8 was amended during execution; the amendment is recorded immediately
> above its task section.


> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a card kind whose face is a 448x368 PNG that an arbitrary external producer pushes to a token-addressed webhook, rendered on the device as a durable, digest-addressed full-canvas image.

**Architecture:** A new server subsystem (`image_sources`) owns token-digest auth, canonical-frame persistence and cadence inference, entirely separate from the frozen plugin registry. Ingest validates the PNG header *before* decoding, canonicalizes through the existing `rasterizer.rs` RGB565 path, and injects the frame into the long-lived runtime worker as a command — the same seam `InjectPluginSnapshot` already uses. `build_card_scene` gains a `Picture` arm that resolves a source id to a resident digest and returns a single-node `SceneImage` scene, so negotiation returns `Native` by construction and rotation moves no bytes. No firmware change, no protocol change, no capability bit.

**Tech Stack:** Rust (axum server, app-core runtime), the `png` crate for decode, `resvg`-adjacent canonicalization already in `server/src/rasterizer.rs`, TypeScript/React for the Tauri companion app.

**Spec:** `docs/superpowers/specs/2026-09-09-deskmate-picture-cards-design.md` — read it before Task 1. The plan argues from the spec; where they disagree, the spec wins and the plan is wrong.

**Branch:** `feat/picture-cards`, worktree `.worktrees/picture-cards`, branched off `chore/debt-cleanup` (which has 14 commits unmerged into `main`).

## Global Constraints

- **Schema moves v6 -> v7.** `CURRENT_SCHEMA_VERSION` lives at `app-core/src/config.rs:355` and there is exactly one. The deployed server compiles its own copy, so **redeploy the server binary before saving a v7 config** or every save fails with the typed "schema version 7 is not supported; expected 6" error.
- **No firmware change. No protocol change. No capability bit.** `PROTOCOL_CURRENT_CAPABILITIES` stays **1003**. Protocol stays **v1**. A picture card compiles to `TemplateKind::DigitalClock` on the wire like every plugin card, so the device learns nothing new.
- **Named constants, each with a boundary test:** `MAX_IMAGE_SOURCES = 8`, `MIN_PUSH_INTERVAL = 5s`, `STALE_MULTIPLE = 3`, `STALE_FLOOR = 15 min`, `STALE_CEILING = 48 h`, `PUSH_TIME_RING = 8`, ingest body limit `1 MiB`, required dimensions **exactly 448x368**, 8-bit, RGB or RGBA.
- **No credential enters the config.** `ImageSource` carries `{ id, name }` only. Token digests live in the server's own store, keyed by source id, mirroring device identities (`server/src/registry.rs`): SHA-256 digests only, plaintext returned once at mint and never persisted.
- **Digest is over the DECODED canonical blob**, never over the PNG — stage 4's rule. The stored `frame_path` holds the decoded RGB565 blob so a reconnect re-syncs without re-decoding and the digest stays stable.
- **`AssetRelease.digests` is a device-wide KEEP-SET; an omission means delete.** The desired-asset set must be a union of plugin-registry assets and every image source's frame, computed by **one function with one caller**. This is the exact shape of the empty-registry wipe fixed during stage 3b.
- **Bytes precede any digest reference.** Asset reconciliation runs before `apply_layout`/any scene naming the digest, the ordering `synchronize_full` already obeys.
- **Failure keeps the last good face.** A failed asset transfer pushes no scene, keeps the previous digest active, and releases nothing.
- **Wall-clock, not `Instant`, for the push-time ring** — it is persisted across restarts and a monotonic clock does not survive one.
- **Naming rule:** `cardLabel()` returns the template name ("Picture"); `cardTitle()` returns the owner's words, null when never typed. Every control acting on one entry names `template — title`.
- **Manifests are frozen, not removed.** No new manifest features, no new curated manifests, no contract amendments.
- **Workspace gates** (all four, from `companion/`, with `export PATH="$HOME/.cargo/bin:$PATH"` first):
  `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace --all-targets`; `cargo test --workspace --doc`. Both test invocations are required — neither alone covers the workspace. Firmware gates are untouched because no firmware changes.

---
## Findings that correct the spec

Three of the spec's stated assumptions did not survive verification against the tree.
They are corrected here; the spec's *intent* is unchanged in every case.

1. **`app_core::secure_file` is NOT public on this branch.** The spec attributes that to
   "V3 sub-project 1", and it is true — on `feat/v3-server-host`, which is not merged.
   Here `app-core/src/lib.rs:10` still reads `mod secure_file;`. Task 1 fixes it.
2. **`secure_file::write_and_replace` appends a trailing `\n`** (`secure_file.rs:126`).
   It is text-oriented. Writing a 329,740-byte canonical blob through it produces a
   329,741-byte file, and reading it back yields a blob that fails `frame_png`'s exact
   length check — a silent corruption, not an error. Task 1 adds a binary-safe sibling.
3. **The `png` crate is not reachable from `server` via `lvgl-sim`.** `lvgl-sim` is only a
   *dev*-dependency of `app-core`, and `server` does not depend on it at all. `png` 0.17.16
   *is* in the graph (`server -> resvg =0.45.1 -> tiny-skia 0.11.4 -> png`), so Task 3 adds
   `png = "0.17"` as a direct dependency of `server`: same already-locked version, no new
   vendor, no lockfile churn.

One further correction, to a conclusion a reader might reach on their own: the doc on
`MAX_DURABLE_REGISTRY_ASSETS` reserves the 32nd digest slot for "the one active volatile
raster frame", which invites the inference that an ingested PNG should be volatile. **It
should not.** The spec rejected volatile explicitly (§3): two PSRAM slots serve one
regenerating card, not a loop of them. Picture frames are durable; the volatile slot stays
reserved for what stage 4 built it for.

## File Structure

| File | Responsibility | Task |
| --- | --- | --- |
| `app-core/src/secure_file.rs` | + `write_and_replace_binary` (no trailing newline); module made `pub` | 1 |
| `app-core/src/config.rs` | `ImageSource`, `CardSettings::Picture`, v7 constant, validation | 2 |
| `app-core/src/store.rs` | v6 -> v7 migration arm, `ConfigOrigin::MigratedV6` | 2 |
| `server/src/image_ingest.rs` | **new** — header-first PNG validation, decode, canonical RGB565 + digest. Pure, no I/O | 3 |
| `server/src/image_sources.rs` | **new** — token mint/verify, frame persistence, push-time ring, capacity | 4 |
| `server/src/image_staleness.rs` | **new** — pure cadence inference (median, clamp, min samples) | 5 |
| `server/src/images.rs` | **new** — the three routes and their typed errors | 6 |
| `server/src/rasterizer.rs` | `encode_rgb565` + `RasterizedFrame` made `pub(crate)`-visible to `image_ingest` | 3 |
| `app-core/src/runtime.rs` | shared frame-scene helper, `Picture` arm, `ImageSourceUpdated`, staleness compare | 7 |
| `app-core/src/commands.rs` | `RuntimeCommand::ImageSourceUpdated` | 7 |
| `server/src/plugin_host.rs` | `desired_assets` returns the **union**; `image_source_frame` | 8 |
| `apps/deskmate/src/**` | Picture card kind: contract, add menu, editor, tile | 9 |
| `docs/config/v7.md`, `docs/images/producer-guide.md` | contracts and producer guidance | 10 |

**Ownership note for parallel execution.** Tasks 3 and 5 create files nothing else touches
and depend only on things already in the tree; they can run concurrently with Task 2. Task
4 needs Task 1. Tasks 6, 7, 8 need 2-5. Task 9 needs Task 2's contract. Task 10 needs
nothing. Do not let two workers hold `config.rs` or `runtime.rs` at once.

---
### Task 1: Publish `secure_file` and give it a binary-safe write

`secure_file` is private on this branch, and its one write function corrupts binary
payloads. Both must be fixed before Task 4 can persist a frame. They are one task because
they are one file and one reviewer's gate.

**Files:**
- Modify: `companion/crates/app-core/src/lib.rs:10`
- Modify: `companion/crates/app-core/src/secure_file.rs`
- Test: `companion/crates/app-core/tests/secure_file.rs` (create if absent)

**Interfaces:**
- Produces: `app_core::secure_file::{write_and_replace, write_and_replace_binary, read_bounded, usable_parent, create_directory, sync_parent, BoundedReadError, FileIoError}` — all `pub`. Task 4 consumes `write_and_replace_binary`, `read_bounded`, `usable_parent`, `create_directory`, `sync_parent`.

- [ ] **Step 1: Cherry-pick the visibility commit that already exists**

```bash
cd /Users/rodion/dev/deskmate/.worktrees/picture-cards
git cherry-pick 994a422
```

That commit is `feat: expose app-core::secure_file as public API for the secrets store`
on `feat/v3-server-host`. Cherry-picking rather than retyping means both branches carry
identical content, so the eventual V3 merge is a no-op instead of a conflict. It also
brings `companion/crates/server/tests/secure_file_reexport.rs` with it.

If it conflicts, the conflict is `lib.rs`'s module list only: keep `pub mod secure_file;`
and every other line unchanged.

- [ ] **Step 2: Write the failing test for a binary-safe write**

Add to `companion/crates/app-core/tests/secure_file.rs`:

```rust
use app_core::secure_file;

#[test]
fn a_binary_write_appends_no_trailing_newline() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("frame.bin");
    // A canonical frame is binary and length-checked by its reader. One extra
    // byte is silent corruption, not an error, so pin the exact length.
    let blob: Vec<u8> = (0u8..=255).cycle().take(4_096).collect();

    secure_file::write_and_replace_binary(&path, &blob).expect("binary write");

    let read_back = std::fs::read(&path).expect("read back");
    assert_eq!(read_back.len(), blob.len());
    assert_eq!(read_back, blob);
}

#[test]
fn the_text_write_still_appends_its_newline() {
    // The text path is what every config writer uses; changing it is out of scope.
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("config.json");

    secure_file::write_and_replace(&path, b"{}").expect("text write");

    assert_eq!(std::fs::read(&path).expect("read back"), b"{}\n");
}

#[cfg(unix)]
#[test]
fn a_binary_write_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("frame.bin");

    secure_file::write_and_replace_binary(&path, b"bytes").expect("binary write");

    let mode = std::fs::metadata(&path).expect("metadata").permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
}
```

If `tempfile` is not already a dev-dependency of `app-core`, add it:
`tempfile = "3"` under `[dev-dependencies]` in `companion/crates/app-core/Cargo.toml`.
Check first — it is used elsewhere in the workspace.

- [ ] **Step 3: Run it and watch it fail**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/picture-cards/companion
cargo test -p app-core --test secure_file
```

Expected: FAIL — `write_and_replace_binary` does not exist.

- [ ] **Step 4: Implement it by extracting the shared body**

In `companion/crates/app-core/src/secure_file.rs`, replace the existing
`write_and_replace` (currently at `:115-135`) with these three functions. The existing
one writes `bytes` then unconditionally writes `b"\n"`; the split keeps that behaviour
for text and omits it for binary, with one copy of the atomic/0600 machinery.

```rust
/// Atomically replaces `target` with `bytes` followed by a trailing newline,
/// at mode 0600. This is the text path: every configuration writer in this
/// crate expects the newline, so it is part of the contract, not an accident.
pub fn write_and_replace(target: &Path, bytes: &[u8]) -> Result<(), FileIoError> {
    write_atomic(target, bytes, true)
}

/// Atomically replaces `target` with exactly `bytes` -- no trailing newline --
/// at mode 0600.
///
/// A canonical RGB565 frame is 329,740 bytes and its reader checks that length
/// exactly (`server::rasterizer::frame_png`). Writing one through the text path
/// yields 329,741 bytes on disk, which reads back as a corrupt frame rather than
/// as an error. Binary payloads use this.
pub fn write_and_replace_binary(target: &Path, bytes: &[u8]) -> Result<(), FileIoError> {
    write_atomic(target, bytes, false)
}

fn write_atomic(target: &Path, bytes: &[u8], trailing_newline: bool) -> Result<(), FileIoError> {
    let mut options = AtomicWriteFile::options();
    secure_atomic_options(&mut options);
    let mut file = options.open(target).map_err(|source| FileIoError {
        operation: FileOperation::CreateTemporary,
        source,
    })?;
    file.write_all(bytes).map_err(|source| FileIoError {
        operation: FileOperation::WriteTemporary,
        source,
    })?;
    if trailing_newline {
        file.write_all(b"\n").map_err(|source| FileIoError {
            operation: FileOperation::FinishTemporary,
            source,
        })?;
    }
    file.commit().map_err(|source| FileIoError {
        operation: FileOperation::SyncAndReplace,
        source,
    })
}
```

- [ ] **Step 5: Run the tests and the reexport gate**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/picture-cards/companion
cargo test -p app-core --test secure_file
cargo test -p server --test secure_file_reexport
cargo test -p app-core --test store
```

Expected: all PASS. The `store` run matters because every config write goes through
`write_and_replace`; if the newline moved, those tests catch it.

- [ ] **Step 6: Mutation probe**

Delete the `if trailing_newline` guard so the binary path also writes `b"\n"`, re-run
`cargo test -p app-core --test secure_file`, and confirm
`a_binary_write_appends_no_trailing_newline` goes red. Restore the guard.

- [ ] **Step 7: Commit**

```bash
git add companion/crates/app-core/src/secure_file.rs \
        companion/crates/app-core/tests/secure_file.rs \
        companion/crates/app-core/Cargo.toml
git commit -m "feat: give secure_file a binary-safe atomic write"
```

---
### Task 2: Schema v7 — `ImageSource` and the `Picture` card

**Files:**
- Modify: `companion/crates/app-core/src/config.rs`
- Modify: `companion/crates/app-core/src/store.rs`
- Test: `companion/crates/app-core/tests/config.rs`, `companion/crates/app-core/tests/store.rs`

**Interfaces:**
- Produces, all `pub` from `app_core::config` (and re-exported at the crate root beside the existing card types):
  - `pub struct ImageSource { pub id: String, pub name: String }`
  - `CardSettings::Picture { id: String, title: String, source_id: String, tap_action: WidgetTapAction, refresh: RefreshPolicy, alert: CardAlert }`
  - `AppConfig.image_sources: Vec<ImageSource>`
  - `pub const MAX_IMAGE_SOURCES: usize = 8;`
  - `pub const MAX_IMAGE_SOURCE_NAME_LEN: usize = 48;`
  - `CURRENT_SCHEMA_VERSION == 7`
- Consumed by: Task 6 (routes read `image_sources`), Task 7 (`Picture` arm), Task 8 (union), Task 9 (TS contract).

**The trap that will bite an implementer who skips it.** `AppConfig` is
`#[serde(deny_unknown_fields)]` and **every one of its fields is required** — the only
`#[serde(default)]` in the whole 2,474-line file is `AppPreferences::orientation`
(`config.rs:942`). The v4/v5/v6 migration arm in `store.rs` works by parsing the *old*
document text directly into the *current* `AppConfig` struct. So adding `image_sources`
without `#[serde(default)]` makes **every existing saved config fail to parse** — and
this repository has already paid once for a validation failure presented as lost settings.
`#[serde(default)]` is mandatory, and Step 1's test is what proves it.

- [ ] **Step 1: Write the failing migration test**

Add to `companion/crates/app-core/tests/store.rs`:

```rust
#[test]
fn a_v6_document_migrates_to_v7_with_no_image_sources_and_loses_nothing() {
    // A real v6 document: no `image_sources` key at all. It must parse, not fail.
    let v6 = serde_json::json!({
        "schema_version": 6,
        "preferences": { "timezone": "UTC", "autostart": false, "paused": false },
        "cards": [{
            "kind": "clock",
            "id": "clock",
            "title": "Desk",
            "show_seconds": true,
            "template": "digital-clock",
            "tap_action": "none",
            "refresh": "device-local",
            "alert": "none"
        }],
        "assets": [],
        "playlists": [{
            "id": "my-playlist",
            "name": "My playlist",
            "advance": "manual",
            "entries": [{ "card_id": "clock", "dwell_seconds": null }]
        }],
        "active_playlist_id": "my-playlist",
        "updater": { "channel": "stable", "automatic": true }
    });

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("config.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&v6).expect("encode")).expect("write");

    let store = app_core::ConfigStore::new(path);
    let loaded = store.load();
    let config = loaded.config();

    assert_eq!(config.schema_version, app_core::CURRENT_SCHEMA_VERSION);
    assert_eq!(config.schema_version, 7);
    assert!(config.image_sources.is_empty(), "v6 knew no sources");
    // Lossless: everything else survived untouched.
    assert_eq!(config.cards.len(), 1);
    assert_eq!(config.active_playlist_id, "my-playlist");
    assert_eq!(config.playlists[0].entries[0].card_id, "clock");
}
```

Match the `updater` object above to the real `UpdaterSettings` field names; read them
from `config.rs` first rather than trusting this snippet. If `ConfigStore::load`'s return
shape differs, follow the existing v3/v4 migration tests in the same file — copy their
call shape exactly.

- [ ] **Step 2: Run it and watch it fail**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/picture-cards/companion
cargo test -p app-core --test store a_v6_document_migrates
```

Expected: FAIL — `no field \`image_sources\`` on `AppConfig`.

- [ ] **Step 3: Add the constant bump, the type, and the field**

In `companion/crates/app-core/src/config.rs`:

At `:355`, change the version:

```rust
pub const CURRENT_SCHEMA_VERSION: u32 = 7;
```

Beside the other bounds (after `MAX_PLAYLIST_NAME_LEN` at `:390`), add:

```rust
/// How many named image sources one configuration may declare.
///
/// Eight frames is about 2.6 MB of the 6 MB `assets` partition and eight of the
/// thirty-one durable digests (`MAX_DURABLE_REGISTRY_ASSETS`), leaving the rest
/// for the curated plugins. Raising it needs both budgets re-checked, not just
/// this number.
pub const MAX_IMAGE_SOURCES: usize = 8;
pub const MAX_IMAGE_SOURCE_NAME_LEN: usize = 48;
```

Add the type beside the other document-level structs (near `AssetSettings`):

```rust
/// A named destination an external producer pushes pictures to.
///
/// Authoring identity and nothing else. The credential that authorizes a push
/// is **not** here and never enters a configuration: the server keeps a SHA-256
/// digest keyed by `id`, exactly as it does for device identities, and the
/// plaintext token is returned once at mint and never persisted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageSource {
    pub id: String,
    pub name: String,
}
```

Add the field to `AppConfig` (`:479-489`), **with `#[serde(default)]`**:

```rust
    /// Named image sources. `#[serde(default)]` is load-bearing: the migration
    /// path parses a v4/v5/v6 document straight into this struct, and those
    /// documents have no such key. Without the default every saved config on
    /// earth fails to parse and the user is told their settings are invalid.
    #[serde(default)]
    pub image_sources: Vec<ImageSource>,
```

Add `image_sources: Vec::new(),` to `impl Default for AppConfig` (`:491-518`).

- [ ] **Step 4: Extend the migration arm and `ConfigOrigin`**

In `companion/crates/app-core/src/store.rs`:

Add `MigratedV6` to `ConfigOrigin` (`:121-131`), beside `MigratedV5`.

Change the arm at `:961` from `version @ (4 | 5)` to `version @ (4 | 5 | 6)`, extend its
`match version` to map `6 => ConfigOrigin::MigratedV6`, and append one sentence to the
existing comment:

```rust
            // ... and the v6->v7 change adds only `image_sources`, which every
            // older document lacks and `#[serde(default)]` supplies as empty, so
            // this stays a version bump with no data transformation.
```

Wherever `ConfigOrigin` is matched exhaustively (search the workspace for
`MigratedV5`), add the `MigratedV6` arm alongside it, mirroring V5's behaviour.

- [ ] **Step 5: Run the migration test**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/picture-cards/companion
cargo test -p app-core --test store
```

Expected: PASS, including every pre-existing migration test.

- [ ] **Step 6: Write the failing tests for the Picture card**

Add to `companion/crates/app-core/tests/config.rs`:

```rust
use app_core::config::{
    AppConfig, CardAlert, CardSettings, ImageSource, RefreshPolicy, ValidationCode,
    WidgetTapAction,
};

fn picture_card(id: &str, source_id: &str) -> CardSettings {
    CardSettings::Picture {
        id: id.to_owned(),
        title: "Limits".to_owned(),
        source_id: source_id.to_owned(),
        tap_action: WidgetTapAction::None,
        refresh: RefreshPolicy::Manual,
        alert: CardAlert::None,
    }
}

#[test]
fn a_picture_card_naming_a_known_source_validates() {
    let mut config = AppConfig::default();
    config.image_sources = vec![ImageSource {
        id: "limits".into(),
        name: "Claude limits".into(),
    }];
    config.cards.push(picture_card("shot", "limits"));
    config.playlists[0].entries.push(app_core::config::PlaylistEntry {
        card_id: "shot".into(),
        dwell_seconds: None,
    });

    assert!(config.validate().is_ok(), "{:?}", config.validate());
}

#[test]
fn a_picture_card_naming_an_unknown_source_is_a_typed_missing_reference() {
    let mut config = AppConfig::default();
    config.cards.push(picture_card("shot", "nope"));
    config.playlists[0].entries.push(app_core::config::PlaylistEntry {
        card_id: "shot".into(),
        dwell_seconds: None,
    });

    let issues = config.validate().expect_err("unknown source must not validate").issues;
    let issue = issues
        .iter()
        .find(|issue| issue.path.ends_with("source_id"))
        .expect("an issue naming source_id");
    // Typed, never a silently blank card.
    assert_eq!(issue.code, ValidationCode::MissingReference);
    assert!(issue.message.contains("nope"));
}

#[test]
fn a_picture_card_compiles_to_the_digital_clock_wire_template() {
    // The device learns nothing new: same byte every plugin card sends.
    let mut config = AppConfig::default();
    config.image_sources = vec![ImageSource { id: "limits".into(), name: "L".into() }];
    config.cards = vec![picture_card("shot", "limits")];
    config.playlists[0].entries = vec![app_core::config::PlaylistEntry {
        card_id: "shot".into(),
        dwell_seconds: None,
    }];

    let compiled = config.compile(1).expect("compiles");
    assert_eq!(
        compiled.layout.widgets[0].template,
        protocol::TemplateKind::DigitalClock
    );
}

#[test]
fn more_than_the_maximum_image_sources_is_rejected() {
    let mut config = AppConfig::default();
    config.image_sources = (0..=app_core::config::MAX_IMAGE_SOURCES)
        .map(|index| ImageSource {
            id: format!("source-{index}"),
            name: format!("Source {index}"),
        })
        .collect();

    let issues = config.validate().expect_err("over capacity").issues;
    assert!(issues.iter().any(|issue| issue.code == ValidationCode::TooMany));
}

#[test]
fn two_image_sources_may_not_share_an_id() {
    let mut config = AppConfig::default();
    config.image_sources = vec![
        ImageSource { id: "same".into(), name: "One".into() },
        ImageSource { id: "same".into(), name: "Two".into() },
    ];

    let issues = config.validate().expect_err("duplicate id").issues;
    assert!(issues.iter().any(|issue| issue.code == ValidationCode::DuplicateId));
}
```

- [ ] **Step 7: Run them and watch them fail**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/picture-cards/companion
cargo test -p app-core --test config
```

Expected: FAIL to compile — no `CardSettings::Picture`.

- [ ] **Step 8: Add the variant in all four required places**

`CardSettings`'s `Deserialize` is **hand-written**, so a new variant needs mirroring in
four places or it will serialize but refuse to parse back.

**(a)** The public enum, after `Plugin` (`config.rs:1286`):

```rust
    /// A card whose face is a PNG an external producer pushed to a named image
    /// source. Like `Plugin`, it carries no `template` field: the picture *is*
    /// the layout, so there is nothing to select among. `source_id` names the
    /// source; the server resolves it to a resident asset digest and pushes a
    /// single full-canvas image scene.
    Picture {
        id: String,
        title: String,
        source_id: String,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
        alert: CardAlert,
    },
```

**(b)** `strict_tagged_enum::CardSettingsInner::Picture` (beside the `Plugin` inner at
`config.rs:158-169`) — identical shape with `super::`-qualified types.

**(c)** `allowed_fields` (`config.rs:317-325`):

```rust
            "picture" => Some(&["kind", "id", "title", "source_id", "tap_action", "refresh", "alert"]),
```

**(d)** The `Deserialize` mapping arm (`config.rs:1409-1424`), mirroring the `Plugin` arm.

Then extend every accessor: `id()` (`:1437`), `alert()` (`:1449`), `refresh()` (`:1461`),
`template()` (`:1480` — returns `None`, same as `Plugin`), `tap_action()` (`:1492`), and
`initial_fields()` (`:1844` — join the `Self::Weather { title, .. } | Self::Plugin { title, .. }`
pattern, since a picture card's title is a field like theirs).

`wire_config` (`:1763`) needs **no change**: its `Some(DisplayTemplate::DigitalClock) | None`
arm already maps a `template()` of `None` to `TemplateKind::DigitalClock`.

- [ ] **Step 9: Add validation**

In `CardSettings::validate` (`config.rs:1496`), add a `Picture` arm modelled on the
`Plugin` arm at `:1717-1746`:

```rust
            Self::Picture {
                title,
                source_id,
                tap_action,
                refresh,
                ..
            } => {
                validate_text(path, "title", title, MAX_WIDGET_TITLE_LEN, issues);
                validate_identifier(
                    &format!("{path}.source_id"),
                    source_id,
                    MAX_WIDGET_ID_LEN,
                    issues,
                );
                validate_composition(path, ProviderKind::Picture, None, tap_action, *refresh, issues);
            }
```

Add `Picture` to `ProviderKind` (`:2140-2149`). In `validate_composition`, give it the
same answers as `Plugin`: the template arm returns `false` (`:2224`), and the refresh arm
allows `Manual | Interval` (`:2268-2275`).

In `AppConfig::validate` (`:523`), after the existing per-card loop, add the document-level
checks — bounds and uniqueness for the sources, then the reference check:

```rust
        validate_collection_bounds(
            "image_sources",
            self.image_sources.len(),
            MAX_IMAGE_SOURCES,
            issues,
        );
        let mut seen_source_ids: BTreeSet<&str> = BTreeSet::new();
        for (index, source) in self.image_sources.iter().enumerate() {
            let path = format!("image_sources[{index}]");
            validate_identifier(&format!("{path}.id"), &source.id, MAX_WIDGET_ID_LEN, issues);
            validate_text(&path, "name", &source.name, MAX_IMAGE_SOURCE_NAME_LEN, issues);
            if !seen_source_ids.insert(source.id.as_str()) {
                issues.push(ValidationIssue::new(
                    format!("{path}.id"),
                    ValidationCode::DuplicateId,
                    format!("image source {:?} is declared more than once", source.id),
                ));
            }
        }
        for (index, card) in self.cards.iter().enumerate() {
            if let CardSettings::Picture { source_id, .. } = card {
                if !seen_source_ids.contains(source_id.as_str()) {
                    issues.push(ValidationIssue::new(
                        format!("cards[{index}].source_id"),
                        ValidationCode::MissingReference,
                        format!("image source {source_id:?} does not exist"),
                    ));
                }
            }
        }
```

Match `validate_collection_bounds`'s real signature (`:2059`) before using it — read it
rather than trusting the argument order above.

- [ ] **Step 10: Run the config tests**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/picture-cards/companion
cargo test -p app-core
```

Expected: PASS. Other crates will still fail to compile because their `CardSettings`
matches are not exhaustive yet — that is expected and is Tasks 7-9's work. Do not
"fix" them by adding `_ => {}` arms; an exhaustive match is how this codebase forces
a new card kind to be handled everywhere.

- [ ] **Step 11: Mutation probes**

Each must turn a test red; restore after each.
1. Delete `#[serde(default)]` from `image_sources` — `a_v6_document_migrates_to_v7…` fails.
2. Change the `MissingReference` push to a silent `continue` — `a_picture_card_naming_an_unknown_source…` fails.
3. Change `MAX_IMAGE_SOURCES` to `9` — `more_than_the_maximum_image_sources_is_rejected` fails.

- [ ] **Step 12: Commit**

```bash
git add companion/crates/app-core/src/config.rs \
        companion/crates/app-core/src/store.rs \
        companion/crates/app-core/tests/config.rs \
        companion/crates/app-core/tests/store.rs
git commit -m "feat: schema v7 — image sources and the picture card"
```

---
### Task 3: PNG ingest — header first, then decode, then canonicalize

Pure, no I/O, no state. Depends only on things already in the tree, so it can be built
concurrently with Task 2.

**Files:**
- Create: `companion/crates/server/src/image_ingest.rs`
- Modify: `companion/crates/server/src/lib.rs` (add `mod image_ingest;`)
- Modify: `companion/crates/server/src/rasterizer.rs` (widen two items' visibility)
- Modify: `companion/crates/server/Cargo.toml` (add `png`)
- Test: inline `#[cfg(test)]` module in `image_ingest.rs`

**Interfaces:**
- Produces:
  ```rust
  pub(crate) struct CanonicalFrame { pub digest: [u8; 32], pub bytes: Vec<u8> }
  pub(crate) enum ImageIngestError { NotPng, WrongDimensions { width: u32, height: u32 }, UnsupportedBitDepth, UnsupportedColorType, Decode }
  pub(crate) fn canonical_frame_from_png(bytes: &[u8]) -> Result<CanonicalFrame, ImageIngestError>;
  ```
- Consumed by: Task 6 (the ingest route).

**Why a direct `png` dependency rather than `Pixmap::decode_png`.** `resvg::tiny_skia::Pixmap::decode_png`
is already reachable from `server` and would need no new dependency — but it decodes in one
shot, which hands a decompression bomb exactly the allocation it is asking for. `png`'s
`Decoder::read_header_info()` reads only the IHDR and returns width, height, bit depth and
colour type **without allocating a pixel buffer**. That is the primitive the spec's
"reject before decode" rule requires, and it is the only reason this task takes a dependency.
`png` 0.17.16 is already in the lockfile (`server -> resvg =0.45.1 -> tiny-skia 0.11.4 -> png`),
so declaring it directly pins no new vendor and moves no lockfile version.

Note for reviewers: `server`'s `resvg` is deliberately `default-features = false,
features = ["text"]` so *usvg* cannot load raster images. This task does not change that.
Decoding happens in the ingest module, under an explicit dimension bound, never inside the
SVG renderer.

- [ ] **Step 1: Add the dependency**

In `companion/crates/server/Cargo.toml`, under `[dependencies]`:

```toml
# Header-first PNG decoding for the image-ingest route. `Pixmap::decode_png` would
# need no new line here but decodes in one shot; `png::Decoder::read_header_info`
# reads the IHDR alone, which is what lets a decompression bomb be rejected before
# anything is allocated. Same version already in the lockfile via tiny-skia.
png = "0.17"
```

- [ ] **Step 2: Widen the two rasterizer items this reuses**

In `companion/crates/server/src/rasterizer.rs`, change `fn encode_rgb565` (`:1145`) and
`const LVGL_IMAGE_HEADER_BYTES` (`:20`) to `pub(crate)`. There must be exactly one
implementation of the canonical format; this task reuses it rather than restating it.

`encode_rgb565` takes a `&Pixmap` and a `&RenderDeadline`. Make the deadline optional for
this caller by adding a `pub(crate) fn encode_rgb565_unbounded(width, height, pixmap) -> Vec<u8>`
that calls it with an already-satisfied deadline, or widen `RenderDeadline`'s constructor —
whichever the existing code makes cleaner. Ingest has its own bound (the exact 448x368
dimension check), so a render deadline adds nothing here.

- [ ] **Step 3: Write the failing tests**

Create `companion/crates/server/src/image_ingest.rs` with only the test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a real PNG of the given size and colour type.
    fn png_of(width: u32, height: u32, color: png::ColorType, depth: png::BitDepth) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, width, height);
            encoder.set_color(color);
            encoder.set_depth(depth);
            let mut writer = encoder.write_header().expect("header");
            let samples = match color {
                png::ColorType::Rgb => 3,
                png::ColorType::Rgba => 4,
                _ => unreachable!("tests use rgb or rgba only"),
            };
            let bytes_per_sample = if depth == png::BitDepth::Sixteen { 2 } else { 1 };
            let data =
                vec![0u8; (width * height) as usize * samples * bytes_per_sample];
            writer.write_image_data(&data).expect("data");
        }
        out
    }

    const W: u32 = 448;
    const H: u32 = 368;

    #[test]
    fn an_exact_448x368_rgb_png_is_accepted() {
        let frame = canonical_frame_from_png(&png_of(W, H, png::ColorType::Rgb, png::BitDepth::Eight))
            .expect("accepted");
        // 12-byte LVGL header + one LE u16 per pixel.
        assert_eq!(frame.bytes.len(), 12 + (W * H * 2) as usize);
    }

    #[test]
    fn an_rgba_png_is_accepted_and_composited_over_black() {
        // A fully transparent RGBA frame composites to black, not to white and
        // not to the decoder's uninitialised buffer.
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, W, H);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("header");
            // Opaque white pixels at zero alpha.
            let data: Vec<u8> = std::iter::repeat([255u8, 255, 255, 0])
                .take((W * H) as usize)
                .flatten()
                .collect();
            writer.write_image_data(&data).expect("data");
        }
        let frame = canonical_frame_from_png(&out).expect("accepted");
        let first_pixel = u16::from_le_bytes([frame.bytes[12], frame.bytes[13]]);
        assert_eq!(first_pixel, 0, "alpha zero must composite to black");
    }

    #[test]
    fn one_pixel_narrow_is_rejected() {
        let error = canonical_frame_from_png(&png_of(447, H, png::ColorType::Rgb, png::BitDepth::Eight))
            .expect_err("447 wide");
        assert!(matches!(error, ImageIngestError::WrongDimensions { width: 447, height: 368 }));
    }

    #[test]
    fn one_pixel_wide_is_rejected() {
        let error = canonical_frame_from_png(&png_of(449, H, png::ColorType::Rgb, png::BitDepth::Eight))
            .expect_err("449 wide");
        assert!(matches!(error, ImageIngestError::WrongDimensions { width: 449, .. }));
    }

    #[test]
    fn sixteen_bit_is_rejected() {
        let error =
            canonical_frame_from_png(&png_of(W, H, png::ColorType::Rgb, png::BitDepth::Sixteen))
                .expect_err("16-bit");
        assert!(matches!(error, ImageIngestError::UnsupportedBitDepth));
    }

    #[test]
    fn a_body_that_is_not_a_png_is_rejected() {
        assert!(matches!(
            canonical_frame_from_png(b"not a png at all").expect_err("garbage"),
            ImageIngestError::NotPng
        ));
    }

    #[test]
    fn a_decompression_bomb_is_rejected_without_allocating_its_pixels() {
        // A valid IHDR declaring an enormous canvas, with almost no body. If the
        // dimension check ran after `next_frame`, this would try to allocate
        // ~48 GB. It must be refused from the header alone.
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, 100_000, 100_000);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            // Writing the header alone is exactly the hostile shape: a declared
            // size with no matching data.
            let _writer = encoder.write_header().expect("header");
        }
        let error = canonical_frame_from_png(&out).expect_err("bomb");
        assert!(
            matches!(error, ImageIngestError::WrongDimensions { width: 100_000, height: 100_000 }),
            "must fail on dimensions, not on a truncated decode: {error:?}"
        );
    }

    #[test]
    fn the_canonical_header_and_digest_are_pinned() {
        // An endianness or header slip must not pass silently.
        let frame = canonical_frame_from_png(&png_of(W, H, png::ColorType::Rgb, png::BitDepth::Eight))
            .expect("accepted");
        // word0 = magic 0x19 | (RGB565 0x12 << 8); word1 = w | h << 16; word2 = stride.
        assert_eq!(&frame.bytes[0..4], &0x0000_1219u32.to_le_bytes());
        assert_eq!(&frame.bytes[4..8], &(W | (H << 16)).to_le_bytes());
        assert_eq!(&frame.bytes[8..12], &(W * 2).to_le_bytes());
        // An all-black frame's digest is a fixed value. Capture it on first run
        // from the failure message and paste it here; do not compute it in the test.
        assert_eq!(
            protocol::digest_hex(&frame.digest),
            "PASTE_THE_OBSERVED_DIGEST_HERE"
        );
    }

    #[test]
    fn the_same_picture_twice_yields_the_same_digest() {
        let png = png_of(W, H, png::ColorType::Rgb, png::BitDepth::Eight);
        let first = canonical_frame_from_png(&png).expect("first");
        let second = canonical_frame_from_png(&png).expect("second");
        assert_eq!(first.digest, second.digest);
    }
}
```

`png::Encoder` is available because `png` is now a direct dependency; it needs no extra
feature.

- [ ] **Step 4: Run and watch them fail**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/picture-cards/companion
cargo test -p server image_ingest
```

Expected: FAIL to compile — `canonical_frame_from_png` does not exist.

- [ ] **Step 5: Implement it**

Prepend to `companion/crates/server/src/image_ingest.rs`:

```rust
//! Turning a producer's PNG into the canonical frame the device already draws.
//!
//! The order of the checks below is the security-relevant part. The 1 MiB body
//! limit bounds the *compressed* bytes; it says nothing about what they expand
//! to. What actually stops a decompression bomb is reading the IHDR alone and
//! refusing on dimensions **before** any pixel buffer is allocated, which is
//! why this uses `read_header_info` rather than a one-shot decode.

use protocol::{SCENE_CANVAS_HEIGHT, SCENE_CANVAS_WIDTH};
use sha2::{Digest, Sha256};

use crate::rasterizer;

/// One accepted picture, in the exact form the asset store and the wire use.
///
/// `digest` hashes the **decoded** blob, never the PNG — stage 4's rule. Two
/// different PNG encodings of the same picture are therefore the same asset,
/// and the digest survives a `png` crate upgrade.
#[derive(Debug, Clone)]
pub(crate) struct CanonicalFrame {
    pub digest: [u8; 32],
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ImageIngestError {
    NotPng,
    WrongDimensions { width: u32, height: u32 },
    UnsupportedBitDepth,
    UnsupportedColorType,
    Decode,
}

impl std::fmt::Display for ImageIngestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Written for the producer: one line, no internals.
        match self {
            Self::NotPng => write!(formatter, "the body is not a PNG"),
            Self::WrongDimensions { width, height } => write!(
                formatter,
                "the picture is {width}x{height}; it must be exactly {SCENE_CANVAS_WIDTH}x{SCENE_CANVAS_HEIGHT}"
            ),
            Self::UnsupportedBitDepth => {
                write!(formatter, "the picture must be 8 bits per channel")
            }
            Self::UnsupportedColorType => {
                write!(formatter, "the picture must be RGB or RGBA")
            }
            Self::Decode => write!(formatter, "the PNG could not be decoded"),
        }
    }
}

pub(crate) fn canonical_frame_from_png(bytes: &[u8]) -> Result<CanonicalFrame, ImageIngestError> {
    let width = u32::try_from(SCENE_CANVAS_WIDTH).expect("fixed canvas");
    let height = u32::try_from(SCENE_CANVAS_HEIGHT).expect("fixed canvas");

    let mut decoder = png::Decoder::new(bytes);
    // Belt and braces beside the dimension check below: even a header this
    // accepts can never ask for more than one canvas worth of pixels.
    decoder.set_limits(png::Limits {
        bytes: (width as usize) * (height as usize) * 4 + 64 * 1_024,
    });

    // Header only. Nothing is allocated for pixels yet.
    let info = decoder
        .read_header_info()
        .map_err(|_| ImageIngestError::NotPng)?;
    let (declared_width, declared_height, depth, color) =
        (info.width, info.height, info.bit_depth, info.color_type);

    if declared_width != width || declared_height != height {
        return Err(ImageIngestError::WrongDimensions {
            width: declared_width,
            height: declared_height,
        });
    }
    if depth != png::BitDepth::Eight {
        return Err(ImageIngestError::UnsupportedBitDepth);
    }
    let samples = match color {
        png::ColorType::Rgb => 3usize,
        png::ColorType::Rgba => 4usize,
        _ => return Err(ImageIngestError::UnsupportedColorType),
    };

    // Only now does anything get allocated.
    let mut reader = decoder.read_info().map_err(|_| ImageIngestError::Decode)?;
    let mut buffer = vec![0u8; reader.output_buffer_size()];
    let frame = reader
        .next_frame(&mut buffer)
        .map_err(|_| ImageIngestError::Decode)?;
    let pixels = &buffer[..frame.buffer_size()];

    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for chunk in pixels.chunks_exact(samples) {
        // The panel has no alpha. Composite over black, matching the
        // rasterizer's `pixmap.fill(Color::BLACK)`, so a transparent producer
        // pixel lands on the same ground a transparent SVG pixel does.
        let alpha = if samples == 4 { u32::from(chunk[3]) } else { 255 };
        let over_black = |channel: u8| -> u8 {
            u8::try_from(u32::from(channel) * alpha / 255).expect("scaled by <= 1")
        };
        rgba.extend_from_slice(&[
            over_black(chunk[0]),
            over_black(chunk[1]),
            over_black(chunk[2]),
            255,
        ]);
    }

    let size = resvg::tiny_skia::IntSize::from_wh(width, height).expect("fixed canvas");
    let pixmap = resvg::tiny_skia::Pixmap::from_vec(rgba, size).ok_or(ImageIngestError::Decode)?;
    let bytes = rasterizer::encode_rgb565_unbounded(width, height, &pixmap);
    let digest = Sha256::digest(&bytes).into();

    Ok(CanonicalFrame { digest, bytes })
}
```

Register the module in `companion/crates/server/src/lib.rs` beside the others:
`mod image_ingest;`.

- [ ] **Step 6: Run, capture the pinned digest, run again**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/picture-cards/companion
cargo test -p server image_ingest
```

`the_canonical_header_and_digest_are_pinned` fails once, printing the observed digest.
Paste that hex into the assertion, re-run, and confirm everything passes.

- [ ] **Step 7: Mutation probes**

Each must turn a test red; restore after each.
1. Move the dimension check to after `next_frame` — `a_decompression_bomb_is_rejected_without_allocating_its_pixels` fails (or the test process dies on allocation, which is also a failure).
2. Digest `bytes` (the PNG) instead of the canonical blob — `the_canonical_header_and_digest_are_pinned` fails.
3. Drop the alpha multiply so RGBA is copied verbatim — `an_rgba_png_is_accepted_and_composited_over_black` fails.

- [ ] **Step 8: Commit**

```bash
git add companion/crates/server/src/image_ingest.rs \
        companion/crates/server/src/lib.rs \
        companion/crates/server/src/rasterizer.rs \
        companion/crates/server/Cargo.toml \
        companion/Cargo.lock
git commit -m "feat: canonicalize a producer's PNG, header checked before decode"
```

---

### Task 5: Inferred staleness

Pure arithmetic over a ring of wall-clock instants. No I/O, no state, no dependency on
Tasks 2-4 — build it concurrently with any of them.

**Files:**
- Create: `companion/crates/server/src/image_staleness.rs`
- Modify: `companion/crates/server/src/lib.rs` (add `mod image_staleness;`)
- Test: inline `#[cfg(test)]` module

**Interfaces:**
- Produces:
  ```rust
  pub(crate) const PUSH_TIME_RING: usize = 8;
  pub(crate) const STALE_MULTIPLE: u32 = 3;
  pub(crate) const STALE_FLOOR: Duration = Duration::from_secs(15 * 60);
  pub(crate) const STALE_CEILING: Duration = Duration::from_secs(48 * 60 * 60);
  pub(crate) fn is_stale(recent_pushes: &[DateTime<Utc>], now: DateTime<Utc>) -> bool;
  pub(crate) fn stale_deadline(recent_pushes: &[DateTime<Utc>]) -> Option<Duration>;
  ```
- Consumed by: Task 4 (the store calls it), Task 6, Task 8.

- [ ] **Step 1: Write the failing tests**

Create `companion/crates/server/src/image_staleness.rs` with the test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + seconds, 0).unwrap()
    }

    /// Pushes every `interval` seconds, oldest first, `count` of them.
    fn cadence(count: usize, interval: i64) -> Vec<DateTime<Utc>> {
        (0..count).map(|index| at(index as i64 * interval)).collect()
    }

    #[test]
    fn fewer_than_three_intervals_is_never_stale() {
        // Three pushes give two intervals. A cadence cannot be inferred yet, so
        // no amount of silence flags it.
        let ring = cadence(3, 60);
        let last = *ring.last().unwrap();
        assert!(!is_stale(&ring, last + chrono::Duration::days(30)));
        assert_eq!(stale_deadline(&ring), None);
    }

    #[test]
    fn four_pushes_are_enough_to_infer_a_cadence() {
        let ring = cadence(4, 60);
        assert!(stale_deadline(&ring).is_some());
    }

    #[test]
    fn a_fast_producer_gets_the_floor_not_three_minutes() {
        // One-minute cadence: 3 x 60s = 180s, which the floor lifts to 15 min.
        let ring = cadence(8, 60);
        assert_eq!(stale_deadline(&ring), Some(STALE_FLOOR));
        let last = *ring.last().unwrap();
        assert!(!is_stale(&ring, last + chrono::Duration::minutes(14)));
        assert!(is_stale(&ring, last + chrono::Duration::minutes(16)));
    }

    #[test]
    fn a_daily_producer_gets_the_ceiling_not_three_days() {
        let ring = cadence(8, 24 * 60 * 60);
        assert_eq!(stale_deadline(&ring), Some(STALE_CEILING));
        let last = *ring.last().unwrap();
        assert!(!is_stale(&ring, last + chrono::Duration::hours(47)));
        assert!(is_stale(&ring, last + chrono::Duration::hours(49)));
    }

    #[test]
    fn a_ten_minute_producer_gets_three_times_its_baseline() {
        // 10 min baseline -> 30 min, inside both bounds, so neither clamp applies.
        let ring = cadence(8, 10 * 60);
        assert_eq!(stale_deadline(&ring), Some(Duration::from_secs(30 * 60)));
    }

    #[test]
    fn the_median_resists_one_late_push() {
        // Seven ~10-minute gaps and one enormous one. A mean would inflate the
        // deadline permanently; the median must not move much.
        let mut ring = cadence(8, 10 * 60);
        let last = *ring.last().unwrap();
        ring.push(last + chrono::Duration::hours(6));
        let deadline = stale_deadline(&ring).expect("enough samples");
        assert_eq!(deadline, Duration::from_secs(30 * 60));
    }

    #[test]
    fn any_push_clears_stale_immediately() {
        let ring = cadence(8, 10 * 60);
        let last = *ring.last().unwrap();
        let long_after = last + chrono::Duration::hours(5);
        assert!(is_stale(&ring, long_after), "silent for five hours");

        let mut recovered = ring.clone();
        recovered.push(long_after);
        assert!(!is_stale(&recovered, long_after), "a push clears it at once");
    }

    #[test]
    fn an_empty_ring_is_never_stale() {
        assert!(!is_stale(&[], at(0)));
        assert_eq!(stale_deadline(&[]), None);
    }
}
```

- [ ] **Step 2: Run and watch them fail**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/picture-cards/companion
cargo test -p server image_staleness
```

Expected: FAIL to compile.

- [ ] **Step 3: Implement it**

Prepend to `companion/crates/server/src/image_staleness.rs`:

```rust
//! Inferring whether a picture source has gone quiet, from its own observed
//! cadence rather than from a declared interval.
//!
//! A declared interval is friction every external producer pays forever, and
//! half of them would declare it wrong. What makes inference safe rather than
//! guesswork is the clamp: a per-minute producer is not flagged after three
//! minutes of silence, and a daily one is not given three days of rope.

use std::time::Duration;

use chrono::{DateTime, Utc};

/// How many accepted push times one source remembers.
pub(crate) const PUSH_TIME_RING: usize = 8;

/// How many baseline intervals of silence mean "gone quiet".
pub(crate) const STALE_MULTIPLE: u32 = 3;

/// No source is flagged sooner than this, however fast it normally pushes.
pub(crate) const STALE_FLOOR: Duration = Duration::from_secs(15 * 60);

/// No source is given more rope than this, however slowly it normally pushes.
pub(crate) const STALE_CEILING: Duration = Duration::from_secs(48 * 60 * 60);

/// The smallest number of *intervals* an inference needs. Three intervals means
/// four pushes: one gap is not a cadence, and two cannot outvote an outlier.
const MIN_INTERVALS: usize = 3;

/// How long this source may stay silent before it counts as stale, or `None`
/// when too few pushes have been seen to infer anything.
pub(crate) fn stale_deadline(recent_pushes: &[DateTime<Utc>]) -> Option<Duration> {
    if recent_pushes.len() < MIN_INTERVALS + 1 {
        return None;
    }
    let mut intervals: Vec<i64> = recent_pushes
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).num_seconds().max(0))
        .collect();
    intervals.sort_unstable();
    // Median, not mean: one late push must not inflate the deadline for good.
    let middle = intervals.len() / 2;
    let baseline = if intervals.len() % 2 == 0 {
        (intervals[middle - 1] + intervals[middle]) / 2
    } else {
        intervals[middle]
    };
    let baseline = u64::try_from(baseline).unwrap_or(0);
    let deadline = Duration::from_secs(baseline.saturating_mul(u64::from(STALE_MULTIPLE)));
    Some(deadline.clamp(STALE_FLOOR, STALE_CEILING))
}

/// Whether this source has been silent past its inferred deadline.
pub(crate) fn is_stale(recent_pushes: &[DateTime<Utc>], now: DateTime<Utc>) -> bool {
    let Some(deadline) = stale_deadline(recent_pushes) else {
        return false;
    };
    let Some(last) = recent_pushes.last() else {
        return false;
    };
    let silence = (now - *last).num_seconds();
    if silence <= 0 {
        return false;
    }
    u64::try_from(silence).unwrap_or(0) > deadline.as_secs()
}
```

Register it in `companion/crates/server/src/lib.rs`: `mod image_staleness;`.

- [ ] **Step 4: Run them**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/picture-cards/companion
cargo test -p server image_staleness
```

Expected: all PASS.

- [ ] **Step 5: Mutation probes**

Each must turn a test red; restore after each.
1. Change `MIN_INTERVALS + 1` to `2` — `fewer_than_three_intervals_is_never_stale` fails.
2. Replace the median with a mean — `the_median_resists_one_late_push` fails.
3. Delete the `.clamp(...)` — both `a_fast_producer_gets_the_floor…` and `a_daily_producer_gets_the_ceiling…` fail.

- [ ] **Step 6: Commit**

```bash
git add companion/crates/server/src/image_staleness.rs companion/crates/server/src/lib.rs
git commit -m "feat: infer a picture source's staleness from its own cadence"
```

---
### Task 4: The image-source store — tokens, frames, and the push-time ring

Needs Task 1 (`write_and_replace_binary`) and Task 5 (`is_stale`). Independent of Tasks 2
and 3.

**Files:**
- Create: `companion/crates/server/src/image_sources.rs`
- Modify: `companion/crates/server/src/lib.rs` (`mod image_sources;`, plus a field on `ServerState`)
- Test: `companion/crates/server/tests/image_sources.rs`

**Interfaces:**
- Produces:
  ```rust
  pub(crate) struct ImageSourceStore { /* root: PathBuf, state: Mutex<..> */ }
  pub(crate) struct MintedSource { pub id: String, pub token: String }   // token: plaintext, once
  pub(crate) struct SourceFrame { pub digest: [u8; 32], pub bytes: Arc<[u8]>, pub stale: bool }
  pub(crate) enum ImageSourceError { UnknownToken, Capacity, Io { message: String }, TooSoon }

  impl ImageSourceStore {
      pub(crate) fn new(root: PathBuf) -> Result<Self, ImageSourceError>;
      pub(crate) fn mint(&self, name: &str) -> Result<MintedSource, ImageSourceError>;
      pub(crate) fn revoke(&self, id: &str) -> Result<(), ImageSourceError>;
      pub(crate) fn authenticate(&self, token: &str) -> Option<String>;      // -> source id
      pub(crate) fn accept(&self, id: &str, frame: CanonicalFrame, now: DateTime<Utc>)
          -> Result<AcceptOutcome, ImageSourceError>;
      pub(crate) fn frame(&self, id: &str, now: DateTime<Utc>) -> Option<SourceFrame>;
      pub(crate) fn all_frames(&self, now: DateTime<Utc>) -> Vec<(String, SourceFrame)>;
  }
  pub(crate) enum AcceptOutcome { Changed { digest: [u8; 32] }, Unchanged }
  ```
- Consumed by: Task 6 (routes), Task 8 (the union).

**Design notes an implementer must not re-decide.**

- **Digests only.** `mint` generates 32 random bytes rendered as 64 lowercase hex — copy
  `registry.rs:558`'s `random_token()` — stores `Sha256` of it, returns the plaintext once,
  and never writes the plaintext anywhere. `authenticate` compares with
  `registry::constant_time_eq` (already `pub(crate)`), scanning the whole table on a miss.
- **Wall clock, not `Instant`.** The ring is persisted and read back after a restart; a
  monotonic clock does not survive one. `chrono::DateTime<Utc>` throughout, and `now` is a
  parameter so the tests are deterministic.
- **The frame file is binary.** Use `secure_file::write_and_replace_binary` from Task 1.
  Using `write_and_replace` appends a newline and silently corrupts the blob.
- **An identical picture is a no-op that still counts as liveness.** If the incoming digest
  equals the stored one, record the push time and return `AcceptOutcome::Unchanged`. A
  producer on a timer sending the same picture is still alive.
- **`MIN_PUSH_INTERVAL` is enforced here**, not in the route, so it is covered by unit
  tests rather than only by an HTTP test. Define `pub(crate) const MIN_PUSH_INTERVAL: Duration = Duration::from_secs(5);`
  and return `ImageSourceError::TooSoon` when the last accepted push is newer than that.
- **Capacity** is `app_core::config::MAX_IMAGE_SOURCES`; `mint` past it is
  `ImageSourceError::Capacity`.

**Persistence layout**, under the server's data root (a sibling of `device-identities.json`):

```
image-sources.json        { schema_version: 1, sources: [ { id, name, token_sha256, recent_push_times: [rfc3339, ...] } ] }
image-frames/<id>.bin     the decoded canonical blob, 329,740 bytes, mode 0600
```

Metadata goes through `secure_file::write_and_replace` (text, newline fine); frames go
through `write_and_replace_binary`. Follow `registry.rs`'s `PersistedRegistry` shape:
`#[serde(deny_unknown_fields)]`, an explicit `schema_version`, and a bounded read via
`secure_file::read_bounded`.

- [ ] **Step 1: Write the failing tests**

Create `companion/crates/server/tests/image_sources.rs`. These are the spec's §10 auth,
idempotence and persistence rows:

```rust
// Test names state the behaviour, not the function under test.

#[test]
fn a_minted_token_authenticates_and_a_wrong_one_does_not() { /* mint; authenticate(token) == Some(id); authenticate("00".repeat(32)) == None */ }

#[test]
fn a_revoked_token_stops_authenticating() { /* mint; revoke(id); authenticate(token) == None */ }

#[test]
fn the_plaintext_token_never_appears_in_the_store_file() {
    // Mirrors the existing device-identity test. Read image-sources.json as a
    // string and assert it does not contain the plaintext.
}

#[test]
fn a_token_still_authenticates_after_the_store_is_reloaded() {
    // Mint against one store, drop it, open a second on the same root.
}

#[test]
fn the_same_picture_twice_is_accepted_once_but_counted_twice() {
    // accept(frame) -> Changed; accept(same frame) -> Unchanged; the ring has
    // two entries, because a producer sending the same picture is still alive.
}

#[test]
fn a_second_push_inside_the_minimum_interval_is_refused() {
    // accept at t; accept at t + 4s -> TooSoon; accept at t + 6s -> Ok.
}

#[test]
fn minting_past_the_maximum_is_refused() { /* MAX_IMAGE_SOURCES mints, then Capacity */ }

#[test]
fn the_ring_keeps_only_the_most_recent_pushes() {
    // Push PUSH_TIME_RING + 3 times; assert the ring length is PUSH_TIME_RING
    // and its last entry is the newest.
}

#[test]
fn a_frame_survives_a_restart_byte_for_byte() {
    // The newline trap: write a frame, reopen the store, and assert the bytes
    // read back are byte-identical and the digest still matches.
}

#[test]
fn a_source_that_has_gone_quiet_reports_stale() {
    // Four pushes ten minutes apart, then ask at +5h: SourceFrame.stale is true.
}

#[test]
fn revoking_a_source_drops_its_frame_from_the_desired_set() {
    // all_frames() no longer names it, and its file is gone.
}
```

Fill each body against the interface above. Build a `CanonicalFrame` by calling Task 3's
`canonical_frame_from_png` on a generated PNG, or construct one directly if the test
crate cannot reach a `pub(crate)` item — in that case make these unit tests inside
`image_sources.rs` instead of an integration test file. **Decide that before writing
them**, because it determines the file.

- [ ] **Step 2: Run them and watch them fail**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/picture-cards/companion
cargo test -p server image_sources
```

Expected: FAIL to compile.

- [ ] **Step 3: Implement the store**

Write `companion/crates/server/src/image_sources.rs` to satisfy the interface. Reuse, do
not reimplement: `registry::constant_time_eq`, `registry.rs:558`'s `random_token()` shape
(`rand::rng().fill_bytes` + `protocol::digest_hex`), `app_core::secure_file` for all I/O,
and `image_staleness::{is_stale, PUSH_TIME_RING}`.

`frame()` reads the blob from disk (or a cached `Arc<[u8]>`) and stamps `stale` by calling
`is_stale(&record.recent_push_times, now)`. Keep the bytes as `Arc<[u8]>` so Task 8 can
hand them to `DesiredAsset` by moving the `Arc`, never re-reading or re-hashing — the
byte-provenance rule `ServerPluginHost::desired_assets` already follows.

Add the store to `ServerState` beside the registry, constructed from the same data root,
and expose it as `state.image_sources()`.

- [ ] **Step 4: Run them**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/picture-cards/companion
cargo test -p server image_sources
```

Expected: all PASS.

- [ ] **Step 5: Mutation probes**

1. Swap `constant_time_eq` for `==` — no test fails. **That is expected and is the point:**
   add a comment recording that this guard is not test-provable and is kept by review, exactly
   as `registry.rs` documents its own. Do not delete it.
2. Use `write_and_replace` instead of `write_and_replace_binary` — `a_frame_survives_a_restart_byte_for_byte` fails.
3. Skip recording the push time on an unchanged picture — `the_same_picture_twice_is_accepted_once_but_counted_twice` fails.
4. Drop the `MIN_PUSH_INTERVAL` check — `a_second_push_inside_the_minimum_interval_is_refused` fails.

- [ ] **Step 6: Commit**

```bash
git add companion/crates/server/src/image_sources.rs companion/crates/server/src/lib.rs \
        companion/crates/server/tests/image_sources.rs
git commit -m "feat: image-source store — digest auth, durable frames, cadence ring"
```

---

### Task 6: The three routes

Needs Tasks 2, 3, 4, 5.

**Files:**
- Create: `companion/crates/server/src/images.rs`
- Modify: `companion/crates/server/src/lib.rs` (merge the router)
- Test: `companion/crates/server/tests/image_routes.rs`

**Interfaces:**
- Produces: `pub(crate) fn routes() -> Router<ServerState>` covering
  - `POST /v1/images/{token}` — producer-facing, `image/png` body, `DefaultBodyLimit::max(1 MiB)`
  - `POST /v1/images` — admin-bearer, mints a source, returns the plaintext **once**
  - `DELETE /v1/images/{id}` — admin-bearer, revokes and drops the frame

**Patterns to copy exactly** (from `admin.rs`):
- Per-route body limit inside the `.route(...)` call, as `put_config` does at `admin.rs:37`.
- `AdminAuthenticated` as the handler's **second** parameter for the two admin routes.
- An error enum with `IntoResponse` and a `#[serde(tag = "kind", rename_all = "kebab-case")]` body.
- CPU work — the decode — inside `tokio::task::spawn_blocking`, with `JoinError` mapping
  to a 500, as every other handler does.

**Two things about the ingest route that are new to this codebase.**
1. It is the **first binary-body route**. Use `axum::body::Bytes` as the extractor. Reject a
   `Content-Type` that is not `image/png` with 415 before touching the body.
2. The token is in the path **and** accepted as `Authorization: Bearer`. One verification
   function, two carriers. The path form is an accepted risk recorded in the spec (§5): the
   token appears in Caddy and Cloudflare access logs, and its blast radius is bounded because
   it is write-only, scoped to one source, reads nothing, and is revoked independently.

**Status codes**, each with a test: 401 bad or revoked token, 413 too large, 415 not a PNG,
422 wrong dimensions or bit depth, 429 too fast, 200 accepted (changed or unchanged).

- [ ] **Step 1: Write the failing tests**

Create `companion/crates/server/tests/image_routes.rs`, following the existing server HTTP
test setup (read `companion/crates/server/tests/` for how a `ServerState` and a test client
are built — do not invent a new harness):

```rust
#[tokio::test]
async fn an_exact_png_is_accepted() { /* 200 */ }

#[tokio::test]
async fn an_unknown_token_is_unauthorized() { /* 401, and the body names no internals */ }

#[tokio::test]
async fn a_revoked_token_is_unauthorized() { /* mint, revoke, push -> 401 */ }

#[tokio::test]
async fn the_token_is_accepted_as_a_bearer_header_too() {
    // Same source, same digest, via Authorization instead of the path.
}

#[tokio::test]
async fn a_body_over_one_mebibyte_is_rejected() { /* 413 */ }

#[tokio::test]
async fn a_body_that_is_not_a_png_is_rejected() { /* 415 */ }

#[tokio::test]
async fn a_wrongly_sized_png_is_unprocessable() { /* 422, message names 448x368 */ }

#[tokio::test]
async fn a_second_push_inside_five_seconds_is_rate_limited() { /* 429 */ }

#[tokio::test]
async fn minting_returns_the_plaintext_exactly_once() {
    // POST /v1/images returns a token; GET-ing the source anywhere else never
    // returns it again.
}

#[tokio::test]
async fn the_producer_routes_need_no_admin_token_and_the_admin_routes_do() {
    // POST /v1/images without a bearer -> 401; POST /v1/images/{token} without
    // one -> 200.
}
```

- [ ] **Step 2: Run and watch them fail; Step 3: implement; Step 4: run them green**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/picture-cards/companion
cargo test -p server --test image_routes
```

**Note for whoever runs this:** Codex's sandbox cannot bind loopback sockets, so an agent
worker will report these as failing-to-run even when they are correct. The controller runs
this file.

- [ ] **Step 5: Mutation probes**

1. Move the `Content-Type` check after the decode — `a_body_that_is_not_a_png_is_rejected` should still fail for the right reason; if it starts returning 422 instead of 415, the check moved. 
2. Remove `DefaultBodyLimit` from the route — `a_body_over_one_mebibyte_is_rejected` fails (the global limit is larger).
3. Return the plaintext token from any route other than mint — `minting_returns_the_plaintext_exactly_once` fails.

- [ ] **Step 6: Commit**

```bash
git add companion/crates/server/src/images.rs companion/crates/server/src/lib.rs \
        companion/crates/server/tests/image_routes.rs
git commit -m "feat: the picture ingest webhook and its two admin routes"
```

---
### Task 7: The runtime — a shared frame-face helper, the `Picture` arm, and the command

Needs Task 2. Touches `runtime.rs` and `commands.rs`; do not run it concurrently with any
other task that holds `runtime.rs`.

**Files:**
- Modify: `companion/crates/app-core/src/runtime.rs`
- Modify: `companion/crates/app-core/src/commands.rs`
- Test: `companion/crates/app-core/tests/runtime.rs`

**Interfaces:**
- Produces:
  ```rust
  // runtime.rs — the one definition of "a frame as a face"
  fn frame_face_scene(revision: u32, digest: [u8; protocol::ASSET_DIGEST_LEN]) -> protocol::Scene;

  // runtime.rs — the host seam, with a default so no existing impl changes
  pub struct ImageSourceFrame {
      pub digest: [u8; protocol::ASSET_DIGEST_LEN],
      pub bytes: std::sync::Arc<[u8]>,   // the decoded canonical blob
      pub stale: bool,
  }
  trait PluginHost {
      fn image_source_frame(&mut self, _source_id: &str) -> Option<ImageSourceFrame> { None }
  }

  // commands.rs
  RuntimeCommand::ImageSourceUpdated { source_id: String, digest: [u8; 32], reply: CommandReply }

  // runtime.rs — public API on RuntimeHandle
  pub fn image_source_updated(&self, source_id: &str, digest: [u8; 32]) -> Result<(), RuntimeError>;
  ```
- Consumed by: Task 6 (the route calls `image_source_updated`), Task 8 (implements `image_source_frame`).

**Deliberate deviation from the spec, recorded here rather than slipped in.** The spec's
§6 command is `ImageSourceUpdated { source_id, digest, bytes, reply }`. This plan drops
`bytes`. The reason: the handler must reconcile the **whole** desired set, not just this
source — `AssetRelease` is a device-wide keep-set and a partial reconcile is precisely the
empty-registry wipe fixed during stage 3b. Since the full set comes from
`PluginHost::desired_assets()`, which already owns this frame's `Arc<[u8]>` by the time the
command is sent, a `bytes` field would be a second copy of the same truth that nothing
reads. `source_id` and `digest` are what the handler actually needs: the digest to compare,
the id to decide whether the visible card is affected. If a reviewer prefers the spec's
shape, restoring it is additive and breaks nothing.

- [ ] **Step 1: Write the failing tests**

Add to `companion/crates/app-core/tests/runtime.rs`. The existing test hosts in that file
(and the five in `runtime.rs`) show the shape; extend one with `image_source_frame`.

```rust
#[test]
fn a_picture_card_builds_a_single_full_canvas_image_scene() {
    // The spec's "no new hardware pixel rows" bargain: instead of a golden
    // frame, assert the scene built is exactly the one node it should be.
    // Build a config with an image source and a picture card, a host whose
    // image_source_frame returns a known digest and stale: false, then render.
    let scene = /* build_card_scene(...) -> CardCandidate::Push(push).scene */;
    assert_eq!(scene.nodes.len(), 1);
    let protocol::SceneNode::Image(image) = &scene.nodes[0] else {
        panic!("a picture card's face is one image node");
    };
    assert_eq!(image.x, 0);
    assert_eq!(image.y, 0);
    assert_eq!(image.w, protocol::SCENE_CANVAS_WIDTH);
    assert_eq!(image.h, protocol::SCENE_CANVAS_HEIGHT);
    assert_eq!(image.digest, KNOWN_DIGEST);
    assert!(!image.recolor);
}

#[test]
fn a_stale_picture_card_adds_the_shared_footer_and_nothing_else() {
    // stale: true -> two nodes, the image plus the same footer every other
    // face draws. The footer overlays the picture; that is accepted, because a
    // picture is full-bleed and there is no reserved strip.
    assert_eq!(scene.nodes.len(), 2);
    assert!(matches!(scene.nodes[1], protocol::SceneNode::Text(_)));
}

#[test]
fn a_picture_source_with_no_frame_yet_says_so_in_words() {
    // Not the "No data yet" badge: there is no frame at all. A state word.
    // host.image_source_frame returns None.
    let text = /* the only Text node's literal value */;
    assert_eq!(text, "Waiting for the first picture");
    assert!(!scene.nodes.iter().any(|node| matches!(node, protocol::SceneNode::Image(_))));
}

#[test]
fn a_picture_card_with_no_host_refuses_distinguishably() {
    // The Tauri app injects no host. The refusal must name that specifically,
    // and must not be confusable with "no frame pushed yet".
    let error = /* build_card_scene with plugin_host: None */.expect_err("no host");
    assert!(error.contains("no image source host"), "{error}");
}

#[test]
fn a_picture_card_negotiates_native_once_its_frame_is_installable() {
    // Never Rasterize, never RefuseLive: an image node carries no binding, so
    // bindings.live is empty, and the digest is durable and resident.
    let decision = render_negotiation::negotiate(&requirements, &profile);
    assert_eq!(decision, RenderDecision::Native);
}

#[test]
fn an_image_source_update_for_a_card_that_is_not_on_screen_pushes_no_scene() {
    // The frame becomes resident; the loop uses it when it next lands there.
    // Assert the device saw asset traffic but no PushScene.
}

#[test]
fn an_image_source_update_for_the_visible_card_pushes_a_scene_after_the_bytes() {
    // Ordering: every asset message precedes the PushScene that names the
    // digest. Assert on the exact recorded message sequence.
}

#[test]
fn a_failed_asset_transfer_pushes_no_scene_and_releases_nothing() {
    // Stage 4's rule: the previous picture stays on the panel.
}

#[test]
fn a_staleness_flip_on_the_visible_card_rebuilds_its_scene() {
    // A comparison per source on the tick, not a timer per source.
}
```

- [ ] **Step 2: Run and watch them fail**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/picture-cards/companion
cargo test -p app-core --test runtime
```

- [ ] **Step 3: Extract the shared frame-face helper**

`raster_frame_push` (`runtime.rs:3502-3530`) already builds exactly the scene a picture
card needs. Split it so there is one definition, not two that can drift:

```rust
/// The one definition of "a frame as a face": a single full-canvas image node
/// naming a digest the device holds. Both the raster path and a picture card
/// draw this, and they must never drift apart.
fn frame_face_scene(revision: u32, digest: [u8; protocol::ASSET_DIGEST_LEN]) -> protocol::Scene {
    protocol::Scene {
        revision,
        background: 0,
        nodes: vec![protocol::SceneNode::Image(protocol::SceneImage {
            x: 0,
            y: 0,
            w: protocol::SCENE_CANVAS_WIDTH,
            h: protocol::SCENE_CANVAS_HEIGHT,
            digest,
            recolor: false,
            color: 0,
        })],
    }
}

fn raster_frame_push(
    card_id: &str,
    revision: u32,
    digest: [u8; protocol::ASSET_DIGEST_LEN],
) -> Result<PushScene, String> {
    let push = PushScene {
        card_id: card_id.to_owned(),
        revision,
        scene: frame_face_scene(revision, digest),
    };
    validate_message(&Message::PushScene(push.clone()))
        .map_err(|error| format!("the raster frame scene is invalid: {error}"))?;
    Ok(push)
}
```

Run `cargo test -p app-core` now: this step alone must change no behaviour.

- [ ] **Step 4: Add the host seam**

In `runtime.rs`, beside `PluginHost` (`:174-203`):

```rust
/// What the host knows about one image source right now.
///
/// `bytes` is the decoded canonical blob, carried as an `Arc` so passing this
/// around costs a pointer rather than 330 KB. It is here because the admin
/// preview route needs the exact stored frame -- see Step 5b -- and re-deriving
/// it would mean rasterizing an image back into itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageSourceFrame {
    pub digest: [u8; protocol::ASSET_DIGEST_LEN],
    pub bytes: std::sync::Arc<[u8]>,
    /// Inferred from the source's own observed push cadence, server-side.
    pub stale: bool,
}
```

and, inside `trait PluginHost`:

```rust
    /// The frame this image source currently holds, or `None` when nothing has
    /// ever been pushed to it.
    ///
    /// A default of `None` is deliberate: every existing implementation --
    /// including the Tauri app's hostless runtime and the five test hosts --
    /// keeps compiling, and a host with no image sources answers honestly
    /// rather than being forced to pretend.
    ///
    /// This lives on `PluginHost` rather than a second injected trait because
    /// `desired_assets` must return the **union** of plugin assets and image
    /// frames from one function with one caller; splitting the seam would mean
    /// two desired sets to reconcile, and `AssetRelease` is device-wide.
    fn image_source_frame(&mut self, _source_id: &str) -> Option<ImageSourceFrame> {
        None
    }
```

- [ ] **Step 5: Add the `Picture` arm to `build_card_scene`**

`build_card_scene` (`:2766`) is currently `if let CardSettings::Plugin { .. } = card { … } else { … }`.
Convert it to a `match` so the compiler enforces the new kind:

```rust
    let scene = match card {
        CardSettings::Plugin { plugin_id, .. } => {
            // ... the existing plugin body, unchanged ...
        }
        CardSettings::Picture { source_id, .. } => {
            let host = plugin_host.ok_or_else(|| {
                format!("card {card_id:?} cannot render because no image source host is configured")
            })?;
            match host.image_source_frame(source_id) {
                Some(frame) => with_scene_data_state(
                    frame_face_scene(revision, frame.digest),
                    SceneDataState { stale: frame.stale, error: None },
                    metrics,
                ),
                // No frame at all, so there is nothing to badge. A state word,
                // the same distinction a plugin card awaiting its first refresh
                // already draws.
                None => waiting_for_first_picture_scene(revision, metrics),
            }
        }
        _ => {
            // ... the existing template body, unchanged ...
        }
    };
```

`waiting_for_first_picture_scene` is a small builder: one centred `SceneText` reading
`"Waiting for the first picture"` on the black ground. Model it on how
`build_template_card_scene` centres a caption, and validate it through the same tail
`build_card_scene` already runs.

Note the refusal string deliberately says **image source host**, not "plugin host", so the
two hostless refusals stay distinguishable in a log.

- [ ] **Step 5b: Let a picture card have a preview at all**

`render_card_preview` (`runtime.rs:3398`) opens by rejecting anything that is not a plugin
card:

```rust
    if !matches!(card, CardSettings::Plugin { .. }) {
        return Err(RuntimeError::NotAPluginCard { card_id: card_id.to_owned() });
    }
```

Left alone, every picture card's preview returns that error and the window prints "The
server has no preview for this card" forever. Add a `Picture` arm **before** that guard.

A picture card needs no rasterization: `CardPreview.frame` is an
`Option<RasterFrame>` and `RasterFrame` is `{ digest, bytes: Arc<[u8]> }` — exactly what
the host already holds. So the preview is the stored frame handed straight back, and
`server::admin::get_card_preview` re-encodes it with the existing `frame_png`. That is one
`Arc` clone, no resvg round trip, and byte-exact by construction rather than by a
rasterizer agreeing with itself.

```rust
    if let CardSettings::Picture { source_id, .. } = card {
        let source_id = source_id.clone();
        let Some(host) = state.plugin_host.as_deref_mut() else {
            return Ok(preview_failure(
                "this card's picture is held by the server, which this host is not".into(),
            ));
        };
        return Ok(match host.image_source_frame(&source_id) {
            Some(frame) => CardPreview {
                frame: Some(RasterFrame { digest: frame.digest, bytes: frame.bytes }),
                state: CardPreviewState::Ok,
                message: None,
                refreshed_at_unix_ms: None,
            },
            // No frame at all, so `Waiting` rather than `Error` -- the same
            // distinction a plugin card awaiting its first refresh draws, and
            // the same words the face itself shows.
            None => CardPreview {
                frame: None,
                state: CardPreviewState::Waiting,
                message: Some("Waiting for the first picture".into()),
                refreshed_at_unix_ms: None,
            },
        });
    }
```

Read `CardPreviewState`'s real variant names before using `Ok`/`Waiting` above; match what
the enum actually declares.

Add a test:

```rust
#[test]
fn a_picture_cards_preview_is_the_stored_frame_not_a_re_render() {
    // Byte-exact: the preview must be the same bytes the device holds, not a
    // rasterization of an image back into itself.
    let preview = /* render_card_preview for a picture card with a known frame */;
    assert_eq!(preview.frame.expect("a frame").bytes.as_ref(), KNOWN_BLOB);
}

#[test]
fn a_picture_card_with_no_frame_previews_as_waiting_not_as_an_error() { }
```

- [ ] **Step 6: Add the command**

In `commands.rs`, beside `InjectPluginSnapshot` (`:92-97`):

```rust
    /// One image source received a new picture. Carries no bytes: the handler
    /// reconciles the whole desired set from the host, which already owns them.
    ImageSourceUpdated {
        source_id: String,
        digest: [u8; 32],
        reply: CommandReply,
    },
```

In `process_command` (`runtime.rs:1606`), add the arm, and implement its handler in this
order — the order is the contract:

```rust
fn apply_image_source_update(
    state: &mut WorkerState,
    scheduler: &mut Scheduler,
    device: &mut dyn RuntimeDevice,
    source_id: &str,
    digest: [u8; protocol::ASSET_DIGEST_LEN],
) -> Result<(), RuntimeError> {
    // 1. Bytes before any digest is referenced. Reconcile the WHOLE desired
    //    set: `AssetRelease` is device-wide, and a partial pass is the
    //    empty-registry wipe all over again.
    // 2. Only then consider pushing a scene.
    // 3. And only if the visible card actually subscribes to this source --
    //    otherwise the frame is simply resident, and the loop uses it when it
    //    next lands there. That is what makes rotation free.
}
```

Add the public wrapper on `RuntimeHandle` beside `inject_plugin_snapshot` (`runtime.rs:944`).

- [ ] **Step 7: Detect a staleness flip on the tick**

Add to `WorkerState` a `last_picture_face: BTreeMap<String, (/* digest */ [u8; 32], /* stale */ bool)>`,
written whenever a picture card's scene is pushed. In the worker's per-tick work, for the
**active** card only — the only one whose scene is on the panel — if it is a `Picture`,
call `host.image_source_frame` and set `active_scene_dirty = true` when the pair differs
from the recorded one. That is the spec's "a comparison per source, not a timer per
source"; it needs no new scheduler deadline.

Evict entries for cards that leave the configuration, mirroring
`Scheduler::retain_event_alert_checks` (`scheduler.rs:239`).

- [ ] **Step 8: Run the tests**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/picture-cards/companion
cargo test -p app-core
```

- [ ] **Step 9: Mutation probes**

1. Reorder the handler to push the scene before reconciling assets — `an_image_source_update_for_the_visible_card_pushes_a_scene_after_the_bytes` fails.
2. Mark `active_scene_dirty` unconditionally — `an_image_source_update_for_a_card_that_is_not_on_screen_pushes_no_scene` fails.
3. Push a scene on a failed transfer — `a_failed_asset_transfer_pushes_no_scene_and_releases_nothing` fails.
4. Return `Some(ImageSourceFrame { stale: false, .. })` where the host means `None` — `a_picture_source_with_no_frame_yet_says_so_in_words` fails.

- [ ] **Step 10: Commit**

```bash
git add companion/crates/app-core/src/runtime.rs companion/crates/app-core/src/commands.rs \
        companion/crates/app-core/tests/runtime.rs
git commit -m "feat: draw a picture card, and take a pushed frame into the runtime"
```

---

### Task 8 — AMENDED DURING EXECUTION (2026-09-10)

Task 8's first implementer **refused to implement it as written**, and was right to.
Two seams it named:

1. `ServerPluginHost` receives only `Arc<PluginRegistry>`. The `ImageSourceStore` lives on
   `ServerState`, and `device_link.rs:89` and `admin.rs:1354` build the host without it. The
   implementer declined to work around this by re-reading frames from disk, because that
   would break the byte-provenance rule this plan states twice.
2. **`PluginHost::desired_assets()` returns `Vec<DesiredAsset>` and cannot fail**, so the
   plan's instruction that exceeding the ceiling "must be a NAMED error, never a silent
   truncation" is not implementable at that seam.

This is the same shape as M3's Task 10a, where an implementer refused to open a second
`device::Session` from a Tauri command rather than write something that would compile, pass
its tests, and put two processes on one cable.

**Resolution, and it does not require a fallible trait.**

The named error already exists one layer down: `AssetSyncError::TooManyDesiredAssets`
(`asset_sync.rs:57`), raised by `compose_asset_keep_set` (`:111`) and by
`reconcile_with_active_volatile_yielding` (`:283`). `compose_asset_keep_set` is already the
one place the wire's digest ceiling lives, and making `desired_assets()` fallible would
restate a bound the sync layer already states — the same reason this codebase calls
`protocol::validate_message` rather than re-encoding its rules at each caller.

So:

- **Do not make `desired_assets()` fallible.** Let the union flow through
  `compose_asset_keep_set`, which already refuses an over-ceiling set by name.
- **Add a mint-time guard** in `ImageSourceStore::mint`, so a person minting one source too
  many is refused at the moment they act, with a message naming the budget, rather than
  discovering it later as a failed device sync. That is the guard that actually protects a
  human; the sync-layer one protects the wire.
- **Ownership expands** to `server/src/lib.rs`, `server/src/device_link.rs` and
  `server/src/admin.rs`, purely to inject the store into the host at its construction sites.

**The arithmetic that made this urgent, checked rather than assumed.**
`MAX_DURABLE_REGISTRY_ASSETS` is 31 and is enforced at registry load **over plugin assets
alone**; `MAX_IMAGE_SOURCES` is 8. 31 + 8 = 39, so the union can exceed what the device can
hold and nothing in the tree caught it. In practice the curated plugins declare **two**
assets between them (`agenda` 1, `aqi` 1, `claude-limits` and `svg-aqi` none), so the real
union is at most 10 — but "cannot happen today" is not a bound, and the budget is now
checked where sources are created.

---

### Task 8: The desired-asset union — one function, one caller

Needs Tasks 4 and 7. **This is the task that can wipe a panel if it is wrong**, so its
test is the exact-request-sequence kind that caught the original wipe.

**Files:**
- Modify: `companion/crates/server/src/plugin_host.rs`
- Test: `companion/crates/server/tests/hostile_device.rs` (extend), plus unit tests

**Interfaces:**
- Produces: `ServerPluginHost::desired_assets` returning plugin-registry assets **united
  with** every image source's frame; `ServerPluginHost::image_source_frame` implemented.

**Why this is dangerous.** Three facts compound:
1. `AssetRelease.digests` is a device-wide **keep-set**; a digest omitted from it is deleted
   (`firmware/main/core/asset_store.c`'s compaction marks absent records DEAD).
2. After every successful native push, if a volatile digest is live the runtime sends a
   release built from `desired_assets()` **alone** (`runtime.rs:3205-3220`).
3. `ensure_durable_assets_for_scene` (`runtime.rs:3332`) refuses a native push with
   `MissingRequiredAsset` when a referenced digest is in neither `confirmed_assets` nor
   `desired_assets()`.

So a picture frame missing from `desired_assets()` does not merely fail to draw — it is
**deleted off the device**. The union is not tidiness.

**And the capacity ceiling now has two contributors.** `MAX_DURABLE_REGISTRY_ASSETS` is 31
(`MAX_ASSET_DIGESTS - 1`, reserving the volatile slot) and is enforced today only at
registry *load*, over plugin assets alone. The union must be bounded too: plugin assets +
image frames must not exceed 31. With `MAX_IMAGE_SOURCES = 8` and the four curated plugins
this fits, but the check belongs in code, not in arithmetic done once in a plan.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn the_desired_set_is_the_union_of_plugin_assets_and_image_frames() {
    // A host with plugins loaded AND sources present returns both, deduplicated
    // by digest, with the plugin bytes' Arc moved rather than re-read.
}

#[test]
fn a_device_with_both_receives_a_release_naming_both() {
    // The hostile_device style: assert the EXACT request sequence the device
    // sees, and that the AssetRelease keep-set contains every plugin digest and
    // every source digest. This is the shape that caught the empty-registry wipe.
}

#[test]
fn an_empty_registry_with_sources_present_still_syncs_the_sources() {
    // The default deployment has no plugins directory. Before the union, that
    // was an empty desired set; now it must carry the frames.
}

#[test]
fn no_plugins_and_no_sources_still_skips_the_release_entirely() {
    // The stage-3b guard must survive: an empty desired set is NOT a no-op
    // reconcile, it is "wipe everything". `synchronize_full` skips the pass.
}

#[test]
fn a_union_over_the_durable_ceiling_is_refused_by_name() {
    // Not silently truncated. Assert the typed error names the limit.
}

#[test]
fn revoking_a_source_removes_its_frame_from_the_desired_set() { }
```

- [ ] **Step 2-4: Run red, implement, run green**

Implement `desired_assets` as the union, keeping `plugin_registry::all_assets()`'s
byte-provenance guarantee (move the `Arc`, never re-read or re-hash) and doing the same for
the store's frames. Implement `image_source_frame` by delegating to
`ImageSourceStore::frame(source_id, Utc::now())`.

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/picture-cards/companion
cargo test -p server
```

- [ ] **Step 5: Mutation probes**

1. Return only the registry's assets — `a_device_with_both_receives_a_release_naming_both` and `an_empty_registry_with_sources_present…` both fail.
2. Delete the ceiling check — `a_union_over_the_durable_ceiling_is_refused_by_name` fails.
3. Delete `synchronize_full`'s empty-set skip — `no_plugins_and_no_sources_still_skips_the_release_entirely` fails.

- [ ] **Step 6: Commit**

```bash
git add companion/crates/server/src/plugin_host.rs companion/crates/server/tests/
git commit -m "feat: one desired-asset set, plugin assets united with picture frames"
```

---
### Task 9: The companion app — a picture card is a peer of every other card

Needs Task 2 (the Rust variant drives the contract fixture). Independent of Tasks 6-8.

**Files:**
- Modify: `companion/apps/deskmate/src-tauri/src/commands.rs` (contract fixture, preview dispatch, server card-state filter)
- Modify: `companion/apps/deskmate/src/lib/types.ts`, `src/lib/types.contract.ts` (generated)
- Modify: `companion/apps/deskmate/src/lib/configDraft.ts` (`cardLabel`, `addCard`, `AddCardRequest`)
- Modify: `companion/apps/deskmate/src/components/CardList.tsx` (add menu, `tileValue`)
- Modify: `companion/apps/deskmate/src/components/CardEditor.tsx` (Picture arm)
- Modify: `companion/apps/deskmate/src/dev/*` (a `picture` scenario)
- Modify: `DESIGN.md` (the documented scenario list)
- Test: `companion/apps/deskmate/tests/configDraft.test.ts`, `tests/components.test.tsx`

**Five compile-time gates will force most of this.** They are the mechanism, so do not
defeat any of them with a wildcard arm:
1. `contract_card_kind` (Rust, `commands.rs:2638`) — an exhaustive `match` on `CardSettings`.
2. `contract_fixture_covers_every_card_settings_variant` (Rust, `commands.rs:3138`) — asserts the kind list.
3. `typescript_contract_fixture_stays_in_sync` (Rust, `commands.rs:3546`) — byte-compares the generated TS fixture.
4. `tileValue()` (TS, `CardList.tsx:95`) — an exhaustive `switch (card.kind)`.
5. `renderMockFrame()` (TS, `mockPreview.ts:162`) — likewise.

`CardEditor.tsx` is **not** exhaustive — it is a sequence of `{card.kind === "…" && (…)}`
guards — so its Picture arm must be added deliberately. Put it directly after the plugin
arm at `CardEditor.tsx:556`.

**One naming decision to get right.** `AddableCardKind` is `Exclude<CardKind, "plugin">`,
and adding `"picture"` to `CardSettings` would sweep it into that union automatically —
which would be **wrong**. Creating a picture card creates its source (spec §3), so adding
one is a server round trip, exactly like adding a plugin card. Make it:

```ts
export type AddableCardKind = Exclude<CardKind, "plugin" | "picture">;

export type AddCardRequest =
  | AddableCardKind
  | { kind: "plugin"; pluginId: string; refreshMinutes: number }
  | { kind: "picture"; sourceId: string; sourceName: string };
```

`cardKindName(kind: AddableCardKind)` then needs no `"picture"` case, and `cardLabel` grows
a picture branch beside its plugin branch, returning the literal `"Picture"` — the template
name, per the repository's naming rule, with `cardTitle` supplying the owner's words as the
quiet second line.

- [ ] **Step 1: Write the failing frontend tests**

Add to `companion/apps/deskmate/tests/configDraft.test.ts`:

```ts
test("a picture card is called Picture, with the owner's words beside it", () => {
  const card: CardSettings = {
    kind: "picture",
    id: "shot",
    title: "Limits",
    source_id: "limits",
    tap_action: "none",
    refresh: "manual",
    alert: "none",
  };
  expect(cardLabel(card, null)).toBe("Picture");
  expect(cardTitle(card)).toBe("Limits");
});

test("adding a picture card enrols it in the loop", () => {
  // addCard({ kind: "picture", sourceId, sourceName }) must add BOTH the card
  // and its playlist entry, and declare the source in image_sources.
});

test("adding a picture card declares its source exactly once", () => {
  // Two cards may share one source; adding a second card for an existing
  // source must not duplicate the image_sources entry.
});
```

- [ ] **Step 2: Run and watch them fail**

```bash
cd /Users/rodion/dev/deskmate/.worktrees/picture-cards/companion/apps/deskmate
bun test
```

- [ ] **Step 3: Work through the gates, in this order**

The order matters — the Rust fixture generates the TypeScript one, so TS edits before
Rust edits get overwritten.

1. Fix `contract_card_kind` with `CardSettings::Picture { .. } => "picture",`.
2. Add a `CardSettings::Picture { .. }` instance to `all_card_settings` in
   `contract_fixtures()` (`commands.rs:2651-2740`), modelled on the plugin instance at
   `:2734`.
3. Extend the expected array in `contract_fixture_covers_every_card_settings_variant`
   with `"picture"`.
4. Regenerate and paste the TypeScript fixture:
   ```bash
   export PATH="$HOME/.cargo/bin:$PATH"
   cd /Users/rodion/dev/deskmate/.worktrees/picture-cards/companion
   cargo test -p deskmate-app print_typescript_contract_fixture -- --ignored --nocapture
   ```
   Paste the output verbatim into `apps/deskmate/src/lib/types.contract.ts`.
5. Add the `picture` arm to the `CardSettings` union in `types.ts:58-134`, and narrow
   `AddableCardKind` as above.
6. `cardLabel` (`configDraft.ts:75`): return `"Picture"` for `card.kind === "picture"`.
7. `addCard` (`configDraft.ts:174`): handle the new request shape — add the card, enrol it
   in the loop (gated on both `MAX_CARDS` and `MAX_PLAYLIST_ENTRIES`, as the existing code
   does), and append to `image_sources` if that id is not already declared. Id stem
   `"picture"` at `configDraft.ts:189`.
8. `tileValue()` (`CardList.tsx:95`): a `"picture"` arm.
9. The add menu: picture is **not** in `addableKinds` (that list is `AddableCardKind`).
   Add it as its own menu entry that calls a new `onAddPicture` prop, which `App.tsx` wires
   to: mint a source on the server, then `addCard({ kind: "picture", … })`. Extend the
   roving-focus `count` at `CardList.tsx:325` to include it.
10. `CardEditor.tsx`: a Picture arm after `:556` showing the source (a select over
    `config.image_sources`, plus the machine id in a `<small>`, mirroring the plugin arm's
    treatment), the push URL, and — **only immediately after minting** — the plaintext
    token, with a copy action and a line saying it is shown once. Never re-fetch or store it.
11. `refreshesOnServer` (`CardEditor.tsx:157`): widen from `card.kind === "plugin"` to a
    set including `"picture"` — the Mac has nothing to refresh for either.
12. `commands.rs:335`: the server card-state filter keeps only
    `CardSettings::Plugin { .. }`. Admit `Picture` so a picture card's provider state and
    errors reach the tile.
13. `commands.rs:993`: the preview dispatch is `matches!(card, CardSettings::Plugin { .. })`.
    Admit `Picture` so the server renders its preview through the existing route. In local
    tier it must fall to `PLUGIN_RENDERS_ON_THE_SERVER`'s sibling — add a picture-specific
    constant rather than reusing a sentence that says "Plugin".
14. `DevicePreview.tsx:13`'s `LIVE_TEMPLATES` — leave it alone. A picture card does not
    re-render at 1 Hz.

- [ ] **Step 4: Add the dev-harness scenario**

Six places, per the harness's own structure:
1. `src/dev/mockBackend.ts:31` — add `"picture"` to `SCENARIOS`.
2. `src/dev/mockBackend.ts:59` — a `case "picture":` arm in `applyScenario()`.
3. A fixture mirroring `src/dev/pluginFixture.ts`.
4. `src/dev/mockBackend.ts:349` — the `render_card_preview` arm.
5. `src/dev/mockPreview.ts:162` — the exhaustive `switch`.
6. `DESIGN.md:252-256` — the documented scenario list.

- [ ] **Step 5: Run every frontend and contract gate**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/picture-cards/companion
cargo test -p deskmate-app
cd apps/deskmate && bun test && bun run build
```

- [ ] **Step 6: Check it by hand, with a real pointer**

```bash
cd /Users/rodion/dev/deskmate/.worktrees/picture-cards/companion/apps/deskmate
VITE_DESKMATE_MOCK=1 bun run dev
```

Open `?scenario=picture`, and `?scenario=picture&theme=dark`. Add a picture card from the
menu **with a real mouse click**.

**Read this before concluding the click works.** The harness runs in Chrome; the app runs
in WKWebView, and they do not agree about focus. WebKit does not move focus to a `<button>`
on mousedown, and a blur handler reading `relatedTarget` sees `null` in exactly that case —
which is how "no card of any kind could be added" shipped once already. Chrome cannot see
that class of defect, and neither can an `AXPress`-driven check, because `AXPress` fires
`click` with no `mousedown` at all. If the add menu changes at all here, the honest check is
a unit test replaying the WebKit sequence — `a click inside the menu is not mistaken for
focus leaving it` already does this in 3 ms — plus a real pointer in the real app.

- [ ] **Step 7: Commit**

```bash
git add companion/apps/deskmate DESIGN.md
git commit -m "feat: a picture card is a peer of every other card in the window"
```

---

### Task 10: Documentation

Independent of every other task. Can be written first, last, or concurrently.

**Files:**
- Create: `docs/config/v7.md`
- Create: `docs/images/producer-guide.md`
- Modify: `docs/plugins/manifest-v1.md`, `docs/plugins/manifest-v2.md` (freeze headers)
- Modify: `companion/crates/server/deploy/README.md` (rollout recipe)
- Modify: `PRODUCT.md`

- [ ] **Step 1: Write `docs/config/v7.md`**

Copy `docs/config/v6.md`'s structure exactly — it is 284 lines with these headings:
`Why the card kind changed`, `Root and stable identity`, `The plugin card kind`,
`Compatibility-check interaction`, `Unknown-field rejection`,
`Compiling cards, playlists, and assets to the wire`, `Validation rules`,
`Migration and compatibility`, `Validation-failure behavior`,
`Where CURRENT_SCHEMA_VERSION lives`.

v7 adds two sections — `The picture card kind` and `Image sources` — and must state:
- `image_sources` is document-level and `#[serde(default)]`, so a v4/v5/v6 document parses.
- A card naming an unknown `source_id` is `ValidationCode::MissingReference`, never blank.
- No credential is ever in the document.
- `MAX_IMAGE_SOURCES = 8`, and why (2.6 MB of the assets partition, 8 of 31 durable digests).
- A picture card compiles to `TemplateKind::DigitalClock` on the wire, like a plugin card,
  so the device learns nothing new.

- [ ] **Step 2: Write `docs/images/producer-guide.md`**

Sections: what a picture card is; the one-line `curl`; an image-requirements table
(exactly 448x368, 8-bit, RGB or RGBA, 1 MiB body cap); authentication (path token **and**
`Authorization: Bearer`, one verification, two carriers); the error table (401/413/415/422/429,
each written for the producer with no internals); cadence and staleness (nothing is
declared — the server infers from the last 8 accepted push times, median of consecutive
intervals × 3, clamped to 15 min…48 h, never stale below 3 intervals, any push clears it);
and designing a good panel.

That last section carries the two facts a producer cannot guess:
- **RLE565 expands 2× on high-entropy input.** A flat-colour UI panel crosses the link at
  ~10 KB; photographic content costs the full ~330 KB. Prefer flat-colour panels.
- **Keep the bottom strip calm.** A stale footer overlays the picture, because a picture is
  full-bleed and there is no reserved strip.

- [ ] **Step 3: Add the freeze headers**

Both `docs/plugins/manifest-v1.md` and `manifest-v2.md` already open with `Status: frozen`,
meaning the *contract* is frozen. Spec §8 asks for more than that. Amend each to say
plainly: **no new manifest features, no new curated manifests, no contract amendments**,
and point at `docs/superpowers/specs/2026-09-09-deskmate-picture-cards-design.md` for what
replaces the path. Say explicitly that the four curated plugins keep working and their
tests stay — frozen, not removed.

- [ ] **Step 4: Extend the deploy runbook**

`companion/crates/server/deploy/README.md` §6 already records the manifest-v2 rollout
recipe. Add the picture-card one, because the order is load-bearing in the same way:

1. **Redeploy the server binary first.** It compiles its own `CURRENT_SCHEMA_VERSION`, so
   until it moves, every save of a v7 config is refused with the typed
   "schema version 7 is not supported; expected 6".
2. Mint the source and capture its token — it is shown once.
3. Point the producer at the webhook.
4. Save the v7 config.

- [ ] **Step 5: Update `PRODUCT.md`**

One short section on what a picture card is for, in product terms: a card anybody can add
from outside this repository with one line of `curl`, no manifest and no SDK.

- [ ] **Step 6: Commit**

```bash
git add docs/config/v7.md docs/images/producer-guide.md docs/plugins/ \
        companion/crates/server/deploy/README.md PRODUCT.md
git commit -m "docs: the v7 config contract, the producer guide, and the manifest freeze"
```

---

### Task 11: Gates, rollout, and the hardware session

- [ ] **Step 1: Run the full workspace gates**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
set -o pipefail
cd /Users/rodion/dev/deskmate/.worktrees/picture-cards/companion
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo test --workspace --doc
```

All four. `--all-targets` adds example and integration targets but **removes doctests**, so
neither invocation alone covers the workspace. Keep them separate so a failure names the
missing coverage. Do not pipe these into `tail` without `set -o pipefail` — a missing
`cargo` on `PATH` then reports as a clean run, which has produced a false green here before.

Firmware gates are untouched: **no firmware changed**, so there is no OTA download check
owed and no memory-layout hazard. Say so explicitly in the commit rather than leaving a
reader to wonder.

- [ ] **Step 2: Frontend gates**

```bash
cd /Users/rodion/dev/deskmate/.worktrees/picture-cards/companion/apps/deskmate
bun test
bun run build
```

- [ ] **Step 3: Deploy the server, in this order**

Per `companion/crates/server/deploy/README.md` §6 and the constraints in `CLAUDE.md`:
build for linux/x86_64 in a throwaway `rust:1.98-bookworm` container over an rsync'd copy
of `companion/`, from a `git archive HEAD` export and never the working tree; reach
docker-vm over Tailscale; keep the root-owned `companion/target/`; and **never `--delete`
the live plugins directory**, which holds five ids including the session fixture
`svg-live-clock` that is not in this repository.

Keep the previous binary as a dated `.bak`, as the last deploy did.

- [ ] **Step 4: Mint the source and point a producer at it**

The `claude-limits` producer lives in the TRMNL repo on docker-vm and currently emits JSON;
converting it to draw a 448x368 PNG (PIL is the obvious tool) is a change to **that**
repository, and committing it is the owner's call — as it was for the `resets_at_epoch`
field. This plan does not commit it.

- [ ] **Step 5: The hardware session — three observations, not a battery**

No firmware change means this is short. Record the observed result in
`docs/hardware/board-notes.md` under a dated heading, per the working agreement.

1. A PNG pushed from the real producer appears on the panel at the mounted orientation.
2. Rotating away from the picture card and back moves **no bytes** — the digest is already
   resident. Confirm from the server log that no asset transfer occurred, not from the
   panel, which cannot show the difference.
3. A second, different picture replaces the first.

**Deliberately not run:** any new `framebuffer_diff` rows. A picture card's face is a
full-canvas `SceneImage`, and `scene-image` has been in the on-target matrix at both
orientations since stage 3b Task 8. New rows would be near-duplicate coverage and would
disturb a hardware-pinned number for nothing. **The 96/10/86 split remains the
2026-09-06 observation; after manifest removal the next-run software prediction is
78/8/70.**

Do not turn this into a long battery of retries. If an observation fails, record what was
seen and stop.

---

## Self-review

**Spec coverage.** §4 data model -> Task 2. §5 ingest and auth -> Tasks 3, 4, 6. §6 runtime
and push path -> Task 7 (and the Mac half in Task 9). §7 inferred staleness -> Task 5, wired
in 4 and 7. §8 freezing the manifest path -> Task 10 Step 3. §9 interactivity constraint ->
no task; it is a non-goal and nothing here forecloses it (a tap would arrive as an
interaction event and produce a new picture through the ordinary `ImageSourceUpdated`
flow). §10 testing -> every task's tests plus its mutation probes; the keep-set union row is
Task 8, the bomb row is Task 3, the pinned-digest row is Task 3. §11 hardware -> Task 11
Step 5. §12 rollout -> Task 11 Steps 3-4 and Task 10 Step 4. §13 open items -> out of scope
by the spec's own words.

**Found during self-review and fixed inline.** The first draft of this plan had no
server-side preview for a picture card at all: `render_card_preview` opens by returning
`NotAPluginCard` for every other kind, so every picture card's preview would have shown
"The server has no preview for this card" permanently. Task 7 Step 5b closes it, and
`ImageSourceFrame` gained its `bytes` field to make the preview byte-exact.

**Known gaps, stated rather than hidden.**
- The spec's `ImageSourceUpdated { bytes }` field is dropped, with the reason recorded in
  Task 7. It is the one place this plan does not do what the spec says.
- Task 4 leaves one decision to its implementer — whether its tests are an integration file
  or an inline module — because that depends on whether `CanonicalFrame` stays `pub(crate)`.
  The step says to decide it before writing the tests rather than discovering it halfway.
- Task 6's HTTP tests cannot be run by a Codex worker (its sandbox denies loopback binds).
  The controller runs them. This is noted in the task.

**Type consistency.** `ImageSource { id, name }`, `CardSettings::Picture { id, title,
source_id, tap_action, refresh, alert }`, `CanonicalFrame { digest, bytes }`,
`SourceFrame { digest, bytes, stale }`, `ImageSourceFrame { digest, stale }` and
`frame_face_scene(revision, digest)` are used with the same names and shapes in every task
that references them. `ImageSourceFrame` (the runtime seam) and `SourceFrame` (the store's
own type) carry the same three things and Task 8 converts between them; they stay distinct
only because `app-core` cannot name a `server` type.
