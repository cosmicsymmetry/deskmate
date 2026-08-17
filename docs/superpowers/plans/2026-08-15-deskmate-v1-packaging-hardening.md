# V1 Packaging & Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the last V1-exit item: a reproducible 1.0.0 macOS release build gated by GitHub Actions CI, with advisories triaged, a written security review, and a passed hands-on install matrix.

**Architecture:** The local-only repo gains a private GitHub remote (`cosmicsymmetry/deskmate`) and one CI workflow with two jobs: `companion` on `macos-latest` (Rust + frontend gates, Tauri release DMG artifact) and `firmware` on `ubuntu-latest` (ESP-IDF build + host tests inside the IDF container, because `firmware/managed_components/` is gitignored and host tests need the fetched tinycbor sources). Hardening is two committed documents — `docs/security/advisories.md` and `docs/security/v1-review.md` — plus fixes they trigger.

**Tech Stack:** GitHub Actions, `gh` CLI (authenticated as `cosmicsymmetry`), Tauri v2 (`@tauri-apps/cli` 2.11.4 via bun), bun 1.3.8 (`bun.lock`, `bun audit`), cargo-audit, ESP-IDF v5.5.5 via `espressif/esp-idf-ci-action`.

**Spec:** `docs/superpowers/specs/2026-08-15-deskmate-v1-packaging-hardening-design.md`

## Global Constraints

- macOS-only, dogfood audience. No Windows work, no notarization, no Developer ID; ad-hoc signing is the documented v1 distribution equivalent.
- GitHub repo is **private**, named `cosmicsymmetry/deskmate`. Push `main` as-is; no history rewrite.
- Lockfiles frozen everywhere: `bun install --frozen-lockfile`, cargo commands `--locked`. CI must fail if a lockfile would change.
- Audit steps in CI are **non-blocking** (`continue-on-error: true`); triage happens in `docs/security/advisories.md`.
- Version is `1.0.0` in `tauri.conf.json`, `src-tauri/Cargo.toml`, and `package.json`.
- ESP-IDF pinned `v5.5.5`, target `esp32s3`. If the `espressif/idf:v5.5.5` docker tag does not exist, fall back to `v5.5` and record that in the workflow comment.
- Conventional commit prefixes. No tags (tags require explicit user authorization).
- Do not touch the wire protocol, firmware behavior, or app features; this plan is packaging/CI/docs plus any fixes the security review or advisories force.
- The repo work happens directly on `main` (solo repo, user-approved); every task ends with a commit, and CI-facing tasks end with a push.

---

### Task 1: Version 1.0.0 and narrowed bundle targets

**Files:**
- Modify: `companion/apps/deskmate/src-tauri/tauri.conf.json`
- Modify: `companion/apps/deskmate/src-tauri/Cargo.toml` (line 3, `version = "0.1.0"`)
- Modify: `companion/apps/deskmate/package.json` (line 3, `"version": "0.1.0"`)

**Interfaces:**
- Produces: version `1.0.0` everywhere; `bundle.targets` = `["app", "dmg"]`. Task 3's CI and Task 6's install matrix build this exact configuration.

- [x] **Step 1: Bump versions**

In `companion/apps/deskmate/src-tauri/tauri.conf.json` change `"version": "0.1.0"` to `"version": "1.0.0"`. In `companion/apps/deskmate/src-tauri/Cargo.toml` change `version = "0.1.0"` to `version = "1.0.0"`. In `companion/apps/deskmate/package.json` change `"version": "0.1.0"` to `"version": "1.0.0"`.

- [x] **Step 2: Narrow bundle targets**

In `tauri.conf.json`, change `"targets": "all"` to:

```json
"targets": ["app", "dmg"]
```

- [x] **Step 3: Verify**

```sh
cd companion/apps/deskmate
jq -r '.version, (.bundle.targets | join(","))' src-tauri/tauri.conf.json
jq -r '.version' package.json
grep '^version' src-tauri/Cargo.toml
cd ../.. && cargo check -p deskmate-app --locked
```

Expected: `1.0.0` / `app,dmg` / `1.0.0` / `version = "1.0.0"`; cargo check succeeds without touching `Cargo.lock` (the workspace lock already contains the members; a version bump alone updates the lockfile's own version field — if `Cargo.lock` changes, commit that change too, it is the one legitimate delta).

- [x] **Step 4: Commit**

```sh
git add companion/apps/deskmate/src-tauri/tauri.conf.json companion/apps/deskmate/src-tauri/Cargo.toml companion/apps/deskmate/package.json companion/Cargo.lock
git commit -m "chore: version 1.0.0; bundle targets narrowed to app+dmg"
```

---

### Task 2: Private GitHub repo, main pushed

**Files:** none (git remote configuration only)

**Interfaces:**
- Produces: remote `origin` → `https://github.com/cosmicsymmetry/deskmate.git`, `main` pushed and tracking. Task 3 pushes workflows here.

- [x] **Step 1: Create the private repo and push**

```sh
cd /Users/rodion/dev/deskmate
gh repo create cosmicsymmetry/deskmate --private --source . --remote origin --push
```

If the name is taken, stop and ask the user for an alternative — do not pick one silently.

- [x] **Step 2: Verify**

```sh
gh repo view cosmicsymmetry/deskmate --json visibility,defaultBranchRef -q '.visibility + " " + .defaultBranchRef.name'
git ls-remote origin main | awk '{print $1}'
git rev-parse HEAD
```

Expected: `PRIVATE main`; the two hashes match.

---

### Task 3: CI workflow, iterated to green

**Files:**
- Create: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: the remote from Task 2; versions/targets from Task 1.
- Produces: a green `ci` workflow on `main` whose `companion` job uploads a `deskmate-dmg` artifact. Tasks 4 and 6 rely on the audit steps and the artifact respectively.

- [x] **Step 1: Write the workflow**

Create `.github/workflows/ci.yml`:

```yaml
name: ci

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  companion:
    runs-on: macos-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
        with:
          workspaces: companion
      - uses: oven-sh/setup-bun@v2
        with:
          bun-version: 1.3.8
      - name: rust fmt
        working-directory: companion
        run: cargo fmt --all --check
      - name: rust clippy
        working-directory: companion
        run: cargo clippy --workspace --all-targets --locked -- -D warnings
      - name: rust tests
        working-directory: companion
        run: cargo test --workspace --locked
      - name: frontend install
        working-directory: companion/apps/deskmate
        run: bun install --frozen-lockfile
      - name: frontend typecheck
        working-directory: companion/apps/deskmate
        run: bun run check
      - name: frontend format
        working-directory: companion/apps/deskmate
        run: bun run format:check
      - name: frontend lint
        working-directory: companion/apps/deskmate
        run: bun run lint
      - name: frontend tests
        working-directory: companion/apps/deskmate
        run: bun test
      - name: frontend build
        working-directory: companion/apps/deskmate
        run: bun run build
      - name: tauri release bundle
        working-directory: companion/apps/deskmate
        run: bunx tauri build
      - name: install cargo-audit
        uses: taiki-e/install-action@v2
        with:
          tool: cargo-audit
      - name: cargo audit (non-blocking)
        working-directory: companion
        run: cargo audit
        continue-on-error: true
      - name: bun audit (non-blocking)
        working-directory: companion/apps/deskmate
        run: bun audit
        continue-on-error: true
      - name: upload dmg
        uses: actions/upload-artifact@v4
        with:
          name: deskmate-dmg
          path: companion/target/release/bundle/dmg/*.dmg
          retention-days: 14
          if-no-files-found: error

  firmware:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      # Host tests run inside the IDF container AFTER idf.py build because
      # firmware/managed_components/ (tinycbor sources) is gitignored and only
      # exists once the component manager has fetched dependencies.
      - name: idf build + host tests
        uses: espressif/esp-idf-ci-action@v1
        with:
          esp_idf_version: v5.5.5
          target: esp32s3
          path: firmware
          command: idf.py build && make -C host_tests clean test
```

- [x] **Step 2: Commit and push**

```sh
git add .github/workflows/ci.yml
git commit -m "ci: macOS companion gate with DMG artifact; ESP-IDF build + host tests"
git push origin main
```

- [x] **Step 3: Watch the run and fix forward until green**

```sh
gh run watch --repo cosmicsymmetry/deskmate --exit-status
```

Likely first-run failures and their fixes (each fix is its own `ci:`-prefixed commit, pushed, then re-watch):
- `espressif/idf:v5.5.5` docker tag missing → change `esp_idf_version` to `v5.5` and note the fallback in the workflow comment.
- Host-test warnings under the container's gcc (Makefile uses `-Wall -Wextra -Werror`, developed under clang) → fix the warning in the test code; do not remove `-Werror`.
- `bun audit` exit/flag behavior differing from expectation → the step is `continue-on-error`, so it cannot fail the job; if the command itself is unavailable, replace the step with `bunx npm audit --omit dev` and note why.
- Tauri bundling reaching for signing identity → ensure no `signingIdentity` is configured; Tauri ad-hoc signs by default when none is set.

- [x] **Step 4: Verify the artifact exists**

```sh
gh run list --repo cosmicsymmetry/deskmate --branch main --limit 1
gh run download --repo cosmicsymmetry/deskmate -n deskmate-dmg -D /tmp/ci-dmg-check $(gh run list --repo cosmicsymmetry/deskmate --branch main --limit 1 --json databaseId -q '.[0].databaseId')
ls -lh /tmp/ci-dmg-check
```

Expected: one `Deskmate_1.0.0_aarch64.dmg` (or `x64` per runner arch) downloaded.

---

### Task 4: Advisory triage document

**Files:**
- Create: `docs/security/advisories.md`
- Possibly modify: `companion/Cargo.toml` / crate manifests / `companion/apps/deskmate/package.json` + lockfiles (only for trivially safe patch-level bumps that clear an advisory)

**Interfaces:**
- Consumes: `cargo audit` / `bun audit` output (locally and from Task 3's CI logs).
- Produces: `docs/security/advisories.md`, referenced by Task 7's closeout.

- [x] **Step 1: Run the audits locally**

```sh
cargo install cargo-audit --locked   # skip if already installed
cd /Users/rodion/dev/deskmate/companion && cargo audit
cd apps/deskmate && bun audit
```

- [x] **Step 2: Check the GTK3/unmaintained question for macOS**

```sh
cd /Users/rodion/dev/deskmate/companion
cargo tree --target aarch64-apple-darwin -i gtk 2>&1 | head -5
cargo tree --target aarch64-apple-darwin -i glib 2>&1 | head -5
```

Expected: "nothing depends on" style output proving GTK/GLib are not in the macOS dependency graph (they are Linux-target deps of Tauri). Record the actual result either way.

- [x] **Step 3: Triage every finding**

For each advisory: if a patch-level dependency bump inside the existing `=x.y.z` pin style clears it and `cargo test --workspace --locked` (after `cargo update -p <crate> --precise <ver>`) plus `bun test` still pass, apply the bump. Otherwise record an exception.

Write `docs/security/advisories.md` with this structure (fill with actual findings; an empty table with a dated "no advisories" line is a valid outcome):

```markdown
# Dependency Advisory Triage — V1

Audited: <date>, cargo-audit <version>, bun <version>.
CI runs both audits non-blocking; this document is the blocking triage.

## Resolved
| Advisory | Crate/package | Action |
|---|---|---|

## Exceptions (accepted, with re-review trigger)
| Advisory | Crate/package | Owner | Impact on Deskmate | Upgrade trigger |
|---|---|---|---|---|

## GTK3 / unmaintained paths
<actual cargo tree result and conclusion for the macOS build>
```

- [x] **Step 4: Verify and commit**

```sh
cd companion && cargo test --workspace --locked && cd apps/deskmate && bun test
git add docs/security/advisories.md companion/Cargo.lock companion/apps/deskmate/bun.lock
git commit -m "docs: dependency advisory triage for V1"
git push origin main
```

Expected: tests pass; CI goes green on the push.

---

### Task 5: Security review document

**Files:**
- Create: `docs/security/v1-review.md`
- Possibly modify: `companion/apps/deskmate/src-tauri/**` (only for fixes the review forces)

**Interfaces:**
- Consumes: the built DMG from Task 3 (or a local `bunx tauri build`).
- Produces: `docs/security/v1-review.md`, referenced by Task 7's closeout.

- [x] **Step 1: Enumerate the review surfaces**

```sh
grep -rn "#\[tauri::command\]" companion/apps/deskmate/src-tauri/src -A 2
cat companion/apps/deskmate/src-tauri/capabilities/main.json
jq '.app.security' companion/apps/deskmate/src-tauri/tauri.conf.json
grep -rn "open\|shell\|Command::new\|process::" companion/apps/deskmate/src-tauri/src | grep -v "//" | head -30
grep -rn "eprintln!\|println!\|log::" companion/apps/deskmate/src-tauri/src companion/crates/app-core/src | head -40
ls -l ~/Library/Application\ Support/io.deskmate.companion/
```

- [x] **Step 2: Inspect the DMG contents**

```sh
hdiutil attach /tmp/ci-dmg-check/*.dmg -nobrowse -mountpoint /tmp/deskmate-dmg
ls -laR /tmp/deskmate-dmg/Deskmate.app/Contents | head -60
codesign -dv /tmp/deskmate-dmg/Deskmate.app 2>&1 | head -5
hdiutil detach /tmp/deskmate-dmg
```

Expected: only the app bundle + Applications symlink; `codesign` shows an ad-hoc signature (`Signature=adhoc`); no stray dev files inside `Contents/Resources`.

- [x] **Step 3: Write the review**

Write `docs/security/v1-review.md` covering, with the actual evidence from steps 1–2:

```markdown
# V1 Security Review — Companion App

Reviewed: <date>, commit <hash>.

## IPC commands
One row per `#[tauri::command]`: name, inputs and whether any input is
attacker-influenceable, what it can do, verdict.

## CSP and capabilities
The configured CSP string, the `main` capability's permission list, and
whether each permission is needed.

## URL / app-launch policy
What (if anything) can open external URLs or spawn processes. Expected
result: nothing — no shell/opener plugin is configured; record the grep proof.

## Logs
Whether any log line can contain secrets, tokens, or personal data
(calendar contents count as personal data — check what eprintln!
persistence/runtime paths emit).

## Config persistence
Path, file mode observed, and whether the directory is user-only.

## Release DMG contents
The observed bundle contents and signature state.

## Findings
| # | Severity | Finding | Disposition (fixed in <commit> / accepted because …) |
|---|---|---|---|
```

Every finding must end fixed or reasoned — none left open.

- [x] **Step 4: Fix what the review forces, verify, commit**

Apply any forced fixes with their own focused commits first (e.g. `fix: redact calendar summaries from persistence warnings`), each verified by `cargo test --workspace --locked`. Then:

```sh
git add docs/security/v1-review.md
git commit -m "docs: V1 security review — IPC, CSP, capabilities, logs, persistence, DMG"
git push origin main
```

---

### Task 6: Hands-on install matrix (human-in-the-loop)

**Files:**
- Modify: this plan file (append the results table to this task)

**Interfaces:**
- Consumes: the CI DMG from Task 3 (`/tmp/ci-dmg-check/*.dmg`) or a fresh `bunx tauri build` bundle.
- Produces: a recorded pass/fail per matrix row; Task 7 cites it.

This task requires the user at the machine for the tray/GUI observations. Automatable steps (install, config swaps, process checks) run via shell; observation steps are questions to the user. Back up the real config first; restore it at the end.

- [x] **Step 1: Back up the live config**

```sh
cp ~/Library/Application\ Support/io.deskmate.companion/config.json /tmp/deskmate-config-backup.json
```

- [x] **Step 2: Clean install + first run**

Quit any running Deskmate instance (tray → Quit; the user does this). Then:

```sh
rm -rf /Applications/Deskmate.app
rm -rf ~/Library/Application\ Support/io.deskmate.companion
hdiutil attach /tmp/ci-dmg-check/*.dmg -nobrowse -mountpoint /tmp/deskmate-dmg
cp -R /tmp/deskmate-dmg/Deskmate.app /Applications/
hdiutil detach /tmp/deskmate-dmg
open -a Deskmate
```

User observes: tray icon appears; no settings window auto-opens (window is configured hidden); first-run guidance appears when settings are opened; with the board attached over USB, the device leaves standalone clock and shows the active playlist.

- [x] **Step 3: Upgrade over an M3-era config**

```sh
osascript -e 'quit app "Deskmate"'
cp companion/crates/app-core/tests/fixtures/released-m3-v1.json ~/Library/Application\ Support/io.deskmate.companion/config.json
open -a Deskmate
```

User observes: app starts cleanly, settings show the migrated card library/playlist (v1→v4 migration), no "last working settings" mislabel. Then check the file was rewritten as v4:

```sh
jq '.version' ~/Library/Application\ Support/io.deskmate.companion/config.json
```

(If the app only persists on change, make any settings change first, then re-check.)

- [x] **Step 4: Single instance, autostart, tray behaviors**

```sh
open -a Deskmate; open -a Deskmate; pgrep -fl Deskmate | grep -c "Deskmate.app" 
```

Expected: one app process. User then exercises: tray → Autostart on, log out/in (or `osascript -e 'tell application "System Events" to get the name of every login item'` to confirm registration), autostart off again; settings window close leaves tray resident; tray → Quit exits fully (`pgrep` empty).

- [x] **Step 5: Offline use + corrupt-config recovery**

Unplug the device: user observes tray shows offline state, app stays healthy, board falls back to standalone clock. Replug: playlist resumes and any held alert replays. Then corrupt the config:

```sh
osascript -e 'quit app "Deskmate"'
printf 'not json' > ~/Library/Application\ Support/io.deskmate.companion/config.json
open -a Deskmate
```

User observes: app starts on defaults, the failure is surfaced as the typed validation failure (never "your last working settings"), and saving from settings recovers a valid file.

- [x] **Step 6: Uninstall + restore**

```sh
osascript -e 'quit app "Deskmate"'
rm -rf /Applications/Deskmate.app
mkdir -p ~/Library/Application\ Support/io.deskmate.companion
cp /tmp/deskmate-config-backup.json ~/Library/Application\ Support/io.deskmate.companion/config.json
```

Confirm no login item remains after uninstall if autostart was left on (`osascript` login-items check again). Reinstall the app afterwards if the user wants to keep daily-driving it (they do — repeat step 2's install lines, config already restored).

- [x] **Step 7: Record results and commit**

Append to this task a results table — one row per matrix item (clean install, first run, M3 upgrade, single instance, autostart on/off, close-to-tray, quit, offline, corrupt recovery, uninstall) with observed result and date. Then:

```sh
git add docs/superpowers/plans/2026-08-15-deskmate-v1-packaging-hardening.md
git commit -m "docs: V1 install matrix results"
git push origin main
```

#### Results (recorded 2026-08-17)

Install/GUI observations by the user at the machine; panel observations via the
webcam harness (`tools/hwcam/`, commissioned 2026-08-15 — see board-notes).
Build under test: CI DMG `Deskmate_1.0.0_aarch64.dmg` from commit `fc3750b`.

| Matrix item | Result | Date | Notes |
|---|---|---|---|
| Clean install (DMG → /Applications) | PASS | 2026-08-15 | Fresh config dir created `0700`, file `0600` (live verification of `ef7df62`) |
| First run | PASS | 2026-08-15 | **Defect found & fixed:** settings window auto-opened on *every* launch (unconditional `show_settings` since the M3 checkpoint). User-approved behavior change: auto-open on first run only (`fc3750b`, TDD). Re-verified on the fixed CI build: window auto-opens once on first run, first-run panel ("Make the display yours") visible, tray icon present, board shows active playlist |
| Upgrade over M3-era config (v1→v4) | PASS | 2026-08-15 | Tray-only launch (fix's negative case), migrated card library/playlist shown, no mislabel; file rewritten as `schema_version` 4 on save (plan's `jq '.version'` check corrected — the field is `schema_version`) |
| Single instance | PASS | 2026-08-15 | 1 process after double `open` (plan's `pgrep \| grep -c` self-match artifact noted; verified with `pgrep -x`) |
| Autostart on/off | PASS | 2026-08-15 | ON: `~/Library/LaunchAgents/Deskmate.plist` created, pref persisted; OFF: plist removed, pref false. One unreproduced anomaly: the first ON toggle silently failed (no plist, checkbox reverted; error path is stderr-only). Did not reproduce across three later toggles |
| Close-to-tray | PASS | 2026-08-15 | Window close leaves tray resident |
| Quit | PASS | 2026-08-15 | Tray → Quit exits fully (`pgrep` empty) |
| Offline (USB unplugged) | PASS | 2026-08-15 | Webcam: board fell back to standalone clock with correct local time and "Connect deskmate app" hint; tray showed offline, app healthy (user) |
| Replug / resume | PASS | 2026-08-17 | Resume observed via app relaunch after a two-day gap with the cable back (autostart was deliberately off, so no auto-start). Live replug-while-running re-adoption was separately verified 2026-08-15 (alert-replay session, board-notes). No alert was pending during this matrix run |
| Corrupt-config recovery | PASS | 2026-08-17 | Typed banner: "invalid config JSON: expected ident at line 1 column 2… unreadable file was left untouched" — **no** "last working settings" mislabel. App ran on defaults (UTC, landscape). Save & apply rewrote a valid v4 file (`0600`); panel recovered to correct orientation |
| Uninstall + restore | PASS | 2026-08-17 | No login item or LaunchAgent residue; real (schema v3) config restored and migrates on load; app reinstalled from the same DMG for daily driving |

---

### Task 7: Closeout — roadmap and CLAUDE.md

**Files:**
- Modify: `docs/superpowers/plans/2026-08-03-deskmate-roadmap.md` (the V1 row)
- Modify: `CLAUDE.md` (Current state)

**Interfaces:**
- Consumes: completed Tasks 1–6.

- [x] **Step 1: Update the roadmap V1 row**

In the V1 row, replace the "Remaining before V1 exit" clause: the CO5300 even-window check closed 2026-08-15 (already true), and the packaging/hardening item is now delivered per this plan — name the CI workflow, the two security docs, and the install-matrix result. State that V1-exit *items* are all closed and that declaring V1 exit (tag, V2 brainstorm) awaits explicit user authorization. Add this plan's filename to the V1 row's Plan column.

- [x] **Step 2: Update CLAUDE.md Current state**

Add a short paragraph: packaging/hardening delivered (private `cosmicsymmetry/deskmate` remote, `ci` workflow green with DMG artifact, advisories triaged in `docs/security/advisories.md`, security review in `docs/security/v1-review.md`, install matrix passed <date>); version is 1.0.0; audits are CI-non-blocking with the triage doc as the blocking record; no tags exist.

- [x] **Step 3: Verify and commit**

Re-read both edited sections for contradictions with this plan's actual outcomes (especially: do not claim anything Task 6 recorded as failed/waived as passed). Then:

```sh
git add docs/superpowers/plans/2026-08-03-deskmate-roadmap.md CLAUDE.md
git commit -m "docs: packaging/hardening delivered; all V1-exit items closed"
git push origin main
gh run watch --repo cosmicsymmetry/deskmate --exit-status
```

Expected: final CI run green.
