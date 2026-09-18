# Plugin Card Parity Implementation Plan

> **STATUS — HISTORICAL IMPLEMENTATION RECORD.** The manifest-plugin and native-companion
> paths described here were retired; this plan is not current architecture or build guidance.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **STATUS (recorded 2026-09-09): Tasks 1-5 DELIVERED; Task 6's rollout was EXECUTED on
> 2026-09-08. None of the 107 boxes were ever ticked and they are NOT a progress signal —
> read the "Rollout record" section at the end of this file, which carries the real
> evidence.** Rollout A-C passed against the live server; Rollout D is recorded as
> partly observed, because no card in the fleet currently names a plugin and the board
> has been offline since 2026-09-07.

**Goal:** A server-rendered plugin card looks and behaves like a built-in card on every
surface of the Mac app — name, live tile value, freshness, preview, add gesture, editor —
without a schema or protocol change.

**Architecture:** The server stays the single owner of plugin rendering and grows three
additive admin surfaces: a richer plugin catalog, a per-card preview PNG rebuilt on the
runtime worker with an unminted revision, and a `hero` field produced from a
manifest-declared summary expression. The Mac reads those over its existing bearer-token
client, projects the server's plugin state onto its own snapshot, and stops faking plugin
refreshes in its hostless runtime. The frontend names, adds, edits and previews plugin
cards through the same code paths as built-ins, with the catalog threaded through one
label helper.

**Tech Stack:** Rust 1.98 (app-core, plugin, server, Tauri v2 shell; `ureq` 3 + rustls
client; `resvg` 0.45.1 rasterizer), React 19 + TypeScript 7 + Vite 8 + Biome + Bun tests,
schema v6 (frozen), protocol v1 (frozen), manifest v2 (amended additively).

**Spec:** `docs/superpowers/specs/2026-09-06-deskmate-plugin-card-parity-design.md`

## Global Constraints

- Config schema stays **v6** and protocol stays **v1**; nothing new reaches the device.
- Manifest v1 stays frozen; the amendment is v2-only and every new key is optional.
- Bounds, verbatim from the spec: `MAX_DISPLAY_NAME_LEN` = 64 bytes, `MAX_DESCRIPTION_LEN`
  = 160 bytes, `MAX_SUMMARY_LEN` = 32 bytes of evaluated output; a `summary` may not use a
  device binding (`time:`, `timer.`, `date`, `field.`) — that is a manifest error.
- Existing `GET /v1/plugins` fields serialize byte-identically to today.
- The preview route rebuilds with revision `0`, never mints `next_scene_revision`, never
  touches the device, never marks the scene dirty.
- The Mac's GETs read bounded bodies: 64 KiB for catalog and status, 1 MiB for a preview.
- Every validation issue stays claimed by a surface that renders it; `cards[i].plugin_id`
  gains a control in the editor.
- A card is called the same thing on every surface (`cardLabel`), and a plugin card is
  never called "Plugin"; fallbacks print a word (`not on the server`, `needs the server`).
- Copy is plain, calm, second person; every state prints a word beside its colour.
- Commit prefixes: `feat:` / `fix:` / `test:` / `docs:`; never rewrite shared history.
- Gates before handoff, from `companion/`: `cargo fmt --all --check`, `cargo clippy
  --workspace --all-targets -- -D warnings`, `cargo test --workspace --all-targets`,
  `cargo test --workspace --doc`; from `companion/apps/deskmate`: `bun test`, `bun run
  check`, `bun run lint`, `bun run format:check`.
- Rollout order: server (registry-compatible) → curated manifests to v2 → deploy → Mac app.

## File Structure

| Area | File | Responsibility |
|---|---|---|
| plugin | `companion/crates/plugin/src/manifest.rs` | parse the three optional v2 keys, bounds, the device-binding check |
| plugin | `companion/crates/plugin/src/summary.rs` (new) | `evaluate_summary` — one expression against a snapshot, truncated to 32 bytes |
| plugin | `companion/plugins/{aqi,agenda,claude-limits,svg-aqi}/manifest.toml` | move to v2 with `display_name`, `description`, `summary` |
| plugin | `docs/plugins/manifest-v2.md` | the amendment, with bounds |
| app-core | `companion/crates/app-core/src/admin.rs` | shared DTOs: `PluginCatalog*`, `CardPreviewResponse`, `CardPreviewState`, `PluginTemplateKind` |
| app-core | `companion/crates/app-core/src/runtime.rs` | `RuntimeCommand::RenderCardPreview`, `RuntimeHandle::render_card_preview`, `CardPreview`; hostless runtime skips plugin scheduling |
| server | `companion/crates/server/src/admin.rs` | catalog fields; `GET /v1/devices/{id}/cards/{card_id}/preview` |
| server | `companion/crates/server/src/rasterizer.rs` | `frame_png` lifted to `pub(crate)` |
| server | `companion/crates/server/src/plugin_refresher.rs` | emit `hero` from the summary |
| Mac | `companion/apps/deskmate/src-tauri/src/server_client.rs` (new) | URL builder, bounded GET, error mapping shared by every server call |
| Mac | `companion/apps/deskmate/src-tauri/src/commands.rs` | `get_server_plugins`, `get_server_card_state`, the plugin arm of `render_card_preview`, contract fixtures |
| Mac | `companion/apps/deskmate/src-tauri/src/lib.rs` | `ServerStateProjection`, handler registration |
| frontend | `src/lib/types.ts`, `src/lib/tauri.ts` | DTOs and wrappers |
| frontend | `src/lib/useAppState.ts` | catalog fetch, 30 s visible-only status poll, one notice |
| frontend | `src/lib/configDraft.ts` | `cardLabel(card, catalog)`, `AddCardRequest`, the plugin arm of `addCard` |
| frontend | `src/components/CardList.tsx` | picker rows from the catalog, tile value and flags from the projection |
| frontend | `src/components/CardEditor.tsx` | the Plugin select, refresh hint, error copy |
| frontend | `src/components/DevicePreview.tsx` | state words on the stage |
| frontend | `src/dev/pluginFixture.ts` (new), `src/dev/mockBackend.ts`, `src/dev/mockPreview.ts`, `src/dev/fixture.ts` | `?scenario=plugin` and the three mocked commands |
| docs | `PRODUCT.md`, `DESIGN.md`, `CLAUDE.md` | positioning amendment, naming bullet, state bullet |

## Ownership — read this before touching a shared file

Each of these is owned by exactly one task. If your task needs one, it *consumes* it.

| Thing | Owner | Everyone else |
|---|---|---|
| `app-core/src/admin.rs` DTOs (all seven) and their tests | Task 2 | use `app_core::admin::*` |
| `app-core/src/lib.rs` re-export list | Task 2 | re-exports everything **except** `PluginLoadFailure`, which stays path-qualified because `server::plugin_registry::PluginLoadFailure` already exists |
| Every `AdminError` / HTTP status mapping | Task 3 | Task 2 names `RuntimeError` variants only and never mentions `AdminError` |
| Every TypeScript file (`src/lib/*`, `src/components/*`, `src/dev/*`) | Task 5 | Task 4 is Rust only; its sole frontend artefact is the **generated** `src/lib/types.contract.ts` |
| `docs/plugins/manifest-v2.md` | Task 1 | Task 6 cross-checks, never rewrites |

## Task order, and the one deliberate red state

Tasks run **1 → 2 → 3 → 4 → 5 → 6**; each depends on the one before.

**Task 2 ends with `cargo check -p server` failing on purpose.** It adds
`RuntimeError::UnknownCard` and `RuntimeError::NotAPluginCard`, and the server's
`From<RuntimeError> for AdminError` conversion is exhaustive, so the server stops compiling
until **Task 3's first step** maps both to the existing unit `AdminError::NotFound`. This is
the arbitration decision that HTTP mapping lives in exactly one task. Do not "fix" it inside
Task 2 with a placeholder arm: Task 2's gates are scoped to `-p app-core -p deskmate-app`,
and the workspace-wide gates run at Task 3's exit.

## Interface Contract

Names every task must use verbatim. A task's own steps define nothing that contradicts this.

**plugin crate** — `MAX_DISPLAY_NAME_LEN` 64, `MAX_DESCRIPTION_LEN` 160, `MAX_SUMMARY_LEN`
32. `PluginManifest` gains `display_name: Option<String>`, `description: Option<String>`,
`summary: Option<String>` (a text node's value is an opaque `String`, so the summary holds
the same). `ManifestError::SummaryUsesDeviceBinding { binding: String }` and
`ManifestError::EmptyString { field: &'static str }`. `evaluate_summary(&PluginManifest,
&ProviderSnapshot<Value>) -> Result<Option<String>, SummaryError>`; `Ok(None)` when
undeclared or when the expression resolves to missing; `Ok(Some(_))` truncated to 32 bytes
on a char boundary.

**app-core** — `admin::{PluginCatalog, PluginCatalogEntry, PluginCatalogAsset,
PluginLoadFailure, PluginTemplateKind, CardPreviewState, CardPreviewResponse}`;
`runtime::CardPreview { frame: Option<RasterFrame>, state, message, refreshed_at_unix_ms }`;
`RuntimeHandle::render_card_preview(&self, card_id: &str) -> Result<CardPreview, RuntimeError>`;
`RuntimeError::{UnknownCard, NotAPluginCard}`.

**server** — `GET /v1/plugins` returns `app_core::admin::PluginCatalog`;
`GET /v1/devices/{id}/cards/{card_id}/preview` returns `CardPreviewResponse`, mapping
unknown device / unknown card / non-plugin card to the existing unit `AdminError::NotFound`
and a missing runtime to `503`; `rasterizer::frame_png` becomes `pub(crate)`;
`plugin_refresher` emits `hero` after `title` only on `Ok(Some(_))`.

**Mac Rust** — `get_server_plugins`, `get_server_card_state`, and the plugin arm of the
existing `render_card_preview`; `ServerCardState { card_id, provider, hero, errors }`;
`PreviewFrame { png_base64: Option<String>, sample: bool, state: Option<String> }` (built-in
cards keep `png_base64` set and `state` `None`); `ServerStateProjection(Mutex<Vec<ServerCardState>>)`.

**Frontend** — `cardLabel(card, catalog?)`; `AddCardRequest = AddableCardKind | { kind:
"plugin"; pluginId: string; refreshMinutes: number }`; `PluginKindOption { id; version;
displayName: string | null; description: string | null; onAdd }`; `CardListProps` and
`CardEditorProps` gain `ownershipTier: DeviceTier | null`; `useAppState` gains
`pluginCatalog`, `catalogError`, `refreshCatalog`, `serverCardState`.

**Two decisions the code forced, not the spec** (see spec §10a): the waiting state returns
no frame and prints a sentence rather than the "No data yet" badge, which stays for sample
frames; and the device ignores unknown `PushData` fields
(`firmware/main/core/template_fields.h`), so `hero` needs no device change.

---

### Task 1: Manifest v2 amendment and the summary expression

Implements spec §3. Adds the three optional presentation keys to manifest v2, evaluates `summary` server-side to a bounded headline, gives all four curated plugins those keys (three move from v1 to v2; `svg-aqi` is already `manifest_version = 2` and only gains the keys), and amends the frozen docs. Nothing here touches the wire, the scene, the server routes or the Mac app — those are later tasks that consume what this one produces.

**This task owns `docs/plugins/manifest-v2.md`** (reconciliation item 5). Task 6 does not rewrite that file; it only cross-checks that Step 10's amendment is present and that the fixture paths Step 10 records under "V1 compatibility" survive. `ManifestError::EmptyString { field: &'static str }` is approved and part of the shared contract (item 7).

Every Rust command below runs from `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion` and invokes cargo by absolute path, `/Users/rodion/.cargo/bin/cargo` — never `export PATH=…`, which the session's shell guard rejects. Git commands use `git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity`. Where a command is piped into `head`/`grep`, that is to *read* output; never judge a cargo gate by a pipeline's exit status, because the pipeline reports the last stage's status.

**Files:**
- Create: `companion/crates/plugin/src/summary.rs`; `companion/crates/plugin/tests/summary.rs`; `companion/crates/plugin/tests/fixtures/manifest_v1_aqi.toml` and `manifest_v1_agenda.toml` (byte-exact copies of today's curated manifests, frozen before they move to v2).
- Modify: `companion/crates/plugin/src/manifest.rs` (module doc 9-18; bounds 31-121 with `MAX_URL_LEN` at 69; `ManifestError` 131-224 ending `IncompleteBoundLineGeometry,` at 223; `PluginManifest` 239-253 with `pub template: Template,` at 252; `RawPluginManifest` 295-309 with `template: Option<Template>,` at 308; `skip_string` 546-564; `check_len` 566-576; `validate` 664-713 with `check_len("version", …)` at 661… 662; `parse_manifest` 863-935 with the `source.root` arm closing at 886 and the `PluginManifest { … }` literal at 923-932; `mod tests` 956-1621); `companion/crates/plugin/src/compile.rs` (`looks_like_binding_namespace` at 184, `classify_expression_source` at 244, `ExpressionSource` at 226-233; **nine** test-module `PluginManifest {` literals whose `manifest_version: ManifestVersion::V1,` lines are at 1021, 1464, 1515, 1588, 1690, 1755, 1786, 1882, 1941); `companion/crates/plugin/src/assets.rs` (one more such literal, `manifest_version: ManifestVersion::V1,` at 358); `companion/crates/plugin/src/lib.rs` (whole file, 35 lines); `companion/crates/plugin/tests/manifest_v2.rs` (the frozen-v1 test at 75-79); `companion/crates/plugin/tests/curated_plugins.rs` (import at 16; append at 176); `companion/plugins/aqi/manifest.toml` (19-25), `companion/plugins/agenda/manifest.toml` (23-29), `companion/plugins/claude-limits/manifest.toml` (23-29), `companion/plugins/svg-aqi/manifest.toml` (whole file, 13 lines); `docs/plugins/manifest-v2.md` (shape 13-27, "V2 adds only" 29-32, "V1 compatibility" 138-149); `docs/plugins/manifest-v1.md` (14-16).

**Interfaces:**
- Consumes: `compile::classify_expression_source(&str) -> ExpressionSource<'_>` and `ExpressionSource::{Literal, Expression, MalformedPartial}` (compile.rs 226-287, already `pub`; `Expression` carries the **trimmed inner body**, not the whole source); `compile::looks_like_binding_namespace(&str) -> bool` (184, private today → `pub(crate)` in Step 5); `expr::{Expr::parse, Expr::eval, EvalContext::with_data, Fuel::new, FUEL_BUDGET, ExprError, EvalValue::is_missing}` (`ExprError` derives `PartialEq, Eq`, so `SummaryError` can too); `manifest::skip_string(&[u8], usize, u8) -> usize` (546); `protocol::truncate_utf8_to_bytes(&str, usize) -> &str` (`protocol/src/util.rs:31`, char-boundary safe); `providers::ProviderSnapshot<serde_json::Value> { value, refreshed_at, age, stale, error }`.
- Produces (plugin crate):
  - `pub const MAX_DISPLAY_NAME_LEN: usize = 64;` `pub const MAX_DESCRIPTION_LEN: usize = 160;` (manifest.rs, re-exported from `lib.rs`) and `pub const MAX_SUMMARY_LEN: usize = 32;` (summary.rs, re-exported).
  - `PluginManifest` gains `pub display_name: Option<String>`, `pub description: Option<String>`, `pub summary: Option<String>` — a text node's `value` is an opaque `String` (manifest.rs 467), so `summary` holds the same. All three are `None` on every v1 manifest.
  - `ManifestError::SummaryUsesDeviceBinding { binding: String }` (parse-time) and `ManifestError::EmptyString { field: &'static str }` for a present-but-empty `display_name`/`description` (reconciliation item 7). `ManifestError::V2FieldInV1 { field }` now also fires for `"display_name"`, `"description"`, `"summary"`.
  - `pub enum SummaryError { MalformedPartialInterpolation { text: String }, Expression(ExprError) }` and `pub fn evaluate_summary(manifest: &PluginManifest, snapshot: &providers::ProviderSnapshot<serde_json::Value>) -> Result<Option<String>, SummaryError>`. `Ok(None)` when undeclared **and** when the expression evaluates to `EvalValue::Missing` (the tile then prints "—", the spec's absent case; an empty-string hero would print nothing). `Ok(Some(_))` is truncated to `MAX_SUMMARY_LEN` bytes on a char boundary. Later tasks: the server's refresher emits `hero` only on `Ok(Some(_))` and logs once per plugin id on `Err`.

- [ ] **Step 1: Repoint the frozen-v1 contract test at fixtures that do not exist yet (red), then freeze byte-exact v1 copies (green).**

  `tests/manifest_v2.rs:75-79` proves the v1 contract against the curated manifests; once they move to v2 (Step 9) that proof would vanish. Replace lines 75-79:

  ```rust
  #[test]
  fn the_frozen_v1_fixture_manifests_reparse_with_every_frozen_field_unchanged() {
      // Byte-exact copies of `companion/plugins/{aqi,agenda}/manifest.toml` as
      // they shipped under v1, frozen when the curated plugins moved to v2
      // (plugin-parity Task 1) so the v1 contract keeps real coverage.
      assert_frozen_v1_fields_are_identical(include_str!("fixtures/manifest_v1_aqi.toml"));
      assert_frozen_v1_fields_are_identical(include_str!("fixtures/manifest_v1_agenda.toml"));
  }
  ```

  Run the red state:

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p plugin --test manifest_v2 2>&1 | grep -E "^error" | head -5
  ```

  Expected: `error: couldn't read ".../tests/fixtures/manifest_v1_aqi.toml": No such file or directory (os error 2)`. Now freeze the copies, while the curated manifests are still v1:

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && cp plugins/aqi/manifest.toml crates/plugin/tests/fixtures/manifest_v1_aqi.toml && cp plugins/agenda/manifest.toml crates/plugin/tests/fixtures/manifest_v1_agenda.toml && cmp plugins/aqi/manifest.toml crates/plugin/tests/fixtures/manifest_v1_aqi.toml && cmp plugins/agenda/manifest.toml crates/plugin/tests/fixtures/manifest_v1_agenda.toml && echo frozen
  ```

  Expected: `frozen`. Then:

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p plugin --test manifest_v2 the_frozen_v1
  ```

  Expected `test the_frozen_v1_fixture_manifests_reparse_with_every_frozen_field_unchanged ... ok`. Commit:

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity add companion/crates/plugin/tests/fixtures/manifest_v1_aqi.toml companion/crates/plugin/tests/fixtures/manifest_v1_agenda.toml companion/crates/plugin/tests/manifest_v2.rs && git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -m "test: freeze v1 copies of the aqi and agenda manifests as plugin fixtures" -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>" -m "Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
  ```

- [ ] **Step 2: Write the failing parse-and-bound tests for `display_name`, `description`, the `summary` source, and the unknown-top-level-key guard.**

  Append to `mod tests` in `manifest.rs` (before its closing `}` at line 1621):

  ```rust
      // -- Plugin-parity Task 1: the v2 presentation keys. --

      /// A v2 header with `extra` inserted among the top-level keys, where
      /// TOML requires them to sit: before the first table header.
      fn v2_manifest_with(extra: &str) -> String {
          format!(
              "manifest_version = 2\nname = \"aqi\"\nversion = \"1.0.0\"\n{extra}\n\n\
               [source]\nkind = \"json\"\nurl = \"https://example.invalid/aqi.json\"\n\
               refresh_minutes = 15\n\n[template]\nkind = \"scene\"\n"
          )
      }

      #[test]
      fn a_v2_manifest_retains_all_three_presentation_keys() {
          let manifest = parse_manifest(&v2_manifest_with(
              "display_name = \"Air quality\"\ndescription = \"EPA index for a location\"\nsummary = \"{{ data.current.aqi }}\"",
          ))
          .expect("v2 presentation keys parse");
          assert_eq!(manifest.display_name.as_deref(), Some("Air quality"));
          assert_eq!(manifest.description.as_deref(), Some("EPA index for a location"));
          assert_eq!(manifest.summary.as_deref(), Some("{{ data.current.aqi }}"));
      }

      #[test]
      fn a_v2_manifest_may_omit_every_presentation_key() {
          let manifest = parse_manifest(&v2_manifest_with("")).expect("the keys are optional");
          assert_eq!(manifest.display_name, None);
          assert_eq!(manifest.description, None);
          assert_eq!(manifest.summary, None);
      }

      #[test]
      fn a_v1_manifest_rejects_each_presentation_key_by_name() {
          for (field, line) in [
              ("display_name", "display_name = \"Air quality\""),
              ("description", "description = \"EPA index\""),
              ("summary", "summary = \"{{ data.aqi }}\""),
          ] {
              let source = MINIMAL_HEADER.replacen("name =", &format!("{line}\nname ="), 1);
              assert_eq!(
                  parse_manifest(&source).unwrap_err(),
                  ManifestError::V2FieldInV1 { field },
                  "v1 must refuse {field}"
              );
          }
      }

      #[test]
      fn an_unknown_top_level_key_is_still_rejected_after_the_presentation_keys_exist() {
          // Spec §3: "Every table remains `deny_unknown_fields`." Adding three
          // OPTIONAL top-level keys is exactly the change that would tempt
          // someone to drop `deny_unknown_fields` from `RawPluginManifest`, and
          // dropping it fails silently -- a mistyped key would simply do
          // nothing. Each probe below is a near-miss of a real key chosen so it
          // is NOT a substring of any accepted key, so a message naming the
          // offender cannot be satisfied by the "expected one of ..." list.
          for key in ["display_naem", "descriptoin", "sumary", "bogus"] {
              let err = parse_manifest(&v2_manifest_with(&format!("{key} = \"x\""))).unwrap_err();
              match err {
                  ManifestError::Toml(message) => assert!(
                      message.contains(key),
                      "the {key} rejection must name the offending key; got: {message}"
                  ),
                  other => panic!("{key} must be refused as a TOML unknown field, got {other:?}"),
              }
          }
      }

      #[test]
      fn a_display_name_one_byte_past_its_cap_is_rejected_by_name() {
          let long = "a".repeat(MAX_DISPLAY_NAME_LEN + 1);
          let err = parse_manifest(&v2_manifest_with(&format!("display_name = \"{long}\"")))
              .unwrap_err();
          assert_eq!(
              err,
              ManifestError::StringTooLong {
                  field: "display_name",
                  limit: MAX_DISPLAY_NAME_LEN,
                  actual: MAX_DISPLAY_NAME_LEN + 1,
              }
          );
      }

      #[test]
      fn a_display_name_exactly_at_its_cap_is_accepted() {
          let exact = "a".repeat(MAX_DISPLAY_NAME_LEN);
          parse_manifest(&v2_manifest_with(&format!("display_name = \"{exact}\"")))
              .expect("64 bytes is the cap, not past it");
      }

      #[test]
      fn a_description_one_byte_past_its_cap_is_rejected_by_name() {
          let long = "a".repeat(MAX_DESCRIPTION_LEN + 1);
          let err = parse_manifest(&v2_manifest_with(&format!("description = \"{long}\"")))
              .unwrap_err();
          assert_eq!(
              err,
              ManifestError::StringTooLong {
                  field: "description",
                  limit: MAX_DESCRIPTION_LEN,
                  actual: MAX_DESCRIPTION_LEN + 1,
              }
          );
      }

      #[test]
      fn an_empty_display_name_or_description_is_rejected_by_name() {
          for (field, line) in [
              ("display_name", "display_name = \"\""),
              ("description", "description = \"\""),
          ] {
              assert_eq!(
                  parse_manifest(&v2_manifest_with(line)).unwrap_err(),
                  ManifestError::EmptyString { field }
              );
          }
      }

      #[test]
      fn a_summary_source_over_the_expression_source_cap_is_rejected_by_name() {
          let long = "a".repeat(MAX_EXPR_SOURCE_LEN + 1);
          let err = parse_manifest(&v2_manifest_with(&format!("summary = \"{long}\""))).unwrap_err();
          assert_eq!(
              err,
              ManifestError::StringTooLong {
                  field: "summary",
                  limit: MAX_EXPR_SOURCE_LEN,
                  actual: MAX_EXPR_SOURCE_LEN + 1,
              }
          );
      }
  ```

  Run:

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p plugin --lib 2>&1 | grep -E "^error" | head -20
  ```

  Expected: it does not compile — `error[E0609]: no field \`display_name\` on type \`PluginManifest\``, `error[E0599]: no variant or associated item named \`EmptyString\` found for enum \`ManifestError\``, and `error[E0425]: cannot find value \`MAX_DISPLAY_NAME_LEN\` in this scope` (and the same for `MAX_DESCRIPTION_LEN`).

- [ ] **Step 3: Implement the two string keys, the `summary` source bound, and the v1 gate.**

  In `manifest.rs`, after `MAX_URL_LEN` (line 69) add:

  ```rust
  /// Maximum byte length of a v2 `display_name`: the same bound as a card
  /// `title`, because it stands wherever a title would.
  pub const MAX_DISPLAY_NAME_LEN: usize = 64;
  /// Maximum byte length of a v2 `description`: the add-menu's second line.
  pub const MAX_DESCRIPTION_LEN: usize = 160;
  ```

  In `ManifestError` (after `IncompleteBoundLineGeometry,`, line 223) add:

  ```rust
      /// A bounded string field that must not be empty (`display_name`,
      /// `description`) was present and empty. Absent is fine -- the field is
      /// optional and falls back -- but present-and-empty is an authoring
      /// error, not a fallback request.
      EmptyString { field: &'static str },
      /// A v2 `summary` expression referenced the device-binding namespace
      /// (`time:`, `timer.`, `date`, `field.`) outside a string literal. The
      /// summary is evaluated on the server at refresh time, where none of
      /// those resolve, so this is a manifest error rather than a refresh-time
      /// failure that logs forever.
      SummaryUsesDeviceBinding { binding: String },
  ```

  In `PluginManifest` (after `pub template: Template,`, line 252) add:

  ```rust
      /// Manifest v2: what the card *is*, on every surface ("Air quality").
      /// `None` on v1 and on a v2 manifest that omits it; callers fall back to
      /// `name`, the identity key, which is never repurposed.
      pub display_name: Option<String>,
      /// Manifest v2: the add-menu's second line. `None` falls back to
      /// "Plugin · <version>" on the caller's side.
      pub description: Option<String>,
      /// Manifest v2: the tile's live value, held as opaque `{{ ... }}`
      /// expression source exactly like a text node's `value` -- evaluated by
      /// `summary::evaluate_summary` at refresh time, never by this module.
      pub summary: Option<String>,
  ```

  In `RawPluginManifest` (after `template: Option<Template>,`, line 308) add:

  ```rust
      #[serde(default)]
      display_name: Option<String>,
      #[serde(default)]
      description: Option<String>,
      #[serde(default)]
      summary: Option<String>,
  ```

  After `check_len` (which ends at line 576) add:

  ```rust
  fn check_non_empty(field: &'static str, value: &str) -> Result<(), ManifestError> {
      if value.is_empty() {
          Err(ManifestError::EmptyString { field })
      } else {
          Ok(())
      }
  }
  ```

  In `PluginManifest::validate`, after `check_len("version", &self.version, MAX_VERSION_LEN)?;` (line 661) add:

  ```rust
          if let Some(display_name) = &self.display_name {
              check_non_empty("display_name", display_name)?;
              check_len("display_name", display_name, MAX_DISPLAY_NAME_LEN)?;
          }
          if let Some(description) = &self.description {
              check_non_empty("description", description)?;
              check_len("description", description, MAX_DESCRIPTION_LEN)?;
          }
          if let Some(summary) = &self.summary {
              check_len("summary", summary, MAX_EXPR_SOURCE_LEN)?;
          }
  ```

  In `parse_manifest`, inside the `if manifest_version == ManifestVersion::V1 {` block, immediately after the `source.root` arm's closing `}` (line 886) add:

  ```rust
          for (field, value) in [
              ("display_name", &raw.display_name),
              ("description", &raw.description),
              ("summary", &raw.summary),
          ] {
              if value.is_some() {
                  return Err(ManifestError::V2FieldInV1 { field });
              }
          }
  ```

  and in the `PluginManifest { … }` literal (923-932) add the three fields after `template: …`, so the literal reads:

  ```rust
      let manifest = PluginManifest {
          manifest_version,
          name: raw.name,
          version: raw.version,
          source: raw.source,
          assets: raw.assets,
          nodes: raw.nodes,
          repeats: raw.repeats,
          template: raw.template.unwrap_or(Template::Scene),
          display_name: raw.display_name,
          description: raw.description,
          summary: raw.summary,
      };
  ```

  Every test-module `PluginManifest` literal in this crate now misses three fields; all ten lead with `manifest_version: ManifestVersion::V1,` (compile.rs 1021, 1464, 1515, 1588, 1690, 1755, 1786, 1882, 1941; assets.rs 358), so add the fields in one pass and check the count:

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && perl -0pi -e 's/^([ \t]+)manifest_version: ManifestVersion::V1,\n/$1manifest_version: ManifestVersion::V1,\n$1display_name: None,\n$1description: None,\n$1summary: None,\n/mg' crates/plugin/src/compile.rs crates/plugin/src/assets.rs && grep -c "summary: None," crates/plugin/src/compile.rs crates/plugin/src/assets.rs
  ```

  Expected: `crates/plugin/src/compile.rs:9` and `crates/plugin/src/assets.rs:1`. Then in `lib.rs` replace the `pub use manifest::{ … }` statement (lines 30-34) with:

  ```rust
  pub use manifest::{
      Align, Asset, Font, FontTier, Glyph, MAX_ASSETS, MAX_DESCRIPTION_LEN, MAX_DISPLAY_NAME_LEN,
      MAX_EXPANDED_SVG_BYTES, MAX_REFRESH_MINUTES, MAX_REPEAT_GROUPS, MAX_REPEAT_SOURCE_LEN,
      MAX_SOURCE_ROOT_LEN, MAX_SVG_SOURCE_BYTES, MIN_REFRESH_MINUTES, ManifestError, ManifestVersion,
      Node, PluginManifest, Point, Repeat, Source, Template, parse_manifest, parse_manifest_bytes,
  };
  ```

  Run:

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo fmt --all
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p plugin
  ```

  Expected: every test passes, including the nine new ones, and the frozen-v1 integration test still passes (its `FrozenV1Manifest` is `deny_unknown_fields` and the frozen copies carry none of the new keys). Mutation probe for the `deny_unknown_fields` guard (do not commit it): delete `#[serde(deny_unknown_fields)]` from `RawPluginManifest` (line 294) and confirm `an_unknown_top_level_key_is_still_rejected_after_the_presentation_keys_exist` fails with `display_naem must be refused as a TOML unknown field, got …`; restore it. Commit:

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity add companion/crates/plugin && git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -m "feat: parse and bound the manifest-v2 display_name, description and summary keys" -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>" -m "Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
  ```

- [ ] **Step 4: Write the failing tests for the summary's device-binding refusal.**

  Append to `mod tests` in `manifest.rs`:

  ```rust
      #[test]
      fn a_summary_naming_a_device_binding_is_rejected_by_name() {
          for (summary, binding) in [
              ("{{ field.title }}", "field.title"),
              ("{{ upper(timer.status) }}", "timer.status"),
              ("{{ time:angle:hour }}", "time:angle:hour"),
              ("{{ date }}", "date"),
              ("{{ default(data.x, field.title) }}", "field.title"),
          ] {
              let err = parse_manifest(&v2_manifest_with(&format!("summary = \"{summary}\"")))
                  .unwrap_err();
              assert_eq!(
                  err,
                  ManifestError::SummaryUsesDeviceBinding {
                      binding: binding.to_string()
                  },
                  "{summary}"
              );
          }
      }

      #[test]
      fn a_summary_reading_data_that_merely_looks_like_a_binding_is_accepted() {
          // `data.date` and `data.timer.status` are provider paths that read as
          // one dotted token, `data.events[0].date` is the case where a binding
          // WORD is a trailing path segment after an array index (agenda's own
          // shape), `"field.title"` is a string literal, and plain text is a
          // literal summary.
          for summary in [
              "{{ data.date }}",
              "{{ data.timer.status }}",
              "{{ data.events[0].date }}",
              r#"{{ \"field.title\" }}"#,
              r#"{{ default(data.dates, \"--\") }}"#,
              "Plain text",
          ] {
              parse_manifest(&v2_manifest_with(&format!("summary = \"{summary}\"")))
                  .unwrap_or_else(|err| panic!("{summary} must be accepted: {err:?}"));
          }
      }
  ```

  Run:

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p plugin --lib a_summary 2>&1 | grep -E "panicked|error\[|test result" | head
  ```

  Expected: `a_summary_naming_a_device_binding_is_rejected_by_name` panics with `called \`Result::unwrap_err()\` on an \`Ok\` value` (the variant exists since Step 3, so this is a runtime failure, not a compile error); the accepting test already passes.

- [ ] **Step 5: Implement the lexical device-binding scan, reusing the compiler's namespace predicate.**

  In `compile.rs`, change line 184 from `fn looks_like_binding_namespace(text: &str) -> bool {` to:

  ```rust
  pub(crate) fn looks_like_binding_namespace(text: &str) -> bool {
  ```

  One definition of the namespace, shared by construction — its own doc at 176-183 says why a second copy is the failure mode. In `manifest.rs` add the import after `use serde::Deserialize;` (line 24):

  ```rust
  use crate::compile::{ExpressionSource, classify_expression_source, looks_like_binding_namespace};
  ```

  and after `check_non_empty` add:

  ```rust
  /// Refuses a `summary` whose `{{ ... }}` body references the device-binding
  /// namespace anywhere outside a string literal. This is a lexical scan, not
  /// expression parsing (the grammar stays `expr.rs`'s, exactly as this
  /// module's doc promises): it reads identifier-shaped tokens
  /// (`[A-Za-z0-9_.:]`, so `timer.status` and `time:angle:hour` are one token
  /// each), skips `"..."` literals with the same [`skip_string`] the nesting
  /// pre-scan uses, and asks [`looks_like_binding_namespace`] -- the compiler's
  /// own predicate -- about every token that is not a trailing path segment
  /// (one preceded by `.`, such as the `date` in `data.events[0].date`). A
  /// literal summary cannot reference a binding, and a malformed pair is
  /// `summary::evaluate_summary`'s named error, at the same layer a text
  /// node's is.
  fn validate_summary_source(summary: &str) -> Result<(), ManifestError> {
      match classify_expression_source(summary) {
          ExpressionSource::Expression(inner) => match summary_device_binding(inner) {
              Some(binding) => Err(ManifestError::SummaryUsesDeviceBinding { binding }),
              None => Ok(()),
          },
          ExpressionSource::Literal(_) | ExpressionSource::MalformedPartial => Ok(()),
      }
  }

  fn summary_device_binding(inner: &str) -> Option<String> {
      fn is_token_byte(byte: u8) -> bool {
          byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':')
      }
      let bytes = inner.as_bytes();
      let mut i = 0;
      while i < bytes.len() {
          if bytes[i] == b'"' {
              i = skip_string(bytes, i, b'"');
          } else if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
              let start = i;
              while i < bytes.len() && is_token_byte(bytes[i]) {
                  i += 1;
              }
              // Token bytes are ASCII, so both ends of this slice are char
              // boundaries: `start` is an ASCII letter and `i` stops at the
              // first non-token byte, which is never a UTF-8 continuation byte.
              let token = &inner[start..i];
              let is_path_segment = start > 0 && bytes[start - 1] == b'.';
              if !is_path_segment && looks_like_binding_namespace(token) {
                  return Some(token.to_string());
              }
          } else {
              i += 1;
          }
      }
      None
  }
  ```

  In `validate`, change the summary arm added in Step 3 to:

  ```rust
          if let Some(summary) = &self.summary {
              check_len("summary", summary, MAX_EXPR_SOURCE_LEN)?;
              validate_summary_source(summary)?;
          }
  ```

  Also amend the module doc's "What this module does not do" paragraph. Replace lines 11-18:

  ```rust
  //! It does not parse the `{{ ... }}` expression syntax that appears inside a
  //! `value` or `glyph` field. That string is stored as bounded, opaque text
  //! -- length-checked, never interpreted -- because a later task owns the
  //! expression grammar. It also does not resolve assets to digests (a later
  //! task) or compile a manifest into a `Scene` (also a later task); it does
  //! not validate node geometry against the 448x368 canvas, because
  //! `protocol::validate_scene` is where that bound already lives and the
  //! compiler that produces a real `Scene` is what calls it.
  ```

  with:

  ```rust
  //! It does not parse the `{{ ... }}` expression syntax that appears inside a
  //! `value` or `glyph` field. That string is stored as bounded, opaque text
  //! -- length-checked, never interpreted -- because a later task owns the
  //! expression grammar. It also does not resolve assets to digests (a later
  //! task) or compile a manifest into a `Scene` (also a later task); it does
  //! not validate node geometry against the 448x368 canvas, because
  //! `protocol::validate_scene` is where that bound already lives and the
  //! compiler that produces a real `Scene` is what calls it. The one
  //! exception is `validate_summary_source`, a lexical token scan that still
  //! parses no grammar.
  ```

  Run:

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo fmt --all
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p plugin
  ```

  Expected: all pass, including both Step 4 tests. Mutation probe (do not commit it): delete the `!is_path_segment && ` guard in `summary_device_binding` and confirm `a_summary_reading_data_that_merely_looks_like_a_binding_is_accepted` fails on `{{ data.events[0].date }}` — that is the case where the array index `[0]` breaks the dotted token, leaving `date` as its own token; restore the guard. Commit:

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity add companion/crates/plugin && git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -m "feat: refuse a manifest-v2 summary that references a device binding" -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>" -m "Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
  ```

- [ ] **Step 6: Write the failing `evaluate_summary` integration tests.**

  Create `companion/crates/plugin/tests/summary.rs`:

  ```rust
  //! Plugin-parity Task 1: the manifest-v2 `summary` expression, evaluated the
  //! way `ServerProviderRefresher` will evaluate it at refresh time -- against
  //! the snapshot's raw value, into one bounded headline or nothing.

  use plugin::{ExprError, MAX_SUMMARY_LEN, SummaryError, evaluate_summary, parse_manifest};
  use providers::ProviderSnapshot;

  const AQI_FIXTURE: &str = include_str!("fixtures/aqi_response.json");

  fn v2_manifest(summary_line: &str) -> String {
      format!(
          "manifest_version = 2\nname = \"aqi\"\nversion = \"1.0.0\"\n{summary_line}\n\n\
           [source]\nkind = \"json\"\nurl = \"https://example.invalid/aqi.json\"\nrefresh_minutes = 15\n\n\
           [template]\nkind = \"scene\"\n"
      )
  }

  fn snapshot(value: serde_json::Value) -> ProviderSnapshot<serde_json::Value> {
      ProviderSnapshot {
          value,
          refreshed_at: None,
          age: None,
          stale: false,
          error: None,
      }
  }

  fn aqi_payload() -> serde_json::Value {
      let root: serde_json::Value = serde_json::from_str(AQI_FIXTURE).expect("fixture is valid JSON");
      assert_eq!(root["status"], "ok");
      root["payload"].clone()
  }

  #[test]
  fn an_undeclared_summary_evaluates_to_none() {
      let manifest = parse_manifest(&v2_manifest("")).expect("v2 without summary parses");
      assert_eq!(evaluate_summary(&manifest, &snapshot(aqi_payload())), Ok(None));
  }

  #[test]
  fn a_data_expression_evaluates_against_the_snapshot_value() {
      let manifest = parse_manifest(&v2_manifest("summary = \"{{ data.current.aqi }}\""))
          .expect("summary parses");
      assert_eq!(
          evaluate_summary(&manifest, &snapshot(aqi_payload())),
          Ok(Some("42".to_string()))
      );
  }

  #[test]
  fn a_literal_summary_is_returned_as_written() {
      let manifest = parse_manifest(&v2_manifest("summary = \"Outside\"")).expect("literal parses");
      assert_eq!(
          evaluate_summary(&manifest, &snapshot(serde_json::json!({}))),
          Ok(Some("Outside".to_string()))
      );
  }

  #[test]
  fn a_missing_value_evaluates_to_none_not_an_empty_string() {
      let manifest = parse_manifest(&v2_manifest("summary = \"{{ data.current.aqi }}\""))
          .expect("summary parses");
      assert_eq!(
          evaluate_summary(&manifest, &snapshot(serde_json::json!({}))),
          Ok(None)
      );
  }

  #[test]
  fn a_stale_error_snapshot_still_evaluates_its_last_good_value() {
      let manifest = parse_manifest(&v2_manifest("summary = \"{{ data.current.aqi }}\""))
          .expect("summary parses");
      let mut stale = snapshot(aqi_payload());
      stale.stale = true;
      stale.error = Some("upstream request timed out".to_string());
      assert_eq!(evaluate_summary(&manifest, &stale), Ok(Some("42".to_string())));
  }

  #[test]
  fn output_one_byte_past_the_cap_is_truncated_on_a_char_boundary() {
      let manifest =
          parse_manifest(&v2_manifest("summary = \"{{ data.headline }}\"")).expect("summary parses");
      let eleven_euros = "€".repeat(11); // 33 bytes: the 32-byte cap falls mid-character
      let data = serde_json::json!({ "headline": eleven_euros });
      let summary = evaluate_summary(&manifest, &snapshot(data))
          .expect("evaluates")
          .expect("declared");
      assert_eq!(summary, "€".repeat(10));
      assert!(summary.len() <= MAX_SUMMARY_LEN);
  }

  #[test]
  fn output_exactly_at_the_cap_is_kept_whole() {
      let exact = "a".repeat(MAX_SUMMARY_LEN);
      let manifest =
          parse_manifest(&v2_manifest(&format!("summary = \"{exact}\""))).expect("literal parses");
      assert_eq!(
          evaluate_summary(&manifest, &snapshot(serde_json::json!({}))),
          Ok(Some(exact))
      );
  }

  #[test]
  fn a_partial_interpolation_is_a_named_error() {
      let manifest = parse_manifest(&v2_manifest("summary = \"AQI {{ data.current.aqi }}\""))
          .expect("the shape parses; interpolation is judged at evaluation");
      assert_eq!(
          evaluate_summary(&manifest, &snapshot(aqi_payload())),
          Err(SummaryError::MalformedPartialInterpolation {
              text: "AQI {{ data.current.aqi }}".to_string()
          })
      );
  }

  #[test]
  fn an_unknown_identifier_is_a_named_expression_error() {
      let manifest = parse_manifest(&v2_manifest("summary = \"{{ bogus }}\"")).expect("shape parses");
      assert_eq!(
          evaluate_summary(&manifest, &snapshot(aqi_payload())),
          Err(SummaryError::Expression(ExprError::UnknownIdentifier {
              name: "bogus".to_string()
          }))
      );
  }
  ```

  Run:

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p plugin --test summary 2>&1 | head -12
  ```

  Expected failure: `error[E0432]: unresolved imports \`plugin::MAX_SUMMARY_LEN\`, \`plugin::SummaryError\`, \`plugin::evaluate_summary\``.

- [ ] **Step 7: Implement `summary.rs` and export it.**

  Create `companion/crates/plugin/src/summary.rs`:

  ```rust
  //! The manifest-v2 `summary` expression: the one value a plugin card shows
  //! on its complication tile, evaluated on the server at refresh time.
  //!
  //! This is deliberately a sibling of `compile.rs`'s text-node path, not a
  //! caller of it: a text node's value becomes a wire `SceneValue`, bounded by
  //! `protocol::MAX_SCENE_TEXT_LEN` and allowed to be a device binding; a
  //! summary becomes a plain provider `Field` string, bounded by
  //! [`MAX_SUMMARY_LEN`], and a device binding is refused at manifest parse
  //! time (`ManifestError::SummaryUsesDeviceBinding`) because nothing here could
  //! resolve one. What the two share -- the whole-value interpolation rule and
  //! the expression grammar -- they share by construction:
  //! [`classify_expression_source`] and [`Expr`].

  use std::fmt;

  use providers::ProviderSnapshot;

  use crate::compile::{ExpressionSource, classify_expression_source};
  use crate::expr::{EvalContext, Expr, ExprError, FUEL_BUDGET, Fuel};
  use crate::manifest::PluginManifest;

  /// Maximum byte length of an evaluated summary. A complication tile shows
  /// one short fact; anything longer is truncated on a char boundary, never
  /// refused -- a long headline is not a broken plugin.
  pub const MAX_SUMMARY_LEN: usize = 32;

  #[derive(Debug, Clone, PartialEq, Eq)]
  pub enum SummaryError {
      /// The summary contains mustache delimiters but is not exactly one
      /// complete `{{ ... }}` expression -- the same whole-value rule a text
      /// node's `value` follows.
      MalformedPartialInterpolation { text: String },
      /// The expression failed to parse or evaluate.
      Expression(ExprError),
  }

  impl fmt::Display for SummaryError {
      fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
          write!(f, "{self:?}")
      }
  }

  impl std::error::Error for SummaryError {}

  /// Evaluates `manifest.summary` against `snapshot.value`.
  ///
  /// Returns `Ok(None)` when the manifest declares no summary and when the
  /// expression evaluates to `EvalValue::Missing` (an absent field, a JSON
  /// `null`, a type mismatch): a missing headline is "no value", which the
  /// tile prints as an em dash, not an empty string it would print as
  /// nothing. A stale or error snapshot is evaluated like any other, because
  /// it still carries the last-good value (`providers::LastGood::complete`'s
  /// contract). `icon()` has no icon map here and evaluates to `Missing`; a
  /// codepoint is not a headline.
  ///
  /// # Errors
  ///
  /// See [`SummaryError`].
  pub fn evaluate_summary(
      manifest: &PluginManifest,
      snapshot: &ProviderSnapshot<serde_json::Value>,
  ) -> Result<Option<String>, SummaryError> {
      let Some(source) = manifest.summary.as_deref() else {
          return Ok(None);
      };
      let inner = match classify_expression_source(source) {
          ExpressionSource::Literal(literal) => return Ok(Some(bound_summary(literal))),
          ExpressionSource::Expression(inner) => inner,
          ExpressionSource::MalformedPartial => {
              return Err(SummaryError::MalformedPartialInterpolation {
                  text: source.to_string(),
              });
          }
      };
      let expr = Expr::parse(inner).map_err(SummaryError::Expression)?;
      let ctx = EvalContext::with_data(&snapshot.value);
      let mut fuel = Fuel::new(FUEL_BUDGET);
      let value = expr.eval(&ctx, &mut fuel).map_err(SummaryError::Expression)?;
      if value.is_missing() {
          return Ok(None);
      }
      Ok(Some(bound_summary(&value.to_string())))
  }

  fn bound_summary(text: &str) -> String {
      protocol::truncate_utf8_to_bytes(text, MAX_SUMMARY_LEN).to_owned()
  }
  ```

  In `lib.rs`: add `mod summary;` after `mod manifest;` (line 19); add, after the `pub use manifest::{ … };` statement,

  ```rust
  pub use summary::{MAX_SUMMARY_LEN, SummaryError, evaluate_summary};
  ```

  and extend the crate doc by appending one sentence to its last paragraph, so line 14 reads:

  ```rust
  //! nodes and asset fonts against a real one). The v2 `summary` key is
  //! evaluated separately by [`summary::evaluate_summary`] into the bounded
  //! headline a card's tile shows.
  ```

  Run:

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo fmt --all
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p plugin --test summary
  ```

  Expected `test result: ok. 9 passed`. Mutation probe (do not commit it): change `MAX_SUMMARY_LEN` to `33` and confirm `output_one_byte_past_the_cap_is_truncated_on_a_char_boundary` fails; restore. Commit:

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity add companion/crates/plugin && git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -m "feat: evaluate the manifest-v2 summary expression to a bounded headline" -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>" -m "Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
  ```

- [ ] **Step 8: Write the failing curated-plugin round-trip tests.**

  In `tests/curated_plugins.rs`, change line 16 to:

  ```rust
  use plugin::{compile_scene_with_assets, evaluate_summary, parse_manifest, resolve_assets};
  ```

  and append at the end of the file:

  ```rust
  // ---------------------------------------------------------------------------
  // Plugin-parity Task 1: every curated plugin is manifest v2 and presents
  // itself -- a display name, a description, and a summary that evaluates to
  // its headline against its own committed fixture.
  // ---------------------------------------------------------------------------

  const CLAUDE_LIMITS_FIXTURE: &str = include_str!("fixtures/claude_limits_response.json");

  fn curated_manifest(plugin_name: &str) -> plugin::PluginManifest {
      let dir = plugins_dir().join(plugin_name);
      let source = std::fs::read_to_string(dir.join("manifest.toml"))
          .unwrap_or_else(|error| panic!("read {plugin_name} manifest.toml: {error}"));
      parse_manifest(&source)
          .unwrap_or_else(|error| panic!("{plugin_name} manifest must parse: {error:?}"))
  }

  #[test]
  fn every_curated_plugin_is_v2_and_declares_all_three_presentation_keys() {
      for (plugin_name, display_name) in [
          ("aqi", "Air quality"),
          ("agenda", "Agenda"),
          ("claude-limits", "Claude limits"),
          ("svg-aqi", "Air quality (SVG)"),
      ] {
          let manifest = curated_manifest(plugin_name);
          assert!(manifest.is_manifest_v2(), "{plugin_name} must be manifest v2");
          assert_eq!(manifest.display_name.as_deref(), Some(display_name), "{plugin_name}");
          assert!(manifest.description.is_some(), "{plugin_name} needs a description");
          assert!(manifest.summary.is_some(), "{plugin_name} needs a summary");
      }
  }

  #[test]
  fn every_curated_summary_evaluates_to_its_headline_against_its_fixture() {
      // `svg-aqi` declares `[source] root = "payload"`, so the provider hands
      // it the same inner value `payload_from_envelope` extracts here. `aqi`
      // and `agenda` unwrap the fixture envelope for the reason recorded in
      // `aqi_fixture.rs:23-26`: the wrapper is a deliberate capture artifact,
      // not something either manifest binds.
      let cases = [
          ("aqi", payload_from_envelope(AQI_FIXTURE), "42"),
          ("svg-aqi", payload_from_envelope(AQI_FIXTURE), "42"),
          ("agenda", payload_from_envelope(AGENDA_FIXTURE), "09:00"),
          (
              "claude-limits",
              serde_json::from_str(CLAUDE_LIMITS_FIXTURE).expect("fixture is valid JSON"),
              "31",
          ),
      ];
      for (plugin_name, data, expected) in cases {
          let manifest = curated_manifest(plugin_name);
          assert_eq!(
              evaluate_summary(&manifest, &snapshot(data)),
              Ok(Some(expected.to_string())),
              "{plugin_name}"
          );
      }
  }
  ```

  Run:

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p plugin --test curated_plugins every_curated 2>&1 | grep -E "panicked|test result"
  ```

  Expected: both fail — the first with `aqi must be manifest v2`, the second with `assertion \`left == right\` failed: aqi` (`Ok(None)` against `Ok(Some("42"))`).

- [ ] **Step 9: Move the four curated manifests to v2 with the three keys, and prove the scenes did not move.**

  Replace `companion/plugins/aqi/manifest.toml` lines 19-25 with:

  ```toml
  manifest_version = 2
  name = "aqi"
  version = "1.0.0"
  # How the card presents itself in the companion app (manifest v2, plugin
  # parity): `display_name` is what the card IS on every surface,
  # `description` is the add-menu's second line, and `summary` is the tile's
  # live value -- evaluated on the server at every refresh and published as
  # the card's `hero` field. None of these touch the scene.
  display_name = "Air quality"
  description = "EPA index for a location"
  summary = "{{ data.current.aqi }}"

  [source]
  kind = "json"
  url = "https://example.invalid/aqi.json"
  refresh_minutes = 15

  [template]
  kind = "scene"
  ```

  Replace `companion/plugins/agenda/manifest.toml` lines 23-29 with:

  ```toml
  manifest_version = 2
  name = "agenda"
  version = "1.0.0"
  # Presentation keys (manifest v2, plugin parity) -- see aqi's manifest for
  # what each one is. The headline is the next event's start time.
  display_name = "Agenda"
  description = "The next events from a calendar feed"
  summary = "{{ data.events[0].time }}"

  [source]
  kind = "json"
  url = "https://example.invalid/agenda.json"
  refresh_minutes = 10

  [template]
  kind = "scene"
  ```

  Replace `companion/plugins/claude-limits/manifest.toml` lines 23-29 with:

  ```toml
  manifest_version = 2
  name = "claude-limits"
  version = "1.0.0"
  # Presentation keys (manifest v2, plugin parity) -- see aqi's manifest for
  # what each one is. The headline is the shortest window's used percentage.
  display_name = "Claude limits"
  description = "Session and weekly Claude Code usage"
  summary = "{{ round(data.windows[0].used_pct, 0) }}"

  [source]
  kind = "json"
  url = "https://deskmate.rodi.one/feeds/119fa3cca44bf0b9bb8c33f6/claude-usage.json"
  refresh_minutes = 10

  [template]
  kind = "scene"
  ```

  Replace the whole of `companion/plugins/svg-aqi/manifest.toml` with:

  ```toml
  manifest_version = 2
  name = "svg-aqi"
  version = "1.0.0"
  # Presentation keys (manifest v2, plugin parity) -- see aqi's manifest for
  # what each one is. `root = "payload"` below means the summary, like the
  # template, evaluates against the envelope's inner value.
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

  Run, in order, each on its own line so a failure names itself:

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p plugin
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p lvgl-sim --test plugin_scene --test claude_limits_scene
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity status --short companion/crates/lvgl-sim/tests/golden
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p server plugin_registry
  ```

  Expected: the plugin crate passes with both Step 8 tests green; `plugin_scene_goldens_match`, `every_plugin_state_has_a_case` (still 16 rows) and `claude_limits_goldens_match` pass **without `BLESS`** — those goldens are compiled from these very manifests through the real `parse_manifest` + `compile_scene_with_assets` path (`crates/lvgl-sim/src/cases.rs:1737-1768`, `compile_plugin_scene`), so a green run is the proof that `manifest_version`, `[template] kind = "scene"` and the three keys change nothing about the scene; the `git status` line prints nothing (no golden was rewritten); and `real_curated_plugins_load_with_their_real_assets` still loads `["agenda", "aqi", "claude-limits", "svg-aqi"]` with no failures. If a golden differs, stop: that is a real compile-path change, not a blessing job. Commit:

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity add companion/plugins companion/crates/plugin/tests/curated_plugins.rs && git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -m "feat: move the four curated plugins to manifest v2 with presentation keys" -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>" -m "Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
  ```

- [ ] **Step 10: Amend the frozen docs (this task owns `manifest-v2.md`).**

  In `docs/plugins/manifest-v2.md`:

  (a) In the top-level shape example, replace lines 14-16 with:

  ```toml
  manifest_version = 2
  name = "svg-aqi"
  version = "1.0.0"
  display_name = "Air quality (SVG)"
  description = "EPA index for a location, drawn from an SVG template"
  summary = "{{ data.current.aqi }}"
  ```

  (b) Replace the sentence at lines 31-32 —

  ```markdown
  node-count budget, repeat budget, stale/error footer, and asset resolution rules remain
  unchanged. V2 adds only the explicit discriminator, source roots, the template union, and
  the two authorable binding positions below.
  ```

  — with:

  ```markdown
  node-count budget, repeat budget, stale/error footer, and asset resolution rules remain
  unchanged. V2 adds only the explicit discriminator, source roots, the template union, the
  two authorable binding positions below, and the three optional presentation keys.
  ```

  (c) Insert this section between "## Top-level shape" (which ends at line 34) and "## `[source].root`" (line 36):

  ```markdown
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
  ```

  (d) Replace "## V1 compatibility" (lines 138-149) with:

  ```markdown
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
  ```

  In `docs/plugins/manifest-v1.md` replace lines 14-16 —

  ```markdown
  The two curated plugins that ship today, `companion/plugins/aqi/manifest.toml` and
  `companion/plugins/agenda/manifest.toml`, are worked examples of everything in this
  document; read them alongside it.
  ```

  — with:

  ```markdown
  The v1 worked examples are the byte-exact copies of the `aqi` and `agenda` manifests as
  they shipped under v1, `companion/crates/plugin/tests/fixtures/manifest_v1_aqi.toml` and
  `companion/crates/plugin/tests/fixtures/manifest_v1_agenda.toml`; the curated plugins
  under `companion/plugins/` moved to manifest v2 on 2026-09-07 and differ from those
  copies only by the v2 header keys (`manifest-v2.md`). Read the copies alongside this
  document.
  ```

  Verify each new name landed in the document that owns it:

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity && for name in manifest_v1_aqi.toml manifest_v1_agenda.toml display_name description summary SummaryUsesDeviceBinding EmptyString MAX_DISPLAY_NAME_LEN MAX_DESCRIPTION_LEN MAX_SUMMARY_LEN deny_unknown_fields; do printf '%-26s %s\n' "$name" "$(grep -rl -- "$name" docs/plugins/ | tr '\n' ' ')"; done
  ```

  Expected: `manifest_v1_aqi.toml` and `manifest_v1_agenda.toml` each list **both** `docs/plugins/manifest-v1.md` and `docs/plugins/manifest-v2.md`; `SummaryUsesDeviceBinding`, `EmptyString`, `MAX_DISPLAY_NAME_LEN`, `MAX_DESCRIPTION_LEN`, `MAX_SUMMARY_LEN` and `deny_unknown_fields` each list at least `docs/plugins/manifest-v2.md`; no name has an empty right-hand column. Commit:

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity add docs/plugins/manifest-v2.md docs/plugins/manifest-v1.md && git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -m "docs: amend manifest v2 with the presentation keys and the frozen v1 fixtures" -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>" -m "Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
  ```

- [ ] **Step 11: Run the workspace gates.**

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo fmt --all --check
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test --workspace --all-targets
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test --workspace --doc
  ```

  Expected: all four exit 0. Run each on its own line and read its exit status directly — do not pipe cargo into `tail` or `head` for a gate, which reports the pipeline's status and hides a failure as a pass. `server`'s `hostile_device.rs` asserts the exact asset-sync request sequence a device sees; it stays green because no asset byte moved. Nothing to commit if green; if `fmt`/`clippy` touch anything, amend into the step's commit that introduced it.

**Task exit:** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --all-targets` and `cargo test --workspace --doc` all green from `companion/`; `cargo test -p lvgl-sim --test plugin_scene --test claude_limits_scene` green **without `BLESS`** and `git status --short companion/crates/lvgl-sim/tests/golden` empty (the three keys do not touch the scene); `cargo test -p server plugin_registry` loads all four curated plugins with no failures; the three mutation probes (Step 3's `deny_unknown_fields`, Step 5's `!is_path_segment` guard, Step 7's `MAX_SUMMARY_LEN`) were performed and reverted; six commits on `feat/plugin-parity`, none touching `protocol`, `app-core`, the server routes or the Mac app.

---

### Task 2: Shared admin DTOs, the honest hostless runtime, and the preview runtime command

**Files:**

- Modify `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/crates/app-core/src/admin.rs` (37 lines today — `AdminConfigErrorBody` and one round-trip test)
- Modify `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/crates/app-core/src/lib.rs` (`pub use admin::AdminConfigErrorBody;` at line 14; `pub use runtime::{…}` at lines 48-53)
- Modify `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/crates/app-core/src/commands.rs` (106 lines — `RuntimeError` at 20-30, its `Display` at 32-50, `CommandReply` at 54, `RuntimeCommand` at 56-106; no test module exists yet)
- Modify `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/crates/app-core/src/runtime.rs` (`use crate::{…}` 30-41; `PluginHost`/`RasterRequest`/`RasterFrame` 134-180; `start_with_plugin_host` 780-828 with `initial_snapshot(&config, …)` at 793; `RuntimeHandle::request` 959-981; `WorkerState` 997-1046; `WorkerState::new` 1049-1084; `replace_config`'s combined provider arm 1146-1171; `run_runtime` 1417-1439 with `state.plugin_host = plugin_host;` at 1438; `process_command`'s `InjectPluginSnapshot` arm 1588-1597; `build_card_scene` 2674-2737 with its `validate_message` at 2734-2735; `raster_request` 3267-3283; `initial_snapshot` 3633-3681; the in-crate `mod tests` from 3708, helpers `rotation_clock_card` 3822, `plugin_card` 3858, `rotation_config` 3869, `fresh_scheduler` 4514)
- Modify `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/crates/app-core/tests/runtime.rs` (`use app_core::{…}` 7-15; `FakePluginHostState` 668-674; `FakePluginHostControl` 676-701; `impl PluginHost for FakePluginHost` 707-737; `plugin_config` 842-857; `start_plugin_runtime` 867-879; `start_runtime` 881-889; `wait_for_snapshot` 891-905; `wait_for` 907-913; the plugin tests 1150-1401)
- Modify `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate/src-tauri/src/commands.rs` (`From<RuntimeError> for IpcError` at 1265-1291, its `NotFound` arm at 1280-1284)
- Modify `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate/src-tauri/src/lib.rs` (`runtime_error_log_label` at 288-301)

This task does **not** touch `companion/crates/server/`. Every HTTP status this project maps is Task 3's.

**Interfaces:**

*Consumes* — `protocol::validate_scene(&Scene) -> Result<(), MessageError>` (`protocol/src/scene.rs:634`, re-exported at `protocol/src/lib.rs:44`); `protocol::validate_message(&Message)` (rejects `PushScene { revision: 0 }` with `MessageError::InvalidValue("scene revision")`, `protocol/src/message.rs:902-904`); `crate::runtime::raster_request(&CardCandidate, &[Field]) -> RasterRequest` (`runtime.rs:3267`); `crate::runtime::build_card_scene(&AppConfig, &str, &[Field], Option<&providers::ProviderSnapshot<serde_json::Value>>, Option<&mut dyn PluginHost>, u32) -> Result<CardCandidate, String>` (`runtime.rs:2674`); `PluginHost::rasterize(&mut self, &RasterRequest) -> Result<RasterFrame, String>` (`runtime.rs:177`); `providers::ProviderSnapshot<T> { value, refreshed_at, age, stale, error }` (`providers/src/lib.rs:24-30`).

*Produces* —

```rust
// app-core/src/admin.rs — all seven DTOs are created here and nowhere else
pub struct PluginCatalog { pub plugins: Vec<PluginCatalogEntry>, pub load_failures: Vec<PluginLoadFailure> }
pub struct PluginCatalogEntry { pub id: String, pub name: String, pub version: String,
    pub node_count: usize, pub assets: Vec<PluginCatalogAsset>, pub display_name: Option<String>,
    pub description: Option<String>, pub manifest_version: u8, pub template: PluginTemplateKind,
    pub refresh_minutes: u16 }
pub struct PluginCatalogAsset { pub file: String, pub kind: String, pub byte_length: usize, pub digest: String }
pub struct PluginLoadFailure { pub id: String, pub error: String }
pub enum PluginTemplateKind { DisplayList, Svg }          // "display-list" | "svg"
pub enum CardPreviewState { Fresh, Stale, Error, Waiting } // lowercase
pub struct CardPreviewResponse { pub png_base64: Option<String>, pub state: CardPreviewState,
    pub message: Option<String>, pub refreshed_at_unix_ms: Option<u64> }

// app-core/src/lib.rs re-exports six of the seven. `PluginLoadFailure` is reachable ONLY as
// `app_core::admin::PluginLoadFailure`, because `server::plugin_registry::PluginLoadFailure`
// already exists and a second one in the crate root's scope is a trap.

// app-core/src/commands.rs
RuntimeError::UnknownCard { card_id: String }
RuntimeError::NotAPluginCard { card_id: String }
pub(crate) type PreviewReply = SyncSender<Result<crate::runtime::CardPreview, RuntimeError>>;
RuntimeCommand::RenderCardPreview { card_id: String, reply: PreviewReply }

// app-core/src/runtime.rs
pub const PREVIEW_SCENE_REVISION: u32 = 0;
pub struct CardPreview { pub frame: Option<RasterFrame>, pub state: CardPreviewState,
    pub message: Option<String>, pub refreshed_at_unix_ms: Option<u64> }
impl RuntimeHandle { pub fn render_card_preview(&self, card_id: &str) -> Result<CardPreview, RuntimeError> }
fn WorkerState::new_with_plugin_host(AppConfig, Instant, &mut Scheduler, Option<Box<dyn PluginHost>>) -> Self
fn initial_snapshot(&AppConfig, RuntimeDiagnostics, renders_plugin_cards: bool) -> AppSnapshot
```

---

- [ ] **Step 1: Write the failing DTO tests.** Append inside the existing `mod tests` block in `companion/crates/app-core/src/admin.rs`, after `invalid_config_body_round_trips_between_server_and_clients`:

```rust
    fn catalog_entry() -> PluginCatalogEntry {
        PluginCatalogEntry {
            id: "aqi".into(),
            name: "aqi".into(),
            version: "1.0.0".into(),
            node_count: 4,
            assets: vec![PluginCatalogAsset {
                file: "icons.ttf".into(),
                kind: "icon-font".into(),
                byte_length: 12,
                digest: "ab12".into(),
            }],
            display_name: Some("Air quality".into()),
            description: None,
            manifest_version: 2,
            template: PluginTemplateKind::DisplayList,
            refresh_minutes: 15,
        }
    }

    /// The first five keys, in this order, are byte-for-byte what the server's
    /// private `PluginResponse` already serializes (`server/src/admin.rs:95-101`).
    /// The five that follow are additive, so an old consumer reads the same
    /// bytes it read before. Pin the whole string: a reordering is a silent
    /// break for nothing.
    #[test]
    fn a_catalog_entry_keeps_the_shipped_keys_and_adds_the_new_ones_after_them() {
        assert_eq!(
            serde_json::to_string(&catalog_entry()).unwrap(),
            r#"{"id":"aqi","name":"aqi","version":"1.0.0","node_count":4,"assets":[{"file":"icons.ttf","kind":"icon-font","byte_length":12,"digest":"ab12"}],"display_name":"Air quality","description":null,"manifest_version":2,"template":"display-list","refresh_minutes":15}"#
        );
    }

    #[test]
    fn a_catalog_round_trips_between_the_server_and_the_mac() {
        let catalog = PluginCatalog {
            plugins: vec![catalog_entry()],
            load_failures: vec![PluginLoadFailure {
                id: "broken".into(),
                error: "manifest is not valid TOML".into(),
            }],
        };
        let json = serde_json::to_string(&catalog).unwrap();
        assert_eq!(
            json,
            format!(
                r#"{{"plugins":[{}],"load_failures":[{{"id":"broken","error":"manifest is not valid TOML"}}]}}"#,
                serde_json::to_string(&catalog_entry()).unwrap()
            )
        );
        assert_eq!(serde_json::from_str::<PluginCatalog>(&json).unwrap(), catalog);
    }

    #[test]
    fn an_svg_template_and_every_preview_state_serialize_as_the_wire_words() {
        assert_eq!(
            serde_json::to_string(&PluginTemplateKind::Svg).unwrap(),
            r#""svg""#
        );
        for (state, word) in [
            (CardPreviewState::Fresh, r#""fresh""#),
            (CardPreviewState::Stale, r#""stale""#),
            (CardPreviewState::Error, r#""error""#),
            (CardPreviewState::Waiting, r#""waiting""#),
        ] {
            assert_eq!(serde_json::to_string(&state).unwrap(), word);
        }
    }

    /// Spec 4.2's invariant, expressed as bytes: a frame exactly when the state
    /// is fresh or stale, a message exactly when it is error or waiting.
    #[test]
    fn a_waiting_preview_carries_a_message_and_no_frame() {
        let response = CardPreviewResponse {
            png_base64: None,
            state: CardPreviewState::Waiting,
            message: Some("Waiting for the first refresh".into()),
            refreshed_at_unix_ms: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        assert_eq!(
            json,
            r#"{"png_base64":null,"state":"waiting","message":"Waiting for the first refresh","refreshed_at_unix_ms":null}"#
        );
        assert_eq!(
            serde_json::from_str::<CardPreviewResponse>(&json).unwrap(),
            response
        );
    }
```

- [ ] **Step 2: Run it and watch it fail to compile.** `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p app-core --lib admin::tests` — expect `error[E0422]: cannot find struct, variant or union type 'PluginCatalogEntry' in this scope`, and the same for `PluginCatalogAsset`, `PluginCatalog`, `PluginLoadFailure`, `PluginTemplateKind`, `CardPreviewState`, `CardPreviewResponse`.

- [ ] **Step 3: Add the seven DTOs.** Insert into `companion/crates/app-core/src/admin.rs` between the `AdminConfigErrorBody` enum and `#[cfg(test)]`:

```rust
/// The admin API's plugin registry, serialized by the server and deserialized
/// by the Mac app. One type, so the two cannot drift; the first five entry
/// fields keep the shipped key order and the rest are additive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginCatalog {
    pub plugins: Vec<PluginCatalogEntry>,
    pub load_failures: Vec<PluginLoadFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginCatalogEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub node_count: usize,
    pub assets: Vec<PluginCatalogAsset>,
    /// Manifest v2's `display_name`. `None` on a v1 manifest; the reader falls
    /// back to `id` rather than inventing a name here.
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub manifest_version: u8,
    pub template: PluginTemplateKind,
    pub refresh_minutes: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginCatalogAsset {
    pub file: String,
    pub kind: String,
    pub byte_length: usize,
    pub digest: String,
}

/// Deliberately NOT re-exported from the crate root: the server already owns a
/// `plugin_registry::PluginLoadFailure`, and two same-named types in one scope
/// is a trap. Reach this one as `app_core::admin::PluginLoadFailure`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginLoadFailure {
    pub id: String,
    pub error: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginTemplateKind {
    DisplayList,
    Svg,
}

/// What a card's preview stage shows. A frame exists exactly for `Fresh` and
/// `Stale`; a message exists exactly for `Error` and `Waiting`. `Waiting` is
/// the ordinary pre-first-fetch state and must stay distinguishable from a
/// fault -- it prints its own state word, never the "No data yet" badge, which
/// means "a real frame rendered from sample data".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CardPreviewState {
    Fresh,
    Stale,
    Error,
    Waiting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardPreviewResponse {
    pub png_base64: Option<String>,
    pub state: CardPreviewState,
    pub message: Option<String>,
    pub refreshed_at_unix_ms: Option<u64>,
}
```

Then replace line 14 of `companion/crates/app-core/src/lib.rs`:

```rust
pub use admin::{
    AdminConfigErrorBody, CardPreviewResponse, CardPreviewState, PluginCatalog, PluginCatalogAsset,
    PluginCatalogEntry, PluginTemplateKind,
};
```

- [ ] **Step 4: Run to pass.** `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p app-core --lib admin::tests` — 5 passed (the pre-existing round-trip plus the four new ones).

- [ ] **Step 5: Prove the key-order pin bites (mutation probe).** Temporarily swap the `display_name` and `description` field declarations in `PluginCatalogEntry`, then run `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p app-core --lib admin::tests::a_catalog_entry_keeps_the_shipped_keys_and_adds_the_new_ones_after_them` — it must fail on the pinned string. Restore the order.

- [ ] **Step 6: Commit.**

```sh
git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity add companion/crates/app-core/src/admin.rs companion/crates/app-core/src/lib.rs && git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -m "feat: give the admin plugin catalog and card preview one shared DTO" -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
```

---

- [ ] **Step 7: Write the failing hostless-runtime test.** In `companion/crates/app-core/tests/runtime.rs`, insert this refresher immediately after `PluginRefresher`'s `impl CalendarRefresher` block (which ends at line 658):

```rust
struct CountingRefresher {
    calls: Arc<AtomicU64>,
}

impl CalendarRefresher for CountingRefresher {
    fn refresh(&mut self, request: CalendarRefreshRequest) -> CalendarRefreshResult {
        self.calls.fetch_add(1, Ordering::Relaxed);
        CalendarRefreshResult {
            generation: request.generation,
            widget_id: request.widget_id,
            fields: Vec::new(),
            value: None,
            refreshed_at: None,
            age: None,
            stale: false,
            error: None,
        }
    }
}
```

and append this test immediately after `plugin_card_without_an_injected_host_is_refused_visibly` (which ends at line 1348):

```rust
/// Spec 5.3. A runtime with no plugin host can neither fetch nor draw a plugin
/// card, so it must say nothing about one rather than faking a refresh. The old
/// behaviour published a permanent `stale` flag carrying the refresher's own
/// internal message -- a defect presented as a state, the same shape the V1
/// validation-mislabeling fix exists to forbid. The Mac projects the server's
/// real provider state over this gap instead.
#[test]
fn a_hostless_runtime_neither_refreshes_nor_reports_a_plugin_card() {
    let control = MockDeviceControl::default();
    let refreshes = Arc::new(AtomicU64::new(0));
    let runtime = RuntimeHandle::start(
        plugin_config(),
        Box::new(MockDevice::new(control.clone())),
        Box::new(CountingRefresher {
            calls: Arc::clone(&refreshes),
        }),
        options(),
    )
    .unwrap();

    let snapshot = wait_for_snapshot(&runtime, Duration::from_secs(1), |snapshot| {
        snapshot.device.connection == ConnectionState::Online
    });
    assert!(
        snapshot.providers.is_empty(),
        "a runtime that cannot fetch a plugin card must not invent a provider state for it"
    );
    assert!(
        !snapshot
            .card_data
            .iter()
            .any(|data| data.card_id == "plugin-card"),
        "the compiled placeholder fields are not data; do not publish them as a card's value"
    );

    thread::sleep(Duration::from_millis(80));
    assert_eq!(
        refreshes.load(Ordering::Relaxed),
        0,
        "no provider deadline may be scheduled for a card this runtime cannot render"
    );
    let settled = runtime.snapshot().unwrap();
    assert!(settled.providers.is_empty());
    runtime.shutdown().unwrap();
}
```

- [ ] **Step 8: Run it and watch it fail on the assertion.** `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p app-core --test runtime a_hostless_runtime_neither_refreshes_nor_reports_a_plugin_card` — expect `assertion failed: snapshot.providers.is_empty()`. Today `replace_config` (`runtime.rs:1146-1171`) schedules a plugin card exactly like a weather card, so one `ProviderSnapshot` is published and `CountingRefresher` is called.

- [ ] **Step 9: Install the plugin host before the first `replace_config`.** In `companion/crates/app-core/src/runtime.rs`, replace `WorkerState::new` (lines 1049-1084) with:

```rust
    fn new(config: AppConfig, now: Instant, scheduler: &mut Scheduler) -> Self {
        Self::new_with_plugin_host(config, now, scheduler, None)
    }

    /// The host must be installed before the first `replace_config`: whether
    /// this runtime schedules and reports a plugin card at all is decided
    /// there, and a host assigned afterwards would arrive one config too late.
    fn new_with_plugin_host(
        config: AppConfig,
        now: Instant,
        scheduler: &mut Scheduler,
        plugin_host: Option<Box<dyn PluginHost>>,
    ) -> Self {
        let mut state = Self {
            config: config.clone(),
            runtime: RuntimeState::Starting,
            device: empty_device(ConnectionState::Connecting),
            persistence: PersistenceState::Clean,
            latest_fields: BTreeMap::new(),
            plugin_snapshots: BTreeMap::new(),
            plugin_host,
            dirty_widgets: BTreeSet::new(),
            push_rejections: BTreeMap::new(),
            pomodoros: BTreeMap::new(),
            pomodoro_snapshots: BTreeMap::new(),
            providers: BTreeMap::new(),
            interrupts: InterruptArbiter::default(),
            armed_event_alerts: BTreeMap::new(),
            active_screen: None,
            active_screen_dirty: false,
            active_scene_dirty: false,
            connected: false,
            ever_connected: false,
            needs_full_sync: true,
            ownership_refused: false,
            generation: 0,
            next_scene_revision: 0,
            confirmed_durable_assets: BTreeSet::new(),
            confirmed_volatile_assets: BTreeSet::new(),
            active_volatile_digest: None,
            pending_raster_card: None,
            next_connect: now,
            last_published: None,
        };
        state.replace_config(config, now, scheduler);
        state.runtime = RuntimeState::Starting;
        state
    }
```

and in `run_runtime` replace lines 1437-1438:

```diff
-    let mut state = WorkerState::new(config, now, &mut scheduler);
-    state.plugin_host = plugin_host;
+    let mut state = WorkerState::new_with_plugin_host(config, now, &mut scheduler, plugin_host);
```

- [ ] **Step 10: Split the combined provider arm in `replace_config`.** In `companion/crates/app-core/src/runtime.rs`, replace lines 1146-1171 (the `Calendar | Weather | JsonFeed | Rss | Plugin` arm) with two arms:

```rust
                CardSettings::Plugin { id, refresh, .. } => {
                    // Spec 5.3: with no plugin host this runtime can neither
                    // fetch nor draw the card, so it schedules nothing and
                    // reports nothing about it -- including the compiled
                    // placeholder fields, which are a stand-in for data, not
                    // data. Dropping the carried-over snapshot with them is
                    // correct: there is no one here to render it.
                    if self.plugin_host.is_none() {
                        self.latest_fields.remove(id);
                        continue;
                    }
                    let interval = refresh
                        .interval_minutes()
                        .map(|minutes| Duration::from_secs(u64::from(minutes) * 60));
                    provider_deadlines.push((id.clone(), interval));
                    let unchanged = previous_config
                        .cards
                        .iter()
                        .any(|previous| previous == card);
                    self.restore_provider(
                        id,
                        unchanged,
                        &mut previous_fields,
                        &mut previous_providers,
                    );
                    if unchanged && let Some(snapshot) = previous_plugin_snapshots.remove(id) {
                        self.plugin_snapshots.insert(id.clone(), snapshot);
                    }
                }
                CardSettings::Calendar { id, refresh, .. }
                | CardSettings::Weather { id, refresh, .. }
                | CardSettings::JsonFeed { id, refresh, .. }
                | CardSettings::Rss { id, refresh, .. } => {
                    let interval = refresh
                        .interval_minutes()
                        .map(|minutes| Duration::from_secs(u64::from(minutes) * 60));
                    provider_deadlines.push((id.clone(), interval));
                    let unchanged = previous_config
                        .cards
                        .iter()
                        .any(|previous| previous == card);
                    self.restore_provider(
                        id,
                        unchanged,
                        &mut previous_fields,
                        &mut previous_providers,
                    );
                }
```

- [ ] **Step 11: Make the pre-worker publication agree with the worker.** In `companion/crates/app-core/src/runtime.rs`, change `initial_snapshot`'s signature (line 3633) and its provider arm (lines 3653-3662):

```rust
fn initial_snapshot(
    config: &AppConfig,
    diagnostics: RuntimeDiagnostics,
    renders_plugin_cards: bool,
) -> AppSnapshot {
```

```rust
            CardSettings::Plugin { id, .. } if renders_plugin_cards => {
                providers.push(ProviderSnapshot {
                    widget_id: id.clone(),
                    state: ProviderState::Idle,
                    last_success_unix_ms: None,
                    age_seconds: None,
                });
            }
            // Matches the worker: a runtime with no host says nothing at all
            // about a plugin card, not even "idle".
            CardSettings::Plugin { .. } => {}
            CardSettings::Calendar { id, .. }
            | CardSettings::Weather { id, .. }
            | CardSettings::JsonFeed { id, .. }
            | CardSettings::Rss { id, .. } => providers.push(ProviderSnapshot {
                widget_id: id.clone(),
                state: ProviderState::Idle,
                last_success_unix_ms: None,
                age_seconds: None,
            }),
```

and its one call site in `start_with_plugin_host` (lines 792-793), computed before `plugin_host` is moved into `worker_inputs`:

```diff
+        let renders_plugin_cards = plugin_host.is_some();
         let diagnostics = Arc::new(RuntimeDiagnosticCounters::default());
-        let initial = initial_snapshot(&config, diagnostics.snapshot());
+        let initial = initial_snapshot(&config, diagnostics.snapshot(), renders_plugin_cards);
```

- [ ] **Step 12: Run to pass, including the neighbours the change must not disturb.** `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p app-core --all-targets` — the new test passes, and so do `plugin_card_without_a_cached_snapshot_is_refused_visibly` (its runtime *has* a host, so `snapshot.providers[0]` still exists), `plugin_card_without_an_injected_host_is_refused_visibly` (the hostless `SceneRefused` still fires, because `push_active_scene` builds the active card regardless of provider scheduling), `plugin_provider_request_preserves_card_and_plugin_identity` (`provider_request` reads `state.config.cards`, not `state.providers`) and the four in-crate asset tests that assign `state.plugin_host` after `WorkerState::new`. A fake state was deleted and nothing else.

- [ ] **Step 13: Prove the guard bites (mutation probe).** Delete the `if self.plugin_host.is_none() { … }` block from the new `CardSettings::Plugin` arm and run `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p app-core --test runtime a_hostless_runtime_neither_refreshes_nor_reports_a_plugin_card` — it must fail. Restore the guard.

- [ ] **Step 14: Commit.**

```sh
git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity add companion/crates/app-core && git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -m "fix: stop a hostless runtime from faking a plugin card's refresh and state" -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
```

---

- [ ] **Step 15: Write the failing test for the two new runtime errors.** Append a new module at the end of `companion/crates/app-core/src/commands.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Two distinct refusals, kept apart on purpose: one card does not exist,
    /// the other exists and is the wrong kind. Folding them into
    /// `UnknownWidget` would make a plugin-id typo and a clock card read
    /// identically in the log and on the wire. What HTTP status each becomes
    /// is the server's business, not app-core's.
    #[test]
    fn the_preview_card_errors_are_typed_and_name_the_card() {
        let unknown = RuntimeError::UnknownCard {
            card_id: "aqi".into(),
        };
        assert_eq!(unknown.to_string(), "unknown card \"aqi\"");
        assert_eq!(
            serde_json::to_string(&unknown).unwrap(),
            r#"{"kind":"unknown-card","card_id":"aqi"}"#
        );

        let wrong_kind = RuntimeError::NotAPluginCard {
            card_id: "clock".into(),
        };
        assert_eq!(wrong_kind.to_string(), "card \"clock\" is not a plugin card");
        assert_eq!(
            serde_json::to_string(&wrong_kind).unwrap(),
            r#"{"kind":"not-a-plugin-card","card_id":"clock"}"#
        );
    }
}
```

- [ ] **Step 16: Run it and watch it fail to compile.** `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p app-core --lib commands::tests` — expect `error[E0599]: no variant or associated item named 'UnknownCard' found for enum 'RuntimeError'`.

- [ ] **Step 17: Add the variants and the two exhaustive matches this task owns.** In `companion/crates/app-core/src/commands.rs`, add to `RuntimeError` after `UnknownScreen { screen_id: String },` (line 27):

```rust
    UnknownCard { card_id: String },
    NotAPluginCard { card_id: String },
```

and to its `Display`, after the `UnknownScreen` arm (line 45):

```rust
            Self::UnknownCard { card_id } => write!(formatter, "unknown card {card_id:?}"),
            Self::NotAPluginCard { card_id } => {
                write!(formatter, "card {card_id:?} is not a plugin card")
            }
```

In `companion/apps/deskmate/src-tauri/src/commands.rs`, widen the `NotFound` arm at lines 1280-1284:

```rust
            RuntimeError::UnknownWidget { .. }
            | RuntimeError::UnknownScreen { .. }
            | RuntimeError::UnknownCard { .. }
            | RuntimeError::NotAPluginCard { .. } => Self::NotFound {
                message: error.to_string(),
            },
```

In `companion/apps/deskmate/src-tauri/src/lib.rs`, add to `runtime_error_log_label` after the `UnknownScreen` arm (line 295):

```rust
        app_core::RuntimeError::UnknownCard { .. } => "unknown-card",
        app_core::RuntimeError::NotAPluginCard { .. } => "not-a-plugin-card",
```

- [ ] **Step 18: Run to pass, and record the one match left open on purpose.** `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p app-core --lib commands::tests && /Users/rodion/.cargo/bin/cargo check -p deskmate-app --all-targets`. From here `cargo check -p server` fails with `error[E0004]: non-exhaustive patterns: 'RuntimeError::UnknownCard { .. }' and 'RuntimeError::NotAPluginCard { .. }' not covered` at `companion/crates/server/src/admin.rs:1050`. That is expected and deliberate: **Task 3 owns every HTTP status this project maps**, and it closes that conversion in its own first step. Do not add an arm there from this task; the workspace-wide gates run at Task 3's exit.

- [ ] **Step 19: Write the failing preview-worker tests.** Append to the in-crate `mod tests` in `companion/crates/app-core/src/runtime.rs` (it already has `use super::*;` plus `CarouselAdvance`, `RefreshPolicy`, and the `plugin_card` / `rotation_clock_card` / `rotation_config` / `fresh_scheduler` helpers):

```rust
    struct PreviewPluginHost {
        renders: Arc<Mutex<Vec<u32>>>,
        requests: Arc<Mutex<Vec<RasterRequest>>>,
    }

    impl PluginHost for PreviewPluginHost {
        fn desired_assets(&mut self) -> Vec<DesiredAsset> {
            Vec::new()
        }

        fn render_scene(
            &mut self,
            _plugin_id: &str,
            _snapshot: &providers::ProviderSnapshot<serde_json::Value>,
            revision: u32,
        ) -> Result<SceneCandidate, String> {
            self.renders.lock().unwrap().push(revision);
            Ok(SceneCandidate::DisplayList(protocol::Scene {
                revision,
                background: 0x1234,
                nodes: Vec::new(),
            }))
        }

        fn rasterize(&mut self, request: &RasterRequest) -> Result<RasterFrame, String> {
            self.requests.lock().unwrap().push(request.clone());
            Ok(RasterFrame {
                digest: [0x55; protocol::ASSET_DIGEST_LEN],
                bytes: Arc::from(&[0x19, 0x12, 0, 0, 0xc0, 1, 0x70, 1, 0x80, 3, 0, 0, 1, 2][..]),
            })
        }
    }

    /// Two plugin cards, the FIRST one active. Every preview here targets the
    /// second, so "the active screen did not move" is an assertion about the
    /// route rather than a tautology about a one-card config.
    fn preview_state(
        now: Instant,
        renders: &Arc<Mutex<Vec<u32>>>,
        requests: &Arc<Mutex<Vec<RasterRequest>>>,
    ) -> (WorkerState, Scheduler) {
        let config = rotation_config(
            vec![
                plugin_card("aqi", "Air quality", "aqi", RefreshPolicy::Manual),
                plugin_card("agenda", "Agenda", "agenda", RefreshPolicy::Manual),
            ],
            CarouselAdvance::Manual,
            &[("aqi", None), ("agenda", None)],
        );
        let mut scheduler = fresh_scheduler(now);
        let mut state = WorkerState::new_with_plugin_host(
            config,
            now,
            &mut scheduler,
            Some(Box::new(PreviewPluginHost {
                renders: Arc::clone(renders),
                requests: Arc::clone(requests),
            })),
        );
        state.next_scene_revision = 41;
        state.active_scene_dirty = false;
        state.active_screen_dirty = false;
        (state, scheduler)
    }

    /// The whole safety property of the preview route, in one place: it builds
    /// at a revision the wire refuses, so the frame it produces is unpushable
    /// by construction rather than by a caller remembering not to send it.
    #[test]
    fn the_preview_revision_is_one_the_wire_refuses() {
        assert_eq!(
            validate_message(&Message::PushScene(PushScene {
                card_id: "aqi".into(),
                revision: PREVIEW_SCENE_REVISION,
                scene: protocol::Scene {
                    revision: PREVIEW_SCENE_REVISION,
                    background: 0,
                    nodes: Vec::new(),
                },
            })),
            Err(protocol::MessageError::InvalidValue("scene revision"))
        );
    }

    #[test]
    fn a_card_preview_renders_at_revision_zero_and_never_mints_a_push_revision() {
        let now = Instant::now();
        let renders = Arc::new(Mutex::new(Vec::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (mut state, _scheduler) = preview_state(now, &renders, &requests);
        state.plugin_snapshots.insert(
            "agenda".into(),
            providers::ProviderSnapshot {
                value: serde_json::json!({"events": []}),
                refreshed_at: DateTime::from_timestamp_millis(1_725_600_000_000),
                age: None,
                stale: false,
                error: None,
            },
        );

        let preview = render_card_preview(&mut state, "agenda").expect("a plugin card previews");

        assert_eq!(*renders.lock().unwrap(), vec![PREVIEW_SCENE_REVISION]);
        assert_eq!(requests.lock().unwrap().len(), 1);
        assert_eq!(preview.state, CardPreviewState::Fresh);
        assert_eq!(preview.message, None);
        assert_eq!(preview.refreshed_at_unix_ms, Some(1_725_600_000_000));
        assert_eq!(
            preview.frame.expect("a fresh preview carries a frame").digest,
            [0x55; protocol::ASSET_DIGEST_LEN]
        );
        assert_eq!(
            state.next_scene_revision, 41,
            "a preview is never sent, so it must not consume a wire revision"
        );
        assert!(
            !state.active_scene_dirty,
            "a preview must not schedule a device push"
        );
        assert_eq!(
            state.active_screen.as_deref(),
            Some("aqi"),
            "the preview route never activates the card it renders"
        );
        assert!(
            !state.active_screen_dirty,
            "a preview must not schedule an activation either"
        );
    }

    #[test]
    fn a_plugin_card_with_no_cached_snapshot_previews_as_waiting_without_a_frame() {
        let now = Instant::now();
        let renders = Arc::new(Mutex::new(Vec::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (mut state, _scheduler) = preview_state(now, &renders, &requests);

        let preview = render_card_preview(&mut state, "agenda").expect("waiting is an outcome");

        assert_eq!(preview.state, CardPreviewState::Waiting);
        assert_eq!(
            preview.message.as_deref(),
            Some("Waiting for the first refresh")
        );
        assert!(preview.frame.is_none());
        assert!(
            renders.lock().unwrap().is_empty(),
            "the pre-first-fetch state must not be reported as a compile failure"
        );
    }

    #[test]
    fn a_stale_or_errored_snapshot_still_draws_its_face() {
        let now = Instant::now();
        let renders = Arc::new(Mutex::new(Vec::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (mut state, _scheduler) = preview_state(now, &renders, &requests);
        state.plugin_snapshots.insert(
            "agenda".into(),
            providers::ProviderSnapshot {
                value: serde_json::json!({"events": []}),
                refreshed_at: None,
                age: None,
                stale: true,
                error: Some("upstream is stale".into()),
            },
        );

        let preview = render_card_preview(&mut state, "agenda").expect("a stale card still draws");

        assert_eq!(preview.state, CardPreviewState::Stale);
        assert_eq!(preview.message, None);
        assert!(
            preview.frame.is_some(),
            "the scene carries the same stale footer the panel shows; the state word is not the fault"
        );
    }

    #[test]
    fn a_card_preview_names_the_wrong_kind_and_the_missing_card_separately() {
        let now = Instant::now();
        let config = rotation_config(
            vec![rotation_clock_card("clock")],
            CarouselAdvance::Manual,
            &[("clock", None)],
        );
        let mut scheduler = fresh_scheduler(now);
        let mut state = WorkerState::new(config, now, &mut scheduler);

        assert!(matches!(
            render_card_preview(&mut state, "clock"),
            Err(RuntimeError::NotAPluginCard { ref card_id }) if card_id == "clock"
        ));
        assert!(matches!(
            render_card_preview(&mut state, "absent"),
            Err(RuntimeError::UnknownCard { ref card_id }) if card_id == "absent"
        ));
    }
```

- [ ] **Step 20: Write the failing handle-level preview tests.** In `companion/crates/app-core/tests/runtime.rs`, replace the `use app_core::{…}` block (lines 7-15) with:

```rust
use app_core::{
    AlertHold, AppConfig, CalendarRefreshRequest, CalendarRefreshResult, CalendarRefresher,
    CardAlert, CardErrorKind, CardField, CardFieldValue, CardPreviewState, CardSettings,
    CarouselAdvance, ConnectionState, DesiredAsset, DeviceCapability, DeviceConnection,
    DeviceOtaState, DeviceTier, DeviceWifiState, DisplayOrientation, DisplayTemplate, NetworkConfig,
    PersistenceState, Playlist, PlaylistEntry, PluginHost, PomodoroAction, PomodoroState,
    ProviderRequest, ProvisioningTier, RasterFrame, RasterRequest, RefreshPolicy, RuntimeDevice,
    RuntimeError, RuntimeHandle, RuntimeOptions, RuntimeState, SceneCandidate, WidgetTapAction,
};
```

replace `FakePluginHostState` (lines 667-674) with:

```rust
#[derive(Default)]
struct FakePluginHostState {
    outcomes: VecDeque<Result<(), String>>,
    renders: Vec<PluginRenderCall>,
    desired_assets: Vec<DesiredAsset>,
    invalid_scene: bool,
    frame: Option<RasterFrame>,
    raster_requests: Vec<RasterRequest>,
}
```

add two methods to `impl FakePluginHostControl` (after `return_invalid_scene`, line 700):

```rust
    fn set_raster_frame(&self, frame: RasterFrame) {
        self.state.lock().unwrap().frame = Some(frame);
    }

    fn raster_requests(&self) -> Vec<RasterRequest> {
        self.state.lock().unwrap().raster_requests.clone()
    }
```

add one method to `impl PluginHost for FakePluginHost` (after `render_scene`, which ends at line 737):

```rust
    fn rasterize(&mut self, request: &RasterRequest) -> Result<RasterFrame, String> {
        let mut state = self.control.state.lock().unwrap();
        state.raster_requests.push(request.clone());
        state
            .frame
            .clone()
            .ok_or_else(|| "the fixture host has no frame".to_owned())
    }
```

and append both tests after `plugin_card_pushes_the_host_scene_unmodified_after_raw_data_arrives` (which ends at line 1190):

```rust
#[test]
fn a_card_preview_answers_without_a_device_push_and_leaves_the_revision_counter_alone() {
    let control = MockDeviceControl::default();
    let host = FakePluginHostControl::default();
    host.set_raster_frame(RasterFrame {
        digest: [0x77; protocol::ASSET_DIGEST_LEN],
        bytes: Arc::from(&[0x19, 0x12, 0, 0, 0xc0, 1, 0x70, 1, 0x80, 3, 0, 0, 1, 2][..]),
    });
    let runtime = start_plugin_runtime(&control, Some(Box::new(host.host())));
    wait_for(Duration::from_secs(1), || {
        control.operations().iter().any(|operation| {
            matches!(operation, Operation::PushScene(push) if push.card_id == "plugin-card")
        })
    });

    let pushed_revision = |operations: &[Operation]| -> Option<u32> {
        operations
            .iter()
            .rev()
            .find_map(|operation| match operation {
                Operation::PushScene(push) if push.card_id == "plugin-card" => Some(push.revision),
                _ => None,
            })
    };
    let scene_pushes = |operations: &[Operation]| -> usize {
        operations
            .iter()
            .filter(|operation| matches!(operation, Operation::PushScene(_)))
            .count()
    };
    let activations = |operations: &[Operation]| -> usize {
        operations
            .iter()
            .filter(|operation| matches!(operation, Operation::Activate(_)))
            .count()
    };
    let before = control.operations();
    let pushes_before = scene_pushes(&before);
    let activations_before = activations(&before);
    let last_revision = pushed_revision(&before).expect("the plugin card was pushed");
    let active_before = runtime.snapshot().unwrap().device.active_screen_id;

    let preview = runtime.render_card_preview("plugin-card").unwrap();

    assert_eq!(preview.state, CardPreviewState::Stale);
    assert_eq!(preview.message, None);
    assert_eq!(
        preview.refreshed_at_unix_ms,
        u64::try_from(plugin_refreshed_at().timestamp_millis()).ok()
    );
    assert_eq!(
        preview.frame.expect("a stale card still draws").digest,
        [0x77; protocol::ASSET_DIGEST_LEN]
    );
    assert_eq!(host.raster_requests().len(), 1);
    assert_eq!(
        host.renders().last().expect("the preview rendered").revision,
        0,
        "a preview builds at the revision the wire refuses"
    );
    let after = control.operations();
    assert_eq!(
        scene_pushes(&after),
        pushes_before,
        "a preview must never reach the device"
    );
    assert_eq!(
        activations(&after),
        activations_before,
        "the preview route never activates the card"
    );
    assert_eq!(
        runtime.snapshot().unwrap().device.active_screen_id,
        active_before,
        "the preview route never changes which card the panel is showing"
    );

    runtime
        .inject_plugin_snapshot(
            "plugin-card",
            "test-plugin",
            providers::ProviderSnapshot {
                value: serde_json::json!({"after": "preview"}),
                refreshed_at: None,
                age: None,
                stale: false,
                error: None,
            },
        )
        .unwrap();
    wait_for(Duration::from_secs(1), || {
        pushed_revision(&control.operations()).is_some_and(|revision| revision > last_revision)
    });
    assert_eq!(
        pushed_revision(&control.operations()),
        Some(last_revision + 1),
        "the preview must not have consumed a revision the next push then skips"
    );
    runtime.shutdown().unwrap();
}

#[test]
fn a_preview_of_a_card_that_is_not_a_plugin_card_is_typed() {
    let control = MockDeviceControl::default();
    let runtime = start_runtime(full_config(), &control, Duration::ZERO);

    assert!(matches!(
        runtime.render_card_preview("clock"),
        Err(RuntimeError::NotAPluginCard { ref card_id }) if card_id == "clock"
    ));
    assert!(matches!(
        runtime.render_card_preview("nope"),
        Err(RuntimeError::UnknownCard { ref card_id }) if card_id == "nope"
    ));
    runtime.shutdown().unwrap();
}
```

- [ ] **Step 21: Run both and watch them fail to compile.** `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p app-core --lib runtime::tests` then `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p app-core --test runtime a_card_preview` — expect `error[E0425]: cannot find function 'render_card_preview' in this scope` and `cannot find value 'PREVIEW_SCENE_REVISION' in this scope` from the first, and `error[E0599]: no method named 'render_card_preview' found for struct 'RuntimeHandle'` plus `cannot find value 'CardPreviewState' in this scope` from the second.

- [ ] **Step 22: Add the command, the value and the constant.** In `companion/crates/app-core/src/commands.rs`, below `pub(crate) type CommandReply` (line 54):

```rust
/// A preview answers with a rendered frame, so it cannot ride `CommandReply`,
/// which carries only success or failure.
pub(crate) type PreviewReply = SyncSender<Result<crate::runtime::CardPreview, RuntimeError>>;
```

and a variant in `RuntimeCommand`, after `InjectPluginSnapshot` (line 87):

```rust
    RenderCardPreview {
        card_id: String,
        reply: PreviewReply,
    },
```

In `companion/crates/app-core/src/runtime.rs`, add `CardPreviewState` to the `use crate::{…}` list between `CardErrorKind` and `CardSettings` (line 32), and insert beside `RasterFrame` (after line 158):

```rust
/// The revision every preview build carries. `protocol::validate_message`
/// rejects `PushScene` revision 0, so a preview candidate cannot reach a
/// device even by mistake -- which is exactly the property the route needs.
pub const PREVIEW_SCENE_REVISION: u32 = 0;

/// One rendered card face for the admin preview route. Never pushed, never
/// minted, never cached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardPreview {
    pub frame: Option<RasterFrame>,
    pub state: CardPreviewState,
    pub message: Option<String>,
    pub refreshed_at_unix_ms: Option<u64>,
}
```

- [ ] **Step 23: Make `request` generic and add the handle method, the worker arm and the worker function.** In `companion/crates/app-core/src/runtime.rs`, replace `RuntimeHandle::request` (lines 959-981) in full:

```rust
    fn request<T>(
        &self,
        command: impl FnOnce(SyncSender<Result<T, RuntimeError>>) -> RuntimeCommand,
    ) -> Result<T, RuntimeError> {
        let (reply_sender, reply_receiver) = mpsc::sync_channel(1);
        match self.sender.try_send(command(reply_sender)) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(_)) => {
                self.diagnostics
                    .command_queue_full
                    .fetch_add(1, Ordering::Relaxed);
                return Err(RuntimeError::QueueFull);
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                return Err(RuntimeError::WorkerStopped);
            }
        }
        match reply_receiver.recv_timeout(self.command_timeout) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => Err(RuntimeError::ResponseTimeout),
            Err(RecvTimeoutError::Disconnected) => Err(RuntimeError::WorkerStopped),
        }
    }
```

Every existing caller infers `T = ()` from the `CommandReply` its closure builds, so no call site changes. Add the handle method after `inject_plugin_snapshot` (line 911):

```rust
    /// Renders one plugin card's face for an admin preview. It runs on the
    /// worker beside every other command, and the worker's single-threading is
    /// the whole of the route's rate limiting: nothing here touches the device,
    /// mints a revision, activates a card, or marks the active scene dirty.
    pub fn render_card_preview(&self, card_id: &str) -> Result<CardPreview, RuntimeError> {
        self.request(|reply| RuntimeCommand::RenderCardPreview {
            card_id: card_id.to_owned(),
            reply,
        })
    }
```

Add the worker arm to `process_command`, after the `InjectPluginSnapshot` arm (line 1597):

```rust
        RuntimeCommand::RenderCardPreview { card_id, reply } => {
            let _ = reply.send(render_card_preview(state, &card_id));
        }
```

And add the function beside `raster_request` (immediately after it, line 3283):

```rust
/// Builds and rasterizes one plugin card's face without sending it. Takes no
/// device: the absence of that parameter is the guarantee, not a comment.
fn render_card_preview(
    state: &mut WorkerState,
    card_id: &str,
) -> Result<CardPreview, RuntimeError> {
    let Some(card) = state.config.cards.iter().find(|card| card.id() == card_id) else {
        return Err(RuntimeError::UnknownCard {
            card_id: card_id.to_owned(),
        });
    };
    if !matches!(card, CardSettings::Plugin { .. }) {
        return Err(RuntimeError::NotAPluginCard {
            card_id: card_id.to_owned(),
        });
    }
    let Some(snapshot) = state.plugin_snapshots.get(card_id).cloned() else {
        return Ok(CardPreview {
            frame: None,
            state: CardPreviewState::Waiting,
            message: Some("Waiting for the first refresh".into()),
            refreshed_at_unix_ms: None,
        });
    };
    let refreshed_at_unix_ms = snapshot
        .refreshed_at
        .and_then(|time| u64::try_from(time.timestamp_millis()).ok());
    let fields = state
        .latest_fields
        .get(card_id)
        .cloned()
        .unwrap_or_default();
    let candidate = match build_card_scene(
        &state.config,
        card_id,
        &fields,
        Some(&snapshot),
        state.plugin_host.as_deref_mut(),
        PREVIEW_SCENE_REVISION,
    ) {
        Ok(candidate) => candidate,
        Err(message) => return Ok(preview_failure(message)),
    };
    let request = raster_request(&candidate, &fields);
    // `build_card_scene` already refuses a plugin card with no host, so this is
    // a policy bug rather than a state -- and a policy bug must surface as a
    // typed outcome, not a panic on the worker thread.
    let Some(host) = state.plugin_host.as_deref_mut() else {
        return Ok(preview_failure(
            "this card needs server-side rasterization, which this host does not perform".into(),
        ));
    };
    match host.rasterize(&request) {
        Ok(frame) => Ok(CardPreview {
            frame: Some(frame),
            // A fetched-but-troubled snapshot still draws: the face carries the
            // stale/error footer the panel shows. `Error` is reserved for "no
            // frame at all", which keeps spec 4.2's invariant true.
            state: if snapshot.stale || snapshot.error.is_some() {
                CardPreviewState::Stale
            } else {
                CardPreviewState::Fresh
            },
            message: None,
            refreshed_at_unix_ms,
        }),
        Err(message) => Ok(preview_failure(message)),
    }
}

fn preview_failure(message: String) -> CardPreview {
    CardPreview {
        frame: None,
        state: CardPreviewState::Error,
        message: Some(message),
        refreshed_at_unix_ms: None,
    }
}
```

- [ ] **Step 24: Bound-check the preview scene without wire-validating it, and re-export.** In `companion/crates/app-core/src/runtime.rs`, replace `build_card_scene`'s validation (lines 2734-2735):

```rust
    if push.revision == PREVIEW_SCENE_REVISION {
        // A preview never becomes a frame on the wire, so validate the scene's
        // own bounds and leave the message rule -- including the nonzero
        // revision that makes a preview unpushable -- to the one path that
        // actually sends messages. Card-id length is already bounded by config
        // validation (`MAX_WIDGET_ID_LEN`).
        protocol::validate_scene(&push.scene)
            .map_err(|error| format!("the host-built scene is invalid: {error}"))?;
    } else {
        validate_message(&Message::PushScene(push.clone()))
            .map_err(|error| format!("the host-built scene is invalid: {error}"))?;
    }
```

and replace `companion/crates/app-core/src/lib.rs`'s runtime re-export (lines 48-53) with:

```rust
pub use runtime::{
    CalendarRefreshRequest, CalendarRefreshResult, CalendarRefresher, CardPreview,
    DeviceConnection, PREVIEW_SCENE_REVISION, PluginHost, ProviderRefreshRequest,
    ProviderRefreshResult, ProviderRefresher, ProviderRequest, RasterFrame, RasterRequest,
    RuntimeDevice, RuntimeHandle, RuntimeOptions, RuntimeSubscription, SceneCandidate,
    SerialRuntimeDevice, SystemCalendarRefresher, SystemProviderRefresher,
};
```

- [ ] **Step 25: Run to pass.** `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p app-core --all-targets` — every new test passes, and `invalid_plugin_host_scene_is_refused_before_device_delivery` still passes because a real push carries a nonzero revision and keeps the `validate_message` branch.

- [ ] **Step 26: Prove the three preview guarantees bite (mutation probes).** Run each of these from `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion`, reverting the mutation before the next:
  1. Change `PREVIEW_SCENE_REVISION` to `1`, then `/Users/rodion/.cargo/bin/cargo test -p app-core --lib runtime::tests::the_preview_revision_is_one_the_wire_refuses` and `/Users/rodion/.cargo/bin/cargo test -p app-core --test runtime a_card_preview_answers_without_a_device_push` — both must fail.
  2. Change `render_card_preview`'s stale branch to always produce `CardPreviewState::Fresh`, then `/Users/rodion/.cargo/bin/cargo test -p app-core --lib runtime::tests::a_stale_or_errored_snapshot_still_draws_its_face` — must fail.
  3. Add `state.active_screen = Some(card_id.to_owned());` at the top of `render_card_preview`, then `/Users/rodion/.cargo/bin/cargo test -p app-core --lib runtime::tests::a_card_preview_renders_at_revision_zero_and_never_mints_a_push_revision` — must fail on "the preview route never activates the card it renders".

- [ ] **Step 27: Commit.**

```sh
git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity add companion/crates/app-core companion/apps/deskmate/src-tauri/src && git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -m "feat: render a plugin card preview on the worker without minting a revision" -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
```

- [ ] **Step 28: Run this task's gates.** `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo fmt --all --check` then `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo clippy -p app-core -p deskmate-app --all-targets -- -D warnings` then `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p app-core --all-targets` then `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p app-core --doc`. All four must be green. Both `cargo test` invocations are required and stay separate lines: `--all-targets` adds the integration targets but removes doctests.

**Task exit:** `cargo fmt --all --check`, `cargo clippy -p app-core -p deskmate-app --all-targets -- -D warnings`, `cargo test -p app-core --all-targets` and `cargo test -p app-core --doc` all green from `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion`; the `GET /v1/plugins` JSON for the five shipped keys is byte-unchanged (pinned by `admin::tests::a_catalog_entry_keeps_the_shipped_keys_and_adds_the_new_ones_after_them`; the server route itself is untouched here); the seven admin DTOs exist in `app-core/src/admin.rs`, six re-exported from the crate root and `PluginLoadFailure` reachable only as `app_core::admin::PluginLoadFailure`; a hostless runtime schedules and reports nothing for a plugin card; `RuntimeHandle::render_card_preview` answers without a device push, without minting a revision and without activating the card. **`cargo check -p server` deliberately does not compile at this point** — `companion/crates/server/src/admin.rs:1050`'s exhaustive conversion from `RuntimeError` has not yet learned `UnknownCard`/`NotAPluginCard`, and Task 3 owns that mapping and closes it in its first step; the workspace-wide gates run at Task 3's exit. No frontend gate applies to this task.

---

### Task 3: Catalog fields, the preview route, and the hero field

Implements spec §4 on the server. Three additive changes behind `AdminAuthenticated`: `GET /v1/plugins` gains five manifest-derived fields and swaps its four local body types for the shared app-core DTOs Task 2 created, so the server serializes and the Mac deserializes *one* type (§4.1); a new `GET /v1/devices/{id}/cards/{card_id}/preview` returns the card's face as a PNG through the real `ServerPluginHost` (§4.2); and `ServerProviderRefresher` publishes the manifest's evaluated `summary` as the card's `hero` field (§4.3). Nothing on the wire, in the device link, or in the registry changes (§4.4) — and one test proves that from the device's end of the link.

This task creates **no** app-core DTOs. Task 2 owns every type in `app-core/src/admin.rs`; Task 3 only consumes them, path-qualifying `app_core::admin::PluginLoadFailure` because `server::plugin_registry::PluginLoadFailure` already owns that name in this crate. Task 3 **does** own the HTTP status mapping for Task 2's two new `RuntimeError` variants, and closes it in Step 1 because nothing in this crate compiles until it is closed.

**Files:**

- Modify `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/crates/server/src/admin.rs` (read 1-146 — imports 3-22, `routes()` 28-38, `get_plugins` 40-87, the four local DTOs 89-116, `asset_kind_name` 118-124; `get_device` 404-438; the test module 525-1013; `AdminError` 1024-1034; `From<RuntimeError> for AdminError` 1050-1076; `ErrorBody`/`IntoResponse` 1078-1119).
- Modify `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/crates/server/src/rasterizer.rs` — lift `frame_png` (2047-2068) out of `#[cfg(test)] pub(crate) mod tests` (1178-1179) to file scope beside `pack_rgb565` (1169-1176), and re-point its one existing caller `assert_raster_regression` (2070-2087).
- Modify `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/crates/server/src/plugin_refresher.rs` — imports 1-14, struct 17-22, `with_fetcher_factory` 40-50, `impl ProviderRefresher` 53-109, `title_field` 111-116, tests 131-358 (`refresher_with` 196-203; `title_or_card_cadence_edits_keep_last_good_until_the_plugin_id_changes` 305-357).
- Modify `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/crates/server/src/lib.rs` — the module list at 16-29.
- Create `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/crates/server/src/test_plugins.rs` — three fixture manifests and one loader, shared by the `admin` and `plugin_refresher` test modules.
- Modify `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/crates/server/tests/support/mod.rs` — add `card_messages_during` beside `flush_socket` (245-264), reusing the private `reply` (266-308).
- Create `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/crates/server/tests/fixtures/one-plugin-card.json`.
- Create `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/crates/server/tests/card_preview.rs`.

**Interfaces:**

*Consumes* (from earlier tasks, unchanged here):

- Task 1: `plugin::PluginManifest { display_name: Option<String>, description: Option<String>, summary: Option<String>, .. }` — `summary` is an `Option<String>`, not an `Option<Expression>`, because a text node's `value` is an opaque `String` (`crates/plugin/src/manifest.rs:467`) and `summary` holds the same shape; `plugin::evaluate_summary(&PluginManifest, &providers::ProviderSnapshot<serde_json::Value>) -> Result<Option<String>, plugin::SummaryError>`; the four curated manifests are `manifest_version = 2` and declare all three keys.
- Task 2 (all seven DTOs, created there, none created here): `app_core::admin::{PluginCatalog, PluginCatalogEntry, PluginCatalogAsset, PluginLoadFailure, PluginTemplateKind, CardPreviewState, CardPreviewResponse}`, re-exported at the crate root **except** `PluginLoadFailure`; `app_core::runtime::CardPreview { frame: Option<RasterFrame>, state: CardPreviewState, message: Option<String>, refreshed_at_unix_ms: Option<u64> }`; `RuntimeHandle::render_card_preview(&self, card_id: &str) -> Result<CardPreview, RuntimeError>`; `RuntimeError::UnknownCard { card_id: String }` and `RuntimeError::NotAPluginCard { card_id: String }`.
- Existing: `crate::plugin_registry::{PluginRegistry, LoadedPlugin}` (`load`, `ids`, `get`); `crate::plugin_host::ServerPluginHost::new(Arc<PluginRegistry>)`; `crate::rasterizer::RasterizedFrame { digest: [u8; 32], bytes: Vec<u8> }`; `crate::runtime_device::WebSocketRuntimeDevice::channel`; `crate::ServerState::{plugins, plugin_load_failures, device_link, claim_link, registry, shutdown}`; `app_core::RasterFrame { digest, bytes: Arc<[u8]> }`; `protocol::digest_hex`; `plugin::{ManifestVersion, Source, Template, MAX_OUTPUT_LEN}`.

*Produces*:

- `GET /v1/plugins -> Json<app_core::admin::PluginCatalog>` — the five pre-existing keys unchanged in name, order and value; `display_name`, `description`, `manifest_version`, `template`, `refresh_minutes` added.
- `GET /v1/devices/{id}/cards/{card_id}/preview -> Json<app_core::admin::CardPreviewResponse>`.
- `From<RuntimeError> for AdminError` maps `UnknownCard` and `NotAPluginCard` to the **existing unit** `AdminError::NotFound` (`crates/server/src/admin.rs:1027`) — not to `AdminError::Runtime { status, message }`.
- `crate::rasterizer::frame_png(bytes: &[u8]) -> Vec<u8>` — `pub(crate)`, no longer test-only.
- `ServerProviderRefresher` emits `Field { key: "hero", value: FieldValue::Text(summary) }` after `title`.
- `support::card_messages_during(&mut DeviceSocket, Duration) -> Vec<&'static str>` for the server's test binaries.

---

- [ ] **Step 1: Close Task 2's deliberate compile break.** Task 2 exits with `cargo check -p server` failing on purpose: `From<RuntimeError> for AdminError` (`/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/crates/server/src/admin.rs:1050-1076`) matches `RuntimeError` exhaustively and does not cover the two variants Task 2 added. Until that is closed the `server` crate does not build at all, so no test in this task — not even a failing one — can run. Nothing else here depends on it, so it is first and it is alone.

  Red state:

```sh
cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo check -p server
```

Expect ``error[E0004]: non-exhaustive patterns: `RuntimeError::UnknownCard { .. }` and `RuntimeError::NotAPluginCard { .. }` not covered``, pointing at the `match error {` on line 1052.

Add one arm covering both, after the `UnknownWidget | UnknownScreen` arm (which ends at line 1073) and before the match's closing brace at line 1074. The whole edited impl, which is what lines 1050-1076 must read afterwards:

```rust
impl From<RuntimeError> for AdminError {
    fn from(error: RuntimeError) -> Self {
        match error {
            RuntimeError::InvalidConfig { issues } => Self::InvalidConfig { issues },
            RuntimeError::QueueFull | RuntimeError::WorkerStopped => Self::Runtime {
                status: StatusCode::SERVICE_UNAVAILABLE,
                message: error.to_string(),
            },
            RuntimeError::ResponseTimeout => Self::Runtime {
                status: StatusCode::GATEWAY_TIMEOUT,
                message: error.to_string(),
            },
            RuntimeError::DeviceDisconnected
            | RuntimeError::Device { .. }
            | RuntimeError::Provider { .. } => Self::Runtime {
                status: StatusCode::BAD_GATEWAY,
                message: error.to_string(),
            },
            RuntimeError::UnknownWidget { .. } | RuntimeError::UnknownScreen { .. } => {
                Self::Runtime {
                    status: StatusCode::UNPROCESSABLE_ENTITY,
                    message: error.to_string(),
                }
            }
            // Naming a card that is not configured, or one that is not a
            // plugin card, is an addressing mistake by the caller -- the same
            // bare 404 an unknown device gets from `get_device`, not a runtime
            // fault with a body. The unit `NotFound` already exists and is
            // what `IntoResponse` turns into a bare `404`.
            RuntimeError::UnknownCard { .. } | RuntimeError::NotAPluginCard { .. } => {
                Self::NotFound
            }
        }
    }
}
```

No new `AdminError` variant is needed: `NotFound` is the existing unit variant at `crates/server/src/admin.rs:1027`, and `impl IntoResponse for AdminError` already turns it into a bare `404` at line 1092. The route that will produce these errors does not exist yet — it arrives in Step 14 — so this arm has no test of its own until then; Step 22's fifth mutation probe is what proves it load-bearing.

  Green state:

```sh
cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo check -p server
```

Expect `Finished`.

```sh
git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -am "$(cat <<'EOF'
fix: map the two card-addressing runtime errors onto the admin 404

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S
EOF
)"
```

- [ ] **Step 2: Add the shared plugin-manifest fixtures.** Create `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/crates/server/src/test_plugins.rs` — used by both the `admin` and `plugin_refresher` test modules, so the manifests exist once:

```rust
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
    let (registry, failures) = PluginRegistry::load(base.path()).expect("load the fixture registry");
    assert!(failures.is_empty(), "unexpected failures: {failures:?}");
    (Arc::new(registry), base)
}
```

Then declare it in `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/crates/server/src/lib.rs`, replacing lines 26-29:

```rust
mod rasterizer;
pub mod registry;
pub mod runtime_device;
mod store;
#[cfg(test)]
mod test_plugins;
```

- [ ] **Step 3: Write the failing catalog tests.** Add to the `mod tests` block in `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/crates/server/src/admin.rs`, after the existing `plugin_catalog_digest_matches_resolve_assets_for_the_same_file` (which keeps passing unchanged):

```rust
    async fn plugin_catalog_json(state: crate::ServerState) -> serde_json::Value {
        let response = crate::app(state)
            .oneshot(
                Request::builder()
                    .uri("/v1/plugins")
                    .header("authorization", "Bearer admin-secret")
                    .body(Body::empty())
                    .expect("catalog request"),
            )
            .await
            .expect("catalog response");
        assert_eq!(response.status(), StatusCode::OK);
        response_json(response).await
    }

    /// The tempdir is returned, not dropped: `ServerState` keeps the path and
    /// deleting the directory out from under it would be a different test.
    fn state_with_registry(
        registry: Arc<crate::plugin_registry::PluginRegistry>,
    ) -> (crate::ServerState, tempfile::TempDir) {
        let config_dir = tempfile::tempdir().expect("config tempdir");
        let state = crate::ServerState::new_with_plugins(
            "admin-secret".into(),
            crate::firmware::FirmwareCatalog::in_memory(),
            config_dir.path().to_path_buf(),
            registry,
            Vec::new(),
        );
        (state, config_dir)
    }

    #[tokio::test]
    async fn the_catalog_keeps_every_field_the_previous_response_carried() {
        // The five keys below are additive. A companion built before this
        // change reads the rest, so a renamed key or a moved value here is a
        // silent break rather than a compile error. Task 2's app-core test
        // pins the serialized key ORDER; this pins the values the server puts
        // in them for the real curated registry.
        let (state, _device_id, _config_dir) = state_with_curated_plugins();
        let mut legacy = plugin_catalog_json(state).await;
        for entry in legacy["plugins"].as_array_mut().expect("plugins array") {
            let entry = entry.as_object_mut().expect("plugin entry object");
            for added in [
                "display_name",
                "description",
                "manifest_version",
                "template",
                "refresh_minutes",
            ] {
                assert!(entry.remove(added).is_some(), "{added} is not in the entry");
            }
        }

        assert_eq!(
            legacy,
            serde_json::json!({
                "plugins": [
                    {"id": "agenda", "name": "agenda", "version": "1.0.0", "node_count": 4,
                     "assets": [{"file": "badge.rgb565", "kind": "image", "byte_length": 812,
                                 "digest": "e19db5d47bbdae46bf80e5a7df400795bce84973817c9f124642bc76bf41d33a"}]},
                    {"id": "aqi", "name": "aqi", "version": "1.0.0", "node_count": 7,
                     "assets": [{"file": "icons.ttf", "kind": "icon-font", "byte_length": 4320,
                                 "digest": "40bbbac715465adf7ba539f53a0cb16991a2c1f73a4fb57af209d39e0c62b327"}]},
                    {"id": "claude-limits", "name": "claude-limits", "version": "1.0.0",
                     "node_count": 12, "assets": []},
                    {"id": "svg-aqi", "name": "svg-aqi", "version": "1.0.0", "node_count": 0,
                     "assets": []}
                ],
                "load_failures": []
            })
        );
    }

    #[tokio::test]
    async fn the_catalog_reports_each_manifests_template_kind_and_cadence() {
        let (state, _device_id, _config_dir) = state_with_curated_plugins();
        let body = plugin_catalog_json(state).await;
        let entry = |id: &str| {
            body["plugins"]
                .as_array()
                .expect("plugins array")
                .iter()
                .find(|entry| entry["id"] == id)
                .unwrap_or_else(|| panic!("no catalog entry for {id}"))
                .clone()
        };

        // The editor's Plugin field and the picker read these; `svg-aqi` is
        // the only curated plugin the panel can never render natively.
        assert_eq!(entry("aqi")["template"], "display-list");
        assert_eq!(entry("svg-aqi")["template"], "svg");
        assert_eq!(entry("aqi")["refresh_minutes"], 15);
        assert_eq!(entry("agenda")["refresh_minutes"], 10);
    }

    #[tokio::test]
    async fn a_v2_manifests_own_display_name_and_description_reach_the_catalog() {
        let (registry, _base) =
            crate::test_plugins::fixture_registry(crate::test_plugins::V2_NAMED_MANIFEST);
        let (state, _config_dir) = state_with_registry(registry);

        let body = plugin_catalog_json(state).await;

        assert_eq!(body["plugins"][0]["display_name"], "Fixture plugin");
        assert_eq!(
            body["plugins"][0]["description"],
            "Names and cadence, threaded from the manifest"
        );
        assert_eq!(body["plugins"][0]["manifest_version"], 2);
        assert_eq!(body["plugins"][0]["refresh_minutes"], 7);
    }

    #[tokio::test]
    async fn a_v1_manifest_reports_version_one_and_no_names_rather_than_failing() {
        // v1 stays frozen: absence is the normal case, not a load failure.
        let (registry, _base) =
            crate::test_plugins::fixture_registry(crate::test_plugins::V1_MANIFEST);
        let (state, _config_dir) = state_with_registry(registry);

        let body = plugin_catalog_json(state).await;

        assert_eq!(body["plugins"][0]["manifest_version"], 1);
        assert!(body["plugins"][0]["display_name"].is_null());
        assert!(body["plugins"][0]["description"].is_null());
        assert_eq!(body["load_failures"].as_array().unwrap().len(), 0);
    }
```

- [ ] **Step 4: Run them and watch them fail.**

```sh
cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p server --lib admin::tests
```

The crate compiles here only because Step 1 closed Task 2's exhaustive-match break; without it this run would stop at `error[E0004]` before any test was built. So the failure to expect is an assertion, not a type error: `assertion failed: entry.remove(added).is_some()` with the message `display_name is not in the entry` in `the_catalog_keeps_every_field_the_previous_response_carried`, and `left: Null, right: "Fixture plugin"` / `left: Null, right: "display-list"` in the other three.

- [ ] **Step 5: Swap the server's local catalog DTOs for the shared app-core types and populate the five fields.** In `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/crates/server/src/admin.rs`, replace the import block at lines 3-7 and line 22 with:

```rust
use app_core::{
    AdminConfigErrorBody, AppConfig, AppSnapshot, BakedFontMetrics, ClockCard,
    MAX_CONFIG_FILE_BYTES, PluginCatalog, PluginCatalogAsset, PluginCatalogEntry,
    PluginTemplateKind, RuntimeError, SaveReceipt, StoreError, ValidationIssue,
    build_digital_clock_scene,
};
```

```rust
use crate::plugin_registry::{LoadedPlugin, PluginRegistry};
```

(`PluginRegistry` stays because `build_operator_scene` still takes one at line 311; `LoadedPlugin` is what `catalog_entry` below needs. `app_core::admin::PluginLoadFailure` is deliberately *not* imported: `crate::plugin_registry::PluginLoadFailure` already owns that name here, so the app-core one is path-qualified at its single use site.)

Then replace `get_plugins` and the four local DTOs — lines 40-116, everything from `async fn get_plugins(` down to and including the closing brace of `struct PluginLoadFailureResponse` — with:

```rust
async fn get_plugins(
    State(state): State<ServerState>,
    _admin: AdminAuthenticated,
) -> Json<PluginCatalog> {
    let plugins = state
        .plugins()
        .ids()
        .filter_map(|id| state.plugins().get(id))
        .map(catalog_entry)
        .collect();
    let load_failures = state
        .plugin_load_failures()
        .iter()
        .map(|failure| app_core::admin::PluginLoadFailure {
            id: failure.id.clone(),
            error: failure.error.to_string(),
        })
        .collect();
    Json(PluginCatalog {
        plugins,
        load_failures,
    })
}

fn catalog_entry(loaded: &LoadedPlugin) -> PluginCatalogEntry {
    let mut assets: Vec<_> = loaded
        .assets
        .iter()
        .map(|(file, asset)| PluginCatalogAsset {
            file: file.to_string(),
            kind: asset_kind_name(asset.kind).to_owned(),
            byte_length: asset.bytes.len(),
            digest: protocol::digest_hex(&asset.digest),
        })
        .collect();
    assets.sort_by(|left, right| left.file.cmp(&right.file));
    let plugin::Source::Json {
        refresh_minutes, ..
    } = &loaded.manifest.source;
    PluginCatalogEntry {
        id: loaded.id.clone(),
        name: loaded.manifest.name.clone(),
        version: loaded.manifest.version.clone(),
        node_count: loaded.manifest.nodes.len()
            + loaded
                .manifest
                .repeats
                .iter()
                .map(|repeat| repeat.nodes.len())
                .sum::<usize>(),
        assets,
        display_name: loaded.manifest.display_name.clone(),
        description: loaded.manifest.description.clone(),
        manifest_version: match loaded.manifest.manifest_version {
            plugin::ManifestVersion::V1 => 1,
            plugin::ManifestVersion::V2 => 2,
        },
        template: match &loaded.manifest.template {
            plugin::Template::Scene => PluginTemplateKind::DisplayList,
            plugin::Template::Svg { .. } => PluginTemplateKind::Svg,
        },
        // `parse_manifest` bounds this to `plugin::MAX_REFRESH_MINUTES`
        // (1440), so the saturating arm is unreachable today; it is here so a
        // future bound change cannot silently wrap a cadence.
        refresh_minutes: u16::try_from(*refresh_minutes).unwrap_or(u16::MAX),
    }
}
```

`asset_kind_name` (now lines 48-54 after the replacement) is unchanged and still returns `&'static str`; `PluginCatalogAsset::kind` is a `String`, hence the `.to_owned()`.

- [ ] **Step 6: Run to pass.**

```sh
cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p server --lib admin::tests
```

Expect `test result: ok.`, including the pre-existing `plugin_catalog_digest_matches_resolve_assets_for_the_same_file`, which reads `digest`/`byte_length`/`load_failures[0].id` and so proves those keys survived the type swap unmoved.

- [ ] **Step 7: Commit.**

```sh
git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -am "$(cat <<'EOF'
feat: serve the plugin catalog from app-core's shared DTO with manifest fields

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S
EOF
)"
```

- [ ] **Step 8: Write the failing preview-route wiring tests.** Add to the `mod tests` block in `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/crates/server/src/admin.rs`:

```rust
    async fn preview_request(
        state: crate::ServerState,
        device_id: &str,
        card_id: &str,
        authorized: bool,
    ) -> Response {
        let mut builder = Request::builder()
            .method("GET")
            .uri(format!("/v1/devices/{device_id}/cards/{card_id}/preview"));
        if authorized {
            builder = builder.header("authorization", "Bearer admin-secret");
        }
        crate::app(state)
            .oneshot(builder.body(Body::empty()).expect("preview request"))
            .await
            .expect("preview response")
    }

    #[tokio::test]
    async fn a_preview_for_an_unknown_device_is_a_not_found() {
        let (state, _device_id, _config_dir) = state_with_curated_plugins();

        let response = preview_request(state, "dev-not-minted", "aqi-card", true).await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_preview_before_any_runtime_exists_is_service_unavailable() {
        // A minted device that has never connected has no runtime to ask.
        // That is temporary and retryable, so it is 503 -- distinct from the
        // 404 an addressing mistake gets.
        let (state, device_id, _config_dir) = state_with_curated_plugins();

        let response = preview_request(state, &device_id, "aqi-card", true).await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let error = response_json(response).await;
        assert_eq!(error["kind"], "runtime");
    }

    #[tokio::test]
    async fn the_preview_route_requires_admin_authentication() {
        let (state, device_id, _config_dir) = state_with_curated_plugins();

        let response = preview_request(state, &device_id, "aqi-card", false).await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
```

- [ ] **Step 9: Write the failing end-to-end preview tests.** Add to the same `mod tests` block, and add `use base64::Engine as _;` beside the module's existing `use tower::ServiceExt as _;` (line 534). These drive the *real* `ServerPluginHost` against the committed fixtures, with a stub refresher standing in for the two curated sources that point at `example.invalid` and can never be fetched:

```rust
    /// Serves the committed AQI payload to every plugin card except
    /// `never-fetched`, which yields no value at all. The worker caches a
    /// plugin snapshot only when `value` is `Some`
    /// (`app-core/src/runtime.rs:2056-2073`), so that card is permanently
    /// pre-first-fetch with no timing assumption.
    struct FixtureRefresher {
        payload: serde_json::Value,
    }

    impl app_core::ProviderRefresher for FixtureRefresher {
        fn refresh(
            &mut self,
            request: app_core::ProviderRefreshRequest,
        ) -> app_core::ProviderRefreshResult {
            let fetched = request.widget_id != "never-fetched";
            app_core::ProviderRefreshResult {
                generation: request.generation,
                widget_id: request.widget_id,
                fields: vec![protocol::Field {
                    key: "title".into(),
                    value: protocol::FieldValue::Text(request.title),
                }],
                value: fetched.then(|| self.payload.clone()),
                refreshed_at: fetched.then(chrono::Utc::now),
                age: fetched.then(|| std::time::Duration::ZERO),
                stale: !fetched,
                error: (!fetched).then(|| "the first refresh has not landed".to_owned()),
            }
        }
    }

    fn preview_config() -> app_core::AppConfig {
        // `AppConfig::default()` already carries the `clock` card, which is
        // the built-in the preview route must refuse.
        let mut config = app_core::AppConfig::default();
        for (id, plugin_id) in [
            ("aqi-card", "aqi"),
            ("svg-card", "svg-aqi"),
            ("never-fetched", "aqi"),
            ("absent-plugin", "not-installed"),
        ] {
            config.cards.push(app_core::CardSettings::Plugin {
                id: id.into(),
                title: "Air quality".into(),
                plugin_id: plugin_id.into(),
                tap_action: app_core::WidgetTapAction::None,
                refresh: app_core::RefreshPolicy::Interval { minutes: 15 },
                alert: app_core::CardAlert::None,
            });
            config.playlists[0].entries.push(app_core::PlaylistEntry {
                card_id: id.into(),
                dwell_seconds: None,
            });
        }
        config
    }

    fn state_with_preview_runtime() -> (crate::ServerState, String, tempfile::TempDir) {
        let (state, device_id, config_dir) = state_with_curated_plugins();
        let (device, connector) =
            crate::runtime_device::WebSocketRuntimeDevice::channel(device_id.clone());
        let runtime = Arc::new(
            app_core::RuntimeHandle::start_with_plugin_host(
                preview_config(),
                Box::new(device),
                Box::new(FixtureRefresher {
                    payload: payload_from_envelope(AQI_FIXTURE),
                }),
                app_core::RuntimeOptions::default(),
                Some(Box::new(crate::plugin_host::ServerPluginHost::new(
                    curated_registry(),
                ))),
            )
            .expect("the preview runtime starts"),
        );
        // No socket is ever attached, and the lease is released immediately.
        // A preview must be served by a *retained* runtime, which is exactly
        // what a device between links leaves behind: `is_live()` is false and
        // `runtime()` is still `Some`.
        let lease = state
            .claim_link(device_id.clone())
            .expect("claim the link slot");
        lease.link().set_runtime(runtime, connector);
        drop(lease);
        (state, device_id, config_dir)
    }

    async fn preview_json(
        state: &crate::ServerState,
        device_id: &str,
        card_id: &str,
    ) -> serde_json::Value {
        let response = preview_request(state.clone(), device_id, card_id, true).await;
        assert_eq!(response.status(), StatusCode::OK);
        response_json(response).await
    }

    /// Polls until the first provider result has reached the worker. The
    /// runtime refreshes providers on its own thread, so "waiting" is a real
    /// transient state here rather than an outcome.
    async fn preview_once_settled(
        state: &crate::ServerState,
        device_id: &str,
        card_id: &str,
    ) -> serde_json::Value {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let body = preview_json(state, device_id, card_id).await;
            if body["state"] != "waiting" {
                return body;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the first plugin refresh never reached the runtime"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    fn assert_preview_png(body: &serde_json::Value) {
        // §4.2's invariant: a frame and a message are mutually exclusive.
        assert!(
            body["message"].is_null(),
            "a rendered frame carried a message"
        );
        let png = base64::engine::general_purpose::STANDARD
            .decode(body["png_base64"].as_str().expect("preview frame"))
            .expect("the preview frame is base64");
        let pixmap = resvg::tiny_skia::Pixmap::decode_png(&png).expect("the preview PNG decodes");
        assert_eq!((pixmap.width(), pixmap.height()), (448, 368));
    }

    #[tokio::test]
    async fn a_display_list_plugin_card_previews_as_a_decodable_448x368_png() {
        let (state, device_id, _config_dir) = state_with_preview_runtime();

        let body = preview_once_settled(&state, &device_id, "aqi-card").await;

        assert_eq!(body["state"], "fresh");
        assert_preview_png(&body);
        state.shutdown();
    }

    #[tokio::test]
    async fn an_svg_template_plugin_card_previews_through_the_same_route() {
        // The SVG arm is the one that is exact by construction: this raster
        // *is* what the panel shows.
        let (state, device_id, _config_dir) = state_with_preview_runtime();

        let body = preview_once_settled(&state, &device_id, "svg-card").await;

        assert_eq!(body["state"], "fresh");
        assert_preview_png(&body);
        state.shutdown();
    }

    #[tokio::test]
    async fn a_card_with_no_cached_snapshot_is_waiting_with_a_message_and_no_frame() {
        // The normal pre-first-fetch state, kept distinguishable from a fault.
        let (state, device_id, _config_dir) = state_with_preview_runtime();

        let body = preview_json(&state, &device_id, "never-fetched").await;

        assert_eq!(body["state"], "waiting");
        assert!(body["png_base64"].is_null());
        assert!(
            body["message"].as_str().is_some_and(|m| !m.is_empty()),
            "a stage with no frame must have a word to print"
        );
        state.shutdown();
    }

    #[tokio::test]
    async fn a_plugin_absent_from_the_registry_is_an_error_naming_only_that_plugin() {
        let (state, device_id, _config_dir) = state_with_preview_runtime();

        let body = preview_once_settled(&state, &device_id, "absent-plugin").await;

        assert_eq!(body["state"], "error");
        assert!(body["png_base64"].is_null());
        assert!(
            body["message"]
                .as_str()
                .expect("an error state carries a message")
                .contains("not-installed"),
            "the message must name the plugin: {}",
            body["message"]
        );
        state.shutdown();
    }

    #[tokio::test]
    async fn a_built_in_card_is_not_previewable_here_and_is_a_not_found() {
        // Built-in cards preview in the Mac's own simulator; this route is
        // the server-rendered path only. `RuntimeError::NotAPluginCard`.
        let (state, device_id, _config_dir) = state_with_preview_runtime();

        let response = preview_request(state.clone(), &device_id, "clock", true).await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        state.shutdown();
    }

    #[tokio::test]
    async fn a_card_absent_from_the_configuration_is_a_not_found() {
        // `RuntimeError::UnknownCard`.
        let (state, device_id, _config_dir) = state_with_preview_runtime();

        let response = preview_request(state.clone(), &device_id, "no-such-card", true).await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        state.shutdown();
    }
```

- [ ] **Step 10: Add the transcript collector to the shared test support.** In `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/crates/server/tests/support/mod.rs`, insert after `flush_socket` (which ends at line 264) and before the private `reply` (line 266):

```rust
/// Every card-bearing message the device receives during `window`, in order,
/// answering each one exactly as [`flush_socket`] does.
///
/// Link housekeeping -- `StatusRequest`, `TimeSync`, `Heartbeat` -- is
/// answered but not recorded: it is the transport keeping itself alive, not a
/// face. Everything that can change what the panel shows is recorded. Note
/// `reply` panics on any message it does not know, the four asset-transfer
/// messages included, so an unexpected asset push fails the caller too.
pub async fn card_messages_during(
    socket: &mut DeviceSocket,
    window: std::time::Duration,
) -> Vec<&'static str> {
    let deadline = tokio::time::Instant::now() + window;
    let mut seen = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return seen;
        }
        let Ok(next) = tokio::time::timeout(remaining, socket.next()).await else {
            return seen;
        };
        match next {
            Some(Ok(WsMessage::Binary(bytes))) => {
                let frame = protocol::decode_wire_frame(&bytes).expect("transcript frame");
                let message = protocol::decode_message(&frame).expect("transcript message");
                if let Some(name) = card_message_name(&message) {
                    seen.push(name);
                }
                reply(socket, frame.request_id, &message).await;
            }
            Some(Ok(WsMessage::Ping(payload))) => {
                socket.send(WsMessage::Pong(payload)).await.unwrap();
            }
            Some(Ok(WsMessage::Pong(_))) => {}
            Some(Ok(other)) => panic!("unexpected transcript WebSocket message: {other:?}"),
            Some(Err(error)) => panic!("transcript WebSocket read failed: {error}"),
            None => panic!("socket closed during the transcript window"),
        }
    }
}

fn card_message_name(message: &Message) -> Option<&'static str> {
    match message {
        Message::ApplyConfig(_) => Some("ApplyConfig"),
        Message::PushData(_) => Some("PushData"),
        Message::PushScene(_) => Some("PushScene"),
        Message::ActivateScreen(_) => Some("ActivateScreen"),
        Message::TriggerInterrupt(_) => Some("TriggerInterrupt"),
        _ => None,
    }
}
```

- [ ] **Step 11: Write the failing device-transcript test.** Create `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/crates/server/tests/fixtures/one-plugin-card.json`:

```json
{
  "schema_version": 6,
  "preferences": {
    "timezone": "UTC",
    "autostart": false,
    "paused": false,
    "orientation": "landscape"
  },
  "cards": [
    {
      "kind": "plugin",
      "id": "aqi-card",
      "title": "Air quality",
      "plugin_id": "aqi",
      "tap_action": { "kind": "none" },
      "refresh": { "kind": "interval", "minutes": 15 },
      "alert": { "kind": "none" }
    }
  ],
  "assets": [],
  "playlists": [
    {
      "id": "main",
      "name": "Main",
      "advance": { "kind": "manual" },
      "entries": [
        { "card_id": "aqi-card", "dwell_seconds": null }
      ]
    }
  ],
  "active_playlist_id": "main",
  "updater": { "channel": "stable", "checks": "notify" }
}
```

Then create `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/crates/server/tests/card_preview.rs`:

```rust
//! The admin card-preview route, seen from the device's end of the link.
//!
//! `crates/server/src/admin.rs` covers what the route *returns* for each of
//! §4.2's outcomes, against the real `ServerPluginHost` and the committed
//! fixtures. This file covers the other half of the claim, which no in-crate
//! test can see: rendering a preview adds nothing to the transcript the
//! device receives.

use std::time::Duration;

use server::{ServerState, app};

mod support;

/// Boots a server on a loopback port and mints one device identity.
/// Mirrors `tests/ownership.rs`'s helper; kept separate so the two files can
/// diverge without one silently changing the other's fixture.
async fn spawn() -> (String, server::registry::DeviceIdentity, String) {
    let state = ServerState::in_memory();
    let identity = state.registry().mint().expect("mint identity");
    let admin_token = support::IN_MEMORY_ADMIN_TOKEN.to_string();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(state)).await.unwrap();
    });
    (
        format!("127.0.0.1:{}", address.port()),
        identity,
        admin_token,
    )
}

// The Err type is tungstenite's, so its size is not ours to reduce.
#[allow(clippy::result_large_err)]
async fn connect_device(
    host: &str,
    token: &str,
) -> Result<support::DeviceSocket, tokio_tungstenite::tungstenite::Error> {
    let request = http::Request::builder()
        .uri(format!("ws://{host}/v1/device/link"))
        .header("Authorization", format!("Bearer {token}"))
        .header("Host", host)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tokio_tungstenite::tungstenite::handshake::client::generate_key(),
        )
        .body(())
        .unwrap();
    tokio_tungstenite::connect_async(request)
        .await
        .map(|(socket, _)| socket)
}

async fn admin_json(client: &reqwest::Client, url: String, admin_token: &str) -> serde_json::Value {
    let response = client
        .get(url)
        .bearer_auth(admin_token)
        .send()
        .await
        .expect("admin GET");
    let body = response.text().await.expect("admin response body");
    serde_json::from_str(&body).expect("admin response JSON")
}

/// Waits for the plugin card's one scheduled provider refresh to be applied,
/// answering device traffic between polls. This server has no plugin
/// registry, so the refresh fails immediately and locally -- no egress, no
/// DNS -- and this settles in milliseconds rather than on a network timeout.
async fn settle_first_refresh(
    socket: &mut support::DeviceSocket,
    client: &reqwest::Client,
    host: &str,
    device_id: &str,
    admin_token: &str,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        support::flush_socket(socket).await;
        let status = admin_json(
            client,
            format!("http://{host}/v1/devices/{device_id}"),
            admin_token,
        )
        .await;
        let settled = status["snapshot"]["providers"]
            .as_array()
            .is_some_and(|providers| {
                providers.iter().any(|provider| {
                    provider["widget_id"] == "aqi-card" && provider["state"]["kind"] == "error"
                })
            });
        if settled {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the plugin card's provider never reported: {status}"
        );
    }
}

#[tokio::test]
async fn rendering_a_preview_adds_nothing_to_the_device_transcript() {
    let (host, identity, admin_token) = spawn().await;
    let mut socket = connect_device(&host, &identity.token)
        .await
        .expect("connect");
    let client = reqwest::Client::new();

    // This server's plugin registry is empty, so the card's provider fails
    // without any egress at all and the preview's outcome is `error`. That is
    // deliberate: the curated manifests point at `example.invalid`, so a live
    // socket test could only ever wait on a DNS failure, and what this test
    // measures is the transcript, not the frame. The four render outcomes are
    // covered in `admin.rs` against the real `ServerPluginHost`.
    let config = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/one-plugin-card.json"
    ))
    .expect("fixture");
    let put = client
        .put(format!(
            "http://{host}/v1/devices/{}/config",
            identity.device_id
        ))
        .bearer_auth(&admin_token)
        .header("Content-Type", "application/json")
        .body(config)
        .send();
    let (response, applied) = tokio::join!(
        put,
        support::drive_until_config(&mut socket, "aqi-card")
    );
    assert_eq!(response.expect("config PUT").status(), 200);
    assert_eq!(applied.widgets.len(), 1);

    // The card's one scheduled refresh must land before the windows below, or
    // its `PushData` would be attributed to the preview. Nothing else is due
    // afterwards: the cadence is 15 minutes, the playlist advances manually,
    // and a plugin card with no snapshot has its scene refused once and
    // recorded, never retried.
    settle_first_refresh(
        &mut socket,
        &client,
        &host,
        &identity.device_id,
        &admin_token,
    )
    .await;
    support::flush_socket(&mut socket).await;

    // Control. Without it the empty transcript below would pass just as well
    // against a collector that reads nothing at all.
    let push = client
        .post(format!(
            "http://{host}/v1/devices/{}/scene",
            identity.device_id
        ))
        .bearer_auth(&admin_token)
        .header("Content-Type", "application/json")
        .body(
            serde_json::json!({
                "card_id": "aqi-card",
                "revision": 17,
                "template": "digital_clock",
                "show_seconds": false,
                "local_now": "2026-08-25T14:37:42",
            })
            .to_string(),
        )
        .send();
    let (pushed, control) = tokio::join!(
        push,
        support::card_messages_during(&mut socket, Duration::from_millis(750))
    );
    assert_eq!(pushed.expect("scene push").status(), 200);
    assert_eq!(control, vec!["PushScene"]);

    // The measurement: five previews, and the device sees nothing.
    let previews = async {
        for _ in 0..5 {
            let body = admin_json(
                &client,
                format!(
                    "http://{host}/v1/devices/{}/cards/aqi-card/preview",
                    identity.device_id
                ),
                &admin_token,
            )
            .await;
            assert_eq!(body["state"], "error", "unexpected preview body: {body}");
            assert!(body["png_base64"].is_null());
        }
    };
    let ((), transcript) = tokio::join!(
        previews,
        support::card_messages_during(&mut socket, Duration::from_millis(750))
    );
    assert_eq!(transcript, Vec::<&'static str>::new());
}
```

- [ ] **Step 12: Run every preview test and watch them fail.**

```sh
cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p server --lib admin::tests && /Users/rodion/.cargo/bin/cargo test -p server --test card_preview
```

The router has no path with that many segments, so axum answers `404` (not `405` — the method matches nothing because the *path* matches nothing). Expect `left: 404, right: 503` in `a_preview_before_any_runtime_exists_is_service_unavailable`, `left: 404, right: 401` in `the_preview_route_requires_admin_authentication`, `left: 404, right: 200` in the four end-to-end bodies, and `unexpected preview body: null` in `card_preview`. **Three of the eleven pass vacuously here** — `a_preview_for_an_unknown_device_is_a_not_found`, `a_built_in_card_is_not_previewable_here_and_is_a_not_found` and `a_card_absent_from_the_configuration_is_a_not_found` all assert `404` and get the router's own `404`. They stop being vacuous at Step 14, where the route exists and each `404` has to be earned; Step 22's fifth probe is what pins that.

- [ ] **Step 13: Lift `frame_png` out of the test module.** In `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/crates/server/src/rasterizer.rs`, delete lines 2047-2068 (the whole `pub(crate) fn frame_png` item) and paste this at file scope immediately after `pack_rgb565` (which ends at line 1176), before `#[cfg(test)]` at 1178:

```rust
/// The decoded RGB565 blob as a PNG, for the admin API's card preview.
/// `assert_raster_regression` uses the same encoder, so a golden and a
/// preview are the same bytes for the same frame. Takes the canonical
/// 12-byte-header blob by reference: a preview must not copy 330 KB to
/// rebuild a `RasterizedFrame` it already holds the bytes of.
pub(crate) fn frame_png(bytes: &[u8]) -> Vec<u8> {
    let width = u32::try_from(SCENE_CANVAS_WIDTH).unwrap();
    let height = u32::try_from(SCENE_CANVAS_HEIGHT).unwrap();
    let mut rgba = Vec::with_capacity(usize::try_from(width * height * 4).unwrap());
    for pair in bytes[12..].as_chunks::<2>().0 {
        let pixel = u16::from_le_bytes(*pair);
        let red = u8::try_from((pixel >> 11) & 0x1f).unwrap();
        let green = u8::try_from((pixel >> 5) & 0x3f).unwrap();
        let blue = u8::try_from(pixel & 0x1f).unwrap();
        rgba.extend_from_slice(&[
            u8::try_from(u16::from(red) * 255 / 31).unwrap(),
            u8::try_from(u16::from(green) * 255 / 63).unwrap(),
            u8::try_from(u16::from(blue) * 255 / 31).unwrap(),
            255,
        ]);
    }
    let size = resvg::tiny_skia::IntSize::from_wh(width, height).unwrap();
    resvg::tiny_skia::Pixmap::from_vec(rgba, size)
        .unwrap()
        .encode_png()
        .unwrap()
}
```

Then re-point its one caller: in `assert_raster_regression` (line 2070 before the deletion, 2048 after), change

```rust
        let actual = frame_png(frame);
```

to

```rust
        let actual = frame_png(&frame.bytes);
```

- [ ] **Step 14: Add the route and the handler.** In `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/crates/server/src/admin.rs`, add `use base64::Engine as _;` after line 22's `use crate::plugin_registry::{LoadedPlugin, PluginRegistry};`, and extend the app-core import with `CardPreviewResponse` (alphabetically first in that list, before `MAX_CONFIG_FILE_BYTES`):

```rust
use app_core::{
    AdminConfigErrorBody, AppConfig, AppSnapshot, BakedFontMetrics, CardPreviewResponse, ClockCard,
    MAX_CONFIG_FILE_BYTES, PluginCatalog, PluginCatalogAsset, PluginCatalogEntry,
    PluginTemplateKind, RuntimeError, SaveReceipt, StoreError, ValidationIssue,
    build_digital_clock_scene,
};
```

Replace `routes()` (lines 28-38) with:

```rust
pub(crate) fn routes() -> Router<ServerState> {
    Router::new()
        .route("/v1/devices", post(create_device))
        .route("/v1/plugins", get(get_plugins))
        .route("/v1/devices/{id}", get(get_device))
        .route(
            "/v1/devices/{id}/config",
            put(put_config).layer(DefaultBodyLimit::max(MAX_CONFIG_FILE_BYTES)),
        )
        .route("/v1/devices/{id}/scene", post(post_scene))
        .route(
            "/v1/devices/{id}/cards/{card_id}/preview",
            get(get_card_preview),
        )
}
```

Add the handler immediately after `get_device` (which ends at line 438), before `struct DeviceStatus`:

```rust
/// Renders one plugin card's current face as a PNG, without touching the
/// device. The runtime builds it from the snapshot the panel is already
/// showing, at revision 0: nothing is minted, nothing is sent, and the card
/// is not activated. Rate limiting is the Mac's polling cadence, not ours.
async fn get_card_preview(
    State(state): State<ServerState>,
    _admin: AdminAuthenticated,
    Path((device_id, card_id)): Path<(String, String)>,
) -> Result<Json<CardPreviewResponse>, AdminError> {
    if !state.registry().contains_device(&device_id) {
        return Err(AdminError::NotFound);
    }
    let runtime = state
        .device_link(&device_id)
        .and_then(|link| link.runtime())
        .ok_or_else(|| AdminError::Runtime {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: "this device has no runtime yet; it has never connected".into(),
        })?;
    let preview = tokio::task::spawn_blocking(move || runtime.render_card_preview(&card_id))
        .await
        .map_err(|_| AdminError::WorkerFailed)?
        .map_err(AdminError::from)?;

    Ok(Json(CardPreviewResponse {
        png_base64: preview.frame.as_ref().map(|frame| {
            base64::engine::general_purpose::STANDARD
                .encode(crate::rasterizer::frame_png(&frame.bytes))
        }),
        state: preview.state,
        message: preview.message,
        refreshed_at_unix_ms: preview.refreshed_at_unix_ms,
    }))
}
```

The `AdminError::from` on the `render_card_preview` result needs no work here: Step 1 already mapped `UnknownCard` and `NotAPluginCard` onto `AdminError::NotFound`, which is what turns the two `404` assertions written in Step 9 from vacuous into earned.

- [ ] **Step 15: Run to pass, including the raster goldens the lift must not disturb.**

```sh
cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p server --lib admin::tests rasterizer:: && /Users/rodion/.cargo/bin/cargo test -p server --test card_preview
```

Expect `test result: ok.` for all three filters. The `raster_regression_*` goldens confirm `frame_png` moved and changed signature without changing a byte.

- [ ] **Step 16: Commit.**

```sh
git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -am "$(cat <<'EOF'
feat: add the admin card preview route and lift frame_png out of tests

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S
EOF
)"
```

- [ ] **Step 17: Write the failing hero-field tests.** In `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/crates/server/src/plugin_refresher.rs`, first make the existing helper reusable — replace `refresher_with` (lines 196-203) with:

```rust
    fn refresher_with_registry(
        registry: Arc<PluginRegistry>,
        responses: Vec<Result<FetchResponse, EgressError>>,
    ) -> ServerProviderRefresher<FakeFetcher> {
        let fetcher = FakeFetcher {
            responses: Arc::new(Mutex::new(responses.into())),
        };
        ServerProviderRefresher::with_fetcher_factory(registry, move || fetcher.clone())
    }

    fn refresher_with(
        responses: Vec<Result<FetchResponse, EgressError>>,
    ) -> ServerProviderRefresher<FakeFetcher> {
        refresher_with_registry(registry(), responses)
    }
```

Then append to the same `mod tests` block:

```rust
    fn fixture_request(widget_id: &str) -> ProviderRefreshRequest {
        ProviderRefreshRequest {
            generation: 7,
            widget_id: widget_id.into(),
            title: "Air quality".into(),
            provider: ProviderRequest::Plugin {
                plugin_id: "fixture".into(),
            },
            active_provider_ids: vec![widget_id.into()],
            now: Utc.with_ymd_and_hms(2026, 8, 29, 12, 0, 0).unwrap(),
        }
    }

    #[test]
    fn a_declared_summary_is_published_as_the_hero_field_after_the_title() {
        let (registry, _base) =
            crate::test_plugins::fixture_registry(crate::test_plugins::V2_NAMED_MANIFEST);
        let mut refresher =
            refresher_with_registry(registry, vec![Ok(ok_response(br#"{"current":{"aqi":42}}"#))]);

        let result = refresher.refresh(fixture_request("fixture-card"));

        // Order matters only in that `title` keeps its existing position:
        // `hero` is additive, and a reader that only knows `title` is unharmed.
        assert_eq!(
            result.fields,
            vec![
                Field {
                    key: "title".into(),
                    value: FieldValue::Text("Air quality".into()),
                },
                Field {
                    key: "hero".into(),
                    value: FieldValue::Text("42".into()),
                },
            ]
        );
    }

    #[test]
    fn a_manifest_without_a_summary_publishes_the_title_alone() {
        // The tile then shows "—", like a weather card with no data.
        let (registry, _base) =
            crate::test_plugins::fixture_registry(crate::test_plugins::V2_UNNAMED_MANIFEST);
        let mut refresher =
            refresher_with_registry(registry, vec![Ok(ok_response(br#"{"current":{"aqi":42}}"#))]);

        let result = refresher.refresh(fixture_request("fixture-card"));

        assert_eq!(result.fields, vec![title_field("Air quality".into())]);
    }

    #[test]
    fn a_summary_that_fails_to_evaluate_drops_the_hero_rather_than_the_refresh() {
        let (registry, _base) =
            crate::test_plugins::fixture_registry(crate::test_plugins::V2_NAMED_MANIFEST);
        // One byte past `expr::MAX_OUTPUT_LEN` is `ExprError::OutputTooLong`,
        // which the evaluator rejects rather than truncating.
        let oversized = format!(
            r#"{{"current":{{"aqi":"{}"}}}}"#,
            "9".repeat(plugin::MAX_OUTPUT_LEN + 1)
        );
        let mut refresher = refresher_with_registry(
            registry,
            vec![
                Ok(ok_response(oversized.as_bytes())),
                Ok(ok_response(oversized.as_bytes())),
            ],
        );

        let result = refresher.refresh(fixture_request("fixture-card"));

        assert_eq!(result.fields, vec![title_field("Air quality".into())]);
        assert!(!result.stale, "a summary failure poisoned the fetched data");
        assert!(
            result.value.is_some(),
            "a summary failure discarded the payload"
        );

        // Once per plugin id, not once per refresh: this fires every cadence.
        refresher.refresh(fixture_request("fixture-card"));
        assert_eq!(refresher.summary_failures.len(), 1);
        assert!(refresher.summary_failures.contains("fixture"));
    }
```

- [ ] **Step 18: Run them and watch them fail.**

```sh
cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p server --lib plugin_refresher::tests
```

Expect `error[E0609]: no field 'summary_failures' on type 'ServerProviderRefresher<FakeFetcher>'` from the third test, and — once that line is the only failure left to fix — `left: [Field { key: "title", .. }]` against a two-entry right-hand side in `a_declared_summary_is_published_as_the_hero_field_after_the_title`.

- [ ] **Step 19: Emit the hero field.** In `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/crates/server/src/plugin_refresher.rs`, change line 3 to `use std::collections::{HashMap, HashSet};`, then replace the struct (lines 17-22) with:

```rust
pub struct ServerProviderRefresher<F: PluginFetcher = SystemPluginFetcher> {
    system: SystemProviderRefresher,
    registry: Arc<PluginRegistry>,
    plugins: HashMap<String, PluginProviderEntry<F>>,
    fetcher_factory: Box<dyn FnMut() -> F + Send>,
    /// Plugin ids whose `summary` has already been reported as failing. A
    /// broken expression fails every cadence, forever; one line is a defect
    /// report, one line every ten minutes is a log flood.
    summary_failures: HashSet<String>,
}
```

Replace the existing `impl<F> ServerProviderRefresher<F>` block (lines 36-51) with:

```rust
impl<F> ServerProviderRefresher<F>
where
    F: PluginFetcher + Send + 'static,
{
    fn with_fetcher_factory(
        registry: Arc<PluginRegistry>,
        fetcher_factory: impl FnMut() -> F + Send + 'static,
    ) -> Self {
        Self {
            system: SystemProviderRefresher::default(),
            registry,
            plugins: HashMap::new(),
            fetcher_factory: Box::new(fetcher_factory),
            summary_failures: HashSet::new(),
        }
    }

    /// The manifest's `summary`, evaluated against what was just fetched.
    /// A failure costs the tile its headline and nothing else: the fetched
    /// data, the freshness state and the title all still reach the card.
    fn evaluate_summary(
        &mut self,
        plugin_id: &str,
        snapshot: &providers::ProviderSnapshot<serde_json::Value>,
    ) -> Option<String> {
        let registry = Arc::clone(&self.registry);
        let plugin = registry.get(plugin_id)?;
        match plugin::evaluate_summary(&plugin.manifest, snapshot) {
            Ok(summary) => summary,
            Err(error) => {
                if self.summary_failures.insert(plugin_id.to_owned()) {
                    tracing::warn!(
                        plugin_id,
                        %error,
                        "plugin summary could not be evaluated; the card tile will show no headline"
                    );
                }
                None
            }
        }
    }
}
```

Replace the whole `impl<F> ProviderRefresher for ServerProviderRefresher<F>` block (lines 53-109) with:

```rust
impl<F> ProviderRefresher for ServerProviderRefresher<F>
where
    F: PluginFetcher + Send + 'static,
{
    fn refresh(&mut self, request: ProviderRefreshRequest) -> ProviderRefreshResult {
        let plugin_id = match &request.provider {
            ProviderRequest::Plugin { plugin_id, .. } => plugin_id.clone(),
            _ => return self.system.refresh(request),
        };

        self.plugins
            .retain(|widget_id, _| request.active_provider_ids.contains(widget_id));

        let needs_replacement = self
            .plugins
            .get(&request.widget_id)
            .is_none_or(|entry| entry.plugin_id != plugin_id);
        if needs_replacement {
            let Some(plugin) = self.registry.get(&plugin_id) else {
                return plugin_error_result(request, format!("plugin {plugin_id:?} is not loaded"));
            };
            let provider =
                match PluginDataProvider::new((self.fetcher_factory)(), &plugin.manifest.source) {
                    Ok(provider) => provider,
                    Err(error) => {
                        return plugin_error_result(
                            request,
                            format!("plugin {plugin_id:?} provider cannot start: {error}"),
                        );
                    }
                };
            self.plugins.insert(
                request.widget_id.clone(),
                PluginProviderEntry {
                    // Cloned, not moved: `plugin_id` is still needed below to
                    // evaluate the summary, and this arm is conditional.
                    plugin_id: plugin_id.clone(),
                    provider,
                },
            );
        }

        // Scoped so the `&mut self.plugins` borrow ends before
        // `evaluate_summary` takes `&mut self`.
        let snapshot = {
            let entry = self
                .plugins
                .get_mut(&request.widget_id)
                .expect("plugin provider entry was inserted above");
            entry.provider.refresh(request.now)
        };
        let hero = self.evaluate_summary(&plugin_id, &snapshot);
        let mut fields = vec![title_field(request.title)];
        fields.extend(hero.map(hero_field));
        ProviderRefreshResult {
            generation: request.generation,
            widget_id: request.widget_id,
            fields,
            value: Some(snapshot.value),
            refreshed_at: snapshot.refreshed_at,
            age: snapshot.age,
            stale: snapshot.stale,
            error: snapshot.error,
        }
    }
}
```

Add the field constructor immediately after `title_field` (which ends at line 116 before this edit):

```rust
/// The tile's live value. This rides the wire in the card's `PushData` and
/// the device drops it: a plugin card's `WidgetConfig.template` is always
/// `TemplateKind::DigitalClock` (`app-core/src/config.rs`'s `wire_config`),
/// whose registry declares only `title`/`show_seconds`/`stale`/`error`, and
/// `firmware/main/core/template_fields.h:59` states that unknown fields are
/// ignored. It is not free -- the device counts it in `unknown_field_count`
/// on every refresh -- but it is safe, and the field exists for the Mac's
/// tile, which reads it out of `card_data`.
fn hero_field(summary: String) -> Field {
    Field {
        key: "hero".into(),
        value: FieldValue::Text(summary),
    }
}
```

`plugin_error_result` is unchanged: there is no snapshot to evaluate a summary against. One existing test now asserts the wrong thing, because its cached last-good payload *is* what the curated `aqi` summary reads — in `title_or_card_cadence_edits_keep_last_good_until_the_plugin_id_changes`, replace lines 328-331:

```rust
        // The cached payload is still `{"current":{"aqi":42}}` and the curated
        // `aqi` manifest's summary reads exactly that, so a last-good refresh
        // publishes a last-good headline too.
        assert_eq!(
            after_title_edit.fields,
            vec![title_field("Outside air".into()), hero_field("42".into())]
        );
```

- [ ] **Step 20: Run to pass.**

```sh
cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p server --lib plugin_refresher::tests
```

Expect `test result: ok.` with eight tests. `successful_fetch_returns_raw_value_and_title_field` still asserts `fields == [title]` and still passes, for a reason worth knowing rather than assuming: it feeds the *raw* `AQI_FIXTURE` body, which is the `{"status":"ok","payload":{...}}` envelope, and the curated `aqi` manifest has no `[source] root`, so `data.current` is absent, `evaluate_summary` returns `Ok(None)`, and no `hero` is emitted. That is the same envelope mismatch `crates/plugin/tests/aqi_fixture.rs:23-26` records; it is not a defect introduced here and must not be "fixed" by editing either the fixture or the manifest.

- [ ] **Step 21: Commit.**

```sh
git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -am "$(cat <<'EOF'
feat: publish a plugin manifest's summary as the card's hero field

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S
EOF
)"
```

- [ ] **Step 22: Mutation-probe the five new bounds.** Per spec §9, before running the full gates, confirm each new assertion is load-bearing rather than vacuous. Apply each edit, run the named test, confirm it **fails**, then revert:
  1. In `catalog_entry`, swap `PluginTemplateKind::DisplayList` and `PluginTemplateKind::Svg` → `the_catalog_reports_each_manifests_template_kind_and_cadence` must fail.
  2. In `get_card_preview`, replace `StatusCode::SERVICE_UNAVAILABLE` with `StatusCode::BAD_GATEWAY` → `a_preview_before_any_runtime_exists_is_service_unavailable` must fail.
  3. In `evaluate_summary`, change the `if self.summary_failures.insert(plugin_id.to_owned())` condition to `if true` → `a_summary_that_fails_to_evaluate_drops_the_hero_rather_than_the_refresh` must fail on the `summary_failures.len()` assertion.
  4. In `support::card_message_name`, add `Message::StatusRequest => Some("StatusRequest"),` as the first arm → `rendering_a_preview_adds_nothing_to_the_device_transcript` must fail on the final `assert_eq!`, proving that window observes live traffic rather than passing because it read nothing.
  5. In `From<RuntimeError> for AdminError`, change the `UnknownCard | NotAPluginCard` arm to `Self::Runtime { status: StatusCode::UNPROCESSABLE_ENTITY, message: error.to_string() }` → both `a_card_absent_from_the_configuration_is_a_not_found` and `a_built_in_card_is_not_previewable_here_and_is_a_not_found` must fail, which is what makes those two 404 assertions non-vacuous now that the route exists.

```sh
cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity && git diff --stat && git status --short
```

Expect an empty diff after the five reverts.

- [ ] **Step 23: Run the workspace gates.**

```sh
cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo fmt --all --check
cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings
cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test --workspace --all-targets
cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test --workspace --doc
```

`--all-targets` covers `crates/server/tests/*` — `ownership.rs`, `hostile_device.rs`, `device_link.rs` and the new `card_preview.rs` must all stay green: the catalog body's existing keys are unchanged, and nothing new is sent to a device, so `hostile_device.rs`'s sequence assertions must not move. `--doc` is required separately because `--all-targets` removes doctests.

- [ ] **Step 24: Commit any formatting the gates produced.**

```sh
git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -am "$(cat <<'EOF'
chore: format the server catalog, preview and refresher changes

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S
EOF
)"
```

**Task exit:** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --all-targets` and `cargo test --workspace --doc` are all green from `companion/`; Task 2's deliberate `cargo check -p server` break is closed, with `RuntimeError::UnknownCard` and `RuntimeError::NotAPluginCard` mapped onto the existing unit `AdminError::NotFound`; `GET /v1/plugins` serves `app_core::admin::PluginCatalog` — no local DTO remains in `server/src/admin.rs` — with every pre-existing field byte-identical and the five added ones populated from the manifest; `GET /v1/devices/{id}/cards/{card_id}/preview` returns a decodable 448×368 PNG for a display-list and an SVG plugin, each of §4.2's four non-frame outcomes, a bare `404` for an unknown device, an unknown card and a non-plugin card, a `503` for a device with no runtime, and `401` unauthenticated; `rendering_a_preview_adds_nothing_to_the_device_transcript` shows the device an exactly empty card transcript across five previews on a link the same collector proves it can see a `PushScene` on; and a manifest's `summary` reaches the card as its `hero` field, with an evaluation failure costing the headline, logged once per plugin id, and nothing else. No frontend gates apply to this task.

---

### Task 4: The Mac talks back to the server

**Files:**
- Create: `companion/apps/deskmate/src-tauri/src/server_client.rs`
- Modify: `companion/apps/deskmate/src-tauri/src/commands.rs` (read 25 for `MAX_SERVER_ERROR_BYTES`; 53–99 for the `*Context::from_desktop` shape; 133–141 for `PreviewFrame`; 465–523 for `save_server_config`/`save_server_config_blocking`; 601–620 for `SaveDestination`/`save_destination`; 727–803 for `render_card_preview` and `preview_template_for`; 1124–1243 for `validate_server_url`, `server_config_url`, `put_server_config`, `server_failure`; 1316–1380 for the test module head, `scratch_directory` and `read_http_request`; 2065–2209 for the three loopback tests; 2252–2298 for `ContractFixtures`/`contract_card_kind`; 2735–2752 for the tail of `contract_fixtures()`; 2771–2798 for the sync/printer tests)
- Modify: `companion/apps/deskmate/src-tauri/src/lib.rs` (read 20–22 for the module list; 105–130 for `NetworkedConfigProjection`; 132–176 for `DesktopSnapshotProjector::project_snapshot`; 178–201 for `DesktopState`; 431–456 for `setup_app`'s `app.manage`; 480–488 for `server_http_agent`; 549–566 for `generate_handler!`; 595–765 for the test module and the two projection tests that model these)
- Modify: `companion/apps/deskmate/src/lib/types.contract.ts` (939 lines, **generated** — regenerated by Step 12's command, never hand-edited)

**This task is Rust only.** It does not touch `src/lib/types.ts`, `src/lib/tauri.ts`, `src/components/DevicePreview.tsx` or anything under `src/dev/` — Task 5 owns every hand-written TypeScript file. The consequence is deliberate and stated in the Task exit: from Step 11 until Task 5 lands, `bun run check` fails on `types.contract.ts` because `IpcContractFixtures` does not yet declare `plugin_catalog` / `server_card_state`. Do not "fix" that by editing `types.ts` or by hand-editing the generated fixture.

**Interfaces:**

*Consumes* (Tasks 1–3 must be merged first):
- `app_core::admin::{PluginCatalog, PluginCatalogEntry, PluginCatalogAsset, PluginLoadFailure, PluginTemplateKind, CardPreviewState, CardPreviewResponse}` — created by **Task 2**, all `Serialize + Deserialize`. Every reference in this task is written path-qualified through `app_core::admin::`, never through a crate-root re-export, so the `PluginLoadFailure` exclusion in the re-export list cannot break it.
  - `PluginCatalog { plugins: Vec<PluginCatalogEntry>, load_failures: Vec<PluginLoadFailure> }`
  - `PluginCatalogEntry { id, name, version, node_count, assets, display_name: Option<String>, description: Option<String>, manifest_version, template: PluginTemplateKind, refresh_minutes }` — the first five keys keep `PluginResponse`'s existing order and JSON (`server/src/admin.rs:96-102`).
  - `PluginCatalogAsset { file, kind, byte_length, digest }`; `PluginLoadFailure { id, error }`; `PluginTemplateKind::{DisplayList, Svg}` → `"display-list" | "svg"`.
  - `CardPreviewResponse { png_base64: Option<String>, state: CardPreviewState, message: Option<String>, refreshed_at_unix_ms: Option<u64> }`; `CardPreviewState::{Fresh, Stale, Error, Waiting}` → lowercase (`"fresh"`, `"stale"`, `"error"`, `"waiting"`).
- Server routes (**Task 3**): `GET /v1/plugins -> PluginCatalog`, `GET /v1/devices/{id}/cards/{card_id}/preview -> CardPreviewResponse`.
- `GET /v1/devices/{id}` JSON (`server/src/admin.rs:404-462`): `{ device_id, connected, last_seen_unix_ms, config: { origin, using_fallback, fallback_reason }, snapshot: null | { config, runtime, device, providers[], pomodoros[], card_data[], card_errors[], persistence, diagnostics } }`. `AdminSnapshot` flattens `AppSnapshot` and injects `device.last_ota_error` / `device.observed_age_seconds`.
- Existing in this crate: `commands::IpcError`, `commands::server_failure`, `commands::validate_server_url`, `commands::validate_target`, `commands::save_destination`, `commands::SaveDestination`, `commands::MAX_SERVER_ERROR_BYTES`, `crate::server_http_agent`, `crate::NetworkedConfigProjection`, `app_core::NetworkSettingsStore::{load, with_admin_token}`.

*Produces*:
```rust
// server_client.rs
pub(crate) const MAX_SERVER_PREVIEW_BYTES: usize = 1_024 * 1_024;
pub(crate) fn server_url(base: &str, path_segments: &[&str]) -> Result<url::Url, IpcError>;
pub(crate) fn get_server_json<T: serde::de::DeserializeOwned>(
    agent: &ureq::Agent, url: &str, admin_token: &str, max_bytes: usize,
) -> Result<T, IpcError>;

// commands.rs
pub struct PreviewFrame { pub png_base64: Option<String>, pub sample: bool, pub state: Option<String> }
pub struct ServerCardState {
    pub card_id: String, pub provider: app_core::ProviderState,
    pub hero: Option<String>, pub errors: Vec<app_core::CardError>,
}
pub(crate) const PLUGIN_RENDERS_ON_THE_SERVER: &str = "Plugin cards render on the server";
pub(crate) const PLUGIN_PREVIEW_NEEDS_A_NEWER_SERVER: &str = "Plugin previews need a newer server";
#[tauri::command] pub async fn get_server_plugins(state) -> Result<app_core::admin::PluginCatalog, IpcError>;
#[tauri::command] pub async fn get_server_card_state(state) -> Result<Vec<ServerCardState>, IpcError>;
#[tauri::command] pub async fn render_card_preview(state, card_id: String) -> Result<PreviewFrame, IpcError>;

// lib.rs
struct ServerStateProjection(Mutex<Vec<commands::ServerCardState>>);
```

---

- [ ] **Step 1: Write the failing bounded-GET tests, and only them.** Create `companion/apps/deskmate/src-tauri/src/server_client.rs` with nothing but its test module, and add `mod server_client;` to `lib.rs` after `mod preview;` (line 22). The three cases are the ones every server GET must survive: the authenticated 200, the rejected token, and a body that outgrows its bound. They reuse the existing harness rather than growing a second one, so first widen it in `commands.rs`: line 1317 `mod tests {` → `pub(crate) mod tests {`, and line 1350 `    fn read_http_request(` → `    pub(crate) fn read_http_request(`. Nothing else in `commands.rs` changes in this step.

```rust
//! One place where a Deskmate server GET is built, read and bounded.
//!
//! Every admin call is bearer-authenticated and reads at most a stated number of
//! bytes before parsing, so an unreachable or hostile server can waste bytes but
//! never memory. The typed failure mapping is `commands::server_failure`, shared
//! with the config PUT: a 401 says the token was rejected in exactly one voice.

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use crate::commands::IpcError;
    use crate::commands::tests::read_http_request;

    use super::{get_server_json, server_url};

    /// Serves one request, replies with `status` and `body`, and hands back the
    /// bytes the client actually sent so a test can assert the method, path and
    /// Authorization header rather than trusting the client's own report.
    fn serve_once(
        status: &'static str,
        body: Vec<u8>,
    ) -> (String, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = String::from_utf8_lossy(&read_http_request(&mut stream)).to_string();
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
            request
        });
        (format!("http://{address}"), handle)
    }

    #[test]
    fn server_url_appends_admin_segments_and_drops_query_and_fragment() {
        assert_eq!(
            server_url("https://desk.example/base/?a=1#f", &["v1", "plugins"])
                .unwrap()
                .as_str(),
            "https://desk.example/base/v1/plugins"
        );
        // A segment that tries to escape into another route is percent-encoded,
        // not obeyed: `url` encodes `/` as `%2F` inside a pushed segment.
        assert_eq!(
            server_url("https://desk.example", &["v1", "devices", "a b/c", "config"])
                .unwrap()
                .as_str(),
            "https://desk.example/v1/devices/a%20b%2Fc/config"
        );
        assert!(server_url("not-a-url", &["v1"]).is_err());
    }

    #[test]
    fn a_bearer_authenticated_get_returns_the_parsed_body() {
        let (base, server) = serve_once("200 OK", br#"{"value":"ok"}"#.to_vec());
        let url = server_url(&base, &["v1", "plugins"]).unwrap().to_string();

        let parsed: serde_json::Value = get_server_json(
            &crate::server_http_agent(),
            &url,
            "admin-secret",
            crate::commands::MAX_SERVER_ERROR_BYTES,
        )
        .unwrap();

        let request = server.join().unwrap();
        assert!(request.starts_with("GET /v1/plugins "));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bearer admin-secret")
        );
        assert_eq!(parsed["value"], "ok");
    }

    #[test]
    fn a_rejected_admin_token_keeps_the_one_shared_token_message() {
        let (base, server) = serve_once("401 Unauthorized", Vec::new());
        let url = server_url(&base, &["v1", "plugins"]).unwrap().to_string();

        let error = get_server_json::<serde_json::Value>(
            &crate::server_http_agent(),
            &url,
            "wrong",
            crate::commands::MAX_SERVER_ERROR_BYTES,
        )
        .unwrap_err();

        server.join().unwrap();
        assert!(matches!(
            error,
            IpcError::InvalidPayload { message } if message == "the server rejected the admin token"
        ));
    }

    /// The bound is enforced while reading, so an oversized body is never fully
    /// allocated and never reaches `serde_json`. 522 bytes against a 512-byte
    /// bound: one byte past the limit is enough to refuse the whole response.
    #[test]
    fn an_oversized_body_is_refused_before_it_is_parsed() {
        let bound = 512;
        let body = format!(r#"{{"pad":"{}"}}"#, "x".repeat(bound)).into_bytes();
        assert!(body.len() > bound);
        let (base, server) = serve_once("200 OK", body);
        let url = server_url(&base, &["v1", "plugins"]).unwrap().to_string();

        let error =
            get_server_json::<serde_json::Value>(&crate::server_http_agent(), &url, "t", bound)
                .unwrap_err();

        server.join().unwrap();
        assert!(matches!(
            error,
            IpcError::RuntimeUnavailable { message } if message.contains("oversized")
        ));
    }
}
```
Run `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p deskmate-app --lib server_client` — expect `error[E0432]: unresolved imports super::get_server_json, super::server_url`.

- [ ] **Step 2: Implement the URL builder and the bounded GET.** Add the code below above the test module in `server_client.rs`. In `commands.rs` make three items reachable — line 25 `const MAX_SERVER_ERROR_BYTES` → `pub(crate) const MAX_SERVER_ERROR_BYTES`, line 1124 `fn validate_server_url` → `pub(crate) fn validate_server_url`, line 1219 `fn server_failure` → `pub(crate) fn server_failure` — and rewrite `server_config_url` (commands.rs:1163-1181) to delegate, so there is one segment builder in the app.

```rust
use serde::de::DeserializeOwned;

use crate::commands::{IpcError, server_failure, validate_server_url};

/// A 448x368 RGB565 frame encoded as PNG is far under this; the bound exists so a
/// server that answers a preview with a firehose is refused, not swallowed.
pub(crate) const MAX_SERVER_PREVIEW_BYTES: usize = 1_024 * 1_024;

/// Appends admin path segments to the operator's base URL. Query and fragment are
/// dropped for the same reason `device_link_url` drops them: an admin base is a
/// prefix, not a request. Segments are percent-encoded by `url` (`/` becomes
/// `%2F`), so a device or card id containing a slash cannot escape into another
/// route. `url` silently drops a `.` or `..` segment, which can only ever shorten
/// the path into a route that does not exist -- a 404, never another device.
pub(crate) fn server_url(base: &str, path_segments: &[&str]) -> Result<url::Url, IpcError> {
    let mut url = validate_server_url(base)?;
    url.set_query(None);
    url.set_fragment(None);
    let mut segments = url
        .path_segments_mut()
        .map_err(|()| IpcError::InvalidPayload {
            message: "server URL cannot be used as a base URL".into(),
        })?;
    segments.pop_if_empty().extend(path_segments);
    drop(segments);
    Ok(url)
}

/// Reads at most `max_bytes` of an admin GET body, then maps the status through the
/// same `server_failure` the config PUT uses. The body is read before the status is
/// judged, because a 422 carries its validation details in it.
///
/// The bound is enforced by `ureq`'s own limited reader rather than by measuring the
/// body afterwards: a limit of `max_bytes + 1` accepts a body of exactly `max_bytes`
/// and fails on the first byte past it, so an oversized response is never fully
/// allocated and never parsed. `BodyExceedsLimit` is the one read failure with its
/// own sentence, because it is the only one that says something about the server
/// rather than about the socket.
pub(crate) fn get_server_json<T: DeserializeOwned>(
    agent: &ureq::Agent,
    url: &str,
    admin_token: &str,
    max_bytes: usize,
) -> Result<T, IpcError> {
    let mut response = agent
        .get(url)
        .header("Authorization", format!("Bearer {admin_token}"))
        .call()
        .map_err(|_| IpcError::RuntimeUnavailable {
            message: "the configured server could not be reached".into(),
        })?;
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .with_config()
        .limit(max_bytes.saturating_add(1) as u64)
        .read_to_vec()
        .map_err(|error| match error {
            ureq::Error::BodyExceedsLimit(_) => IpcError::RuntimeUnavailable {
                message: "the server returned an oversized response".into(),
            },
            _ => IpcError::RuntimeUnavailable {
                message: "the server returned an unreadable response".into(),
            },
        })?;
    if !(200..300).contains(&status) {
        return Err(server_failure(status, &body));
    }
    serde_json::from_slice(&body).map_err(|_| IpcError::RuntimeUnavailable {
        message: "the server returned a response this app could not read".into(),
    })
}
```
and in `commands.rs`, replace the whole of `server_config_url` (lines 1163-1181):
```rust
fn server_config_url(server_url: &str, device_id: &str) -> Result<url::Url, IpcError> {
    validate_target(device_id, MAX_DEVICE_ID_LEN, "device ID")?;
    crate::server_client::server_url(server_url, &["v1", "devices", device_id, "config"])
}
```
and, in `server_failure`, make the catch-all arm operation-neutral now that GETs share it — replace line 1240:
```rust
        _ => IpcError::RuntimeUnavailable {
            message: format!("the server rejected the request (HTTP {status})"),
        },
```
Run `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p deskmate-app --lib server_client server_config_url server_invalid_config server_transport` — all green, including the untouched `server_config_url_preserves_the_base_path_and_encodes_the_device_id` (commands.rs:1729). Commit:
```sh
git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -am "feat: share one bounded, bearer-authenticated server GET

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
```

- [ ] **Step 3: Write the failing `PreviewFrame` outcome-mapping test.** Every §4.2 outcome and the two Mac-side outcomes get exactly one row, added at the end of `commands.rs`'s test module (before `fn typescript_contract_source`, commands.rs:2771). This is a pure function, so it is tested without a socket; Step 9 proves the socket separately.

```rust
    #[test]
    fn every_server_preview_outcome_maps_to_one_stage_state() {
        use app_core::admin::{CardPreviewResponse, CardPreviewState};

        let rendered = plugin_preview_frame(CardPreviewResponse {
            png_base64: Some("iVBORw0KGgo=".into()),
            state: CardPreviewState::Stale,
            message: None,
            refreshed_at_unix_ms: Some(1_787_000_000_000),
        });
        assert_eq!(
            rendered,
            PreviewFrame {
                png_base64: Some("iVBORw0KGgo=".into()),
                sample: false,
                state: None,
            }
        );

        // `sample` means "a real frame rendered, from an empty field set" -- it is
        // what puts the "No data yet" badge on a drawn image. A waiting plugin card
        // has no frame at all, so it is NOT sample: it prints the state sentence.
        let waiting = plugin_preview_frame(CardPreviewResponse {
            png_base64: None,
            state: CardPreviewState::Waiting,
            message: Some("Waiting for the first refresh".into()),
            refreshed_at_unix_ms: None,
        });
        assert_eq!(
            waiting,
            PreviewFrame {
                png_base64: None,
                sample: false,
                state: Some("Waiting for the first refresh".into()),
            }
        );

        let failed = plugin_preview_frame(CardPreviewResponse {
            png_base64: None,
            state: CardPreviewState::Error,
            message: Some("Plugin \"x\" is not loaded on the server".into()),
            refreshed_at_unix_ms: None,
        });
        assert_eq!(
            failed,
            PreviewFrame {
                png_base64: None,
                sample: false,
                state: Some("Plugin \"x\" is not loaded on the server".into()),
            }
        );

        // A server that says nothing still says something on the stage.
        let mute = plugin_preview_frame(CardPreviewResponse {
            png_base64: None,
            state: CardPreviewState::Error,
            message: None,
            refreshed_at_unix_ms: None,
        });
        assert!(matches!(mute.state, Some(message) if !message.is_empty()));
    }

    #[test]
    fn a_plugin_card_the_mac_cannot_render_says_where_it_renders() {
        assert_eq!(
            unrendered_plugin_frame(PLUGIN_RENDERS_ON_THE_SERVER),
            PreviewFrame {
                png_base64: None,
                sample: false,
                state: Some("Plugin cards render on the server".into()),
            }
        );
        assert_eq!(
            unrendered_plugin_frame(PLUGIN_PREVIEW_NEEDS_A_NEWER_SERVER).state,
            Some("Plugin previews need a newer server".into())
        );
    }
```
Run `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p deskmate-app --lib preview_outcome says_where_it_renders` — expect `error[E0425]: cannot find function plugin_preview_frame in this scope`.

- [ ] **Step 4: Make `PreviewFrame` nullable and add the mapping.** Replace the doc comment and struct at `commands.rs:133-141` with the block below, and add the two constants and two functions immediately after `sim_field` (commands.rs:830).

```rust
/// A rendered card preview. `png_base64` is `None` when there is nothing to draw
/// and `state` then names why, in the server's own words: the stage prints that
/// sentence rather than "Preview unavailable", which stays reserved for a real
/// transport failure. Built-in cards always set `png_base64` and never `state`.
///
/// `sample` is set when the card has never published data (the runtime holds no
/// `CardDataSnapshot` for it): the request still renders, with an empty field set,
/// so the image is the firmware's own unconfigured appearance for that template
/// rather than an invented placeholder. It therefore only ever accompanies a real
/// frame -- a card with no frame is not a sample of anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewFrame {
    pub png_base64: Option<String>,
    pub sample: bool,
    pub state: Option<String>,
}
```
```rust
/// Chosen from the tier the Mac already knows, before any socket is opened: in
/// local tier there is no server to render on and this app has no plugin host.
pub(crate) const PLUGIN_RENDERS_ON_THE_SERVER: &str = "Plugin cards render on the server";

/// Chosen only after a networked-tier request came back 404. Spec section 10 makes
/// the Mac's GETs additive, so a server built before the preview route answers 404
/// and this is the honest reading of it.
pub(crate) const PLUGIN_PREVIEW_NEEDS_A_NEWER_SERVER: &str =
    "Plugin previews need a newer server";

fn unrendered_plugin_frame(state: &str) -> PreviewFrame {
    PreviewFrame {
        png_base64: None,
        sample: false,
        state: Some(state.to_owned()),
    }
}

/// Section 4.2 promises `png_base64` exactly when the state is `fresh` or `stale`
/// and `message` exactly when it is `error` or `waiting`. This keys off the frame
/// rather than the word, so a server that breaks that invariant still produces a
/// stage that is either an image or a sentence, never a blank black rectangle.
fn plugin_preview_frame(response: app_core::admin::CardPreviewResponse) -> PreviewFrame {
    match response.png_base64 {
        Some(png_base64) => PreviewFrame {
            png_base64: Some(png_base64),
            sample: false,
            state: None,
        },
        None => PreviewFrame {
            png_base64: None,
            sample: false,
            state: Some(
                response
                    .message
                    .unwrap_or_else(|| "The server sent no preview for this card".to_owned()),
            ),
        },
    }
}
```
Then repair the one existing construction site, `render_card_preview`'s tail (commands.rs:785-788):
```rust
    Ok(PreviewFrame {
        png_base64: Some(BASE64_STANDARD.encode(png)),
        sample,
        state: None,
    })
```
Run `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p deskmate-app --lib preview` — green. Commit:
```sh
git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -am "feat: let a preview frame carry a state sentence instead of a PNG

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
```

- [ ] **Step 5: Write the failing server-card-state projection test.** The input is real `GET /v1/devices/{id}` JSON, pasted in the shape `server/src/admin.rs:439-462` serializes — including `device.observed_age_seconds`, which no Mac DTO declares, to pin that the Mac parses only what it needs. Add both tests after the pair from Step 3.

```rust
    #[test]
    fn server_card_state_covers_plugin_cards_only_and_reads_the_hero_field() {
        let body = serde_json::json!({
            "device_id": "desk-1",
            "connected": true,
            "last_seen_unix_ms": 1_787_000_000_000_u64,
            "config": { "origin": "current", "using_fallback": false, "fallback_reason": null },
            "snapshot": {
                "config": {
                    "cards": [
                        { "kind": "clock", "id": "clock", "title": "Desk",
                          "show_seconds": true, "template": { "kind": "digital-clock" },
                          "tap_action": { "kind": "none" },
                          "refresh": { "kind": "device-local" }, "alert": { "kind": "none" } },
                        { "kind": "plugin", "id": "air", "title": "", "plugin_id": "aqi",
                          "tap_action": { "kind": "none" },
                          "refresh": { "kind": "interval", "minutes": 15 },
                          "alert": { "kind": "none" } },
                        { "kind": "plugin", "id": "news", "title": "", "plugin_id": "agenda",
                          "tap_action": { "kind": "none" },
                          "refresh": { "kind": "interval", "minutes": 30 },
                          "alert": { "kind": "none" } }
                    ]
                },
                "device": { "observed_age_seconds": 4, "last_ota_error": null },
                "providers": [
                    { "widget_id": "air", "state": { "kind": "fresh" },
                      "last_success_unix_ms": 1_787_000_000_000_i64, "age_seconds": 30 },
                    { "widget_id": "clock", "state": { "kind": "idle" },
                      "last_success_unix_ms": null, "age_seconds": null }
                ],
                "card_data": [
                    { "card_id": "air", "fields": [
                        { "key": "title", "value": { "kind": "text", "value": "Air quality" } },
                        { "key": "hero", "value": { "kind": "text", "value": "42" } } ] },
                    { "card_id": "clock", "fields": [] }
                ],
                "card_errors": [
                    { "kind": "scene-refused", "card_id": "news",
                      "message": "no snapshot cached yet" },
                    { "kind": "data-refused", "card_id": "clock", "message": "ignored" }
                ]
            }
        });

        let states = project_server_card_state(&serde_json::from_value(body).unwrap());

        assert_eq!(
            states,
            vec![
                ServerCardState {
                    card_id: "air".into(),
                    provider: ProviderState::Fresh,
                    hero: Some("42".into()),
                    errors: Vec::new(),
                },
                // No provider entry yet is Idle, not an invented staleness.
                ServerCardState {
                    card_id: "news".into(),
                    provider: ProviderState::Idle,
                    hero: None,
                    errors: vec![CardError {
                        kind: CardErrorKind::SceneRefused,
                        card_id: "news".into(),
                        message: "no snapshot cached yet".into(),
                    }],
                },
            ]
        );
    }

    #[test]
    fn a_device_with_no_runtime_projects_no_plugin_state() {
        let body = serde_json::json!({
            "device_id": "desk-1", "connected": false, "last_seen_unix_ms": null,
            "config": { "origin": "defaults", "using_fallback": false, "fallback_reason": null },
            "snapshot": null
        });
        assert!(project_server_card_state(&serde_json::from_value(body).unwrap()).is_empty());
    }
```
Run `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p deskmate-app --lib server_card_state no_runtime_projects` — expect `error[E0422]: cannot find struct, variant or union type ServerCardState in this scope`.

- [ ] **Step 6: Implement `ServerCardState` and its projection.** Add to `commands.rs` immediately after the `PreviewFrame` struct (i.e. after the block added in Step 4 at line ~148).

```rust
/// The server's view of one plugin card, projected onto the Mac's snapshot so a
/// plugin tile carries the same value, freshness and error copy a built-in does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerCardState {
    pub card_id: String,
    pub provider: app_core::ProviderState,
    pub hero: Option<String>,
    pub errors: Vec<app_core::CardError>,
}

/// Deliberately partial. `AdminSnapshot` injects `last_ota_error` and
/// `observed_age_seconds` into `device` after serialization, so parsing the whole
/// `AppSnapshot` back would couple this app to a shape only the server writes.
/// These four collections are all a plugin tile needs.
#[derive(Deserialize)]
pub(crate) struct ServerDeviceStatus {
    snapshot: Option<ServerRuntimeSnapshot>,
}

#[derive(Deserialize)]
struct ServerRuntimeSnapshot {
    config: ServerSnapshotConfig,
    providers: Vec<app_core::ProviderSnapshot>,
    card_data: Vec<app_core::CardDataSnapshot>,
    card_errors: Vec<app_core::CardError>,
}

#[derive(Deserialize)]
struct ServerSnapshotConfig {
    cards: Vec<CardSettings>,
}

fn project_server_card_state(status: &ServerDeviceStatus) -> Vec<ServerCardState> {
    let Some(snapshot) = status.snapshot.as_ref() else {
        return Vec::new();
    };
    snapshot
        .config
        .cards
        .iter()
        .filter(|card| matches!(card, CardSettings::Plugin { .. }))
        .map(|card| {
            let card_id = card.id();
            ServerCardState {
                card_id: card_id.to_owned(),
                provider: snapshot
                    .providers
                    .iter()
                    .find(|provider| provider.widget_id == card_id)
                    .map_or(app_core::ProviderState::Idle, |provider| {
                        provider.state.clone()
                    }),
                // The summary the manifest declares arrives as `hero`; a non-text
                // value is not a headline, so it is not shown as one.
                hero: snapshot
                    .card_data
                    .iter()
                    .find(|data| data.card_id == card_id)
                    .and_then(|data| data.fields.iter().find(|field| field.key == "hero"))
                    .and_then(|field| match &field.value {
                        CardFieldValue::Text { value } => Some(value.clone()),
                        CardFieldValue::Integer { .. } | CardFieldValue::Boolean { .. } => None,
                    }),
                errors: snapshot
                    .card_errors
                    .iter()
                    .filter(|error| error.card_id == card_id)
                    .cloned()
                    .collect(),
            }
        })
        .collect()
}
```
Run `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p deskmate-app --lib server_card_state no_runtime_projects` — green. Commit:
```sh
git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -am "feat: project the server's plugin card state onto Mac DTOs

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
```

- [ ] **Step 7: Write the failing projection-overlay test in `lib.rs`.** It mirrors `a_local_save_clears_the_server_projection_and_unknown_tier_never_projects_it` (lib.rs:678) — the same tier discipline, on the other projection. Add both items at the end of `lib.rs`'s test module.

```rust
    /// One plugin card as a hostless Mac runtime reports it: an idle provider entry
    /// that will never refresh, an empty field set, and no error. The overlay has to
    /// replace all three, not append beside them.
    fn plugin_card_snapshot() -> AppSnapshot {
        use app_core::{
            CardDataSnapshot, DeviceCounters, DeviceSnapshot, ProviderSnapshot, RuntimeDiagnostics,
        };

        AppSnapshot {
            config: AppConfig::default(),
            runtime: RuntimeState::Running,
            device: DeviceSnapshot {
                connection: ConnectionState::Online,
                port_name: None,
                firmware_version: Some("2.0.0".into()),
                protocol_version: Some(1),
                max_protocol_version: Some(1),
                capabilities: Vec::new(),
                unknown_capability_bits: 0,
                uptime_ms: None,
                free_heap: None,
                rotation: None,
                tier: Some(DeviceTier::Networked),
                wifi_state: None,
                wifi_rssi: None,
                ip: None,
                last_network_error: None,
                ota_state: None,
                active_screen_id: None,
                counters: DeviceCounters::default(),
            },
            providers: vec![ProviderSnapshot {
                widget_id: "air".into(),
                state: ProviderState::Idle,
                last_success_unix_ms: None,
                age_seconds: None,
            }],
            pomodoros: Vec::new(),
            card_data: vec![CardDataSnapshot {
                card_id: "air".into(),
                fields: Vec::new(),
            }],
            card_errors: Vec::new(),
            persistence: PersistenceState::Clean,
            diagnostics: RuntimeDiagnostics::default(),
        }
    }

    #[test]
    fn server_card_state_overlays_only_in_networked_tier() {
        use app_core::{CardErrorKind, CardFieldValue};

        let projection = ServerStateProjection::default();
        projection
            .replace(vec![commands::ServerCardState {
                card_id: "air".into(),
                provider: ProviderState::Fresh,
                hero: Some("42".into()),
                errors: vec![app_core::CardError {
                    kind: CardErrorKind::SceneRefused,
                    card_id: "air".into(),
                    message: "no snapshot cached yet".into(),
                }],
            }])
            .unwrap();

        // Cable-out (tier unknown) must not paint server state onto a display this
        // Mac may own locally.
        let mut cabled_out = plugin_card_snapshot();
        projection.project(None, &mut cabled_out);
        assert_eq!(cabled_out.providers.len(), 1);
        assert_eq!(cabled_out.providers[0].state, ProviderState::Idle);
        assert!(cabled_out.card_data[0].fields.is_empty());
        assert!(cabled_out.card_errors.is_empty());

        let mut app = plugin_card_snapshot();
        projection.project(Some(DeviceTier::Networked), &mut app);
        assert_eq!(app.providers.len(), 1, "the stale entry was appended to");
        assert_eq!(app.providers[0].widget_id, "air");
        assert_eq!(app.providers[0].state, ProviderState::Fresh);
        assert_eq!(app.card_data.len(), 1);
        assert_eq!(app.card_data[0].fields.len(), 1);
        assert_eq!(app.card_data[0].fields[0].key, "hero");
        assert_eq!(
            app.card_data[0].fields[0].value,
            CardFieldValue::Text {
                value: "42".into()
            }
        );
        assert_eq!(app.card_errors.len(), 1);
        assert_eq!(app.card_errors[0].card_id, "air");
    }
```
Run `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p deskmate-app --lib overlays_only_in_networked` — expect `error[E0433]: failed to resolve: use of undeclared type ServerStateProjection`.

- [ ] **Step 8: Implement `ServerStateProjection` and wire it in.** Add the type after `NetworkedConfigProjection`'s `impl` block (lib.rs:130), then make the four wiring edits below.

```rust
/// The last successful `get_server_card_state` poll. In networked tier the server
/// owns every plugin card's data, so its answer replaces whatever the local
/// hostless runtime holds for those ids -- and only those ids. A failed poll keeps
/// the previous projection rather than blanking a tile that was correct a moment
/// ago; the frontend surfaces the failure as one notice.
///
/// The provider entry is rebuilt with no timestamps because `ServerCardState`
/// deliberately carries none: a tile reads the state word, and a fetch time
/// measured on the server is not a fact about this Mac's clock.
#[derive(Default)]
struct ServerStateProjection(Mutex<Vec<commands::ServerCardState>>);

impl ServerStateProjection {
    fn replace(&self, states: Vec<commands::ServerCardState>) -> Result<(), commands::IpcError> {
        *self.0.lock().map_err(|_| commands::IpcError::Internal {
            message: "server plugin state is unavailable".into(),
        })? = states;
        Ok(())
    }

    fn project(&self, tier: Option<DeviceTier>, app: &mut AppSnapshot) {
        if !matches!(tier, Some(DeviceTier::Networked)) {
            return;
        }
        let Ok(states) = self.0.lock() else {
            return;
        };
        for state in states.iter() {
            let card_id = state.card_id.as_str();
            app.providers
                .retain(|provider| provider.widget_id != card_id);
            app.providers.push(app_core::ProviderSnapshot {
                widget_id: state.card_id.clone(),
                state: state.provider.clone(),
                last_success_unix_ms: None,
                age_seconds: None,
            });
            app.card_data.retain(|data| data.card_id != card_id);
            if let Some(hero) = state.hero.as_ref() {
                app.card_data.push(app_core::CardDataSnapshot {
                    card_id: state.card_id.clone(),
                    fields: vec![app_core::CardField {
                        key: "hero".into(),
                        value: app_core::CardFieldValue::Text {
                            value: hero.clone(),
                        },
                    }],
                });
            }
            app.card_errors.retain(|error| error.card_id != card_id);
            app.card_errors.extend(state.errors.iter().cloned());
        }
    }
}
```
(a) `DesktopSnapshotProjector` (lib.rs:132-137) gains a field:
```rust
struct DesktopSnapshotProjector {
    network_store: Arc<NetworkSettingsStore>,
    networked_config: Arc<NetworkedConfigProjection>,
    server_card_state: Arc<ServerStateProjection>,
    last_known_tier: Mutex<Option<DeviceTier>>,
    has_saved_config: Arc<AtomicBool>,
}
```
(b) `project_snapshot` (lib.rs:140-150) applies it after the config projection:
```rust
    fn project_snapshot(&self, mut app: AppSnapshot) -> DesktopSnapshot {
        if let Some(tier) = app.device.tier {
            self.remember_device_tier(tier);
        }
        self.networked_config
            .project(app.device.tier, &mut app.config);
        self.server_card_state.project(app.device.tier, &mut app);
        DesktopSnapshot {
            app,
            has_saved_config: self.has_saved_config.load(Ordering::Acquire),
        }
    }
```
(c) `DesktopState` (lib.rs:178-192) gains the same handle, after `networked_config`:
```rust
    networked_config: Arc<NetworkedConfigProjection>,
    server_card_state: Arc<ServerStateProjection>,
```
(d) `setup_app` (lib.rs:432-456) constructs it once and shares it:
```rust
    let networked_config = Arc::new(NetworkedConfigProjection::default());
    let server_card_state = Arc::new(ServerStateProjection::default());
    let has_saved_config = Arc::new(AtomicBool::new(has_saved_config));
    let snapshot_projector = DesktopSnapshotProjector {
        network_store: Arc::clone(&network_store),
        networked_config: Arc::clone(&networked_config),
        server_card_state: Arc::clone(&server_card_state),
        last_known_tier: Mutex::new(last_known_tier),
        has_saved_config: Arc::clone(&has_saved_config),
    };

    app.manage(DesktopState {
        runtime: Arc::clone(&runtime),
        store,
        network_store,
        networked_config,
        server_card_state,
        snapshot_projector,
        server_client: server_http_agent(),
        has_saved_config,
        tray,
        snapshot_worker: Mutex::new(None),
        mutation_lock: Arc::new(Mutex::new(())),
        quitting: AtomicBool::new(false),
        // One dedicated thread owns the process-wide `Simulator` for the app's
        // lifetime (see `preview` module docs); nothing else may construct one.
        preview: preview::spawn(),
    });
```
(e) the existing literal in `projecting_an_observed_tier_persists_it_for_the_next_cable_out_snapshot` (lib.rs:719-724) gains the field:
```rust
        let projector = DesktopSnapshotProjector {
            network_store: Arc::clone(&network_store),
            networked_config: Arc::new(NetworkedConfigProjection::default()),
            server_card_state: Arc::new(ServerStateProjection::default()),
            last_known_tier: Mutex::new(Some(DeviceTier::Local)),
            has_saved_config: Arc::new(AtomicBool::new(false)),
        };
```
Run `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p deskmate-app --lib` — green. `rustc` reports one `field \`server_card_state\` is never read` warning on `DesktopState` until Step 10 adds its only reader; that is expected and `-D warnings` is not run until Step 13. Commit:
```sh
git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -am "feat: overlay server plugin state onto the desktop snapshot

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
```

- [ ] **Step 9: Write the failing loopback tests for the three commands' blocking halves.** These exercise the whole path a command takes minus Tauri's `State`: real sockets, real network store, real bearer header, real URL. Add after Step 5's pair in `commands.rs`'s test module.

```rust
    /// Builds a `ServerQueryContext` pointed at a loopback listener with a stored
    /// admin token, in the tier the caller names.
    fn server_query_fixture(
        label: &str,
        tier: app_core::DeviceTier,
    ) -> (ServerQueryContext, std::net::TcpListener, std::path::PathBuf) {
        let directory = scratch_directory(label);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let network_store = Arc::new(NetworkSettingsStore::new(
            directory.join("network-settings.json"),
        ));
        network_store
            .save(NetworkSettingsUpdate::new(
                format!("http://{address}"),
                "desk-1",
                Some(tier),
                Some("admin-secret".into()),
            ))
            .unwrap();
        let context = ServerQueryContext {
            agent: crate::server_http_agent(),
            network_store,
        };
        (context, listener, directory)
    }

    fn answer_once(
        listener: std::net::TcpListener,
        status: &'static str,
        body: &'static str,
    ) -> std::thread::JoinHandle<String> {
        use std::io::Write as _;

        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = String::from_utf8_lossy(&read_http_request(&mut stream)).to_string();
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            request
        })
    }

    #[test]
    fn the_catalog_command_reads_the_shared_admin_dto() {
        let (context, listener, directory) =
            server_query_fixture("catalog", app_core::DeviceTier::Networked);
        let server = answer_once(
            listener,
            "200 OK",
            r#"{"plugins":[{"id":"aqi","name":"aqi","version":"1.0.0","node_count":4,
                "assets":[{"file":"icons.ttf","kind":"icon-font","byte_length":12,"digest":"ab"}],
                "display_name":"Air quality","description":"EPA index for a location",
                "manifest_version":2,"template":"display-list","refresh_minutes":15}],
              "load_failures":[{"id":"broken","error":"unknown key"}]}"#,
        );

        let catalog = fetch_server_plugins(&context).unwrap();

        let request = server.join().unwrap();
        assert!(request.starts_with("GET /v1/plugins "));
        assert_eq!(catalog.plugins.len(), 1);
        assert_eq!(
            catalog.plugins[0].display_name.as_deref(),
            Some("Air quality")
        );
        assert_eq!(catalog.plugins[0].refresh_minutes, 15);
        assert_eq!(
            catalog.plugins[0].template,
            app_core::admin::PluginTemplateKind::DisplayList
        );
        assert_eq!(catalog.load_failures[0].id, "broken");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn the_card_state_command_asks_for_this_device_and_keeps_only_plugin_cards() {
        let (context, listener, directory) =
            server_query_fixture("card-state", app_core::DeviceTier::Networked);
        let server = answer_once(
            listener,
            "200 OK",
            r#"{"device_id":"desk-1","connected":true,"last_seen_unix_ms":1,
                "config":{"origin":"current","using_fallback":false,"fallback_reason":null},
                "snapshot":{"config":{"cards":[
                    {"kind":"plugin","id":"air","title":"","plugin_id":"aqi",
                     "tap_action":{"kind":"none"},"refresh":{"kind":"interval","minutes":15},
                     "alert":{"kind":"none"}}]},
                  "providers":[{"widget_id":"air","state":{"kind":"fresh"},
                    "last_success_unix_ms":null,"age_seconds":null}],
                  "card_data":[{"card_id":"air","fields":[
                    {"key":"hero","value":{"kind":"text","value":"42"}}]}],
                  "card_errors":[]}}"#,
        );

        let states = fetch_server_card_state(&context).unwrap();

        let request = server.join().unwrap();
        assert!(request.starts_with("GET /v1/devices/desk-1 "));
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].hero.as_deref(), Some("42"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_plugin_preview_asks_the_card_route_and_returns_its_frame() {
        let (context, listener, directory) =
            server_query_fixture("preview-ok", app_core::DeviceTier::Networked);
        let server = answer_once(
            listener,
            "200 OK",
            r#"{"png_base64":"iVBORw0KGgo=","state":"fresh","message":null,
                "refreshed_at_unix_ms":1787000000000}"#,
        );

        let frame = plugin_card_preview(&context, Some(app_core::DeviceTier::Networked), "air")
            .unwrap();

        let request = server.join().unwrap();
        assert!(request.starts_with("GET /v1/devices/desk-1/cards/air/preview "));
        assert_eq!(frame.png_base64.as_deref(), Some("iVBORw0KGgo="));
        assert_eq!(frame.state, None);
        assert!(!frame.sample);
        fs::remove_dir_all(directory).unwrap();
    }

    /// Spec section 10: the Mac's GETs are additive, so a server that has never
    /// heard of this route 404s. That reads as "needs a newer server", not as a
    /// fault, and specifically not as the local-tier sentence, which is chosen
    /// before any request is made.
    ///
    /// An unknown device, an unknown card and a non-plugin card also 404, and this
    /// call path cannot reach any of them: the Mac only asks for a card that is in
    /// its own config, for the device id it is configured with, and only when that
    /// card is `CardSettings::Plugin`. They are not distinguished, and pretending to
    /// would mean inventing a difference the status code does not carry.
    #[test]
    fn a_preview_route_an_old_server_lacks_degrades_instead_of_erroring() {
        let (context, listener, directory) =
            server_query_fixture("preview-404", app_core::DeviceTier::Networked);
        let server = answer_once(listener, "404 Not Found", "");

        let frame = plugin_card_preview(&context, Some(app_core::DeviceTier::Networked), "air")
            .unwrap();

        server.join().unwrap();
        assert_eq!(
            frame.state.as_deref(),
            Some(PLUGIN_PREVIEW_NEEDS_A_NEWER_SERVER)
        );
        assert_eq!(frame.png_base64, None);
        fs::remove_dir_all(directory).unwrap();
    }

    /// Local tier has no server to render on and the Mac has no plugin host, so it
    /// says so from the tier alone rather than opening a socket at all.
    #[test]
    fn a_local_tier_plugin_preview_never_reaches_the_network() {
        let (context, listener, directory) =
            server_query_fixture("preview-local", app_core::DeviceTier::Local);

        let frame =
            plugin_card_preview(&context, Some(app_core::DeviceTier::Local), "air").unwrap();

        assert_eq!(frame.state.as_deref(), Some(PLUGIN_RENDERS_ON_THE_SERVER));
        assert_eq!(frame.png_base64, None);
        drop(listener);
        fs::remove_dir_all(directory).unwrap();
    }
```
Run `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p deskmate-app --lib catalog_command card_state_command plugin_preview local_tier_plugin` — expect `error[E0422]: cannot find struct, variant or union type ServerQueryContext in this scope`.

- [ ] **Step 10: Implement the query context, the three blocking halves and the three commands.** Add the block below to `commands.rs` after `ProvisionContext`'s `impl` (line 90), and replace `render_card_preview` (commands.rs:727-789) entirely with the version that follows.

```rust
/// Everything a read-only server call needs, cloned out of `DesktopState` so the
/// blocking half can move onto a worker thread. No config, no runtime, no locks:
/// these calls never mutate anything the desktop owns.
#[derive(Clone)]
struct ServerQueryContext {
    agent: ureq::Agent,
    network_store: Arc<NetworkSettingsStore>,
}

impl ServerQueryContext {
    fn from_desktop(state: &DesktopState) -> Self {
        Self {
            agent: state.server_client.clone(),
            network_store: Arc::clone(&state.network_store),
        }
    }

    /// The store lends the token for one call and never returns it; a missing token
    /// is a typed instruction, not a transport failure.
    fn with_admin_token<T>(
        &self,
        operation: impl FnOnce(&str) -> Result<T, IpcError>,
    ) -> Result<T, IpcError> {
        self.network_store
            .with_admin_token(operation)
            .map_err(IpcError::from)?
            .ok_or_else(|| IpcError::InvalidPayload {
                message: "Enter the admin token in Network setup before reading server state."
                    .into(),
            })?
    }
}

fn fetch_server_plugins(
    context: &ServerQueryContext,
) -> Result<app_core::admin::PluginCatalog, IpcError> {
    let settings = context.network_store.load().settings().clone();
    let url =
        crate::server_client::server_url(&settings.server_url, &["v1", "plugins"])?.to_string();
    context.with_admin_token(|token| {
        crate::server_client::get_server_json(&context.agent, &url, token, MAX_SERVER_ERROR_BYTES)
    })
}

fn fetch_server_card_state(context: &ServerQueryContext) -> Result<Vec<ServerCardState>, IpcError> {
    let settings = context.network_store.load().settings().clone();
    validate_target(&settings.device_id, MAX_DEVICE_ID_LEN, "device ID")?;
    let url = crate::server_client::server_url(
        &settings.server_url,
        &["v1", "devices", &settings.device_id],
    )?
    .to_string();
    let status: ServerDeviceStatus = context.with_admin_token(|token| {
        crate::server_client::get_server_json(&context.agent, &url, token, MAX_SERVER_ERROR_BYTES)
    })?;
    Ok(project_server_card_state(&status))
}

/// The tier decides where a plugin card's face comes from, and it decides first:
/// in local tier there is no server and no plugin host, so this returns the
/// sentence without opening a socket. Only in networked tier is a request made,
/// and only then can a 404 mean what `PLUGIN_PREVIEW_NEEDS_A_NEWER_SERVER` says.
fn plugin_card_preview(
    context: &ServerQueryContext,
    tier: Option<app_core::DeviceTier>,
    card_id: &str,
) -> Result<PreviewFrame, IpcError> {
    validate_target(card_id, MAX_WIDGET_ID_LEN, "card ID")?;
    let settings = context.network_store.load().settings().clone();
    if save_destination(tier, &settings) != SaveDestination::Server {
        return Ok(unrendered_plugin_frame(PLUGIN_RENDERS_ON_THE_SERVER));
    }
    validate_target(&settings.device_id, MAX_DEVICE_ID_LEN, "device ID")?;
    let url = crate::server_client::server_url(
        &settings.server_url,
        &[
            "v1",
            "devices",
            &settings.device_id,
            "cards",
            card_id,
            "preview",
        ],
    )?
    .to_string();
    let response = context.with_admin_token(|token| {
        crate::server_client::get_server_json::<app_core::admin::CardPreviewResponse>(
            &context.agent,
            &url,
            token,
            crate::server_client::MAX_SERVER_PREVIEW_BYTES,
        )
    });
    match response {
        Ok(response) => Ok(plugin_preview_frame(response)),
        // Spec section 10: additive routes fail closed against an older server.
        Err(IpcError::NotFound { .. }) => Ok(unrendered_plugin_frame(
            PLUGIN_PREVIEW_NEEDS_A_NEWER_SERVER,
        )),
        Err(error) => Err(error),
    }
}

async fn on_server_worker<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, IpcError> + Send + 'static,
) -> Result<T, IpcError> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|_| IpcError::RuntimeUnavailable {
            message: "the server request worker stopped unexpectedly".into(),
        })?
}

#[tauri::command]
pub async fn get_server_plugins(
    state: State<'_, DesktopState>,
) -> Result<app_core::admin::PluginCatalog, IpcError> {
    let context = ServerQueryContext::from_desktop(&state);
    on_server_worker(move || fetch_server_plugins(&context)).await
}

#[tauri::command]
pub async fn get_server_card_state(
    state: State<'_, DesktopState>,
) -> Result<Vec<ServerCardState>, IpcError> {
    let context = ServerQueryContext::from_desktop(&state);
    let states = on_server_worker(move || fetch_server_card_state(&context)).await?;
    // The snapshot stream is what a tile actually reads, so a successful poll
    // updates the projection before it answers the caller.
    state.server_card_state.replace(states.clone())?;
    Ok(states)
}
```
and the complete replacement for `render_card_preview`:
```rust
/// Renders one card exactly as the firmware's own template would: the same
/// `SimTemplate` `wire_config` would compile it to, the same last-good field values
/// the device holds (`AppSnapshot.card_data`), the same timezone offset the device's
/// clock is synced to, and the currently configured mounting orientation.
///
/// A card with no published data yet (no `CardDataSnapshot` entry, or one with an
/// empty field set) renders with an empty field vector instead of inventing sample
/// text: the firmware's own per-template defaults show through, which is the
/// device's actual unconfigured appearance. `sample` tells the caller this happened
/// so the settings UI can badge it, without the renderer itself lying about what it
/// drew.
///
/// A plugin card is the one kind this simulator cannot draw: it has no
/// `DisplayTemplate`, and its face was compiled from a manifest and rasterized by
/// the server. That card's preview is fetched rather than rendered, which is why
/// this command is `async` -- the fetch runs on a blocking worker, never on the
/// thread that would otherwise stall the settings window for the agent's timeout.
#[tauri::command]
pub async fn render_card_preview(
    state: State<'_, DesktopState>,
    card_id: String,
) -> Result<PreviewFrame, IpcError> {
    validate_target(&card_id, MAX_WIDGET_ID_LEN, "card ID")?;
    let snapshot = state.runtime.snapshot().map_err(IpcError::from)?;
    let card = snapshot
        .config
        .cards
        .iter()
        .find(|card| card.id() == card_id)
        .ok_or_else(|| IpcError::NotFound {
            message: format!("no card with id {card_id:?}"),
        })?;

    if matches!(card, CardSettings::Plugin { .. }) {
        let context = ServerQueryContext::from_desktop(&state);
        let tier = snapshot.device.tier;
        let requested = card_id.clone();
        return on_server_worker(move || plugin_card_preview(&context, tier, &requested)).await;
    }

    let template = preview_template_for(card, &card_id)?;

    let data = snapshot
        .card_data
        .iter()
        .find(|data| data.card_id == card_id);
    let (fields, sample) = match data {
        Some(data) if !data.fields.is_empty() => {
            (data.fields.iter().map(sim_field).collect(), false)
        }
        _ => (Vec::new(), true),
    };

    let now = chrono::Utc::now();
    let utc_offset_minutes = utc_offset_minutes(&snapshot.config.preferences.timezone, now)
        .map_err(|message| IpcError::Internal { message })?;
    let orientation = match snapshot.config.preferences.orientation {
        DisplayOrientation::Landscape => lvgl_sim::SimOrientation::Landscape,
        DisplayOrientation::LandscapeFlipped => lvgl_sim::SimOrientation::LandscapeFlipped,
    };

    let request = lvgl_sim::RenderRequest {
        template,
        fields,
        utc_offset_minutes,
        now_unix_seconds: now.timestamp(),
        orientation,
    };
    let png = state
        .preview
        .render(request)
        .map_err(|message| IpcError::Internal { message })?;
    Ok(PreviewFrame {
        png_base64: Some(BASE64_STANDARD.encode(png)),
        sample,
        state: None,
    })
}
```
`preview_template_for` is now unreachable from this command but stays as the typed refusal for any future caller; keep it and its test (commands.rs:1432). Register the two new commands in `lib.rs`'s `generate_handler!` (line 549-565) after `commands::render_card_preview`:
```rust
            commands::render_card_preview,
            commands::get_server_plugins,
            commands::get_server_card_state,
        ])
```
Run `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p deskmate-app --lib` — green, and the `field is never read` warning from Step 8 is gone. Commit:
```sh
git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -am "feat: read the plugin catalog, card state and preview from the server

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
```

- [ ] **Step 11: Extend the contract fixture and watch the sync test fail.** Add two fields to `ContractFixtures` (commands.rs:2253-2286), immediately after `autostart_status: AutostartStatus,` and before `preview_frame: PreviewFrame,` so the generated JSON key order matches:

```rust
        autostart_status: AutostartStatus,
        plugin_catalog: app_core::admin::PluginCatalog,
        server_card_state: Vec<ServerCardState>,
        preview_frame: PreviewFrame,
    }
```
and their values in `contract_fixtures()`, replacing the tail at commands.rs:2739-2751:
```rust
            autostart_status: AutostartStatus {
                enabled: true,
                preference_enabled: false,
            },
            plugin_catalog: app_core::admin::PluginCatalog {
                plugins: vec![app_core::admin::PluginCatalogEntry {
                    id: "aqi".into(),
                    name: "aqi".into(),
                    version: "1.0.0".into(),
                    node_count: 4,
                    assets: vec![app_core::admin::PluginCatalogAsset {
                        file: "icons.ttf".into(),
                        kind: "icon-font".into(),
                        byte_length: 40_960,
                        digest: "0f1e2d3c".into(),
                    }],
                    display_name: Some("Air quality".into()),
                    description: Some("EPA index for a location".into()),
                    manifest_version: 2,
                    template: app_core::admin::PluginTemplateKind::DisplayList,
                    refresh_minutes: 15,
                }],
                load_failures: vec![app_core::admin::PluginLoadFailure {
                    id: "broken".into(),
                    error: "unknown key \"summry\"".into(),
                }],
            },
            server_card_state: vec![ServerCardState {
                card_id: "air-quality".into(),
                provider: ProviderState::Fresh,
                hero: Some("42".into()),
                errors: vec![CardError {
                    kind: CardErrorKind::SceneRefused,
                    card_id: "air-quality".into(),
                    message: "no snapshot cached yet".into(),
                }],
            }],
            preview_frame: PreviewFrame {
                png_base64: Some("iVBORw0KGgo=".into()),
                sample: true,
                state: None,
            },
        }
    }
```
Run `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p deskmate-app --lib typescript_contract_fixture_stays_in_sync` — expect `assertion \`left == right\` failed`, with the checked file carrying `"png_base64": "iVBORw0KGgo="` and no `plugin_catalog`.

- [ ] **Step 12: Regenerate the contract fixture.** It is generated, never hand-written; the ignored printer test is the source. The extraction is done in Python rather than `sed` because `--nocapture` can glue libtest's `test <name> ... ` prefix onto the first printed line, which defeats a `^`-anchored match.

```sh
cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion
/Users/rodion/.cargo/bin/cargo test -p deskmate-app --lib \
  commands::tests::print_typescript_contract_fixture -- --ignored --exact --nocapture \
  > apps/deskmate/src/lib/types.contract.ts.raw
python3 -c 'import pathlib
raw = pathlib.Path("apps/deskmate/src/lib/types.contract.ts.raw").read_text()
head = "// Generated by the Rust IPC contract test"
tail = "} as const satisfies IpcContractFixtures;"
start = raw.index(head)
end = raw.index(tail) + len(tail)
pathlib.Path("apps/deskmate/src/lib/types.contract.ts").write_text(raw[start:end] + "\n")'
rm apps/deskmate/src/lib/types.contract.ts.raw
```
Run `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p deskmate-app --lib typescript_contract_fixture_stays_in_sync` — green (the test compares byte-for-byte, so a green run proves the extraction). Confirm the raw file is gone with `git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity status --short apps/deskmate/src/lib`, which must show only `M companion/apps/deskmate/src/lib/types.contract.ts`. Commit:
```sh
git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -am "feat: carry the plugin catalog and server card state across the IPC contract

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
```

- [ ] **Step 13: Mutate the three guards a green suite would not catch, then run the full Rust gates.** Each mutation is one edit, reverted before the next.

  (a) Delete the `.limit(max_bytes.saturating_add(1) as u64)` line in `get_server_json` (`with_config()` defaults to `u64::MAX`, i.e. unbounded). Run `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p deskmate-app --lib an_oversized_body` — it must fail on `.unwrap_err()` because the oversized body is read and parsed successfully. Restore the line.

  (b) In `plugin_preview_frame`'s `None` arm change `sample: false` to `sample: true`. Run `... --lib every_server_preview_outcome` — it must fail on the `waiting` row. This is the guard that keeps the "No data yet" badge attached to a real frame instead of to an empty stage. Restore `false`.

  (c) Delete `if !matches!(tier, Some(DeviceTier::Networked)) { return; }` from `ServerStateProjection::project`. Run `... --lib overlays_only_in_networked` — it must fail on the cable-out half. Restore it.

  Then, from `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion`:
```sh
cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion \
  && /Users/rodion/.cargo/bin/cargo fmt --all --check \
  && /Users/rodion/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings \
  && /Users/rodion/.cargo/bin/cargo test --workspace --all-targets \
  && /Users/rodion/.cargo/bin/cargo test --workspace --doc
```
Commit formatting fallout only (if `cargo fmt --all --check` failed, run `cargo fmt --all` first and re-run the chain):
```sh
git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -am "test: pin the server GET bound, the waiting stage state and the networked-only overlay

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
```

**Task exit:** from `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion` — `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --all-targets` and `cargo test --workspace --doc` all green with the absolute `/Users/rodion/.cargo/bin/cargo`; `types.contract.ts` regenerated by Step 12's command and never hand-edited; all three mutation probes in Step 13 observed failing before restoration. The frontend gates (`bun run check`, `bun run lint`, `bun run format:check`, `bun test` from `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate`) are **expected to fail** on `types.contract.ts` at this point, because `IpcContractFixtures` in the hand-written `src/lib/types.ts` does not yet declare `plugin_catalog` or `server_card_state` and `PreviewFrame` is not yet nullable there — Task 5 owns every hand-written TypeScript file and closes that gap.

---

### Task 5: Every surface treats a plugin card like a card

**Files:**

- Create `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate/src/dev/pluginFixture.ts`
- Modify `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate/src/lib/types.ts` (440 lines; `CardDataSnapshot` at 332-335, `PreviewFrame` at 380-383, `IpcContractFixtures` at 407-440)
- Modify `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate/src/lib/tauri.ts` (174 lines; `renderCardPreview` at 168-170)
- Modify `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate/src/lib/configDraft.ts` (748 lines; `cardLabel` 76-78, `cardKindName` 89-106, `addCard` 123-233, `filmstripSegments` 451-481)
- Modify `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate/src/lib/useAppState.ts` (348 lines; `ownershipTier` at 326-330, return at 332-346)
- Modify `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate/src/components/CardList.tsx` (592 lines; `PluginKindOption` 42-47, `CardListProps` 49-61, `tileValue` 81-119, `controlLabel` 121-125, `renderCardTile` 328-434, plugin menu row 576-579)
- Modify `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate/src/components/CardEditor.tsx` (688 lines; props 29-55, heading 154-163, trouble note 181-194, refresh block 500-528)
- Modify `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate/src/components/LoopRing.tsx` (370 lines; props 16-22, `segments` memo at 73)
- Modify `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate/src/components/DevicePreview.tsx` (89 lines)
- Modify `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate/src/App.tsx` (708 lines; `handleAdd` 247-254, notices 461-483, `<CardList>` 535-547, `<CardEditor>` 549-566, `<LoopRing>` 383-389)
- Modify `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate/src/dev/mockBackend.ts` (390 lines; `SCENARIOS` 24-34, `applyScenario` 50-129, `render_card_preview` 311-331)
- Modify `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate/src/dev/mockPreview.ts` (209 lines; `renderMockFrame` 148-208)
- Modify `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate/tests/components.test.tsx` (2594 lines)
- Modify `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate/tests/configDraft.test.ts` (687 lines)
- Modify `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate/tests/useAppState.test.ts` (131 lines)

**No new test file is created, and that is deliberate.** `bun test` runs every file in one process and `mock.module` is process-global with no guaranteed file order — verified here: a second file importing `../src/lib/tauri` receives `components.test.tsx`'s module mock. So the IPC-wrapper test lives in `components.test.tsx`, which captures the real wrappers before its own `mock.module` line and mocks `@tauri-apps/api/core` underneath them (also verified against this repo).

**No CSS file is touched.** Every element this task adds reuses an existing token-based class (`.field`, `.field small`, `.flag`, `.notice--warn`, `.stage__unavailable`, `.menu__item`, `.data-note`). One consequence is recorded rather than fixed: `src/styles.css:1250` scopes the invalid-control border to `.field input[aria-invalid="true"]`, so the editor's new `<select>` reports invalidity through `aria-invalid` and the visible `FieldIssues` message, not a red border. Widening that selector is a visual decision this task does not own.

**Interfaces:**

*Consumes* (delivered by Task 4; do not redefine them here):

```rust
#[tauri::command] async fn get_server_plugins(..) -> Result<app_core::admin::PluginCatalog, IpcError>
#[tauri::command] async fn get_server_card_state(..) -> Result<Vec<ServerCardState>, IpcError>
#[tauri::command] async fn render_card_preview(.., card_id: String) -> Result<PreviewFrame, IpcError>
pub struct ServerCardState { pub card_id: String, pub provider: app_core::ProviderState,
                             pub hero: Option<String>, pub errors: Vec<app_core::CardError> }
pub struct PreviewFrame { pub png_base64: Option<String>, pub sample: bool, pub state: Option<String> }
```

`PluginCatalog` serializes with plain snake_case field names (`load_failures`, `node_count`, `byte_length`, `display_name`, `manifest_version`, `refresh_minutes`); `PluginTemplateKind` is `"display-list" | "svg"`.

*Produces*:

```ts
// src/lib/types.ts
export interface PluginCatalog { plugins: PluginCatalogEntry[]; load_failures: PluginLoadFailure[] }
export interface PluginCatalogEntry { id: string; name: string; version: string; node_count: number;
  assets: PluginCatalogAsset[]; display_name: string | null; description: string | null;
  manifest_version: number; template: PluginTemplateKind; refresh_minutes: number }
export interface PluginCatalogAsset { file: string; kind: string; byte_length: number; digest: string }
export interface PluginLoadFailure { id: string; error: string }
export type PluginTemplateKind = "display-list" | "svg";
export interface ServerCardState { card_id: string; provider: ProviderState; hero: string | null;
  errors: CardError[] }
export interface PreviewFrame { png_base64: string | null; sample: boolean; state: string | null }

// src/lib/tauri.ts
export function getServerPlugins(): Promise<PluginCatalog>;
export function getServerCardState(): Promise<ServerCardState[]>;

// src/lib/configDraft.ts
export function cardLabel(card: CardSettings, catalog?: PluginCatalog | null): string;
export type PluginCardFlag = "not on the server" | "needs the server";
export function pluginCardFlag(card: CardSettings, catalog: PluginCatalog | null,
  tier: DeviceTier | null): PluginCardFlag | null;
export type AddCardRequest = AddableCardKind | { kind: "plugin"; pluginId: string; refreshMinutes: number };
export function addCard(config: AppConfig, request: AddCardRequest): { config: AppConfig; cardId: string | null };
export function filmstripSegments(config: AppConfig, catalog?: PluginCatalog | null): FilmstripSegment[];
export function cardKindName(kind: AddableCardKind): string;   // the "plugin" arm is deleted

// src/lib/useAppState.ts
export const SERVER_CARD_STATE_POLL_MS = 30_000;
export const SERVER_PLUGIN_NOTICE = "Couldn't reach the server for plugin data.";
export interface IntervalScheduler { set: (handler: () => void, ms: number) => number; clear: (handle: number) => void }
export interface ServerCardStatePollOptions { fetchCardState: () => Promise<ServerCardState[]>;
  onCardState: (state: ServerCardState[]) => void; onError: (error: IpcError) => void;
  focusTarget?: EventTargetLike; visibilityTarget?: VisibilityTargetLike;
  intervalMs?: number; scheduler?: IntervalScheduler }
export function startServerCardStatePoll(options: ServerCardStatePollOptions): () => void;
// AppStateValue gains: pluginCatalog, catalogError, refreshCatalog, serverCardState

// components
interface CardListProps  { ...; pluginKinds: PluginKindOption[]; catalog: PluginCatalog | null;
                           serverCardState: ServerCardState[]; ownershipTier: DeviceTier | null }
export interface PluginKindOption { id: string; version: string; displayName: string | null;
                                    description: string | null; onAdd: () => void }
interface CardEditorProps { ...; catalog: PluginCatalog | null; ownershipTier: DeviceTier | null }
interface LoopRingProps   { ...; catalog: PluginCatalog | null }

// src/dev/pluginFixture.ts
export const MOCK_PLUGIN_CATALOG: PluginCatalog;
export function mockPluginConfig(): AppConfig;
export function mockPluginCardState(): ServerCardState[];
export function mockPluginProviders(): ProviderSnapshot[];
export function mockPluginCardData(): CardDataSnapshot[];
```

---

- [ ] **Step 1: Type the catalog, the server card state and the widened preview frame, and give each a wrapper.**

  Write the failing test in `tests/components.test.tsx`. Three edits, all above the first `describe`. First, capture the real wrappers and mock the IPC bridge underneath them — insert immediately after the `import * as tauriModule from "../src/lib/tauri";` line (line 20):

  ```tsx
  // The two server reads are the only wrappers this suite proves end to end, so it
  // keeps a reference to the REAL ones before `mock.module` below replaces the
  // module's namespace in place, and mocks the Tauri bridge underneath them instead.
  // A second test file cannot do this: `mock.module` is process-global with no
  // guaranteed file order, so whichever file ran first would win.
  const coreInvocations: { command: string; args?: Record<string, unknown> }[] = [];
  mock.module("@tauri-apps/api/core", () => ({
    invoke: async (command: string, args?: Record<string, unknown>) => {
      coreInvocations.push({ command, args });
      return command === "get_server_plugins" ? { plugins: [], load_failures: [] } : [];
    },
  }));
  const realGetServerPlugins = tauriModule.getServerPlugins;
  const realGetServerCardState = tauriModule.getServerCardState;
  ```

  Second, add the two mutable impls beside `previewImpl` (after line 42's declaration):

  ```tsx
  let serverPluginsImpl: () => Promise<PluginCatalog> = async () => ({
    plugins: [],
    load_failures: [],
  });
  let serverCardStateImpl: () => Promise<ServerCardState[]> = async () => [];
  ```

  Third, add them to the `mock.module("../src/lib/tauri", …)` factory (line 80-95), directly under `renderCardPreview`:

  ```tsx
    renderCardPreview: (cardId: string) => previewImpl(cardId),
    getServerPlugins: () => serverPluginsImpl(),
    getServerCardState: () => serverCardStateImpl(),
  ```

  Add `PluginCatalog` and `ServerCardState` to the `import type { … } from "../src/lib/types";` block (lines 22-31). Then the test itself, at the end of the `describe("settings accessibility and states", …)` block:

  ```tsx
  test("each server read names its backend command exactly, and passes no arguments", async () => {
    coreInvocations.length = 0;
    expect(await realGetServerPlugins()).toEqual({ plugins: [], load_failures: [] });
    expect(await realGetServerCardState()).toEqual([]);
    expect(coreInvocations).toEqual([
      { command: "get_server_plugins", args: undefined },
      { command: "get_server_card_state", args: undefined },
    ]);
  });
  ```

  Run it: `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun test tests/components.test.tsx -t "names its backend command exactly"` → fails with `TypeError: realGetServerPlugins is not a function`.

  Implement — append the DTOs to `src/lib/types.ts` directly after `CardDataSnapshot` (line 335):

  ```ts
  export type PluginTemplateKind = "display-list" | "svg";

  export interface PluginCatalogAsset {
    file: string;
    kind: string;
    byte_length: number;
    digest: string;
  }

  export interface PluginCatalogEntry {
    id: string;
    name: string;
    version: string;
    node_count: number;
    assets: PluginCatalogAsset[];
    display_name: string | null;
    description: string | null;
    manifest_version: number;
    template: PluginTemplateKind;
    refresh_minutes: number;
  }

  export interface PluginLoadFailure {
    id: string;
    error: string;
  }

  export interface PluginCatalog {
    plugins: PluginCatalogEntry[];
    load_failures: PluginLoadFailure[];
  }

  /** The server's own view of one plugin card, projected onto this window's snapshot. */
  export interface ServerCardState {
    card_id: string;
    provider: ProviderState;
    hero: string | null;
    errors: CardError[];
  }
  ```

  Replace `PreviewFrame` (lines 380-383) with:

  ```ts
  /**
   * `png_base64` is null exactly when the renderer produced no pixels; `state` then
   * carries the word for why ("Waiting for the first refresh", "Plugin cards render on
   * the server", the server's own error). Built-in cards keep `png_base64` set and
   * `state` null, so nothing about them changes.
   */
  export interface PreviewFrame {
    png_base64: string | null;
    sample: boolean;
    state: string | null;
  }
  ```

  Then widen `IpcContractFixtures` (lines 407-440) in the same file. Task 4 added `plugin_catalog` and `server_card_state` to the Rust contract fixture, which regenerated `src/lib/types.contract.ts`; that file ends `} as const satisfies IpcContractFixtures;`, so without matching members here every new fixture key is an excess property and `bun run check` fails. The two go between `autostart_status` and `preview_frame`, matching the fixture's key order — the whole edited interface:

  ```ts
  // This fixture shape is generated from Rust serialization in a backend test, then
  // compiled against these declarations. Either side changing makes CI fail.
  export interface IpcContractFixtures {
    snapshot: AppSnapshot;
    configs: AppConfig[];
    card_settings: CardSettings[];
    playlists: Playlist[];
    playlist_entries: PlaylistEntry[];
    card_alerts: CardAlert[];
    alert_holds: AlertHold[];
    carousel_advances: CarouselAdvance[];
    calendar_sources: CalendarSource[];
    display_templates: DisplayTemplate[];
    tap_actions: WidgetTapAction[];
    refresh_policies: RefreshPolicy[];
    weather_units: WeatherUnits[];
    asset_sources: AssetSettings["source"][];
    asset_kinds: AssetSettings["kind"][];
    update_channels: UpdaterSettings["channel"][];
    update_check_policies: UpdaterSettings["checks"][];
    display_orientations: DisplayOrientation[];
    device_capabilities: DeviceCapability[];
    runtime_states: RuntimeState[];
    connection_states: ConnectionState[];
    provider_states: ProviderState[];
    pomodoro_states: PomodoroState[];
    card_data: CardDataSnapshot[];
    persistence_states: PersistenceState[];
    validation_codes: ValidationCode[];
    pomodoro_actions: PomodoroAction[];
    errors: IpcError[];
    draft_validation: DraftValidation;
    config_apply_result: ConfigApplyResult;
    autostart_status: AutostartStatus;
    plugin_catalog: PluginCatalog;
    server_card_state: ServerCardState[];
    preview_frame: PreviewFrame;
  }
  ```

  Add the two wrappers to `src/lib/tauri.ts` after `renderCardPreview` (line 170), and add `PluginCatalog` and `ServerCardState` to its `import type` block (lines 4-15):

  ```ts
  export function getServerPlugins(): Promise<PluginCatalog> {
    return invokeTyped("get_server_plugins");
  }

  export function getServerCardState(): Promise<ServerCardState[]> {
    return invokeTyped("get_server_card_state");
  }
  ```

  Two mechanical consequences of the nullable frame, both in this commit. The suite's seventeen `PreviewFrame` literals gain the third field:

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && \
    sed -i '' -E 's/(png_base64: [^,]+, sample: (true|false))/\1, state: null/g' \
      tests/components.test.tsx && \
    grep -c "state: null" tests/components.test.tsx
  ```

  That must print `17`. And `DevicePreview` stops assuming a frame has pixels — replace lines 22, 41 and 75-83 of `src/components/DevicePreview.tsx`:

  ```tsx
  const [frame, setFrame] = useState<{ pngBase64: string | null; sample: boolean } | null>(null);
  ```

  ```tsx
          setFrame({ pngBase64: rendered.png_base64, sample: rendered.sample });
  ```

  ```tsx
          ) : frame?.pngBase64 ? (
            <img
              alt={`Device preview of ${widget.id}`}
              width={448}
              height={368}
              src={`data:image/png;base64,${frame.pngBase64}`}
            />
          ) : null}
          {/* The badge means "a real frame, drawn from sample data". A frame with no
              pixels has nothing to badge; Step 14 gives that case its own word. */}
          {frame?.sample && frame.pngBase64 && <span className="stage__badge">No data yet</span>}
  ```

  Run to pass: `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun test && /Users/rodion/.bun/bin/bun run check`. Both must be silent. `bun run check` has been red since Task 4 regenerated `src/lib/types.contract.ts` — `error: Object literal may only specify known properties, and 'plugin_catalog' does not exist in type 'IpcContractFixtures'`, and the same for `server_card_state` — and the widened interface above is the only thing in this task that clears it, so a green `check` here is the proof that the two fixture keys and the two members agree.

  Commit:

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -am \
    "feat: type the server plugin catalog, card state and stateful preview frame" -m \
  "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
  ```

- [ ] **Step 2: Name a plugin card by its display name, and delete the word "Plugin".**

  Write the failing test in `tests/configDraft.test.ts`. Add `cardKindName`, `cardName` and `pluginCardFlag` to the import block (lines 3-34), and widen the type import on line 35 to
  `import type { AddableCardKind, AppConfig, CardSettings, DeviceTier, PluginCatalog, ValidationIssue } from "../src/lib/types";`.
  Then add a catalog helper beside `pluginCard()` (line 78-88) and two tests:

  ```ts
  function pluginCatalog(displayName: string | null = "Air quality"): PluginCatalog {
    return {
      plugins: [
        {
          id: "com.example.air-quality",
          name: "aqi",
          version: "1.0.0",
          node_count: 7,
          assets: [],
          display_name: displayName,
          description: "EPA index for a location",
          manifest_version: 2,
          template: "display-list",
          refresh_minutes: 15,
        },
      ],
      load_failures: [],
    };
  }

  test("a plugin card is named by its display name, and falls back to its id twice over", () => {
    const card = pluginCard();
    expect(cardLabel(card, pluginCatalog())).toBe("Air quality");
    // Fallback one: the catalog knows the plugin but the manifest declared no name.
    expect(cardLabel(card, pluginCatalog(null))).toBe("com.example.air-quality");
    // Fallback two: no catalog at all (local tier, or the server was unreachable).
    expect(cardLabel(card, null)).toBe("com.example.air-quality");
    expect(cardLabel(card)).toBe("com.example.air-quality");
    // A catalog that does not carry this id cannot rename it either.
    expect(cardLabel({ ...card, plugin_id: "com.example.gone" }, pluginCatalog())).toBe(
      "com.example.gone",
    );
  });

  test("no card kind is called \u201cPlugin\u201d on any surface", () => {
    const kinds: AddableCardKind[] = [
      "clock",
      "pomodoro",
      "calendar",
      "weather",
      "json-feed",
      "rss",
    ];
    expect(kinds.map(cardKindName)).not.toContain("Plugin");
    expect(cardName(pluginCard())).toBe("Office air");
    expect(cardName({ ...pluginCard(), title: "" })).toBe("com.example.air-quality");
  });
  ```

  Run it: `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun test tests/configDraft.test.ts -t "named by its display name"` → fails with `expect(received).toBe(expected)  Expected: "Air quality"  Received: "com.example.air-quality"`.

  Implement in `src/lib/configDraft.ts`. Add `DeviceTier` and `PluginCatalog` to the `import type` block (lines 1-9), then replace `cardLabel` (lines 66-78) and `cardKindName` (lines 89-106):

  ```ts
  /**
   * What a card is called on every surface that identifies one: the loop tile, the
   * ring legend, the editor heading, the picker.
   *
   * It is the template's name, not the owner's title, by explicit owner direction: a
   * person meeting a card called "Outside" or "Desk" for the first time learns nothing
   * from it, where "Weather" and "Digital clock" say what the thing is. The owner's own
   * words survive as `cardTitle` — a quiet second line beside the label, never the
   * thing that names the card.
   *
   * A plugin card's template is its plugin, so its display name is whatever the
   * server's catalog declares for it. Both fallbacks land on the plugin id rather than
   * the word "Plugin": an id at least identifies the thing, where a category name
   * identified every plugin card identically. The word that says *why* a bare id is
   * showing is `pluginCardFlag`, not this — a name is a name, not a diagnosis.
   */
  export function cardLabel(card: CardSettings, catalog?: PluginCatalog | null): string {
    if (card.kind !== "plugin") {
      return cardKindName(card.kind);
    }
    const entry = catalog?.plugins.find((plugin) => plugin.id === card.plugin_id);
    return entry?.display_name ?? card.plugin_id;
  }
  ```

  ```ts
  export function cardKindName(kind: AddableCardKind): string {
    switch (kind) {
      case "clock":
        return "Digital clock";
      case "pomodoro":
        return "Pomodoro";
      case "calendar":
        return "ICS calendar";
      case "weather":
        return "Weather";
      case "json-feed":
        return "JSON feed";
      case "rss":
        return "RSS feed";
    }
  }
  ```

  Narrowing the parameter to `AddableCardKind` is what makes the deletion permanent: a future caller cannot hand it a plugin card without a type error. `CardKind` is now unused in this file — drop it from the `import type` block on line 4 or `bun run lint` fails. `cardName`'s own plugin arm (`card.title || card.plugin_id`, line 62) is unchanged and still compiles, because its other arms narrow to addable kinds.

  Run to pass: `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun test tests/configDraft.test.ts && /Users/rodion/.bun/bin/bun run check`.

  Commit:

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -am \
    "feat: name a plugin card by its catalog display name" -m \
  "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
  ```

- [ ] **Step 3: Print the word that explains a bare plugin id.**

  Write the failing test in `tests/configDraft.test.ts`:

  ```ts
  test("a bare plugin id always carries the word that explains it", () => {
    const card = pluginCard();
    const missing = { ...card, plugin_id: "com.example.gone" };
    const networked: DeviceTier = "networked";
    const local: DeviceTier = "local";

    // Networked, catalog loaded, plugin present: the name is the whole story.
    expect(pluginCardFlag(card, pluginCatalog(), networked)).toBeNull();
    // Networked, catalog loaded, plugin absent: the server does not have it.
    expect(pluginCardFlag(missing, pluginCatalog(), networked)).toBe("not on the server");
    // Local tier: there is no server to render it, whatever a stale catalog says.
    expect(pluginCardFlag(card, pluginCatalog(), local)).toBe("needs the server");
    expect(pluginCardFlag(missing, null, local)).toBe("needs the server");
    // Networked with no catalog yet: unknown is not the same as absent, so no word.
    expect(pluginCardFlag(missing, null, networked)).toBeNull();
    // Ownership not yet resolved: also unknown, also silent.
    expect(pluginCardFlag(missing, null, null)).toBeNull();
    // Built-in cards never carry it.
    expect(pluginCardFlag(initialConfig().cards[0], null, local)).toBeNull();
  });
  ```

  Run it: `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun test tests/configDraft.test.ts -t "word that explains it"` → fails with `TypeError: pluginCardFlag is not a function`.

  Implement in `src/lib/configDraft.ts`, directly under `cardLabel`:

  ```ts
  export type PluginCardFlag = "not on the server" | "needs the server";

  /**
   * The word beside a plugin card whose name could not be resolved. Order matters:
   * local tier is the reason that outranks every other, because no catalog, however
   * complete, can make a plugin render on a Mac that owns the display itself. A
   * missing catalog in networked tier is deliberately silent — the window has not
   * heard from the server yet, and "not on the server" would be an accusation the
   * app cannot support.
   */
  export function pluginCardFlag(
    card: CardSettings,
    catalog: PluginCatalog | null,
    tier: DeviceTier | null,
  ): PluginCardFlag | null {
    if (card.kind !== "plugin") {
      return null;
    }
    if (tier === "local") {
      return "needs the server";
    }
    if (!catalog) {
      return null;
    }
    return catalog.plugins.some((plugin) => plugin.id === card.plugin_id)
      ? null
      : "not on the server";
  }
  ```

  Run to pass: `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun test tests/configDraft.test.ts && /Users/rodion/.bun/bin/bun run check`.

  Commit:

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -am \
    "feat: flag a plugin card the catalog or the tier cannot render" -m \
  "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
  ```

- [ ] **Step 4: Add a plugin card through the one add path every other card uses.**

  Write the failing test in `tests/configDraft.test.ts`:

  ```ts
  test("a plugin card is added by the same call, with the same two caps, as any other", () => {
    const { config, cardId } = addCard(initialConfig(), {
      kind: "plugin",
      pluginId: "com.example.air-quality",
      refreshMinutes: 15,
    });
    expect(cardId).toBe("plugin");
    const added = config.cards.at(-1);
    if (added?.kind !== "plugin") {
      throw new Error("addCard did not append the plugin card");
    }
    expect(added).toEqual({
      kind: "plugin",
      id: "plugin",
      title: "",
      plugin_id: "com.example.air-quality",
      tap_action: { kind: "none" },
      refresh: { kind: "interval", minutes: 15 },
      alert: { kind: "none" },
    });
    // Enrolled in the loop by the same code path as a built-in add.
    expect(config.playlists[0].entries.at(-1)?.card_id).toBe("plugin");

    // The card id is minted, never derived: a 64-byte plugin id cannot fit the
    // 32-byte card-id bound, and a second card of the same plugin must not collide.
    const second = addCard(config, {
      kind: "plugin",
      pluginId: "com.example.air-quality",
      refreshMinutes: 15,
    });
    expect(second.cardId).toBe("plugin-2");

    const cardFull: AppConfig = {
      ...initialConfig(),
      cards: Array.from({ length: 8 }, (_, index) => ({
        ...initialConfig().cards[0],
        id: `c${index}`,
      })),
    };
    expect(addCard(cardFull, { kind: "plugin", pluginId: "aqi", refreshMinutes: 15 })).toEqual({
      config: cardFull,
      cardId: null,
    });

    const entryFull: AppConfig = {
      ...initialConfig(),
      playlists: [
        {
          ...initialConfig().playlists[0],
          entries: Array.from({ length: 8 }, (_, index) => ({
            card_id: `c${index}`,
            dwell_seconds: null,
          })),
        },
      ],
    };
    expect(addCard(entryFull, { kind: "plugin", pluginId: "aqi", refreshMinutes: 15 })).toEqual({
      config: entryFull,
      cardId: null,
    });
  });
  ```

  Run it: `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun test tests/configDraft.test.ts -t "same two caps"` → fails to compile with `error: Argument of type '{ kind: "plugin"; pluginId: string; refreshMinutes: number; }' is not assignable to parameter of type 'AddableCardKind'`, and at runtime the call throws before any assertion.

  Implement in `src/lib/configDraft.ts` — replace the whole of `addCard` and its doc comment (lines 119-233) with this. Everything after the `switch` is byte-for-byte the current tail:

  ```ts
  /**
   * What the caller is asking to add. A built-in is named by its kind; a plugin is
   * named by its registry id plus the cadence its manifest declares, because the
   * catalog is the only thing that knows either.
   */
  export type AddCardRequest =
    | AddableCardKind
    | { kind: "plugin"; pluginId: string; refreshMinutes: number };

  /// Appends a new card with sane defaults for its kind and enrols it at the end
  /// of the active loop in the same draft. Supports all six built-in card kinds
  /// and a plugin from the server's catalog. Both v6 limits are checked before
  /// either collection changes, so adding is atomic even when a legacy card
  /// outside the loop has filled only one limit.
  export function addCard(
    config: AppConfig,
    request: AddCardRequest,
  ): {
    config: AppConfig;
    cardId: string | null;
  } {
    const playlist = activePlaylist(config);
    if (
      config.cards.length >= MAX_CARDS ||
      !playlist ||
      playlist.entries.length >= MAX_PLAYLIST_ENTRIES
    ) {
      return { config, cardId: null };
    }
    const used = new Set(config.cards.map((card) => card.id));
    // "plugin" as the id stem, never the plugin id: `plugin_id` is bounded at 64 bytes
    // and a card id at 32, and two cards of one plugin are legitimate.
    const cardId = nextId(typeof request === "string" ? request : "plugin", used);
    const common = {
      id: cardId,
      tap_action: { kind: "none" } as const,
      alert: { kind: "none" } as const,
    };

    let card: CardSettings;
    if (typeof request !== "string") {
      card = {
        kind: "plugin",
        ...common,
        // Blank on purpose: the display name already says what the card is, and a
        // pre-filled title would be a second name nobody chose.
        title: "",
        plugin_id: request.pluginId,
        refresh: { kind: "interval", minutes: request.refreshMinutes },
      };
    } else {
      switch (request) {
        case "clock":
          card = {
            kind: request,
            ...common,
            title: "Desk",
            show_seconds: true,
            template: { kind: "digital-clock" },
            refresh: { kind: "device-local" },
          };
          break;
        case "pomodoro":
          card = {
            kind: request,
            ...common,
            label: "Focus",
            duration_seconds: 25 * 60,
            template: { kind: "progress-ring" },
            tap_action: { kind: "start-pause" },
            refresh: { kind: "device-local" },
            alert: { kind: "on-timer-finish", hold: { kind: "until-dismissed" } },
          };
          break;
        case "calendar":
          card = {
            kind: request,
            ...common,
            title: "Up next",
            source: { kind: "url", value: "" },
            template: { kind: "row-list" },
            refresh: { kind: "interval", minutes: 15 },
          };
          break;
        case "weather":
          card = {
            kind: request,
            ...common,
            title: "Weather",
            location: "",
            units: "metric",
            // `icon-badge-text` is the template weather's field composition was designed
            // for (`value`/`label`/`badge`/`icon`/`temperature_tenths`/
            // `apparent_temperature_tenths`/`unit`), and `wire_config()` now lowers it to
            // the device — see `companion/crates/app-core/src/config.rs`. `icon_asset_id`
            // stays unset here: rendering a pushed custom icon needs
            // `CAPABILITY_ASSET_TRANSFER`, which is a later milestone task; until then the
            // device renders its built-in icon for the `icon` field.
            template: { kind: "icon-badge-text", icon_asset_id: null },
            refresh: { kind: "interval", minutes: 30 },
          };
          break;
        case "json-feed":
          card = {
            kind: request,
            ...common,
            title: "Feed",
            url: "",
            mappings: [],
            // `big-number-label` is the template json-feed's single mapped value is
            // designed for, and `wire_config()` now lowers it to the device — see
            // `companion/crates/app-core/src/config.rs`.
            template: { kind: "big-number-label" },
            refresh: { kind: "interval", minutes: 15 },
          };
          break;
        case "rss":
          card = {
            kind: request,
            ...common,
            title: "Headlines",
            url: "",
            max_items: 3,
            template: { kind: "row-list" },
            refresh: { kind: "interval", minutes: 30 },
          };
          break;
      }
    }

    // Spread the COPIED config's own `cards` array, not the original `config.cards` —
    // otherwise the `cards` key here overwrites `copyConfig`'s deep copy with a shallow
    // spread of the original elements, silently making that deep copy dead work on this
    // path (every pre-existing card in the returned draft would alias the live snapshot's
    // card objects instead of being an independent copy).
    const copied = copyConfig(config);
    const withCard = { ...copied, cards: [...copied.cards, card] };
    return { config: addEntry(withCard, playlist.id, cardId), cardId };
  }
  ```

  Run to pass: `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun test tests/configDraft.test.ts && /Users/rodion/.bun/bin/bun run check`.

  Commit:

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -am \
    "feat: add a plugin card through addCard's one path" -m \
  "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
  ```

- [ ] **Step 5: Let the ring legend read the catalog too.**

  Write the failing test in `tests/configDraft.test.ts` (`pluginCard()` in this file has id `plugin-card`):

  ```ts
  test("filmstripSegments names a plugin segment from the catalog it is given", () => {
    const config: AppConfig = {
      ...initialConfig(),
      cards: [pluginCard()],
      playlists: [
        {
          ...initialConfig().playlists[0],
          advance: { kind: "timed", default_dwell_seconds: 20 },
          entries: [{ card_id: "plugin-card", dwell_seconds: 30 }],
        },
      ],
    };
    expect(filmstripSegments(config, pluginCatalog())[0].name).toBe("Air quality");
    expect(filmstripSegments(config)[0].name).toBe("com.example.air-quality");
  });
  ```

  Run it: `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun test tests/configDraft.test.ts -t "names a plugin segment"` → fails with `expect(received).toBe(expected)  Expected: "Air quality"  Received: "com.example.air-quality"`.

  Implement in `src/lib/configDraft.ts` — replace `filmstripSegments` (lines 451-481) in full. One parameter added, one call site changed inside; everything else is the current body:

  ```ts
  export function filmstripSegments(
    config: AppConfig,
    catalog?: PluginCatalog | null,
  ): FilmstripSegment[] {
    const playlist = activePlaylist(config);
    if (!playlist) {
      return [];
    }
    const fallback = playlist.advance.kind === "timed" ? playlist.advance.default_dwell_seconds : 0;
    const cards = new Map(config.cards.map((card) => [card.id, card]));
    const entries = playlist.entries.flatMap((entry) => {
      const card = cards.get(entry.card_id);
      return card ? [{ card, entry }] : [];
    });
    const dwellSeconds = entries.map(({ entry }) =>
      playlist.advance.kind === "timed" ? (entry.dwell_seconds ?? fallback) : 0,
    );
    const total = dwellSeconds.reduce((sum, seconds) => sum + seconds, 0);
    const equalShare = entries.length > 0 ? 100 / entries.length : 0;
    let offset = 0;
    return entries.map(({ card }, index) => {
      const widthPercent = total > 0 ? (dwellSeconds[index] / total) * 100 : equalShare;
      const segment: FilmstripSegment = {
        cardId: card.id,
        name: cardLabel(card, catalog),
        title: cardTitle(card),
        dwellSeconds: dwellSeconds[index],
        widthPercent,
        offsetPercent: offset,
      };
      offset += widthPercent;
      return segment;
    });
  }
  ```

  Run to pass: `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun test tests/configDraft.test.ts && /Users/rodion/.bun/bin/bun run check`.

  Commit:

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -am \
    "feat: thread the plugin catalog through the loop segments" -m \
  "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
  ```

- [ ] **Step 6: Poll the server's plugin state on a 30 s beat, only while the window is visible.**

  Write the failing test in `tests/useAppState.test.ts`, reusing the file's existing `FakeEventTarget` and `flushPromises`. Extend its imports to
  `import { startAppStateSubscription, startServerCardStatePoll, type AppStateSubscriptionOptions } from "../src/lib/useAppState";`
  and `import type { AppSnapshot, IpcError, ServerCardState } from "../src/lib/types";`, then append:

  ```ts
  describe("startServerCardStatePoll", () => {
    /** A scheduler whose `clear` really stops the handler, so "after stop" means it. */
    function fakeScheduler() {
      const handlers = new Map<number, () => void>();
      let nextHandle = 0;
      let cleared = 0;
      return {
        scheduler: {
          set: (handler: () => void) => {
            nextHandle += 1;
            handlers.set(nextHandle, handler);
            return nextHandle;
          },
          clear: (handle: number) => {
            cleared += 1;
            handlers.delete(handle);
          },
        },
        tick: () => {
          for (const handler of [...handlers.values()]) {
            handler();
          }
        },
        cleared: () => cleared,
      };
    }

    test("polls on start, on focus and on each tick, but never while hidden", async () => {
      const focusTarget = new FakeEventTarget();
      const visibilityTarget = new FakeEventTarget();
      const { scheduler, tick, cleared } = fakeScheduler();
      const accepted: ServerCardState[][] = [];
      let fetches = 0;

      const stop = startServerCardStatePoll({
        fetchCardState: async () => {
          fetches += 1;
          return [];
        },
        onCardState: (state) => accepted.push(state),
        onError: () => {},
        focusTarget,
        visibilityTarget,
        scheduler,
      });
      await flushPromises();
      expect(fetches).toBe(1);
      expect(accepted).toHaveLength(1);

      focusTarget.dispatch("focus");
      tick();
      await flushPromises();
      expect(fetches).toBe(3);

      visibilityTarget.visibilityState = "hidden";
      tick();
      focusTarget.dispatch("focus");
      await flushPromises();
      expect(fetches).toBe(3);

      visibilityTarget.visibilityState = "visible";
      visibilityTarget.dispatch("visibilitychange");
      await flushPromises();
      expect(fetches).toBe(4);

      stop();
      tick();
      focusTarget.dispatch("focus");
      await flushPromises();
      expect(fetches).toBe(4);
      expect(cleared()).toBe(1);
      expect(focusTarget.count("focus")).toBe(0);
      expect(visibilityTarget.count("visibilitychange")).toBe(0);
    });

    test("a failed poll is reported as a typed error and never as a card state", async () => {
      const { scheduler } = fakeScheduler();
      const errors: IpcError[] = [];
      const stop = startServerCardStatePoll({
        fetchCardState: async () => {
          throw { category: "runtime-unavailable", message: "no runtime for this device" };
        },
        onCardState: () => {
          throw new Error("a failed poll must not publish a card state");
        },
        onError: (error) => errors.push(error),
        scheduler,
      });
      await flushPromises();
      expect(errors).toEqual([
        { category: "runtime-unavailable", message: "no runtime for this device" },
      ]);
      stop();
    });
  });
  ```

  Run it: `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun test tests/useAppState.test.ts` → fails with `TypeError: startServerCardStatePoll is not a function`.

  Implement in `src/lib/useAppState.ts`, immediately after `startAppStateSubscription` (line 99) so the two live side by side and share the same `EventTargetLike`/`VisibilityTargetLike` shapes. Add `getServerCardState` and `getServerPlugins` to the value import from `./tauri` (lines 3-15) and `PluginCatalog`, `ServerCardState` to the type import (lines 16-24):

  ```ts
  export const SERVER_CARD_STATE_POLL_MS = 30_000;
  export const SERVER_PLUGIN_NOTICE = "Couldn't reach the server for plugin data.";

  export interface IntervalScheduler {
    set: (handler: () => void, ms: number) => number;
    clear: (handle: number) => void;
  }

  export interface ServerCardStatePollOptions {
    fetchCardState: () => Promise<ServerCardState[]>;
    onCardState: (state: ServerCardState[]) => void;
    onError: (error: IpcError) => void;
    focusTarget?: EventTargetLike;
    visibilityTarget?: VisibilityTargetLike;
    intervalMs?: number;
    scheduler?: IntervalScheduler;
  }

  /**
   * The server's plugin state is the one thing this window cannot learn from its own
   * runtime, so it asks — on a slow beat, and only while someone is looking. A hidden
   * window polling a homelab every thirty seconds forever is a cost with no reader.
   *
   * Shaped like `startAppStateSubscription`: an imperative start returning its own
   * cleanup, so the hook is a two-line caller and the behaviour is testable without a
   * renderer.
   */
  export function startServerCardStatePoll(options: ServerCardStatePollOptions): () => void {
    let active = true;
    const scheduler: IntervalScheduler = options.scheduler ?? {
      set: (handler, ms) => window.setInterval(handler, ms),
      clear: (handle) => window.clearInterval(handle),
    };
    const visible = () =>
      options.visibilityTarget === undefined ||
      options.visibilityTarget.visibilityState === "visible";
    const poll = () => {
      if (!visible()) {
        return;
      }
      void options
        .fetchCardState()
        .then((state) => {
          if (active) {
            options.onCardState(state);
          }
        })
        .catch((error) => {
          if (active) {
            options.onError(toIpcError(error));
          }
        });
    };
    const onFocus: EventListener = () => poll();
    const onVisibilityChange: EventListener = () => {
      if (visible()) {
        poll();
      }
    };
    options.focusTarget?.addEventListener("focus", onFocus);
    options.visibilityTarget?.addEventListener("visibilitychange", onVisibilityChange);
    const handle = scheduler.set(poll, options.intervalMs ?? SERVER_CARD_STATE_POLL_MS);
    poll();
    return () => {
      active = false;
      scheduler.clear(handle);
      options.focusTarget?.removeEventListener("focus", onFocus);
      options.visibilityTarget?.removeEventListener("visibilitychange", onVisibilityChange);
    };
  }
  ```

  Run to pass: `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun test tests/useAppState.test.ts && /Users/rodion/.bun/bin/bun run check`.

  Commit:

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -am \
    "feat: poll the server's plugin card state while the window is visible" -m \
  "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
  ```

- [ ] **Step 7: Hold the catalog, the card state and one notice in `useAppState`.**

  Write the failing test in `tests/components.test.tsx`, in the mounted-`App` describe block (the one containing the pairing tests around line 400). It restores both server impls in `finally`, matching this file's existing convention for shared mutable mocks:

  ```tsx
  test("an unreachable server costs the window one notice and no plugin state", async () => {
    snapshotImpl = async () => ({
      ...snapshot,
      device: { ...snapshot.device, tier: "networked" },
      config: {
        ...snapshot.config,
        cards: [pluginCard("air")],
        playlists: [
          {
            id: "workday",
            name: "Workday",
            advance: { kind: "timed" as const, default_dwell_seconds: 20 },
            entries: [{ card_id: "air", dwell_seconds: null }],
          },
        ],
        active_playlist_id: "workday",
      },
      providers: [],
      pomodoros: [],
      card_data: [],
      card_errors: [],
    });
    networkSettingsImpl = async () => ({
      server_url: "https://desk.example",
      device_id: "desk-1",
      tier: "networked",
    });
    previewImpl = async () => ({ png_base64: "cHJldmlldw==", sample: false, state: null });
    let catalogCalls = 0;
    serverPluginsImpl = async () => {
      catalogCalls += 1;
      if (catalogCalls === 1) {
        throw { category: "runtime-unavailable", message: "connection refused" };
      }
      return {
        plugins: [
          {
            id: "com.example.air-quality",
            name: "aqi",
            version: "1.0.0",
            node_count: 7,
            assets: [],
            display_name: "Air quality",
            description: "EPA index for a location",
            manifest_version: 2,
            template: "display-list" as const,
            refresh_minutes: 15,
          },
        ],
        load_failures: [],
      };
    };
    serverCardStateImpl = async () => [];

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await renderPreviewInto(root, <App />);
      await waitFor(() => {
        expect(container.textContent).toContain("Couldn't reach the server for plugin data.");
      });
      // A successful card-state poll must not clear a failed catalog read: the two
      // failures are tracked apart even though they share one sentence.
      expect(container.textContent).toContain("com.example.air-quality");

      const retry = buttonWithText(container, "Try again");
      expect(retry).toBeDefined();
      await act(async () => retry?.click());
      await waitFor(() => {
        expect(container.textContent).not.toContain("Couldn't reach the server for plugin data.");
        expect(container.textContent).toContain("Air quality");
      });
      expect(catalogCalls).toBe(2);
    } finally {
      await act(async () => root.unmount());
      container.remove();
      serverPluginsImpl = async () => ({ plugins: [], load_failures: [] });
      serverCardStateImpl = async () => [];
    }
  });
  ```

  Run it: `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun test tests/components.test.tsx -t "costs the window one notice"` → fails with `expect(received).toContain(expected)  … "Couldn't reach the server for plugin data."`.

  Implement in `src/lib/useAppState.ts`. First the state, added beside the existing `useState` calls in `useAppState` (after line 178's `networkSettingsLoaded`):

  ```ts
  const [pluginCatalog, setPluginCatalog] = useState<PluginCatalog | null>(null);
  const [serverCardState, setServerCardState] = useState<ServerCardState[]>([]);
  // Two failures, one sentence. Tracked apart because a working card-state poll is
  // not evidence that the catalog read succeeded, and clearing one on the other's
  // success would make the notice depend on which promise settled first.
  const [catalogFailed, setCatalogFailed] = useState(false);
  const [cardStateFailed, setCardStateFailed] = useState(false);
  const [catalogGeneration, setCatalogGeneration] = useState(0);
  const refreshCatalog = useCallback(() => setCatalogGeneration((current) => current + 1), []);
  ```

  Then the two effects and the derived sentence, inserted between `ownershipTier`'s computation (line 330) and the `return {` on line 332:

  ```ts
  const catalogError = catalogFailed || cardStateFailed ? SERVER_PLUGIN_NOTICE : null;

  useEffect(() => {
    if (ownershipTier !== "networked") {
      // Local tier has no server, so a catalog held from a previous pairing would be
      // a stale promise the app cannot keep. `pluginCardFlag` prints the reason.
      setPluginCatalog(null);
      setServerCardState([]);
      setCatalogFailed(false);
      setCardStateFailed(false);
      return;
    }
    let active = true;
    void getServerPlugins()
      .then((catalog) => {
        if (active) {
          setPluginCatalog(catalog);
          setCatalogFailed(false);
        }
      })
      .catch(() => {
        if (active) {
          setCatalogFailed(true);
        }
      });
    return () => {
      active = false;
    };
  }, [ownershipTier, catalogGeneration]);

  useEffect(() => {
    if (ownershipTier !== "networked") {
      return;
    }
    return startServerCardStatePoll({
      fetchCardState: getServerCardState,
      onCardState: (next) => {
        setServerCardState(next);
        setCardStateFailed(false);
      },
      onError: () => setCardStateFailed(true),
      focusTarget: window,
      visibilityTarget: document,
    });
  }, [ownershipTier]);
  ```

  Add the four fields to `AppStateValue` (after `ownershipTier` on line 118):

  ```ts
    /** The server's plugin registry, or null in local tier and before the first read. */
    pluginCatalog: PluginCatalog | null;
    /** One sentence for either server read having failed; null when both are fine. */
    catalogError: string | null;
    refreshCatalog: () => void;
    serverCardState: ServerCardState[];
  ```

  and to the returned object (after `ownershipTier,` on line 339):

  ```ts
      pluginCatalog,
      catalogError,
      refreshCatalog,
      serverCardState,
  ```

  Then render the notice in `src/App.tsx`. Destructure the four new fields from `useAppState()` (after `ownershipTier,` on line 75):

  ```tsx
      pluginCatalog,
      catalogError,
      refreshCatalog,
      serverCardState,
  ```

  and insert the notice immediately before the card-error notice (line 464's `{snapshot.card_errors.length > 0 && (`):

  ```tsx
          {/* One notice for either server read failing. The last projection and the
              last catalog are kept — a plugin card keeps its name and its value
              rather than blanking because a poll missed. */}
          {catalogError && (
            <aside className="notice notice--warn" role="status">
              <div>
                <strong>{catalogError}</strong>
                <p>Plugin names and previews are the last ones this window received.</p>
                <button className="button button--quiet" type="button" onClick={refreshCatalog}>
                  Try again
                </button>
              </div>
            </aside>
          )}

  ```

  Run to pass: `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun test tests/components.test.tsx && /Users/rodion/.bun/bin/bun run check`.

  Commit:

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -am \
    "feat: read the server plugin catalog and card state from the window" -m \
  "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
  ```

- [ ] **Step 8: Make a plugin tile a complication: its name, its live value, its flags.**

  First give every existing `<CardList>` in the suite the three new props, mechanically:

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && \
    sed -i '' 's|^\( *\)pluginKinds={\[\]}$|\1pluginKinds={[]}\
\1catalog={null}\
\1serverCardState={[]}\
\1ownershipTier="networked"|' tests/components.test.tsx && \
    grep -c 'ownershipTier="networked"' tests/components.test.tsx
  ```

  That must print `19`. The twentieth call site is the multi-line `pluginKinds={…}` in the "menu arrow wrapping ignores stale refs" test (around line 1225); add the same three lines by hand directly under its closing `}` for `pluginKinds`.

  Now replace the test at lines 651-681 (`"plugin cards have a lossless tile and title editor but cannot be added"`) — its assertions pin exactly the behaviour this task removes (`tile-label">com.example.air-quality`, `card-tile__value numeral">Plugin`, `not.toContain("Plugins on the server")`). Its editor half is superseded by Step 10; delete the whole test and put this in its place:

  ```tsx
  test("a plugin tile is named, valued and flagged like any other complication", () => {
    const plugin = pluginCard();
    const catalog: PluginCatalog = {
      plugins: [
        {
          id: "com.example.air-quality",
          name: "aqi",
          version: "1.0.0",
          node_count: 7,
          assets: [],
          display_name: "Air quality",
          description: "EPA index for a location",
          manifest_version: 2,
          template: "display-list",
          refresh_minutes: 15,
        },
      ],
      load_failures: [],
    };
    const render = (
      cardCatalog: PluginCatalog | null,
      serverCardState: ServerCardState[],
      tier: "local" | "networked",
    ) =>
      renderToStaticMarkup(
        <CardList
          config={cardListConfig([plugin])}
          issues={[]}
          cardData={[]}
          pomodoros={[]}
          providers={[]}
          pluginKinds={[]}
          catalog={cardCatalog}
          serverCardState={serverCardState}
          ownershipTier={tier}
          selectedCardId={plugin.id}
          onSelect={() => {}}
          onAdd={() => {}}
          onChange={() => {}}
          onRemove={() => {}}
        />,
      );

    const live = render(
      catalog,
      [{ card_id: plugin.id, provider: { kind: "fresh" }, hero: "42", errors: [] }],
      "networked",
    );
    expect(live).toContain('class="tile-label">Air quality<');
    expect(live).toContain('<strong class="card-tile__value numeral">42</strong>');
    expect(live).toContain('class="card-tile__name">Office air<');
    expect(live).toContain('aria-label="Remove Air quality — Office air');
    // Nothing on the tile says "plugin", and the wire id is still never shown.
    expect(live).not.toContain(">Plugin<");
    expect(live).not.toContain(plugin.id);

    // No headline yet reads like a weather card with no data, not like a fault.
    expect(render(catalog, [], "networked")).toContain(
      '<strong class="card-tile__value numeral">—</strong>',
    );
    // The two flags, each beside the bare id it explains.
    expect(render({ plugins: [], load_failures: [] }, [], "networked")).toContain(
      '<span class="flag">not on the server</span>',
    );
    expect(render(catalog, [], "local")).toContain('<span class="flag">needs the server</span>');
  });
  ```

  Run it: `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun test tests/components.test.tsx -t "named, valued and flagged"` → fails to compile with `error: Property 'catalog' does not exist on type 'IntrinsicAttributes & CardListProps'`.

  Implement in `src/components/CardList.tsx`. Add `pluginCardFlag` to the value import from `../lib/configDraft` (lines 10-25) and `DeviceTier`, `PluginCatalog`, `ServerCardState` to the type import from `../lib/types` (lines 27-36). Then replace `CardListProps` (lines 49-61):

  ```tsx
  interface CardListProps {
    config: AppConfig;
    issues: ValidationIssue[];
    cardData: CardDataSnapshot[];
    pomodoros: PomodoroSnapshot[];
    providers: ProviderSnapshot[];
    pluginKinds: PluginKindOption[];
    /** The server's registry, or null in local tier and before the first read. */
    catalog: PluginCatalog | null;
    /** The server's own rows for this device's plugin cards. Empty in local tier. */
    serverCardState: ServerCardState[];
    ownershipTier: DeviceTier | null;
    selectedCardId: string | null;
    onSelect: (cardId: string) => void;
    onAdd: (kind: AddableCardKind) => void;
    onChange: (config: AppConfig) => void;
    onRemove: (cardId: string) => void;
  }
  ```

  Replace `tileValue` (lines 80-119) and `controlLabel` (lines 121-125):

  ```tsx
  /** The live fact that makes a tile a complication rather than a list row. */
  function tileValue(
    card: CardSettings,
    data: CardDataSnapshot | undefined,
    pomodoro: PomodoroSnapshot | undefined,
    now: Date,
    timezone: string,
    pluginHero: string | null,
  ): string {
    switch (card.kind) {
      case "clock":
        try {
          return new Intl.DateTimeFormat("en-GB", {
            hour: "2-digit",
            minute: "2-digit",
            timeZone: timezone,
          }).format(now);
        } catch {
          return "--:--";
        }
      case "pomodoro": {
        const seconds = pomodoro?.remaining_seconds ?? card.duration_seconds;
        return `${Math.floor(seconds / 60)}:${String(Math.floor(seconds % 60)).padStart(2, "0")}`;
      }
      case "weather":
      case "json-feed":
        return fieldText(data, "hero") ?? "—";
      case "calendar":
      case "rss": {
        const rowCount = (data?.fields ?? []).filter(
          (field) =>
            /^row\d+_title$/.test(field.key) &&
            field.value.kind === "text" &&
            field.value.value.trim() !== "",
        ).length;
        return rowCount > 0 ? String(rowCount) : "—";
      }
      case "plugin":
        // The server's evaluated `summary`, or the same em dash a weather card shows
        // before its first fetch. A tile owns one fact; it does not narrate.
        return pluginHero ?? "—";
    }
  }

  /** Every control that acts on one card names template and typed title together. */
  function controlLabel(card: CardSettings, catalog: PluginCatalog | null): string {
    const title = cardTitle(card);
    return title ? `${cardLabel(card, catalog)} — ${title}` : cardLabel(card, catalog);
  }
  ```

  Add the three props to the destructure (lines 127-139), after `pluginKinds,`:

  ```tsx
    pluginKinds,
    catalog,
    serverCardState,
    ownershipTier,
  ```

  Three exact-context edits inside `renderCardTile`:

  ```diff
       const stale =
         providerTrouble(providers.find((candidate) => candidate.widget_id === card.id)) !== null;
       const index = inLoop ? (entryIndex ?? -1) : -1;
  -    const label = controlLabel(card);
  +    const label = controlLabel(card, catalog);
  +    // The server's row for this card, when it has one, and the word that explains a
  +    // bare plugin id when the catalog or the tier cannot resolve it.
  +    const serverState = serverCardState.find((row) => row.card_id === card.id) ?? null;
  +    const pluginFlag = pluginCardFlag(card, catalog, ownershipTier);
       return (
  ```

  ```diff
  -          <span className="tile-label">{cardLabel(card)}</span>
  +          <span className="tile-label">{cardLabel(card, catalog)}</span>
             <strong className="card-tile__value numeral">
  -            {tileValue(card, data, pomodoro, now, config.preferences.timezone)}
  +            {tileValue(
  +              card,
  +              data,
  +              pomodoro,
  +              now,
  +              config.preferences.timezone,
  +              serverState?.hero ?? null,
  +            )}
             </strong>
  ```

  ```diff
           <span className="card-tile__flags">
             {!inLoop && <span className="flag">not in loop</span>}
  +          {pluginFlag && <span className="flag">{pluginFlag}</span>}
             {stale && <span className="flag flag--stale">stale</span>}
  ```

  The `stale` flag itself is unchanged: it already reads `providers`, which the Mac's projection overlays with the server's real provider state for plugin cards, so a plugin card now goes stale for the same reason and at the same moment a weather card does.

  Finally keep `App.tsx` compiling — add the three props to its `<CardList>` (line 541) with placeholders Step 13 replaces:

  ```diff
             pluginKinds={[]}
  +          catalog={null}
  +          serverCardState={[]}
  +          ownershipTier={ownershipTier}
             selectedCardId={selectedCardId}
  ```

  Run to pass: `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun test tests/components.test.tsx && /Users/rodion/.bun/bin/bun run check`.

  Commit:

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -am \
    "feat: render a plugin tile with its name, headline and flags" -m \
  "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
  ```

- [ ] **Step 9: Offer the catalog's plugins in the add menu, with the fallbacks the spec names.**

  Write the failing test in `tests/components.test.tsx`:

  ```tsx
  test("the add menu lists server plugins by display name, with description fallbacks", async () => {
    let added: { pluginId: string; refreshMinutes: number } | null = null;
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await act(async () =>
        root.render(
          <CardList
            config={cardListConfig([clockCard("clock", "Desk")])}
            issues={[]}
            cardData={[]}
            pomodoros={[]}
            providers={[]}
            pluginKinds={[
              {
                id: "aqi",
                version: "1.0.0",
                displayName: "Air quality",
                description: "EPA index for a location",
                onAdd: () => {
                  added = { pluginId: "aqi", refreshMinutes: 15 };
                },
              },
              {
                id: "agenda",
                version: "2.1.0",
                displayName: null,
                description: null,
                onAdd: () => {},
              },
            ]}
            catalog={null}
            serverCardState={[]}
            ownershipTier="networked"
            selectedCardId={null}
            onSelect={() => {}}
            onAdd={() => {}}
            onChange={() => {}}
            onRemove={() => {}}
          />,
        ),
      );
      const slot = container.querySelector<HTMLButtonElement>(".card-tile__add");
      await act(async () => slot?.click());
      expect(container.textContent).toContain("Plugins on the server");
      expect(container.textContent).toContain("Air quality");
      expect(container.textContent).toContain("EPA index for a location");
      // Both fallbacks: the id names it, "Plugin · <version>" describes it.
      expect(container.textContent).toContain("agenda");
      expect(container.textContent).toContain("Plugin · 2.1.0");

      const row = [...container.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(
        (button) => button.textContent?.includes("Air quality"),
      );
      await act(async () => row?.click());
      expect(added).toEqual({ pluginId: "aqi", refreshMinutes: 15 });
      expect(container.querySelector('[role="menu"]')).toBeNull();
    } finally {
      await act(async () => root.unmount());
      container.remove();
    }
  });
  ```

  The existing "menu arrow wrapping ignores stale refs" test passes a `pluginKinds` literal without `displayName`; add `displayName: null,` to that object in this commit (it keeps its `description: "Plugin weather"`, which the widened type still admits).

  Run it: `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun test tests/components.test.tsx -t "lists server plugins by display name"` → fails to compile with `error: Object literal may only specify known properties, and 'displayName' does not exist in type 'PluginKindOption'`.

  Implement in `src/components/CardList.tsx` — replace `PluginKindOption` (lines 40-47):

  ```tsx
  /** One row in the menu's server group, built by `App` from the plugin catalog. */
  export interface PluginKindOption {
    id: string;
    version: string;
    /** The manifest's `display_name`, or null when it declared none. */
    displayName: string | null;
    /** The manifest's `description`, or null when it declared none. */
    description: string | null;
    onAdd: () => void;
  }
  ```

  and the row's text (lines 576-579):

  ```diff
                           <span>
  -                          <strong>{plugin.id}</strong>
  -                          <small>{plugin.description ?? `Plugin · ${plugin.version}`}</small>
  +                          <strong>{plugin.displayName ?? plugin.id}</strong>
  +                          <small>{plugin.description ?? `Plugin · ${plugin.version}`}</small>
                           </span>
  ```

  Run to pass: `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun test tests/components.test.tsx && /Users/rodion/.bun/bin/bun run check`.

  Commit:

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -am \
    "feat: list the server's plugins in the add menu" -m \
  "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
  ```

- [ ] **Step 10: Give `plugin_id` a home: the editor's Plugin field.**

  First give every `<CardEditor>` in the suite the two new props, mechanically (there are exactly four `providerRefreshing={false}` lines — one in the `renderCardEditor` helper and three direct renders):

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && \
    sed -i '' 's|^\( *\)providerRefreshing={false}$|\1providerRefreshing={false}\
\1catalog={null}\
\1ownershipTier="networked"|' tests/components.test.tsx && \
    grep -c 'ownershipTier="networked"' tests/components.test.tsx
  ```

  That must print `23` (19 from Step 8 plus these 4). Then replace the `renderCardEditor` helper (lines 221-244 as they now stand) so its two new props come from parameters instead of literals — the file's other calls pass three arguments or fewer and stay unchanged:

  ```tsx
    function renderCardEditor(
      card: CardSettings,
      issues: ValidationIssue[] = [],
      cardError: CardError | null = null,
      catalog: PluginCatalog | null = null,
      ownershipTier: "local" | "networked" = "networked",
    ) {
      return renderToStaticMarkup(
        <CardEditor
          card={card}
          config={cardListConfig([card])}
          issues={issues}
          entryIssues={[]}
          cardError={cardError}
          pomodoro={null}
          provider={null}
          timerBusy={false}
          filePickerBusy={false}
          providerRefreshing={false}
          catalog={catalog}
          ownershipTier={ownershipTier}
          onChange={() => {}}
          onConfigChange={() => {}}
          onRemove={() => {}}
          onTimerAction={() => {}}
          onChooseCalendarFile={() => {}}
          onRefreshProvider={() => {}}
        />,
      );
    }
  ```

  Now the failing test, in the same describe block:

  ```tsx
  test("the editor names the plugin, owns its issue, and says when it cannot change it", () => {
    const plugin = pluginCard();
    const catalog: PluginCatalog = {
      plugins: [
        {
          id: "com.example.air-quality",
          name: "aqi",
          version: "1.0.0",
          node_count: 7,
          assets: [],
          display_name: "Air quality",
          description: "EPA index for a location",
          manifest_version: 2,
          template: "display-list",
          refresh_minutes: 15,
        },
      ],
      load_failures: [],
    };

    const editable = renderCardEditor(plugin, [], null, catalog);
    expect(editable).toContain('id="editor-heading">Air quality<');
    expect(editable).toContain("<span>Plugin</span>");
    expect(editable).toContain("Air quality · 1.0.0");
    expect(editable).toContain("com.example.air-quality");
    expect(editable).toContain("<span>Name</span>");
    expect(editable).not.toMatch(/<select[^>]*disabled/);

    // A validation issue on the id now attaches to the control that can fix it.
    const flagged = renderCardEditor(
      plugin,
      [
        {
          path: "cards[0].plugin_id",
          code: "missing-reference",
          message: "That plugin is not installed on the server.",
        },
      ],
      null,
      catalog,
    );
    expect(flagged).toContain('aria-invalid="true"');
    expect(flagged).toContain("That plugin is not installed on the server.");

    // An id the catalog lacks stays selected, and says so rather than resetting.
    const unknown = renderCardEditor(
      { ...plugin, plugin_id: "com.example.gone" },
      [],
      null,
      catalog,
    );
    expect(unknown).toContain("Not installed on the server");

    // No catalog: read-only, with the reason the tier makes true.
    const localHtml = renderCardEditor(plugin, [], null, null, "local");
    expect(localHtml).toMatch(/<select[^>]*disabled/);
    expect(localHtml).toContain("Needs the server to render");
    expect(renderCardEditor(plugin, [], null, null, "networked")).toContain(
      "The plugin list comes from the server",
    );
  });
  ```

  Run it: `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun test tests/components.test.tsx -t "names the plugin, owns its issue"` → fails to compile with `error: Property 'catalog' does not exist on type 'IntrinsicAttributes & CardEditorProps'`.

  Implement in `src/components/CardEditor.tsx`. Add `DeviceTier` and `PluginCatalog` to the type import (lines 13-24), then the two props in `CardEditorProps`, directly after `provider` (line 46):

  ```tsx
    /// The server's plugin registry, or null in local tier and before the first read.
    /// The Plugin field is read-only without it, because there is nothing to choose from.
    catalog: PluginCatalog | null;
    ownershipTier: DeviceTier | null;
  ```

  Add them to the destructure (lines 106-123), after `provider,`:

  ```tsx
    catalog,
    ownershipTier,
  ```

  Add the resolved entry and the refresh-owner flag beside `trouble` (line 141):

  ```diff
     const fieldIssues = (field: string) => issuesForField(issues, field);
     const trouble = providerTrouble(provider);
  +  const catalogEntry =
  +    card.kind === "plugin"
  +      ? (catalog?.plugins.find((entry) => entry.id === card.plugin_id) ?? null)
  +      : null;
     const setAlert = (alert: CardAlert) => onChange({ ...card, alert });
  ```

  Thread the catalog through both places that name the card (lines 148 and 157-158):

  ```diff
     const title = cardTitle(card);
  -  const controlName = title ? `${cardLabel(card)} — ${title}` : cardLabel(card);
  +  const controlName = title ? `${cardLabel(card, catalog)} — ${title}` : cardLabel(card, catalog);
  ```

  ```diff
           <div className="editor-title">
  -          <h2 id="editor-heading">{cardLabel(card)}</h2>
  +          <h2 id="editor-heading">{cardLabel(card, catalog)}</h2>
             {cardTitle(card) && <span className="editor-title__kind">{cardTitle(card)}</span>}
           </div>
  ```

  Add the Plugin field immediately above the shared "Refresh every" block (before line 500's `{(card.kind === "calendar" ||`):

  ```tsx
          {card.kind === "plugin" && (
            <label className="field">
              <span>Plugin</span>
              <select
                value={card.plugin_id}
                disabled={catalog === null}
                onChange={(event) => onChange({ ...card, plugin_id: event.currentTarget.value })}
                aria-invalid={fieldIssues("plugin_id").length > 0}
              >
                {/* A saved id the registry no longer carries stays selected and says
                    why. Dropping it would silently rewrite the document on first
                    render, which is a data loss nobody asked for. */}
                {!catalogEntry && (
                  <option
                    value={card.plugin_id}
                  >{`${card.plugin_id} · Not installed on the server`}</option>
                )}
                {catalog?.plugins.map((entry) => (
                  <option key={entry.id} value={entry.id}>
                    {`${entry.display_name ?? entry.id} · ${entry.version}`}
                  </option>
                ))}
              </select>
              {/* The machine id, quietly — it is the only identity a card has when the
                  catalog is unreachable, so it is never the thing that disappears. */}
              <small>
                {catalog === null
                  ? `${card.plugin_id} · ${
                      ownershipTier === "local"
                        ? "Needs the server to render"
                        : "The plugin list comes from the server"
                    }`
                  : card.plugin_id}
              </small>
              <FieldIssues issues={fieldIssues("plugin_id")} />
            </label>
          )}

  ```

  Keep `App.tsx` compiling — add the two props to its `<CardEditor>` (after line 559's `providerRefreshing=…`), with a placeholder Step 13 replaces:

  ```diff
             providerRefreshing={refreshingProviderId === selectedCardId}
  +          catalog={null}
  +          ownershipTier={ownershipTier}
             onChange={handleWidgetChange}
  ```

  Run to pass: `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun test tests/components.test.tsx && /Users/rodion/.bun/bin/bun run check`.

  Commit:

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -am \
    "feat: give a plugin card's id an editor control that can fix it" -m \
  "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
  ```

- [ ] **Step 11: Say where a plugin card refreshes, and stop offering a Refresh the Mac cannot perform.**

  Write the failing test in `tests/components.test.tsx`:

  ```tsx
  test("a plugin card states the manifest's cadence and refuses a refresh it cannot do", () => {
    const plugin = pluginCard();
    const catalog: PluginCatalog = {
      plugins: [
        {
          id: "com.example.air-quality",
          name: "aqi",
          version: "1.0.0",
          node_count: 7,
          assets: [],
          display_name: "Air quality",
          description: null,
          manifest_version: 2,
          template: "display-list",
          refresh_minutes: 15,
        },
      ],
      load_failures: [],
    };
    const html = renderToStaticMarkup(
      <CardEditor
        card={plugin}
        config={cardListConfig([plugin])}
        issues={[]}
        entryIssues={[]}
        cardError={null}
        pomodoro={null}
        provider={{
          widget_id: plugin.id,
          state: { kind: "stale", message: "Feed timed out after 10s" },
          last_success_unix_ms: 1,
          age_seconds: 5400,
        }}
        timerBusy={false}
        filePickerBusy={false}
        providerRefreshing={false}
        catalog={catalog}
        ownershipTier="networked"
        onChange={() => {}}
        onConfigChange={() => {}}
        onRemove={() => {}}
        onTimerAction={() => {}}
        onChooseCalendarFile={() => {}}
        onRefreshProvider={() => {}}
      />,
    );
    expect(html).toContain("Feed timed out after 10s");
    expect(html).toContain("This card refreshes on the server.");
    expect(html).toMatch(/<button[^>]*disabled[^>]*>.*?Refresh/s);
    expect(html).toContain("The server fetches this plugin every 15 minutes.");
  });
  ```

  Run it: `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun test tests/components.test.tsx -t "states the manifest's cadence"` → fails with `expect(received).toContain(expected)  … "This card refreshes on the server."`.

  Implement in `src/components/CardEditor.tsx`. Add the flag beside `catalogEntry`:

  ```diff
     const catalogEntry =
       card.kind === "plugin"
         ? (catalog?.plugins.find((entry) => entry.id === card.plugin_id) ?? null)
         : null;
  +  // The server owns a plugin card's fetching, so the Mac has nothing to refresh.
  +  // The reason rides the same sentence as the trouble, because a disabled control
  +  // with no stated reason is worse than no control.
  +  const refreshesOnServer = card.kind === "plugin";
     const setAlert = (alert: CardAlert) => onChange({ ...card, alert });
  ```

  Replace the trouble note (lines 181-194):

  ```tsx
        {trouble && (
          <p className="data-note" role="status">
            <span>
              {refreshesOnServer ? `${trouble} This card refreshes on the server.` : trouble}
            </span>
            <button
              className="text-button"
              type="button"
              disabled={providerRefreshing || refreshesOnServer}
              onClick={onRefreshProvider}
            >
              <Icon name="refresh" />
              {providerRefreshing ? "Refreshing…" : "Refresh"}
            </button>
          </p>
        )}
  ```

  Add the cadence hint inside the shared "Refresh every" field, directly above its `FieldIssues` (line 526):

  ```diff
               <option value="60">1 hour</option>
             </select>
  +          {catalogEntry && (
  +            <small>
  +              {`The server fetches this plugin every ${catalogEntry.refresh_minutes} minutes.`}
  +            </small>
  +          )}
             <FieldIssues issues={fieldIssues("refresh")} />
  ```

  Run to pass: `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun test tests/components.test.tsx && /Users/rodion/.bun/bin/bun run check`.

  Commit:

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -am \
    "feat: name where a plugin card refreshes instead of offering a dead control" -m \
  "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
  ```

- [ ] **Step 12: Name a plugin card the same way in the ring legend.**

  First give the suite's six existing `<LoopRing>` renders the new prop:

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && \
    sed -i '' 's|^\( *\)<LoopRing$|\1<LoopRing\
\1  catalog={null}|' tests/components.test.tsx && \
    grep -c "catalog={null}" tests/components.test.tsx
  ```

  That must print `6`. Then the failing test:

  ```tsx
  test("the ring legend calls a plugin card what every other surface calls it", () => {
    const plugin = pluginCard();
    const html = renderToStaticMarkup(
      <LoopRing
        config={cardListConfig([plugin])}
        issues={[]}
        catalog={{
          plugins: [
            {
              id: "com.example.air-quality",
              name: "aqi",
              version: "1.0.0",
              node_count: 7,
              assets: [],
              display_name: "Air quality",
              description: null,
              manifest_version: 2,
              template: "display-list",
              refresh_minutes: 15,
            },
          ],
          load_failures: [],
        }}
        selectedCardId={plugin.id}
        onSelect={() => {}}
        onChange={() => {}}
      />,
    );
    expect(html).toContain('class="loop__entry-name">Air quality<');
    expect(html).toContain('class="loop__entry-title">Office air<');
  });
  ```

  Run it: `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun test tests/components.test.tsx -t "ring legend calls a plugin card"` → fails with `expect(received).toContain(expected)  … 'class="loop__entry-name">Air quality<'`.

  Implement in `src/components/LoopRing.tsx`. Add `PluginCatalog` to the type import on line 13, replace `LoopRingProps` (lines 16-22) and the destructure and memo (lines 66 and 73):

  ```tsx
  interface LoopRingProps {
    config: AppConfig;
    issues: ValidationIssue[];
    /** The server's plugin registry, so a plugin arc is named the same here as elsewhere. */
    catalog: PluginCatalog | null;
    selectedCardId: string | null;
    onSelect: (cardId: string) => void;
    onChange: (config: AppConfig) => void;
  }
  ```

  ```diff
  -export function LoopRing({ config, issues, selectedCardId, onSelect, onChange }: LoopRingProps) {
  +export function LoopRing({
  +  config,
  +  issues,
  +  catalog,
  +  selectedCardId,
  +  onSelect,
  +  onChange,
  +}: LoopRingProps) {
  ```

  ```diff
  -  const segments = useMemo(() => filmstripSegments(config), [config]);
  +  const segments = useMemo(() => filmstripSegments(config, catalog), [config, catalog]);
  ```

  Keep `App.tsx` compiling — add the prop to its `<LoopRing>` (line 383-389), with a placeholder Step 13 replaces:

  ```diff
             <LoopRing
               config={draft}
               issues={issues}
  +            catalog={null}
               selectedCardId={selectedCardId}
  ```

  Run to pass: `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun test tests/components.test.tsx && /Users/rodion/.bun/bin/bun run check`.

  Commit:

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -am \
    "feat: name plugin cards in the loop ring legend" -m \
  "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
  ```

- [ ] **Step 13: Wire the window: one catalog, one card-state list, one add path.**

  Write the failing test in `tests/components.test.tsx`, mounting the real `App` so every prop is checked end to end:

  ```tsx
  test("the window names a plugin card from the catalog everywhere at once", async () => {
    snapshotImpl = async () => ({
      ...snapshot,
      config: {
        ...snapshot.config,
        cards: [pluginCard("air")],
        playlists: [
          {
            id: "workday",
            name: "Workday",
            advance: { kind: "timed" as const, default_dwell_seconds: 20 },
            entries: [{ card_id: "air", dwell_seconds: null }],
          },
        ],
        active_playlist_id: "workday",
      },
      device: { ...snapshot.device, tier: "networked" },
      providers: [],
      pomodoros: [],
      card_data: [],
      card_errors: [],
    });
    networkSettingsImpl = async () => ({
      server_url: "https://desk.example",
      device_id: "desk-1",
      tier: "networked",
    });
    serverPluginsImpl = async () => ({
      plugins: [
        {
          id: "com.example.air-quality",
          name: "aqi",
          version: "1.0.0",
          node_count: 7,
          assets: [],
          display_name: "Air quality",
          description: "EPA index for a location",
          manifest_version: 2,
          template: "display-list" as const,
          refresh_minutes: 15,
        },
      ],
      load_failures: [],
    });
    serverCardStateImpl = async () => [
      { card_id: "air", provider: { kind: "fresh" as const }, hero: "42", errors: [] },
    ];
    previewImpl = async () => ({ png_base64: null, sample: false, state: null });

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    try {
      await renderPreviewInto(root, <App />);
      await waitFor(() => {
        expect(container.querySelector(".card-tile__value")?.textContent).toBe("42");
      });
      // Tile, legend and editor heading all say the same thing.
      expect(container.querySelector(".tile-label")?.textContent).toBe("Air quality");
      expect(container.querySelector(".loop__entry-name")?.textContent).toBe("Air quality");
      expect(container.querySelector("#editor-heading")?.textContent).toBe("Air quality");
      // And the menu offers it back.
      const slot = container.querySelector<HTMLButtonElement>(".card-tile__add");
      await act(async () => slot?.click());
      expect(container.textContent).toContain("Plugins on the server");
      const row = [...container.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(
        (button) => button.textContent?.includes("Air quality"),
      );
      await act(async () => row?.click());
      await waitFor(() => {
        expect(container.querySelectorAll(".card-tile__body")).toHaveLength(2);
      });
    } finally {
      await act(async () => root.unmount());
      container.remove();
      serverPluginsImpl = async () => ({ plugins: [], load_failures: [] });
      serverCardStateImpl = async () => [];
    }
  });
  ```

  Run it: `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun test tests/components.test.tsx -t "names a plugin card from the catalog everywhere"` → fails at `expect(container.querySelector(".card-tile__value")?.textContent).toBe("42")`, receiving `"—"`, because `App` still passes `catalog={null} serverCardState={[]} pluginKinds={[]}`.

  Implement in `src/App.tsx`. Widen the imports: add `type AddCardRequest` to the `./lib/configDraft` import (lines 12-24), add `type PluginKindOption` to the `./components/CardList` import (line 4), and drop the now-unused `AddableCardKind` from the type import block on line 38 or `bun run lint` fails.

  Replace `handleAdd` (lines 247-254):

  ```tsx
    const handleAdd = (request: AddCardRequest) => {
      const result = addCard(draft, request);
      if (!result.cardId) {
        return;
      }
      replaceDraft(result.config);
      setSelectedCardId(result.cardId);
    };
  ```

  Add the menu rows beside it, after `handleAdd`:

  ```tsx
    // The add menu's server group, built from the catalog and nothing else. It is
    // empty in local tier and before the first read, which is why the group only
    // renders when it has rows.
    const pluginKinds: PluginKindOption[] = (pluginCatalog?.plugins ?? []).map((entry) => ({
      id: entry.id,
      version: entry.version,
      displayName: entry.display_name,
      description: entry.description,
      onAdd: () =>
        handleAdd({ kind: "plugin", pluginId: entry.id, refreshMinutes: entry.refresh_minutes }),
    }));
  ```

  Then replace the four placeholder props added in Steps 8, 10 and 12:

  ```diff
             <LoopRing
               config={draft}
               issues={issues}
  -            catalog={null}
  +            catalog={pluginCatalog}
               selectedCardId={selectedCardId}
  ```

  ```diff
  -            pluginKinds={[]}
  -            catalog={null}
  -            serverCardState={[]}
  +            pluginKinds={pluginKinds}
  +            catalog={pluginCatalog}
  +            serverCardState={serverCardState}
               ownershipTier={ownershipTier}
  ```

  ```diff
               providerRefreshing={refreshingProviderId === selectedCardId}
  -            catalog={null}
  +            catalog={pluginCatalog}
               ownershipTier={ownershipTier}
  ```

  And thread the catalog into the one notice that already printed `cardLabel`, so the last surface agrees with the rest (line 472):

  ```diff
                       <p key={cardError.card_id}>
  -                      <strong>{card ? cardLabel(card) : cardError.card_id}</strong> —{" "}
  +                      <strong>{card ? cardLabel(card, pluginCatalog) : cardError.card_id}</strong>{" "}
  +                      —{" "}
                         {cardError.message}
                       </p>
  ```

  Run to pass: `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun test tests/components.test.tsx && /Users/rodion/.bun/bin/bun run check`.

  Commit:

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -am \
    "feat: wire the plugin catalog and server card state through the window" -m \
  "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
  ```

- [ ] **Step 14: Print a state word on the stage instead of a fault.**

  Write the failing test in `tests/components.test.tsx`, in the `DevicePreview` block beside `mountPreview`:

  ```tsx
  test("a frameless preview prints its state word, never the fault message", async () => {
    previewImpl = async () => ({
      png_base64: null,
      sample: false,
      state: "Waiting for the first refresh",
    });
    const { container, root } = await mountPreview(pluginCard("air"));
    await waitFor(() => {
      expect(container.textContent).toContain("Waiting for the first refresh");
    });
    expect(container.textContent).not.toContain("Preview unavailable");
    expect(container.querySelector("img")).toBeNull();
    // The badge means "a real frame from sample data". There is no frame here.
    expect(container.querySelector(".stage__badge")).toBeNull();
    await act(async () => root.unmount());
  });
  ```

  This is arbitration item 9 on the stage: a waiting plugin card has no frame at all, so it gets the state **word**, and the "No data yet" badge stays reserved for a rendered sample frame. That supersedes spec §5.2's earlier "maps `waiting` to `sample: true`" sentence and makes §8's "Waiting" row read "state word".

  Run it: `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun test tests/components.test.tsx -t "prints its state word"` → fails with `expect(received).toContain(expected)  … "Waiting for the first refresh"`.

  Implement in `src/components/DevicePreview.tsx` — carry `state` on the frame and give it the same type as the fault line by reusing that class, so the two read as one family without inventing a rule. Three exact-context edits:

  ```diff
     const widget = cards.find((card) => card.id === selectedWidgetId) ?? cards[0];
  -  const [frame, setFrame] = useState<{ pngBase64: string | null; sample: boolean } | null>(null);
  +  const [frame, setFrame] = useState<{
  +    pngBase64: string | null;
  +    sample: boolean;
  +    state: string | null;
  +  } | null>(null);
     const [unavailable, setUnavailable] = useState(false);
  ```

  ```diff
           if (!cancelled && requested === generation.current) {
  -          setFrame({ pngBase64: rendered.png_base64, sample: rendered.sample });
  +          setFrame({
  +            pngBase64: rendered.png_base64,
  +            sample: rendered.sample,
  +            state: rendered.state,
  +          });
             setUnavailable(false);
           }
  ```

  ```diff
           ) : frame?.pngBase64 ? (
             <img
               alt={`Device preview of ${widget.id}`}
               width={448}
               height={368}
               src={`data:image/png;base64,${frame.pngBase64}`}
             />
  +        ) : frame?.state ? (
  +          /* A state, not a fault: "Preview unavailable" stays reserved for a
  +             transport failure, which is the one case the reader can do nothing
  +             about. */
  +          <p className="stage__unavailable">{frame.state}</p>
           ) : null}
  ```

  `LIVE_TEMPLATES` stays `{clock, pomodoro}`: a plugin preview is rendered by the server behind a 30 s raster floor, so a 1 Hz re-request would be a poll the server refuses to answer differently.

  Run to pass: `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun test tests/components.test.tsx && /Users/rodion/.bun/bin/bun run check`.

  Commit:

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -am \
    "feat: print a preview state word on the stage" -m \
  "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
  ```

- [ ] **Step 15: Make every plugin state reachable in the browser harness.**

  Create `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate/src/dev/pluginFixture.ts`:

  ```ts
  /**
   * Dev-only plugin fixtures. Never ships: only `VITE_DESKMATE_MOCK=1` aliases the
   * Tauri bridge at `src/dev/`, so a production build resolves the real bridge and
   * tree-shakes this away.
   *
   * The catalog mirrors the four curated plugins in `companion/plugins/`, with the
   * display names, descriptions and cadences their v2 manifests declare and the real
   * byte lengths of their committed assets. `node_count` is the registry's own sum
   * (`[[nodes]]` plus every `[[repeats]] nodes`), and digests are plainly fake
   * constants: nothing in the window renders a digest, and inventing a plausible
   * SHA-256 would be a lie a reader might trust.
   */
  import type {
    AppConfig,
    CardDataSnapshot,
    CardSettings,
    PluginCatalog,
    ProviderSnapshot,
    ServerCardState,
  } from "../lib/types";
  import { mockConfig } from "./fixture";

  export const MOCK_PLUGIN_CATALOG: PluginCatalog = {
    plugins: [
      {
        id: "aqi",
        name: "aqi",
        version: "1.0.0",
        node_count: 7,
        assets: [
          { file: "icons.ttf", kind: "icon-font", byte_length: 4320, digest: "dev-digest-aqi" },
        ],
        display_name: "Air quality",
        description: "EPA index for a location",
        manifest_version: 2,
        template: "display-list",
        refresh_minutes: 15,
      },
      {
        id: "agenda",
        name: "agenda",
        version: "1.0.0",
        node_count: 7,
        assets: [
          { file: "badge.rgb565", kind: "image", byte_length: 812, digest: "dev-digest-agenda" },
        ],
        display_name: "Agenda",
        description: "Your next few events",
        manifest_version: 2,
        template: "display-list",
        refresh_minutes: 10,
      },
      {
        id: "claude-limits",
        name: "claude-limits",
        version: "1.0.0",
        node_count: 12,
        assets: [],
        display_name: "Claude usage",
        description: "Session and weekly subscription limits",
        manifest_version: 2,
        template: "display-list",
        refresh_minutes: 10,
      },
      {
        id: "svg-aqi",
        name: "svg-aqi",
        version: "1.0.0",
        node_count: 0,
        assets: [],
        display_name: "Air quality, drawn",
        description: "The same index, drawn as SVG",
        manifest_version: 2,
        template: "svg",
        refresh_minutes: 15,
      },
    ],
    load_failures: [],
  };

  function pluginCard(id: string, title: string, pluginId: string, minutes: number): CardSettings {
    return {
      kind: "plugin",
      id,
      title,
      plugin_id: pluginId,
      tap_action: { kind: "none" },
      refresh: { kind: "interval", minutes },
      alert: { kind: "none" },
    };
  }

  /**
   * Two plugin cards in the loop — one display-list, one SVG — and two outside it, so
   * one scenario reaches every outcome the preview route can produce: a frame, a
   * stale frame, "waiting for the first refresh", and a plugin the registry lost.
   */
  export function mockPluginConfig(): AppConfig {
    return {
      ...mockConfig(),
      cards: [
        pluginCard("air", "Office air", "aqi", 15),
        pluginCard("air-svg", "", "svg-aqi", 15),
        pluginCard("usage", "", "claude-limits", 10),
        pluginCard("retired", "Old panel", "com.example.retired", 30),
      ],
      playlists: [
        {
          id: "day",
          name: "Workday",
          advance: { kind: "timed", default_dwell_seconds: 20 },
          entries: [
            { card_id: "air", dwell_seconds: 30 },
            { card_id: "air-svg", dwell_seconds: null },
          ],
        },
      ],
      active_playlist_id: "day",
    };
  }

  export function mockPluginCardState(): ServerCardState[] {
    return [
      { card_id: "air", provider: { kind: "fresh" }, hero: "42", errors: [] },
      {
        card_id: "air-svg",
        provider: { kind: "stale", message: "Feed timed out after 10s" },
        hero: "51",
        errors: [],
      },
      { card_id: "usage", provider: { kind: "idle" }, hero: null, errors: [] },
      {
        card_id: "retired",
        provider: { kind: "error", message: "Plugin \u201ccom.example.retired\u201d is not loaded" },
        hero: null,
        errors: [],
      },
    ];
  }

  /** What the Mac's projection puts into `providers` for those same cards. */
  export function mockPluginProviders(): ProviderSnapshot[] {
    return mockPluginCardState().map((state) => ({
      widget_id: state.card_id,
      state: state.provider,
      last_success_unix_ms: state.hero === null ? null : 1,
      age_seconds: state.hero === null ? null : 300,
    }));
  }

  /** And into `card_data`, as the `hero` field the tile reads. */
  export function mockPluginCardData(): CardDataSnapshot[] {
    return mockPluginCardState()
      .filter((state) => state.hero !== null)
      .map((state) => ({
        card_id: state.card_id,
        fields: [{ key: "hero", value: { kind: "text" as const, value: state.hero ?? "" } }],
      }));
  }
  ```

  Extend `src/dev/mockBackend.ts`. Add the import beside the others (after line 15):

  ```ts
  import {
    MOCK_PLUGIN_CATALOG,
    mockPluginCardData,
    mockPluginCardState,
    mockPluginConfig,
    mockPluginProviders,
  } from "./pluginFixture";
  ```

  Add the two scenarios to `SCENARIOS` (lines 24-34), after `"carderror",`:

  ```ts
    "plugin",
    "plugin-local",
  ```

  Add the arm to `applyScenario`, before `default:` (line 125):

  ```ts
      case "plugin":
      case "plugin-local":
        config = mockPluginConfig();
        snapshot = mockSnapshot(config);
        snapshot.providers = mockPluginProviders();
        snapshot.card_data = mockPluginCardData();
        snapshot.pomodoros = [];
        if (scenario === "plugin-local") {
          // No server, so no catalog and no rendering: `pluginCardFlag` prints the
          // word and the stage says where these cards are drawn.
          snapshot.device.tier = "local";
          network = { server_url: "", device_id: "", tier: "local" };
        }
        break;
  ```

  Add a hero lookup beside `delay` (line 246):

  ```ts
  /** The `hero` field the mock projection published for one card, if any. */
  function heroFor(cardId: string): string | null {
    const field = snapshot.card_data
      .find((candidate) => candidate.card_id === cardId)
      ?.fields.find((candidate) => candidate.key === "hero");
    if (!field) {
      return null;
    }
    return field.value.kind === "text" ? field.value.value : String(field.value.value);
  }
  ```

  Add the two command arms to `mockInvoke`, after the `get_autostart_status` case (line 256):

  ```ts
      case "get_server_plugins":
        return delay(MOCK_PLUGIN_CATALOG) as Promise<T>;
      case "get_server_card_state":
        return delay(scenario === "plugin" ? mockPluginCardState() : []) as Promise<T>;
  ```

  And replace the `render_card_preview` arm (lines 311-331) — today's blanket plugin refusal becomes the four outcomes:

  ```ts
      case "render_card_preview": {
        const cardId = args?.cardId as string;
        const card = config.cards.find((candidate) => candidate.id === cardId);
        if (!card) throw { category: "not-found", message: "No such card." };
        if (card.kind === "plugin") {
          if (scenario === "plugin-local") {
            return {
              png_base64: null,
              sample: false,
              state: "Plugin cards render on the server",
            } as T;
          }
          if (!MOCK_PLUGIN_CATALOG.plugins.some((plugin) => plugin.id === card.plugin_id)) {
            return {
              png_base64: null,
              sample: false,
              state: `Plugin \u201c${card.plugin_id}\u201d is not loaded on the server`,
            } as T;
          }
          if (heroFor(cardId) === null) {
            return {
              png_base64: null,
              sample: false,
              state: "Waiting for the first refresh",
            } as T;
          }
        }
        const timer = snapshot.pomodoros.find((candidate) => candidate.widget_id === cardId);
        return {
          png_base64: renderMockFrame(
            card,
            snapshot.card_data.find((candidate) => candidate.card_id === cardId),
            config.preferences.timezone,
            timer?.remaining_seconds ?? null,
          ),
          sample: true,
          state: null,
        } as T;
      }
  ```

  Finally give a plugin card a real fixture frame instead of a black rectangle — replace `case "plugin": break;` in `src/dev/mockPreview.ts` (lines 203-204):

  ```ts
      case "plugin":
        heroCaption(
          ctx,
          card.title || card.plugin_id,
          fieldValue(data, "hero") ?? "--",
          "Rendered on the server",
        );
        break;
  ```

  Verify by eye rather than by assertion — the harness is a reviewing surface, and a test that re-states its fixture proves nothing:

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && \
    VITE_DESKMATE_MOCK=1 /Users/rodion/.bun/bin/bun run dev
  ```

  Open `?scenario=plugin` (four named tiles reading `42` / `51` / `—` / `—`; a `stale` flag on the SVG card; `not in loop` + `not on the server` on the retired one; the add menu listing four plugins) and `?scenario=plugin-local` (four `needs the server` flags, a read-only Plugin select, "Plugin cards render on the server" on the stage). Stop the dev server, then run the gates:

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && \
    /Users/rodion/.bun/bin/bun run check && /Users/rodion/.bun/bin/bun run lint
  ```

  Commit:

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity add \
    companion/apps/deskmate/src/dev/pluginFixture.ts && \
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -am \
    "feat: reach every plugin card state in the dev harness" -m \
  "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
  ```

- [ ] **Step 16: Prove the whole surface, then hand it off.**

  Run every frontend gate:

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && \
    /Users/rodion/.bun/bin/bun test && \
    /Users/rodion/.bun/bin/bun run check && \
    /Users/rodion/.bun/bin/bun run lint && \
    /Users/rodion/.bun/bin/bun run format:check
  ```

  Then mutation-probe the four assertions this task's value rests on, exactly as the one-loop change did — a green suite is not evidence until a deleted line makes it red. Delete each, confirm the named test fails, restore it:

  1. `src/lib/configDraft.ts`, `pluginCardFlag`'s `if (tier === "local") { return "needs the server"; }` → "a bare plugin id always carries the word that explains it" fails.
  2. `src/lib/configDraft.ts`, `cardLabel`'s `return entry?.display_name ?? card.plugin_id;` → `return card.plugin_id;` → "a plugin card is named by its display name, and falls back to its id twice over" fails.
  3. `src/lib/useAppState.ts`, `startServerCardStatePoll`'s `if (!visible()) { return; }` → "polls on start, on focus and on each tick, but never while hidden" fails.
  4. `src/components/DevicePreview.tsx`, the `) : frame?.state ? (` branch → "a frameless preview prints its state word, never the fault message" fails.

  If any probe stays green, the test is asserting its own fixture rather than the behaviour; fix the test before continuing.

  Commit only if a probe forced a test change:

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -am \
    "test: pin the plugin-card behaviour a mutation probe found unguarded" -m \
  "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
  ```

**Task exit:** `bun test`, `bun run check`, `bun run lint` and `bun run format:check` all green from `/Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate`; `src/lib/types.contract.ts` unchanged by this task (Task 4's Rust fixture owns it — confirm with `git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity diff --stat HEAD~16 -- companion/apps/deskmate/src/lib/types.contract.ts` printing nothing), which is exactly why Step 1 had to widen `IpcContractFixtures` instead; the four mutation probes above each turn a named test red; `?scenario=plugin` and `?scenario=plugin-local` between them render every row of spec §8's table.

---

### Task 6: Docs, rollout order, and the deploy

**Files:**

- Create: `companion/crates/plugin/tests/manifest_v2_doc.rs`
- Read only, **do not modify**: `docs/plugins/manifest-v2.md` (Task 1 owns every word of it — Step 10 of Task 1 rewrites the top-level shape example, the "V2 adds only …" sentence, the new `## Presentation keys` section, and the `## V1 compatibility` section at 138-149). This task cross-checks that Task 1's amendment landed and that the two fixture paths it recorded under `## V1 compatibility` survive; it never edits the file.
- Modify: `PRODUCT.md` (read 44-145; exact blocks replaced at 52-56, 82-86, 99-101, 112-114, 138)
- Modify: `DESIGN.md` (read 145-185 and 235-263; exact blocks replaced at 153-155, 167, 173, 241-244)
- Modify: `CLAUDE.md` (read 140-190; replace the stale `pluginKinds` clause at 161-163 and insert one sibling bullet after 179, before `- **V2's exit gate is OPEN` at 180)
- Modify: `companion/crates/server/deploy/README.md` (read 1-60 and the tail; 216 lines today — insert four lines into §2 after line 41, append a `## 6.` section at the end)
- Modify: `docs/superpowers/plans/2026-09-07-deskmate-plugin-card-parity.md` (125 lines today; append the rollout record at the end)

**Interfaces:**

*Consumes*
- `plugin::MAX_DISPLAY_NAME_LEN: usize` (= 64) and `plugin::MAX_DESCRIPTION_LEN: usize` (= 160), re-exported from `manifest.rs` — Task 1
- `plugin::MAX_SUMMARY_LEN: usize` (= 32), re-exported from the new `summary.rs` — Task 1
- `plugin::parse_manifest(source: &str) -> Result<PluginManifest, ManifestError>` (`companion/crates/plugin/src/manifest.rs:863`; `ManifestError` derives `Debug` at `manifest.rs:131`)
- `PluginManifest::is_manifest_v1(&self) -> bool` (`manifest.rs:640`)
- `PluginManifest::{display_name, description, summary}: Option<String>` — Task 1
- Task 1's committed v1 fixtures `companion/crates/plugin/tests/fixtures/manifest_v1_aqi.toml` and `manifest_v1_agenda.toml`, and the four curated `companion/plugins/{agenda,aqi,claude-limits,svg-aqi}/manifest.toml` moved to `manifest_version = 2`
- `GET /v1/plugins -> app_core::admin::PluginCatalog` with `display_name` / `description` / `manifest_version` / `template` / `refresh_minutes` — Task 3
- The `?scenario=plugin` dev harness and `src/dev/pluginFixture.ts` — Task 5

*Produces*
- No new code interface. One doc-contract cross-check test (`manifest_v2_doc.rs`), the amended positioning/design/state prose, the written redeploy recipe, and the rollout record.

---

- [ ] **Step 1: Write the doc-contract cross-check test.** It pins `docs/plugins/manifest-v2.md` — which Task 1 owns and this task never edits — to this crate's own constants, to the four curated manifests on disk, and to the two v1 fixture paths Task 1 recorded, so a number stated once in prose and once in a `const` cannot disagree.

  Create `companion/crates/plugin/tests/manifest_v2_doc.rs`:

  ```rust
  //! Task 6: the frozen manifest-v2 contract document, this crate's bounds, and the
  //! manifests on disk cannot drift apart.
  //!
  //! `docs/plugins/manifest-v2.md` is a frozen contract other people read before they
  //! write a manifest, and Task 1 owns every word of it. This test owns none of that
  //! prose and changes none of it: it pins only the numbers and paths that also live
  //! in code. A bound stated once in prose and once in a `const` will eventually
  //! disagree, and the disagreement is invisible until someone's manifest is rejected
  //! for a reason the document says is legal.

  use std::path::{Path, PathBuf};

  /// The curated plugins, all four of which Task 1 moved to `manifest_version = 2`.
  const CURATED_IDS: [&str; 4] = ["agenda", "aqi", "claude-limits", "svg-aqi"];

  /// The byte-exact v1 copies Task 1 froze before the curated manifests moved to v2.
  /// They are all the coverage manifest v1 has left, and `manifest-v2.md`'s
  /// `## V1 compatibility` section names both paths. If either the prose or the file
  /// goes away, v1 coverage has quietly gone with it.
  const V1_FIXTURES: [&str; 2] = [
      "companion/crates/plugin/tests/fixtures/manifest_v1_aqi.toml",
      "companion/crates/plugin/tests/fixtures/manifest_v1_agenda.toml",
  ];

  fn repo_root() -> PathBuf {
      // `CARGO_MANIFEST_DIR` is `companion/crates/plugin`.
      Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
  }

  fn read(path: &Path) -> String {
      std::fs::read_to_string(path)
          .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
  }

  fn contract_doc() -> String {
      read(&repo_root().join("docs/plugins/manifest-v2.md"))
  }

  /// True when `line` states `value` as a number in its own right, so that a row
  /// claiming `MAX_SUMMARY_LEN` is 32 is not satisfied by the "32" hiding inside
  /// some neighbouring figure.
  fn states_number(line: &str, value: usize) -> bool {
      let text = value.to_string();
      line.match_indices(&text).any(|(at, _)| {
          let before = line[..at].chars().next_back();
          let after = line[at + text.len()..].chars().next();
          !before.is_some_and(|c| c.is_ascii_digit()) && !after.is_some_and(|c| c.is_ascii_digit())
      })
  }

  #[test]
  fn the_contract_document_states_every_display_bound_with_its_real_value() {
      let doc = contract_doc();
      for (name, value) in [
          ("MAX_DISPLAY_NAME_LEN", plugin::MAX_DISPLAY_NAME_LEN),
          ("MAX_DESCRIPTION_LEN", plugin::MAX_DESCRIPTION_LEN),
          ("MAX_SUMMARY_LEN", plugin::MAX_SUMMARY_LEN),
      ] {
          let mentions: Vec<&str> = doc.lines().filter(|line| line.contains(name)).collect();
          assert!(
              !mentions.is_empty(),
              "docs/plugins/manifest-v2.md never names `{name}`"
          );
          assert!(
              mentions.iter().any(|line| states_number(line, value)),
              "docs/plugins/manifest-v2.md names `{name}` but never states its value \
               {value}; the lines mentioning it are {mentions:?}"
          );
      }
  }

  #[test]
  fn the_contract_document_names_both_presentation_key_errors() {
      let doc = contract_doc();
      for error in ["SummaryUsesDeviceBinding", "EmptyString"] {
          assert!(
              doc.contains(error),
              "docs/plugins/manifest-v2.md never names `ManifestError::{error}`, so an \
               author cannot look up why their manifest was rejected"
          );
      }
  }

  #[test]
  fn manifest_v2_doc_records_the_three_presentation_keys_the_curated_plugins_use() {
      let doc = contract_doc();
      for id in CURATED_IDS {
          let source = read(&repo_root().join("companion/plugins").join(id).join("manifest.toml"));
          let manifest = plugin::parse_manifest(&source)
              .unwrap_or_else(|error| panic!("{id}/manifest.toml parses: {error:?}"));
          assert!(!manifest.is_manifest_v1(), "{id} is not manifest_version = 2");
          assert!(manifest.display_name.is_some(), "{id} declares no display_name");
          assert!(manifest.description.is_some(), "{id} declares no description");
          assert!(manifest.summary.is_some(), "{id} declares no summary");
          assert!(
              doc.contains(&format!("`{id}`")),
              "manifest-v2.md never names `{id}`, so its move to v2 is unrecorded"
          );
      }
  }

  #[test]
  fn the_v1_fixture_paths_the_contract_names_exist_and_still_parse_as_v1() {
      let doc = contract_doc();
      for path in V1_FIXTURES {
          assert!(
              doc.contains(path),
              "manifest-v2.md no longer names `{path}`; v1's only remaining coverage is \
               then unfindable from the contract that points at it"
          );
          let manifest = plugin::parse_manifest(&read(&repo_root().join(path)))
              .unwrap_or_else(|error| panic!("{path} parses: {error:?}"));
          assert!(manifest.is_manifest_v1(), "{path} is no longer manifest v1");
          assert_eq!(manifest.display_name, None, "{path} gained a v2 key");
          assert_eq!(manifest.description, None, "{path} gained a v2 key");
          assert_eq!(manifest.summary, None, "{path} gained a v2 key");
      }
  }

  #[test]
  fn the_contract_no_longer_claims_the_shipped_manifests_are_v1() {
      let doc = contract_doc();
      for stale in ["byte-for-byte unchanged", "The two shipped v1 manifests"] {
          assert!(
              !doc.contains(stale),
              "manifest-v2.md still says {stale:?}; all four shipped manifests are v2 now"
          );
      }
  }
  ```

  Run it:

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p plugin --test manifest_v2_doc
  ```

  Expected: `test result: ok. 5 passed; 0 failed`. Green is the correct first outcome here and is not evidence on its own: this test drives no production change, it cross-checks a document Task 1 already wrote, and Step 2's mutation probe is what proves each assertion bites. A **red** run is a real finding, and the panic names which assertion Task 1 left unmet — fix it in Task 1's files (`docs/plugins/manifest-v2.md`, the curated manifests, the frozen fixtures), never by editing this test.

- [ ] **Step 2: Mutation probe — break the document twice and confirm the test catches each.** Both mutations are uncommitted and restored with `git checkout --`; the contract document is Task 1's and must end this step byte-identical to how Task 1 committed it.

  Probe A — misstate a bound in the prose:

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity && python3 - <<'PY'
  from pathlib import Path

  path = Path("docs/plugins/manifest-v2.md")
  text = path.read_text()
  needle = "1..=64 bytes (`MAX_DISPLAY_NAME_LEN`"
  assert needle in text, "Task 1's display_name bound row moved; re-read the document"
  path.write_text(text.replace(needle, "1..=48 bytes (`MAX_DISPLAY_NAME_LEN`", 1))
  PY
  ```

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p plugin --test manifest_v2_doc
  ```

  Expected: `the_contract_document_states_every_display_bound_with_its_real_value` fails with `docs/plugins/manifest-v2.md names \`MAX_DISPLAY_NAME_LEN\` but never states its value 64`. Restore:

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity checkout -- docs/plugins/manifest-v2.md
  ```

  Probe B — drop the fixture path Task 1 recorded, the loss Task 6 exists to make loud:

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity && python3 - <<'PY'
  from pathlib import Path

  path = Path("docs/plugins/manifest-v2.md")
  lines = path.read_text().splitlines(keepends=True)
  kept = [line for line in lines if "manifest_v1_aqi.toml" not in line]
  assert len(kept) == len(lines) - 1, "expected exactly one line naming the aqi v1 fixture"
  path.write_text("".join(kept))
  PY
  ```

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test -p plugin --test manifest_v2_doc
  ```

  Expected: `the_v1_fixture_paths_the_contract_names_exist_and_still_parse_as_v1` fails with `manifest-v2.md no longer names \`companion/crates/plugin/tests/fixtures/manifest_v1_aqi.toml\``. Restore and confirm the document is untouched:

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity checkout -- docs/plugins/manifest-v2.md && \
    git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity status --short docs/plugins/manifest-v2.md
  ```

  Expected: `git status --short` prints nothing. Then commit the test alone:

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity add companion/crates/plugin/tests/manifest_v2_doc.rs && git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -m "test: pin manifest-v2's display bounds and curated v2 coverage to the code" -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>" -m "Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
  ```

- [ ] **Step 3: Amend PRODUCT.md — the preview claim, objects, surfaces, naming, evidence.** Positioning item 1 currently promises exact pixels without qualification; the spec's §2 accepted a narrower claim for plugin cards, and PRODUCT.md is where that is recorded rather than glossed. Five exact block replacements.

  (a) Replace lines 52-56, exactly:

  ```markdown
  1. **The preview is not a mock.** `render_card_preview` sends host-built scenes through
     the firmware/LVGL renderer and returns exact-pixel PNG frames; the retired C
     templates survive only in `lvgl-sim` as the reference oracle, not as templates
     shipped on the device. A framebuffer diff harness proves the simulator and firmware
     agree pixel for pixel. What the app shows is what the panel will show.
  ```

  with:

  ```markdown
  1. **The preview is not a mock.** For the six built-in kinds, `render_card_preview` sends
     host-built scenes through the firmware/LVGL renderer and returns exact-pixel PNG
     frames; the retired C templates survive only in `lvgl-sim` as the reference oracle,
     not as templates shipped on the device. A framebuffer diff harness proves the
     simulator and firmware agree pixel for pixel. What the app shows is what the panel
     will show.

     **For a plugin card the claim is narrower, and it is stated rather than glossed
     (2026-09-07).** The Mac holds no plugin registry and no rasterizer, so a plugin
     card's preview is rendered on the server by `resvg` and returned as a PNG built from
     the same cached snapshot the panel's face is built from. For an SVG-template plugin
     that is exact by construction — the raster *is* what the panel shows. For a
     display-list plugin it can differ exactly where `docs/scene/template-parity-ledger.md`
     already records a difference: LVGL ellipsizes an overflowing line where the raster
     shows it whole. It is a render of the real card from the real data, never a drawing
     of a card that does not exist. Rendering the server's compiled scene in the Mac's own
     simulator, byte-exact, remains available later as an additive extension of the same
     route.
  ```

  (b) Replace lines 82-86, exactly:

  ```markdown
  **Objects.** The app edits six built-in card kinds (max 8): `clock`,
  `pomodoro`, `calendar`, `weather`, `json-feed`, and `rss`. Schema v6 also carries
  server-side `plugin` cards: the app shows them (labelled by plugin id, with a tile and a
  name/refresh editor) but cannot add one, and its hostless runtime refuses to render them
  with a typed reason; the server renders them.
  ```

  with:

  ```markdown
  **Objects.** The app edits six built-in card kinds (max 8): `clock`,
  `pomodoro`, `calendar`, `weather`, `json-feed`, and `rss`. Schema v6 also carries
  server-side `plugin` cards, and since 2026-09-07 a plugin card is a peer of a built-in
  one in `networked` tier: added from the same menu with the same gesture, named by its
  manifest's `display_name`, showing a live headline on its tile, carrying the same
  freshness and error states, previewing on the stage, and edited in an editor whose
  Plugin field owns `cards[i].plugin_id`. The server still renders it; the Mac reads the
  server's catalog, per-card state and preview over the admin bearer. In `local` tier
  there is no server to render it, so the card is flagged `needs the server` and the stage
  says so — and the hostless runtime no longer schedules a refresh it cannot perform, which
  is what used to show as a permanent `stale`.
  ```

  (c) Replace lines 99-101, exactly:

  ```markdown
  and error notices; the loop grid (complication tiles in loop order, reordered in place,
  with the add-card slot and its menu of built-in kinds and, when a registry is available,
  plugins); the per-card editor, which also edits the card's dwell; `DevicePreview`;
  ```

  with:

  ```markdown
  and error notices; the loop grid (complication tiles in loop order, reordered in place,
  with the add-card slot and its menu of built-in kinds and, in networked tier, the
  server's plugins with their descriptions); the per-card editor, which also edits the
  card's dwell and, for a plugin card, chooses its plugin; `DevicePreview`;
  ```

  (d) Replace lines 112-114, exactly:

  ```markdown
  - Clock faces carry no title chip and no eyebrow on the device. User-visible card
    identity is card-kind-first (`cardLabel()` returns `cardKindName()`); the owner's
    `title` is a quiet second line that distinguishes cards of the same kind.
  ```

  with:

  ```markdown
  - Clock faces carry no title chip and no eyebrow on the device. User-visible card
    identity is card-kind-first: `cardLabel(card, catalog)` returns the template name for a
    built-in card and the plugin's `display_name` for a plugin card — never the word
    "Plugin", and never a bare machine id without a word saying why; the owner's `title` is
    a quiet second line that distinguishes cards of the same kind.
  ```

  (e) Replace line 138, exactly:

  ```markdown
  - Real, working exact-pixel device previews via IPC (`render_card_preview`).
  ```

  with:

  ```markdown
  - Real, working exact-pixel device previews via IPC (`render_card_preview`) for the six
    built-in kinds, and server-rendered PNG previews for plugin cards.
  ```

  Verify:

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity && grep -c "when a registry is available" PRODUCT.md; grep -n 'narrower, and it is stated\|peer of a built-in\|cardLabel(card, catalog)' PRODUCT.md
  ```

  Expected: the `grep -c` prints `0` (and exits 1, which is the expected status for no match); the second `grep -n` prints exactly three lines.

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity add PRODUCT.md && git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -m "docs: state the plugin preview's narrower claim and plugin card parity in PRODUCT" -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>" -m "Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
  ```

- [ ] **Step 4: Amend DESIGN.md — the loop bullet, the menu bullet, the naming bullet, the harness states.** The naming rule is the one an implementer is most likely to break next, so it says what a plugin card is called and what is printed when the catalog cannot name it. Four exact block replacements.

  (a) Replace lines 153-155, exactly:

  ```markdown
    earlier / later buttons that appear on hover, or ⌥ ← → on a focused tile. Adding a card
    is one dashed slot at the end of the grid; the new card joins the loop at once. A tile
    owns one fact, so dwell is never printed on it: the ring draws the proportion and the
  ```

  with:

  ```markdown
    earlier / later buttons that appear on hover, or ⌥ ← → on a focused tile. Adding a card
    is one dashed slot at the end of the grid; the new card joins the loop at once, whether
    it is one of the six built-in kinds or one of the server's plugins (2026-09-07) — one
    `addCard` path, one capacity guard, one gesture. A tile
    owns one fact, so dwell is never printed on it: the ring draws the proportion and the
  ```

  (b) Replace line 167, exactly:

  ```markdown
    window. When that list is long enough to want search, it becomes a picker window.
  ```

  with:

  ```markdown
    window. Since 2026-09-07 that group is real: in networked tier it lists the server's
    plugins, each a display name over a description, from `GET /v1/plugins`. When the list
    is long enough to want search, it becomes a picker window.
  ```

  (c) Replace line 173, exactly:

  ```markdown
    the failure it prevents second.
  ```

  with:

  ```markdown
    the failure it prevents second. **A plugin card obeys the same rule** (2026-09-07): its
    name is the manifest's `display_name` — "Air quality", not `aqi`, and never the word
    "Plugin" — read from the server's catalog and threaded through the one helper,
    `cardLabel(card, catalog)`. When the catalog cannot name it, the surface prints the
    word saying why (`not on the server`, `needs the server`) beside the id, because a bare
    machine id is a name that teaches nothing and a silent fallback is a state that hides.
    The editor's Plugin `<select>` is where that id is changed; it is the home of a
    `cards[i].plugin_id` issue, which is otherwise an issue with no control.
  ```

  (d) Replace lines 241-244, exactly:

  ```markdown
  Every one of these has a designed treatment, and each is reachable in the dev harness
  (`VITE_DESKMATE_MOCK=1 bun run dev`, then `?scenario=…`): `default`, `offline`,
  `standalone`, `local`, `unowned`, `invalid`, `firstrun`, `empty`, `carderror`. Add
  `&theme=dark` or `&theme=light` to pin the scheme.
  ```

  with:

  ```markdown
  Every one of these has a designed treatment, and each is reachable in the dev harness
  (`VITE_DESKMATE_MOCK=1 bun run dev`, then `?scenario=…`): `default`, `offline`,
  `standalone`, `local`, `unowned`, `invalid`, `firstrun`, `empty`, `carderror`, `plugin`.
  Add `&theme=dark` or `&theme=light` to pin the scheme.
  ```

  Verify:

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity && grep -n 'A plugin card obeys the same rule\|that group is real\|`carderror`, `plugin`' DESIGN.md
  ```

  Expected: exactly three lines.

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity add DESIGN.md && git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -m "docs: extend the one-name rule to plugin cards and name the plugin harness state" -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>" -m "Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
  ```

- [ ] **Step 5a: Correct the stale `pluginKinds` clause in CLAUDE.md.** The one-loop bullet says the app passes `pluginKinds` as `[]` and instructs a future reader not to fabricate a registry; that is now wrong, and a stale instruction is worse than no instruction.

  Replace lines 161-163, exactly:

  ```markdown
    opens a menu** of built-in kinds plus a `pluginKinds` group the app passes as `[]`
    today (there is no plugin registry source in the app; do not fabricate one), and the
    new card joins the loop at once (`addCard` now enrols, gated on both `MAX_CARDS` and
  ```

  with:

  ```markdown
    opens a menu** of built-in kinds plus a `pluginKinds` group — empty until 2026-09-07,
    and since then built from the server's catalog (see the next bullet) — and the
    new card joins the loop at once (`addCard` now enrols, gated on both `MAX_CARDS` and
  ```

  Verify: `cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity && grep -c "do not fabricate one" CLAUDE.md` prints `0`.

- [ ] **Step 5b: Insert the CLAUDE.md sibling bullet and commit.** Insert immediately after line 179 (`inside the closed Settings sheet (pre-existing).`) and before line 180 (`- **V2's exit gate is OPEN`):

  ```markdown
  - **A plugin card is a peer of a built-in card in the companion app, and the SERVER
    renders its preview (2026-09-07; spec
    `docs/superpowers/specs/2026-09-06-deskmate-plugin-card-parity-design.md`, plan
    `docs/superpowers/plans/2026-09-07-deskmate-plugin-card-parity.md`, branch
    `feat/plugin-parity`).** Owner direction: "the user should not see a difference between
    server rendered cards and regular cards"; approach 2 of three was chosen — the server
    renders the previews, because the Mac has neither a plugin registry nor a rasterizer.
    **Schema stays v6, protocol stays v1, nothing new reaches the device.**
    - **Manifest v2 gains three optional keys** (`docs/plugins/manifest-v2.md`, whose bounds
      section is machine-checked by `crates/plugin/tests/manifest_v2_doc.rs`):
      `display_name` (64 bytes), `description` (160 bytes), and `summary` — a **data**
      expression whose evaluated output is truncated to 32 bytes and published as the card's
      `hero` field. A device binding (`time:`, `timer.`, `date`, `field.`) in a summary is a
      parse-time `SummaryUsesDeviceBinding`, and a present-but-empty `display_name` or
      `description` is `EmptyString` rather than a fallback request. **Manifest v1 stays
      frozen**; all four curated plugins moved to v2, so v1 coverage now rides on the two
      byte-exact copies under `crates/plugin/tests/fixtures/`, not on the shipped manifests.
    - **Two additive admin surfaces, both admin-bearer.** `GET /v1/plugins` carries the new
      keys plus `manifest_version`, `template` and `refresh_minutes`;
      `GET /v1/devices/{id}/cards/{card_id}/preview` returns the card's face as a PNG,
      built on the runtime worker with **revision 0** — never minted, never sent, no device
      I/O, no raster floor, no dirty mark, and the card is never activated. All seven shared
      DTOs live in `app-core/src/admin.rs` so the server serializes and the Mac deserializes
      **one** type, re-exported from the crate root **except** `PluginLoadFailure`, which
      stays `app_core::admin::PluginLoadFailure` because
      `server::plugin_registry::PluginLoadFailure` already exists and a second name in one
      scope is a trap. Existing catalog fields serialize byte-identically to before. An
      unknown or non-plugin card is `RuntimeError::UnknownCard` / `NotAPluginCard` in
      app-core and the **existing unit** `AdminError::NotFound` in the server.
    - **The new `hero` field is safe on the wire, and this was verified rather than
      assumed.** A plugin card's wire template is always `DigitalClock`, whose firmware
      registry declares only `title`, `show_seconds`, `stale` and `error`;
      `firmware/main/core/template_fields.h` states that unknown fields are ignored, so the
      device drops `hero` and no device-side change was needed.
    - **The Mac stopped pretending.** With no plugin host the hostless runtime schedules no
      provider deadline for a plugin card and reports no provider entry for it; the
      permanent `stale` flag it used to show was a defect, not a state. A
      `ServerStateProjection` overlays the server's per-card provider state, `hero` value
      and errors onto the snapshot in networked tier only (polled every 30 s while visible,
      64 KiB bodies for catalog and status, 1 MiB for a preview).
    - **A waiting plugin card prints a state WORD, not the "No data yet" badge.** The badge
      means "a real frame rendered from sample data"; a card waiting for its first refresh
      has no frame at all, so the preview returns `png_base64: null, sample: false,
      state: "Waiting for the first refresh"` and the stage prints that sentence.
    - **The accepted deviation is the preview's fidelity, and PRODUCT.md states it.** A
      display-list plugin's preview comes from `resvg`, not LVGL, so it can differ exactly
      where `docs/scene/template-parity-ledger.md` says (LVGL ellipsizes an overflowing
      line; the raster shows it whole). An SVG-template plugin is exact by construction.
      Byte-exact Mac-side rendering of the server's compiled scene stays available later as
      an additive extension of the same route.
    - **Rollout is server-then-Mac and the order is load-bearing**: a v2 manifest pushed to a
      server built before this change fails `deny_unknown_fields`, so the plugin drops out of
      the registry into `load_failures` and its cards go dark. Redeploy the binary first,
      then the manifests. Against a server that predates the preview route the stage says
      "Plugin previews need a newer server" — a distinct sentence from local tier's "Plugin
      cards render on the server", which is chosen from the known tier before any request is
      made. The recipe is written down in `companion/crates/server/deploy/README.md` §6.
    - **Rendering a plugin card in local tier is an explicit NON-GOAL** — there is no server
      to render it — so the card is flagged `needs the server` and the stage says so. Do not
      add a Mac-side plugin renderer to "fix" it.
  ```

  Verify:

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity && grep -n "A plugin card is a peer of a built-in card in the companion app" CLAUDE.md
  ```

  Expected: exactly one line.

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity add CLAUDE.md && git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -m "docs: record plugin card parity and its rollout order in CLAUDE.md" -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>" -m "Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
  ```

- [ ] **Step 6: Write the redeploy recipe into the deploy runbook.** It currently lives only in CLAUDE.md's prose and scattered board-notes entries; a rollout that must be repeated belongs in the runbook beside the unit files.

  (a) Insert four lines into §2, after line 41 (`file, so no second unit-local value should be added.`) and the blank line that follows it, immediately before line 43 (`**\`DESKMATE_ADMIN_TOKEN\` is the whole V2 auth story for the Mac-facing`):

  ```markdown
  `DESKMATE_PLUGINS_DIR` is optional and defaults to `/var/lib/deskmate/plugins`
  (`main.rs`'s `DEFAULT_PLUGINS_DIR`, line 26); it must be absolute. That directory holds
  **content**, not code: one subdirectory per plugin id, each with a `manifest.toml`
  and its declared assets. It is deployed separately from the binary -- see §6.

  ```

  (b) Append §6 to the end of the file:

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity && cat >> companion/crates/server/deploy/README.md <<'MDEOF'

  ## 6. Redeploying an already-running server

  This is the recipe the live deployment at `deskmate.rodi.one` actually uses. There is no
  Rust toolchain on the VM: the binary is cross-built in a throwaway container over an
  rsync'd source export.

  **Reach the VM over Tailscale** (`docker-vm`, `100.93.166.123`). `~/.ssh/config` pins its
  LAN address (`192.168.8.20`), which is unreachable from any other network, so use the
  Tailscale address explicitly.

  **Export from `git archive HEAD`, never from the working tree.** A dirty tree deploys code
  nobody can reproduce. The export needs `tools/fonts/` as well as `companion/`, because the
  server does `include_bytes!("../../../../tools/fonts/...")` for the pinned rasterizer faces
  (`rasterizer.rs:23-24`, `plugin_host.rs:329`) -- a `companion/`-only export stopped
  building at stage 4.

  ```sh
  # On the Mac, from the repository root:
  rm -rf /tmp/deskmate-deploy && mkdir -p /tmp/deskmate-deploy
  git archive HEAD companion tools/fonts | tar -x -C /tmp/deskmate-deploy

  # Sources only. `target/` on the VM is root-owned and left by the previous deploy:
  # keeping it turns a cold build into roughly 40 seconds, so the exclude below is what
  # protects it from --delete. Never drop it.
  rsync -a --delete --exclude 'target/' \
    /tmp/deskmate-deploy/companion/ rodion@100.93.166.123:~/deskmate-build/companion/
  rsync -a --delete \
    /tmp/deskmate-deploy/tools/fonts/ rodion@100.93.166.123:~/deskmate-build/tools/fonts/
  ```

  Build and install on the VM. The container image must match
  `companion/rust-toolchain.toml` (`1.98.0`). **The bin target is `server`, not
  `deskmate-server`** -- the installed file is renamed on the way in, and
  `-p server --bin deskmate-server` fails with "no bin target named".

  ```sh
  ssh rodion@100.93.166.123
  cd ~/deskmate-build
  sudo -n docker run --rm -v "$PWD":/work -w /work/companion rust:1.98-bookworm \
    cargo build --release -p server
  sudo -n cp -a /usr/local/bin/deskmate-server "/usr/local/bin/deskmate-server.bak-$(date +%Y%m%d)"
  sudo -n install -m 0755 companion/target/release/server /usr/local/bin/deskmate-server
  ```

  **Deploy plugin content after the binary, never before.** Every manifest table is
  `deny_unknown_fields`, so a manifest using keys the running binary does not know fails to
  load and the plugin drops out of the registry into `load_failures` -- its cards go dark.
  Sync into the directory `DESKMATE_PLUGINS_DIR` names, **without** `--delete`: the live
  directory may hold ids that are not in the repository (session fixtures such as
  `svg-live-clock`), and deleting them is not part of a redeploy.

  ```sh
  # From the Mac, from the same export:
  rsync -a /tmp/deskmate-deploy/companion/plugins/ rodion@100.93.166.123:~/deskmate-plugins/
  ssh rodion@100.93.166.123 'sudo -n rsync -a ~/deskmate-plugins/ /var/lib/deskmate/plugins/'
  ```

  Restart and read the registry line:

  ```sh
  ssh rodion@100.93.166.123
  sudo -n systemctl restart deskmate-server
  sudo -n journalctl -u deskmate-server -n 5 --no-pager   # plugin registry loaded ... failure_count=0
  ```

  A non-zero `failure_count` means a manifest the running binary cannot parse; an empty
  registry logs `no plugins loaded; plugin cards will be refused ...` instead. Read the
  failures through `GET /v1/plugins`'s `load_failures` before touching anything else.

  **Verify the admin surface without ever printing the token.** Read it into a shell
  variable and use it only in the header; never `echo` it, never pass it on a command line
  in a shell whose history is kept.

  ```sh
  ssh rodion@100.93.166.123 'bash -s' <<'REMOTE'
  set -eu
  TOKEN=$(sudo -n grep "^DESKMATE_ADMIN_TOKEN=" /etc/deskmate/server.env | cut -d= -f2-)
  BIND=$(sudo -n grep "^DESKMATE_SERVER_BIND=" /etc/deskmate/server.env | cut -d= -f2-)
  curl -s -H "Authorization: Bearer $TOKEN" "http://$BIND/v1/plugins" | python3 -m json.tool
  unset TOKEN
  REMOTE
  ```

  **Redeploy whenever the config schema or the manifest contract moves.** The server
  compiles its own `CURRENT_SCHEMA_VERSION` in, so a schema bump on the app side does
  nothing to a live deployment until the binary is replaced; the symptom is a typed
  "schema version N is not supported; expected M" on the first save, which reads like a
  config problem rather than a deploy problem.
  MDEOF
  ```

  Verify:

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity && grep -nE "## 6\. Redeploying|DESKMATE_PLUGINS_DIR|exclude 'target/'" companion/crates/server/deploy/README.md
  ```

  Expected: exactly four lines — the §2 sentence, the §6 heading, the `--exclude 'target/'` rsync line, and §6's "the directory `DESKMATE_PLUGINS_DIR` names".

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity add companion/crates/server/deploy/README.md && git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -m "docs: write the server redeploy recipe and plugin-content order into the runbook" -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>" -m "Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
  ```

- [ ] **Step 7: Run the four Rust workspace gates.** These are the gates the whole change is held to, and this task is the last one before rollout. Keep them as four separate lines so a failure names the missing coverage directly.

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo fmt --all --check
  ```

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings
  ```

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test --workspace --all-targets
  ```

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion && /Users/rodion/.cargo/bin/cargo test --workspace --doc
  ```

  Expected: `cargo fmt` silent, clippy with no warnings (the workspace runs `clippy::pedantic` at warn with `-D warnings`, so a pedantic lint in the new test is a failure), and both test invocations ending `test result: ok` with `0 failed`. Both test lines are required: `--all-targets` adds the example and integration targets but drops doctests.

- [ ] **Step 8: Run the four frontend gates.**

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun test && /Users/rodion/.bun/bin/bun run check && /Users/rodion/.bun/bin/bun run lint && /Users/rodion/.bun/bin/bun run format:check
  ```

  Expected: `bun test` reports `0 fail`; `check` (`tsc --noEmit`) prints nothing; Biome reports `Checked N files` with no diagnostics for both `lint` (`biome lint .`) and `format:check` (`biome format .`).

- [ ] **Step 9: Walk `?scenario=plugin` in the dev harness and confirm every plugin state renders without hardware.** The harness is the only place all of the spec's §8 states are reachable at once, and DESIGN.md now promises `plugin` is among them.

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && VITE_DESKMATE_MOCK=1 /Users/rodion/.bun/bin/bun run dev
  ```

  Vite serves on `http://localhost:1420` (`vite.config.ts`, `strictPort: true`). Confirm, in order:

  1. Load `http://localhost:1420/` with no `?scenario=` (`mockBackend.ts:379-382` prints the roster only when the resolved scenario is `default`, and any unrecognised value falls back to `default`). The console prints `[deskmate mock] scenarios: …, plugin — add ?scenario=<name>` — `plugin` is in `SCENARIOS`.
  2. Load `http://localhost:1420/?scenario=plugin`. The loop grid names the two plugin cards by **display name**, not by id and not "Plugin"; the owner's title is the quiet second line; each tile shows the `hero` value from the fixture, or `—` where the fixture declares no summary.
  3. The add-card slot's menu shows a "Plugins on the server" group with the four curated plugins, each a display name over a description.
  4. The editor's **Plugin** `<select>` names the card's plugin; an id the fixture catalog lacks renders as the selected-but-invalid "Not installed on the server" option and carries the `cards[i].plugin_id` issue with `aria-invalid`.
  5. The stage renders the fixture PNG for a plugin card, and prints a **state word** — never "Preview unavailable" — where the fixture supplies no frame; a waiting card prints "Waiting for the first refresh" and shows no "No data yet" badge.
  6. `?scenario=plugin&theme=light` and `&theme=dark` both read correctly.

  Then stop the dev server (`Ctrl-C`). Nothing is committed here; a failure is a defect in Task 5, not in this task's docs.

- [ ] **Step 10: Rollout A — deploy the server binary, before any manifest moves.** Follow §6 of the runbook exactly, from the worktree root on the Mac and then on the VM.

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity && \
    rm -rf /tmp/deskmate-deploy && mkdir -p /tmp/deskmate-deploy && \
    git archive HEAD companion tools/fonts | tar -x -C /tmp/deskmate-deploy && \
    rsync -a --delete --exclude 'target/' \
      /tmp/deskmate-deploy/companion/ rodion@100.93.166.123:~/deskmate-build/companion/ && \
    rsync -a --delete \
      /tmp/deskmate-deploy/tools/fonts/ rodion@100.93.166.123:~/deskmate-build/tools/fonts/
  ```

  ```sh
  ssh rodion@100.93.166.123 'cd ~/deskmate-build && \
    sudo -n docker run --rm -v "$PWD":/work -w /work/companion rust:1.98-bookworm \
      cargo build --release -p server && \
    sudo -n cp -a /usr/local/bin/deskmate-server \
      "/usr/local/bin/deskmate-server.bak-$(date +%Y%m%d)" && \
    sudo -n install -m 0755 companion/target/release/server /usr/local/bin/deskmate-server && \
    sudo -n systemctl restart deskmate-server && \
    sudo -n journalctl -u deskmate-server -n 5 --no-pager'
  ```

  Expected: the container build finishes (roughly 40 s warm against the retained root-owned `target/`), and the journal's last lines carry `plugin registry loaded path=/var/lib/deskmate/plugins plugin_count=<n> failure_count=0` (`main.rs:146-152`). The manifests on the VM are still v1-shaped at this point and must load cleanly; a non-zero `failure_count` here means the binary regressed, so roll back to the `.bak-<date>` binary before going further.

- [ ] **Step 11: Rollout B — deploy the four curated v2 manifests and restart.** Binary first is the whole reason this is a separate step.

  ```sh
  rsync -a /tmp/deskmate-deploy/companion/plugins/ rodion@100.93.166.123:~/deskmate-plugins/ && \
  ssh rodion@100.93.166.123 'sudo -n rsync -a ~/deskmate-plugins/ /var/lib/deskmate/plugins/ && \
    sudo -n systemctl restart deskmate-server && \
    sudo -n journalctl -u deskmate-server -n 5 --no-pager'
  ```

  Expected: `plugin registry loaded … failure_count=0` again, with `plugin_count` unchanged from Step 10 (no plugin dropped out). No `--delete` on either rsync: the live directory may hold session-fixture ids that are not in the repository.

- [ ] **Step 12: Rollout C — verify `GET /v1/plugins` shows the display names on the live server.** The token is read into a shell variable from the env file and used only in the header; it is never echoed, never on a command line, and is unset afterwards.

  ```sh
  ssh rodion@100.93.166.123 'bash -s' <<'REMOTE'
  set -eu
  TOKEN=$(sudo -n grep "^DESKMATE_ADMIN_TOKEN=" /etc/deskmate/server.env | cut -d= -f2-)
  BIND=$(sudo -n grep "^DESKMATE_SERVER_BIND=" /etc/deskmate/server.env | cut -d= -f2-)
  curl -s -H "Authorization: Bearer $TOKEN" "http://$BIND/v1/plugins" | python3 -m json.tool
  unset TOKEN
  REMOTE
  ```

  Expected: a `plugins` array with one entry per curated id — `agenda`, `aqi`, `claude-limits`, `svg-aqi` — each carrying a non-null `display_name` and `description`, `"manifest_version": 2`, a `template` of `"display-list"` or `"svg"`, a `refresh_minutes`, and `"load_failures": []`. A `null` display name means the manifest sync did not land; a **missing** `display_name` key means the running binary is the old one, so Step 10 did not take.

  Then confirm the public path still authenticates, from the Mac, with no token involved:

  ```sh
  curl -s -o /dev/null -w '%{http_code}\n' https://deskmate.rodi.one/v1/plugins
  ```

  Expected: `401` — the tunnel, TLS and the running binary are all live and the admin surface is not open.

- [ ] **Step 13: Rollout D — the Mac app second, against the live server.** The app's GETs are additive and fail closed, so this order is safe; running it the other way round would have shown the fallbacks and taught nothing.

  ```sh
  cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun run tauri dev
  ```

  With the existing `dev-0005` pairing (networked tier, admin token already stored), confirm on the real app:

  1. The add-card menu's plugin group lists the four display names with their descriptions — the same strings Step 12 printed.
  2. A plugin card in the loop is named by its display name, and its tile shows the live `hero` value the server's summary produced, with the real provider flag rather than a permanent `stale`.
  3. Selecting that card renders a preview PNG on the stage; a card whose plugin has no cached snapshot yet prints the state word "Waiting for the first refresh" rather than "Preview unavailable", and shows no "No data yet" badge.
  4. The editor's Plugin select names the card's plugin and offers the other three.

  Quit the app. Record exactly what was observed; do not record an observation that was not made.

- [ ] **Step 14: Record the rollout evidence in the plan and commit.** Append to the end of `docs/superpowers/plans/2026-09-07-deskmate-plugin-card-parity.md` (125 lines today), filling in the real outputs from Steps 10-13 — dates, `plugin_count`, the four catalog entries, and what the app showed. Keep it factual: this is the durable record that the deploy happened and what it produced.

  ```markdown
  ## Rollout record

  | Step | When | Result |
  |---|---|---|
  | Server binary redeployed (§6 recipe) | `<date>` | `plugin registry loaded … plugin_count=<n> failure_count=0`; previous binary kept as `deskmate-server.bak-<date>` |
  | Curated v2 manifests deployed | `<date>` | `plugin_count` unchanged, `failure_count=0` |
  | `GET /v1/plugins` on the live server | `<date>` | four ids with non-null `display_name`/`description`, `manifest_version` 2, `load_failures: []` |
  | Public `GET /v1/plugins` unauthenticated | `<date>` | `401` |
  | Mac app against the live server | `<date>` | `<what was observed in Step 13, items 1-4>` |

  Not observed, and not to be described as observed: anything on the physical panel. This
  change sends nothing new to the device; the panel's faces are unchanged by it.
  ```

  ```sh
  git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity add docs/superpowers/plans/2026-09-07-deskmate-plugin-card-parity.md && git -C /Users/rodion/dev/deskmate/.worktrees/plugin-parity commit -m "docs: record the plugin-parity server redeploy and rollout evidence" -m "Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>" -m "Claude-Session: https://claude.ai/code/session_01PRmcypTBQdvwFcuxzSaJ3S"
  ```

**Task exit:** `cargo test -p plugin --test manifest_v2_doc` green (5 passed) and both Step 2 mutation probes observed red then restored, with `git status --short docs/plugins/manifest-v2.md` printing nothing (Task 1's contract document is byte-unchanged by this task); the four workspace gates from `companion/` green — `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --all-targets`, `cargo test --workspace --doc`; the four frontend gates from `companion/apps/deskmate` green — `bun test`, `bun run check`, `bun run lint`, `bun run format:check`; `?scenario=plugin` renders every state in Step 9 with no console errors; `GET /v1/plugins` on `deskmate.rodi.one` returns all four curated plugins with non-null `display_name` and `description`, `manifest_version` 2, and `load_failures: []`, and the unauthenticated public path returns `401`; and the rollout record in the plan carries only observations that were actually made.

## Rollout record

Task 6's implementer worked under an explicit hard boundary: no `ssh`, `rsync`, `scp`,
`systemctl`, or `curl` against `deskmate.rodi.one` or the VM, and nothing that reaches
outside the worktree. A production deploy is the owner's action, not an agent's. Steps
10-13 were therefore **documented, not executed** — the exact commands live in
`companion/crates/server/deploy/README.md` §6 (redeploy) and are repeated below for the
owner to run and fill in.

| Step | When | Result |
|---|---|---|
| Server binary redeployed (§6 recipe) | 2026-09-08 | Built in `rust:1.98-bookworm` from a `git archive origin/main` export (`faac9ab`); previous binary kept as `/usr/local/bin/deskmate-server.bak-20260908`. Restart logged `plugin registry loaded path=/var/lib/deskmate/plugins plugin_count=5 failure_count=0` — the new binary against the **old v1** manifests, which is manifest v1 staying frozen |
| Curated v2 manifests deployed | 2026-09-08 | `rsync` without `--delete` into `/var/lib/deskmate/plugins/`; restart logged `plugin_count=5 failure_count=0` again. Count is 5, not 4: the live directory also holds the session fixture `svg-live-clock`, which is not in the repository and was deliberately not deleted |
| `GET /v1/plugins` on the live server | 2026-09-08 | `200`. All four curated ids carry non-null `display_name`/`description`, `manifest_version: 2`, `template`, and `refresh_minutes`; `load_failures: []`. `svg-live-clock` returns `display_name: null` / `description: null`, which is the absent-key case reaching the wire as a null rather than falling back to its id |
| Public `GET /v1/plugins` unauthenticated | 2026-09-08 | `401` from `https://deskmate.rodi.one/v1/plugins` |
| Mac app against the live server | 2026-09-08 | **Partly observed.** A release bundle built from `faac9ab` (`Deskmate_1.0.0_aarch64.dmg`) is installed at `/Applications/Deskmate.app`; it launches, is tray-resident, and its settings window renders the loop, the ring, the dashed add-card slot ("Built in, or a plugin") and the `not in loop` / `alerts` flags against the live server with no error notice. The catalog contract was verified directly rather than by eye: the live `GET /v1/plugins` body through the public tunnel deserializes cleanly into `app_core::admin::PluginCatalog`, the same type the app uses — 5 plugins, 0 load failures. **Not observed:** the add-menu display names, a live `hero` value, and the preview PNG / state word, because `dev-0005` has been disconnected since 2026-09-07 (`GET .../preview` correctly returns the typed 503 "this device has no runtime yet; it has never connected") **and** because `dev-0005`'s config no longer contains any plugin card — see the note below |

**Not observed, and not to be described as observed: anything past what Tasks 1-5 already
verified in software and the dev harness.** This change sends nothing new to the device;
the panel's faces are unchanged by it. Task 6 verified, in this worktree, without touching
the live server: the doc-contract test (`cargo test -p plugin --test manifest_v2_doc`, 5
passed) and its two mutation probes, all four Rust workspace gates, all four frontend
gates, and a full walk of `?scenario=plugin` in the dev harness.

**Rollout A-C were executed on 2026-09-08 and passed; Rollout D is partly observed** (see
the table above). Two facts found while executing it, neither caused by this change:

1. **`dev-0005` carries no plugin card any more.** Both the server's
   `/var/lib/deskmate/configs/dev-0005.json` and the Mac's own config store hold the same
   four built-in cards (clock, pomodoro, weather, rss) at schema v6, with no
   `claude-limits` entry. CLAUDE.md's claim that `dev-0005`'s config carries that card is
   therefore stale as written; it was true when the plugin was deployed on 2026-09-03 and
   stopped being true no later than the 2026-09-06 hardware session, whose save is the
   most recent write to both files. Nothing in the fleet currently authors a plugin card,
   so the parity surface has nothing live to draw.
2. **The preview renders the device's framebuffer, not the viewer's view.** With
   `preferences.orientation: "landscape-flipped"`, `render_card_preview`
   (`companion/apps/deskmate/src-tauri/src/commands.rs:1031-1035`) passes
   `SimOrientation::LandscapeFlipped` straight through, so the settings window shows the
   clock upside down. That is faithful to the bytes the panel receives and unfaithful to
   what a person standing at the panel sees, since the 270 degree mounting is exactly what
   cancels the flip. It is undocumented and unpinned by any test, and it predates this
   change. Recorded as a finding, not fixed: which of the two the preview should show is a
   design decision, not a cleanup.

The original commands, for reference:

1. **Rollout A — redeploy the server binary**, from the worktree root:

   ```sh
   \
     rm -rf /tmp/deskmate-deploy && mkdir -p /tmp/deskmate-deploy && \
     git archive HEAD companion tools/fonts | tar -x -C /tmp/deskmate-deploy && \
     rsync -a --delete --exclude 'target/' \
       /tmp/deskmate-deploy/companion/ rodion@100.93.166.123:~/deskmate-build/companion/ && \
     rsync -a --delete \
       /tmp/deskmate-deploy/tools/fonts/ rodion@100.93.166.123:~/deskmate-build/tools/fonts/
   ```

   ```sh
   ssh rodion@100.93.166.123 'cd ~/deskmate-build && \
     sudo -n docker run --rm -v "$PWD":/work -w /work/companion rust:1.98-bookworm \
       cargo build --release -p server && \
     sudo -n cp -a /usr/local/bin/deskmate-server \
       "/usr/local/bin/deskmate-server.bak-$(date +%Y%m%d)" && \
     sudo -n install -m 0755 companion/target/release/server /usr/local/bin/deskmate-server && \
     sudo -n systemctl restart deskmate-server && \
     sudo -n journalctl -u deskmate-server -n 5 --no-pager'
   ```

   Expect the journal's last lines to carry
   `plugin registry loaded path=/var/lib/deskmate/plugins plugin_count=<n> failure_count=0`
   with the manifests still v1-shaped at this point. A non-zero `failure_count` means the
   binary regressed — roll back to the `.bak-<date>` binary before continuing.

2. **Rollout B — deploy the four curated v2 manifests and restart:**

   ```sh
   rsync -a /tmp/deskmate-deploy/companion/plugins/ rodion@100.93.166.123:~/deskmate-plugins/ && \
   ssh rodion@100.93.166.123 'sudo -n rsync -a ~/deskmate-plugins/ /var/lib/deskmate/plugins/ && \
     sudo -n systemctl restart deskmate-server && \
     sudo -n journalctl -u deskmate-server -n 5 --no-pager'
   ```

   Expect `plugin registry loaded … failure_count=0` again, with `plugin_count` unchanged
   from step 1.

3. **Rollout C — verify `GET /v1/plugins` on the live server**, token read into a shell
   variable and never echoed:

   ```sh
   ssh rodion@100.93.166.123 'bash -s' <<'REMOTE'
   set -eu
   TOKEN=$(sudo -n grep "^DESKMATE_ADMIN_TOKEN=" /etc/deskmate/server.env | cut -d= -f2-)
   BIND=$(sudo -n grep "^DESKMATE_SERVER_BIND=" /etc/deskmate/server.env | cut -d= -f2-)
   curl -s -H "Authorization: Bearer $TOKEN" "http://$BIND/v1/plugins" | python3 -m json.tool
   unset TOKEN
   REMOTE
   ```

   Expect one entry per curated id (`agenda`, `aqi`, `claude-limits`, `svg-aqi`) with
   non-null `display_name`/`description`, `"manifest_version": 2`, a `template`, a
   `refresh_minutes`, and `"load_failures": []`. Then, from the Mac, with no token:

   ```sh
   curl -s -o /dev/null -w '%{http_code}\n' https://deskmate.rodi.one/v1/plugins
   ```

   Expect `401`.

4. **Rollout D — the Mac app against the live server:**

   ```sh
   cd /Users/rodion/dev/deskmate/.worktrees/plugin-parity/companion/apps/deskmate && /Users/rodion/.bun/bin/bun run tauri dev
   ```

   With the existing `dev-0005` pairing (networked tier, admin token already stored),
   confirm: the add-card menu's plugin group lists the four display names with their
   descriptions matching step 3's output; a plugin card in the loop is named by its
   display name with a live `hero` value and a real provider flag; selecting it renders a
   preview PNG, or the state word "Waiting for the first refresh" for a card with no
   cached snapshot yet (never "Preview unavailable", never a spurious "No data yet"
   badge); and the editor's Plugin select names the card's plugin and offers the other
   three. Quit the app and fill in the table above with what was actually observed.
