# Deskmate accounts and claiming -- design

Track A, first sub-project (`docs/roadmap.md`). Brainstormed with the owner on
2026-09-22/23. Status: **approved design, awaiting spec review**.

## Goal

Turn the single-owner server into one that serves many accounts, and let a new owner
put a pre-flashed panel on their Wi-Fi and tie it to their account from the web page,
with no `deskmate-cli`. The same code runs our hosted service and a self-hosted server.

## Decisions taken with the owner

- **Scope of this spec: accounts plus claiming.** Plan limits appear only as the
  `Entitlements` seam (section 5); billing, plan limits proper and Docker/Compose
  self-host packaging are the next Track A sub-projects.
- **Sign-in: email link plus "Sign in with Google".** No passwords. Everything must be
  free to run: email goes through any SMTP server (hosted: a free tier such as
  Resend's; self-host: their own, or none).
- **Claiming: the web page sets the panel up over USB (Web Serial).** No firmware
  change. Chromium on a computer only; a phone or Safari/Firefox is told so. A
  device-hosted Wi-Fi setup hotspot is possible later as its own firmware piece.
- **Self-host follows the industry pattern:** the first person to set a server up owns
  it (proven by a setup code printed to the log, as Jenkins does), sign-ups are then
  closed unless the owner opens them, and email is optional.
- **Open core, hosted-only paid features.** Paid features live in a private crate that
  only our hosted build compiles in. The public repo builds a complete self-host server
  without it. No licence keys and no enforcement code: the code the self-host build
  lacks cannot be switched on. If self-hosters later want to pay, the private code can
  move into a source-available `ee/` directory with signed licence keys; nothing here
  prevents that. **This supersedes the roadmap's "Self-hosted: always free"**; the
  roadmap is amended in the same commit as this spec.
- **Implementation approach: built into the server** with ready-made crates, not a
  separate identity service (Authentik, Zitadel, Keycloak, Kratos) and not a
  TypeScript auth sidecar. A self-hoster keeps running one binary.

Decided without the owner, as implementation detail: a panel has exactly one owning
account, an account may own several panels, and there is no household sharing.

## Cost boundaries

None of the three is crossed. The config document stays **schema v10** -- only where
the files live changes, not their content. The wire is untouched and there is no
firmware change. **Track A does not take the schema/wire lock.**

## 1. Architecture and storage

### Identity database

A new `identity` module in `crates/server` over one SQLite file,
`$DESKMATE_CONFIG_DIR/identity.db` (`rusqlite` with the `bundled` feature, so there is
no system dependency). It holds identity only:

| Table | Holds |
|---|---|
| `accounts` | `id` (random, filesystem-safe), `email` (unique, lowercased), `email_verified`, `created_at`, `is_instance_owner` |
| `sessions` | `id_sha256`, `account_id`, `created_at`, `expires_at`, `last_seen_at` |
| `login_tokens` | `token_sha256`, `email`, `expires_at`, `used_at` |
| `google_identities` | `google_sub`, `account_id` |
| `device_owners` | `device_id` (primary key), `account_id`, `state` (`pending` or `active`), `claimed_at` |
| `instance` | `signups_open` (null until the owner first sets it) |

Secrets are stored only as SHA-256 digests. The database is written with the same
0600 discipline as the existing JSON stores. Access from async handlers goes through
`spawn_blocking`, the rule the `IntegrationStore` already follows.

### One folder per account

What an account's panels show moves under `configs/accounts/<account_id>/`:

```
configs/
  identity.db
  device-identities.json          # stays instance-wide: ids and tokens are the server's
  accounts/<account_id>/
    image-sources.json
    image-frames/<source>.bin
    data-cards.json
    devices/<device_id>.json      # the v10 config document, unchanged
```

**OAuth integrations stay instance-wide** (amended while planning, 2026-09-23):
`secrets.enc` and `producer-credentials.json` remain at the config root and only the
instance owner can reach them. They exist for the Google Calendar producer, which was
withdrawn with V3's exit, so no card uses them; splitting them per account would be
work and risk with no user. They move per account if and when an integration comes
back as a product feature.

The existing store types keep their code and are opened with an account root instead of
the config root. A panel that changes owner starts from a clean config because the old
one stays in the previous owner's folder (and is deleted with it).

`DESKMATE_DATA_CARDS`, which today overrides the path of the single `data-cards.json`,
is retired: a per-account file cannot have one global path.

### Runtime scoping

A picture source changing notifies only the panels owned by the same account (today it
notifies every panel, `lib.rs:347-350`). The data-card refresh schedule iterates
accounts. Nothing else in the device runtime changes: a device link still authenticates
with its own bearer and one live socket per device.

### Request authentication

- `OperatorAuthenticated` is replaced by an **`AccountSession`** extractor that yields
  the account id and whether it is the instance owner.
- Every `/v1/app/*` route runs inside that account. Any route that names a device, a
  picture source, a face or a producer credential resolves it within the account; one
  belonging to another account is **404, not 403**, so ids are not an oracle.
- The admin bearer (`DESKMATE_ADMIN_TOKEN`) remains for `/v1/devices*`, `deskmate-cli`,
  and the recovery route (section 3). **It is no longer a way to sign in to the web
  app.** The three routes that trade it for a cookie -- `POST /v1/app/session`,
  `POST /v1/session/login`, `POST /v1/manage/login` -- are removed.
- `/v1/manage/*` and `/v1/integrations/*` require an account session belonging to the
  instance owner.
- State-changing requests (`POST`, `PUT`, `PATCH`, `DELETE`) authenticated by cookie
  must carry an `Origin` equal to `DESKMATE_PUBLIC_URL`'s origin, in addition to
  `SameSite=Lax`. Bearer-authenticated requests (devices, producers, admin) are exempt.

### New configuration

- `DESKMATE_PUBLIC_URL` (required), e.g. `https://deskmate.rodi.one`. Used for sign-in
  links, the device link URL returned by claiming, the `Origin` check and the Google
  sign-in redirect.
- `DESKMATE_SMTP_URL` (optional), e.g. `smtps://user:pass@smtp.resend.com:465`, and
  `DESKMATE_MAIL_FROM` (required if SMTP is set).
- `DESKMATE_SIGNUPS` = `closed` (default) or `open`.
- `DESKMATE_OWNER_EMAIL`, read only by the one-time migration (section 6).

## 2. Sign-in and sessions

### Email link

1. `POST /v1/app/auth/email {email}` always answers **202** with the same body,
   whether or not the account exists, so the page never reveals who has one.
2. If the account exists, or sign-ups are open, the server stores the SHA-256 of a
   256-bit random token (15-minute expiry) and mails
   `<DESKMATE_PUBLIC_URL>/signin?token=<token>`.
3. **Opening the link does not sign in.** The page shows one "Sign in" button whose
   `POST /v1/app/auth/link {token}` consumes the token. Mail scanners and link
   previewers `GET` links automatically, and a `GET` that consumed the token would burn
   it before the person clicks.
4. The token is single use. Consuming it marks the email verified, creates the account
   if it is new, and creates a session.

**2026-10-02 completion:** instance discovery also returns `email_delivery` (`email`
or `server-log`), so the confirmation screen describes the configured destination
without promising delivery or revealing whether an account exists. The screen supports
another request, address correction and visible rate-limit errors. Missing/expired links
have a route back to sign-in. Authentication responses use `Cache-Control: no-store`;
the SPA and authentication responses use `Referrer-Policy: no-referrer` to keep link
credentials out of subsequent request referrers. Session loss clears the loaded account
state and closes its stream; sign-out remains available before a panel is claimed.
Email and Google registration wait until the first owner completes setup, even when
sign-ups default to open. Regression tests reproduced and now prevent registration
from creating an ownerless instance before the setup code is used.

Verification on 2026-10-02: 870 Rust tests passed (2 intentionally ignored), doctests,
fmt and Clippy passed; 222 web tests, types, lint, format and production build passed;
556 faces tests, types, lint and format passed. The real Rust server and built SPA in
Chrome completed setup, sign-out, SMTP delivery to a local sink, explicit email-link
confirmation, authenticated account access, reload/server-restart persistence, and rejection of link
reuse after sign-out. Desktop/mobile captures had no horizontal overflow or browser
JavaScript errors. Google HTTP tests cover browser binding, nonce, PKCE, replay,
verified/authoritative email matching, new and returning accounts, closed registration
and failure throttling; startup with client ID/secret alone was also exercised. Chrome
also completed a Google round trip through a local provider simulator on a different
site, with PKCE and nonce validation, new-account creation and sign-out. Google provider
responses were simulated: live consent, production credentials and external inbox
deliverability were not exercised. A read-only production instance check still reported
Google disabled. These local checks preceded deployment.

Deployment on 2026-10-02: PR #20 merged as `149d07f` after all applicable CI jobs
passed (the companion job needed one rerun for an existing runtime fixture timing
failure). The full release, including the already-live Moon Phase faces update,
shipped to `deskmate.rodi.one` at 11:30 UTC. The service is active and its installed
binary matches the release build. Production Chrome checks passed: an existing
session survived the restart, a fresh log-delivered link signed in, sign-out revoked
the session, and reuse of a consumed link was rejected. Protected endpoints return
401 without a session; auth responses carry `no-store` and `no-referrer`; the page
and built assets load with no browser JavaScript errors. Production has no SMTP or
Google credentials configured, so email links use the server log and Google remains
disabled. External email delivery and live Google consent remain unverified. Google sign-in
was switched on later the same day from Track D, with its consent screen in testing
(see the roadmap's D row); SMTP is still unset.

### Google

- `GET /v1/app/auth/google/start` and `/callback`, reusing the existing PKCE and state
  machinery and the existing Google client settings, with scope `openid email` and its
  own redirect path under `DESKMATE_PUBLIC_URL`.
- Identity is read from the ID token returned **directly by the token endpoint over
  TLS** (OpenID Connect Core 3.1.3.7 permits skipping signature validation in that
  case), so no JWKS fetch and no new egress host: `egress.rs` stays at
  `oauth2.googleapis.com`, POST only. `email_verified` must be true, or the sign-in is
  refused.
- Lookup order: `google_sub`, then a verified-email match (so an email-link account
  can later use Google and land in the same account), then create -- only if sign-ups
  are open.
- **2026-10-02 sign-in hardening:** each attempt is bound to a ten-minute HttpOnly,
  Secure, SameSite=Lax `__Host-deskmate_google` cookie, validated before consuming
  state. PKCE, nonce, audience and authorized-party checks bind the token exchange.
  Automatic email matching additionally requires a Google-managed mailbox (Gmail or
  Workspace); other existing addresses use an email link to prove current ownership.
  Google sign-in only needs client ID/secret: the integration redirect and encrypted
  refresh-token store are optional and independent.
- The button appears only when `DESKMATE_GOOGLE_CLIENT_ID` is set.

### Sessions

- Cookie unchanged in name and flags: `__Host-deskmate_session`, HttpOnly, Secure,
  SameSite=Lax, Path=/. Its value becomes an opaque 256-bit random id whose digest is in
  `sessions`, replacing today's HMAC-signed timestamp.
- 30 days, sliding: `expires_at` and `last_seen_at` are extended at most once a day.
- Sign out deletes the row. "Sign out everywhere" deletes all of the account's rows.

### Abuse limits

In memory, no new infrastructure: sign-in emails are limited to 5 per address and 20
per client IP per hour; link and setup-code submissions to 5 failures per IP per 15
minutes. On a public server the cheap attacks are flooding someone's inbox and burning
the free SMTP quota. This is also the login rate limit `CLAUDE.md` names as the
prerequisite for any human-chosen secret; there are still no passwords.

The client IP is the peer address, or the `X-Forwarded-For` value appended by the
trusted local proxy when the peer is loopback (the deployment is Cloudflare ->
cloudflared -> Caddy -> server on `127.0.0.1`).

### Mail

A `Mailer` trait with two implementations: SMTP through `lettre` when
`DESKMATE_SMTP_URL` is set, and a log mailer otherwise, which prints the link to the
server log inside a clearly marked block. Mail failures are logged and do not change the
202 answer.

### Account settings (in the existing `SettingsSheet`)

The account's email; sign out; sign out everywhere; **delete account**, which removes
the account's folder and rows and releases its panels (each identity is revoked, so the
board stops linking until it is set up again). The instance owner also sees the
sign-ups switch, and cannot delete their account while other accounts exist.

## 3. First run and recovery

### Setup code

- On boot, if `accounts` is empty, the server generates an 8-character setup code from
  an alphabet without look-alike characters (shown `XXXX-XXXX`), holds it only in
  memory, regenerates it on every boot, and logs it in a clearly marked block.
- `GET /v1/app/instance` answers `{setup_required, google_enabled, signups_open}`,
  unauthenticated. With `setup_required` the web app shows **"Set up this server"**:
  setup code and email.
- `POST /v1/app/setup {code, email}` creates the instance-owner account and a session
  directly, with no email round trip: the code already proves the caller can read the
  server's log. The email is unverified until the first sign-in that uses it.
- Once an account exists the code is erased and the route answers 404.

### Sign-ups

`DESKMATE_SIGNUPS` seeds the value; the instance owner's switch in settings stores
`instance.signups_open`, which then overrides the env. Our hosted server sets `open`.
There is no invite system.

### Recovery

`POST /v1/admin/signin-link {email}` with the admin bearer returns a working sign-in
link for an existing account; the self-host docs give the one `curl` line. (Amended while
planning: `deskmate-cli` has no HTTP client at all -- it speaks only USB serial -- so a
`signin-link` subcommand would add an HTTP stack for a break-glass path.) The admin token
is the break-glass key: it never signs anyone in by itself, but it can always mint a way
back in.

## 4. Claiming a panel

### Where

"Add a panel" in `SettingsSheet`, where device ownership already lives. An account with
no panels has no window to show -- the window needs a snapshot, and a snapshot needs a
device -- so its startup screen **is** the setup flow (amended while planning); the
settings entry covers the second panel onwards. Without `navigator.serial`, the
control is replaced by "Setting up a panel needs Chrome or Edge on a computer."

### Flow

1. **Connect.** The port picker is filtered to Espressif's USB Serial/JTAG
   (`vendorId 0x303a`, `productId 0x1001`). Opening the port resets the board, so the
   page sends `StatusRequest` every 500 ms for up to 10 s until one is answered, then
   requires protocol version 2 and capability bit 7 (`Networking`). Anything else gets
   a plain-language refusal before any write.
2. **Wi-Fi.** The page asks for the network name and password. **They are sent only
   over the cable to the board, never to the server.**
3. **Claim.** `POST /v1/app/devices/claim` mints a fresh identity with the existing
   `Registry::mint`, records it in `device_owners` as `pending` for this account, and
   answers `{device_id, token, link_url}` -- the only time the token exists in
   plaintext.
4. **Write.** The page sends `NetworkConfig {ssid, psk, server_url: link_url, device_id,
   token, utc_offset_minutes: <browser's>, tier: 1}` and requires its `Ack`.
5. **Wi-Fi check over the cable.** The page keeps polling `StatusRequest` and shows
   key 25 (Wi-Fi state) live. On `failed` it shows key 29 (the board's own last network
   error) and offers to retype the password; a retry resends the **same** identity,
   which the page still holds in memory, rather than minting another. If the port drops
   because the board restarts on the new settings, the page reopens it and continues.
6. **Link check from the server.** Once Wi-Fi is connected the page polls
   `GET /v1/app/devices` for up to 60 s. The first successful device link flips the
   panel to `active`; the page says it is connected and can be unplugged.

A `pending` panel with no link after 24 hours is removed and its identity revoked, by the
same periodic task that already runs data-card refreshes.

### Removing a panel

`DELETE /v1/app/devices/{id}` revokes the identity, drops the ownership row and the
panel's config file, and closes its live link. The board falls back to its standalone
clock.

### Second-hand panels -- known limitation

The cable wins: the firmware accepts `NetworkConfig` over USB in networked tier
(`net_config_usb_message_allowed`), so running setup with a panel claims it under a new
identity. `StatusResponse` does not carry the provisioned device id, so the server cannot
tell which old identity the board had; the previous owner sees that panel as offline
until they remove it. Fixing that means adding the device id to `StatusResponse`, a wire
change, and is deliberately not done here.

### Unchanged

`deskmate-cli` provisioning. A device minted with the admin bearer is owned by the
instance owner, so the operator's cable workflow keeps working.

### The browser side

- `apps/deskmate/src/lib/serial/`: a codec for the subset the page speaks (COBS,
  CRC-32C, the envelope, canonical CBOR for `StatusRequest`, `NetworkConfig`,
  `StatusResponse`, `Ack`, `Error`) and a `SerialPort` interface with the Web Serial
  implementation and a fake for tests.
- The page allows one outstanding request, times out after 2000 ms, and ignores
  `DeviceEvent`s, as `docs/protocol/v2.md` requires of a host.

## 5. The open-core seam

- An **`Entitlements`** trait answers per-account questions: the card, picture-source
  and panel limits, and whether a named feature is on. The public implementation returns
  today's limits (`MAX_CONFIG_CARDS` 8, `MAX_IMAGE_SOURCES` 8) and turns every feature
  on; an unlimited panel count.
- `ServerState` takes the implementation, and an optional router to merge, as
  constructor arguments. The hosted binary lives in a private repository, depends on
  `deskmate-server` as a library, and supplies its own. **The public `Cargo.toml` never
  names the private crate**, so a self-hoster's build cannot fail on a dependency they
  cannot fetch.
- `GET /v1/app/instance` also reports `edition: "self-hosted" | "hosted"`, supplied by
  the same constructor argument.
- Nothing in this spec ships a paid feature. The seam exists so the next sub-project
  (plan limits and billing) has one place to hook in.

## 6. Migration

Runs once at startup, before anything is served, when `identity.db` is absent and the
flat layout is present (any of `device-identities.json`, `image-sources.json`,
`data-cards.json`, `producer-credentials.json`, `secrets.enc`, `dev-*.json`).

1. Requires `DESKMATE_OWNER_EMAIL`; without it the server **refuses to start** and says
   what to set. It never guesses an owner.
2. Creates `identity.db` under a temporary name with the owner account
   (`is_instance_owner`, email unverified) and one `active` `device_owners` row per
   registry device.
3. Copies `image-sources.json`, `image-frames/` and `data-cards.json` into
   `accounts/<id>/`, and each `dev-*.json` into `accounts/<id>/devices/`.
   `device-identities.json`, `secrets.enc` and `producer-credentials.json` stay where
   they are.
4. Renames `identity.db` into place, then moves the copied originals into
   `configs/legacy-<UTC timestamp>/`. They are kept, never deleted.

A crash before step 4 leaves no `identity.db`, so the next start migrates again from the
untouched originals. `dev-0005` keeps its identity and token and re-links with no cable;
the owner then signs in with a link (printed to the log until SMTP is configured).

**Deploy:** the binary and `dist/` in the same `deploy.sh` run -- the old sign-in screen
(admin token) cannot sign in to the new server. `server.env` gains
`DESKMATE_PUBLIC_URL` and `DESKMATE_OWNER_EMAIL` first.

## 7. Error handling

| Situation | Behaviour |
|---|---|
| Unknown email, sign-ups closed | 202, no mail |
| Expired, used or unknown link token | 400 "This sign-in link has expired", with a button back to sign-in |
| Google email unverified | Refused with a sentence saying so |
| Rate limit hit | 429 with the retry time in words |
| Board answers another protocol version or lacks `Networking` | Refused before any write: "This panel needs a firmware update first" |
| Board does not answer within 10 s | "The panel didn't respond. Unplug it, plug it back in, and try again" |
| Wi-Fi `failed` | Board's own error text (key 29), retype the password, same identity |
| No link within 60 s after Wi-Fi connects | Says the panel reached Wi-Fi but not the server, and keeps it `pending` |
| Resource in another account | 404 |
| SMTP failure | Logged; the 202 is unchanged |
| Migration without `DESKMATE_OWNER_EMAIL` | Server refuses to start, naming the variable |

## 8. Testing and verification

### Rust

- Identity store: link tokens single use and expiring; **the database file never
  contains a plaintext token** (asserted by scanning the file's bytes).
- Sessions: sliding expiry, sign out, sign out everywhere.
- **`tests/isolation.rs`**: account B gets 404 for account A's panel, picture source,
  frame, face settings and producer credential, through every route that takes an id.
  This file guards the largest risk in the spec.
- Setup code: single use, rate-limited, 404 afterwards.
- `Origin` enforcement; Google callback through the existing mock transport, including
  an unverified email refused; recovery link requires the admin bearer.
- Migration against a fixture of the flat layout: byte-identical contents on the far
  side, originals under `legacy-*`, a second start does nothing, and a crash before the
  final rename migrates again.
- **Mutation probes**, the lesson of the 2026-09-19 sweep: deleting the ownership check,
  the `Origin` check, the single-use mark and the email rate limit must each turn a test
  red.

### Web

- The TypeScript codec is tested against **golden frames written by `crates/protocol`**,
  so the two implementations cannot drift -- the same idea as the faces' golden SVGs.
- Sign-in, setup and claim screens are DOM-tested with the fake serial port.
- The mock harness (`VITE_DESKMATE_MOCK=1`) gains the signed-out, first-run and claim
  states, and gains nothing the shipped app lacks.

### Real checks the suite cannot replace

1. **Spike, before any claim UI:** open `dev-0005` from Chrome with Web Serial and
   observe how long after the reset it answers `StatusRequest`, and whether
   `NetworkConfig` applies live or restarts the board. The claim flow's timings are set
   from what it shows. Needs the owner to plug the panel into the Mac once.
2. Drive the real build in Chrome against a local server: first-run setup code, a
   sign-in link from the log, and a second account that cannot see the first one's data.
3. Claim `dev-0005` from the deployed page. This re-provisions the owner's live panel
   with a new identity, so it happens only when the owner says so. Recorded in
   `docs/hardware/board-notes.md` with what was observed.

## Out of scope

Plan limits and billing; Docker/Compose packaging and self-host docs; invites and
household sharing; passkeys; a device-hosted Wi-Fi setup hotspot; the device id in
`StatusResponse`; changing the email on an account.
