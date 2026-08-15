# Deskmate V1 Packaging & Hardening — Design

**Status:** Approved 2026-08-15.
**Supersedes:** M4 Task 9 (`docs/superpowers/plans/2026-08-05-deskmate-m4-v1-completion.md`),
which the reset spec (`2026-08-11-deskmate-v1-reset-design.md` §7) deferred to V1's exit
"rewritten against V1 scope at planning time". This document is that rewrite. It is the
last V1-exit item on the roadmap.

## 0. Scope decisions (user-directed, 2026-08-15)

1. **Audience: dogfood.** The V1 packaged build runs on the user's Mac only. macOS-only;
   no Windows work of any kind. Ad-hoc signing is the documented v1 distribution
   equivalent — no Apple Developer account, no notarization. Hardening (advisory triage,
   security review) still runs at full strength.
2. **Build gate: GitHub repo + Actions now.** The repo — currently local-only with no
   remote — is pushed to a **private** GitHub repo and CI is added there, rather than a
   local release script. The pipeline exists before V2 needs it.

Dropped from the original Task 9, with reasons: Windows CI/bundles/testing (no V1
story; host handoff is V2+), notarization (buys nothing on the owner's own machine),
updater fault-injection (no updater exists until V2's OTA/update mechanism).

## 1. Repo goes remote

Private repo `cosmicsymmetry/deskmate` on GitHub. `main` is pushed as-is: no history
rewrite, no branch protection ceremony for a solo repo. The remote is a mirror plus CI
trigger; local remains the primary working copy.

## 2. CI (GitHub Actions)

One workflow, jobs on push and pull_request to `main`:

- **companion** (`macos-latest`):
  `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`;
  `cargo test --workspace`; `bun install --frozen-lockfile`; frontend typecheck
  (`tsc --noEmit`), `biome format`/`biome lint` checks, `bun test`; production
  `vite build`; `tauri build` producing a DMG/.app artifact uploaded with a bounded
  retention window.
- **firmware** (`ubuntu-latest`):
  `make -C firmware/host_tests clean test`; full `idf.py build` via
  `espressif/esp-idf-ci-action` pinned to the ESP-IDF 5.x release the project uses.

Lockfiles are frozen everywhere (`--frozen-lockfile`, `--locked`); CI fails if any
lockfile would change. Actions are pinned by major version at minimum.

## 3. Advisory posture

`cargo audit` and `bun audit` (or the bun-ecosystem equivalent) run in CI,
**non-blocking initially** so a new upstream advisory cannot brick unrelated work.
Every advisory present at implementation time is either resolved or recorded as an
exception — owner, impact, upgrade trigger — in `docs/security/advisories.md`
(checked in). If GTK3/unmaintained paths appear in the dependency tree, verify whether
they are actually linked into the macOS build (typically Linux-only) and record the
finding either way.

## 4. Security review

A written pass, output committed as `docs/security/v1-review.md`, covering:

- every `#[tauri::command]` IPC surface (inputs trusted/untrusted, what each can do),
- the CSP and `capabilities/main.json`,
- URL/app-launch policy (what, if anything, can open external URLs or apps),
- log contents (no secrets, tokens, or PII),
- config-persistence file permissions,
- contents of the release DMG (nothing unexpected ships).

Findings are fixed in the same task or recorded with a reason.

## 5. Packaging

- Version `1.0.0` across `tauri.conf.json` and the workspace/package manifests.
- `bundle.targets` narrowed from `"all"` to exactly the DMG + .app targets.
- Ad-hoc signing documented as the v1 distribution equivalent (Gatekeeper
  right-click-open on first launch; irrelevant on the building machine itself).
- Upgrade behavior: replace the `.app`; config migration is already proven by the
  v0→v4 schema chain and its fixtures.

## 6. Hands-on install matrix

Run once on the user's Mac, results recorded in the implementation plan (not
board-notes — no board involvement):

clean install → first run → upgrade over an M3-era config → single instance →
autostart on/off → close-to-tray → quit → offline use (device unplugged) →
corrupt-config recovery → uninstall.

## 7. Exit criteria

- CI green on GitHub for `main`, including the firmware job.
- A DMG artifact downloadable from a CI run.
- Advisories triaged: zero unexplained; exceptions recorded in
  `docs/security/advisories.md`.
- `docs/security/v1-review.md` committed with all findings fixed or reasoned.
- Install matrix passed and recorded.

Closing this task closes the last V1-exit item. Declaring V1 exit itself (any tag,
V2 brainstorm) is a separate decision requiring explicit user authorization.

## 8. Out of scope

Windows anything; notarization/Developer ID; auto-update/updater plumbing (V2);
public distribution; release notes/support docs for third parties; CI for docs or
release automation beyond the single workflow above.
