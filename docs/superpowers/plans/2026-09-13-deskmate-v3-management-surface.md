# V3 Sub-Project 4 — Management Surface Implementation Plan

> **STATUS: EXECUTED 2026-09-13, all five tasks complete.** Boxes are ticked as
> verified. Commits: `5749461` (one `AdminAuthenticated`), `78987b3` (the two
> listings), `64ed968` (the surface itself). Full gate green: fmt, clippy, 643
> workspace tests, doctests, 112 frontend tests, `tsc --noEmit`.
>
> Four deviations, each forced by what execution turned up:
>
> | Planned | Actual |
> |---|---|
> | Tasks 3, 4 and 5 as three commits | **One commit.** A view layer with no consumer is dead code that fails `-D warnings`, so "the dashboard renders" is the smallest unit that can be green on its own. Task 2 hit the same wall and needed a temporary `#[allow(dead_code)]`, removed in the next commit. |
> | Actions call the stores directly | **Extracted `revoke_integration_action` and `mint_producer_action` into `oauth::routes`**, called by both the JSON route and the form action. Calling the stores directly would have reimplemented — and risked half-implementing — sub-project 3's revocation coupling. A test mints through the dashboard and vends through the API to prove one store. |
> | (not planned) | **`render_error`,** added on review. `connect_integration`'s failure path rendered the *sign-in form* for a "too many pending consents" error, which tells the operator to re-authenticate and hides the cause. |
> | (not planned) | Four accessors the model needs: `ServerState::registry_device_ids` / `device_is_linked`, `TokenManager::integration_ids`, `ProducerCredentialStore::has_credential` — all presence-only, no credential leaves any of them. |
>
> `clippy::format_push_string` rejects `push_str(&format!(…))`; the render
> functions use `write!` into the buffer instead.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A single server-rendered page at `/v1/manage` that makes the Mac app unnecessary for owning a networked device: device connection status, integration health, image-source liveness, and the four actions an operator needs.

**Architecture:** Server-rendered HTML on the existing Axum router, gated by the `OperatorAuthenticated` extractor sub-project 2 built. No SPA, no build pipeline, no template-engine dependency — a handful of pure `fn render_*(…) -> String` functions over plain data structs, unit-testable without a server, plus thin handlers. Browser-facing action routes live under `/v1/manage/…` and redirect; the machine-facing `/v1/…` API routes keep their JSON and status codes unchanged.

**Tech Stack:** Rust, Axum 0.8 (`Html`, `Form`, `Redirect`), the existing `SessionSigner` cookie.

**Spec:** `docs/superpowers/specs/2026-09-12-deskmate-v3-server-host-revision.md` (§2 item 3, §7 the two health axes, §10 sub-project 4, §13). Sub-project 3's plan is `2026-09-12-deskmate-v3-token-vending.md`.

## Global Constraints

- **Schema stays v10, protocol v2.** No config field, no wire message, no firmware change.
- **`IntegrationStore` and `ProducerCredentialStore` hold a sync `Mutex` across disk I/O.** Any handler that writes goes through `tokio::task::spawn_blocking`.
- **Never render a credential except the one-time mint result**, and never put one in a URL, a redirect target, or a query string — those reach browser history and access logs.
- **Every interpolated value is HTML-escaped.** Integration ids are charset-validated, but image-source *names* and device ids are not; one unescaped `<` is a stored XSS on the operator's own dashboard.
- **`cargo` needs `export PATH="$HOME/.cargo/bin:$PATH"`**, and never pipe cargo into `tail` — redirect and check `$?`.
- **Server tests bind loopback**; the controller runs them, not a sandboxed subagent.
- **This worktree needs `bun install` in `companion/apps/deskmate`** before the frontend gate means anything (project memory).

## File Structure

| File | Responsibility |
|---|---|
| `companion/crates/server/src/auth.rs` | **Modify.** Gains the one shared `AdminAuthenticated`, replacing two copies. |
| `companion/crates/server/src/admin.rs`, `src/images.rs` | **Modify.** Drop their local copies. |
| `companion/crates/server/src/registry.rs` | **Modify.** `device_ids()` — ids only, never digests. |
| `companion/crates/server/src/image_sources.rs` | **Modify.** `summaries()` — metadata only, no frame bytes. |
| `companion/crates/server/src/manage/mod.rs` | **Create.** Router and handlers. |
| `companion/crates/server/src/manage/view.rs` | **Create.** Pure render functions + HTML escaping. |
| `companion/crates/server/tests/manage.rs` | **Create.** Route-level behaviour. |

---

### Task 1: One `AdminAuthenticated`

Two copies exist; the dashboard would be the third. Both reject with a bare 401, so a single extractor is behaviour-preserving.

**Files:** Modify `src/auth.rs`, `src/admin.rs`, `src/images.rs`

**Interfaces:**
- Produces: `pub(crate) struct AdminAuthenticated;` in `auth.rs`, with `Rejection = AdminUnauthorized`, whose `IntoResponse` is a bare `401`.

- [x] **Step 1: Move the extractor into `auth.rs`**

Define `AdminAuthenticated` and a `pub(crate) struct AdminUnauthorized;` whose `IntoResponse` is `StatusCode::UNAUTHORIZED.into_response()` — bare, no body, matching both current behaviours.

- [x] **Step 2: Delete both local copies**

Remove `admin.rs`'s `struct AdminAuthenticated` + impl and `images.rs`'s, importing `crate::auth::AdminAuthenticated` in each. Keep `ImageRouteError::AdminUnauthorized` only if something still constructs it; if nothing does, delete the variant.

- [x] **Step 3: Verify no behaviour moved**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/v3-server-host/companion
cargo test -p server --all-targets > /tmp/s4-t1.log 2>&1; echo "exit=$?"
```

Expected: `exit=0`, with the existing admin and image-route 401 tests unchanged and still passing. **If any assertion needed editing, stop** — the refactor was supposed to preserve behaviour exactly.

- [x] **Step 4: Commit**

---

### Task 2: The two read-only listings the dashboard needs

**Files:** Modify `src/registry.rs`, `src/image_sources.rs`; tests inline.

**Interfaces:**
- `Registry::device_ids(&self) -> Vec<String>` — sorted, ids only.
- `ImageSourceStore::summaries(&self, now: DateTime<Utc>) -> Vec<SourceSummary>` where
  `pub(crate) struct SourceSummary { pub id: String, pub name: String, pub has_frame: bool, pub stale: bool, pub last_push: Option<DateTime<Utc>> }`.

- [x] **Step 1: Write the failing tests**

```rust
    #[test]
    fn device_ids_lists_every_minted_identity_and_no_digest() {
        let registry = Registry::new();
        let first = registry.mint().expect("mint");
        let second = registry.mint().expect("mint");
        let ids = registry.device_ids();
        assert!(ids.contains(&first.device_id.to_string()));
        assert!(ids.contains(&second.device_id.to_string()));
        assert_eq!(ids.len(), 2);
    }
```

```rust
    #[test]
    fn summaries_report_liveness_without_carrying_frame_bytes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ImageSourceStore::new(dir.path().to_path_buf()).expect("store");
        let minted = store.mint("kitchen", 8).expect("mint");
        let summaries = store.summaries(Utc::now());
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].id, minted.id);
        assert_eq!(summaries[0].name, "kitchen");
        assert!(!summaries[0].has_frame, "nothing has been pushed yet");
        assert_eq!(summaries[0].last_push, None);
    }
```

- [x] **Step 2: Run them and watch them fail**

Expected: FAIL — neither method exists.

- [x] **Step 3: Implement both**

`device_ids` reads the registry's token table and maps to device ids, sorted, recovering a poisoned lock as the rest of that file does. `summaries` reads each `SourceRecord` and reports `is_stale(&record.recent_push_times, now)` plus `record.recent_push_times.last().copied()`. **Neither may clone frame bytes** — `all_frames` exists for the push path and is the wrong tool here.

- [x] **Step 4: Run them and watch them pass, then commit**

---

### Task 3: The view layer, as pure functions

Rendering is where an injection bug would live, and it needs no server to test.

**Files:** Create `src/manage/view.rs`

**Interfaces:**
- `pub(crate) fn escape_html(value: &str) -> String`
- `pub(crate) struct DashboardModel { pub devices: Vec<DeviceRow>, pub integrations: Vec<IntegrationRow>, pub sources: Vec<SourceRow> }` plus the three row structs
- `pub(crate) fn render_dashboard(model: &DashboardModel) -> String`
- `pub(crate) fn render_login(error: Option<&str>) -> String`
- `pub(crate) fn render_minted_credential(integration_id: &str, token: &str) -> String`

- [x] **Step 1: Write the failing tests**

```rust
    #[test]
    fn escaping_covers_every_character_that_can_leave_an_attribute_or_element() {
        assert_eq!(
            escape_html(r#"<script>"x"&'y'</script>"#),
            "&lt;script&gt;&quot;x&quot;&amp;&#39;y&#39;&lt;/script&gt;"
        );
    }

    #[test]
    fn a_hostile_image_source_name_cannot_inject_markup() {
        // Source names are operator-supplied at mint time and pass through no
        // charset validation, unlike integration ids.
        let model = DashboardModel {
            devices: Vec::new(),
            integrations: Vec::new(),
            sources: vec![SourceRow {
                id: "image-1".to_string(),
                name: "<img src=x onerror=alert(1)>".to_string(),
                has_frame: true,
                stale: false,
                last_push: Some("2026-09-13T00:00:00Z".to_string()),
            }],
        };
        let html = render_dashboard(&model);
        assert!(!html.contains("<img src=x"));
        assert!(html.contains("&lt;img src=x"));
    }

    #[test]
    fn the_two_health_axes_are_rendered_distinctly() {
        // Spec §7: a stale card with a healthy integration means the producer is
        // broken; a stale card with NeedsReconnect means the owner must act.
        // Collapsing them into one "error" is the failure this page exists to
        // prevent.
        let model = DashboardModel {
            devices: Vec::new(),
            integrations: vec![IntegrationRow {
                id: "google".to_string(),
                health: "NeedsReconnect".to_string(),
                needs_reconnect: true,
                has_producer_credential: true,
            }],
            sources: vec![SourceRow {
                id: "image-1".to_string(),
                name: "calendar".to_string(),
                has_frame: true,
                stale: true,
                last_push: Some("2026-09-13T00:00:00Z".to_string()),
            }],
        };
        let html = render_dashboard(&model);
        assert!(html.contains("Reconnect"));
        assert!(html.contains("Stale"));
    }

    #[test]
    fn the_minted_credential_page_says_it_is_shown_once() {
        let html = render_minted_credential("google", "abc123");
        assert!(html.contains("abc123"));
        assert!(html.to_lowercase().contains("once"));
    }
```

- [x] **Step 2: Run them and watch them fail**

- [x] **Step 3: Implement the view**

`escape_html` replaces `&` first, then `<`, `>`, `"`, `'`. Order matters: escaping `&` after the others would double-escape their entities. Every interpolation in every render function goes through it, with no exceptions for values that "cannot" contain markup.

Keep the markup plain: one `<style>` block, semantic tables, no external assets — the CSP-free page still must not depend on a font host or CDN, matching how the app already refuses them.

- [x] **Step 4: Run them, watch them pass**

- [x] **Step 5: Probe the escaping**

Make `escape_html` return its input unchanged and re-run: `a_hostile_image_source_name_cannot_inject_markup` and the escaping test must both FAIL. Restore.

- [x] **Step 6: Commit**

---

### Task 4: `GET /v1/manage` and the login pages

`google_callback` already redirects to `/v1/manage`, which 404s today — this closes that.

**Files:** Create `src/manage/mod.rs`; modify `src/lib.rs` (module + `.merge`); create `tests/manage.rs`

**Interfaces:**
- `pub(crate) fn routes() -> Router<ServerState>`
- `GET /v1/manage` → 200 HTML, or 401 bare without a session
- `GET /v1/manage/login` → 200 HTML form (always reachable)
- `POST /v1/manage/login` → form-encoded `token`; on success sets the session cookie and 303s to `/v1/manage`; on failure re-renders the form with an error and 401

- [x] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn the_dashboard_refuses_an_unauthenticated_caller_with_a_bare_401() {
    let response = get(&state(), "/v1/manage", None).await;
    assert_eq!(response.0, StatusCode::UNAUTHORIZED);
    assert!(response.1.is_empty(), "no dashboard content may leak");
}

#[tokio::test]
async fn the_dashboard_renders_for_the_admin_bearer() {
    let (status, body) = get(&state(), "/v1/manage", Some("in-memory-admin-token")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("<html") || body.contains("<!doctype"));
}

#[tokio::test]
async fn the_login_page_is_reachable_without_a_session() {
    let (status, body) = get(&state(), "/v1/manage/login", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("<form"));
}

#[tokio::test]
async fn a_wrong_token_does_not_mint_a_session() {
    let (status, headers) = post_form(&state(), "/v1/manage/login", "token=wrong").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!headers.contains_key(header::SET_COOKIE));
}

#[tokio::test]
async fn the_right_token_mints_a_session_cookie() {
    let (status, headers) =
        post_form(&state(), "/v1/manage/login", "token=in-memory-admin-token").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let cookie = headers
        .get(header::SET_COOKIE)
        .expect("a session cookie")
        .to_str()
        .unwrap();
    assert!(cookie.contains("HttpOnly"), "the cookie must not be script-readable");
    assert!(cookie.contains("Secure"));
}

#[tokio::test]
async fn the_dashboard_never_renders_a_stored_credential() {
    // The page reports presence and health, never values (spec §8, and V2's
    // NVS no-readback rule).
    let state = state_with_stored_grant();
    let (_, body) = get(&state, "/v1/manage", Some("in-memory-admin-token")).await;
    assert!(!body.contains(STORED_REFRESH_TOKEN));
    assert!(!body.contains(STORED_CLIENT_SECRET));
}
```

- [x] **Step 2: Run, watch them fail (404s)**

- [x] **Step 3: Implement the handlers**

`GET /v1/manage` builds a `DashboardModel` from `Registry::device_ids()` + `ServerState::device_link()` for liveness, the integration store's ids + `TokenManager::health`, and `ImageSourceStore::summaries()`. `POST /v1/manage/login` verifies with `verify_admin_token` and mints through the same `SessionSigner` and TTL the JSON login route uses — one cookie format, not two.

- [x] **Step 4: Run, watch them pass, then commit**

---

### Task 5: The three actions

**Files:** Modify `src/manage/mod.rs`, `tests/manage.rs`

**Interfaces:**
- `POST /v1/manage/integrations/{id}/connect` → 303 to the provider's consent URL
- `POST /v1/manage/integrations/{id}/revoke` → 303 back to `/v1/manage`
- `POST /v1/manage/integrations/{id}/producer` → 200 HTML showing the credential once

- [x] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn minting_from_the_dashboard_shows_the_credential_once_and_not_in_a_url() {
    let state = state_with_stored_grant();
    let (status, body) = post_form_authed(
        &state,
        "/v1/manage/integrations/google/producer",
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "rendered inline, never redirected");
    // A redirect would put the credential in the Location header, browser
    // history and every access log between here and the operator.
    assert!(body.contains("once"));
}

#[tokio::test]
async fn the_dashboard_actions_refuse_an_unauthenticated_caller() {
    for path in [
        "/v1/manage/integrations/google/connect",
        "/v1/manage/integrations/google/revoke",
        "/v1/manage/integrations/google/producer",
    ] {
        let (status, _) = post_form(&state(), path, "").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{path} must be gated");
    }
}

#[tokio::test]
async fn revoking_from_the_dashboard_redirects_back_to_it() {
    let state = state_with_stored_grant();
    let (status, headers) =
        post_form_authed_raw(&state, "/v1/manage/integrations/google/revoke", "").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers.get(header::LOCATION).unwrap(), "/v1/manage");
}
```

- [x] **Step 2: Run, watch them fail**

- [x] **Step 3: Implement, reusing the stores through `spawn_blocking`**

Each action calls exactly what the JSON route calls — the same `IntegrationRuntime`, the same `ProducerCredentialStore` — so there is one implementation of each behaviour and the browser path cannot drift from the API path. The mint action renders; the other two redirect.

- [x] **Step 4: Verify the `spawn_blocking` discipline by inspection**

```bash
grep -n "producer_credentials()\." companion/crates/server/src/manage/mod.rs
```

Every hit must sit inside a `spawn_blocking` closure.

- [x] **Step 5: Full gate**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /Users/rodion/dev/deskmate/.worktrees/v3-server-host/companion
cargo fmt --all --check > /tmp/s4-fmt.log 2>&1; echo "fmt=$?"
cargo clippy --workspace --all-targets -- -D warnings > /tmp/s4-clippy.log 2>&1; echo "clippy=$?"
cargo test --workspace --all-targets > /tmp/s4-test.log 2>&1; echo "test=$?"
cargo test --workspace --doc > /tmp/s4-doc.log 2>&1; echo "doc=$?"
cd apps/deskmate && bun install && bun test && bun run check
```

- [x] **Step 6: Commit**

---

## What this plan deliberately does not do

- **No calendar picker, no multi-account UI.** One integration, `primary`, per the spec's lower-order items.
- **No device actions.** The dashboard *reports* device connection state; minting and provisioning stay cable-and-API operations (spec's provisioning trap).
- **No CSS framework or JS.** Four pages of forms do not need a build step, and the page must not depend on an external host.
- **No reference producer.** It follows in `tools/picture-producers/` and needs a real Google grant.
