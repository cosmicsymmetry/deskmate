# Accounts and Claiming Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. **Tick each box as its verification passes** -- see `CLAUDE.md` on plans whose boxes lie.

> **STATUS (2026-09-23):** Tasks 2-12 implemented (Codex workers), reviewed and committed on
> `feat/track-a-accounts`, each with the orchestrator running the integration tests the
> workers' sandbox could not and mutation probes on every guard (recorded in each commit
> message). Task 13 steps 1-3 done: full gates green (776 Rust, 215 web, faces suite);
> step 2 was driven against the real server with `curl` over every route the page calls
> plus headless-Chrome screenshots of the screens -- the Chrome extension kept losing its
> tab group -- and found that an unset `RUST_LOG` hid the setup code (fixed, `f7ce7c1`);
> step 3 migrated a copy of the live config (5 devices, 4 picture sources, the loop intact).
> **Open:** Task 1 (Web Serial spike, needs the owner at the board); Task 13 steps 4-7.
> **Merged 2026-09-28 (PR #5), after C1 phase 1 was merged in and made per-account.**
> Deploy rides C1's next deploy (owner's choice): the live server was running C1's phase-2
> branch, and replacing it mid-session was ruled out. `DESKMATE_PUBLIC_URL` and
> `DESKMATE_OWNER_EMAIL=re.aleksandrov1@gmail.com` are already in the live `server.env`
> (backed up first), so that first start migrates instead of refusing. Task 13 steps 5-6
> (observe the migration live; claim `dev-0005`) are owed after that deploy.

**Goal:** Turn the single-owner server into a multi-account one (email-link and Google sign-in, one folder per account, a first-run setup code) and let a signed-in owner put a panel on their Wi-Fi and claim it from the web page over Web Serial.

**Architecture:** A new `identity` module over one SQLite file holds accounts, sessions, sign-in tokens, Google links and device ownership. Each account gets an `AccountSpace` (its image sources, frames, data-card specs and device configs under `configs/accounts/<id>/`) that the server opens lazily. An `AccountSession` extractor replaces `OperatorAuthenticated` and scopes every `/v1/app/*` lookup to one account. The web app gains sign-in/setup screens, and a TypeScript codec for the four wire messages the setup page speaks over Web Serial.

**Tech Stack:** Rust (axum 0.8, tokio, `rusqlite` bundled, `lettre` for SMTP), React 19 + bun test + happy-dom, Web Serial API.

**Spec:** `docs/superpowers/specs/2026-09-23-deskmate-accounts-and-claiming-design.md` (read it first; this plan argues from it).

## Global Constraints

- Config schema stays **v10**, protocol stays **v2**, **no firmware change**. Anything that would touch `CURRENT_SCHEMA_VERSION`, `docs/protocol/v2.md`, `crates/protocol` message shapes or `firmware/` is out of scope -- stop and ask.
- Nothing may cost money to run. No new outbound host: `egress.rs` stays `oauth2.googleapis.com`, POST only.
- Secrets (session ids, sign-in tokens, device tokens, setup code) are never persisted in plaintext; the identity db stores SHA-256 digests via `credential::token_digest`.
- Sign-in link expiry **15 minutes**; session **30 days**, slid at most once per day; setup code **8 chars**, alphabet `23456789ABCDEFGHJKMNPQRSTUVWXYZ`, shown `XXXX-XXXX`.
- Rate limits: sign-in emails **5 per address per hour** and **20 per IP per hour**; link, setup-code and Google-callback failures **5 per IP per 15 minutes**.
- A resource in another account answers **404**, never 403.
- Cookie `__Host-deskmate_session`, `HttpOnly; Secure; SameSite=Lax; Path=/`.
- Cookie-authenticated `POST/PUT/PATCH/DELETE` require `Origin` == origin of `DESKMATE_PUBLIC_URL`.
- The admin bearer never yields a browser session. Routes that traded it for a cookie are removed.
- Web Serial filter: `usbVendorId 0x303a`, `usbProductId 0x1001`. The Wi-Fi password is never sent to the server.
- Async handlers touching SQLite or the filesystem go through `tokio::task::spawn_blocking`.
- Copy: follow `DESIGN.md`/`PRODUCT.md`; no label above a heading, no card inside a card.
- `cargo` needs `export PATH="$HOME/.cargo/bin:$PATH"`; never pipe cargo into `tail` (zsh reports tail's status) -- redirect to a file and check `$?`.

## Review Focus

1. **Email case and whitespace** -- `  Rodion@Example.COM ` signs in to the same account as `rodion@example.com`. Pinned in Task 2 (`normalize_email`) and Task 5.
2. **A sign-in link opened twice, or after a newer link was sent** -- the second use says the link has expired, and an older unused link still works until its own expiry. Pinned in Task 5.
3. **A panel unplugged mid-setup, or a board that restarts after `NetworkConfig`** -- the page reopens the port and continues, or gives the "unplug and retry" sentence; it never mints a second identity for one attempt. Pinned in Task 10.
4. **Two browser tabs signed in to different accounts on one machine** -- impossible by design (one cookie); signing in as B replaces A's session cookie, and A's server-side session stays valid until it expires or is signed out. Pinned in Task 4 (a second sign-in yields a new cookie; the old session id still resolves to A).
5. **A panel's link arriving while its owner deletes the account** -- the link is refused after the deletion and the identity is gone from the registry; no config is written into a deleted folder. Pinned in Task 7.

---

## File Structure

**Rust -- `companion/crates/server/`**

| File | Responsibility |
|---|---|
| `src/identity/mod.rs` (create) | `IdentityStore`: SQLite schema, accounts, sessions, sign-in tokens, Google links, device owners, instance settings |
| `src/identity/email.rs` (create) | `normalize_email` |
| `src/accounts.rs` (create) | `AccountSpace` (per-account stores) and the lazy map on `ServerState` |
| `src/entitlements.rs` (create) | `Entitlements` trait, `SelfHosted` impl, `Edition` |
| `src/web_auth/mod.rs` (create) | `AccountSession`, `InstanceOwner`, `SameOrigin`, `ClientIp` extractors; cookie helpers |
| `src/web_auth/rate_limit.rs` (create) | in-memory sliding-window `RateLimiter` |
| `src/web_auth/routes.rs` (create) | `/v1/app/instance`, `/v1/app/setup`, `/v1/app/auth/*`, `/v1/app/account*`, `/v1/admin/signin-link` |
| `src/web_auth/google.rs` (create) | Google sign-in start/callback, ID-token claim check |
| `src/web_auth/setup_code.rs` (create) | first-run setup code |
| `src/mailer.rs` (create) | `Mailer` trait, `SmtpMailer`, `LogMailer` |
| `src/claim.rs` (create) | `/v1/app/devices` list/claim/delete, pending GC |
| `src/migrate.rs` (create) | one-time flat-layout migration |
| `src/lib.rs` (modify) | `ServerState` fields/constructors, `ServerOptions`, router, notify filtering |
| `src/oauth/session.rs` (modify) | `SessionSigner`/`OperatorAuthenticated` deleted; `set_cookie_header` moves to `web_auth` |
| `src/registry.rs` (modify) | `revoke`, high-water scan over `accounts/*/devices` |
| `src/data_cards.rs`, `src/data_cards/worker.rs` (modify) | specs/tasks per account; catalog stays global |
| `src/app_api/mod.rs`, `src/images.rs`, `src/manage/mod.rs`, `src/oauth/routes.rs`, `src/admin.rs`, `src/device_link.rs` (modify) | switch to `AccountSession`/`InstanceOwner`, resolve within account |
| `src/main.rs` (modify) | new env vars, migration, options, connect-info serve, housekeeping task |
| `tests/support/mod.rs` (modify) | `TestAccount`, `owner_account`, `second_account`, `mint_owned_device`, `cookie_request` helpers |
| `tests/isolation.rs` (create) | account B cannot see account A's resources through any route |
| `tests/accounts.rs` (create) | sign-in, setup, sessions, Google, rate limits, Origin, recovery |
| `tests/migration.rs` (create) | migration against a flat-layout fixture |
| `deploy/deskmate-server.env.example`, `deploy/README.md` (modify) | new env vars, recovery curl |

**Web -- `companion/apps/deskmate/`**

| File | Responsibility |
|---|---|
| `src/lib/serial/codec.ts` (create) | COBS, CRC-32C, envelope, CBOR subset, the four messages |
| `src/lib/serial/port.ts` (create) | `PanelPort` interface, `WebSerialPort`, request/response with 2000 ms timeout |
| `src/lib/serial/setup.ts` (create) | `runPanelSetup` state machine (connect -> claim -> write -> Wi-Fi -> link) |
| `src/lib/account.ts` (create) | account/instance/auth/device HTTP calls |
| `src/components/AuthScreens.tsx` (create) | `SignInScreen`, `CheckInbox`, `LinkLanding`, `SetupScreen` |
| `src/components/PanelSetup.tsx` (create) | the setup flow UI |
| `src/components/AccountSection.tsx` (create) | settings: email, sign out, everywhere, delete, sign-ups switch, panel list, "Add a panel" |
| `src/App.tsx`, `src/components/NetworkPanel.tsx`, `src/lib/backendClient.ts`, `src/lib/apiErrors.ts`, `src/lib/useAppState.ts`, `src/dev/backendClient.ts`, `src/dev/mockBackend.ts` (modify) | wire it in; remove the admin-token forms |
| `tests/serialCodec.test.ts`, `tests/panelSetup.test.ts`, `tests/authScreens.test.tsx`, `tests/accountSection.test.tsx` (create) | |

---

### Task 1: Web Serial spike on `dev-0005` (throwaway)

This answers two questions the setup flow's timings depend on. Nothing from it is kept except the numbers.

**Files:**
- Create (scratch, not committed): `$SCRATCH/serial-spike.html`
- Modify: `docs/hardware/board-notes.md` (append an entry with what was observed)

- [ ] **Step 1: Write the spike page.** One HTML file with an inline script that: requests a port filtered to `{usbVendorId: 0x303a, usbProductId: 0x1001}`, opens it at 115200, logs timestamps, sends the `StatusRequest` bytes from `protocol/fixtures/v2/status_request.bin` (copy the hex from `protocol/fixtures/v2/manifest.txt`) every 500 ms, and logs every chunk received as hex. A second button sends `network_config.bin` bytes **only if the owner agrees** (it rewrites the panel's network settings -- skip it unless asked; the first question matters more).
- [ ] **Step 2: Serve it** with `python3 -m http.server` from the scratch dir and open `http://localhost:8000/serial-spike.html` in Chrome (Web Serial needs a secure context; localhost counts).
- [ ] **Step 3: Ask the owner to plug `dev-0005` into the Mac.** Observe: (a) does opening the port reset the board (the panel reboots), and after how many ms does the first `StatusResponse` arrive; (b) does the board also print log text on this port that the deframer must skip (the console is UART0-only per the memory note, so expect none). Record both.
- [ ] **Step 4: Record in `docs/hardware/board-notes.md`** a dated entry: port open -> reset yes/no, first-response latency, stray bytes yes/no, Chrome version. If the latency exceeds 10 s, change the Global Constraint "10 s" figure in Task 10 before starting it.
- [ ] **Step 5: Commit** `docs: Web Serial reset and first-response timing on dev-0005`.

---

### Task 2: Identity store

**Files:**
- Modify: `companion/crates/server/Cargo.toml` (add `rusqlite = { version = "0.37", features = ["bundled"] }` -- use the newest 0.x that builds on rust 1.97 and record the version in the commit message)
- Create: `companion/crates/server/src/identity/mod.rs`, `companion/crates/server/src/identity/email.rs`
- Modify: `companion/crates/server/src/lib.rs` (`mod identity;`)
- Test: unit tests inside `identity/mod.rs` and `identity/email.rs`

**Interfaces:**
- Consumes: `crate::credential::{token_digest, random_token}` (SHA-256 digest and 32-byte hex token).
- Produces (all `pub(crate)`):

```rust
pub struct AccountId(pub String);             // "acc_" + 32 lowercase hex; Clone, Eq, Hash, Debug, Display
pub struct Account { pub id: AccountId, pub email: String, pub email_verified: bool,
                     pub created_at: DateTime<Utc>, pub is_instance_owner: bool }
pub enum DeviceState { Pending, Active }
pub struct DeviceOwner { pub device_id: String, pub account_id: AccountId,
                         pub state: DeviceState, pub claimed_at: DateTime<Utc> }
pub enum IdentityError { Sqlite(String), EmailTaken, InvalidEmail, NotFound }

impl IdentityStore {
    pub fn open(path: &Path) -> Result<Self, IdentityError>;
    pub fn account_count(&self) -> Result<u64, IdentityError>;
    pub fn create_account(&self, email: &str, verified: bool, instance_owner: bool, now: DateTime<Utc>) -> Result<Account, IdentityError>;
    pub fn account(&self, id: &AccountId) -> Result<Option<Account>, IdentityError>;
    pub fn account_by_email(&self, email: &str) -> Result<Option<Account>, IdentityError>;
    pub fn accounts(&self) -> Result<Vec<Account>, IdentityError>;
    pub fn mark_email_verified(&self, id: &AccountId) -> Result<(), IdentityError>;
    pub fn delete_account(&self, id: &AccountId) -> Result<Vec<String>, IdentityError>; // released device ids
    pub fn create_session(&self, account: &AccountId, now: DateTime<Utc>) -> Result<String, IdentityError>; // plaintext id
    pub fn session_account(&self, plaintext: &str, now: DateTime<Utc>) -> Result<Option<Account>, IdentityError>;
    pub fn delete_session(&self, plaintext: &str) -> Result<(), IdentityError>;
    pub fn delete_sessions_for(&self, account: &AccountId) -> Result<(), IdentityError>;
    pub fn create_login_token(&self, email: &str, now: DateTime<Utc>) -> Result<String, IdentityError>;
    pub fn consume_login_token(&self, plaintext: &str, now: DateTime<Utc>) -> Result<Option<String>, IdentityError>; // email
    pub fn account_for_google(&self, sub: &str) -> Result<Option<Account>, IdentityError>;
    pub fn link_google(&self, sub: &str, account: &AccountId) -> Result<(), IdentityError>;
    pub fn assign_device(&self, device_id: &str, account: &AccountId, state: DeviceState, now: DateTime<Utc>) -> Result<(), IdentityError>;
    pub fn device_owner(&self, device_id: &str) -> Result<Option<DeviceOwner>, IdentityError>;
    pub fn devices_for(&self, account: &AccountId) -> Result<Vec<DeviceOwner>, IdentityError>;
    pub fn activate_device(&self, device_id: &str) -> Result<(), IdentityError>;
    pub fn release_device(&self, device_id: &str) -> Result<(), IdentityError>;
    pub fn stale_pending(&self, claimed_before: DateTime<Utc>) -> Result<Vec<String>, IdentityError>;
    pub fn signups_open(&self) -> Result<Option<bool>, IdentityError>;
    pub fn set_signups_open(&self, open: bool) -> Result<(), IdentityError>;
}
pub const SESSION_TTL_DAYS: i64 = 30;
pub const LOGIN_TOKEN_TTL_MINUTES: i64 = 15;
pub fn normalize_email(raw: &str) -> Option<String>;
```

- [x] **Step 1: Write the failing tests** (in `identity/mod.rs` `#[cfg(test)] mod tests`, using `tempfile::tempdir()`):

```rust
fn store() -> (tempfile::TempDir, IdentityStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = IdentityStore::open(&dir.path().join("identity.db")).unwrap();
    (dir, store)
}
fn t(secs: i64) -> DateTime<Utc> { DateTime::from_timestamp(1_800_000_000 + secs, 0).unwrap() }

#[test]
fn login_token_is_single_use_and_expires() {
    let (_d, s) = store();
    let token = s.create_login_token("a@example.com", t(0)).unwrap();
    assert_eq!(s.consume_login_token(&token, t(60)).unwrap().as_deref(), Some("a@example.com"));
    assert_eq!(s.consume_login_token(&token, t(61)).unwrap(), None, "second use");
    let late = s.create_login_token("a@example.com", t(0)).unwrap();
    assert_eq!(s.consume_login_token(&late, t(15 * 60)).unwrap(), None, "expired at 15 min");
}

#[test]
fn an_older_unused_link_still_works_after_a_newer_one() {
    let (_d, s) = store();
    let first = s.create_login_token("a@example.com", t(0)).unwrap();
    let _second = s.create_login_token("a@example.com", t(10)).unwrap();
    assert!(s.consume_login_token(&first, t(20)).unwrap().is_some());
}

#[test]
fn session_slides_at_most_once_a_day_and_expires() {
    let (_d, s) = store();
    let a = s.create_account("a@example.com", true, true, t(0)).unwrap();
    let sid = s.create_session(&a.id, t(0)).unwrap();
    let day = 86_400;
    assert!(s.session_account(&sid, t(29 * day)).unwrap().is_some()); // slides to 59 days
    assert!(s.session_account(&sid, t(58 * day)).unwrap().is_some());
    let idle = s.create_session(&a.id, t(0)).unwrap();
    assert!(s.session_account(&idle, t(30 * day)).unwrap().is_none(), "unused session expires");
}

#[test]
fn delete_sessions_for_signs_out_everywhere() {
    let (_d, s) = store();
    let a = s.create_account("a@example.com", true, true, t(0)).unwrap();
    let one = s.create_session(&a.id, t(0)).unwrap();
    let two = s.create_session(&a.id, t(0)).unwrap();
    s.delete_sessions_for(&a.id).unwrap();
    assert!(s.session_account(&one, t(1)).unwrap().is_none());
    assert!(s.session_account(&two, t(1)).unwrap().is_none());
}

#[test]
fn database_file_never_contains_a_plaintext_secret() {
    let (dir, s) = store();
    let a = s.create_account("a@example.com", true, true, t(0)).unwrap();
    let sid = s.create_session(&a.id, t(0)).unwrap();
    let login = s.create_login_token("a@example.com", t(0)).unwrap();
    drop(s); // checkpoint WAL into the main file
    let mut bytes = Vec::new();
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        bytes.extend(std::fs::read(entry.unwrap().path()).unwrap());
    }
    let hay = String::from_utf8_lossy(&bytes);
    assert!(!hay.contains(&sid), "session id in plaintext");
    assert!(!hay.contains(&login), "login token in plaintext");
}

#[test]
fn email_is_unique_after_normalisation() {
    let (_d, s) = store();
    s.create_account("a@example.com", true, true, t(0)).unwrap();
    assert!(matches!(s.create_account("  A@Example.COM ", false, false, t(1)), Err(IdentityError::EmailTaken)));
    assert!(s.account_by_email("A@EXAMPLE.com").unwrap().is_some());
}

#[test]
fn deleting_an_account_releases_its_devices_and_sessions() {
    let (_d, s) = store();
    let a = s.create_account("a@example.com", true, true, t(0)).unwrap();
    s.assign_device("dev-0001", &a.id, DeviceState::Active, t(0)).unwrap();
    let sid = s.create_session(&a.id, t(0)).unwrap();
    assert_eq!(s.delete_account(&a.id).unwrap(), vec!["dev-0001".to_string()]);
    assert!(s.device_owner("dev-0001").unwrap().is_none());
    assert!(s.session_account(&sid, t(1)).unwrap().is_none());
}

#[test]
fn stale_pending_lists_only_pending_claims_older_than_the_cutoff() {
    let (_d, s) = store();
    let a = s.create_account("a@example.com", true, true, t(0)).unwrap();
    s.assign_device("dev-0001", &a.id, DeviceState::Pending, t(0)).unwrap();
    s.assign_device("dev-0002", &a.id, DeviceState::Pending, t(100)).unwrap();
    s.assign_device("dev-0003", &a.id, DeviceState::Active, t(0)).unwrap();
    assert_eq!(s.stale_pending(t(50)).unwrap(), vec!["dev-0001".to_string()]);
}
```

And in `identity/email.rs`:

```rust
#[test]
fn normalises_and_rejects() {
    assert_eq!(normalize_email("  Rodion@Example.COM ").as_deref(), Some("rodion@example.com"));
    for bad in ["", "no-at", "a@@b.c", "a b@c.d", "@c.d", "a@", "a@b\u{0}.c"] {
        assert_eq!(normalize_email(bad), None, "{bad:?}");
    }
    assert_eq!(normalize_email(&format!("{}@b.co", "a".repeat(260))), None, "over 254 bytes");
}
```

- [x] **Step 2: Run to verify they fail.** `cd companion && cargo test -p server identity > /tmp/t.log 2>&1; echo $?; grep -E "error|test result" /tmp/t.log | head` -- expected: compile errors (module missing).
- [x] **Step 3: Implement.** Key parts:

```rust
// identity/email.rs
pub(crate) fn normalize_email(raw: &str) -> Option<String> {
    let email = raw.trim().to_lowercase();
    if email.is_empty() || email.len() > 254 { return None; }
    if email.chars().any(|c| c.is_whitespace() || c.is_control()) { return None; }
    let (local, domain) = email.split_once('@')?;
    if local.is_empty() || domain.is_empty() || domain.contains('@') { return None; }
    Some(email)
}
```

```rust
// identity/mod.rs -- schema, applied in `open` inside one transaction
const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS accounts (
  id TEXT PRIMARY KEY, email TEXT NOT NULL UNIQUE, email_verified INTEGER NOT NULL,
  created_at INTEGER NOT NULL, is_instance_owner INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS sessions (
  id_sha256 BLOB PRIMARY KEY, account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL, last_seen_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS login_tokens (
  token_sha256 BLOB PRIMARY KEY, email TEXT NOT NULL, expires_at INTEGER NOT NULL, used_at INTEGER);
CREATE TABLE IF NOT EXISTS google_identities (
  google_sub TEXT PRIMARY KEY, account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE);
CREATE TABLE IF NOT EXISTS device_owners (
  device_id TEXT PRIMARY KEY, account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  state TEXT NOT NULL CHECK (state IN ('pending','active')), claimed_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS instance (key TEXT PRIMARY KEY, value TEXT NOT NULL);
";
```

- `open`: create the parent dir with `app_core::secure_file::create_directory`; `Connection::open`; on unix set the db file's mode to `0o600` with `std::os::unix::fs::PermissionsExt` right after opening (and again for `-wal`/`-shm` if present); `execute_batch(SCHEMA)`. Wrap the connection in `std::sync::Mutex`.
- Timestamps are unix seconds (`DateTime::timestamp()`).
- Plaintext secrets: `random_token()` (64 hex chars); stored as `token_digest(&plaintext).to_vec()`.
- `session_account`: select by digest where `expires_at > now`; if `now - last_seen_at >= 86_400`, `UPDATE sessions SET last_seen_at = now, expires_at = now + 30 days`. Opportunistically `DELETE FROM sessions WHERE expires_at <= now` and `DELETE FROM login_tokens WHERE expires_at <= now` in the same call.
- `consume_login_token`: `UPDATE login_tokens SET used_at = ?now WHERE token_sha256 = ? AND used_at IS NULL AND expires_at > ?now RETURNING email`.
- `create_account` normalises the email (`InvalidEmail` on `None`), maps a UNIQUE violation to `EmailTaken`, and generates `AccountId(format!("acc_{}", &random_token()[..32]))`.
- `delete_account`: in one transaction, select device ids for the account, then `DELETE FROM accounts` (cascades).
- `Drop` is not required: `Connection` closes on drop and SQLite checkpoints the WAL on the last close.

- [x] **Step 4: Run the tests** -- same command; expected: all `identity` tests pass.
- [x] **Step 5: Clippy** `cargo clippy -p server --all-targets -- -D warnings > /tmp/c.log 2>&1; echo $?` -- expected `0`.
- [x] **Step 6: Commit** `feat(server): identity store over SQLite` (message names the rusqlite version).

---

### Task 3: Entitlements, server options and per-account spaces

This is the restructuring task: after it the server keeps each account's picture sources, frames, data-card specs and device configs in its own folder. Auth is still the old operator cookie here; Task 4 swaps it. To keep the tree green between the two tasks, **every existing route resolves to the instance owner's space** in this task (a temporary shim, `ServerState::operator_space()`, deleted in Task 4).

**Files:**
- Create: `companion/crates/server/src/entitlements.rs`, `companion/crates/server/src/accounts.rs`
- Modify: `companion/crates/server/src/lib.rs`, `src/registry.rs`, `src/data_cards.rs`, `src/data_cards/worker.rs`, `src/images.rs`, `src/app_api/mod.rs`, `src/admin.rs`, `src/device_link.rs`, `src/manage/mod.rs`, `src/main.rs`
- Modify: `companion/crates/server/tests/support/mod.rs`, and any test that builds state and mints devices
- Test: unit tests in `accounts.rs`, `registry.rs`; existing suites stay green

**Interfaces:**
- Consumes: `IdentityStore`, `AccountId`, `DeviceState` (Task 2).
- Produces:

```rust
// entitlements.rs
pub enum Edition { SelfHosted, Hosted }          // serialises "self-hosted" / "hosted"
pub trait Entitlements: Send + Sync + 'static {
    fn max_cards(&self, account: &AccountId) -> usize;
    fn max_image_sources(&self, account: &AccountId) -> usize;
    fn max_panels(&self, account: &AccountId) -> Option<usize>; // None = unlimited
    fn feature_enabled(&self, account: &AccountId, feature: &str) -> bool;
}
pub struct SelfHosted;   // MAX_CONFIG_CARDS, MAX_IMAGE_SOURCES, None, true

// lib.rs
pub struct ServerOptions {
    pub public_url: url::Url,                     // default "https://deskmate.test/"
    pub edition: Edition,                         // default SelfHosted
    pub entitlements: Arc<dyn Entitlements>,      // default Arc::new(SelfHosted)
    pub signups_default: bool,                    // default false
    pub extra_routes: Option<axum::Router<ServerState>>, // merged by app_with_web; for the hosted binary
}
impl ServerState {
    pub fn new_with_options(admin_token: String, firmware: FirmwareCatalog, config_directory: PathBuf, options: ServerOptions) -> Self;
    pub(crate) fn identity(&self) -> &IdentityStore;
    pub(crate) fn account_space(&self, account: &AccountId) -> Arc<AccountSpace>; // opens lazily, cached
    pub(crate) fn drop_account_space(&self, account: &AccountId) -> Option<Arc<AccountSpace>>;
    pub(crate) fn space_for_device(&self, device_id: &str) -> Option<Arc<AccountSpace>>; // via device_owners
    pub(crate) fn public_url(&self) -> &url::Url;
    pub(crate) fn entitlements(&self) -> &dyn Entitlements;
    pub(crate) fn edition(&self) -> Edition;
}

// accounts.rs
pub(crate) struct AccountSpace {
    pub account_id: AccountId,
    pub root: PathBuf,                                   // <config>/accounts/<id>
    pub image_sources: Arc<ImageSourceStore>,            // ImageSourceStore::new(root)
    pub data_cards: Mutex<DataCardSpecs>,                // root/data-cards.json
    pub configs: DeviceConfigStores,                     // DeviceConfigStores::new(root/devices)
}

// registry.rs
pub fn revoke(&self, device_id: &str) -> Result<bool, RegistryError>;
```

`ServerOptions` gains `mailer: Arc<dyn Mailer>` in Task 5, not here. `app_with_web` merges `options.extra_routes` (if any) inside the middleware layer, next to the other API routers.

- [x] **Step 1: Write the failing tests.**

In `accounts.rs`:

```rust
#[test]
fn account_space_lives_under_its_own_folder_and_is_cached() {
    let state = ServerState::in_memory();
    let owner = state.identity().create_account("a@example.com", true, true, Utc::now()).unwrap();
    let space = state.account_space(&owner.id);
    assert!(space.root.ends_with(format!("accounts/{}", owner.id.0)));
    assert!(Arc::ptr_eq(&space, &state.account_space(&owner.id)));
    assert_eq!(space.configs.for_device("dev-0001").store.path(),
               space.root.join("devices/dev-0001.json"));
}

#[test]
fn space_for_device_follows_ownership() {
    let state = ServerState::in_memory();
    let a = state.identity().create_account("a@example.com", true, true, Utc::now()).unwrap();
    assert!(state.space_for_device("dev-0001").is_none());
    state.identity().assign_device("dev-0001", &a.id, DeviceState::Active, Utc::now()).unwrap();
    assert_eq!(state.space_for_device("dev-0001").unwrap().account_id, a.id);
}
```

In `registry.rs` tests:

```rust
#[test]
fn revoke_removes_the_identity_and_persists() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(DEVICE_IDENTITY_STORE_FILE);
    let registry = Registry::load(&path);
    let minted = registry.mint().unwrap();
    assert!(registry.revoke(&minted.device_id).unwrap());
    assert!(registry.authenticate(&minted.token).is_none());
    assert!(!Registry::load(&path).contains_device(&minted.device_id));
    assert!(!registry.revoke(&minted.device_id).unwrap(), "second revoke is a no-op");
}

#[test]
fn high_water_scan_sees_per_account_device_files() {
    let dir = tempfile::tempdir().unwrap();
    let devices = dir.path().join("accounts/acc_x/devices");
    std::fs::create_dir_all(&devices).unwrap();
    std::fs::write(devices.join("dev-0042.json"), b"{}").unwrap();
    std::fs::write(dir.path().join(DEVICE_IDENTITY_STORE_FILE), b"not json").unwrap();
    let registry = Registry::load(dir.path().join(DEVICE_IDENTITY_STORE_FILE));
    assert_eq!(registry.mint().unwrap().device_id, "dev-0043");
}
```

In `lib.rs` tests (notify filtering) -- extend the existing `image_notifications` test hook: record the device ids notified, and assert that a source change in account A's space notifies A's linked device and not B's. Use `tests/support` helpers from Step 3 if simpler as an integration test in `tests/image_routes.rs`:

```rust
#[tokio::test]
async fn a_picture_push_wakes_only_the_owning_accounts_panels() {
    let state = ServerState::in_memory();
    let a = support::owner_account(&state);
    let b = support::second_account(&state, "b@example.com");
    let dev_a = support::mint_owned_device(&state, &a);
    let dev_b = support::mint_owned_device(&state, &b);
    let (server, mut sock_a, mut sock_b) = support::link_both(&state, &dev_a, &dev_b).await;
    let minted = support::mint_image_source_for(&state, &a, "Weather");
    support::push_png(&server, &minted.token).await;
    support::drive_until_asset_begin(&mut sock_a).await;       // A receives the frame
    support::assert_no_asset_begin_within(&mut sock_b, Duration::from_millis(300)).await;
}
```

(The helper names in that test are defined in Step 3; `drive_until_asset_begin`/`assert_no_asset_begin_within` are thin wrappers over the existing `DeviceSocket` reading loop in `support/mod.rs`.)

- [x] **Step 2: Run to verify they fail** (`cargo test -p server > /tmp/t.log 2>&1; echo $?`) -- compile errors.
- [x] **Step 3: Implement.**
  1. `entitlements.rs` as in Interfaces. `app_core` limits (`MAX_CONFIG_CARDS`, `MAX_IMAGE_SOURCES`) stay the hard ceiling; the trait can only lower them. The store-level checks remain; add an entitlement check where a source is minted (`images.rs::mint_source`) and where a config is saved (`app_api::save_config`), answering `AppApiError::Validation` with an issue at path `cards` / `422` for sources: "This account can have at most N picture sources."
  2. `ServerState`: add `identity: IdentityStore` (opened at `config_directory.join("identity.db")`; in `in_memory*` inside the tempdir), `accounts: Mutex<HashMap<AccountId, Arc<AccountSpace>>>`, `options: ServerOptions` fields. **Remove** `image_sources`, `configs`, and the spec half of `data_cards` from `StateInner`. `new(...)` becomes `new_with_options(..., ServerOptions::default())`.
  3. Data cards: split `DataCardState` into a global `FaceCatalogState { faces, catalog, catalog_reloader }` kept on `StateInner`, and per-account `DataCardSpecs { spec_path, specs, tasks, outcomes }` on `AccountSpace`. `start_data_cards(state)` now iterates `identity().accounts()` and calls `start_account_data_cards(state, &space)`; `account_space()` calls it for a space it opens for the first time after startup. `worker::spawn_refresher` takes `Arc<AccountSpace>` and uses `space.image_sources` for `accept`, and `notify_image_source_changed(space.account_id, ...)`. `stop_refreshers` stops every open space's tasks. `DESKMATE_DATA_CARDS` and `main.rs::data_card_spec_path` are deleted.
  4. `notify_image_source_changed(&self, account: &AccountId, source_id, digest, origin)`: iterate `device_links`, and notify only links whose `LiveLink.account_id == *account`. Add `account_id: AccountId` to `LiveLink`, set when the link is created in `device_link.rs`.
  5. `device_link.rs`: after `AuthenticatedDevice`, resolve `state.space_for_device(&device_id)`; `None` -> `401` with a rate-limited `warn!(target: "deskmate_server::device_link", ...)`. Then `identity().activate_device(&device_id)` (a no-op for active), and build the runtime from `space.configs.for_device(..)` and `ServerImageSourceHost::new(space.image_sources.clone())`.
  6. `registry.rs`: `revoke` removes the entry under the lock and persists with the existing save path; `scan_config_high_water` also scans `accounts/*/devices/dev-NNNN.json`.
  7. **Temporary shim**: `pub(crate) fn operator_space(&self) -> Result<Arc<AccountSpace>, AppApiError>` returns the instance owner's space (the first `is_instance_owner` account) or `NotFound("set up this server first")`. Every current route that used `image_sources()`/`configs()` calls it. `admin::create_device` mints, then `assign_device(.., instance owner, Active, now)`, and answers `409 {"error": "set up this server first"}` when there is no instance owner. `list_devices` lists `devices_for(owner)`.
  8. `tests/support/mod.rs`: add

```rust
pub struct TestAccount { pub id: AccountId, pub cookie: String /* "__Host-deskmate_session=<sid>" */ }
pub fn owner_account(state: &ServerState) -> TestAccount;            // creates instance owner if missing, new session
pub fn second_account(state: &ServerState, email: &str) -> TestAccount;
pub fn mint_owned_device(state: &ServerState, account: &TestAccount) -> DeviceIdentity; // registry mint + assign Active
```

  `AccountId` and the identity accessors must be reachable from integration tests: expose them behind the existing `test-support`-style pattern the crate already uses for test hooks (if none exists for the server crate, add `#[doc(hidden)] pub` accessors -- `identity()` and `account_space()` -- and note it in the commit). `cookie` stays unused until Task 4 but is minted now so Task 4 only changes the extractor.
  9. Update every existing test that calls `state.registry().mint()` or `POST /v1/devices` before an owner exists to call `owner_account` first (`ownership.rs::spawn_state`, `companion_api.rs`, `hostile_device.rs`, `image_routes.rs`, `device_link.rs`, `manage.rs`, `oauth_routes.rs`). Tests that reload by building a second `ServerState::new` on the same directory keep working because the identity db is in that directory.
- [x] **Step 4: Run the full server suite** `cargo test -p server --all-targets > /tmp/t.log 2>&1; echo $?` -- expected `0`. Then the workspace gates from `CLAUDE.md` (fmt, clippy, both test invocations).
- [x] **Step 5: Commit** `feat(server): per-account spaces and the entitlements seam`.

---

### Task 4: Account sessions replace the operator cookie

**Files:**
- Create: `companion/crates/server/src/web_auth/mod.rs`, `src/web_auth/rate_limit.rs`
- Modify: `src/oauth/session.rs` (delete `SessionSigner`, `SessionClaims`, `OperatorAuthenticated`, `new_sid`; keep nothing that `web_auth` does not need), `src/oauth/routes.rs` (delete `POST /v1/session/login`; `start_google`/callback/revoke/producer routes take `InstanceOwner`; the pending stash keys on the session digest instead of `sid`), `src/manage/mod.rs` (delete `GET/POST /v1/manage/login`; handlers take `InstanceOwner`; a missing session redirects to `/`), `src/app_api/mod.rs` (delete `POST /v1/app/session`; `DELETE /v1/app/session` deletes the session row; every handler takes `AccountSession` and resolves within `account_space(&session.account.id)`), `src/images.rs` (Operator -> `AccountSession`, resolve source ids within the account), `src/lib.rs` (remove `sessions` field and the `operator_space` shim), `src/main.rs` (serve with `into_make_service_with_connect_info::<SocketAddr>()`)
- Create: `companion/crates/server/tests/isolation.rs`
- Modify: every integration test that used the admin bearer or the old cookie against `/v1/app/*`, `/v1/images`, `/v1/manage`, `/v1/integrations` -- switch to `TestAccount.cookie` + an `Origin` header
- Modify: `crates/server/tests/support/mod.rs` (`spawn_http` serves with connect info)

**Interfaces:**
- Consumes: `IdentityStore::{session_account, delete_session}`, `ServerState::{account_space, public_url}`.
- Produces:

```rust
pub(crate) const SESSION_COOKIE: &str = "__Host-deskmate_session";
pub(crate) fn set_session_cookie(plaintext_sid: &str) -> HeaderValue;     // Max-Age = 30 days
pub(crate) fn clear_session_cookie() -> HeaderValue;                     // Max-Age=0
pub(crate) struct AccountSession { pub account: Account, pub session_plaintext: String }
pub(crate) struct InstanceOwner(pub AccountSession);                     // 404 when not the owner
pub(crate) struct SameOrigin;           // rejects unsafe methods whose Origin != public origin (403)
pub(crate) struct ClientIp(pub IpAddr); // peer, or right-most X-Forwarded-For entry when peer is loopback
pub(crate) struct RateLimiter { /* Mutex<HashMap<String, VecDeque<Instant>>> */ }
impl RateLimiter {
    pub fn new(limit: usize, window: Duration) -> Self;
    pub fn check(&self, key: &str, now: Instant) -> Result<(), Duration>;  // Err(retry_after)
    pub fn record(&self, key: &str, now: Instant);
}
```

`AccountSession` rejects with `401` and an empty body when there is no valid session; it also enforces `SameOrigin` for unsafe methods, so no handler can forget it. `InstanceOwner` answers `404` for a non-owner session (the owner-only surface is not an oracle either).

- [x] **Step 1: Write the failing tests.** `tests/isolation.rs` is the centrepiece:

```rust
//! Account B must not see account A's resources through any route that takes an id.
mod support;
use support::*;

struct World { server: HttpTestServer, a: TestAccount, b: TestAccount, dev_a: String, source_a: String }

async fn world() -> World {
    let state = with_fake_faces(ServerState::in_memory());
    let a = owner_account(&state);
    let b = second_account(&state, "b@example.com");
    let dev_a = mint_owned_device(&state, &a).device_id;
    let source_a = mint_image_source_for(&state, &a, "Weather").id;
    let server = spawn_http(app(state)).await;
    World { server, a, b, dev_a, source_a }
}

#[tokio::test]
async fn account_b_gets_404_for_every_route_naming_account_a_resources() {
    let w = world().await;
    let d = &w.dev_a; let s = &w.source_a;
    let cases: Vec<(&str, String, Option<serde_json::Value>)> = vec![
        ("GET",  format!("/v1/app/{d}/snapshot"), None),
        ("GET",  format!("/v1/app/{d}/events"), None),
        ("PUT",  format!("/v1/app/{d}/config"), Some(support::valid_config_json())),
        ("POST", format!("/v1/app/{d}/config/validate"), Some(support::valid_config_json())),
        ("POST", format!("/v1/app/{d}/preview"), Some(serde_json::json!({"card_id": "clock"}))),
        ("POST", format!("/v1/app/{d}/pomodoro"), Some(serde_json::json!({"card_id": "p", "action": "start"}))),
        ("DELETE", format!("/v1/app/devices/{d}"), None),
        ("DELETE", format!("/v1/images/{s}"), None),
        ("PUT",  format!("/v1/images/{s}/face"), Some(serde_json::json!({"kind": "weather"}))),
    ];
    for (method, path, body) in cases {
        let status = cookie_request(&w.server, &w.b, method, &path, body).await.status();
        assert_eq!(status, 404, "{method} {path} leaked to account B");
    }
}

#[tokio::test]
async fn lists_are_scoped_to_the_signed_in_account() {
    let w = world().await;
    let devices: Vec<serde_json::Value> = cookie_request(&w.server, &w.b, "GET", "/v1/app/devices", None).await.json().await.unwrap();
    assert!(devices.is_empty());
    let sources: Vec<serde_json::Value> = cookie_request(&w.server, &w.b, "GET", "/v1/images", None).await.json().await.unwrap();
    assert!(sources.iter().all(|s| s["id"] != w.source_a));
}

#[tokio::test]
async fn owner_only_surfaces_are_404_for_other_accounts() {
    let w = world().await;
    for (m, p) in [("GET", "/v1/manage"), ("POST", "/v1/integrations/google"),
                   ("PUT", "/v1/app/instance/signups")] {
        assert_eq!(cookie_request(&w.server, &w.b, m, p, None).await.status(), 404, "{m} {p}");
    }
}
```

(`DELETE /v1/app/devices/{id}` and `PUT /v1/app/instance/signups` arrive in Tasks 7 and 5; add their rows to this file in those tasks, not now -- delete those two rows here and re-add them there.)

`tests/accounts.rs` (session part):

```rust
#[tokio::test]
async fn no_session_is_401_and_a_wrong_origin_is_403() {
    let state = ServerState::in_memory();
    let a = owner_account(&state);
    let server = spawn_http(app(state)).await;
    let client = reqwest::Client::new();
    assert_eq!(client.get(format!("{}/v1/app/devices", server.base_url)).send().await.unwrap().status(), 401);
    let resp = client.delete(format!("{}/v1/app/session", server.base_url))
        .header("cookie", &a.cookie).header("origin", "https://evil.example").send().await.unwrap();
    assert_eq!(resp.status(), 403);
    let resp = client.delete(format!("{}/v1/app/session", server.base_url))
        .header("cookie", &a.cookie).send().await.unwrap();
    assert_eq!(resp.status(), 403, "missing Origin on an unsafe method");
}

#[tokio::test]
async fn sign_out_deletes_the_session_server_side() {
    let state = ServerState::in_memory();
    let a = owner_account(&state);
    let server = spawn_http(app(state)).await;
    assert_eq!(cookie_request(&server, &a, "DELETE", "/v1/app/session", None).await.status(), 204);
    assert_eq!(cookie_request(&server, &a, "GET", "/v1/app/devices", None).await.status(), 401);
}

#[tokio::test]
async fn the_admin_bearer_is_not_a_browser_session() {
    let state = ServerState::in_memory();
    owner_account(&state);
    let server = spawn_http(app(state)).await;
    let resp = reqwest::Client::new().get(format!("{}/v1/app/devices", server.base_url))
        .bearer_auth(IN_MEMORY_ADMIN_TOKEN).send().await.unwrap();
    assert_eq!(resp.status(), 401);
    for path in ["/v1/app/session", "/v1/session/login", "/v1/manage/login"] {
        let resp = reqwest::Client::new().post(format!("{}{path}", server.base_url))
            .header("origin", "https://deskmate.test").json(&serde_json::json!({"token": IN_MEMORY_ADMIN_TOKEN}))
            .send().await.unwrap();
        assert!(matches!(resp.status().as_u16(), 404 | 405), "{path} still trades the admin token");
    }
}

#[tokio::test]
async fn a_second_sign_in_does_not_invalidate_the_first_session() {
    let state = ServerState::in_memory();
    let a = owner_account(&state);
    let again = support::new_session(&state, &a);   // another session for the same account
    let server = spawn_http(app(state)).await;
    assert_eq!(cookie_request(&server, &a, "GET", "/v1/app/devices", None).await.status(), 200);
    assert_eq!(cookie_request(&server, &again, "GET", "/v1/app/devices", None).await.status(), 200);
}
```

`web_auth/rate_limit.rs` unit test:

```rust
#[test]
fn sliding_window_allows_limit_then_reports_retry_after() {
    let limiter = RateLimiter::new(2, Duration::from_secs(60));
    let t0 = Instant::now();
    for _ in 0..2 { limiter.check("k", t0).unwrap(); limiter.record("k", t0); }
    let wait = limiter.check("k", t0 + Duration::from_secs(10)).unwrap_err();
    assert_eq!(wait, Duration::from_secs(50));
    assert!(limiter.check("k", t0 + Duration::from_secs(61)).is_ok());
    assert!(limiter.check("other", t0).is_ok());
}
```

`ClientIp` unit test (build `Parts` by hand):

```rust
#[test]
fn forwarded_for_is_trusted_only_from_loopback() {
    let from_proxy = client_ip(Some("127.0.0.1:5000".parse().unwrap()), Some("198.51.100.7, 203.0.113.9"));
    assert_eq!(from_proxy, "203.0.113.9".parse::<IpAddr>().unwrap(), "right-most hop, appended by our proxy");
    let direct = client_ip(Some("192.0.2.1:5000".parse().unwrap()), Some("203.0.113.9"));
    assert_eq!(direct, "192.0.2.1".parse::<IpAddr>().unwrap(), "a remote peer cannot spoof");
    assert_eq!(client_ip(None, None), IpAddr::from([0, 0, 0, 0]));
}
```

(`client_ip(peer: Option<SocketAddr>, forwarded: Option<&str>) -> IpAddr` is the pure function the extractor calls.)

- [x] **Step 2: Run to verify they fail.**
- [x] **Step 3: Implement** per Interfaces. Notes:
  - Cookie parsing: take the first `__Host-deskmate_session=` value from any `Cookie` header, as `session_cookie_value` did.
  - `SameOrigin`: compare the `Origin` header string to `public_url().origin().ascii_serialization()`; `GET`/`HEAD`/`OPTIONS` pass.
  - Resolution inside an account: `app_api`'s `known_device` becomes `owned_device(&state, &session, &device_id) -> Result<Arc<AccountSpace>, AppApiError>`, checking `identity().device_owner(id)` is this account (else `NotFound { message: "no device with id ..." }` -- the same message whether it exists elsewhere or nowhere). `images.rs` resolves `{id}` through `space.image_sources.summaries()`; the push route (`POST /v1/images/{key}`, producer bearer) authenticates **across** accounts: add `ServerState::authenticate_image_producer(token) -> Option<(Arc<AccountSpace>, String)>` that tries each account's store (open all spaces at startup via `start_data_cards`, so this is a scan over open spaces).
  - The oauth callback's pending stash was keyed by `sid`; key it by the session digest hex instead, so a different session cannot complete someone else's consent.
  - Delete `SessionSigner` and the HMAC use of the admin token; `hmac` stays only if something else uses it (check with `cargo udeps`-free grep; remove the dependency if unused).
  - Update `app_api/mod.rs`'s module doc and `src/lib/types.contract.ts` regeneration if a DTO changed (run the ignored `print_typescript_contract_fixture` test and copy its output).
- [x] **Step 4: Run the full gates**, then **mutation probe**: temporarily replace the ownership comparison in `owned_device` with `true`, run `cargo test -p server --test isolation`, confirm it FAILS, restore. Do the same for the `SameOrigin` comparison against `tests/accounts.rs`. Record both results in the commit message.
- [x] **Step 5: Commit** `feat(server): account sessions replace the admin-token cookie`.

---

### Task 5: Mail, email-link sign-in, first-run setup, sign-ups and recovery

**Files:**
- Modify: `companion/crates/server/Cargo.toml` (add `lettre = { version = "0.11", default-features = false, features = ["builder", "smtp-transport", "tokio1", "tokio1-rustls", "ring", "webpki-roots"] }` -- confirm the feature names against the resolved lettre version with `cargo add`; the requirement is SMTP over rustls on tokio with no OpenSSL)
- Create: `src/mailer.rs`, `src/web_auth/setup_code.rs`, `src/web_auth/routes.rs`
- Modify: `src/lib.rs` (`ServerOptions.mailer`, `setup_code` on `StateInner`, rate limiters on `StateInner`, merge `web_auth::routes::routes()`), `tests/accounts.rs`, `tests/isolation.rs` (re-add the `PUT /v1/app/instance/signups` row)

**Interfaces:**
- Produces:

```rust
pub trait Mailer: Send + Sync + 'static {
    fn send_sign_in_link(&self, to: &str, link: &str) -> Pin<Box<dyn Future<Output = Result<(), MailError>> + Send + '_>>;
}
pub struct LogMailer;                 // info!(target: "deskmate_server::mail", ...) in a marked block
pub struct SmtpMailer { /* lettre AsyncSmtpTransport<Tokio1Executor>, from: Mailbox */ }
impl SmtpMailer { pub fn from_url(smtp_url: &str, from: &str) -> Result<Self, MailError>; }
#[cfg(test)] pub struct RecordingMailer { pub sent: Mutex<Vec<(String, String)>> } // exposed to integration tests the same way as Task 3's accessors

pub(crate) struct SetupCode { /* Mutex<Option<String>> */ }
impl SetupCode {
    pub fn generate() -> (Self, String);           // returns the display form XXXX-XXXX
    pub fn matches(&self, presented: &str) -> bool; // strips '-' and whitespace, uppercases, constant-time
    pub fn erase(&self);
    pub fn is_active(&self) -> bool;
}
```

Routes (all JSON; all unsafe ones take `SameOrigin`; all take `ClientIp` where a limit applies):

| Route | Behaviour |
|---|---|
| `GET /v1/app/instance` | `{setup_required, google_enabled, signups_open, edition}`; no auth |
| `POST /v1/app/setup {code, email}` | 404 once any account exists; 429 after 5 failures/IP/15 min; wrong code 400 `{"error":"That setup code is not right."}`; success creates the instance owner (unverified), erases the code, sets the cookie, 200 `{account}` |
| `POST /v1/app/auth/email {email}` | invalid email 400; else **always 202 `{}`**; mails only if the account exists or sign-ups are open; limits 5/address/hour, 20/IP/hour -> 429 `{"error": "...", "retry_after_seconds": n}` |
| `POST /v1/app/auth/link {token}` | 429 after 5 failures/IP/15 min; invalid/used/expired 400 `{"error":"This sign-in link has expired."}`; valid: create the account if new (sign-ups must still be open, else 400 same sentence), mark verified, cookie, 200 `{account}` |
| `GET /v1/app/account` | `AccountSession` -> `{id, email, email_verified, is_instance_owner}` |
| `POST /v1/app/sessions/revoke-all` | deletes all the account's sessions, clears the cookie, 204 |
| `PUT /v1/app/instance/signups {open}` | `InstanceOwner`; stores the flag, 200 `{signups_open}` |
| `POST /v1/admin/signin-link {email}` | `AdminAuthenticated`; 404 for an unknown account; 200 `{link}` |

`signups_open` = `identity().signups_open()?.unwrap_or(options.signups_default)`. The link is `public_url.join("/signin?token=<t>")`.

- [x] **Step 1: Write the failing tests** in `tests/accounts.rs` (state built with `RecordingMailer`; helper `support::state_with_mailer() -> (ServerState, Arc<RecordingMailer>)`; the setup code is read back via a `#[doc(hidden)] pub fn setup_code_for_tests(&self) -> Option<String>` on `ServerState`):

```rust
#[tokio::test]
async fn first_run_setup_creates_the_owner_once() {
    let (state, _mail) = state_with_mailer();
    let code = state.setup_code_for_tests().unwrap();
    let server = spawn_http(app(state)).await;
    let inst: Value = get_json(&server, "/v1/app/instance").await;
    assert_eq!(inst["setup_required"], true);
    assert_eq!(post_json(&server, "/v1/app/setup", json!({"code": "WRONG-CODE", "email": "o@example.com"})).await.status(), 400);
    let ok = post_json(&server, "/v1/app/setup", json!({"code": code.to_lowercase(), "email": "o@example.com"})).await;
    assert_eq!(ok.status(), 200);
    assert!(ok.headers()["set-cookie"].to_str().unwrap().starts_with("__Host-deskmate_session="));
    assert_eq!(post_json(&server, "/v1/app/setup", json!({"code": code, "email": "x@example.com"})).await.status(), 404);
    assert_eq!(get_json::<Value>(&server, "/v1/app/instance").await["setup_required"], false);
}

#[tokio::test]
async fn setup_code_guesses_are_rate_limited() {
    let (state, _m) = state_with_mailer();
    let server = spawn_http(app(state)).await;
    for _ in 0..5 { post_json(&server, "/v1/app/setup", json!({"code": "AAAA-AAAA", "email": "o@example.com"})).await; }
    assert_eq!(post_json(&server, "/v1/app/setup", json!({"code": "AAAA-AAAA", "email": "o@example.com"})).await.status(), 429);
}

#[tokio::test]
async fn sign_in_email_does_not_reveal_accounts_and_respects_closed_signups() {
    let (state, mail) = state_with_mailer();
    owner_account(&state);                              // sign-ups default closed
    let server = spawn_http(app(state)).await;
    let known = post_json(&server, "/v1/app/auth/email", json!({"email": "OWNER@example.com "})).await;
    let unknown = post_json(&server, "/v1/app/auth/email", json!({"email": "stranger@example.com"})).await;
    assert_eq!((known.status(), unknown.status()), (StatusCode::ACCEPTED, StatusCode::ACCEPTED));
    assert_eq!(known.text().await.unwrap(), unknown.text().await.unwrap());
    let sent = mail.sent.lock().unwrap();
    assert_eq!(sent.len(), 1, "only the existing account is mailed");
    assert_eq!(sent[0].0, "owner@example.com");
    assert!(sent[0].1.starts_with("https://deskmate.test/signin?token="));
}

#[tokio::test]
async fn a_link_signs_in_once_and_verifies_the_email() {
    let (state, mail) = state_with_mailer();
    owner_account(&state);
    let server = spawn_http(app(state)).await;
    post_json(&server, "/v1/app/auth/email", json!({"email": "owner@example.com"})).await;
    let token = token_from_link(&mail.sent.lock().unwrap()[0].1);
    let first = post_json(&server, "/v1/app/auth/link", json!({"token": token})).await;
    assert_eq!(first.status(), 200);
    let second = post_json(&server, "/v1/app/auth/link", json!({"token": token})).await;
    assert_eq!(second.status(), 400);
    assert_eq!(second.json::<Value>().await.unwrap()["error"], "This sign-in link has expired.");
}

#[tokio::test]
async fn open_signups_create_an_account_from_a_link() {
    let (state, mail) = state_with_mailer();
    let owner = owner_account(&state);
    let server = spawn_http(app(state)).await;
    assert_eq!(cookie_request(&server, &owner, "PUT", "/v1/app/instance/signups", Some(json!({"open": true}))).await.status(), 200);
    post_json(&server, "/v1/app/auth/email", json!({"email": "friend@example.com"})).await;
    let token = token_from_link(&mail.sent.lock().unwrap()[0].1);
    let resp = post_json(&server, "/v1/app/auth/link", json!({"token": token})).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.json::<Value>().await.unwrap()["account"]["is_instance_owner"], false);
}

#[tokio::test]
async fn sign_in_emails_are_rate_limited_per_address() {
    let (state, mail) = state_with_mailer();
    owner_account(&state);
    let server = spawn_http(app(state)).await;
    for _ in 0..5 { post_json(&server, "/v1/app/auth/email", json!({"email": "owner@example.com"})).await; }
    let resp = post_json(&server, "/v1/app/auth/email", json!({"email": "owner@example.com"})).await;
    assert_eq!(resp.status(), 429);
    assert_eq!(mail.sent.lock().unwrap().len(), 5);
}

#[tokio::test]
async fn recovery_link_needs_the_admin_bearer() {
    let (state, _m) = state_with_mailer();
    owner_account(&state);
    let server = spawn_http(app(state)).await;
    let c = reqwest::Client::new();
    let url = format!("{}/v1/admin/signin-link", server.base_url);
    assert_eq!(c.post(&url).json(&json!({"email": "owner@example.com"})).send().await.unwrap().status(), 401);
    let ok = c.post(&url).bearer_auth(IN_MEMORY_ADMIN_TOKEN).json(&json!({"email": "owner@example.com"})).send().await.unwrap();
    let link = ok.json::<Value>().await.unwrap()["link"].as_str().unwrap().to_string();
    assert_eq!(post_json(&server, "/v1/app/auth/link", json!({"token": token_from_link(&link)})).await.status(), 200);
}
```

`post_json` sends `Origin: https://deskmate.test`. `setup_code.rs` unit test: `generate()` yields `^[2-9A-HJKMNP-Z]{4}-[2-9A-HJKMNP-Z]{4}$`, `matches` accepts lowercase and no dash, rejects after `erase`.

- [x] **Step 2: Run to verify they fail.**
- [x] **Step 3: Implement.** Setup code is generated in `new_with_options` when `account_count() == 0`, and logged there:

```rust
tracing::warn!(target: "deskmate_server::setup",
    "\n==============================================\n  Deskmate setup code: {code}\n  Open {url} to set up this server.\n==============================================",
    url = options.public_url);
```

`LogMailer` logs the same shape with the link. Rate-limit keys: `"email:<normalized>"`, `"ip-email:<ip>"`, `"ip-fail:<ip>"` (setup and link failures share the failure bucket). Mail sending happens on a spawned task so the 202 does not wait on SMTP; a failure is `warn!`ed.
- [x] **Step 4: Gates**, then **mutation probes**: (a) remove the `used_at IS NULL` clause in `consume_login_token` -> `a_link_signs_in_once...` must fail; (b) make the per-address limiter `check` always `Ok` -> `sign_in_emails_are_rate_limited...` must fail. Restore, record results in the commit.
- [x] **Step 5: Commit** `feat(server): email-link sign-in, first-run setup code and recovery link`.

---

### Task 6: Sign in with Google

**Files:**
- Create: `src/web_auth/google.rs`
- Modify: `src/web_auth/routes.rs` (merge routes), `src/lib.rs` (`google_sign_in: Option<GoogleSignIn>` on `StateInner`, set by a `set_google_sign_in` like `set_integrations`), `src/main.rs` (build it when Google is configured)
- Test: unit tests in `google.rs`; integration tests in `tests/accounts.rs`

**Interfaces:**
- Consumes: `oauth::transport::OAuthTransport`, `oauth::pkce::{generate_pkce, generate_state}` (make them `pub(crate)`), `GoogleOAuthConfig { client_id, client_secret, token_uri, auth_uri, .. }`.
- Produces:

```rust
pub(crate) struct GoogleSignIn { config: GoogleOAuthConfig, transport: Arc<dyn OAuthTransport>, pending: Mutex<HashMap<String, (String /*verifier*/, DateTime<Utc>)>> }
pub(crate) struct GoogleClaims { pub sub: String, pub email: String }
pub(crate) fn claims_from_id_token(id_token: &str, client_id: &str, now: DateTime<Utc>) -> Result<GoogleClaims, &'static str>;
// routes: GET /v1/app/auth/google/start  -> 303 to Google (scope "openid email", redirect_uri = public_url + "/v1/app/auth/google/callback")
//         GET /v1/app/auth/google/callback?state&code -> 303 "/" with cookie, or 303 "/?signin_error=<reason>"
```

- [x] **Step 1: Write the failing tests.**

```rust
fn id_token(claims: serde_json::Value) -> String {
    use base64::prelude::{BASE64_URL_SAFE_NO_PAD as B, Engine as _};
    format!("{}.{}.sig", B.encode(br#"{"alg":"RS256"}"#), B.encode(claims.to_string()))
}
#[test]
fn accepts_a_verified_email_for_this_client_only() {
    let now = Utc::now();
    let good = json!({"iss": "https://accounts.google.com", "aud": "cid", "sub": "123",
                      "email": "A@Example.com", "email_verified": true, "exp": now.timestamp() + 60});
    let claims = claims_from_id_token(&id_token(good.clone()), "cid", now).unwrap();
    assert_eq!((claims.sub.as_str(), claims.email.as_str()), ("123", "a@example.com"));
    for (field, value) in [("aud", json!("other")), ("iss", json!("https://evil.example")),
                           ("email_verified", json!(false)), ("exp", json!(now.timestamp() - 1))] {
        let mut bad = good.clone(); bad[field] = value;
        assert!(claims_from_id_token(&id_token(bad), "cid", now).is_err(), "{field}");
    }
    assert!(claims_from_id_token("not-a-jwt", "cid", now).is_err());
}
```

Integration (fake transport answering the token POST with `{"id_token": ..., "access_token": "x", "expires_in": 3600}`):

```rust
#[tokio::test]
async fn google_sign_in_links_to_the_account_with_the_same_verified_email() {
    let (state, server, transport) = google_world(/* signups closed */).await; // owner o@example.com exists
    let start = no_redirect_get(&server, "/v1/app/auth/google/start").await;
    let state_param = query_param(start.headers()["location"].to_str().unwrap(), "state");
    transport.answer_id_token(json!({"sub": "g-1", "email": "o@example.com", "email_verified": true, ...}));
    let cb = no_redirect_get(&server, &format!("/v1/app/auth/google/callback?state={state_param}&code=c")).await;
    assert_eq!(cb.headers()["location"], "/");
    assert!(cb.headers()["set-cookie"].to_str().unwrap().starts_with("__Host-deskmate_session="));
    assert!(state.identity().account_for_google("g-1").unwrap().is_some());
}

#[tokio::test]
async fn google_sign_in_does_not_create_accounts_while_signups_are_closed() {
    // same, email stranger@example.com -> location "/?signin_error=signups-closed", no cookie
}

#[tokio::test]
async fn a_callback_with_an_unknown_state_is_refused() {
    // callback?state=nope&code=c -> "/?signin_error=expired", and the transport saw no call
}
```

`GET /v1/app/instance` reports `google_enabled: true` only when `set_google_sign_in` was called -- assert both ways.
- [x] **Step 2: Run to verify they fail.**
- [x] **Step 3: Implement.** `claims_from_id_token`: split on `.`, require 3 parts, base64url-decode (no padding) part 2, parse JSON, check `iss in ["https://accounts.google.com", "accounts.google.com"]`, `aud == client_id` (string, or array containing it), `exp > now`, `email_verified == true` (bool or `"true"`), `normalize_email(email)`. Doc-comment the reason there is no signature check: the token came straight from `token_uri` over TLS through `egress`, which OpenID Connect Core 3.1.3.7 permits. Pending entries expire after 600 s and are capped at 32, like `IntegrationRuntime`. Callback failures count toward the `ip-fail` bucket. Lookup order: `account_for_google(sub)` -> `account_by_email(email)` then `link_google` -> create (if sign-ups open) then `link_google` -> else `signups-closed`. Mark the email verified.
- [x] **Step 4: Gates.**
- [x] **Step 5: Commit** `feat(server): sign in with Google`.

---

### Task 7: Claiming, panels and account deletion

**Files:**
- Create: `src/claim.rs`
- Modify: `src/app_api/mod.rs` (the `GET /v1/app/devices` handler moves to `claim.rs`; `DeviceRow` gains `state: "pending" | "active"`), `src/lib.rs` (router, `close_link(device_id)`), `src/web_auth/routes.rs` (`DELETE /v1/app/account`), `src/main.rs` (housekeeping task), `apps/deskmate/src/lib/types.contract.ts` (regenerate)
- Test: `tests/accounts.rs`, `tests/isolation.rs` (re-add the `DELETE /v1/app/devices/{id}` row)

**Interfaces:**

| Route | Behaviour |
|---|---|
| `GET /v1/app/devices` | the account's panels: `{id, connected, has_saved_config, configured_at, state}` |
| `POST /v1/app/devices/claim` | checks `entitlements().max_panels`; `registry().mint()`; `assign_device(.., Pending, now)`; 201 `{device_id, token, link_url}` where `link_url` = `public_url` with scheme `wss` (or `ws` for `http`) and path `/v1/device/link` |
| `DELETE /v1/app/devices/{id}` | owned check (404); `registry().revoke`; `release_device`; delete `space.root/devices/<id>.json`; `close_link(id)`; 204 |
| `DELETE /v1/app/account` | `AccountSession`; the instance owner gets 409 `{"error": "Other accounts use this server. Remove them first."}` while `account_count() > 1`; otherwise: for each owned device revoke + close link; stop the space's data-card tasks; `drop_account_space`; `identity().delete_account`; remove `accounts/<id>` recursively; clear the cookie; 204 |

```rust
pub(crate) async fn collect_stale_claims(state: &ServerState, now: DateTime<Utc>) -> usize; // revoke + release pending older than 24 h
pub(crate) fn spawn_housekeeping(state: ServerState) -> JoinHandle<()>; // every hour: collect_stale_claims; stops on shutdown
```

- [x] **Step 1: Write the failing tests.**

```rust
#[tokio::test]
async fn a_claimed_panel_is_pending_until_it_links_then_active() {
    let state = ServerState::in_memory();
    let a = owner_account(&state);
    let server = spawn_http(app(state.clone())).await;
    let claim: Value = cookie_request(&server, &a, "POST", "/v1/app/devices/claim", None).await.json().await.unwrap();
    assert_eq!(claim["link_url"], "wss://deskmate.test/v1/device/link");
    let rows: Vec<Value> = cookie_request(&server, &a, "GET", "/v1/app/devices", None).await.json().await.unwrap();
    assert_eq!(rows[0]["state"], "pending");
    let _sock = connect_device(&server.host(), claim["token"].as_str().unwrap()).await;
    wait_until(|| async { cookie_request(&server, &a, "GET", "/v1/app/devices", None).await
        .json::<Vec<Value>>().await.unwrap()[0]["state"] == "active" }).await;
}

#[tokio::test]
async fn stale_pending_claims_are_revoked_after_a_day() {
    let state = ServerState::in_memory();
    let a = owner_account(&state);
    let pending = support::claim_pending(&state, &a);        // mint + assign Pending at now - 25h
    assert_eq!(claim::collect_stale_claims(&state, Utc::now()).await, 1);
    assert!(!state.registry().contains_device(&pending.device_id));
}

#[tokio::test]
async fn removing_a_panel_revokes_it_and_drops_its_live_link() {
    let state = ServerState::in_memory();
    let a = owner_account(&state);
    let dev = mint_owned_device(&state, &a);
    let server = spawn_http(app(state.clone())).await;
    let mut sock = connect_device(&server.host(), &dev.token).await;
    assert_eq!(cookie_request(&server, &a, "DELETE", &format!("/v1/app/devices/{}", dev.device_id), None).await.status(), 204);
    support::expect_socket_closed(&mut sock).await;
    assert!(support::try_connect_device(&server.host(), &dev.token).await.is_err(), "revoked token refused");
}

#[tokio::test]
async fn deleting_an_account_removes_its_folder_and_refuses_its_panel() {
    let state = ServerState::in_memory();
    let owner = owner_account(&state);
    let b = second_account(&state, "b@example.com");
    let dev = mint_owned_device(&state, &b);
    let root = state.account_space(&b.id).root.clone();
    let server = spawn_http(app(state.clone())).await;
    assert_eq!(cookie_request(&server, &owner, "DELETE", "/v1/app/account", None).await.status(), 409);
    assert_eq!(cookie_request(&server, &b, "DELETE", "/v1/app/account", None).await.status(), 204);
    assert!(!root.exists());
    assert!(support::try_connect_device(&server.host(), &dev.token).await.is_err());
    assert_eq!(cookie_request(&server, &b, "GET", "/v1/app/devices", None).await.status(), 401);
}

#[tokio::test]
async fn a_panel_linking_during_account_deletion_writes_nothing_into_the_deleted_folder() {
    // Review Focus 5: start deleting account B while its device connects in a loop;
    // after deletion, assert accounts/<b> does not exist and the registry has no b device.
    // Drive: spawn a task that reconnects dev every 10 ms for 500 ms; DELETE /v1/app/account midway.
}
```

- [x] **Step 2: Run to verify they fail.**
- [x] **Step 3: Implement.** Ordering in account deletion matters for Review Focus 5: **revoke every device in the registry first** (so a new link cannot authenticate), then close live links, then stop data-card tasks, drop the space, delete the rows, and only then remove the folder. The link handler already refuses a device with no owner row (Task 3), which closes the window between `revoke` and `delete_account`.
- [x] **Step 4: Gates**; regenerate the TS contract fixture; **mutation probe**: skip `registry().revoke` in `DELETE /v1/app/devices/{id}` -> `removing_a_panel...` must fail.
- [x] **Step 5: Commit** `feat(server): claim, remove and delete: panels belong to accounts`.

---

### Task 8: Migration, startup wiring and deploy config

**Files:**
- Create: `src/migrate.rs`, `tests/migration.rs`, `tests/fixtures/flat-layout/` (a copy of a realistic flat config dir: `device-identities.json` with two devices, `dev-0001.json` and `dev-0005.json` v10 documents taken from existing test fixtures, `image-sources.json` with one source, `image-frames/<id>.bin`, `data-cards.json`, `producer-credentials.json`)
- Modify: `src/main.rs`, `deploy/deskmate-server.env.example`, `deploy/README.md`, `CLAUDE.md` (current-state bullets on auth -- see Step 5)

**Interfaces:**

```rust
pub enum MigrationOutcome { NotNeeded, Migrated { account: AccountId, devices: usize, legacy_dir: PathBuf } }
pub enum MigrationError { OwnerEmailRequired, InvalidOwnerEmail, Io(String), Identity(IdentityError) }
pub fn migrate_if_needed(config_dir: &Path, owner_email: Option<&str>, now: DateTime<Utc>) -> Result<MigrationOutcome, MigrationError>;
```

- [x] **Step 1: Write the failing tests** in `tests/migration.rs` (copy the fixture dir into a tempdir first):

```rust
#[test]
fn migrates_the_flat_layout_into_the_owner_account() {
    let dir = copy_fixture("flat-layout");
    let outcome = migrate_if_needed(dir.path(), Some("Owner@Example.com"), Utc::now()).unwrap();
    let MigrationOutcome::Migrated { account, devices, legacy_dir } = outcome else { panic!() };
    assert_eq!(devices, 2);
    let space = dir.path().join("accounts").join(&account.0);
    for (from, to) in [("image-sources.json", "image-sources.json"), ("data-cards.json", "data-cards.json"),
                       ("dev-0005.json", "devices/dev-0005.json")] {
        assert_eq!(std::fs::read(legacy_dir.join(from)).unwrap(), std::fs::read(space.join(to)).unwrap(), "{from}");
    }
    assert!(dir.path().join("device-identities.json").exists(), "registry stays at the root");
    assert!(dir.path().join("producer-credentials.json").exists(), "integrations stay at the root");
    let state = ServerState::new(IN_MEMORY_ADMIN_TOKEN.into(), FirmwareCatalog::in_memory(), dir.path().to_path_buf());
    assert!(state.space_for_device("dev-0005").is_some());
}

#[test]
fn refuses_without_an_owner_email_and_touches_nothing() {
    let dir = copy_fixture("flat-layout");
    let before = snapshot_tree(dir.path());
    assert!(matches!(migrate_if_needed(dir.path(), None, Utc::now()), Err(MigrationError::OwnerEmailRequired)));
    assert_eq!(snapshot_tree(dir.path()), before);
}

#[test]
fn runs_once() {
    let dir = copy_fixture("flat-layout");
    migrate_if_needed(dir.path(), Some("o@example.com"), Utc::now()).unwrap();
    assert!(matches!(migrate_if_needed(dir.path(), Some("o@example.com"), Utc::now()).unwrap(), MigrationOutcome::NotNeeded));
}

#[test]
fn a_crash_before_the_final_rename_migrates_again_from_untouched_originals() {
    let dir = copy_fixture("flat-layout");
    let before = snapshot_tree(dir.path());
    migrate::fail_before_rename_for_tests(dir.path(), "o@example.com"); // runs steps 1-3, returns before step 4
    assert!(!dir.path().join("identity.db").exists());
    for (path, bytes) in &before { assert_eq!(&std::fs::read(dir.path().join(path)).unwrap(), bytes, "{path}"); }
    assert!(matches!(migrate_if_needed(dir.path(), Some("o@example.com"), Utc::now()).unwrap(), MigrationOutcome::Migrated { .. }));
}

#[test]
fn a_fresh_empty_directory_needs_no_migration() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(migrate_if_needed(dir.path(), None, Utc::now()).unwrap(), MigrationOutcome::NotNeeded));
}
```

- [x] **Step 2: Run to verify they fail.**
- [x] **Step 3: Implement.**
  - Trigger: `identity.db` absent **and** any of `device-identities.json`, `image-sources.json`, `data-cards.json`, `producer-credentials.json`, `secrets.enc`, `dev-*.json` present.
  - Steps as in spec section 6: build `identity.db.migrating` (delete a stale one first), create the owner (`is_instance_owner`, unverified), one `Active` `device_owners` row per registry device (parse `device-identities.json`'s `devices[].device_id`), copy (not move) the per-account files into `accounts/<id>/` (starting from an empty `accounts/<id>` -- remove any partial one from a crashed run), `std::fs::rename` `identity.db.migrating` -> `identity.db`, `sync_parent`, then move the copied originals into `legacy-<%Y%m%dT%H%M%SZ>/`. Log the outcome at `warn!` with the account id and legacy dir.
  - `main.rs`: read `DESKMATE_PUBLIC_URL` (required, must parse, scheme `https` or `http` only for a loopback host; error text names the variable), `DESKMATE_SMTP_URL` + `DESKMATE_MAIL_FROM` (both or neither), `DESKMATE_SIGNUPS` (`open`/`closed`, default closed), `DESKMATE_OWNER_EMAIL`; call `migrate_if_needed` before `ServerState::new_with_options`; exit 1 with the error text on `OwnerEmailRequired`; `spawn_housekeeping`; serve with connect info. Delete `data_card_spec_path` and `DESKMATE_DATA_CARDS`.
  - `deploy/deskmate-server.env.example`: add the four variables with comments; remove `DESKMATE_DATA_CARDS`.
  - `deploy/README.md`: a "Signing in" section: first run and the setup code (`journalctl -u deskmate-server | grep -A3 'setup code'`), sign-in links in the log without SMTP, the recovery line:

```sh
curl -sS -X POST https://<host>/v1/admin/signin-link \
  -H "Authorization: Bearer $DESKMATE_ADMIN_TOKEN" -H 'content-type: application/json' \
  -d '{"email":"you@example.com"}'
```

- [x] **Step 4: Gates.**
- [x] **Step 5: Update `CLAUDE.md`**: the "THE COMPANION IS A WEB APP" bullet says the API is "gated by the operator session cookie the page trades the admin token for" and the "No edge auth" bullet says `/v1/app/*` needs "the operator session cookie traded for `DESKMATE_ADMIN_TOKEN`" -- rewrite both to: account sessions (email link, Google), first-run setup code, admin token = CLI/recovery only, and the login rate limit now exists. Keep the path-scoped edge-gate warning as is. Add the `DESKMATE_DATA_CARDS` removal to the faces bullet (it mentions the variable).
- [x] **Step 6: Commit** `feat(server): one-time migration into the owner's account, and startup wiring`.

---

### Task 9: TypeScript serial codec

**Files:**
- Create: `companion/apps/deskmate/src/lib/serial/codec.ts`
- Test: `companion/apps/deskmate/tests/serialCodec.test.ts`

**Interfaces:**

```ts
export const PROTOCOL_VERSION = 2;
export const MessageType = { StatusRequest: 1, StatusResponse: 2, Ack: 4, Error: 8, NetworkConfig: 13, FactoryReset: 14 } as const;
export const CAPABILITY_NETWORKING = 1n << 7n;
export type NetworkConfig = { ssid: string; psk: string; serverUrl: string; deviceId: string; token: string; utcOffsetMinutes: number; tier: 0 | 1 };
export type StatusResponse = { protocolVersion: number; firmwareVersion: string; capabilities: bigint; tier: number;
  wifiState: 0 | 1 | 2 | 3; ip: string; lastNetworkError?: string; /* other keys decoded but unused */ };
export type Ack = { acknowledgedType: number };
export type DeviceError = { code: number; diagnostic: string };
export type Decoded =
  | { type: "status"; requestId: number; status: StatusResponse }
  | { type: "ack"; requestId: number; ack: Ack }
  | { type: "error"; requestId: number; error: DeviceError }
  | { type: "other"; requestId: number; messageType: number };
export function crc32c(bytes: Uint8Array): number;
export function encodeStatusRequest(requestId: number): Uint8Array;               // wire bytes incl. trailing 0x00
export function encodeNetworkConfig(requestId: number, config: NetworkConfig): Uint8Array;
export class Deframer { push(chunk: Uint8Array): Array<Decoded | { type: "invalid"; reason: string }>; }
export function validateNetworkConfig(config: NetworkConfig): string | null;      // null = ok; else a sentence
export function rawDecoded(wire: Uint8Array): Uint8Array;                         // COBS-decode one wire frame (test helper)
```

- [x] **Step 1: Write the failing tests**, driven by the Rust-generated fixtures shared with the firmware:

```ts
import { expect, test } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { crc32c, Deframer, encodeNetworkConfig, encodeStatusRequest, rawDecoded, validateNetworkConfig } from "../src/lib/serial/codec";

const FIXTURES = join(import.meta.dir, "../../../../protocol/fixtures/v2");
const fixture = (name: string) => new Uint8Array(readFileSync(join(FIXTURES, name)));

test("crc32c check value", () => {
  expect(crc32c(new TextEncoder().encode("123456789"))).toBe(0xe3069283);
});

test("status request is byte-identical to the Rust fixture", () => {
  const expected = fixture("status_request.bin");
  const requestId = new DataView(rawDecoded(expected).buffer).getUint32(4, true);
  expect(encodeStatusRequest(requestId)).toEqual(expected);
});

test("network config is byte-identical to the Rust fixture", () => {
  // Field values are the ones crates/protocol/examples/generate-fixtures.rs uses for network_config.bin
  // (request id 7, max-length strings, utc offset 240, networked). Copy them from that file verbatim.
  expect(encodeNetworkConfig(7, NETWORK_CONFIG_FIXTURE_VALUES)).toEqual(fixture("network_config.bin"));
});

test("decodes every status response fixture", () => {
  for (const name of ["status_response.bin", "status_response_networked.bin", "status_response_ota_failed.bin"]) {
    const [decoded] = new Deframer().push(fixture(name));
    expect(decoded.type).toBe("status");
  }
  const [networked] = new Deframer().push(fixture("status_response_networked.bin"));
  if (networked.type !== "status") throw new Error("expected status");
  expect(networked.status.tier).toBe(1);
});

test("decodes ack and error fixtures", () => {
  const acks = readdirSync(FIXTURES).filter((n) => n.startsWith("ack_"));
  expect(acks.length).toBeGreaterThan(0);
  for (const name of acks) expect(new Deframer().push(fixture(name))[0].type).toBe("ack");
  expect(new Deframer().push(fixture("error.bin"))[0].type).toBe("error");
});

test("hostile input never throws and does not poison the next frame", () => {
  const good = fixture("status_response.bin");
  for (const name of ["bad_crc.bin", "garbage.bin", "overlong.bin", "invalid_cbor.bin", "duplicate_keys.bin"]) {
    const out = new Deframer().push(new Uint8Array([...fixture(name), ...good]));
    expect(out.at(-1)?.type).toBe("status");
    expect(out.slice(0, -1).every((d) => d.type === "invalid")).toBe(true);
  }
});

test("frames split across chunks reassemble", () => {
  const good = fixture("status_response.bin");
  const d = new Deframer();
  expect(d.push(good.slice(0, 5))).toEqual([]);
  expect(d.push(good.slice(5))[0].type).toBe("status");
});

test("network config limits match the wire", () => {
  const base = { ssid: "home", psk: "secret", serverUrl: "wss://x/v1/device/link", deviceId: "dev-0001", token: "t", utcOffsetMinutes: 0, tier: 1 as const };
  expect(validateNetworkConfig(base)).toBeNull();
  expect(validateNetworkConfig({ ...base, ssid: "é".repeat(17) })).toMatch(/network name/i); // 34 UTF-8 bytes > 32
  expect(validateNetworkConfig({ ...base, psk: "x".repeat(65) })).toMatch(/password/i);
});
```

(Adjust fixture names to what `protocol/fixtures/v2/manifest.txt` lists; `rawDecoded` is an exported test helper that COBS-decodes one wire frame. `NETWORK_CONFIG_FIXTURE_VALUES` is a constant in the test file copied from `generate-fixtures.rs`.)
- [x] **Step 2: Run to verify they fail** (`cd companion/apps/deskmate && bun test tests/serialCodec.test.ts`).
- [x] **Step 3: Implement.** Canonical CBOR subset: unsigned/negative ints (major 0/1, shortest form), text strings (major 3), maps (major 5) with **integer keys in ascending order**, booleans, and `u64` capabilities read as `bigint`. The decoder rejects duplicate keys, indefinite lengths and trailing bytes; unknown keys are skipped except where `docs/protocol/v2.md` says closed. Limits: `MAX_DECODED_FRAME` 2048, `MAX_WIRE_FRAME` 2058; UTF-8 byte lengths via `TextEncoder`: ssid ≤ 32, psk ≤ 64, server_url ≤ 128, device_id ≤ 32, token ≤ 128, `utcOffsetMinutes` in `-840..=840`. Flags must be 0; version must be 2 (a v1 frame decodes as `invalid: version`).
- [x] **Step 4: Run** `bun test && bun run check && bun run format:check` from `apps/deskmate`.
- [x] **Step 5: Commit** `feat(web): wire codec for panel setup, tested on the firmware's own fixtures`.

---

### Task 10: Panel port and the setup state machine

**Files:**
- Create: `src/lib/serial/port.ts`, `src/lib/serial/setup.ts`
- Test: `tests/panelSetup.test.ts`

**Interfaces:**

```ts
// port.ts
export interface PanelPort {
  open(): Promise<void>;
  write(bytes: Uint8Array): Promise<void>;
  readable(): AsyncIterable<Uint8Array>;
  close(): Promise<void>;
  onDisconnect(listener: () => void): void;
}
export const PANEL_USB_FILTER = { usbVendorId: 0x303a, usbProductId: 0x1001 } as const;
export function serialSupported(): boolean;                         // "serial" in navigator
export async function requestPanelPort(): Promise<PanelPort>;       // navigator.serial.requestPort({filters:[PANEL_USB_FILTER]})
export class PanelLink {                                             // one outstanding request, 2000 ms timeout, ignores DeviceEvent
  constructor(port: PanelPort);
  status(): Promise<StatusResponse>;
  networkConfig(config: NetworkConfig): Promise<void>;               // resolves on Ack(13), rejects with DeviceError text
}

// setup.ts
export type SetupStep =
  | { kind: "connecting" }
  | { kind: "incompatible"; message: string }
  | { kind: "no-response" }
  | { kind: "wifi-form"; error?: string }
  | { kind: "writing" }
  | { kind: "wifi-joining" }
  | { kind: "wifi-failed"; boardError: string }
  | { kind: "linking" }
  | { kind: "linked"; deviceId: string }
  | { kind: "unreachable-server"; deviceId: string };
export type SetupDeps = {
  port: PanelPort;
  claim: () => Promise<{ device_id: string; token: string; link_url: string }>;
  isLinked: (deviceId: string) => Promise<boolean>;
  utcOffsetMinutes: () => number;
  sleep: (ms: number) => Promise<void>;
  now: () => number;
};
export class PanelSetup {
  constructor(deps: SetupDeps, onStep: (step: SetupStep) => void);
  connect(): Promise<void>;                              // poll status every 500 ms up to 10 s; checks v2 + Networking
  submitWifi(ssid: string, password: string): Promise<void>; // claims once, writes, watches Wi-Fi, then the link
}
```

- [x] **Step 1: Write the failing tests** with a `FakePort` that answers from a script (it uses the codec to build responses, so the fake speaks the real wire):

```ts
test("a board that never answers ends in no-response after 10 s", async () => { ... expect(last).toEqual({ kind: "no-response" }); });
test("a v1 board or one without Networking is refused before any write", async () => { ... expect(port.written).toHaveLength(pollsOnly) ... });
test("the Wi-Fi password is written only to the port, never to claim()", async () => {
  // claim is a mock; assert its calls have no arguments and nothing it returned contains the psk;
  // assert the NetworkConfig frame on the port decodes to the typed psk.
});
test("a failed join shows the board's error, and a retry reuses the same identity", async () => {
  // status sequence: wifiState 1, then 3 with lastNetworkError "auth failed"
  // expect step wifi-failed "auth failed"; submitWifi again -> claim called exactly once in total,
  // the second NetworkConfig carries the same device_id/token.
});
test("a board that restarts after NetworkConfig is reopened and the flow continues", async () => {
  // FakePort fires onDisconnect after the Ack; the next open() succeeds; wifiState 2; isLinked true -> linked
});
test("Wi-Fi up but no server link within 60 s ends in unreachable-server and keeps the claim", async () => { ... });
test("happy path ends in linked", async () => { ... });
```

`FakePort` spec: constructed with a list of `StatusResponse` overrides answered in order to successive `StatusRequest`s (the last one repeats), an `ackNetworkConfig: boolean | DeviceError`, an optional `silent: true` (answers nothing), and `disconnectAfterAck: boolean` (fires the disconnect listener once, then answers again after the next `open()`). It records every decoded frame written (`written`). The fake clock advances `now()` by each `sleep(ms)`. Write each test body fully against that fake; each asserts the final `SetupStep` and the relevant `written` frames.
- [x] **Step 2: Run to verify they fail.**
- [x] **Step 3: Implement.** Request ids start at 1 and wrap skipping 0. `PanelLink` drops `DeviceEvent`s (type 11, request id 0) and responses with a stale id. `connect()` tolerates the reset measured in Task 1 by polling; on `onDisconnect` during `submitWifi` it re-`open()`s up to 3 times with 1 s between tries. The claim result is memoised in the `PanelSetup` instance so a retry never mints again.
- [x] **Step 4: Run** the web gates.
- [x] **Step 5: Commit** `feat(web): panel setup over Web Serial`.

---

### Task 11: Sign-in, setup and link screens

**Files:**
- Create: `src/lib/account.ts`, `src/components/AuthScreens.tsx`, `tests/authScreens.test.tsx`
- Modify: `src/App.tsx` (signed-out branch; `/signin` path; `?signin_error=`), `src/lib/apiErrors.ts` (`SESSION_REQUIRED_MESSAGE` -> "This browser is not signed in."), `src/lib/backendClient.ts` (delete `signIn`, `signInAndSelectDevice`; `deviceId()` throws `NO_PANELS` error when the list is empty; export `isNoPanels`), `src/lib/useAppState.ts` (drop `signIn`/`signInAndSelectDevice`), `src/dev/backendClient.ts` + `src/dev/mockBackend.ts` (scenarios `signedout`, `setup`, `nopanels`; mocks for the `account.ts` calls; delete mock `signIn`), existing tests that referenced the admin-token form

**Interfaces:**

```ts
// account.ts -- all go through the same fetch wrapper as backendClient (credentials: "same-origin")
export type Instance = { setup_required: boolean; google_enabled: boolean; signups_open: boolean; edition: "self-hosted" | "hosted" };
export type Account = { id: string; email: string; email_verified: boolean; is_instance_owner: boolean };
export type PanelRow = { id: string; connected: boolean; has_saved_config: boolean; configured_at: number | null; state: "pending" | "active" };
export function getInstance(): Promise<Instance>;
export function completeSetup(code: string, email: string): Promise<Account>;
export function requestSignInLink(email: string): Promise<void>;          // 429 -> DeskmateApiError with the server sentence
export function consumeSignInLink(token: string): Promise<Account>;
export function googleSignInUrl(): string;                                // "/v1/app/auth/google/start"
export function getAccount(): Promise<Account>;
export function signOut(): Promise<void>;
export function signOutEverywhere(): Promise<void>;
export function deleteAccount(): Promise<void>;
export function setSignupsOpen(open: boolean): Promise<boolean>;
export function listPanels(): Promise<PanelRow[]>;
export function claimPanel(): Promise<{ device_id: string; token: string; link_url: string }>;
export function removePanel(id: string): Promise<void>;
```

Screens (`AuthScreens.tsx`), each a `<main className="startup">` like today's:
- `SetupScreen` -- heading "Set up this server", fields "Setup code" (hint: "It's in the server's log.") and "Email", button "Set up".
- `SignInScreen` -- heading "Sign in to Deskmate", "Email" + "Email me a sign-in link", and, when `google_enabled`, a "Sign in with Google" link to `googleSignInUrl()`. Shows `signin_error` sentences: `signups-closed` -> "This server isn't taking new accounts.", `expired` -> "That sign-in took too long. Try again.", anything else -> "Google sign-in didn't work. Try again."
- `CheckInbox` -- "Check your inbox", "We sent a link to {email}. It works for 15 minutes." and, when `edition === "self-hosted"`, "No email set up on this server? The link is in the server's log."
- `LinkLanding` -- rendered when `location.pathname === "/signin"`: heading "Sign in to Deskmate", one button "Sign in"; on click `consumeSignInLink(token)`, then `history.replaceState(null, "", "/")` and reload state; on failure the server's sentence and a "Back to sign-in" button.

App order: `getInstance()` first; `setup_required` -> `SetupScreen`; `/signin` -> `LinkLanding`; session missing -> `SignInScreen`; `isNoPanels` -> the Task 12 `PanelSetup` screen; else the window.

- [x] **Step 1: Write the failing tests** (`tests/authScreens.test.tsx`, with `backendMocks`-style mocks for `account.ts`): the setup screen submits code+email and then loads the window; the sign-in screen shows the inbox screen after submit and never says whether the email exists; the Google button appears only when `google_enabled`; `?signin_error=signups-closed` shows its sentence; `/signin?token=abc` does **not** call `consumeSignInLink` until the button is pressed (Review Focus: mail scanners); a failed link shows "This sign-in link has expired."; the old "Admin token" field is gone from the app (`expect(container.textContent).not.toContain("Admin token")`).
- [x] **Step 2: Run to verify they fail.**
- [x] **Step 3: Implement.** Remove the admin-token form from `NetworkPanel.tsx` (its `onSignIn` prop and the Server base URL / Device ID / Admin token fields); keep its status `<dl>`. Update its doc comment: panels are set up from "Add a panel", `deskmate-cli` remains for cable maintenance.
- [x] **Step 4: Run** the web gates, then `VITE_DESKMATE_MOCK=1 bun run dev` and look at `?scenario=signedout`, `?scenario=setup`, `?scenario=nopanels` in Chrome.
- [x] **Step 5: Commit** `feat(web): sign in with an email link or Google, and first-run setup`.

---

### Task 12: Panel setup screen and the account section

**Files:**
- Create: `src/components/PanelSetup.tsx`, `src/components/AccountSection.tsx`, `tests/accountSection.test.tsx`
- Modify: `src/App.tsx` (no-panels startup screen renders `PanelSetup`; settings sheet gains an "Account" section and a "Panels" section above "Device"), `src/dev/mockBackend.ts` (a fake `PanelPort` for the `nopanels` scenario so the harness can walk the flow without hardware -- it only answers `StatusRequest` and `NetworkConfig`, both of which the shipped app really sends)

**UI copy (exact):**
- Unsupported browser: "Setting up a panel needs Chrome or Edge on a computer."
- Step 1: heading "Add your panel", "Plug the panel into this computer with its USB cable.", button "Connect".
- Incompatible: "This panel needs a firmware update first."
- No response: "The panel didn't respond. Unplug it, plug it back in, and try again."
- Wi-Fi form: "Wi-Fi network", "Wi-Fi password", note "Your Wi-Fi password goes to the panel over the cable. It is never sent to Deskmate's server.", button "Connect to Wi-Fi".
- Joining: "Joining {ssid}…"; failed: "The panel couldn't join {ssid}: {boardError}" + the form again.
- Linking: "Connected to Wi-Fi. Waiting for the panel to reach the server…"
- Unreachable: "The panel is on Wi-Fi but hasn't reached the server yet. Leave it plugged in; it keeps trying. You can close this page."
- Linked: "Your panel is connected. You can unplug it from the computer." + "Done".
- Account section: the email; "Sign out"; "Sign out everywhere"; "Delete account" behind a confirm step in the same sheet ("This deletes your cards and releases your panels. It can't be undone." + "Delete my account" / "Keep it") -- **no browser `confirm()`**; for the instance owner, a switch "Let new people sign up".
- Panels section: one row per panel -- id, "Online"/"Offline"/"Waiting for first connection" (pending), and "Remove" behind the same in-sheet confirm; then "Add a panel" (opens `PanelSetup` inside the sheet).

- [x] **Step 1: Write the failing tests**: the unsupported-browser sentence replaces "Add a panel" when `serialSupported()` is false; `PanelSetup` renders each `SetupStep` kind with the copy above (drive a mocked `PanelSetup` class, not hardware); "Delete account" needs two presses and calls `deleteAccount` once; a non-owner sees no sign-ups switch; removing a panel calls `removePanel` and drops the row.
- [x] **Step 2: Run to verify they fail.**
- [x] **Step 3: Implement.** `PanelSetup.tsx` owns one `PanelSetup` (Task 10) per attempt, created on "Connect" with `requestPanelPort()` and `SetupDeps` wired to `claimPanel`, `listPanels` (linked = row with `connected && state === "active"`), `() => -new Date().getTimezoneOffset()`.
- [x] **Step 4: Run** the web gates; `bun run build`.
- [x] **Step 5: Commit** `feat(web): add a panel from the page, and account settings`.

---

### Task 13: Real checks, gates, merge and deploy

- [x] **Step 1: Full local gates** (all of `CLAUDE.md` "Verification", companion part, plus the faces suite). Record counts.
- [x] **Step 2: Drive the real build locally in Chrome.** `bun run build`; run the server with a fresh `DESKMATE_CONFIG_DIR` in the scratch dir, `DESKMATE_PUBLIC_URL=http://localhost:8443`, `DESKMATE_WEB_DIR=<dist>`, no SMTP. Walk: setup code from the log -> owner -> open sign-ups -> a second browser profile signs up with a link from the log -> confirm it sees no panel and none of the owner's picture sources -> sign out everywhere -> the owner's other tab is signed out. (Add `http` + loopback as an accepted `DESKMATE_PUBLIC_URL` if Task 8 did not.) Note that `__Host-` cookies need `Secure`, which Chrome allows on `http://localhost` -- if it does not in the installed version, run Caddy locally with `tls internal` instead.
- [x] **Step 3: Migration dry run on a copy of the live config.** `rsync` `/var/lib/private/deskmate/configs` from docker-vm into the scratch dir, run the new binary locally against the copy with `DESKMATE_OWNER_EMAIL` set, confirm the migration log line, then sign in via the logged link and see the live loop in the window.
- [ ] **Step 4: Push the branch, open a PR, confirm CI green with `gh run list`** (not grey/skipped).
- [ ] **Step 5: Deploy.** Add `DESKMATE_PUBLIC_URL=https://deskmate.rodi.one`, `DESKMATE_OWNER_EMAIL=<owner's>` and (optionally) SMTP to `/etc/deskmate/server.env` on docker-vm; `deploy.sh` (binary + web together). Check: the log shows the migration; `dev-0005` re-links (server side); the owner signs in with the logged link. Say exactly which of these were observed.
- [ ] **Step 6: Claim `dev-0005` from the deployed page -- only when the owner says so.** It re-provisions the live panel with a new identity. Record the observed steps (Wi-Fi join time, link time) in `docs/hardware/board-notes.md`.
- [ ] **Step 7: Merge** after owner approval; update the roadmap board line (Track A: sub-project 1 merged; next: plan limits and billing, and self-host packaging).
