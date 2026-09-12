# Dependency Advisory Triage — V1

Audited: 2026-08-15, cargo-audit 0.22.2, bun 1.3.8.
CI runs both audits non-blocking; this document is the blocking triage.

## Resolved

| Advisory | Crate/package | Action |
|---|---|---|
| GHSA-2v37-7h3g-55p8 | `nanoid` | Added the Bun override `"nanoid": "^3.3.18"` and regenerated `bun.lock`, resolving `vite -> postcss -> nanoid` to 3.3.18. The follow-up `bun audit` reported “No vulnerabilities found”; format, lint, typecheck, and all 80 frontend tests passed. |

## Exceptions (accepted, with re-review trigger)

| Advisory | Crate/package | Owner | Impact on Deskmate | Upgrade trigger |
|---|---|---|---|---|
| RUSTSEC-2024-0413 | `atk` 0.18.2 | Rodion | Unmaintained crate in Tauri's Linux-only GTK3 path (`gtk -> atk`). It is absent from the macOS dependency graph and is neither shipped nor executed by the macOS companion. | Resolve before supporting a Linux build, or when a pinned Tauri release removes/replaces GTK3. |
| RUSTSEC-2024-0416 | `atk-sys` 0.18.2 | Rodion | Unmaintained crate pulled through Tauri's Linux-only GTK3 path (`gtk -> atk -> atk-sys` and `gtk -> gtk-sys -> atk-sys`). It is absent from the macOS dependency graph. | Resolve before supporting a Linux build, or when a pinned Tauri release removes/replaces GTK3. |
| RUSTSEC-2024-0412 | `gdk` 0.18.2 | Rodion | Unmaintained crate pulled by Tauri's Linux-only GTK/WebKit stack (`gtk -> gdk` and `webkit2gtk -> gdk`). It is absent from the macOS dependency graph. | Resolve before supporting a Linux build, or when a pinned Tauri/Wry release removes/replaces GTK3. |
| RUSTSEC-2024-0418 | `gdk-sys` 0.18.2 | Rodion | Unmaintained crate pulled by Tauri's Linux-only GTK/WebKit stack through `gdk`, `gtk-sys`, and `webkit2gtk-sys`. It is absent from the macOS dependency graph. | Resolve before supporting a Linux build, or when a pinned Tauri/Wry release removes/replaces GTK3. |
| RUSTSEC-2024-0411 | `gdkwayland-sys` 0.18.2 | Rodion | Unmaintained Linux window-system binding pulled by `tauri -> tauri-runtime-wry -> tao -> gdkwayland-sys`. It is absent from the macOS dependency graph. | Resolve before supporting a Linux/Wayland build, or when the pinned Tauri/Tao stack replaces GTK3. |
| RUSTSEC-2024-0417 | `gdkx11` 0.18.2 | Rodion | Unmaintained Linux X11 binding pulled by `tauri -> tauri-runtime-wry -> wry -> gdkx11`. It is absent from the macOS dependency graph. | Resolve before supporting a Linux/X11 build, or when the pinned Tauri/Wry stack replaces GTK3. |
| RUSTSEC-2024-0414 | `gdkx11-sys` 0.18.2 | Rodion | Unmaintained Linux X11 binding pulled through `tauri-runtime-wry -> tao -> gdkx11-sys` and `wry -> gdkx11 -> gdkx11-sys`. It is absent from the macOS dependency graph. | Resolve before supporting a Linux/X11 build, or when the pinned Tauri/Tao/Wry stack replaces GTK3. |
| RUSTSEC-2024-0415 | `gtk` 0.18.2 | Rodion | Unmaintained GTK3 binding included by Tauri's target-specific Linux backend (`tauri`, `tauri-runtime`, `tauri-runtime-wry`, Tao, and Wry). It is absent from the macOS dependency graph. | Resolve before supporting a Linux build, or when a pinned Tauri release moves off GTK3. |
| RUSTSEC-2024-0420 | `gtk-sys` 0.18.2 | Rodion | Unmaintained GTK3 FFI binding pulled through Linux-only `gtk` and WebKit/dialog/tray dependencies. It is absent from the macOS dependency graph. | Resolve before supporting a Linux build, or when a pinned Tauri release moves off GTK3. |
| RUSTSEC-2024-0419 | `gtk3-macros` 0.18.2 | Rodion | Unmaintained macro crate pulled by the Linux-only path `tauri -> gtk -> gtk3-macros`. It is absent from the macOS dependency graph and is not used to build the macOS companion. | Resolve before supporting a Linux build, or when a pinned Tauri release moves off GTK3. |
| RUSTSEC-2024-0370 | `proc-macro-error` 1.0.4 | Rodion | Unmaintained macro helper pulled only through the GTK3 stack (`glib -> glib-macros -> proc-macro-error` and `gtk -> gtk3-macros -> proc-macro-error`). A macOS-target reverse tree prints nothing, so it is not used to build or run the companion. | Resolve before supporting a Linux build, or when the pinned GTK/GLib transitive stack removes it. |
| RUSTSEC-2025-0081 | `unic-char-property` 0.9.0 | Rodion | Unmaintained, with no reported vulnerability. It is a macOS runtime transitive dependency through `tauri-utils -> urlpattern -> unic-ucd-ident -> unic-char-property`, used for URL-pattern Unicode identifier classification; Deskmate does not depend on it directly. | Re-review on every Tauri/`tauri-utils` upgrade; replace when `urlpattern` adopts a maintained implementation, or immediately if a security advisory appears. |
| RUSTSEC-2025-0075 | `unic-char-range` 0.9.0 | Rodion | Unmaintained, with no reported vulnerability. It is a macOS runtime transitive dependency through `tauri-utils -> urlpattern -> unic-ucd-ident`, directly and through `unic-char-property`; Deskmate does not depend on it directly. | Re-review on every Tauri/`tauri-utils` upgrade; replace when `urlpattern` adopts a maintained implementation, or immediately if a security advisory appears. |
| RUSTSEC-2025-0080 | `unic-common` 0.9.0 | Rodion | Unmaintained, with no reported vulnerability. It is a macOS runtime transitive dependency through `tauri-utils -> urlpattern -> unic-ucd-ident -> unic-ucd-version -> unic-common`; Deskmate does not depend on it directly. | Re-review on every Tauri/`tauri-utils` upgrade; replace when `urlpattern` adopts a maintained implementation, or immediately if a security advisory appears. |
| RUSTSEC-2025-0100 | `unic-ucd-ident` 0.9.0 | Rodion | Unmaintained, with no reported vulnerability. It is pulled into the macOS runtime by `tauri-utils -> urlpattern -> unic-ucd-ident` for URL-pattern identifier handling; Deskmate does not depend on it directly. | Re-review on every Tauri/`tauri-utils` upgrade; replace when `urlpattern` adopts a maintained implementation, or immediately if a security advisory appears. |
| RUSTSEC-2025-0098 | `unic-ucd-version` 0.9.0 | Rodion | Unmaintained, with no reported vulnerability. It is a macOS runtime transitive dependency through `tauri-utils -> urlpattern -> unic-ucd-ident -> unic-ucd-version`; Deskmate does not depend on it directly. | Re-review on every Tauri/`tauri-utils` upgrade; replace when `urlpattern` adopts a maintained implementation, or immediately if a security advisory appears. |
| RUSTSEC-2024-0429 | `glib` 0.18.5 | Rodion | The advisory reports unsound `VariantStrIter` iterator implementations, but this GLib version is pulled only by Tauri's Linux GTK/WebKit stack. `cargo tree` confirms it is absent from the macOS graph, so the affected code is not shipped or executed. | Resolve before supporting a Linux build, or when a pinned Tauri release upgrades/replaces the affected GLib stack. |

## GTK3 / unmaintained paths

The recorded macOS-target reverse dependency checks produced the following output.

`cargo tree --target aarch64-apple-darwin -i gtk`:

```text
warning: nothing to print.

To find dependencies that require specific target platforms, try to use option `--target all` first, and then narrow your search scope accordingly.
```

`cargo tree --target aarch64-apple-darwin -i glib`:

```text
warning: nothing to print.

To find dependencies that require specific target platforms, try to use option `--target all` first, and then narrow your search scope accordingly.
```

The GTK3/GLib crates remain in `Cargo.lock` because Tauri, Tao, Wry, WebKitGTK, dialog, and tray dependencies use them on Linux. Neither `gtk` nor `glib` is in the `aarch64-apple-darwin` dependency graph, so those warnings do not affect the macOS-only V1 artifact. They must be resolved or re-triaged before Deskmate adds Linux support.

## First-party findings (not dependency advisories)

Everything above is dependency triage from the V1 audit. This section is the index of
defects found in **Deskmate's own code**, so this file remains the one blocking triage
record CLAUDE.md points at. Full analysis for both entries below lives in
`docs/security/stage5-plugin-upload-risk-review.md` (R2); they were found while reviewing
stage 5's threat model. The durable-image finding still applies to Picture; the plugin
egress finding is retained as history after that entire path was removed in schema v9.

| Found | Where | Finding | Exploitable today? | Status |
|---|---|---|---|---|
| 2026-09-09 | `firmware/main/ui/scene_view.c:780` (`build_image`) | The durable image-asset path validates the asset kind, and that the blob is longer than an `lv_image_header_t` — but never that the header's own `w * h * bpp` fits inside `data_size`. The node's scene bounds limit the box that is *drawn*, not the buffer LVGL *walks*, so an undersized payload with an oversized header is an out-of-bounds read. The **volatile** raster path is not affected: `volatile_asset_store.c:104-124` pins `total_length` to exactly one canonical frame. | **No.** Durable assets reach the device only from the server that owns it, and every curated blob is produced by the canonical encoder. It is a missing invariant, not a live vulnerability. | **Open, deliberately deferred.** The fix is firmware C, so it must ride a firmware wave and be re-verified with an on-board OTA download check (this repo has twice lost days to memory-layout shifts with every test green). Fixing it in isolation would create exactly the hardware debt it is meant to avoid. It also violates CLAUDE.md's own standing rule — "treat all bytes received from the host as untrusted: bound lengths and counts" — so it should not wait indefinitely. |
| 2026-09-09 | Retired `server::egress` + `plugin::manifest` path | A plugin's declared source URL was pinned to `https` at parse time, but every redirect hop was re-validated through `egress_guard`, which accepted `http` as well. An `https` source could therefore redirect the fetch into cleartext. | **No longer reachable.** Manifest fetches and all server egress were removed in schema v9. | **RETIRED 2026-09-11.** The downgrade was fixed on 2026-09-09; the path and its regression test were subsequently deleted with manifest plugins. |
