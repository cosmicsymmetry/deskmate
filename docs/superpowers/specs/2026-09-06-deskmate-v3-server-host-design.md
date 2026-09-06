# Deskmate V3 — Self-Sufficient Server Host — Design

**Date:** 2026-09-06
**Status:** Draft for Rodion. Nothing here is implemented. Approach (Option A —
single-owner, self-sufficient homelab server) is decided; the design choices below need
sign-off before a plan is written.
**Prior contracts:**
- `docs/superpowers/specs/2026-08-18-deskmate-v2-networked-device-design.md` — V2. §5 (the
  server), §8 (out of scope), §10 (roadmap → this milestone). The device-facing contract
  V2 froze is the contract V3 builds behind.
- `docs/superpowers/specs/2026-08-06-deskmate-google-calendar-proposal.md` — the earlier
  calendar analysis. Written for the **Mac app** as OAuth client. V3 relocates that
  credential to the server, which inverts several of its conclusions (§5 records how).
- `docs/config/v6.md` — the frozen authoring config. `docs/protocol/v1.md` — the frozen
  wire contract.

## 0. What V3 delivers / does not

**Delivers.** A homelab server that no longer needs the Mac app to be a useful owner of a
networked device. Two things:

1. **Server-held OAuth integrations, Google Calendar first.** The owner authorizes Google
   once through a browser consent flow. The server stores the resulting refresh token
   **encrypted at rest**, auto-refreshes access tokens, and a server-side provider turns
   the owner's real calendar into the existing `calendar` card — which renders on the
   device **with the Mac app quit**, at real (minutes, not hours) freshness.
2. **A minimal web management surface.** Enough that the Mac app is no longer *required*:
   start and revoke an OAuth integration, and see device connection status and each
   integration's health. Admin-token gated, single operator.

**Does not deliver (explicitly deferred, not designed here).** User accounts and signup;
per-user multi-tenancy or data isolation; billing; the plugin-upload sandbox (that is the
scene spec's stage 5). The data model is single-owner throughout. §7 marks the one seam
where a future multi-tenant step slots in, without building it.

**Requires no firmware change.** Protocol stays v1, capabilities are untouched, and the
compiled wire config / scene a device consumes is byte-unchanged: a Google calendar
compiles to the identical `row-list` card an ICS calendar already does. V3 replaces and
extends **the server** only. (One authoring-side schema addition is proposed — see §1 and
the open question in §8; it changes no byte the device sees.)

## 1. The two foundational decisions (signed off 2026-09-06)

Everything else follows from these. Both are now decided (see §10); the recommendation and
the alternatives it was chosen over are kept here for the record. Recommendation first,
then the alternatives.

### 1.1 Secret storage — where the refresh token lives

**Recommendation: keep configs as JSON-on-disk and add ONE encrypted secrets file, keyed
by a 32-byte key read from a keyfile at startup. Do not introduce SQLite.**

The server already persists per-device config as JSON and device identities as a
digest-only JSON registry (`registry.rs`, `store.rs`), and `app-core::secure_file`
already gives atomic, `0600`, fsync-on-replace writes. Secrets are a *different kind* of
data from those two, so they get their own file rather than being smuggled into either:
`secrets.enc` next to `device-identities.json`, an encrypted blob whose plaintext is a
small JSON map of `integration_id → { provider, refresh_token, client_secret?, scopes,
obtained_at }`.

| Option | What it is | Verdict |
|---|---|---|
| **A. Plaintext file, `0600`** | Store the refresh token like any other config field, relying on filesystem perms. | **Reject.** A refresh token is long-lived read access to the owner's calendar. Perms alone lose it to a stray backup, a `tar` of `/var/lib`, an accidental commit, or a snapshot copied off the box. The project already refuses to store even device tokens in the clear (digests only); a *more* sensitive credential must not regress that. |
| **B. JSON configs + one encrypted secrets blob, key from keyfile/env** | AEAD-encrypt only the secrets file; leave configs and identities as they are. | **Recommend.** Smallest change, no new storage engine, encrypts exactly the bytes that need it. Reuses the atomic-write and permission machinery already in the tree. |
| **C. SQLite (+ SQLCipher or app-level column encryption)** | Move configs, identities and secrets into a database. | **Reject for V3.** A migration of two working JSON stores, a new dependency, and a query layer, to serve one single-tenant owner. YAGNI. SQLite earns its place at the *multi-tenant* step (§7), where per-user rows and indices start to matter — not before. |

**The encryption primitive.** A vetted AEAD, not hand-rolled: `age` (X25519 +
ChaCha20-Poly1305, passphrase or key-file recipient) or, staying inside the RustCrypto
family the crate already pulls (`sha2`), `chacha20poly1305` with a random 24-byte nonce
per write prepended to the ciphertext. Either is a single small dependency. Recommend
`age` — it is a file-format, not a construction we assemble, and its `scrypt`/`x25519`
recipients give a documented key-rotation story.

**Where the key comes from — and its threat model.** The key is 32 bytes, provided as:

- **`DESKMATE_SECRETS_KEY_FILE`** — path to a `0600` keyfile the operator creates once
  (`head -c32 /dev/urandom | base64 > key`). **Recommended.** A keyfile is not visible in
  `ps`, `/proc/<pid>/environ`, `systemctl show`, or a shell history the way an env var is,
  and it survives a `journalctl` dump of the unit's environment.
- **`DESKMATE_SECRETS_KEY`** (base64) — accepted as a fallback for containerized secrets
  managers that inject env only. Documented as second-choice for the reasons above.

The process **refuses to start** if an encrypted secrets file exists but no key is
configured (fail-closed, same posture as `DESKMATE_FIRMWARE_VERSION` in `main.rs`).

*Threat model, stated plainly so it is not overclaimed.* Encryption at rest defends the
credential against **offline** exposure: stolen disks, backups, snapshots, an accidental
`git add`, a shared VM image. It does **not** defend against a **live-server
compromise** — a process that can read the keyfile and the ciphertext can mint access
tokens, because it must, to use the calendar. That is inherent to a server that acts on
the owner's behalf while unattended, and is the accepted boundary. The keyfile living
outside the config directory (so a config-directory backup does not carry the key)
is what makes the offline story real rather than theatrical.

### 1.2 The OAuth consent + management surface

**Recommendation: extend the existing admin router with a handful of server-rendered HTML
pages. Do not build a separate web app.**

The server is already an Axum service behind the tunnel with an admin-token auth
extractor (`admin.rs`, `AdminAuthenticated`). The consent flow is intrinsically
server-side — it *is* HTTP redirects the server must terminate — so the pages belong where
the routes already are.

| Option | What it is | Verdict |
|---|---|---|
| **A. Server-rendered HTML on the admin router** | 3–4 routes returning small HTML pages (status dashboard, "connect Google" button, callback landing, revoke confirm), plus the OAuth callback route. Plain templates, no JS build. | **Recommend.** One binary, one deploy, one auth story, one place untrusted input is hardened. The consent redirect and callback have to be here regardless; the status page is a few more lines beside them. |
| **B. Separate SPA + admin JSON API** | A static-hosted front-end talking to JSON endpoints. | **Reject for V3.** A build pipeline, a second artifact to ship, CORS, and asset serving — to give one operator four buttons. The JSON API it would need is exactly today's admin API; wrapping it in an SPA is churn. Revisit only if the surface grows past a dashboard. |
| **C. Keep it in the Mac app / CLI** | Drive consent from a Tauri command or `deskmate-cli`. | **Reject.** The stated goal is that the Mac app is *not required*. The callback still has to land on the server, so this adds a client without removing a server route. |

**Auth for the surface.** Reuse the existing `DESKMATE_ADMIN_TOKEN` as the single
operator credential. The management pages are gated by the same admin extractor as
`POST /v1/devices`. A browser-friendly session (a signed, `HttpOnly`, `Secure` cookie
minted after the operator presents the admin token once on a login page) avoids pasting a
bearer token into a browser for every click; the cookie carries no privilege the admin
token does not already carry. This is the explicit stand-in for V3-deferred accounts, and
§7 marks where real per-user auth replaces it.

## 2. Architecture

```
                       Cloudflare Tunnel  →  Caddy  →  deskmate server (Axum)
                                                          │
  browser (owner) ── admin cookie ─────────────────────► │  admin + management router
                                                          │    ├─ GET  /v1/manage                (status dashboard, HTML)
                                                          │    ├─ POST /v1/integrations/google    (start consent → 302 to Google)
                                                          │    ├─ GET  /v1/integrations/google/callback  (code+state → tokens)
                                                          │    ├─ POST /v1/integrations/{id}/revoke
                                                          │    └─ (existing) /v1/devices, /config, /scene, /plugins
                                                          │
  Google OAuth / Calendar  ◄── egress-guarded HTTPS ──────┤  IntegrationStore (secrets.enc, AEAD)
   accounts.google.com                                     │  TokenManager (refresh, cache access tokens in memory)
   oauth2.googleapis.com                                   │  GoogleCalendarProvider  ──┐
   www.googleapis.com                                      │                            │ ProviderRefreshResult (Fields)
                                                           │  ServerProviderRefresher ──┘
  device (WSS, networked tier)  ◄── unchanged wire ────────┤  per-device app-core RuntimeHandle  →  calendar / row-list card
```

The new pieces are all server-crate additions behind the frozen device surface:

- **`IntegrationStore`** — owns `secrets.enc`; encrypt/decrypt, atomic replace, load at
  startup. The only component that touches refresh tokens or the client secret in the
  clear, and only in memory.
- **`TokenManager`** — exchanges an authorization code for tokens, refreshes an access
  token when it is near expiry, caches the live access token in memory (never on disk),
  and reports a typed integration health (`Connected` / `NeedsReconnect` / `Error`).
- **`GoogleCalendarProvider`** — a server-side `Provider`-shaped source that, given a live
  access token, calls the Calendar API and returns the same `Vec<Field>` the ICS calendar
  produces, so the existing `calendar` card renders it unchanged.
- **`ServerProviderRefresher`** already wraps `SystemProviderRefresher` and intercepts one
  request variant (`Plugin`) while delegating the rest (`plugin_refresher.rs`). The Google
  calendar is served by the **same seam**: the refresher intercepts a Google-sourced
  calendar request and delegates everything else. This is the established pattern, not a
  new one.

**`app-core ← plugin` (and now `app-core ← google-provider`) direction is preserved.**
`app-core` defines the *request shape* (a `CalendarSource::Google { integration_id }`
authoring variant and the `ProviderRequest` it lowers to) but holds no token store, no
`reqwest`, no OAuth. The server implements the fetch, exactly as it implements plugin
fetches today, and injects it through `RuntimeHandle::start_with_plugin_host`'s sibling
injection point. `app-core` never calls into the server.

## 3. The OAuth flow, end to end

Google is a **confidential** client here, which is the pivotal difference from the
2026-08-06 proposal. That proposal used PKCE + loopback because a *distributed desktop
binary cannot hold a client secret*. The V3 server is a single, operator-controlled
process; it *can* hold a secret. So the flow is the standard server-side authorization
code grant, with PKCE kept as defense-in-depth.

1. **Register the app (one-time, manual, by the owner).** A Google Cloud project, OAuth
   client of type *Web application*, one authorized redirect URI:
   `https://deskmate.rodi.one/v1/integrations/google/callback` (exact match, HTTPS,
   through the tunnel). Client id and client secret are handed to the server as
   configuration (client secret goes into the encrypted store, or a keyfile — see §6).
   **Publishing status is a load-bearing decision — see §8, open question 3.**
2. **Consent start.** Operator clicks "Connect Google" on `/v1/manage`. The server
   generates a random `state` and a PKCE `code_verifier`, stashes both server-side keyed
   to the operator session (short TTL, single use), and 302s to Google's auth endpoint
   with `scope=https://www.googleapis.com/auth/calendar.events.readonly`,
   `access_type=offline`, `prompt=consent` (forces a refresh token to be returned),
   `code_challenge`, and `state`.
3. **Callback.** Google redirects back with `code` and `state`. The server: verifies
   `state` matches the stashed value (CSRF); exchanges `code` + `code_verifier` +
   client secret at `oauth2.googleapis.com/token` over an **egress-guarded** HTTPS call;
   receives `access_token`, `refresh_token`, `expires_in`.
4. **Token store.** The `refresh_token` is written into `secrets.enc` via `IntegrationStore`
   (encrypt → atomic replace). The `access_token` is held **in memory only** in
   `TokenManager` with its expiry. The callback page reports success and links back to the
   dashboard.
5. **Refresh.** Before any Calendar call, `TokenManager` checks the cached access token; if
   it is missing or within a skew window of expiry, it POSTs the refresh token to the
   token endpoint (egress-guarded) and caches the new access token. Refresh tokens do not
   normally rotate for Google, so `secrets.enc` is written only at authorize and revoke,
   not on every refresh.
6. **Provider consumption.** On each refresh tick, `GoogleCalendarProvider` asks
   `TokenManager` for a live access token and calls the Calendar API (§4). The resulting
   fields flow through `ProviderRefreshResult` into the per-device runtime exactly as
   ICS fields do.

**Token lifecycle failures** all resolve to typed integration health and are covered in §5.

## 4. The Google Calendar provider

**Endpoint.** `GET https://www.googleapis.com/calendar/v3/calendars/{calendarId}/events`
with `Authorization: Bearer <access_token>`. `calendarId` defaults to `primary`; the
management UI can later let the owner pick a calendar from `calendarList.list`, but V3
ships `primary` and treats calendar choice as a follow-on (§7). Query params mirror the
ICS provider's projection: `timeMin=now`, `singleEvents=true` (expand recurrence so the
server does not reimplement RRULE — the ICS provider's `MAX_RECURRENCE_DAYS` logic is not
needed here), `orderBy=startTime`, `maxResults` capped to the card's row budget
(`providers::ics::MAX_CALENDAR_ROWS` = 5).

**Incremental sync.** Persist Google's `syncToken` per integration (in the plaintext
config side, not the secrets blob — it is not a credential) and pass it on subsequent
calls for a cheap delta. On `410 Gone`, drop the token and do a full resync. This is the
one piece of Google-specific machinery worth building; without it a 15-minute cadence
re-downloads the whole horizon each tick.

**Field shape — reuse, do not reinvent.** The card that renders this is the existing
`calendar` card on the `row-list` template. The ICS provider already produces the exact
`Vec<Field>` that template consumes (title + up to five event rows, each a formatted
"time — summary"), plus the shared stale/error footer fields. `GoogleCalendarProvider`
produces the **same field keys and the same formatting**, so the card, the scene builder,
and the device are all untouched. The provider is a new data *source* for an existing
projection, not a new projection.

**Bounding untrusted input.** Google's response is untrusted like any network body: cap
the response size (reuse `MAX_PROVIDER_RESPONSE_BYTES`), truncate summaries to the card's
field width, cap event count, and reject malformed JSON to last-good. The egress guard
(§6) bounds the fetch itself.

## 5. Data flow into an existing card, and error handling

**Authoring binding.** A calendar card names its source. Today: `CalendarSource::File` or
`::Url` (`config.rs`). V3 adds `CalendarSource::Google { integration_id }`. When the
runtime lowers a calendar card to a `ProviderRequest`, a Google source lowers to a request
the `ServerProviderRefresher` recognizes and serves from `GoogleCalendarProvider` bound to
that integration; `File`/`Url` continue to fall through to the ICS provider unchanged. The
integration is a server-side object; the card references it by id, so no credential ever
enters the config document, the wire, or the Mac app.

**Error handling — every failure maps to an existing projection plus, where the operator
must act, a distinct integration state.** The `calendar` card already has last-good +
stale-age + error semantics (`LastGood`, the stale/error footer). V3 reuses them:

| Condition | Card behaviour | Integration health (management UI) |
|---|---|---|
| Access token expired (normal) | none — refreshed transparently | `Connected` |
| Refresh fails, network/5xx/timeout | last-good events, `stale` footer | `Error(transient)`, retried with backoff |
| Refresh returns `invalid_grant` (revoked at Google, or refresh token expired) | last-good then error footer as it ages | **`NeedsReconnect`** — an actionable "Reconnect Google" prompt, distinct from a generic provider error |
| Calendar API `403`/`429` rate limit | last-good, `stale` | `Error(rate_limited)`; exponential backoff, honour `Retry-After` if present; refresh cadence is per-integration so a limit on one does not stall others |
| No integration configured for the card's `integration_id` | error footer naming the missing integration | n/a (config error, surfaced at save/validate) |

`NeedsReconnect` is the one state that is not just "stale": a revoked or expired grant
cannot self-heal, so it must be visibly different from an outage the server will retry out
of on its own. This mirrors the 2026-08-06 proposal's insistence on a distinct
"reconnect" state, relocated from the Mac's settings to the server's dashboard.

## 6. Security

- **Encryption at rest.** §1.1. Only `IntegrationStore` decrypts, only in memory; the
  access token is never persisted; the key lives outside the config directory.
- **Client secret.** It is a credential; it goes in `secrets.enc` (or a dedicated
  `0600` keyfile named by env). Not in a plaintext config, not in the repo, not in the
  admin-visible config document.
- **Redirect URI.** Exactly one, HTTPS, registered verbatim with Google. The server
  validates the incoming callback's `state` against a stashed, single-use, short-TTL
  value bound to the operator session (CSRF), and validates `code` only from the exact
  callback route. PKCE `code_verifier` is checked by Google.
- **Egress.** The OAuth token exchange, the refresh call, and the Calendar API call all go
  through the existing SSRF guard (`egress::fetch` / resolve-then-pin). Google's hosts
  (`accounts.google.com`, `oauth2.googleapis.com`, `www.googleapis.com`) are globally
  routable public names, so they **pass the allowlist naturally** — the guard is an
  allowlist of globally-routable unicast, not a per-host list, and needs no Google-specific
  exception. Routing these calls through the guard (rather than a raw `reqwest`) keeps the
  resolve-then-pin and redirect re-validation protections on the credential-bearing paths,
  which is worth the small plumbing. *Verify at build time that no Google endpoint resolves
  into a denied range in this deployment* — it does not, but the exit gate asserts it so a
  future guard change cannot silently break auth.
- **Management surface exposure.** The dashboard is public-internet-reachable through the
  tunnel; it is admin-gated (cookie minted from the admin token), rate-limited by the
  existing process-wide caps, and leaks nothing about integrations to an unauthenticated
  caller (a `/v1/manage` without a valid session is a bare 401, same posture as `auth.rs`).
- **No token readback.** Consistent with V2's NVS rule: the management UI reports
  integration *presence and health*, never token values.

## 7. Where multi-tenancy slots in (deferred, not built)

The single seam to preserve so a future multi-tenant step is additive rather than a
rewrite: **key integrations, configs, and secrets by an `owner_id`, defaulted to a single
constant `owner_id` in V3.** Concretely —

- `IntegrationStore`'s map is `integration_id → record` today; a multi-tenant version is
  `(owner_id, integration_id) → record`. Reserve the compound key shape now by threading a
  single-valued `owner_id` through, so the storage format does not have to change later.
- The admin cookie becomes a per-user session; the admin token becomes one operator's
  credential among many. `AdminAuthenticated` becomes `Authenticated { owner_id }`.
- This is also the point where **SQLite earns its place** (§1.1 option C): per-user rows,
  indices, and concurrent writers are what a database is for, and none of that is present
  at single-owner scale.

V3 builds none of this. It only avoids hard-coding assumptions (a global secrets map, a
single admin identity) that would make the above a rewrite instead of an extension.

## 8. Sub-project decomposition and suggested order

1. **Secrets-at-rest foundation.** `IntegrationStore`: AEAD encrypt/decrypt over a small
   JSON secrets map, key from keyfile/env, atomic `0600` writes reusing `secure_file`,
   fail-closed startup, load/round-trip tests. Prerequisite for everything; ships with no
   user-visible behaviour, which makes it independently testable.
2. **Generic OAuth integration framework, Google as the first identity.** `TokenManager`
   and the consent/callback/refresh/revoke routes; state + PKCE; typed integration health;
   egress-guarded token calls. Provider-agnostic in shape (so a second integration later
   is a data-source addition, not a re-architecture) but only Google is wired.
3. **Google Calendar provider + authoring binding.** `GoogleCalendarProvider` with
   incremental sync and 410 fallback; the `CalendarSource::Google` schema addition
   (authoring-side, → schema v7, no wire/firmware change); the `ServerProviderRefresher`
   interception. This is the sub-project the brief wants specced in the most depth, above.
4. **Management web surface.** `/v1/manage` dashboard (device connection status +
   per-integration health), connect/revoke buttons, the admin-cookie login. Depends on
   2 for the actions and 3 for the health it displays.
5. **(Deferred) multi-tenant foundation.** Not V3. §7 is the note that keeps it cheap.

Order is a dependency chain: 1 → 2 → 3, with 4 layered on once 2 and 3 exist. Each of
1–3 has automated coverage that does not need hardware; only the end-to-end gate (§9) does.

## 9. Verification and exit gate

### 9.1 Automated (no hardware)

- **Secrets round-trip.** Encrypt a secrets map, drop the in-memory copy, reload from
  disk with the key, decrypt, assert equality; assert a wrong/absent key fails closed and
  the plaintext never appears in the on-disk bytes (the same test shape the device-token
  digest store uses).
- **OAuth state machine.** `state` mismatch is rejected; a replayed `state` is rejected;
  PKCE verifier is carried; `invalid_grant` maps to `NeedsReconnect`; a transient 5xx maps
  to retryable `Error` and preserves last-good.
- **Provider.** Against recorded Calendar API fixtures: fields match the ICS field shape
  byte-for-byte for an equivalent event set (so the card is provably unchanged); `410`
  triggers a full resync; oversize/malformed bodies fall to last-good.
- **Egress.** Assert Google's three hosts resolve to allowed (globally-routable) addresses
  through the guard in this deployment, so a guard change that would break auth fails a
  test rather than production.
- Workspace gates per `CLAUDE.md` Verification (`cargo fmt`/`clippy`/`test --workspace
  --all-targets`/`--doc`). Server integration tests bind loopback — run by the controller,
  not a sandboxed subagent (per project memory).

### 9.2 End-to-end acceptance (the milestone's real proof)

1. **Authorize once, Mac quit, real calendar renders.** From a browser: connect Google
   through the dashboard, grant consent, land back on success. **Quit the Mac app.** A
   networked-tier device with a Google calendar card shows the owner's actual upcoming
   events, fresh within the refresh cadence. This is the headline demo, and it must be
   proven first — the analogue of V2's "unplug the cable, quit the Mac, cards keep
   updating".
2. **Token survives server restart.** Restart the server process. Without any
   re-authorization, the calendar card refreshes again (the encrypted refresh token
   reloaded and a new access token minted). Assert `secrets.enc` on disk contains no
   plaintext token.
3. **Revocation is handled.** Revoke Deskmate's access from the Google account security
   page. The card ages to an error state and the dashboard shows **`NeedsReconnect`**, not
   a generic error. Re-connecting through the dashboard restores it.
4. **Outage is not revocation.** Kill outbound network briefly; the card holds last-good
   and goes `stale`, the integration health stays retryable (not `NeedsReconnect`), and it
   recovers on its own when the network returns.

Recorded in `docs/hardware/board-notes.md` for the on-device observations, per the standing
preference, using the webcam harness for panel reads.

## 10. Decisions taken (owner, 2026-09-06) and lower-order items

The three sign-off questions this spec was written around are **decided**:

1. **Schema → v7. DECIDED.** A Google calendar source is expressed as an additive
   `CalendarSource::Google { integration_id }`, moving the shared authoring schema to
   **v7** with lossless migration (the routine path used for v4→v5→v6). This is a server +
   `app-core` + Mac-IPC change requiring a server redeploy, and it changes **no byte the
   device consumes** — a Google calendar compiles to the same wire card as ICS. The
   device-facing contract (protocol v1, the wire config the firmware reads) stays frozen;
   "v6 frozen" in the brief meant that contract, which is untouched. The rejected
   alternative (a side server-side card→integration mapping keyed by device+card id) is not
   pursued.

2. **Publish the Google OAuth app "In production", UNVERIFIED. DECIDED.** Leaving it in
   "Testing" expires refresh tokens after 7 days, which is fatal for an always-on display;
   "In production" gives the owner's account a non-expiring refresh token behind a one-time
   "unverified app" consent warning, which is harmless for a single owner. Google's
   verification review is **not** on V3's critical path (it would only remove the warning or
   let other people connect — both deferred). `calendar.events.readonly` is a sensitive
   scope but that is acceptable unverified for one owner.

3. **Encryption key: `0600` keyfile, at-rest boundary. DECIDED.** The key lives in a
   `0600` keyfile outside the config dir (`DESKMATE_SECRETS_KEY_FILE`, env-base64 fallback,
   fail-closed startup). At-rest protection is real (backups, stolen disk, accidental git
   commit); a live-server compromise can still read tokens, and that boundary is **accepted**
   — it is inherent to an unattended server acting on the owner's behalf, and a stricter
   posture (hardware-backed key, decrypt-on-operator-unlock) would defeat the "works with the
   Mac quit" goal.

Lower-order items, notable but not blocking: default calendar is `primary` (calendar
picker deferred to §7); one Google account/integration in V3 (the store shape allows more,
the UI ships one); and the `age` vs RustCrypto-`chacha20poly1305` choice is an
implementation call, both acceptable, `age` preferred for its rotation story.
