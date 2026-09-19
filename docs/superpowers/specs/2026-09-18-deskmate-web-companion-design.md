# Deskmate web companion — design

Status: delivered on 2026-09-18 after owner approval (direct direction, `/goal`). The
migration language below records the change; the web companion is now the current system.

## 1. Why

The owner's stated problem was iteration speed: the companion surface had been a second
distribution boundary, so every UI change required rebuilding, signing and relaunching a
native bundle. The browser SPA removes that boundary and can be exercised by the same
browser automation used during development. It is served by the Rust server it talks to;
`d51e554` made an image source describe its own settings and `3616af0` made the add menu
build itself from `/v1/faces`, so a new face needs no SPA change.

**This design does not claim to fix deploy speed.** Serving the UI from the server makes
a CSS change cost a file copy, not a Rust build -- but only because §5 deliberately
serves the SPA from a directory rather than embedding it in the binary. Rust-side change
(a new face, a new endpoint) costs exactly what it costs today.

## 2. Scope

**In:** every companion surface for a *networked* device -- the card loop, the
card editor, the preview, image sources and their face settings, preferences, and the
settings sheet's device/link/update disclosure.

**Out, and deliberately:**

- **Direct USB ownership.** A browser tab is not a daemon: it dies when closed, is
  throttled in the background, and has no autostart. This is not a Web Serial
  limitation -- it is a process-lifetime one.
- **Cable operations** (`provision_device`, `factory_reset_device`). `CLAUDE.md` already
  states provisioning is a cable operation by design. `crates/deskmate-cli` performs all
  five of them today and remains the supported path.
- **Autostart**, and the `secure_file.rs` token-at-rest story, which the server's own
  operator session replaces.

Web Serial makes both of the above *possible* later (`crates/protocol` has zero
dependencies and so compiles to `wasm32-unknown-unknown` unchanged). It is out of scope
here and gets its own spec if wanted.

## 3. Architecture

```
Browser (Chrome)              Cloudflare → Caddy               deskmate-server (:8443)
  SPA  ──fetch/SSE──►  /             ──no edge auth──►          static  ← DESKMATE_WEB_DIR
                       /assets/*     ──no edge auth──►
                       /v1/app/*     ──no edge auth──►          app_api  ← operator cookie
                       /v1/manage/*  ──no edge auth──►          manage   ← operator cookie
  board ──wss────────► /v1/device/*  ──no edge auth──►          device bearer
  producer ─POST─────► /v1/images/*  ──no edge auth──►          producer bearer
  OTA  ──GET─────────► /v1/firmware/* ─no edge auth──►          unauthenticated by design
```

The owner removed the short-lived Caddy `basic_auth` gate on 2026-09-18: the app's
operator session is the only browser gate, and the login form and JS bundle are public.
The split by path still matters if an edge gate is ever restored. The board and producers
send their own `Authorization` header, which `basic_auth` would consume, so a host-wide
gate breaks the device link. Any future edge gate must cover only `/`, `/assets/*`,
`/v1/app/*`, and `/v1/manage/*`; device, image, and firmware authentication stays as
shown above.

### 3.1 The server gate and any future edge gate

The current deployment has no Caddy authentication gate. Protected browser data routes
use the server's `OperatorAuthenticated` session because the server binds
`192.168.8.20:8443` and is reachable on the LAN around Caddy (`~/homelab/CLAUDE.md:110`
-- it cannot bind loopback, Caddy is a bridge container). Trusting a Caddy-set header
instead would be trusting a header any LAN client can set.

If path-scoped Caddy auth returns, it can coexist with the server gate because Caddy
would use `Authorization: Basic` while the server uses a cookie. It must not expand to
the host-wide paths named above. `SESSION_TTL` is 30 days so the app login is not a
daily ritual.

### 3.2 The snapshot, online and offline

`AppSnapshot` is what the whole UI renders from. The server can already produce it:
`LiveLink` holds an `Arc<RuntimeHandle>` and `RuntimeHandle::snapshot()` exists.

But **the board is normally powered off** (`CLAUDE.md`), so the offline path is the
common one, not the edge case:

- device linked → `runtime.snapshot()`, unchanged.
- not linked → synthesize: the stored `AppConfig` from `DeviceConfigStores`, a
  `DeviceSnapshot` in `ConnectionState::Disconnected`, empty pomodoro/card-data/error
  vectors, and the store's own `DeviceConfigStatus` as `PersistenceState`.

The synthesized snapshot is honest about what it is: it reports disconnected rather than
inventing device state. The UI already renders that state -- it is the `offline`
scenario the mock harness has had all along.

## 4. Server: the `app_api` module

`crates/server/src/app_api/`, mounted under `/v1/app`, keeps protected data routes behind
`OperatorAuthenticated`. `POST /v1/app/session` trades the admin token for the cookie,
and `DELETE /v1/app/session` only expires the caller's cookie.

| Route | SPA operation | Notes |
|---|---|---|
| `GET /v1/app/devices` | List devices | ids + link state; the SPA picks one |
| `GET /v1/app/{id}/snapshot` | Read current state | §3.2 |
| `GET /v1/app/{id}/events` | Subscribe to state | SSE, snapshot per event |
| `POST /v1/app/{id}/config/validate` | Validate draft | `AppConfig::compile` |
| `PUT /v1/app/{id}/config` | Save and apply config | shares `put_config`'s body |
| `POST /v1/app/{id}/preview` | Render preview | `lvgl-sim`, §4.1 |
| `POST /v1/app/{id}/pomodoro` | Control pomodoro | live runtime only |

Image sources and faces are **not** duplicated here: `/v1/images` and `/v1/faces` already
exist and already carry the shapes the app wants. Browser lifecycle calls use the same
operator session; producer frame ingestion keeps its producer bearer. The SPA does not
retain the admin token after exchanging it for the cookie.

Provisioning and factory reset remain cable operations handled by `crates/deskmate-cli`;
the SPA has no local-ownership or autostart controls. Resuming delivery is a config write
(`paused: false`), preserving the one-off "Resume sending" escape hatch.

### 4.1 Preview

`POST /v1/app/{id}/preview` returns the `PreviewFrame` consumed by the SPA. The server's
`lvgl-sim` dependency is a `cc` build of LVGL, which compiles on Linux in CI alongside
the server.

`crates/lvgl-sim/tests/preview_path.rs` stays the honest check, and
`CLAUDE.md`'s rule holds unchanged: **one renderer**. Nothing here adds a second.

### 4.2 The type contract must not be lost

The TS↔Rust fixture generator lives beside the Rust API DTOs in
`crates/server/src/app_api/contract.rs`. Its test writes
`apps/deskmate/src/lib/types.contract.ts` and asserts the checked file matches byte for
byte, so drift between the JSON shape and the frontend contract fails the server test.

## 5. Serving the SPA

`DESKMATE_WEB_DIR` (default: none -- the routes 404 and the server behaves exactly as it
does today) points at a directory of built assets. `GET /` and any unmatched non-`/v1`
path serve `index.html`; `/assets/*` serves hashed files.

**Serving from a directory rather than embedding in the binary is the deliberate
choice**, and it is the one thing in this design that touches the owner's stated pain:
a UI change becomes `bun run build` + rsync + nothing, with no Rust compile and no
`systemctl restart`. Embedding would have made every CSS tweak a 35-minute cold build.

## 6. Frontend

`src/lib/backend.ts` is the component-facing boundary and exports the HTTP client in
`src/lib/backendClient.ts`. Requests use `fetch`, while state updates arrive through an
`EventSource`. Under `VITE_DESKMATE_MOCK=1`, `vite.config.ts` aliases that client to
`src/dev/backendClient.ts`, keeping the mock harness as the offline development path.

Removed from the UI, following §2: the provisioning form, factory reset, the local-tier
ownership choice, and the autostart toggle. `SettingsSheet` remains the product's only
disclosure, per `CLAUDE.md`; it loses the controls whose transport no longer exists and
keeps device identity, link state, Wi-Fi/IP, update state and preferences.

## 7. What this does not touch

`CURRENT_SCHEMA_VERSION` stays 10. The wire stays protocol v2. Firmware statics are
untouched, so no flash and no OTA re-verification. **None of the three expensive
boundaries is crossed**, which is what makes this a days-long change rather than a
milestone.

## 8. Testing

- `app_api` gets integration tests in the shape of `crates/server/tests/image_routes.rs`
  (real loopback binds, real router).
- The moved contract test keeps `types.contract.ts` generated and asserted.
- `crates/lvgl-sim/tests/preview_path.rs` keeps proving the preview path.
- The frontend's `bun test` suite survives the module swap; `src/dev/mockBackend.ts`
  keeps every scenario renderable without a server.
- End-to-end: the real SPA driven in Chrome against the deployed server.

## 9. Deploy

1. `bun run build` in `apps/deskmate` → `dist/`.
2. rsync `dist/` to `/var/lib/deskmate/web` on docker-vm.
3. `DESKMATE_WEB_DIR=/var/lib/deskmate/web` in `/etc/deskmate/server.env`.
4. Rebuild and install the server binary by the existing runbook (rsync to
   `~/deskmate-build`, throwaway `rust:1.98-bookworm`, `sudo install`, restart).
5. Leave Caddy without an authentication gate, per the owner's 2026-09-18 decision. If
   that decision changes, the prerequisite is the path-scoped design in §3; never add a
   host-wide `basic_auth` block.

Order matters once: the binary must be installed before `DESKMATE_WEB_DIR` means
anything. No Caddy change is part of the current deploy.
