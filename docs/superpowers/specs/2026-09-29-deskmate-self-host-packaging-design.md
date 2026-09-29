# Self-host packaging

Track A / ROD-6, 2026-09-29. Implementation and verification record.

## Boundary and choice

Use the completed ROD-4 issue document, revision 3 (resource limits and public/self-host
boundary), which supersedes the unmerged commercial draft at `81f521b`. This work ships
the existing public `SelfHosted` runtime. It does not implement the proposed hosted
allowances, accounting supervisor, paid policy or billing. No commercial decision is
needed to package the public server.

Considered a multi-stage Docker image and a native bundle. Choose the native bundle:
it exercises the same public Rust/Bun inputs, works without an installed container
engine or owner's tunnel, and keeps web/faces directories independently replaceable.
The cost is a Rust/C/Bun toolchain on the build host, target-specific native modules,
and an explicit optional systemd/Caddy service recipe. No unattended installer edits
system services, buys infrastructure, enables third-party services or provisions a panel.

The build script fetches only the two preview C component subsets from the public
Espressif registry. Archive timestamps vary, so content is pinned as SHA-256 over
sorted UTF-8 relative paths, NUL, and each file's SHA-256 digest. Source bytes were
compared against installed lockfile-resolved firmware components with zero mismatches:
1,134 LVGL files and 26 CBOR files. Component version drift or content drift fails
closed. This is a server-build input operation, not a firmware/static/wire change.

Bundle: native server + cable CLI, browser assets, faces source/plugins/fonts and
platform-native runtime dependencies. Bun stays an explicit installed dependency.
Instance: mode-0700 directory, mode-0600 generated environment file, persistent config
store and empty firmware directory. The initializer refuses existing instances.
The runner parses environment data without executing shell text, ignores inherited
Deskmate overrides, and execs the server so SIGTERM reaches graceful shutdown.

`DESKMATE_WEB_DIR` still reads the SPA per request. The faces package remains an
external directory read at use, with catalog refresh. The private hosted crate never
appears in public Cargo resolution. Local plugins are available; the managed reviewed
directory and paid surfaces are absent. Frozen contracts remain unchanged.

## Acceptance checks

- [x] Clean checkout: public preview downloads, locked Rust build, locked web/faces
  installation and complete bundle with fonts; no pre-existing managed components.
- [x] Initialize private state, run real server, Chrome displays first-run setup and
  accepts setup code, then shows Add your panel without external credentials.
- [x] Restart preserves the owner and does not return to setup; reinitialization refuses.
- [x] Bundled local plugin renders an offline fixture, exercising native raster/fonts.
- [x] Full Rust/web/faces gates, docs/link review and diff checks pass.
- [ ] PR created, CI confirmed green, merge through PR.

Exact evidence and deviations belong below as checks run. A local setup check proves
neither Linux service installation nor physical-panel/OTA behavior. No hardware is
required or authorized for this packaging validation.

## Work record

- Created `track-a/self-host-packaging` in the designated Track A worktree; merged
  locally available `origin/main` to include shipped plugin runtime while retaining
  the shared main's frame-exporter commits. No other worktree modified.
- Initial clean build underway from a local Git clone, not the dirty working directory.
  Runtime startup hook resets PATH; validation invokes Bash without that hook and
  explicitly names the existing Rust/Bun toolchains. Product instructions assume an
  ordinary operator shell and use the documented PATH setup.
- GitHub connection reports ready but repository lookup returns 404. Managed Git wrapper
  also reports incomplete identity. PR/CI steps remain outstanding pending access.

- Verification complete locally: see the exact [guide record](../../self-host.md#verification-record).
  829 Rust tests passed / one ignored fixture printer; doctest command passed (zero
  tests); web 216 and faces 425 passed. All specified format/type/lint/build gates passed.
  First-run setup, restart persistence and log-mail sign-in passed in real Chrome;
  PNG fixture rendered through the installed bundle and was visually inspected.
- Initial web typecheck could not find `tsc` because the agent runtime's Bash startup
  hook replaced Bun's PATH; rerun without that hook passed. No source change was made
  to hide the environment failure. Initial bundle omitted fonts; corrected before
  acceptance in `6473074`. No schema, wire, firmware or production service mutation.
- Outstanding: restore repository access, fetch/reconcile current main, create PR,
  confirm CI and merge. `gh run list` exited 4 (no managed credentials); connected
  GitHub repository lookup returned 404. Unblock owner: Chief Claude must restore
  Vault's repository scope and managed Git identity. Local branch remains intact.

- Continuation 2026-09-29: repository access repair verified outside the network
  sandbox using the runtime-provided GitHub CLI credential. `git fetch origin`
  succeeded; `HEAD..origin/main` is empty, so no integration change is needed.
  The branch is clean at `1d0816b` before these status-only documentation edits;
  prior full local gates and clean-checkout/browser evidence still apply.
  Historical access-blocker notes above are resolved; next is branch CI and PR merge.
