# Deskmate simplification cleanup — 2026-09-05

Source: the whole-repository simplification review of 2026-09-03/04 (207 findings from
23 review areas, 164 confirmed by independent verifiers, 26 contested and adjudicated,
17 refuted and kept). The review was static: nobody built, flashed, or pushed anything.
This plan applies the confirmed part and records what was deliberately left alone.

Branch: `feat/v2-networked-device`. Base: `4ee63ed`. Protocol stays v1 and additive,
the config schema stays **v6**, `CURRENT_CAPABILITIES` stays **1003**, and every frozen
contract (`docs/protocol/v1.md`, `docs/config/v6.md`, `docs/plugins/manifest-v1.md` and
`-v2.md`) is untouched except where a sentence is corrected to match shipped code, and
each such correction is dated in the document.

## Scope decisions

Applied in this plan:

- **Repo hygiene.** The accidentally tracked `firmware/build-diag/` tree (2,366 generated
  files, 217 MB, entered in `300d91f`), the generated `sdkconfig.diagbuild`, the rejected
  `sdkconfig.swaes` experiment (its option is recorded in board-notes first), 50
  non-macOS icons, 52 flipped golden PNGs that re-pinned one buffer reversal, a
  `.gitignore` that named 24 host-test binaries one by one, a CI job that ran the
  typecheck three times, and four unused Cargo/build entries.
- **Stale documents.** README, PRODUCT.md, the roadmap's "Current" paragraph, the deploy
  runbook's schema version, and a handful of code comments that describe removed paths.
- **Defects the review found** (22), each with a test that fails without the fix where
  the behaviour is host-testable. The ones that change observable behaviour are listed
  in the execution notes below with the decision taken.
- **Host-side simplifications** (about 65): dead types and accessors, code written three
  to five times across crates (UTF-8 byte truncation, the request/response table,
  request-id allocation, hex encoding, secure file I/O, text-field helpers, FieldIssues),
  redundant re-validation, and tests that re-prove one fact many times.
- **Firmware simplifications** (27 of 29): duplicate state and validators in `core/`,
  accessors that exist only for host-test introspection, dead failure branches after
  `lvgl_port_lock(0)`, the undocumented OTA-check sentinel request id, and test overlap.
  **All firmware changes ride ONE image.** See "Hardware owed".
- **Refactors** (11 of 14): the simulator's asset decode/re-encode path and its second
  digest index, the shared golden harness, table-driven egress deny tests, the
  `DirectHttpFetcher` test fixture, the dev-only 0x7D asset-font probe together with its
  host oracle, and the generated Rust/TypeScript IPC contract (attempted with a fallback
  to hand-adding the two missing members; see execution notes for which landed).

Deferred to the owner, on purpose (they change product behaviour or a frozen contract,
or need a hardware session of their own):

- Schema v7 removals: the inert `AppConfig::updater`, the three `WidgetTapAction`
  variants that can never compile, and the `future-v7.json` fixture.
- The M2 CLI commands (`apply-config`, `push-clock`, `push-calendar`, `pomodoro`), which
  cannot draw a card on scene-native firmware; they are labelled legacy in the README.
- The standalone clock's brightness gesture (a `feat/display-brightness` worktree exists;
  it was not touched).
- Slimming CLAUDE.md to its rules; public Rust API with no in-repo caller;
  `publish = false` on app-core; `SaveReceipt.warning`/`generation` and the second
  directory fsync behind them (an API-shape change); the unreachable
  `RasterRequest::DisplayList` raster path (a design amendment).
- Two high-risk firmware image changes that need goldens, the panel and an OTA download
  in one session: trimming `lv_conf.h`'s unused widget suite and direction-specific codec
  entry points.
- The `aqi_fixture.rs` consolidation (its header is the record of the fixture envelope).
- Two optional taste items in `scene_build.rs` (chip/surface helper extraction and the
  `background` parameter whose oracle comment calls it load-bearing).

## Method

Ten Codex workers (gpt-5.6-sol, high effort) implemented disjoint batches in isolated git
worktrees off `4ee63ed`, each with an ownership list, the reviewer evidence and every
verifier correction per item, and an order to skip any item whose claim did not survive
re-checking against the code. Two more batches (server; Tauri/IPC contract) ran after the
first wave merged, because they consume helpers the first wave exported. The controller
squash-merged each branch as one conventional commit and ran the full gates after each
merge:

```sh
make -C firmware/host_tests clean test
make -C firmware/host_tests sanitize
. "$HOME/esp/esp-idf/export.sh" && idf.py -C firmware build && idf.py -C firmware size
cd companion && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
cd companion && cargo test --workspace --all-targets && cargo test --workspace --doc
cd companion/apps/deskmate && bun run check && bun run format:check && bun run lint && bun test
```

## Hardware owed

Nothing in this plan was observed on the board. Two checks are owed before the firmware
side is trusted, and they are the same two CLAUDE.md always names:

1. **The on-board OTA download check** for the new image, because firmware statics moved
   (the before/after `idf.py size` lines are in the execution notes). This repository has
   twice lost days to memory-layout shifts with every test green.
2. **A look at the panel** at both orientations after the image is on the device: the
   standalone clock (its per-second invalidate is now conditional), a ticking card face,
   and one plugin face.

## Execution notes

Baseline on the untouched tree (`6a4b4a2`): host tests and sanitizer pass; `idf.py size`
DIRAM 203,891 (`.bss` 87,128, `.data` 23,128), IRAM 16,384/16,384 with 0 remaining, image
1,600,551 bytes.

Batches landed, one squash commit each, in merge order:

| Commit | Batch | Gates run on the merged tree |
| --- | --- | --- |
| `59fea2e` | repo hygiene (controller) | tree inspection only |
| `580e2a8` | stale documents | link check |
| `6dbcb71` | firmware link/UI | host tests, sanitize, `idf.py build` + size |
| `6a387fa` | frontend | `bun run check`, format, lint, 97 tests |
| `f0cd919` | app-core config/store | workspace check, app-core 262 tests, TS check + tests |
| `3e1979c` | firmware core | host tests, sanitize, `idf.py build` + size |
| `8fc534b` | app-core runtime | workspace check, app-core 262, server lib 193 |
| `2867b68` | plugin crate | (see below) |
| `29e4524` | protocol / device / engine / providers | workspace check |
| `b2994e4` | app-core scene / asset sync | workspace check, app-core 257 tests |
| `332c9b6` | Tauri shell / IPC contract | workspace check, unit suites, TS check + 100 tests |
| `465e037` | lvgl-sim | host tests, sanitize, `idf.py build` + size (image unchanged) |
| `5a572e1` | server | (full gates below) |
| `f40958a` | integration fix: `SceneDataState::OK` was removed by the scene batch after the server batch branched | full gates below |

**Final gates on `f40958a`, all green:** `cargo fmt --all --check`; `cargo clippy
--workspace --all-targets -- -D warnings`; `cargo test --workspace --all-targets` (859
tests, 0 failed, including the loopback-bound server tests the sandbox had denied the
workers); `cargo test --workspace --doc`; `bun run check`, `format:check`, `lint`, `bun
test` (100 tests); `make -C firmware/host_tests clean test` and `sanitize`; `idf.py -C
firmware build` and `size` (the table above; the final image is byte-identical to the
post-core build, so the simulator batch's firmware edits were dev-diag-only as claimed).

Firmware size after the two firmware batches, same tree, same toolchain:

| Point | DIRAM | `.bss` | `.data` | IRAM | image bytes |
| --- | --- | --- | --- | --- | --- |
| baseline | 203,891 | 87,128 | 23,128 | 16,384 (0 left) | 1,600,551 |
| after link/UI batch | 203,875 | 87,112 | 23,128 | 16,384 (0 left) | 1,600,183 |
| after core batch | 203,867 | 87,104 | 23,128 | 16,384 (0 left) | 1,599,379 |

`.bss` moved by 24 bytes in total. That is the class of shift that has broken OTA
downloads before, so the on-board download check is owed, not optional.

Items skipped on evidence, with the reason the worker recorded:

- Server registry on `app_core::secure_file` (the shared-secure-file-I/O item):
  `secure_file` and every function it would need are `pub(crate)` in app-core, so the
  adoption needs an app-core API decision first. Left in place; the registry keeps its
  own bounded read / 0600 atomic write.
- The ts-rs generated IPC contract: ts-rs 12.0.1 cannot represent the
  `serialize_with`/`deserialize_with` contract on `DeviceSnapshot.unknown_capability_bits`,
  so the hand-built contract stays, hardened as described in `332c9b6`.

- `ConfigStore::path` (and the two accessors bundled with it): a server test calls it
  (`server/src/lib.rs`, `in_memory_config_root_is_independent_from_firmware_storage`).
- The two optional `scene_build.rs` taste items were not attempted.
- Historical plan files that name removed symbols (`link_state_accept_push`,
  `asset_transfer_resume_offset`, `wifi_station_time_synced`, `AssetSyncReport`, the old
  `build_analog_clock_scene` signature) were left as history; CLAUDE.md and board-notes
  carry a dated annotation where the reference read as current fact.

Decisions taken during execution (owner may reverse):

- docs/config v4-v6: the v3 fallback playlist is synthesized in authored array order, as
  the code and its test do; the three documents were amended and dated rather than the
  code changed.
- The tray "Pause pushing" item is removed (wave 2), because CLAUDE.md records the pause
  control as removed and says `setPushingPaused` has no everyday control any more; the
  one-off "Resume sending" notice stays.
- The standalone clock's per-second full-canvas invalidate is now conditional on the
  online hint changing. That decision is inside a static LVGL timer path and is not
  host-testable without adding production surface, so it has no host test; it is on the
  panel list above.

Two process notes. The Codex workers could not `git commit` (the sandbox cannot write
the worktree's `.git/worktrees/<name>/index.lock`) or run `idf.py` (`psutil`
`sysctl` PermissionError), so the controller committed every branch and ran every
ESP-IDF build; three workers were killed mid-run when the harness stopped its background
tasks and were resumed by Codex session id with their worktree edits intact.
A note on one interrupted check: the `cargo check` run after the plugin merge failed with
an arity error in `lvgl-sim` because the scene batch was merged into the working tree
while that check was mid-build; the checks that ran after it on the same tree pass, and
the final full gates below are the ones that count.
