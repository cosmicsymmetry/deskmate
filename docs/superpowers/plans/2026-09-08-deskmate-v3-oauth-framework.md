# V3 OAuth Integration Framework (Google first) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **STATUS (recorded 2026-09-08): Tasks 1-7 are IMPLEMENTED AND COMMITTED; the
> checkboxes below were never ticked and are NOT a progress signal — read the commits,
> not the boxes.** One commit per task, `ca319af`..`ee69120`: the egress-guarded POST-form
> helper, PKCE/`state` primitives, the OAuth transport seam and Google token-response
> classifier, `TokenManager`, the operator session cookie and `OperatorAuthenticated`
> gate, the consent/callback/revoke routes with the `IntegrationRuntime` stash, and the
> `main.rs` wiring. What the boxes cannot tell you is what remains: this sub-project has
> had **no whole-branch review**, and the plan's own Task 6 Step 5 route integration test
> is controller-run — Codex's sandbox denies loopback binds, so a green report from it
> does not cover those. Verify both before calling sub-project 2 complete.

**Goal:** Build the server-side OAuth framework — a `TokenManager` (code exchange, skew-window refresh, revoke, typed health), a provider-agnostic token transport routed through the egress guard, the three consent/callback/revoke routes, and the admin-cookie gate they sit behind — with Google wired as the first (and only) identity.

**Architecture:** All in the `server` crate. A new `oauth` module holds PKCE/`state` primitives, an injectable `OAuthTransport` seam (production impl calls the egress guard, tests inject a fake), a `TokenManager` that owns the in-memory access-token cache and drives the existing `IntegrationStore` for durable refresh-token/client-secret storage, an operator session cookie signed with HMAC-SHA256 keyed by the existing `DESKMATE_ADMIN_TOKEN`, and the Axum routes. The egress guard gains a POST-form helper (it was GET-only) so credential-bearing calls keep resolve-then-pin. No `app-core`, config-schema, protocol, or firmware change.

**Tech Stack:** Rust (server crate), Axum 0.8, `reqwest` (through the egress guard only), `chacha20poly1305`/`IntegrationStore` (sub-project 1, already committed), `hmac` + `sha2` (cookie signing, PKCE challenge), `base64` (URL-safe), `rand` 0.9 (state/verifier/sid entropy), `chrono` (expiry/TTL), `serde`/`serde_json` (token responses).

**Spec:** `docs/superpowers/specs/2026-09-06-deskmate-v3-server-host-design.md` (§2 architecture, §3 OAuth flow end-to-end, §6 security, §8 sub-project 2).

## Global Constraints

- **Server crate only.** No edits to `app-core`, `providers`, `plugin`, `protocol`, or firmware. **No `CalendarSource::Google`, no schema v7** — that is sub-project 3. This sub-project is provider-agnostic plumbing; only Google is wired.
- **Every Google HTTPS call goes through the egress guard** (`egress.rs`), never a raw `reqwest::Client`. Google's hosts (`accounts.google.com`, `oauth2.googleapis.com`) are public unicast and pass the allowlist naturally; **add no per-host exception.** The guard is GET-only today, so Task 1 adds a POST-form helper that reuses `egress_guard`, resolve-then-pin, `build_pinned_client`, and `read_capped_body`.
- **`IntegrationStore::{get,put,remove}` hold a `std::sync::Mutex` across their disk I/O** (seal + `write_and_replace` + fsync). Every call from an async handler or from `TokenManager` MUST go through `tokio::task::spawn_blocking` with a cloned `Arc<IntegrationStore>`; never call them directly on the async executor. `TokenManager`'s private `store_get`/`store_put`/`store_remove` are the only call sites and each wraps `spawn_blocking`.
- **Access tokens live in memory only, never on disk.** `IntegrationStore` persists only `refresh_token` + `client_secret`, written at authorize and revoke — **not on refresh** (Google refresh tokens do not rotate).
- **Unauthenticated ⇒ bare 401**, matching `auth.rs`: the rejection carries no body and never logs the presented credential.
- **`state` + PKCE stash is short-TTL (600 s), single-use (removed on callback), and bound to the operator session** (`sid`); the callback verifies `state` before any token exchange (CSRF).
- **Testability without network:** `TokenManager` is generic over `Arc<dyn OAuthTransport>` and an injected clock, so refresh/exchange/revoke/health are unit-tested against a fake. Route and egress tests that bind loopback **cannot run under Codex's sandbox** (project memory: sandbox denies loopback binds); the controller runs those.
- **Pin exact dependency versions**, mirroring `server/Cargo.toml`'s `resvg = "=0.45.1"`: add `hmac = "=0.12.1"` (RustCrypto, pairs with the existing `sha2 = "0.10"`).
- **Verification per `CLAUDE.md`:** from `companion/`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --all-targets`, `cargo test --workspace --doc`.
- **Dashboard is out of scope.** `GET /v1/manage` (HTML) is sub-project 4. This plan's callback redirects to `/v1/manage` with `Redirect::to`; until sub-project 4 exists that path 404s, which is acceptable — the redirect target is a URL, not a dependency.

---

### Task 1: Extend the egress guard with a POST-form helper

`egress::fetch` is GET-only. Add `fetch_post_form`, reusing every guard step, and extract the pinned-client builder so the GET and POST paths share one audited construction.

**Files:**
- Modify: `companion/crates/server/src/egress.rs`

**Interfaces:**
- Consumes: `egress_guard`, `HopResolver`/`RealResolver`, `read_capped_body`, `FetchResponse`, `EgressError`, `TOTAL_FETCH_BUDGET`, `describe_reqwest_error`, `REQUEST_TIMEOUT` (all existing in `egress.rs`).
- Produces:
  - `pub async fn fetch_post_form(url: &str, form: &[(&str, &str)]) -> Result<FetchResponse, EgressError>` — single-hop POST of an `application/x-www-form-urlencoded` body under the full guard; a 3xx is returned as-is (POST redirects are not followed).
  - `fn build_pinned_client(host: &str, pinned_addr: std::net::SocketAddr) -> Result<reqwest::Client, EgressError>` (private) — the shared `resolve`/`no_proxy`/`redirect(none)`/`timeout` builder.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` in `egress.rs`:

```rust
    #[tokio::test]
    async fn post_form_refuses_the_cloud_metadata_address() {
        // Literal-IP deny check runs before any network I/O, so this is offline.
        let error = super::fetch_post_form("http://169.254.169.254/token", &[("a", "b")])
            .await
            .expect_err("metadata address must be denied");
        assert!(matches!(error, EgressError::Denied { .. }));
    }

    #[tokio::test]
    async fn post_form_rejects_a_non_http_scheme() {
        let error = super::fetch_post_form("ftp://example.com/token", &[])
            .await
            .expect_err("non-http scheme must be rejected");
        assert!(matches!(error, EgressError::UnsupportedScheme(_)));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd companion && cargo test -p server --lib egress::tests::post_form`
Expected: FAIL — `cannot find function fetch_post_form in module super`.

- [ ] **Step 3: Write the implementation**

In `egress.rs`, add the shared client builder (place it just above `fetch_inner`) and refactor `fetch_inner` to use it. Replace the inline client-build block inside `fetch_inner`'s loop:

```rust
        let client = reqwest::Client::builder()
            .resolve(&host, pinned_addr)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(REQUEST_TIMEOUT)
            .no_proxy()
            .build()
            .map_err(|error| EgressError::Request(describe_reqwest_error(&error)))?;
```

with:

```rust
        let client = build_pinned_client(&host, pinned_addr)?;
```

and add the extracted builder plus the POST path:

```rust
/// Builds the reqwest client used for one pinned hop, shared by the GET
/// ([`fetch`]) and POST ([`fetch_post_form`]) paths so both get identical
/// resolve-then-pin, no-redirect, timeout and no-proxy behaviour.
///
/// DO NOT REMOVE `.no_proxy()`: reqwest's `auto_sys_proxy` defaults to true and
/// the underlying hyper-util connector reads HTTP_PROXY/HTTPS_PROXY/ALL_PROXY
/// from the environment unconditionally. If any were set, the connection would
/// go to the proxy instead of `pinned_addr`, silently defeating resolve-then-pin
/// -- the `.resolve()` override would never be consulted. `.no_proxy()` clears
/// any configured proxy and disables that environment lookup.
fn build_pinned_client(
    host: &str,
    pinned_addr: SocketAddr,
) -> Result<reqwest::Client, EgressError> {
    reqwest::Client::builder()
        .resolve(host, pinned_addr)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(REQUEST_TIMEOUT)
        .no_proxy()
        .build()
        .map_err(|error| EgressError::Request(describe_reqwest_error(&error)))
}

/// POSTs `form` as `application/x-www-form-urlencoded` to `url` under the full
/// egress guard. Used for OAuth token exchange, refresh, and revoke (spec §6:
/// credential-bearing calls keep resolve-then-pin). Single-hop by design: a
/// token endpoint answering a POST with a redirect is not a flow to follow, so a
/// 3xx is returned to the caller as-is (and treated as an error there) rather
/// than re-issued as a POST or silently downgraded to GET.
pub async fn fetch_post_form(url: &str, form: &[(&str, &str)]) -> Result<FetchResponse, EgressError> {
    post_form_with_resolver(url, &RealResolver, form).await
}

async fn post_form_with_resolver(
    url: &str,
    resolver: &impl HopResolver,
    form: &[(&str, &str)],
) -> Result<FetchResponse, EgressError> {
    match tokio::time::timeout(TOTAL_FETCH_BUDGET, post_form_inner(url, resolver, form)).await {
        Ok(result) => result,
        Err(_elapsed) => Err(EgressError::Timeout),
    }
}

async fn post_form_inner(
    url: &str,
    resolver: &impl HopResolver,
    form: &[(&str, &str)],
) -> Result<FetchResponse, EgressError> {
    let current = egress_guard(url)?;
    let (host, pinned_addr) = resolver.resolve(&current).await?;
    let client = build_pinned_client(&host, pinned_addr)?;
    let response = client
        .post(current.clone())
        .form(form)
        .send()
        .await
        .map_err(|error| EgressError::Request(describe_reqwest_error(&error)))?;
    let status = response.status().as_u16();
    let body = read_capped_body(response).await?;
    Ok(FetchResponse { status, body })
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd companion && cargo test -p server --lib egress::`
Expected: PASS — the two new tests plus every existing egress test (the `build_pinned_client` refactor is behaviour-preserving for the GET path).

- [ ] **Step 5: Commit**

```bash
git add companion/crates/server/src/egress.rs
git commit -m "feat: add egress-guarded POST-form fetch for OAuth token calls"
```

---

### Task 2: PKCE and `state` primitives

Pure, fully unit-testable generators for the CSRF `state`, the PKCE `code_verifier`, and its S256 `code_challenge`.

**Files:**
- Create: `companion/crates/server/src/oauth/mod.rs`
- Create: `companion/crates/server/src/oauth/pkce.rs`
- Modify: `companion/crates/server/src/lib.rs` (add `pub mod oauth;` beside the other `pub mod` lines)

**Interfaces:**
- Consumes: `rand` 0.9, `sha2`, `base64` (all already available to the server crate).
- Produces:
  - `pub struct PkcePair { pub verifier: String, pub challenge: String }`
  - `pub fn generate_pkce() -> PkcePair` — 32 random bytes → base64url-no-pad verifier; challenge = base64url-no-pad(SHA-256(verifier)).
  - `pub fn challenge_for(verifier: &str) -> String`
  - `pub fn generate_state() -> String` — 32 random bytes, base64url-no-pad.

- [ ] **Step 1: Write the failing test**

Create `companion/crates/server/src/oauth/pkce.rs`:

```rust
//! PKCE (RFC 7636) and CSRF-`state` primitives for the OAuth consent flow.

use base64::prelude::{BASE64_URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use sha2::{Digest, Sha256};

pub struct PkcePair {
    pub verifier: String,
    pub challenge: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_is_the_base64url_sha256_of_the_verifier() {
        let pair = generate_pkce();
        assert_eq!(challenge_for(&pair.verifier), pair.challenge);
    }

    #[test]
    fn verifier_meets_rfc7636_length_and_charset() {
        let pair = generate_pkce();
        // 32 bytes base64url-no-pad = 43 chars, inside RFC 7636's 43..=128.
        assert_eq!(pair.verifier.len(), 43);
        assert!(
            pair.verifier
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "verifier must be base64url with no padding"
        );
        assert!(!pair.challenge.contains('='), "challenge must be unpadded");
    }

    #[test]
    fn generated_values_are_distinct_each_call() {
        assert_ne!(generate_state(), generate_state());
        assert_ne!(generate_pkce().verifier, generate_pkce().verifier);
    }

    #[test]
    fn challenge_for_matches_a_known_vector() {
        // RFC 7636 Appendix B verifier/challenge pair.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            challenge_for(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }
}
```

Create `companion/crates/server/src/oauth/mod.rs`:

```rust
//! Server-side OAuth integration framework (spec §2, §3, §6). Provider-agnostic
//! plumbing with Google wired as the first identity; no `app-core`, config, or
//! firmware surface is touched.

pub mod pkce;
```

Add to `companion/crates/server/src/lib.rs`, beside the existing `pub mod` declarations:

```rust
pub mod oauth;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd companion && cargo test -p server --lib oauth::pkce`
Expected: FAIL — `cannot find function generate_pkce` / `challenge_for` / `generate_state`.

- [ ] **Step 3: Write the implementation**

Add to `oauth/pkce.rs` (above the test module):

```rust
/// Generates a fresh PKCE verifier and its S256 challenge.
pub fn generate_pkce() -> PkcePair {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let verifier = BASE64_URL_SAFE_NO_PAD.encode(bytes);
    let challenge = challenge_for(&verifier);
    PkcePair { verifier, challenge }
}

/// The S256 `code_challenge` for a verifier: base64url-no-pad(SHA-256(verifier)).
#[must_use]
pub fn challenge_for(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    BASE64_URL_SAFE_NO_PAD.encode(digest)
}

/// A random opaque CSRF `state` value.
#[must_use]
pub fn generate_state() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    BASE64_URL_SAFE_NO_PAD.encode(bytes)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd companion && cargo test -p server --lib oauth::pkce`
Expected: PASS — all four tests, including the RFC 7636 known-answer vector.

- [ ] **Step 5: Commit**

```bash
git add companion/crates/server/src/oauth/mod.rs companion/crates/server/src/oauth/pkce.rs companion/crates/server/src/lib.rs
git commit -m "feat: add PKCE and CSRF-state primitives for the OAuth consent flow"
```

---

### Task 3: OAuth transport seam and Google token-response parsing

The injectable `OAuthTransport` (production = egress-guarded POST; tests = fake) and the pure classification of a Google token endpoint response into success or a typed endpoint error.

**Files:**
- Create: `companion/crates/server/src/oauth/transport.rs`
- Modify: `companion/crates/server/src/oauth/mod.rs` (add `pub mod transport;`)

**Interfaces:**
- Consumes: `crate::egress::{fetch_post_form, FetchResponse}` (Task 1), `serde_json`.
- Produces:
  - `pub type OAuthFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;`
  - `pub trait OAuthTransport: Send + Sync { fn post_form(&self, url: String, form: Vec<(String, String)>) -> OAuthFuture<'_, Result<FetchResponse, TransportError>>; }`
  - `pub struct TransportError(pub String)` (`Debug`, `Clone`, `thiserror::Error`)
  - `pub struct EgressTransport;` implementing `OAuthTransport` via `fetch_post_form`.
  - `pub struct GoogleTokenResponse { pub access_token: String, pub expires_in: u64, pub refresh_token: Option<String> }` (serde `Deserialize`, `#[serde(default)]` on `refresh_token`/`expires_in`)
  - `pub enum TokenEndpointError { InvalidGrant, Provider { status: u16, detail: String }, Malformed(String) }` (`Debug`, `thiserror::Error`)
  - `pub fn classify_token_response(status: u16, body: &[u8]) -> Result<GoogleTokenResponse, TokenEndpointError>`

- [ ] **Step 1: Write the failing test**

Create `companion/crates/server/src/oauth/transport.rs`:

```rust
//! The token-endpoint transport seam and Google response classification.
//!
//! Production sends through the egress guard (spec §6); tests inject a fake so
//! exchange/refresh/revoke are proven without touching Google or the network.

use std::pin::Pin;

use serde::Deserialize;

use crate::egress::{self, FetchResponse};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_a_successful_exchange() {
        let body = br#"{"access_token":"at","expires_in":3600,"refresh_token":"rt"}"#;
        let parsed = classify_token_response(200, body).expect("ok");
        assert_eq!(parsed.access_token, "at");
        assert_eq!(parsed.expires_in, 3600);
        assert_eq!(parsed.refresh_token.as_deref(), Some("rt"));
    }

    #[test]
    fn classifies_a_refresh_without_a_new_refresh_token() {
        let body = br#"{"access_token":"at2","expires_in":3599}"#;
        let parsed = classify_token_response(200, body).expect("ok");
        assert_eq!(parsed.refresh_token, None);
    }

    #[test]
    fn a_200_without_access_token_is_malformed() {
        let body = br#"{"expires_in":3600}"#;
        assert!(matches!(
            classify_token_response(200, body),
            Err(TokenEndpointError::Malformed(_))
        ));
    }

    #[test]
    fn invalid_grant_is_its_own_variant() {
        let body = br#"{"error":"invalid_grant","error_description":"expired"}"#;
        assert!(matches!(
            classify_token_response(400, body),
            Err(TokenEndpointError::InvalidGrant)
        ));
    }

    #[test]
    fn other_4xx_errors_are_provider_errors() {
        let body = br#"{"error":"invalid_client"}"#;
        match classify_token_response(401, body) {
            Err(TokenEndpointError::Provider { status, detail }) => {
                assert_eq!(status, 401);
                assert!(detail.contains("invalid_client"));
            }
            other => panic!("expected Provider, got {other:?}"),
        }
    }

    #[test]
    fn a_5xx_is_a_provider_error_even_with_an_opaque_body() {
        assert!(matches!(
            classify_token_response(503, b"upstream down"),
            Err(TokenEndpointError::Provider { status: 503, .. })
        ));
    }
}
```

Add `pub mod transport;` to `oauth/mod.rs`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd companion && cargo test -p server --lib oauth::transport`
Expected: FAIL — `cannot find function classify_token_response` / types undefined.

- [ ] **Step 3: Write the implementation**

Add to `oauth/transport.rs` (above the test module):

```rust
/// Boxed future returned by [`OAuthTransport`] so the trait stays object-safe
/// (`Arc<dyn OAuthTransport>` is what `TokenManager` holds).
pub type OAuthFuture<'a, T> = Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, thiserror::Error)]
#[error("oauth transport error: {0}")]
pub struct TransportError(pub String);

/// The seam a [`TokenManager`](super::token::TokenManager) uses to reach the
/// token/revoke endpoints. Production is [`EgressTransport`]; tests inject a fake.
pub trait OAuthTransport: Send + Sync {
    fn post_form(
        &self,
        url: String,
        form: Vec<(String, String)>,
    ) -> OAuthFuture<'_, Result<FetchResponse, TransportError>>;
}

/// Production transport: every call goes through the egress guard (spec §6).
pub struct EgressTransport;

impl OAuthTransport for EgressTransport {
    fn post_form(
        &self,
        url: String,
        form: Vec<(String, String)>,
    ) -> OAuthFuture<'_, Result<FetchResponse, TransportError>> {
        Box::pin(async move {
            let pairs: Vec<(&str, &str)> =
                form.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            egress::fetch_post_form(&url, &pairs)
                .await
                .map_err(|error| TransportError(error.to_string()))
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct GoogleTokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub expires_in: u64,
    #[serde(default)]
    pub refresh_token: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum TokenEndpointError {
    /// The grant was rejected as no longer valid (revoked, or refresh token
    /// expired). The operator must re-consent -- distinct from a transient error.
    #[error("authorization was revoked or expired (invalid_grant)")]
    InvalidGrant,
    /// Any other non-success status.
    #[error("token endpoint returned {status}: {detail}")]
    Provider { status: u16, detail: String },
    /// A 200 whose body could not be parsed as a token response.
    #[error("token endpoint returned an unparseable success body: {0}")]
    Malformed(String),
}

#[derive(Debug, Deserialize)]
struct TokenErrorBody {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

/// Classifies a token/refresh endpoint response. Pure over `(status, body)` so
/// the whole decision tree is unit-tested without a transport.
pub fn classify_token_response(
    status: u16,
    body: &[u8],
) -> Result<GoogleTokenResponse, TokenEndpointError> {
    if (200..300).contains(&status) {
        return serde_json::from_slice::<GoogleTokenResponse>(body)
            .map_err(|error| TokenEndpointError::Malformed(error.to_string()));
    }
    if let Ok(parsed) = serde_json::from_slice::<TokenErrorBody>(body) {
        if parsed.error == "invalid_grant" {
            return Err(TokenEndpointError::InvalidGrant);
        }
        let detail = match parsed.error_description {
            Some(description) => format!("{}: {description}", parsed.error),
            None => parsed.error,
        };
        return Err(TokenEndpointError::Provider { status, detail });
    }
    Err(TokenEndpointError::Provider {
        status,
        detail: String::from_utf8_lossy(body).chars().take(128).collect(),
    })
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd companion && cargo test -p server --lib oauth::transport`
Expected: PASS — all six classification tests.

- [ ] **Step 5: Commit**

```bash
git add companion/crates/server/src/oauth/transport.rs companion/crates/server/src/oauth/mod.rs
git commit -m "feat: add injectable OAuth transport seam and Google token-response classifier"
```

---

### Task 4: `TokenManager` — exchange, refresh-with-skew, revoke, health

The core. Owns the in-memory access-token cache and the health map; drives `IntegrationStore` through `spawn_blocking`; refreshes within a skew window; classifies failures into `Connected`/`NeedsReconnect`/`Error`.

**Files:**
- Create: `companion/crates/server/src/oauth/token.rs`
- Modify: `companion/crates/server/src/oauth/mod.rs` (add `pub mod token;` and the `GoogleOAuthConfig` used by tests and later tasks)

**Interfaces:**
- Consumes: `crate::secrets::{IntegrationStore, IntegrationSecret}` (sub-project 1), `super::transport::{OAuthTransport, TransportError, classify_token_response, TokenEndpointError}` (Task 3), `super::GoogleOAuthConfig` (defined here), `chrono`.
- Produces:
  - `pub struct GoogleOAuthConfig { pub client_id: String, pub client_secret: String, pub redirect_uri: String, pub scopes: Vec<String>, pub auth_uri: String, pub token_uri: String, pub revoke_uri: String }` with `impl Default` returning Google's real endpoints and the `calendar.events.readonly` scope.
  - `pub enum IntegrationHealth { Connected, NeedsReconnect, Error(String) }` (`Debug`, `Clone`, `PartialEq`, `Eq`)
  - `pub enum TokenError { NeedsReconnect, Transport(String), Provider { status: u16, detail: String }, Malformed(String), NotFound, Store(String), Internal(String) }` (`Debug`, `thiserror::Error`)
  - `pub struct TokenManager` with:
    - `pub fn new(store: Arc<IntegrationStore>, transport: Arc<dyn OAuthTransport>, oauth: GoogleOAuthConfig) -> Self`
    - `pub fn with_clock(store, transport, oauth, now: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>) -> Self`
    - `pub async fn exchange_code(&self, integration_id: &str, code: &str, code_verifier: &str) -> Result<(), TokenError>`
    - `pub async fn access_token(&self, integration_id: &str) -> Result<String, TokenError>`
    - `pub async fn revoke(&self, integration_id: &str) -> Result<(), TokenError>`
    - `pub fn health(&self, integration_id: &str) -> Option<IntegrationHealth>`

- [ ] **Step 1: Write the failing test**

Create `companion/crates/server/src/oauth/token.rs`:

```rust
//! `TokenManager`: OAuth code exchange, skew-window refresh, revoke, and typed
//! integration health (spec §3, §5, §6).
//!
//! Access tokens are cached in memory only. The durable `IntegrationStore` holds
//! just the refresh token and client secret, written at authorize and revoke --
//! never on refresh, because Google refresh tokens do not rotate. Every store
//! call hops through `spawn_blocking`: `IntegrationStore` holds a std Mutex
//! across its seal+write+fsync, which must not run on the async executor.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Utc};

use crate::secrets::{IntegrationSecret, IntegrationStore};
use super::GoogleOAuthConfig;
use super::transport::{OAuthTransport, TokenEndpointError, TransportError, classify_token_response};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::egress::FetchResponse;
    use crate::oauth::transport::OAuthFuture;
    use std::collections::VecDeque;

    struct FakeTransport {
        responses: Mutex<VecDeque<Result<FetchResponse, TransportError>>>,
        calls: Mutex<Vec<(String, Vec<(String, String)>)>>,
    }

    impl FakeTransport {
        fn new(responses: Vec<Result<FetchResponse, TransportError>>) -> Arc<Self> {
            Arc::new(Self {
                responses: Mutex::new(responses.into()),
                calls: Mutex::new(Vec::new()),
            })
        }
        fn calls(&self) -> Vec<(String, Vec<(String, String)>)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl OAuthTransport for FakeTransport {
        fn post_form(
            &self,
            url: String,
            form: Vec<(String, String)>,
        ) -> OAuthFuture<'_, Result<FetchResponse, TransportError>> {
            self.calls.lock().unwrap().push((url, form));
            let response = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("FakeTransport ran out of queued responses");
            Box::pin(async move { response })
        }
    }

    fn ok(body: &str) -> Result<FetchResponse, TransportError> {
        Ok(FetchResponse { status: 200, body: body.as_bytes().to_vec() })
    }
    fn status(code: u16, body: &str) -> Result<FetchResponse, TransportError> {
        Ok(FetchResponse { status: code, body: body.as_bytes().to_vec() })
    }

    fn store() -> Arc<IntegrationStore> {
        let dir = tempfile::tempdir().expect("tempdir");
        // Leak the tempdir so the on-disk secrets file outlives the test body.
        let path = dir.keep().join(crate::secrets::SECRETS_STORE_FILE);
        Arc::new(
            IntegrationStore::open(path, crate::secrets::SecretsKey::from_bytes([9u8; 32]))
                .expect("open store"),
        )
    }

    fn manager(
        store: Arc<IntegrationStore>,
        transport: Arc<dyn OAuthTransport>,
        now: DateTime<Utc>,
    ) -> TokenManager {
        TokenManager::with_clock(
            store,
            transport,
            GoogleOAuthConfig::default(),
            Arc::new(move || now),
        )
    }

    #[tokio::test]
    async fn exchange_persists_refresh_token_and_reports_connected() {
        let store = store();
        let transport =
            FakeTransport::new(vec![ok(r#"{"access_token":"at","expires_in":3600,"refresh_token":"rt"}"#)]);
        let manager = manager(Arc::clone(&store), transport.clone(), Utc::now());

        manager.exchange_code("google-primary", "auth-code", "verifier-abc").await.expect("exchange");

        let stored = tokio::task::spawn_blocking({
            let store = Arc::clone(&store);
            move || store.get("google-primary")
        })
        .await
        .unwrap();
        assert_eq!(stored.expect("stored").refresh_token, "rt");
        assert_eq!(manager.health("google-primary"), Some(IntegrationHealth::Connected));

        let call = &transport.calls()[0];
        assert!(call.1.iter().any(|(k, v)| k == "grant_type" && v == "authorization_code"));
        assert!(call.1.iter().any(|(k, v)| k == "code_verifier" && v == "verifier-abc"));
    }

    #[tokio::test]
    async fn cached_access_token_is_reused_until_near_expiry() {
        let store = store();
        // Only ONE token response is queued: a second network call would panic.
        let transport =
            FakeTransport::new(vec![ok(r#"{"access_token":"at","expires_in":3600,"refresh_token":"rt"}"#)]);
        let now = Utc::now();
        let manager = manager(store, transport, now);
        manager.exchange_code("id", "code", "v").await.expect("exchange");

        // Well before expiry: served from cache, no second transport call.
        assert_eq!(manager.access_token("id").await.expect("cached"), "at");
    }

    #[tokio::test]
    async fn near_expiry_triggers_a_refresh() {
        let store = store();
        let transport = FakeTransport::new(vec![
            ok(r#"{"access_token":"first","expires_in":30,"refresh_token":"rt"}"#),
            ok(r#"{"access_token":"second","expires_in":3600}"#),
        ]);
        let now = Utc::now();
        let manager = manager(store, transport.clone(), now);
        manager.exchange_code("id", "code", "v").await.expect("exchange");

        // expires_in 30s is inside the 60s skew window, so this refreshes.
        assert_eq!(manager.access_token("id").await.expect("refresh"), "second");
        let refresh_call = &transport.calls()[1];
        assert!(refresh_call.1.iter().any(|(k, v)| k == "grant_type" && v == "refresh_token"));
        assert!(refresh_call.1.iter().any(|(k, v)| k == "refresh_token" && v == "rt"));
    }

    #[tokio::test]
    async fn invalid_grant_on_refresh_sets_needs_reconnect() {
        let store = store();
        let transport = FakeTransport::new(vec![
            ok(r#"{"access_token":"first","expires_in":10,"refresh_token":"rt"}"#),
            status(400, r#"{"error":"invalid_grant"}"#),
        ]);
        let manager = manager(store, transport, Utc::now());
        manager.exchange_code("id", "code", "v").await.expect("exchange");

        assert!(matches!(manager.access_token("id").await, Err(TokenError::NeedsReconnect)));
        assert_eq!(manager.health("id"), Some(IntegrationHealth::NeedsReconnect));
    }

    #[tokio::test]
    async fn transport_failure_on_refresh_sets_error_health() {
        let store = store();
        let transport = FakeTransport::new(vec![
            ok(r#"{"access_token":"first","expires_in":10,"refresh_token":"rt"}"#),
            Err(TransportError("connection reset".to_string())),
        ]);
        let manager = manager(store, transport, Utc::now());
        manager.exchange_code("id", "code", "v").await.expect("exchange");

        assert!(matches!(manager.access_token("id").await, Err(TokenError::Transport(_))));
        assert!(matches!(manager.health("id"), Some(IntegrationHealth::Error(_))));
    }

    #[tokio::test]
    async fn revoke_clears_the_stored_secret_and_calls_the_endpoint() {
        let store = store();
        let transport = FakeTransport::new(vec![
            ok(r#"{"access_token":"at","expires_in":3600,"refresh_token":"rt"}"#),
            ok(""), // revoke endpoint 200
        ]);
        let manager = manager(Arc::clone(&store), transport.clone(), Utc::now());
        manager.exchange_code("id", "code", "v").await.expect("exchange");

        manager.revoke("id").await.expect("revoke");

        let stored = tokio::task::spawn_blocking({
            let store = Arc::clone(&store);
            move || store.get("id")
        })
        .await
        .unwrap();
        assert_eq!(stored, None, "revoke must remove the stored secret");
        assert!(transport.calls().iter().any(|(url, form)| {
            url.contains("revoke") && form.iter().any(|(k, v)| k == "token" && v == "rt")
        }));
        assert_eq!(manager.health("id"), None);
    }

    #[test]
    fn token_manager_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<TokenManager>();
    }
}
```

Add to `oauth/mod.rs`:

```rust
pub mod token;
pub use token::{GoogleOAuthConfig, IntegrationHealth, TokenError, TokenManager};
```

Wait — `GoogleOAuthConfig` is defined in `token.rs` but `mod.rs` also declares it in the interface as `super::GoogleOAuthConfig`. Keep the definition in `token.rs` and re-export from `mod.rs` as above; `token.rs`'s `use super::GoogleOAuthConfig;` line must be removed since the type is local. (Resolve in Step 3.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cd companion && cargo test -p server --lib oauth::token`
Expected: FAIL — `cannot find type TokenManager` / `GoogleOAuthConfig` / `IntegrationHealth`.

- [ ] **Step 3: Write the implementation**

Replace the `use super::GoogleOAuthConfig;` line in `token.rs` with nothing (the type is defined here), and add above the test module:

```rust
const REFRESH_SKEW_SECONDS: i64 = 60;

/// Google endpoint + client configuration. Defaults are Google's real URIs and
/// the read-only calendar scope; tests override the URIs with dummies because
/// the fake transport ignores them.
#[derive(Debug, Clone)]
pub struct GoogleOAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    pub auth_uri: String,
    pub token_uri: String,
    pub revoke_uri: String,
}

impl Default for GoogleOAuthConfig {
    fn default() -> Self {
        Self {
            client_id: String::new(),
            client_secret: String::new(),
            redirect_uri: "https://deskmate.rodi.one/v1/integrations/google/callback".to_string(),
            scopes: vec!["https://www.googleapis.com/auth/calendar.events.readonly".to_string()],
            auth_uri: "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
            token_uri: "https://oauth2.googleapis.com/token".to_string(),
            revoke_uri: "https://oauth2.googleapis.com/revoke".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegrationHealth {
    Connected,
    NeedsReconnect,
    Error(String),
}

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    #[error("authorization was revoked or expired; reconnect required")]
    NeedsReconnect,
    #[error("token transport failed: {0}")]
    Transport(String),
    #[error("token endpoint returned {status}: {detail}")]
    Provider { status: u16, detail: String },
    #[error("token endpoint returned an unparseable body: {0}")]
    Malformed(String),
    #[error("no stored credentials for that integration")]
    NotFound,
    #[error("secrets store error: {0}")]
    Store(String),
    #[error("internal error: {0}")]
    Internal(String),
}

struct CachedToken {
    access_token: String,
    expires_at: DateTime<Utc>,
}

pub struct TokenManager {
    store: Arc<IntegrationStore>,
    transport: Arc<dyn OAuthTransport>,
    oauth: GoogleOAuthConfig,
    now: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    cache: Mutex<HashMap<String, CachedToken>>,
    health: Mutex<HashMap<String, IntegrationHealth>>,
}

impl TokenManager {
    pub fn new(
        store: Arc<IntegrationStore>,
        transport: Arc<dyn OAuthTransport>,
        oauth: GoogleOAuthConfig,
    ) -> Self {
        Self::with_clock(store, transport, oauth, Arc::new(Utc::now))
    }

    pub fn with_clock(
        store: Arc<IntegrationStore>,
        transport: Arc<dyn OAuthTransport>,
        oauth: GoogleOAuthConfig,
        now: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    ) -> Self {
        Self {
            store,
            transport,
            oauth,
            now,
            cache: Mutex::new(HashMap::new()),
            health: Mutex::new(HashMap::new()),
        }
    }

    pub async fn exchange_code(
        &self,
        integration_id: &str,
        code: &str,
        code_verifier: &str,
    ) -> Result<(), TokenError> {
        let form = vec![
            ("grant_type".to_string(), "authorization_code".to_string()),
            ("code".to_string(), code.to_string()),
            ("redirect_uri".to_string(), self.oauth.redirect_uri.clone()),
            ("client_id".to_string(), self.oauth.client_id.clone()),
            ("client_secret".to_string(), self.oauth.client_secret.clone()),
            ("code_verifier".to_string(), code_verifier.to_string()),
        ];
        let tokens = self.post_tokens(integration_id, &self.oauth.token_uri.clone(), form).await?;
        let refresh_token = tokens.refresh_token.ok_or_else(|| {
            let detail = "authorization response contained no refresh_token".to_string();
            self.set_health(integration_id, IntegrationHealth::Error(detail.clone()));
            TokenError::Provider { status: 200, detail }
        })?;

        let secret = IntegrationSecret {
            provider: "google".to_string(),
            refresh_token,
            client_secret: Some(self.oauth.client_secret.clone()),
            scopes: self.oauth.scopes.clone(),
            obtained_at: (self.now)().timestamp(),
        };
        self.store_put(integration_id.to_string(), secret).await?;
        self.cache_token(integration_id, tokens.access_token, tokens.expires_in);
        self.set_health(integration_id, IntegrationHealth::Connected);
        Ok(())
    }

    pub async fn access_token(&self, integration_id: &str) -> Result<String, TokenError> {
        if let Some(token) = self.cached_valid(integration_id) {
            return Ok(token);
        }
        let secret = self
            .store_get(integration_id.to_string())
            .await?
            .ok_or(TokenError::NotFound)?;
        let client_secret = secret
            .client_secret
            .unwrap_or_else(|| self.oauth.client_secret.clone());
        let form = vec![
            ("grant_type".to_string(), "refresh_token".to_string()),
            ("refresh_token".to_string(), secret.refresh_token),
            ("client_id".to_string(), self.oauth.client_id.clone()),
            ("client_secret".to_string(), client_secret),
        ];
        let tokens = self.post_tokens(integration_id, &self.oauth.token_uri.clone(), form).await?;
        // A refresh does NOT rotate the refresh token, so the store is untouched.
        self.cache_token(integration_id, tokens.access_token.clone(), tokens.expires_in);
        self.set_health(integration_id, IntegrationHealth::Connected);
        Ok(tokens.access_token)
    }

    pub async fn revoke(&self, integration_id: &str) -> Result<(), TokenError> {
        let secret = self.store_get(integration_id.to_string()).await?;
        if let Some(secret) = &secret {
            // Best-effort remote revoke; local removal proceeds regardless, because
            // the operator asked to disconnect.
            let form = vec![("token".to_string(), secret.refresh_token.clone())];
            let _ = self
                .transport
                .post_form(self.oauth.revoke_uri.clone(), form)
                .await;
        }
        let existed = self.store_remove(integration_id.to_string()).await?;
        self.cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(integration_id);
        self.health.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(integration_id);
        if existed {
            Ok(())
        } else {
            Err(TokenError::NotFound)
        }
    }

    pub fn health(&self, integration_id: &str) -> Option<IntegrationHealth> {
        self.health
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(integration_id)
            .cloned()
    }

    /// Sends a token/refresh request and maps a [`TokenEndpointError`] to a
    /// [`TokenError`] plus the matching health transition.
    async fn post_tokens(
        &self,
        integration_id: &str,
        url: &str,
        form: Vec<(String, String)>,
    ) -> Result<super::transport::GoogleTokenResponse, TokenError> {
        let response = self
            .transport
            .post_form(url.to_string(), form)
            .await
            .map_err(|error| {
                self.set_health(integration_id, IntegrationHealth::Error(error.0.clone()));
                TokenError::Transport(error.0)
            })?;
        classify_token_response(response.status, &response.body).map_err(|error| match error {
            TokenEndpointError::InvalidGrant => {
                self.set_health(integration_id, IntegrationHealth::NeedsReconnect);
                TokenError::NeedsReconnect
            }
            TokenEndpointError::Provider { status, detail } => {
                self.set_health(integration_id, IntegrationHealth::Error(detail.clone()));
                TokenError::Provider { status, detail }
            }
            TokenEndpointError::Malformed(detail) => {
                self.set_health(integration_id, IntegrationHealth::Error(detail.clone()));
                TokenError::Malformed(detail)
            }
        })
    }

    fn cached_valid(&self, integration_id: &str) -> Option<String> {
        let now = (self.now)();
        let cache = self.cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = cache.get(integration_id)?;
        if now + Duration::seconds(REFRESH_SKEW_SECONDS) < entry.expires_at {
            Some(entry.access_token.clone())
        } else {
            None
        }
    }

    fn cache_token(&self, integration_id: &str, access_token: String, expires_in: u64) {
        let expires_at = (self.now)() + Duration::seconds(expires_in as i64);
        self.cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(integration_id.to_string(), CachedToken { access_token, expires_at });
    }

    fn set_health(&self, integration_id: &str, health: IntegrationHealth) {
        self.health
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(integration_id.to_string(), health);
    }

    // --- IntegrationStore hops: the store holds a std Mutex across seal+write+
    // fsync, so every call runs on the blocking pool, never the async executor.

    async fn store_get(&self, integration_id: String) -> Result<Option<IntegrationSecret>, TokenError> {
        let store = Arc::clone(&self.store);
        tokio::task::spawn_blocking(move || store.get(&integration_id))
            .await
            .map_err(|error| TokenError::Internal(error.to_string()))
    }

    async fn store_put(&self, integration_id: String, secret: IntegrationSecret) -> Result<(), TokenError> {
        let store = Arc::clone(&self.store);
        tokio::task::spawn_blocking(move || store.put(integration_id, secret))
            .await
            .map_err(|error| TokenError::Internal(error.to_string()))?
            .map_err(|error| TokenError::Store(error.to_string()))
    }

    async fn store_remove(&self, integration_id: String) -> Result<bool, TokenError> {
        let store = Arc::clone(&self.store);
        tokio::task::spawn_blocking(move || store.remove(&integration_id))
            .await
            .map_err(|error| TokenError::Internal(error.to_string()))?
            .map_err(|error| TokenError::Store(error.to_string()))
    }
}
```

Note the test's `store()` helper needs `SecretsKey` and `SECRETS_STORE_FILE` from `crate::secrets`; both are already `pub` from sub-project 1. `dir.keep()` is `tempfile::TempDir::keep` (persists the directory); if the pinned `tempfile` predates `keep`, use `into_path()` instead — check `tempfile`'s version in `Cargo.lock` and use whichever the pinned version exposes.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd companion && cargo test -p server --lib oauth::token`
Expected: PASS — all seven tests (exchange/persist/Connected, cache-reuse, refresh-on-skew, invalid_grant→NeedsReconnect, transport→Error, revoke-clears, Send+Sync).

- [ ] **Step 5: Commit**

```bash
git add companion/crates/server/src/oauth/token.rs companion/crates/server/src/oauth/mod.rs
git commit -m "feat: add TokenManager with skew-window refresh, revoke, and typed health"
```

---

### Task 5: Operator session cookie, `OperatorAuthenticated` gate, login route

An HMAC-SHA256 cookie signed with the existing admin token, an extractor that accepts either the admin bearer token or a valid cookie, and the login route that mints the cookie.

**Files:**
- Create: `companion/crates/server/src/oauth/session.rs`
- Modify: `companion/crates/server/src/oauth/mod.rs` (add `pub mod session;`)
- Modify: `companion/crates/server/Cargo.toml` (add `hmac`)

**Interfaces:**
- Consumes: `hmac`, `sha2`, `base64` (URL-safe), `crate::registry::constant_time_eq` (existing `pub(crate)`), `crate::auth::bearer_token` (existing `pub(crate)`), `crate::ServerState` (Task 6 adds `integrations()`), `chrono`.
- Produces:
  - `pub const SESSION_COOKIE: &str = "deskmate_session";`
  - `pub struct SessionSigner { key: Vec<u8> }` with `pub fn from_admin_token(token: &str) -> Self`, `pub fn mint(&self, sid: &str, now: DateTime<Utc>, ttl: Duration) -> String`, `pub fn verify(&self, value: &str, now: DateTime<Utc>) -> Option<SessionClaims>`, `pub fn new_sid() -> String`.
  - `pub struct SessionClaims { pub sid: String, pub exp: i64 }`
  - `pub struct OperatorAuthenticated { pub sid: String }` implementing `FromRequestParts<ServerState>` (bearer-admin OR cookie ⇒ Ok; otherwise bare 401).

- [ ] **Step 1: Add the dependency**

In `companion/crates/server/Cargo.toml`, under `[dependencies]` near `sha2`:

```toml
# HMAC-SHA256 for signing the short-lived operator session cookie (spec §1.2, §6),
# keyed by DESKMATE_ADMIN_TOKEN. Pairs with the existing sha2 0.10; pinned exactly.
hmac = "=0.12.1"
```

- [ ] **Step 2: Write the failing test**

Create `companion/crates/server/src/oauth/session.rs`:

```rust
//! Operator session cookie: an HMAC-SHA256 token over `sid:exp`, keyed by the
//! existing DESKMATE_ADMIN_TOKEN (spec §1.2, §6). The stand-in for V3-deferred
//! accounts; it carries no privilege the admin token does not already carry.

use base64::prelude::{BASE64_URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Duration, Utc};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::registry::constant_time_eq;

type HmacSha256 = Hmac<Sha256>;

pub const SESSION_COOKIE: &str = "deskmate_session";

pub struct SessionSigner {
    key: Vec<u8>,
}

pub struct SessionClaims {
    pub sid: String,
    pub exp: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signer() -> SessionSigner {
        SessionSigner::from_admin_token("admin-token-value")
    }

    #[test]
    fn mint_then_verify_round_trips() {
        let now = Utc::now();
        let cookie = signer().mint("sid-1", now, Duration::hours(12));
        let claims = signer().verify(&cookie, now).expect("valid cookie");
        assert_eq!(claims.sid, "sid-1");
    }

    #[test]
    fn an_expired_cookie_is_rejected() {
        let signed_at = Utc::now();
        let cookie = signer().mint("sid-1", signed_at, Duration::seconds(1));
        let later = signed_at + Duration::seconds(2);
        assert!(signer().verify(&cookie, later).is_none());
    }

    #[test]
    fn a_tampered_cookie_is_rejected() {
        let now = Utc::now();
        let cookie = signer().mint("sid-1", now, Duration::hours(1));
        let mut tampered = cookie.clone();
        tampered.push('x');
        assert!(signer().verify(&tampered, now).is_none());
    }

    #[test]
    fn a_cookie_from_a_different_key_is_rejected() {
        let now = Utc::now();
        let cookie = signer().mint("sid-1", now, Duration::hours(1));
        let other = SessionSigner::from_admin_token("a-different-admin-token");
        assert!(other.verify(&cookie, now).is_none());
    }

    #[test]
    fn new_sid_is_unique() {
        assert_ne!(SessionSigner::new_sid(), SessionSigner::new_sid());
    }
}
```

Add `pub mod session;` to `oauth/mod.rs`.

- [ ] **Step 3: Run test to verify it fails**

Run: `cd companion && cargo test -p server --lib oauth::session`
Expected: FAIL — `cannot find type SessionSigner`.

- [ ] **Step 4: Write the implementation**

Add to `session.rs` (above the test module):

```rust
use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use rand::RngCore;

use crate::ServerState;
use crate::auth::bearer_token;

impl SessionSigner {
    #[must_use]
    pub fn from_admin_token(token: &str) -> Self {
        Self { key: token.as_bytes().to_vec() }
    }

    /// Signs `sid:exp`, where `exp` is `now + ttl` in unix seconds. Value shape:
    /// `base64url(payload).base64url(hmac)`.
    #[must_use]
    pub fn mint(&self, sid: &str, now: DateTime<Utc>, ttl: Duration) -> String {
        let exp = (now + ttl).timestamp();
        let payload = format!("{sid}:{exp}");
        let tag = self.tag(payload.as_bytes());
        format!(
            "{}.{}",
            BASE64_URL_SAFE_NO_PAD.encode(payload.as_bytes()),
            BASE64_URL_SAFE_NO_PAD.encode(tag)
        )
    }

    /// Verifies the signature (constant-time) and the expiry, returning the claims.
    #[must_use]
    pub fn verify(&self, value: &str, now: DateTime<Utc>) -> Option<SessionClaims> {
        let (payload_b64, tag_b64) = value.split_once('.')?;
        let payload = BASE64_URL_SAFE_NO_PAD.decode(payload_b64).ok()?;
        let presented_tag = BASE64_URL_SAFE_NO_PAD.decode(tag_b64).ok()?;
        let expected_tag = self.tag(&payload);
        if !constant_time_eq(&expected_tag, &presented_tag) {
            return None;
        }
        let payload = String::from_utf8(payload).ok()?;
        let (sid, exp) = payload.rsplit_once(':')?;
        let exp: i64 = exp.parse().ok()?;
        if now.timestamp() >= exp {
            return None;
        }
        Some(SessionClaims { sid: sid.to_string(), exp })
    }

    /// A random session id embedded in the cookie so a `state` stash can be bound
    /// to the operator session that started it (spec §3).
    #[must_use]
    pub fn new_sid() -> String {
        let mut bytes = [0u8; 16];
        rand::rng().fill_bytes(&mut bytes);
        BASE64_URL_SAFE_NO_PAD.encode(bytes)
    }

    fn tag(&self, payload: &[u8]) -> Vec<u8> {
        let mut mac = HmacSha256::new_from_slice(&self.key).expect("HMAC accepts any key length");
        mac.update(payload);
        mac.finalize().into_bytes().to_vec()
    }
}

/// Reads the session cookie value from the `Cookie` header, if present.
fn session_cookie_value(parts: &Parts) -> Option<String> {
    let header = parts.headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    header.split(';').find_map(|pair| {
        let (name, value) = pair.trim().split_once('=')?;
        (name == SESSION_COOKIE).then(|| value.to_string())
    })
}

/// Bare 401, matching `auth.rs`: no body, no logged credential.
#[derive(Debug)]
pub struct OperatorAuthError;

impl IntoResponse for OperatorAuthError {
    fn into_response(self) -> Response {
        StatusCode::UNAUTHORIZED.into_response()
    }
}

/// An operator request: authenticated either by the admin bearer token (the
/// `sid` is then the fixed `"bearer-admin"`) or by a valid session cookie.
pub struct OperatorAuthenticated {
    pub sid: String,
}

impl FromRequestParts<ServerState> for OperatorAuthenticated {
    type Rejection = OperatorAuthError;

    #[allow(clippy::unused_async_trait_impl)]
    async fn from_request_parts(
        parts: &mut Parts,
        state: &ServerState,
    ) -> Result<Self, Self::Rejection> {
        if let Some(token) = bearer_token(parts) {
            if state.verify_admin_token(token) {
                return Ok(Self { sid: "bearer-admin".to_string() });
            }
        }
        if let Some(runtime) = state.integrations() {
            if let Some(value) = session_cookie_value(parts) {
                if let Some(claims) = runtime.sessions().verify(&value, Utc::now()) {
                    return Ok(Self { sid: claims.sid });
                }
            }
        }
        Err(OperatorAuthError)
    }
}

/// Standard cookie attributes for the session cookie: `HttpOnly` (JS cannot read
/// it), `Secure` (HTTPS only -- the server is always behind the tunnel), and
/// `SameSite=Lax` (so the top-level OAuth callback redirect still carries it).
#[must_use]
pub fn set_cookie_header(value: &str, ttl: Duration) -> String {
    format!(
        "{SESSION_COOKIE}={value}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age={}",
        ttl.num_seconds()
    )
}

// `Arc` is used by callers constructing a shared signer; keep the import live.
#[allow(unused_imports)]
use Arc as _EnsureArcImport;
```

Remove the `use std::sync::Arc;` / `_EnsureArcImport` shim if `Arc` ends up unused after Task 6 wiring; it is listed here only so this file compiles standalone. (Delete both lines once Task 6 confirms `Arc` is not needed in `session.rs`.)

This references `state.integrations()` and `runtime.sessions()`, added in Task 6; until then `session.rs` will not compile against `ServerState`. Sequence Task 6 immediately after and run the combined test suite there. To keep Task 5 independently green, gate the extractor test behind Task 6: the five `SessionSigner` unit tests above do **not** touch `ServerState` and pass now; the extractor is exercised by Task 6's route tests.

- [ ] **Step 5: Run the signer tests to verify they pass**

Run: `cd companion && cargo test -p server --lib oauth::session::tests`
Expected: PASS — the five `SessionSigner` tests. (The extractor's `ServerState` calls compile only after Task 6; if the crate does not yet build because of `state.integrations()`, proceed to Task 6 and run the suite there. Do not commit a non-building crate — if blocked, fold this commit into Task 6.)

- [ ] **Step 6: Commit**

```bash
git add companion/crates/server/Cargo.toml companion/crates/server/Cargo.lock companion/crates/server/src/oauth/session.rs companion/crates/server/src/oauth/mod.rs
git commit -m "feat: add HMAC operator session cookie and OperatorAuthenticated gate"
```

---

### Task 6: OAuth consent/callback/revoke routes, `IntegrationRuntime`, ServerState wiring

The three routes plus the login route, the `state`/PKCE stash bound to the session, and the `ServerState` seam (`OnceLock`) that holds the runtime without disturbing existing constructors.

**Files:**
- Create: `companion/crates/server/src/oauth/routes.rs`
- Modify: `companion/crates/server/src/oauth/mod.rs` (`pub mod routes;`, `IntegrationRuntime`, re-exports)
- Modify: `companion/crates/server/src/lib.rs` (add `integrations: OnceLock<Arc<oauth::IntegrationRuntime>>` to `StateInner`; `ServerState::{integrations, set_integrations}`; merge `oauth::routes::routes()` in `app()`)

**Interfaces:**
- Consumes: `TokenManager`, `GoogleOAuthConfig` (Task 4); `SessionSigner`, `OperatorAuthenticated`, `set_cookie_header`, `SESSION_COOKIE` (Task 5); `pkce::{generate_pkce, generate_state}` (Task 2); `crate::auth::bearer_token`; `axum`; `url::Url`; `chrono`.
- Produces:
  - `pub struct IntegrationRuntime` with `pub fn new(token_manager: Arc<TokenManager>, sessions: SessionSigner, oauth: GoogleOAuthConfig) -> Self`, `pub fn token_manager(&self) -> &Arc<TokenManager>`, `pub fn sessions(&self) -> &SessionSigner`, `pub fn start_consent(&self, sid: &str, integration_id: &str) -> String` (stashes `PendingAuth`, returns the Google auth URL), `pub fn take_pending(&self, state: &str, now: DateTime<Utc>) -> Option<PendingAuth>`.
  - `pub struct PendingAuth { pub integration_id: String, pub code_verifier: String, pub sid: String }`
  - `pub(crate) fn routes() -> axum::Router<ServerState>` — `POST /v1/session/login`, `POST /v1/integrations/google`, `GET /v1/integrations/google/callback`, `POST /v1/integrations/{id}/revoke`.
  - `ServerState::integrations(&self) -> Option<Arc<oauth::IntegrationRuntime>>`, `ServerState::set_integrations(&self, runtime: Arc<oauth::IntegrationRuntime>)`.

- [ ] **Step 1: Write the failing test**

Create `companion/crates/server/src/oauth/routes.rs`:

```rust
//! OAuth consent, callback, and revoke routes plus the operator login route
//! (spec §3). Route handlers stay thin; the `state`/PKCE stash and the auth-URL
//! construction live on `IntegrationRuntime` so they are unit-testable.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use url::Url;

use crate::ServerState;
use crate::auth::bearer_token;
use super::session::{OperatorAuthenticated, SessionSigner, set_cookie_header};
use super::token::{GoogleOAuthConfig, TokenError, TokenManager};

const PENDING_TTL: Duration = Duration::seconds(600);
const SESSION_TTL: Duration = Duration::hours(12);
const DEFAULT_INTEGRATION_ID: &str = "google-primary";

pub struct PendingAuth {
    pub integration_id: String,
    pub code_verifier: String,
    pub sid: String,
}

struct StashedAuth {
    pending: PendingAuth,
    created_at: DateTime<Utc>,
}

pub struct IntegrationRuntime {
    token_manager: Arc<TokenManager>,
    sessions: SessionSigner,
    oauth: GoogleOAuthConfig,
    pending: Mutex<HashMap<String, StashedAuth>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::transport::{OAuthFuture, OAuthTransport, TransportError};
    use crate::egress::FetchResponse;

    struct NoTransport;
    impl OAuthTransport for NoTransport {
        fn post_form(
            &self,
            _url: String,
            _form: Vec<(String, String)>,
        ) -> OAuthFuture<'_, Result<FetchResponse, TransportError>> {
            Box::pin(async { Err(TransportError("no transport in this test".to_string())) })
        }
    }

    fn runtime() -> IntegrationRuntime {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.keep().join(crate::secrets::SECRETS_STORE_FILE);
        let store = Arc::new(
            crate::secrets::IntegrationStore::open(
                path,
                crate::secrets::SecretsKey::from_bytes([3u8; 32]),
            )
            .expect("store"),
        );
        let token_manager =
            Arc::new(TokenManager::new(store, Arc::new(NoTransport), GoogleOAuthConfig::default()));
        IntegrationRuntime::new(
            token_manager,
            SessionSigner::from_admin_token("admin"),
            GoogleOAuthConfig::default(),
        )
    }

    #[test]
    fn start_consent_returns_a_google_url_carrying_state_and_challenge() {
        let runtime = runtime();
        let url = runtime.start_consent("sid-1", "google-primary");
        let parsed = Url::parse(&url).expect("valid url");
        assert_eq!(parsed.host_str(), Some("accounts.google.com"));
        let query: HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(query.get("code_challenge_method").map(String::as_str), Some("S256"));
        assert!(query.contains_key("code_challenge"));
        assert_eq!(query.get("access_type").map(String::as_str), Some("offline"));
        assert!(query.contains_key("state"));
    }

    #[test]
    fn a_stashed_state_is_single_use() {
        let runtime = runtime();
        let url = runtime.start_consent("sid-1", "google-primary");
        let state = Url::parse(&url).unwrap().query_pairs().into_owned()
            .find(|(k, _)| k == "state").unwrap().1;

        let first = runtime.take_pending(&state, Utc::now());
        assert!(first.is_some(), "first take must succeed");
        assert!(runtime.take_pending(&state, Utc::now()).is_none(), "state must be single-use");
    }

    #[test]
    fn an_unknown_state_returns_none() {
        assert!(runtime().take_pending("never-issued", Utc::now()).is_none());
    }

    #[test]
    fn an_expired_stash_returns_none() {
        let runtime = runtime();
        let url = runtime.start_consent("sid-1", "google-primary");
        let state = Url::parse(&url).unwrap().query_pairs().into_owned()
            .find(|(k, _)| k == "state").unwrap().1;
        let much_later = Utc::now() + Duration::seconds(601);
        assert!(runtime.take_pending(&state, much_later).is_none());
    }
}
```

Add to `oauth/mod.rs`:

```rust
pub mod routes;
pub use routes::{IntegrationRuntime, PendingAuth};
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd companion && cargo test -p server --lib oauth::routes`
Expected: FAIL — `IntegrationRuntime`/`start_consent`/`take_pending` undefined, and `ServerState::integrations` (used by Task 5's extractor) missing.

- [ ] **Step 3: Write the implementation**

Add to `routes.rs` (above the test module):

```rust
impl IntegrationRuntime {
    #[must_use]
    pub fn new(
        token_manager: Arc<TokenManager>,
        sessions: SessionSigner,
        oauth: GoogleOAuthConfig,
    ) -> Self {
        Self { token_manager, sessions, oauth, pending: Mutex::new(HashMap::new()) }
    }

    #[must_use]
    pub fn token_manager(&self) -> &Arc<TokenManager> {
        &self.token_manager
    }

    #[must_use]
    pub fn sessions(&self) -> &SessionSigner {
        &self.sessions
    }

    /// Generates `state` + PKCE, stashes them bound to `sid`, and returns the
    /// Google authorization URL to redirect the operator to.
    pub fn start_consent(&self, sid: &str, integration_id: &str) -> String {
        let state = super::pkce::generate_state();
        let pkce = super::pkce::generate_pkce();

        self.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(
            state.clone(),
            StashedAuth {
                pending: PendingAuth {
                    integration_id: integration_id.to_string(),
                    code_verifier: pkce.verifier,
                    sid: sid.to_string(),
                },
                created_at: Utc::now(),
            },
        );

        let mut url = Url::parse(&self.oauth.auth_uri).expect("auth_uri is a valid URL");
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.oauth.client_id)
            .append_pair("redirect_uri", &self.oauth.redirect_uri)
            .append_pair("scope", &self.oauth.scopes.join(" "))
            .append_pair("access_type", "offline")
            .append_pair("prompt", "consent")
            .append_pair("code_challenge", &pkce.challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state);
        url.into()
    }

    /// Removes and returns the stash for `state` if it exists and is unexpired.
    /// Single-use: a second call for the same `state` returns `None`.
    pub fn take_pending(&self, state: &str, now: DateTime<Utc>) -> Option<PendingAuth> {
        let mut pending = self.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let stashed = pending.remove(state)?;
        if now - stashed.created_at > PENDING_TTL {
            return None;
        }
        Some(stashed.pending)
    }
}

#[derive(Debug)]
enum RouteError {
    Unauthorized,
    BadRequest(String),
    Upstream(String),
    NotConfigured,
}

impl IntoResponse for RouteError {
    fn into_response(self) -> Response {
        match self {
            RouteError::Unauthorized => StatusCode::UNAUTHORIZED.into_response(),
            RouteError::BadRequest(message) => (StatusCode::BAD_REQUEST, message).into_response(),
            RouteError::Upstream(message) => (StatusCode::BAD_GATEWAY, message).into_response(),
            RouteError::NotConfigured => (
                StatusCode::SERVICE_UNAVAILABLE,
                "OAuth integrations are not configured on this server",
            )
                .into_response(),
        }
    }
}

fn token_error_to_route(error: TokenError) -> RouteError {
    match error {
        TokenError::NeedsReconnect => {
            RouteError::BadRequest("authorization was revoked or expired; reconnect".to_string())
        }
        TokenError::NotFound => RouteError::BadRequest("no such integration".to_string()),
        other => RouteError::Upstream(other.to_string()),
    }
}

pub(crate) fn routes() -> Router<ServerState> {
    Router::new()
        .route("/v1/session/login", post(login))
        .route("/v1/integrations/google", post(start_google))
        .route("/v1/integrations/google/callback", get(google_callback))
        .route("/v1/integrations/{id}/revoke", post(revoke_integration))
}

/// Exchanges the admin bearer token for a short-lived session cookie.
async fn login(State(state): State<ServerState>, parts: axum::http::request::Parts) -> Response {
    let presented = match bearer_token(&parts) {
        Some(token) if state.verify_admin_token(token) => token,
        _ => return StatusCode::UNAUTHORIZED.into_response(),
    };
    let _ = presented;
    let Some(runtime) = state.integrations() else {
        return RouteError::NotConfigured.into_response();
    };
    let sid = SessionSigner::new_sid();
    let cookie = runtime.sessions().mint(&sid, Utc::now(), SESSION_TTL);
    let header_value = match HeaderValue::from_str(&set_cookie_header(&cookie, SESSION_TTL)) {
        Ok(value) => value,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(header::SET_COOKIE, header_value);
    response
}

#[derive(Deserialize)]
struct StartQuery {
    integration_id: Option<String>,
}

/// Starts consent: builds the Google authorization URL and redirects the operator.
async fn start_google(
    State(state): State<ServerState>,
    operator: OperatorAuthenticated,
    Query(query): Query<StartQuery>,
) -> Result<Redirect, RouteError> {
    let runtime = state.integrations().ok_or(RouteError::NotConfigured)?;
    let integration_id = query.integration_id.unwrap_or_else(|| DEFAULT_INTEGRATION_ID.to_string());
    let url = runtime.start_consent(&operator.sid, &integration_id);
    Ok(Redirect::to(&url))
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// Google's redirect back: verifies `state` (CSRF, single-use, TTL, session-bound)
/// then exchanges the code. Redirects to the management page on success.
async fn google_callback(
    State(state): State<ServerState>,
    operator: OperatorAuthenticated,
    Query(query): Query<CallbackQuery>,
) -> Result<Redirect, RouteError> {
    let runtime = state.integrations().ok_or(RouteError::NotConfigured)?;
    if let Some(error) = query.error {
        return Err(RouteError::BadRequest(format!("authorization was declined: {error}")));
    }
    let code = query.code.ok_or_else(|| RouteError::BadRequest("missing code".to_string()))?;
    let state_value = query.state.ok_or_else(|| RouteError::BadRequest("missing state".to_string()))?;

    let pending = runtime
        .take_pending(&state_value, Utc::now())
        .ok_or_else(|| RouteError::BadRequest("unknown or expired state".to_string()))?;
    if pending.sid != operator.sid {
        return Err(RouteError::Unauthorized);
    }

    runtime
        .token_manager()
        .exchange_code(&pending.integration_id, &code, &pending.code_verifier)
        .await
        .map_err(token_error_to_route)?;
    Ok(Redirect::to("/v1/manage"))
}

/// Revokes an integration and clears its stored secret.
async fn revoke_integration(
    State(state): State<ServerState>,
    _operator: OperatorAuthenticated,
    Path(id): Path<String>,
) -> Result<StatusCode, RouteError> {
    let runtime = state.integrations().ok_or(RouteError::NotConfigured)?;
    runtime.token_manager().revoke(&id).await.map_err(token_error_to_route)?;
    Ok(StatusCode::NO_CONTENT)
}
```

Now wire `ServerState`. In `companion/crates/server/src/lib.rs`, add to the `use` block:

```rust
use std::sync::OnceLock;
```

Add the field to `StateInner` (it defaults empty, so no constructor needs to change):

```rust
    /// The OAuth integration runtime, attached at startup by `set_integrations`
    /// when integrations are configured. `OnceLock` so existing constructors are
    /// untouched and a deployment without integrations simply never sets it.
    integrations: OnceLock<Arc<oauth::IntegrationRuntime>>,
```

Every `StateInner { ... }` literal in `lib.rs` must add `integrations: OnceLock::new(),`. (There is one, inside `with_config_temp_dir`.) Add the accessor methods to `impl ServerState`:

```rust
    /// The OAuth integration runtime, if one was attached at startup.
    pub fn integrations(&self) -> Option<Arc<oauth::IntegrationRuntime>> {
        self.inner.integrations.get().map(Arc::clone)
    }

    /// Attaches the OAuth integration runtime once, at startup. Subsequent calls
    /// are ignored (the first attachment wins).
    pub fn set_integrations(&self, runtime: Arc<oauth::IntegrationRuntime>) {
        let _ = self.inner.integrations.set(runtime);
    }
```

Merge the OAuth routes in `app()`, beside `.merge(admin::routes())`:

```rust
        .merge(admin::routes())
        .merge(oauth::routes::routes())
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd companion && cargo test -p server --lib oauth::`
Expected: PASS — the four `routes` unit tests plus every earlier `oauth` module test. The crate now builds fully (Task 5's extractor resolves `state.integrations()` / `runtime.sessions()`).

- [ ] **Step 5: Write the route integration test (controller-run)**

Create `companion/crates/server/tests/oauth_routes.rs`:

```rust
//! Route-level checks that need a full `ServerState` and Axum. These bind
//! loopback and therefore CANNOT run under Codex's sandbox (project memory:
//! sandbox denies loopback binds) -- the controller runs them.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use server::oauth::{GoogleOAuthConfig, IntegrationRuntime, TokenManager};
use server::oauth::session::SessionSigner;
use server::{ServerState, app};
use tower::ServiceExt;

fn state_with_integrations() -> ServerState {
    let state = ServerState::in_memory();
    // Reuse the server's own in-memory config dir for the secrets file.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.keep().join("secrets.enc");
    let store = Arc::new(
        server::secrets::IntegrationStore::open(
            path,
            server::secrets::SecretsKey::from_bytes([1u8; 32]),
        )
        .expect("store"),
    );
    let token_manager = Arc::new(TokenManager::new(
        store,
        Arc::new(server::oauth::transport::EgressTransport),
        GoogleOAuthConfig::default(),
    ));
    let runtime = Arc::new(IntegrationRuntime::new(
        token_manager,
        SessionSigner::from_admin_token("in-memory-admin-token"),
        GoogleOAuthConfig::default(),
    ));
    state.set_integrations(runtime);
    state
}

#[tokio::test]
async fn callback_with_unknown_state_is_rejected_before_any_exchange() {
    let app = app(state_with_integrations());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/integrations/google/callback?code=abc&state=never-issued")
                .header(header::AUTHORIZATION, "Bearer in-memory-admin-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // No stash for that state -> 400, and crucially no token exchange was attempted.
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn oauth_routes_reject_an_unauthenticated_caller_with_a_bare_401() {
    let app = app(state_with_integrations());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/integrations/google")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn start_consent_redirects_to_google_for_an_admin_caller() {
    let app = app(state_with_integrations());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/integrations/google")
                .header(header::AUTHORIZATION, "Bearer in-memory-admin-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response.headers().get(header::LOCATION).unwrap().to_str().unwrap();
    assert!(location.starts_with("https://accounts.google.com/o/oauth2/v2/auth"));
}
```

This test uses `server::secrets` and `server::oauth::{transport, session}` as public paths — confirm `lib.rs` has `pub mod secrets;` (sub-project 1) and that `oauth/mod.rs` declares `pub mod transport;`, `pub mod session;` (it does). Add `tower = { ... , features = [..., "util"] }` is already present; `ServiceExt::oneshot` needs the `util` feature, which `server/Cargo.toml` already enables.

- [ ] **Step 6: Run the route tests (controller)**

Run: `cd companion && cargo test -p server --test oauth_routes`
Expected: PASS — unknown-state rejected with 400, unauthenticated 401, admin start 303 to Google. (Controller runs this; it binds loopback via `oneshot`, which the sandbox forbids.)

- [ ] **Step 7: Commit**

```bash
git add companion/crates/server/src/oauth/routes.rs companion/crates/server/src/oauth/mod.rs companion/crates/server/src/lib.rs companion/crates/server/tests/oauth_routes.rs
git commit -m "feat: add OAuth consent/callback/revoke routes and integration runtime wiring"
```

---

### Task 7: Production wiring of the integration runtime in `main.rs`

Build the runtime from environment configuration when Google OAuth is configured, and attach it. Absent config leaves integrations off and the server starts exactly as before.

**Files:**
- Modify: `companion/crates/server/src/main.rs`

**Interfaces:**
- Consumes: `server::oauth::{GoogleOAuthConfig, IntegrationRuntime, TokenManager}`, `server::oauth::session::SessionSigner`, `server::oauth::transport::EgressTransport`, `server::secrets::open_integration_store` (sub-project 1), `ServerState::set_integrations` (Task 6).
- Produces: `fn google_oauth_config_from_env() -> Option<GoogleOAuthConfig>` and the attach sequence in `main`.

- [ ] **Step 1: Write the failing test**

Add to `main.rs`'s `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn google_config_is_none_without_client_id() {
        // No DESKMATE_GOOGLE_CLIENT_ID set in this unit's environment.
        assert!(super::google_oauth_config_from_env().is_none());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd companion && cargo test -p server --bin server google_config_is_none`
Expected: FAIL — `cannot find function google_oauth_config_from_env`.

- [ ] **Step 3: Write the implementation**

Add to `main.rs`:

```rust
const ENV_GOOGLE_CLIENT_ID: &str = "DESKMATE_GOOGLE_CLIENT_ID";
const ENV_GOOGLE_CLIENT_SECRET: &str = "DESKMATE_GOOGLE_CLIENT_SECRET";
const ENV_GOOGLE_REDIRECT_URI: &str = "DESKMATE_GOOGLE_REDIRECT_URI";

/// Builds a `GoogleOAuthConfig` from the environment, or `None` if the client id
/// is unset (integrations then stay off and the server runs exactly as before).
fn google_oauth_config_from_env() -> Option<server::oauth::GoogleOAuthConfig> {
    let client_id = std::env::var(ENV_GOOGLE_CLIENT_ID).ok().filter(|value| !value.is_empty())?;
    let mut config = server::oauth::GoogleOAuthConfig {
        client_id,
        ..server::oauth::GoogleOAuthConfig::default()
    };
    if let Ok(secret) = std::env::var(ENV_GOOGLE_CLIENT_SECRET) {
        config.client_secret = secret;
    }
    if let Ok(redirect) = std::env::var(ENV_GOOGLE_REDIRECT_URI) {
        config.redirect_uri = redirect;
    }
    Some(config)
}
```

In `main`, after `let state = ServerState::new_with_plugins(...)` and before binding the listener, add:

```rust
    if let Some(oauth) = google_oauth_config_from_env() {
        match server::secrets::open_integration_store(&config_dir) {
            Ok(store) => {
                let token_manager = std::sync::Arc::new(server::oauth::TokenManager::new(
                    std::sync::Arc::new(store),
                    std::sync::Arc::new(server::oauth::transport::EgressTransport),
                    oauth.clone(),
                ));
                let runtime = std::sync::Arc::new(server::oauth::IntegrationRuntime::new(
                    token_manager,
                    server::oauth::session::SessionSigner::from_admin_token(&admin_token),
                    oauth,
                ));
                state.set_integrations(runtime);
                tracing::info!("google oauth integration enabled");
            }
            Err(error) => panic!(
                "{ENV_GOOGLE_CLIENT_ID} is set but the secrets store could not open \
                 (fail-closed per the secrets contract): {error}"
            ),
        }
    }
```

`admin_token` is already bound in `main`; `state` is used again below for `shutdown_state = state.clone()`, and `set_integrations` takes `&self`, so no move conflict is introduced. `open_integration_store` fails closed if `secrets.enc` exists but no key is configured, which is the intended startup refusal.

- [ ] **Step 4: Run tests and the full gate**

Run:
```bash
cd companion
cargo test -p server --bin server google_config_is_none
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo test --workspace --doc
```
Expected: PASS across the board. (The `--workspace` runs and `tests/oauth_routes.rs` are the controller's, per the loopback-sandbox note.)

- [ ] **Step 5: Commit**

```bash
git add companion/crates/server/src/main.rs
git commit -m "feat: wire the Google OAuth integration runtime from the environment at startup"
```

---

## Self-Review

**1. Spec coverage.**

- §2 architecture: `IntegrationRuntime` holds `TokenManager` + `SessionSigner` + `GoogleOAuthConfig` + the pending stash (Task 6); `TokenManager` owns the in-memory access-token cache and drives `IntegrationStore` (Task 4); `EgressTransport` keeps all Google calls on the guard (Tasks 1, 3); `ServerState` seam via `OnceLock` (Task 6). The `app-core ← plugin` direction is untouched — nothing here is added to `app-core`. ✓
- §3 OAuth flow end-to-end: register (env config, Task 7) → consent start with `state`+PKCE+`access_type=offline`+`prompt=consent` (Task 6 `start_consent`, Task 2 primitives) → callback verifies `state` single-use/TTL/session-bound before exchange (Task 6 `google_callback`, `take_pending`) → token store persists refresh token + client secret, access token cached in memory (Task 4 `exchange_code`) → refresh within skew (Task 4 `access_token`) → provider consumption is sub-project 3, correctly absent. ✓
- §6 security: encryption at rest is sub-project 1 (reused, not re-done); access token never persisted (Task 4, asserted by the store-holds-only-refresh design and the no-write-on-refresh comment); redirect URI is a single configured value (Task 4 default, Task 7 override); `state` CSRF + PKCE (Tasks 2, 6); egress guard on every credential call including a re-pinned redirect check (Task 1); admin-cookie gate, `HttpOnly`/`Secure`/`SameSite=Lax` (Task 5); no token readback (no route returns a token; revoke returns 204); bare 401 on unauthenticated (Tasks 5, 6, asserted in the route test). ✓
- §8 sub-project 2 boundary: `TokenManager`, the four routes, `state`+PKCE stash, admin-cookie auth — all delivered. Calendar provider and `/v1/manage` HTML explicitly deferred (stated in Global Constraints; callback redirects to `/v1/manage` as a URL, not a built dependency). ✓
- Hard constraint 1 (sync Mutex in async): Task 4's `store_get`/`store_put`/`store_remove` each wrap `spawn_blocking` with a cloned `Arc`, with an explicit comment; no other code calls the store. ✓
- Hard constraint 2 (egress, no raw reqwest, no per-host exception): Task 1 extends the guard; Task 3's `EgressTransport` is the only transport; Google hosts pass the allowlist unchanged. ✓
- Hard constraint 3 (no app-core/schema): confirmed — every file touched is under `companion/crates/server/`. ✓
- Hard constraint 4 (follow existing patterns): `OperatorAuthenticated` mirrors `admin.rs`'s `AdminAuthenticated` extractor and `auth.rs`'s bare-401 posture; routes use the `admin::routes()` `Router<ServerState>` style; `constant_time_eq`/`bearer_token` reused rather than reimplemented. ✓
- Hard constraint 5 (testability): fake transport + injected clock (Task 4) cover refresh-on-skew, cache reuse, invalid_grant→NeedsReconnect, transport→Error, revoke-clears; PKCE/state and cookie are pure-unit-tested (Tasks 2, 5); loopback route/egress tests flagged controller-run (Tasks 1, 6). ✓

**2. Placeholder scan.** No "TBD"/"TODO"/"handle errors"/"similar to Task N". Every code step carries a real body and every test a real assertion. Two conditional instructions are concrete, not placeholders: the `tempfile` `keep()`-vs-`into_path()` note names the exact fallback and how to choose (check `Cargo.lock`), and the `session.rs` `Arc` import shim names the exact lines to delete once Task 6 lands. The Task 5→6 build-order dependency is called out explicitly with a run instruction, not left implicit. ✓

**3. Type consistency.**

- `OAuthTransport::post_form(&self, url: String, form: Vec<(String, String)>) -> OAuthFuture<'_, Result<FetchResponse, TransportError>>` — identical signature in the trait (Task 3), `EgressTransport` (Task 3), `FakeTransport` (Task 4), and `NoTransport` (Task 6). ✓
- `FetchResponse { status: u16, body: Vec<u8> }` is `egress`'s existing type, reused across transport and `TokenManager`; `classify_token_response(status: u16, body: &[u8])` matches its fields. ✓
- `GoogleOAuthConfig` defined in `token.rs`, re-exported from `oauth/mod.rs`, consumed by routes (Task 6) and `main.rs` (Task 7) as `server::oauth::GoogleOAuthConfig`. The Task 4 interface line said `super::GoogleOAuthConfig`; Step 3 corrects it to a local definition and removes the stray `use super::GoogleOAuthConfig;` — noted inline so no dangling import remains. ✓
- `IntegrationHealth::{Connected, NeedsReconnect, Error(String)}` used identically in Task 4's transitions and assertions. ✓
- `TokenManager::{new, with_clock, exchange_code, access_token, revoke, health}` — names and signatures match between the interface block, the implementation, and every call in routes (`exchange_code`, `revoke`) and tests. ✓
- `IntegrationRuntime::{new, token_manager, sessions, start_consent, take_pending}` consistent between Task 6's interface, impl, routes, and tests; `PendingAuth { integration_id, code_verifier, sid }` fields match producer (`start_consent`) and consumer (`google_callback`). ✓
- `SessionSigner::{from_admin_token, mint, verify, new_sid}`, `SESSION_COOKIE`, `set_cookie_header`, `OperatorAuthenticated { sid }` consistent across Tasks 5 and 6 and the route test. ✓
- `IntegrationSecret { provider, refresh_token, client_secret: Option<String>, scopes, obtained_at: i64 }` matches the committed sub-project-1 struct exactly (Task 4 constructs it with those fields). ✓
- `ServerState::{integrations, set_integrations}` produced in Task 6, consumed by Task 5's extractor and Task 7's wiring — same signatures. ✓

No gaps found. The plan covers §2, §3, §6, and sub-project 2's §8 scope, respects all five hard constraints, and is internally type-consistent. One inter-task build-order coupling (Task 5's extractor needs Task 6's `ServerState` methods) is explicitly flagged with instructions rather than left to fail silently.
