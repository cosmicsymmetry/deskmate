# V3 Sub-Project 3 — Token Vending and the Egress Boundary Implementation Plan

> **STATUS: EXECUTED 2026-09-13, Tasks 1-5 complete.** Boxes below are ticked as
> they were verified, not in advance. Commits: `986dbc6` (merge), `5012663`
> (egress), `2bbb346` (credentials), `e98859a` (vend), plus the coupling commit.
> Five deviations from the plan as written, each forced by what the code turned
> out to be:
>
> | Planned | Actual |
> |---|---|
> | `fetch_post_form(url, Vec<(String, String)>)` | `&[(&str, &str)]`, and the host check belongs on `fetch_post_form` rather than `post_form_inner`, which is shared with the resolver-injected test driving `http://token.invalid` |
> | Delete the GET path | Also deleted `MAX_REDIRECTS`, `TooManyRedirects` and `BadRedirect`, which the POST client's disabled redirects made unreachable; **retargeted** the two body-cap tests and the status-capture test onto the POST path rather than deleting them, because `read_capped_body` is still live there |
> | Disk test asserts the plaintext is absent | Too weak, and a mutation probe proved it: a `token_digest` copying the token's bytes verbatim passed it, since the bytes are hex-encoded on the way out and the substring never appears. Rewritten to assert the stored value **is** SHA-256(token), which fails that mutation |
> | Test harness with `.bearer()` helpers | `tests/oauth_routes.rs` drives `tower::ServiceExt::oneshot` directly; helpers written to match |
> | `NeedsReconnect` → 409 via `map_vend_error` | A separate `vend_error_to_route`, leaving sub-project 2's `token_error_to_route` (and its 400) untouched on the consent/revoke routes |
>
> One gap recorded rather than closed: `read_capped_body`'s `content_length`
> precheck is not independently covered — deleting it fails nothing, because the
> streaming limiter catches the same case. That was equally true before this
> work; it is an optimization, not a second guard.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let an external producer obtain a live Google access token from the server, so the producer — not the server — calls the third-party API, renders a 448x368 face, and pushes it through the image-source path that already ships.

**Architecture:** The server stays a credential *custodian*. A producer authenticates with its own digest-only credential (the contract `image_sources.rs` and `registry.rs` already use), and receives Google's own access token with Google's own expiry. `egress.rs` survives the merge from `main` cut down to one job — the OAuth token call to exactly one permitted host — and a test makes that boundary executable.

**Tech Stack:** Rust, Axum 0.8-style extractors, `tokio::task::spawn_blocking`, `sha2`, `rand`, `protocol::digest_hex`, `app_core::secure_file`.

**Spec:** `docs/superpowers/specs/2026-09-12-deskmate-v3-server-host-revision.md` (§5 token vending, §8 security, §10 decomposition, §13 exit gate, §14 merge order). Read §1 before Task 2 — it is the invariant the whole task exists to protect.

## Global Constraints

Copied verbatim from the spec and CLAUDE.md. Every task's requirements implicitly include these.

- **Schema stays v10 and protocol stays v2.** This sub-project adds no config field, no wire message, and no firmware change. If you find yourself editing `app-core/src/config.rs` or anything under `firmware/`, stop — you have left the plan.
- **`IntegrationStore` holds a sync `Mutex` across disk I/O.** Any async Axum handler calling `put` / `remove` must go through `tokio::task::spawn_blocking`. The same applies to the new producer-credential store, which follows the same shape.
- **Credentials are persisted as SHA-256 digests only; the plaintext is returned exactly once.** No route ever reads a credential back.
- **`https` is required on every credential-path URL.** The old `http` allowance existed for plugin feeds, which no longer exist.
- **Never log, `Debug`-print, or include in an error body:** a refresh token, a client secret, a producer credential, or an access token.
- **`cargo` is not on the Bash tool's PATH.** Run `export PATH="$HOME/.cargo/bin:$PATH"` first.
- **Never pipe a cargo invocation into `tail`** — zsh reports `tail`'s exit status, hiding a failure as a pass. Redirect to a file and check `$?`.
- **Server integration tests bind loopback, which a sandboxed subagent cannot do.** A sandboxed implementer will report green on tests it never ran. The controller runs every `cargo test -p server` invocation in this plan.

## File Structure

| File | Responsibility |
|---|---|
| `companion/crates/server/src/egress.rs` | **Modify.** Reduced to the credential path: guard, resolve-then-pin, POST-form, capped body. The plugin-feed GET path goes. Gains the one-host allowlist. |
| `companion/crates/server/src/producer_credentials.rs` | **Create.** Digest-only store mapping a producer credential to one integration id. Mint / authenticate / revoke. Mirrors `image_sources.rs`. |
| `companion/crates/server/src/oauth/token.rs` | **Modify.** Expose the cached token's expiry so the vend response can carry it. |
| `companion/crates/server/src/oauth/routes.rs` | **Modify.** Three routes: mint, revoke, vend. Revocation coupling. |
| `companion/crates/server/src/lib.rs` | **Modify.** One `StateInner` field and its accessor. |
| `companion/crates/server/src/main.rs` | **Modify.** Open the store at startup. |
| `companion/crates/server/tests/producer_credentials.rs` | **Create.** Store behaviour. |
| `companion/crates/server/tests/oauth_routes.rs` | **Modify.** Route behaviour; extends the existing sub-project 2 suite. |
| `docs/images/producer-guide.md` | **Modify.** How a producer obtains a token. |
| `companion/crates/server/deploy/deskmate-server.env.example` | **Modify.** No new secret; document the store path. |

---

### Task 1: Merge `main` into the branch

The branch is 18 commits ahead of `main` and 78 behind. Nothing else in this plan compiles until this lands. The collision was measured against merge base `faac9ab` — see spec §14 for the table.

**Files:**
- Resolve: `companion/crates/server/src/egress.rs` (modify/delete)
- Resolve: `companion/crates/server/src/lib.rs`, `src/main.rs`, `Cargo.toml`, `companion/Cargo.lock`
- Resolve: `companion/crates/app-core/src/lib.rs`, `src/secure_file.rs` (expected to be self-resolving)

**Interfaces:**
- Consumes: nothing.
- Produces: a branch on which `oauth/`, `secrets.rs` and `egress.rs` coexist with `image_sources.rs`, `image_ingest.rs` and `images.rs`.

- [x] **Step 1: Start the merge and capture the conflict list**

```bash
cd /Users/rodion/dev/deskmate/.worktrees/v3-server-host
git merge main 2>&1 | tee /tmp/v3-merge.log
git status --short | grep -E '^(UU|AA|DU|UD|AU|UA)'
```

Expected: conflicts on the six files named above and nothing else. **If a file outside that list conflicts, stop and report** — the spec's §14 measurement was wrong and the plan needs amending before you continue.

- [x] **Step 2: Resolve `egress.rs` — keep it**

`main` deleted this file with the plugin feeds it was written for. The branch modified it: sub-project 2's `ca319af` added `fetch_post_form` for the token call. Keep the branch's version wholesale; Task 2 cuts it down.

```bash
git checkout --ours companion/crates/server/src/egress.rs
git add companion/crates/server/src/egress.rs
```

- [x] **Step 3: Resolve `app-core` — verify the two sides are identical, then take either**

Both sides independently promoted `secure_file`'s `BoundedReadError` and `FileIoError` from `pub(crate)` to `pub` — the branch for `IntegrationStore`, `main` for the image-source store.

```bash
git diff --diff-filter=U --stat -- companion/crates/app-core/
```

Resolve by keeping the `pub` form in both files. `companion/crates/server/tests/secure_file_reexport.rs` is byte-identical on both sides; keep one copy.

- [x] **Step 4: Resolve `lib.rs` — both routers, minus the plugins**

`main` removed the plugin router and its `StateInner` fields (`plugins`, `plugin_load_failures`); the branch added `integrations: OnceLock<Arc<oauth::IntegrationRuntime>>`. The merged result keeps `main`'s field set plus the branch's `integrations` field, and the merged router is:

```rust
    Router::new()
        .route("/v1/device/link", get(device_link::handler))
        .route("/v1/device/firmware", get(firmware::check))
        .route("/v1/firmware/{filename}", get(firmware::download))
        .merge(admin::routes())
        .merge(images::routes())
        .merge(oauth::routes::routes())
        .layer(middleware)
        .with_state(state)
```

Delete every `plugin*` module declaration and `use` from the branch side. Those modules are gone from the working tree after this merge; a leftover `mod plugin_host;` is a compile error that tells you which one you missed.

> **EXECUTED 2026-09-13.** The merge produced exactly the six conflicts §14 predicted
> and nothing else. Resolutions actually taken, including three the measurement did not
> foresee:
> - `secure_file.rs`: `main`'s side wholesale. It did more than promote visibility — it
>   split the writer into `write_and_replace` (text, appends a newline) and
>   `write_and_replace_binary` (exact bytes, for the 329,740-byte canonical frame). The
>   branch's `secrets.rs` already trims with `trim_ascii_end()`, so `secrets.enc` is
>   unaffected. The only branch-side line not in `main`'s version is the unconditional
>   newline write, which `write_atomic`'s flag replaced.
> - `lib.rs`: **`main` deleted the `pub mod egress;` declaration and git auto-merged that
>   deletion**, because the branch never touched that line — only the file. Keeping
>   `egress.rs` in Step 2 is not enough; the declaration must be restored by hand or the
>   module is silently dropped from the crate.
> - `main.rs`: `main` also removed `use std::sync::Arc;`, which existed only for
>   `Arc::new(plugins)`. The OAuth wiring below needs it. Restored.
> - `Cargo.toml`: kept `hmac` (session cookie), `png`/`tiny-skia` (image ingest),
>   `reqwest` + `url` (egress); dropped `resvg`/`roxmltree` with the plugins.

- [x] **Step 5: Resolve `main.rs` and `Cargo.toml`**

Keep `main`'s startup wiring (image sources, ingest) and add the branch's integration wiring (`open_integration_store`, `set_integrations`). In `Cargo.toml`, keep `main`'s dependency set and re-add what the branch needs and `main` dropped: `reqwest` with its `form` feature (`egress.rs` uses it), `chacha20poly1305`, `zeroize`, `base64`. Drop anything only the plugin path used (`resvg` and friends) — if it turns out something still needs one, the build says so by name.

- [x] **Step 6: Regenerate the lock file and complete the merge**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/v3-server-host/companion
git checkout --theirs Cargo.lock && cargo check --workspace > /tmp/v3-merge-check.log 2>&1; echo "exit=$?"
```

Expected: `exit=0`. Read `/tmp/v3-merge-check.log` on any failure; do not pipe this to `tail`.

- [x] **Step 7: Run the full workspace gate**

This is the step that decides whether "mechanical" was true. Budget ~35 minutes cold.

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/v3-server-host/companion
cargo fmt --all --check > /tmp/v3-fmt.log 2>&1; echo "fmt=$?"
cargo clippy --workspace --all-targets -- -D warnings > /tmp/v3-clippy.log 2>&1; echo "clippy=$?"
cargo test --workspace --all-targets > /tmp/v3-test.log 2>&1; echo "test=$?"
cargo test --workspace --doc > /tmp/v3-doc.log 2>&1; echo "doc=$?"
```

Expected: all four report `0`. Both test invocations are required — `--all-targets` adds integration targets but removes doctests, so neither alone covers the workspace.

- [x] **Step 8: Commit the merge**

```bash
git commit --no-edit
git log --oneline -1
```

---

### Task 2: Reduce `egress.rs` to the credential path and make the boundary a test

**Files:**
- Modify: `companion/crates/server/src/egress.rs`

**Interfaces:**
- Consumes: `egress::fetch_post_form` as it exists after Task 1.
- Produces: `pub const IDENTITY_HOST: &str = "oauth2.googleapis.com";` and an `EgressError::HostNotPermitted { host: String }` variant.
- **Verified signature (do not guess):** `pub async fn fetch_post_form(url: &str, form: &[(&str, &str)]) -> Result<FetchResponse, EgressError>`. It keeps that signature and gains the host check.
- **The check goes in `fetch_post_form`, not `post_form_inner`.** Production reaches the network only through `EgressTransport::post_form` → `fetch_post_form`, so the public entry point is the real boundary. `post_form_inner` is shared with `post_form_with_resolver`, which the existing sub-project 2 test `post_form_resolves_then_pins_and_sends_the_form` drives against `http://token.invalid:PORT/token` — putting the check deeper would break that test for no security gain, since the resolver-injected path is private to the module and unreachable from production.

- [x] **Step 1: Write the failing tests**

Add to `egress.rs`'s existing `mod tests`. These assert the §1 invariant: the server may reach the identity host and nothing else. Every case must be refused **before any I/O**, like the metadata-IP tests already there.

```rust
    #[tokio::test]
    async fn the_calendar_api_host_is_not_reachable_from_the_server() {
        // www.googleapis.com is the PRODUCER's egress, never the server's.
        // If this ever passes, the server has become a content fetcher again.
        let error = fetch_post_form(
            "https://www.googleapis.com/calendar/v3/calendars/primary/events",
            &[],
        )
        .await
        .expect_err("the Calendar API must not be reachable from the server");
        assert!(matches!(error, EgressError::HostNotPermitted { .. }));
    }

    #[tokio::test]
    async fn the_consent_host_is_not_reachable_from_the_server() {
        // accounts.google.com is a 302 the BROWSER follows, not a fetch the
        // server makes, so it has no business on the allowlist.
        let error = fetch_post_form("https://accounts.google.com/o/oauth2/v2/auth", &[])
            .await
            .expect_err("the consent host must not be reachable from the server");
        assert!(matches!(error, EgressError::HostNotPermitted { .. }));
    }

    #[tokio::test]
    async fn an_arbitrary_host_is_not_reachable_from_the_server() {
        let error = fetch_post_form("https://example.invalid/token", &[])
            .await
            .expect_err("only the identity host is permitted");
        assert!(matches!(error, EgressError::HostNotPermitted { .. }));
    }

    #[tokio::test]
    async fn the_identity_host_passes_the_allowlist() {
        // Asserts only that the ALLOWLIST admits it. Resolution and transport
        // may fail in a sandbox; anything but HostNotPermitted means the host
        // cleared this check, which is what is under test.
        let outcome = fetch_post_form("https://oauth2.googleapis.com/token", &[]).await;
        assert!(!matches!(outcome, Err(EgressError::HostNotPermitted { .. })));
    }
```

- [x] **Step 2: Run them and watch them fail**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/v3-server-host/companion
cargo test -p server --lib egress:: > /tmp/t2.log 2>&1; echo "exit=$?"
```

Expected: FAIL — `EgressError` has no `HostNotPermitted` variant, so this does not compile.

- [x] **Step 3: Add the allowlist**

In `egress.rs`, add the constant and the variant, then enforce it at the top of `fetch_post_form` — after `egress_guard` parses the URL and before any resolution:

```rust
/// The server's only permitted outbound host. `accounts.google.com` is a
/// redirect the browser follows, and `www.googleapis.com` is the producer's
/// egress; neither belongs here. Widening this constant means the server has
/// started fetching something, which spec §1 forbids.
pub const IDENTITY_HOST: &str = "oauth2.googleapis.com";
```

```rust
    let url = egress_guard(url)?;
    let host = url.host_str().unwrap_or_default();
    if host != IDENTITY_HOST {
        return Err(EgressError::HostNotPermitted {
            host: host.to_owned(),
        });
    }
```

Add to `EgressError`:

```rust
    #[error("host {host} is not the permitted identity host")]
    HostNotPermitted { host: String },
```

- [x] **Step 4: Run the tests and watch them pass**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/v3-server-host/companion
cargo test -p server --lib egress:: > /tmp/t2.log 2>&1; echo "exit=$?"
```

Expected: `exit=0`.

- [x] **Step 5: Delete the plugin-feed GET path**

`fetch`, `fetch_with_budget`, `fetch_with_resolver` and `fetch_inner` served plugin feeds, which no longer exist. Remove them and any test that only covers them. Keep `FetchResponse` — `fetch_post_form` returns it — and keep `egress_guard`, `deny_reason_for_ip`, `is_globally_routable`, `resolve_and_pin`, `build_pinned_client`, `read_capped_body`.

Compile after deleting. If `cargo check` reports an unused function you kept, delete that too; if it reports a missing one you deleted, restore it. **Do not delete `deny_reason_for_ip` or the CIDR tables** even though the allowlist now makes them a second line of defence — they are what stops a DNS answer pointing the pinned connection at a private address.

- [x] **Step 6: Verify the guard still guards, by mutation**

Delete the `if host != IDENTITY_HOST` block and re-run Step 4's command. Expected: three of the four new tests FAIL. Restore the block. A boundary test that passes against a deleted boundary is not a boundary test.

- [x] **Step 7: Commit**

```bash
git add companion/crates/server/src/egress.rs
git commit -m "feat: the server's egress is one host, and a test says so

Reduces egress.rs to the credential path that survives the subtraction and
adds the allowlist spec section 1 requires: oauth2.googleapis.com and nothing
else. accounts.google.com is a browser redirect, not a server fetch, and
www.googleapis.com is the producer's egress -- both are refused, each with a
test. The plugin-feed GET path goes with the plugins it served.

Mutation-probed: deleting the host check fails three of the four tests."
```

---

### Task 3: The producer credential store

**Files:**
- Create: `companion/crates/server/src/producer_credentials.rs`
- Modify: `companion/crates/server/src/lib.rs` (add `mod producer_credentials;`)
- Test: `companion/crates/server/tests/producer_credentials.rs`

**Interfaces:**
- Consumes: `app_core::secure_file`, `crate::registry::constant_time_eq`, `protocol::digest_hex`.
- Produces (all `pub`, not `pub(crate)` — `tests/producer_credentials.rs` is an
  integration test and reaches them through `use server::producer_credentials::...`):
  - `pub struct ProducerCredentialStore`
  - `pub fn open(root: PathBuf) -> Result<Self, ProducerCredentialError>`
  - `pub fn mint(&self, integration_id: &str) -> Result<MintedProducerCredential, ProducerCredentialError>`
  - `pub fn authenticate(&self, token: &str) -> Option<String>` (returns the integration id)
  - `pub fn revoke(&self, integration_id: &str) -> Result<bool, ProducerCredentialError>`
  - `pub struct MintedProducerCredential { pub integration_id: String, pub token: String }`

- [x] **Step 1: Write the failing tests**

Create `companion/crates/server/tests/producer_credentials.rs`:

```rust
//! The producer credential follows the image-source contract: digests on disk,
//! plaintext once, constant-time comparison, independent revocation.

use server::producer_credentials::ProducerCredentialStore;

fn store() -> (tempfile::TempDir, ProducerCredentialStore) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = ProducerCredentialStore::open(dir.path().to_path_buf()).expect("open");
    (dir, store)
}

#[test]
fn a_minted_credential_authenticates_to_its_integration() {
    let (_dir, store) = store();
    let minted = store.mint("google").expect("mint");
    assert_eq!(store.authenticate(&minted.token).as_deref(), Some("google"));
}

#[test]
fn an_unknown_credential_authenticates_to_nothing() {
    let (_dir, store) = store();
    store.mint("google").expect("mint");
    assert!(store.authenticate("not-a-real-token").is_none());
}

#[test]
fn the_plaintext_credential_never_reaches_the_disk() {
    // The whole point of the digest-only contract. If this fails, a stolen
    // backup is a live Google credential.
    let (dir, store) = store();
    let minted = store.mint("google").expect("mint");
    let on_disk = std::fs::read_to_string(dir.path().join("producer-credentials.json"))
        .expect("store file");
    assert!(
        !on_disk.contains(&minted.token),
        "the plaintext credential was persisted"
    );
}

#[test]
fn minting_again_rotates_and_retires_the_previous_credential() {
    let (_dir, store) = store();
    let first = store.mint("google").expect("first mint");
    let second = store.mint("google").expect("second mint");
    assert_ne!(first.token, second.token);
    assert!(
        store.authenticate(&first.token).is_none(),
        "a rotated credential must stop working"
    );
    assert_eq!(store.authenticate(&second.token).as_deref(), Some("google"));
}

#[test]
fn revoking_stops_the_credential_and_reports_whether_one_existed() {
    let (_dir, store) = store();
    let minted = store.mint("google").expect("mint");
    assert!(store.revoke("google").expect("revoke"));
    assert!(store.authenticate(&minted.token).is_none());
    assert!(!store.revoke("google").expect("second revoke"));
}

#[test]
fn a_credential_survives_a_restart() {
    let dir = tempfile::tempdir().expect("tempdir");
    let minted = {
        let store = ProducerCredentialStore::open(dir.path().to_path_buf()).expect("open");
        store.mint("google").expect("mint")
    };
    let reopened = ProducerCredentialStore::open(dir.path().to_path_buf()).expect("reopen");
    assert_eq!(
        reopened.authenticate(&minted.token).as_deref(),
        Some("google")
    );
}
```

- [x] **Step 2: Run them and watch them fail**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/v3-server-host/companion
cargo test -p server --test producer_credentials > /tmp/t3.log 2>&1; echo "exit=$?"
```

Expected: FAIL — the module does not exist.

- [x] **Step 3: Write the store**

Create `companion/crates/server/src/producer_credentials.rs`. Persist `{ schema_version, credentials: [{ integration_id, token_sha256 }] }` through `app_core::secure_file` so the file lands atomically at `0600`, exactly as `image_sources.rs` does.

```rust
//! Producer credentials: a write-scoped secret that lets one external producer
//! ask for one integration's access token.
//!
//! Follows the contract `image_sources.rs` and `registry.rs` already use --
//! only the SHA-256 digest is persisted and the plaintext is returned exactly
//! once -- so a stolen store file yields nothing usable. A producer credential
//! is deliberately NOT the image-source token: pushing a picture and minting a
//! Google access token are different privileges and revoke independently.

use std::path::PathBuf;
use std::sync::Mutex;

use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::registry::constant_time_eq;

const STORE_FILE: &str = "producer-credentials.json";
const STORE_SCHEMA_VERSION: u32 = 1;
const MAX_STORE_FILE_BYTES: usize = 64 * 1_024;

#[derive(Debug, thiserror::Error)]
pub enum ProducerCredentialError {
    #[error("producer credential store i/o failed: {message}")]
    Io { message: String },
    #[error("producer credential store is malformed: {message}")]
    Malformed { message: String },
}

pub struct MintedProducerCredential {
    pub integration_id: String,
    /// Returned exactly once. The store keeps only its digest.
    pub token: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct PersistedCredential {
    integration_id: String,
    token_sha256: String,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct PersistedStore {
    schema_version: u32,
    credentials: Vec<PersistedCredential>,
}

struct Credential {
    integration_id: String,
    token_digest: [u8; 32],
}

pub struct ProducerCredentialStore {
    root: PathBuf,
    state: Mutex<Vec<Credential>>,
}
```

Then `open` (read-or-empty, decode digests), `mint` (generate, replace any existing entry for that integration id, save, return plaintext once), `authenticate` (digest the presented token, scan with `constant_time_eq`, return the integration id), and `revoke` (drop the entry, save, report whether one existed).

Reuse the `random_token` and `token_digest` shapes from `image_sources.rs` verbatim — 32 random bytes rendered by `protocol::digest_hex`, and `Sha256::digest(token.as_bytes()).into()`. **Scan every entry on a miss and compare with `constant_time_eq`**, never `==`: how many leading bytes a wrong credential shares with a real one must not be observable as a timing difference.

Recover a poisoned lock with `unwrap_or_else(std::sync::PoisonError::into_inner)`, as `registry.rs` does — a panic mid-request must not turn every later authentication into a panic on an internet-facing endpoint.

Add `pub mod producer_credentials;` to `lib.rs`.

- [x] **Step 4: Run the tests and watch them pass**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/v3-server-host/companion
cargo test -p server --test producer_credentials > /tmp/t3.log 2>&1; echo "exit=$?"
```

Expected: `exit=0`, six tests passing.

- [x] **Step 5: Probe the two tests that matter**

Change `constant_time_eq` to `==` and re-run: every test still passes, which is expected and is exactly why that line needs the comment explaining it rather than a test. Restore it. Then change `mint` to persist `token` instead of its digest and re-run: `the_plaintext_credential_never_reaches_the_disk` must FAIL. Restore. A test suite that cannot see a plaintext credential hit the disk is not protecting the thing this store exists for.

- [x] **Step 6: Commit**

```bash
git add companion/crates/server/src/producer_credentials.rs companion/crates/server/src/lib.rs companion/crates/server/tests/producer_credentials.rs
git commit -m "feat: producer credentials, digest-only and independently revocable

A producer credential lets one producer ask for one integration's access
token. It is deliberately not the image-source token: pushing a picture and
minting a Google credential are different privileges that must revoke
independently.

Follows the registry contract -- digests on disk, plaintext returned once,
constant-time comparison, poisoned locks recovered. Mutation-probed: writing
the plaintext instead of the digest fails the disk test."
```

---

### Task 4: The vend route

**Files:**
- Modify: `companion/crates/server/src/oauth/token.rs`
- Modify: `companion/crates/server/src/oauth/routes.rs`
- Modify: `companion/crates/server/src/lib.rs`, `src/main.rs`
- Modify: `companion/crates/server/tests/oauth_routes.rs`
- Modify: `docs/images/producer-guide.md`

**Interfaces:**
- Consumes: `ProducerCredentialStore` (Task 3), `TokenManager::access_token` and `TokenManager::revoke` (sub-project 2).
- Produces:
  - `TokenManager::access_token_with_expiry(&self, integration_id: &str) -> Result<(String, DateTime<Utc>), TokenError>`
  - `POST /v1/integrations/{id}/producer` (admin) → `201 { "integration_id": "...", "token": "..." }`
  - `DELETE /v1/integrations/{id}/producer` (admin) → `204`
  - `POST /v1/integrations/{id}/token` (producer bearer) → `200 { "access_token": "...", "expires_at": "RFC3339" }`

- [x] **Step 1: Thread the expiry through `TokenManager`**

`CachedToken` already holds `expires_at: DateTime<Utc>`; only the accessor drops it. Change the private `cached_valid` to return `Option<(String, DateTime<Utc>)>` — `(token.access_token.clone(), token.expires_at)` — then add:

```rust
    /// The live access token and the expiry Google gave it. The vend route
    /// needs both; `access_token` stays the convenience wrapper so sub-project
    /// 2's callers are untouched.
    pub async fn access_token_with_expiry(
        &self,
        integration_id: &str,
    ) -> Result<(String, DateTime<Utc>), TokenError> {
        if let Some(pair) = self.cached_valid(integration_id) {
            return Ok(pair);
        }
        let gate = self.refresh_gate(integration_id);
        let _turn = gate.lock().await;
        if let Some(pair) = self.cached_valid(integration_id) {
            return Ok(pair);
        }
        // ...the rest of today's `access_token` body unchanged, returning the
        // `(token, expires_at)` pair `cache_token` just stored rather than the
        // bare string.
    }

    pub async fn access_token(&self, integration_id: &str) -> Result<String, TokenError> {
        self.access_token_with_expiry(integration_id)
            .await
            .map(|(token, _)| token)
    }
```

**Keep the per-integration refresh gate exactly where it is.** It is the fix the 2026-09-09 review forced, and N producers polling is precisely the shape it exists for. If you find yourself moving or removing the `let _turn = gate.lock().await;` line, you are reintroducing the refresh stampede.

- [x] **Step 2: Write the failing route tests**

Append to `companion/crates/server/tests/oauth_routes.rs`, following the harness that file already sets up for sub-project 2.

```rust
#[tokio::test]
async fn a_producer_credential_vends_an_access_token() {
    let harness = harness_with_connected_integration().await;
    let minted = harness.mint_producer_credential("google").await;

    let response = harness
        .post("/v1/integrations/google/token")
        .bearer(&minted.token)
        .send()
        .await;

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await;
    assert!(body.get("access_token").is_some());
    assert!(body.get("expires_at").is_some());
}

#[tokio::test]
async fn the_vend_response_carries_no_refresh_token_or_client_secret() {
    // The custodian boundary: a producer gets a credential Google can revoke,
    // never the grant that mints them.
    let harness = harness_with_connected_integration().await;
    let minted = harness.mint_producer_credential("google").await;

    let body = harness
        .post("/v1/integrations/google/token")
        .bearer(&minted.token)
        .send()
        .await
        .text()
        .await;

    assert!(!body.contains(harness.refresh_token()));
    assert!(!body.contains(harness.client_secret()));
    assert!(!body.to_lowercase().contains("refresh_token"));
}

#[tokio::test]
async fn an_unauthenticated_vend_is_refused() {
    let harness = harness_with_connected_integration().await;
    let response = harness.post("/v1/integrations/google/token").send().await;
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn the_admin_token_does_not_vend() {
    // An admin credential living in a producer's environment is the thing the
    // separate producer credential exists to avoid.
    let harness = harness_with_connected_integration().await;
    let response = harness
        .post("/v1/integrations/google/token")
        .bearer(harness.admin_token())
        .send()
        .await;
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn a_credential_for_another_integration_cannot_vend() {
    // Uniform 401, not 403: a distinguishable rejection would tell an attacker
    // which integration ids exist.
    let harness = harness_with_connected_integration().await;
    let minted = harness.mint_producer_credential("google").await;
    let response = harness
        .post("/v1/integrations/dropbox/token")
        .bearer(&minted.token)
        .send()
        .await;
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn a_revoked_grant_vends_needs_reconnect_not_a_generic_error() {
    let harness = harness_with_invalid_grant().await;
    let minted = harness.mint_producer_credential("google").await;

    let response = harness
        .post("/v1/integrations/google/token")
        .bearer(&minted.token)
        .send()
        .await;

    assert_eq!(response.status(), 409);
    assert!(response.text().await.contains("reconnect"));
}
```

Add the two harness helpers the tests use — `mint_producer_credential`, and `harness_with_invalid_grant` queueing the fake transport with an `invalid_grant` response — beside the existing sub-project 2 helpers.

- [x] **Step 3: Run them and watch them fail**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/v3-server-host/companion
cargo test -p server --test oauth_routes > /tmp/t4.log 2>&1; echo "exit=$?"
```

Expected: FAIL — the routes do not exist.

- [x] **Step 4: Add the routes**

In `oauth/routes.rs`, add a `ProducerAuthenticated(String)` extractor that reads the bearer token, calls `ProducerCredentialStore::authenticate`, and yields the integration id — rejecting with `401` when absent, unknown, **or not equal to the path's `{id}`**. Then:

```rust
async fn vend_token(
    State(state): State<ServerState>,
    ProducerAuthenticated(integration_id): ProducerAuthenticated,
) -> Result<Json<VendedToken>, OAuthRouteError> {
    let runtime = state.integrations().ok_or(OAuthRouteError::NotConfigured)?;
    let (access_token, expires_at) = runtime
        .token_manager()
        .access_token_with_expiry(&integration_id)
        .await
        .map_err(map_vend_error)?;
    Ok(Json(VendedToken {
        access_token,
        expires_at,
    }))
}
```

`map_vend_error` maps `TokenError::NeedsReconnect` → `409` with a body naming reconnection, `NotFound` → `404`, and everything else → `502` with **no detail from the provider body** — a token-endpoint error can quote the request back.

The mint and revoke routes go through `spawn_blocking`, because the store writes to disk under a sync `Mutex`:

```rust
    let minted = tokio::task::spawn_blocking(move || state.producer_credentials().mint(&id))
        .await
        .map_err(|_| OAuthRouteError::WorkerFailed)?
        .map_err(|error| OAuthRouteError::Store(error.to_string()))?;
```

Wire `ProducerCredentialStore` into `StateInner` with an accessor beside the existing ones, and open it in `main.rs` next to the integration store.

- [x] **Step 5: Run the tests and watch them pass**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/v3-server-host/companion
cargo test -p server --test oauth_routes > /tmp/t4.log 2>&1; echo "exit=$?"
```

Expected: `exit=0`, with sub-project 2's existing tests still green.

- [x] **Step 6: Prove the route takes no producer-supplied target**

Spec §5 names this as the trap the route must not reintroduce: a vend route that
accepted a URL, host, or scope from its caller would recreate the SSRF surface the
deleted egress guard existed to bound, without the guard. The handler's signature is the
real defence — it takes no body and no query — and this test pins it.

```rust
#[tokio::test]
async fn a_producer_supplied_target_is_ignored_entirely() {
    let harness = harness_with_connected_integration().await;
    let minted = harness.mint_producer_credential("google").await;

    let response = harness
        .post("/v1/integrations/google/token")
        .bearer(&minted.token)
        .json(&serde_json::json!({
            "token_uri": "https://attacker.example/token",
            "scope": "https://www.googleapis.com/auth/drive",
        }))
        .send()
        .await;

    // The body is not an error and not an instruction: it is ignored, and the
    // integration record remains the only source of host and scope.
    assert_eq!(response.status(), 200);
    assert_eq!(harness.transport_hosts_called().await, vec!["oauth2.googleapis.com"]);
}
```

Run it, watch it pass without new code (the handler already takes no body), then probe:
add a `Json<VendRequest>` parameter that reads `token_uri` and routes on it, and confirm
the assertion on `transport_hosts_called` fails. Remove the parameter again.

- [x] **Step 7: Verify the `spawn_blocking` discipline by inspection**

Spec §13.1 requires it and no test can see it — a direct call compiles and passes, it
just stalls the executor under load. `IntegrationStore` and `ProducerCredentialStore`
both hold a sync `Mutex` across disk I/O.

```bash
grep -n "producer_credentials()\.\(mint\|revoke\)\|integrations()\.\(put\|remove\)" companion/crates/server/src/oauth/routes.rs
```

Every hit must sit inside a `tokio::task::spawn_blocking(...)` closure. The 2026-09-09
review verified this rule by inspection too; it found zero violations, and that is the
standard to hold.

- [x] **Step 8: Document the route for producers**

Add a section to `docs/images/producer-guide.md`, after "Authentication", written for the producer the way the rest of that file is:

```markdown
## Obtain a third-party access token (integrations only)

A producer that renders someone's calendar or mail needs a credential for that
service. Deskmate holds the OAuth grant; you ask it for a short-lived access
token and call the API yourself.

```sh
curl -X POST -H 'Authorization: Bearer PRODUCER_CREDENTIAL' \
  https://deskmate.rodi.one/v1/integrations/google/token
```

```json
{ "access_token": "ya29....", "expires_at": "2026-09-12T15:04:05Z" }
```

The token is the provider's own, with the provider's own expiry — typically one
hour. Cache it until `expires_at` and ask again; do not ask per request. The
producer credential is separate from your image-source token and is revoked
separately, so pushing pictures keeps working if the integration is
disconnected.

| Status | Meaning | Producer action |
| --- | --- | --- |
| 401 | The producer credential is invalid, revoked, or belongs to another integration. | Ask the operator to mint a new one. |
| 404 | No such integration. | Check the integration id in the URL. |
| 409 | The owner's authorization was revoked or expired. | Stop polling and tell the operator to reconnect; retrying cannot fix this. |
| 502 | The provider's token endpoint failed. | Retry with backoff; keep pushing your last good frame meanwhile. |
```

Note in `deploy/deskmate-server.env.example` that producer credentials live in `producer-credentials.json` under the config root and need no new environment variable.

- [x] **Step 9: Commit**

```bash
git add companion/crates/server/src/oauth/ companion/crates/server/src/lib.rs companion/crates/server/src/main.rs companion/crates/server/tests/oauth_routes.rs docs/images/producer-guide.md companion/crates/server/deploy/deskmate-server.env.example
git commit -m "feat: vend the provider's access token to an authenticated producer

The server is a custodian, not an issuer: it hands out Google's own access
token with Google's own expiry, so revoking at the Google account page reaches
the producer. Never the refresh token, never the client secret -- with a test
that greps the response body for both.

A producer credential that names another integration is refused with a uniform
401 rather than a 403, so the rejection does not enumerate integration ids.
An invalid_grant vends 409 naming reconnection, because no amount of retrying
fixes a revoked grant."
```

---

### Task 5: Revocation coupling, and the full gate

**Files:**
- Modify: `companion/crates/server/src/oauth/routes.rs`
- Modify: `companion/crates/server/tests/oauth_routes.rs`

**Interfaces:**
- Consumes: everything above.
- Produces: no new signatures. Revoking an integration also revokes its producer credential.

- [x] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn revoking_an_integration_also_revokes_its_producer_credential() {
    // Otherwise a producer keeps a credential against a dead integration and
    // the failure surfaces as a confusing 401 loop instead of the operator
    // seeing NeedsReconnect.
    let harness = harness_with_connected_integration().await;
    let minted = harness.mint_producer_credential("google").await;

    let revoked = harness
        .post("/v1/integrations/google/revoke")
        .bearer(harness.admin_token())
        .send()
        .await;
    assert!(revoked.status().is_success());

    let response = harness
        .post("/v1/integrations/google/token")
        .bearer(&minted.token)
        .send()
        .await;
    assert_eq!(response.status(), 401);
}
```

- [x] **Step 2: Run it and watch it fail**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/v3-server-host/companion
cargo test -p server --test oauth_routes revoking_an_integration > /tmp/t5.log 2>&1; echo "exit=$?"
```

Expected: FAIL — the credential still authenticates after the integration is gone.

- [x] **Step 3: Couple the two revocations**

In the existing revoke handler, after `TokenManager::revoke` succeeds, revoke the producer credential for the same integration through `spawn_blocking`. Revoke the credential **even if the remote revoke failed** — `TokenManager::revoke` already removes local state regardless, because the operator asked to disconnect, and leaving a live producer credential behind would contradict that.

- [x] **Step 4: Run it and watch it pass**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/v3-server-host/companion
cargo test -p server --test oauth_routes > /tmp/t5.log 2>&1; echo "exit=$?"
```

Expected: `exit=0`.

- [x] **Step 5: Run the full workspace gate**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/v3-server-host/companion
cargo fmt --all --check > /tmp/g-fmt.log 2>&1; echo "fmt=$?"
cargo clippy --workspace --all-targets -- -D warnings > /tmp/g-clippy.log 2>&1; echo "clippy=$?"
cargo test --workspace --all-targets > /tmp/g-test.log 2>&1; echo "test=$?"
cargo test --workspace --doc > /tmp/g-doc.log 2>&1; echo "doc=$?"
cd ../apps/deskmate && bun test && bun run check
```

Expected: every gate `0`. **The controller runs these**, not a sandboxed subagent — `server/tests/` does real loopback binds.

- [x] **Step 6: Commit**

```bash
git add companion/crates/server/src/oauth/routes.rs companion/crates/server/tests/oauth_routes.rs
git commit -m "fix: revoking an integration revokes its producer credential

A producer holding a live credential against a revoked integration would loop
on 401 while the operator sees nothing actionable. The credential goes even
when the remote revoke fails, matching TokenManager::revoke's existing rule
that local state goes because the operator asked to disconnect."
```

---

## What this plan deliberately does not do

- **No management surface.** Sub-project 4 gets its own plan, written at this one's exit so its findings feed forward — the repo's standing practice. Until then the mint route is the operator's interface, via `curl` with the admin token.
- **No reference producer.** It belongs in `tools/picture-producers/` beside `claude_limits_png.py`, and it needs a real Google grant to develop against, so it is its own sub-project after the management surface. This plan ends when the seam works and is tested.
- **No `AdminAuthenticated` unification.** It is duplicated between `admin.rs` and `images.rs` and the management surface will be its third consumer; the spec assigns that cleanup to sub-project 4, where a third copy would otherwise appear.
- **No hardware.** V3 touches no firmware. The end-to-end gate (spec §13.2) needs the panel, and therefore waits on the protocol-v2 flash — but nothing in this plan does.
