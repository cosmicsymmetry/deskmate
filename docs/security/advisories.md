# Dependency Advisory Triage — V1

Audited: 2026-08-15, cargo-audit 0.22.2, bun 1.3.8.

**Status (updated 2026-09-18): historical V1 triage.** The accepted Rust dependency
exceptions below were resolved when the Tauri companion was deleted. The current
`companion/Cargo.lock` contains no `tauri` entries and none of the crates covered by
those exceptions. This document preserves the disposition of the audit; it is not a
fresh audit of the current dependency graph.

## Resolved

| Advisory | Crate/package | Action |
|---|---|---|
| GHSA-2v37-7h3g-55p8 | `nanoid` | Added the Bun override `"nanoid": "^3.3.18"` and regenerated `bun.lock`, resolving `vite -> postcss -> nanoid` to 3.3.18. The follow-up `bun audit` reported “No vulnerabilities found”; format, lint, typecheck, and all 80 frontend tests passed. |
| RUSTSEC-2024-0370, RUSTSEC-2024-0411–0420, RUSTSEC-2024-0429, RUSTSEC-2025-0075, RUSTSEC-2025-0080–0081, RUSTSEC-2025-0098, RUSTSEC-2025-0100 | Tauri/GTK3/GLib/`unic-*` transitive crates | Resolved on 2026-09-18 by deletion of the Tauri companion and its dependency graph. A lockfile check found zero `tauri` entries and none of `atk`, `atk-sys`, `gdk`, `gdk-sys`, `gdkwayland-sys`, `gdkx11`, `gdkx11-sys`, `gtk`, `gtk-sys`, `gtk3-macros`, `glib`, `proc-macro-error`, `unic-char-property`, `unic-char-range`, `unic-common`, `unic-ucd-ident`, or `unic-ucd-version`. |

## First-party findings (not dependency advisories)

Everything above is dependency triage from the historical V1 audit. This section is the
index of defects found in **Deskmate's own code**. Full analysis for both entries below lives in
`docs/security/stage5-plugin-upload-risk-review.md` (R2); they were found while reviewing
stage 5's threat model. The durable-image finding still applies to Picture; the plugin
egress finding is retained as history after that entire path was removed in schema v9.

| Found | Where | Finding | Exploitable today? | Status |
|---|---|---|---|---|
| 2026-09-09 | `firmware/main/ui/scene_view.c:780` (`build_image`) | The durable image-asset path validates the asset kind, and that the blob is longer than an `lv_image_header_t` — but never that the header's own `w * h * bpp` fits inside `data_size`. The node's scene bounds limit the box that is *drawn*, not the buffer LVGL *walks*, so an undersized payload with an oversized header is an out-of-bounds read. The **volatile** raster path is not affected: `volatile_asset_store.c:104-124` pins `total_length` to exactly one canonical frame. | **No.** Durable assets reach the device only from the server that owns it, and every curated blob is produced by the canonical encoder. It is a missing invariant, not a live vulnerability. | **Open, deliberately deferred.** The fix is firmware C, so it must ride a firmware wave and be re-verified with an on-board OTA download check (this repo has twice lost days to memory-layout shifts with every test green). Fixing it in isolation would create exactly the hardware debt it is meant to avoid. It also violates CLAUDE.md's own standing rule — "treat all bytes received from the host as untrusted: bound lengths and counts" — so it should not wait indefinitely. |
| 2026-09-09 | Retired `server::egress` + `plugin::manifest` path | A plugin's declared source URL was pinned to `https` at parse time, but every redirect hop was re-validated through `egress_guard`, which accepted `http` as well. An `https` source could therefore redirect the fetch into cleartext. | **No longer reachable.** Manifest fetches and all server egress were removed in schema v9. | **RETIRED 2026-09-11.** The downgrade was fixed on 2026-09-09; the path and its regression test were subsequently deleted with manifest plugins. |
