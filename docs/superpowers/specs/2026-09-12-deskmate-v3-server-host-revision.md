# Deskmate V3 Server Host — Design Revision (Producer-Side Integrations)

> **This supersedes `2026-09-06-deskmate-v3-server-host-design.md`.** That document is
> kept, not deleted: it is the record of what sub-projects 1 and 2 were built and
> reviewed against, and their plans' STATUS headers cite its section numbers. Where the
> two disagree, **this document is the contract.**

**Approved by the owner 2026-09-12**, choosing "server holds credentials, producers fetch
and render" over two alternatives (§12).

## 0. Why this revision exists

The 2026-09-06 spec was written against a server that no longer exists. Between that date
and 2026-09-12, schema v8, v9 and v10 and protocol v2 landed on `main`, and the
legacy-subtraction spec removed the layer V3's sub-project 3 was designed to extend.

Every load-bearing seam the original §2–§5 name is gone from `main`:

| Seam the 2026-09-06 spec builds on | State on `main` at `2e25a22` |
|---|---|
| `ServerProviderRefresher` / `SystemProviderRefresher` | removed (schema v8/v9) |
| `plugin_refresher.rs` — the interception point | removed |
| `providers::ics` — the field shape to match byte-for-byte | removed (−7,332 lines) |
| `ProviderRefreshResult`, the `Field` bag | removed |
| `MAX_PROVIDER_RESPONSE_BYTES`, the egress guard | removed |
| `CalendarSource`, the `calendar` card, the `row-list` template | removed |

Three further statements in that spec are now false as written: it says protocol stays
**v1** (`main` is v2, `PROTOCOL_VERSION = 2`), it proposes a schema addition to **v7**
(`main` is v10, `CURRENT_SCHEMA_VERSION = 10`), and it describes a Google calendar
compiling to "the identical `row-list` card an ICS calendar already does" — a card kind
that no longer exists. `main` has exactly three card kinds: `Clock`, `Pomodoro`,
`Picture` (`companion/crates/app-core/src/config.rs:823`).

**Sub-projects 1 and 2 are not affected by any of this.** They are credential custody —
`IntegrationStore` and `TokenManager` — and touch none of the removed seams. They remain
delivered, reviewed, and correct. It is sub-project 3, and only sub-project 3, that was
built on rubble.

## 1. The invariant this revision is organized around

What schema v8 bought was not "no calendars". It was:

> **The server never fetches the content of a card face.**

The scene-rendering spec rules a headless browser out permanently for the same reason —
"it would make the homelab an arbitrary-code-execution host". Producers push finished
pixels *in*; the server never reaches *out* for a face. `reqwest` survives in
`companion/crates/server/Cargo.toml` but is referenced by zero lines of server source:
the subtraction left the dependency behind, not the behaviour.

This revision holds that invariant exactly, and reads the CLAUDE.md line "the server
makes no outbound HTTP at all" as the description of that state rather than as a
prohibition on speaking to an identity provider. **The server may talk to an OAuth token
endpoint. It may never talk to a content endpoint.** §8 makes that a tested boundary
rather than a convention.

## 2. What V3 delivers / does not

**Delivers.** A homelab server that no longer needs the Mac app to be a useful owner of a
networked device. Three things:

1. **Server-held OAuth credential custody, Google first.** The owner authorizes once
   through a browser consent flow the server terminates. The refresh token is stored
   encrypted at rest; access tokens live in memory only. *Unchanged from 2026-09-06, and
   already built.*
2. **Token vending to authenticated producers.** A producer presents its credential and
   receives a short-lived access token for one integration. It calls the third-party API
   itself, renders a 448x368 face, and pushes the PNG to its image source through the
   existing `POST /v1/images/{token}` path. The server holds the grant; the producer does
   the fetching and the drawing. *This replaces the deleted `GoogleCalendarProvider`.*
3. **A minimal web management surface.** Start and revoke an integration; see device
   connection status, per-integration health, and per-image-source liveness and staleness.

**Does not deliver.** User accounts and signup; multi-tenancy; billing; the plugin-upload
sandbox (the scene spec's stage 5). Single-owner throughout; §11 marks the one seam a
future multi-tenant step slots into.

**Requires no firmware change and no schema change.** This is stronger than the original
spec managed. A Google calendar is a `Picture` card bound to an image source — a shape
schema v10 already has. The `CalendarSource::Google` addition, and with it the v7 bump the
original §10 decision 1 signed off, is **withdrawn**: there is nothing to add it to. V3 is
schema-neutral, so it does not trip the "redeploy whenever the schema moves" trap.

## 3. Architecture

```
                    Cloudflare Tunnel -> Caddy -> deskmate server (Axum)
                                                     |
 browser (owner) -- admin cookie ------------------> |  admin + management router
                                                     |    |- GET  /v1/manage                     (dashboard, HTML)
                                                     |    |- POST /v1/integrations/google        (start consent -> 302)
                                                     |    |- GET  /v1/integrations/google/callback
                                                     |    |- POST /v1/integrations/{id}/revoke
                                                     |    `- POST /v1/integrations/{id}/token    (NEW: vend to producer)
                                                     |
 accounts.google.com  <-- browser redirect only ---- (the SERVER never fetches this host)
 oauth2.googleapis.com  <-- server's ONLY egress --- |  IntegrationStore (secrets.enc, AEAD)
                                                     |  TokenManager (refresh, in-memory access tokens)
                                                     |
 producer (calendar renderer, outside this repo)     |
   |  1. POST /v1/integrations/{id}/token  --------> |
   |  2. GET  googleapis.com/calendar/v3 ... (producer's own egress, NOT the server's)
   |  3. render 448x368 PNG
   `- 4. POST /v1/images/{token}  -----------------> |  image_ingest -> canonical frame -> durable asset
                                                     |
 device (WSS, networked tier) <-- protocol v2 ------ |  per-device runtime -> Picture card scene
```

The server gains exactly one new route and one new concept. Everything below the
management router already exists on `main` and is untouched.

## 4. What survives from 2026-09-06 unchanged

These sections carry over verbatim and are **not** re-litigated here:

- **§1.1 secret storage** — AEAD `secrets.enc`, `0600` keyfile outside the config dir,
  env-base64 fallback, fail-closed startup. Built (`994a422`..`e472113`).
- **§1.2 management surface shape** — server-rendered HTML on the admin router, not an
  SPA, not the Mac app. Admin-token-minted session cookie.
- **§3 the OAuth flow** — confidential client, authorization code grant, PKCE as
  defense in depth, `access_type=offline`, `prompt=consent`. Built (`ca319af`..`ee69120`)
  and reviewed 2026-09-09.
- **§10 decision 2** — publish the Google app "In production", unverified, because
  "Testing" expires refresh tokens after 7 days and that is fatal for an always-on panel.
- **§10 decision 3** — the `0600` keyfile and the accepted live-compromise boundary.

Two carry-forward facts from the sub-project 1 plan's STATUS header that this design
depends on: `IntegrationStore` holds a sync `Mutex` across disk I/O, so **any async Axum
handler calling `put`/`remove` must go through `spawn_blocking`** — the new token-vending
route included; and the key-source fail-closed precedence plus the redacted
`IntegrationSecret` `Debug` are pinned by `e472113`.

## 5. Token vending — the one new seam

`POST /v1/integrations/{id}/token`, authenticated as a **producer**, not as the admin.

**Why a distinct credential.** The image-source token is already write-only, scoped to one
source, and independently revocable (`docs/images/producer-guide.md`). A producer that can
push to a source should not thereby be able to mint Google access tokens, and an admin
token should not have to live in a producer's environment. So an integration carries its
own producer credential, minted and revoked alongside the integration, following the
registry contract the image sources already use: **only the SHA-256 digest is persisted,
and the plaintext is returned exactly once.**

**Response.** The live access token and its absolute expiry, nothing else. Never the
refresh token, never the client secret — consistent with V2's NVS no-readback rule and the
original §6.

**What is vended is Google's access token, not a server-minted one.** The server is a
custodian, not an issuer: `TokenManager` already holds a live access token with Google's
own expiry (typically one hour), and the vend path hands out that value. It does not wrap
it, re-sign it, or shorten it. A producer therefore holds a credential Google itself can
revoke, which is the property that makes revocation at the Google account page work
end to end (§13.2 gate 3).

**Scope of what is vended.** One integration id, the scopes that integration was granted,
and nothing broader. A producer holding a calendar-scoped token cannot reach another
integration.

**Cadence.** The vend path calls `TokenManager::access_token`, which already serializes
concurrent callers behind a per-integration async gate — the refresh-stampede fix the
2026-09-09 review forced, with a test that spawns 8 racing callers against a fake queued
with one response. N producers polling is exactly the shape that fix was built for.

**The trap this route must not reintroduce.** It hands out a credential that reaches the
public internet. It must never accept a producer-supplied URL, host, or scope — the
integration record is the only source of those. A route that took a target URL would
recreate the SSRF surface the deleted egress guard existed to bound, without the guard.

## 6. The reference producer (Google Calendar)

Lives in **`tools/picture-producers/`**, which is where this repository already keeps
them. The precedent is `claude_limits_png.py`: a standalone Python script that reads
whatever data it likes, draws the finished 448x368 face with PIL, and POSTs the bytes —
shipped with a `deploy/` systemd service, timer and wrapper script installed to
`/opt/deskmate-producers/`. Its own docstring states the contract this design depends on:
"There is no manifest, no expression language and no scene -- which is the whole point of
the card kind."

A producer is not linked into the server and shares no code with it; it is a separate
process with its own credentials and its own egress. Keeping it in-tree is a packaging
decision, not an architectural one, and it does not weaken §1 — the *server* still fetches
nothing. The Google Calendar producer follows `claude_limits_png.py`'s shape.

What it does, on its own schedule: vend a token; call
`GET https://www.googleapis.com/calendar/v3/calendars/primary/events` with
`timeMin=now`, `singleEvents=true`, `orderBy=startTime`; render a 448x368 8-bit RGB PNG;
`POST /v1/images/{token}`.

What the original §4 specified that is now the producer's business, not the server's:
incremental sync with `syncToken` and the `410 Gone` full-resync fallback, response-size
caps, summary truncation, event-count caps. These are good engineering and belong in the
producer guide as recommendations; they are no longer server contracts, because the server
never sees a Calendar API response.

**The row projection is gone and is not coming back.** The original spec's central
constraint — produce a `Vec<Field>` byte-identical to the ICS provider's so the card is
provably unchanged — has no referent. The producer draws whatever it draws, within the
picture contract: exactly 448x368, 8-bit, RGB or RGBA composited over black, at most
1 MiB, PNG only. An identical push is a no-op that still counts as liveness.

## 7. Error handling and health

The original §5's error table mapped failures onto the `calendar` card's last-good and
stale-footer semantics. Those do not exist. The replacement is the staleness model already
shipped in `companion/crates/server/src/image_staleness.rs`: a source's deadline is **3x
its observed baseline cadence, clamped to a 15-minute floor and a 48-hour ceiling**. A
frozen face is the picture card's defined behaviour between pushes, so a dead producer
degrades by going stale, not by blanking the panel.

Failures now sort into two independent axes, and keeping them independent is the point:

| Condition | Where it shows | Card behaviour |
|---|---|---|
| Access token expired, refreshed transparently | nowhere | unaffected |
| Token vend fails: network/5xx/timeout | integration health `Error(transient)`, retried | last pushed frame, ages toward stale |
| Token vend fails: `invalid_grant` (revoked at Google, or refresh token expired) | **`NeedsReconnect`** on the dashboard | last pushed frame, ages toward stale |
| Producer is down, or its own API calls fail | image-source staleness on the dashboard | last pushed frame, marked stale past deadline |
| Producer pushes a malformed frame | 415/422 to the producer, one line | last **good** frame retained |

`NeedsReconnect` survives from the original §5 and remains the one state that is not just
"stale": a revoked grant cannot self-heal, so it must look different from an outage the
server will retry out of. It is now a dashboard state only — the panel shows a stale
picture either way, and that is correct. **Distinguishing them at the panel would require
the server to draw, which is the thing it does not do.**

The dashboard is where the two axes are read together, and that is its main justification:
a stale card with a healthy integration means the producer is broken; a stale card with
`NeedsReconnect` means the owner must re-authorize.

## 8. Security

Carried from the original §6, with the changes the new shape forces:

- **Encryption at rest, client secret custody, redirect-URI handling, no token readback,
  management-surface exposure** — unchanged. The client secret lives in
  `DESKMATE_GOOGLE_CLIENT_SECRET_FILE` at `0600` with no silent fallback, and the
  configured value wins over the stored copy (both from the 2026-09-09 review).
- **Egress is now a one-host allowlist, and it is tested.** The deleted guard's job shrinks
  to a single permitted outbound host: `oauth2.googleapis.com`, for the code exchange, the
  refresh, and the revoke. **`accounts.google.com` is not on the list and must not be** — the
  consent step is a `302` the *browser* follows, not a fetch the server makes, and
  `www.googleapis.com` (the Calendar API) is now the producer's egress, not the server's. A
  test asserts the server makes **no outbound request to any other host**, which is the
  executable form of §1's invariant. The
  original spec's "assert Google's hosts resolve to allowed ranges" check is withdrawn —
  there is no range-based guard left to break.
- **`https` is required on all four credential-path URLs** (2026-09-09 review). The old
  justification for permitting `http` — plugin feeds — died with the plugins.
- **Producer credentials follow the image-source contract**: digest-only persistence,
  plaintext returned once, independently revocable, write-scoped to one integration.
- **Revocation is complete.** Revoking an integration must revoke its producer credential
  in the same operation, or a producer keeps a valid credential against a dead integration
  and the failure surfaces as a confusing `401` loop rather than `NeedsReconnect`.

## 9. Schema and wire impact: none

- Schema stays **v10**. No `docs/config/v11.md`, no migration, no Mac-app deployment
  boundary, and the "redeploy whenever the schema moves" trap is not tripped.
- Protocol stays **v2**, capabilities stay **2016**, and no firmware change is implied.
- The device sees a `Picture` card and a full-canvas image scene — a path already proven
  on the panel (board-notes, "Picture cards on the panel", 2026-09-10).

This is the revision's strongest claim: **V3 now touches nothing the device or the Mac app
consumes.** It is a server-side feature end to end.

## 10. Sub-project decomposition, revised

| # | Sub-project | State |
|---|---|---|
| 1 | Secrets-at-rest foundation (`IntegrationStore`) | **Delivered**, `994a422`..`e472113`, reviewed APPROVE |
| 2 | OAuth integration framework (Google first) | **Delivered**, `ca319af`..`ee69120`, reviewed APPROVE-WITH-FIXES, all fixes applied |
| 3 | ~~Google Calendar provider + `CalendarSource::Google`~~ → **Token vending + producer credential** | **Withdrawn and replaced.** Scope is now §5: one route, one credential type, revocation coupling, and the egress-boundary test |
| 3b | Reference Google Calendar producer | `tools/picture-producers/`, following `claude_limits_png.py`; the exit gate's proof. Its own sub-project, after the management surface, because it needs a real Google grant to develop against |
| 4 | Management web surface | Unchanged in intent, extended: image-source liveness and staleness join integration health (§7) |
| 5 | Multi-tenant foundation | Still deferred; §11 |

Sub-project 3 is now **much smaller than it was** — it was the deepest-specced piece in the
original document and is now the shallowest. The work it used to represent moved out of
the repository entirely.

One cleanup belongs to sub-project 4, not a separate effort: `AdminAuthenticated` is
implemented twice — `companion/crates/server/src/admin.rs` and again in `images.rs`, with a
comment explaining that the first is private. The management surface is the third consumer;
it should unify them rather than add a fourth.

The dead `reqwest` dependency becomes live again under this design, used only by the token
endpoint. It stays in `Cargo.toml` — but §8's egress test is what keeps it honest.

## 11. Where multi-tenancy slots in (deferred, not built)

Unchanged from the original §7: key integrations, configs and secrets by an `owner_id`
defaulted to a single constant; `AdminAuthenticated` becomes `Authenticated { owner_id }`;
SQLite earns its place only at that point. Producer credentials join the list of things
that key by `owner_id` when it arrives.

## 12. Decisions taken

1. **Server holds credentials; producers fetch and render. DECIDED (owner, 2026-09-12).**
   Chosen over (B) moving OAuth wholly into the producer — which zeroes the server's
   outbound HTTP but discards 18 reviewed commits, gives every producer its own encrypted
   token store and consent flow, and still needs a public HTTPS callback host per producer,
   so N credential stores replace one; and over (C) dropping integrations from V3 entirely,
   leaving only the management surface — cheapest, but it abandons the goal that the Mac
   app is not required for real data.
2. **The schema bump is withdrawn.** The original §10 decision 1 (schema → v7 for
   `CalendarSource::Google`) has no referent and is void. V3 is schema-neutral.
3. **The original §10 decisions 2 and 3 stand** (publish unverified; `0600` keyfile and the
   accepted live-compromise boundary).
4. **The row projection is not recreated.** A producer's face is a picture, not a set of
   fields. Reintroducing a field bag to make calendars "native" would walk schema v8
   backwards.

## 13. Verification and exit gate

### 13.1 Automated (no hardware)

- **Secrets round-trip** and **OAuth state machine** — as built for sub-projects 1 and 2;
  re-run after the merge in §14, not rewritten.
- **Token vending.** A producer credential mints once and verifies by digest; a revoked
  credential is refused; vending returns an access token and expiry and never the refresh
  token or client secret; revoking the integration revokes the producer credential.
- **The egress boundary.** The server issues outbound requests to `oauth2.googleapis.com`
  and to no other host — `accounts.google.com` and `www.googleapis.com` included. This is
  §1's invariant in executable form and is the single most important new test in this
  revision.
- **`spawn_blocking` discipline.** The vend path does not call `IntegrationStore::put` /
  `remove` on the async executor.
- Workspace gates per CLAUDE.md. **Server integration tests bind loopback — the controller
  runs them, not a sandboxed subagent** (recorded project memory; a sandboxed implementer
  reports green while shipping a failing test).

### 13.2 End-to-end acceptance

1. **Authorize once, Mac quit, real calendar on the panel.** Connect Google through the
   dashboard; start the reference producer; quit the Mac app. A networked-tier device with
   a picture card bound to that source shows the owner's actual upcoming events. This is
   the headline demo and the analogue of V2's "unplug the cable, quit the Mac".
2. **Token survives server restart.** Restart the process; without re-authorization the
   producer's next vend succeeds. `secrets.enc` contains no plaintext token.
3. **Revocation is legible.** Revoke at Google's account page. The dashboard shows
   `NeedsReconnect` — not a generic error — and the card goes stale. Reconnecting restores
   it.
4. **Producer failure is distinguishable from grant failure.** Stop the producer with the
   integration healthy: the source goes stale, integration health stays `Connected`. That
   these two read differently on the dashboard is §7's whole purpose.

Panel observations are recorded in `docs/hardware/board-notes.md` per the standing
preference. **None of this is gated on the protocol-v2 flash** — V3 touches no firmware —
but the fleet is down until that flash happens, so the end-to-end gate cannot run before it.

## 14. Merge order and branch hygiene

`feat/v3-server-host` is **18 commits ahead of `main` and 78 behind** as of 2026-09-12.
Its last merge from `main` was `0c2f377` (2026-09-08), before schema v8, v9, v10 and
protocol v2.

1. **Merge `main` into the branch before writing any sub-project 3 code.** The collision
   was measured against the true merge base (`faac9ab`) on 2026-09-12, not estimated:

   | Kind | Files | Resolution |
   |---|---|---|
   | modify/delete | `server/src/egress.rs` | **The one real decision — see below.** |
   | both changed | `server/src/lib.rs`, `server/src/main.rs` | Router composition and startup wiring; `main` removed the plugin router and providers, the branch adds the OAuth router |
   | both changed | `server/Cargo.toml`, `Cargo.lock` | Reconcile dep sets; regenerate the lock |
   | both changed | `app-core/src/lib.rs`, `app-core/src/secure_file.rs` | **Both sides made the *identical* `pub(crate)` → `pub` promotion** — the branch for `IntegrationStore`, `main` for the image-source store. `server/tests/secure_file_reexport.rs` is byte-identical on both sides |
   | carried in clean | all of `server/src/oauth/`, `secrets.rs`, `tests/oauth_routes.rs`, both sub-project plans | New on the branch, untouched by `main` |

   So sub-projects 1 and 2 survive the merge substantially intact. **The claim that this
   is "mechanical" still must be verified by the full workspace gate rather than assumed**
   — budget a cold run (~35 min; `cargo` needs `export PATH="$HOME/.cargo/bin:$PATH"`, and
   piping cargo into `tail` reports `tail`'s status in zsh, hiding a failure as a pass).

   **`egress.rs` is the decision, and this revision already made it.** The module exists on
   the branch — sub-project 2's first commit (`ca319af`) added `fetch_post_form` to it for
   the token call — and `main` deleted it along with the plugin feeds it was originally
   written for. It is **kept, reduced**: the resolve-then-pin, redirects-disabled, capped-body
   machinery is exactly what §8's one-host allowlist needs, and deleting it would mean
   rewriting that guarantee from scratch for the token endpoint. What goes is the plugin-feed
   GET path and the range-based allowlist §8 withdraws; what stays is the POST-form path and
   the pinning. §8's wording — "the deleted guard" — describes its state on `main`, not a
   thing to be recreated: on the merged branch it is a surviving module with its scope cut.
2. **Re-run sub-project 1 and 2's own tests** after the merge. Their green reports predate
   78 commits of breaking change.
3. Only then start sub-project 3 as scoped in §5.

The 2026-09-06 spec gets a superseding header pointing here. The two sub-project plans keep
their STATUS headers — their work shipped and their section references still resolve
against the superseded document, which is why it is kept.
