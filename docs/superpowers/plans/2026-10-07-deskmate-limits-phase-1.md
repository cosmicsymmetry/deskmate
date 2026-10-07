# Limits phase 1: the account-policy hook and R1 -- Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the server one per-account policy snapshot that every existing cap reads, close the source-cap race, and let `describe` tell the server which children a face can use so it stops starting the ones that cannot help.

**Architecture:** `Entitlements` collapses to one method returning an immutable `AccountPolicy`; `SelfHosted` returns exactly today's values, so nothing a self-hoster or the live server sees changes except one corrected error sentence. The image-source cap moves inside `ImageSourceStore`'s mutex. The faces package's `describe` gains three additive keys (`origin`, `views`, `selector`); the server skips the `views` child and the selector exchange for a kind that declares neither, which is every plugin today.

**Tech Stack:** Rust 1.98 (axum, serde, tokio), TypeScript on Bun 1.4 for `companion/faces`.

**Spec:** `docs/superpowers/specs/2026-10-04-deskmate-resource-limits-design.md` (revision 6, approved by the owner 2026-10-07). This plan is section 12's first task plus R1 from section 8. The CPU ledger, cadence floor, fallback spacing, `POST` admission, R2 and child supervision are later plans; do not start them here.

## Global Constraints

- No change to `CURRENT_SCHEMA_VERSION`, the device wire, or firmware statics.
- `SelfHosted` returns exactly today's values: panels `None`, cards `MAX_CONFIG_CARDS` (8), image sources `MAX_IMAGE_SOURCES` (8).
- `unsafe_code = "forbid"` stays.
- `CatalogFace` keeps no `deny_unknown_fields`; every new key is optional, and an absent `views`/`selector` means **true** so an older faces package dispatches exactly as today.
- `FaceDescriptor` (sent to the browser) does not change shape.
- The card-limit message is per panel: `This panel can have at most {maximum} cards.`
- A self-hoster never sees an upgrade prompt; the store's own ceiling keeps its existing body (`the image-source capacity has been reached`).
- Commits use conventional prefixes and end with the session's attribution lines.
- `cargo` needs `export PATH="$HOME/.cargo/bin:$PATH"`; redirect cargo output to a file and check `$?`, never pipe into `tail`.

## Review Focus

- An **older faces package** whose `describe` has no `origin`/`views`/`selector` keys must keep running the views child and the selector exactly as today (Task 3 test `an_older_catalog_without_capability_keys_keeps_every_child`).
- A **withdrawn plugin tombstone** in the catalog must declare no views and no selector, so a card left on a withdrawn plugin never starts a views child (Task 3 TypeScript test).
- **Two concurrent mints for the last slot** must yield exactly one success, under both the store ceiling and a lower policy cap (Task 2 test `the_last_slot_goes_to_exactly_one_of_two_concurrent_mints`).
- A **policy cap below an account's existing count** (a downgrade) must refuse new sources without touching the existing ones (Task 2 test `a_cap_below_the_existing_count_refuses_and_keeps_every_source`).
- A **built-in face** (weather, Hacker News) must still stage its views and answer staged taps after R1 (Task 4 test `a_builtin_still_stages_and_selects`).

---

### Task 1: `AccountPolicy` replaces the four `Entitlements` methods

**Files:**
- Modify: `companion/crates/server/src/entitlements.rs` (whole file)
- Modify: `companion/crates/server/src/lib.rs:35` (re-export)
- Modify: `companion/crates/server/src/claim.rs:91-99`
- Modify: `companion/crates/server/src/images.rs:259-268` (read the policy; the race fix is Task 2)
- Modify: `companion/crates/server/src/app_api/mod.rs:559-573`
- Modify: `companion/crates/server/tests/accounts.rs:218-237` (`OnePanel`)
- Test: `companion/crates/server/src/entitlements.rs` (unit), `companion/crates/server/tests/companion_api.rs` (card limit)

**Interfaces:**
- Produces:
  ```rust
  pub struct AccountPolicy { pub max_panels: Option<usize>, pub max_cards: usize, pub max_image_sources: usize }
  impl AccountPolicy {
      pub const SELF_HOSTED: AccountPolicy;
      pub fn effective_cards(&self) -> usize;          // min(max_cards, MAX_CONFIG_CARDS)
      pub fn effective_image_sources(&self) -> usize;  // min(max_image_sources, MAX_IMAGE_SOURCES)
  }
  pub trait Entitlements: Send + Sync + 'static { fn policy(&self, account: &AccountId) -> AccountPolicy; }
  ```
  Re-exported from the crate root as `server::AccountPolicy`. Later plans add fields; this one adds only what existing checks read. `feature_enabled` is deleted: it has no production caller.

- [ ] **Step 1: Write the failing unit test** at the bottom of `entitlements.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_hosted_is_exactly_the_structural_ceiling() {
        let account = AccountId("acct-self-host-test".to_owned());
        assert_eq!(
            SelfHosted.policy(&account),
            AccountPolicy {
                max_panels: None,
                max_cards: MAX_CONFIG_CARDS,
                max_image_sources: MAX_IMAGE_SOURCES,
            }
        );
    }

    #[test]
    fn a_policy_above_the_structure_is_clamped_to_it() {
        let generous = AccountPolicy {
            max_panels: Some(100),
            max_cards: 99,
            max_image_sources: 99,
        };
        assert_eq!(generous.effective_cards(), MAX_CONFIG_CARDS);
        assert_eq!(generous.effective_image_sources(), MAX_IMAGE_SOURCES);
        let strict = AccountPolicy { max_cards: 2, max_image_sources: 4, ..generous };
        assert_eq!(strict.effective_cards(), 2);
        assert_eq!(strict.effective_image_sources(), 4);
    }
}
```

- [ ] **Step 2: Run it and see it fail**

Run: `cd companion && cargo test -p server --lib entitlements > /tmp/t1.log 2>&1; echo $?; grep -E "error|test result" /tmp/t1.log | head`
Expected: non-zero; `cannot find type AccountPolicy` / `no method named policy`.

- [ ] **Step 3: Replace the trait and `SelfHosted`** in `entitlements.rs` (keep `Edition` as is):

```rust
/// What one account may hold, as one immutable snapshot. Read it at the
/// moment of admission; never cache it across a lock release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountPolicy {
    /// `None` means the edition sets no panel limit.
    pub max_panels: Option<usize>,
    /// Per PANEL, not per account: each panel's config holds its own cards.
    pub max_cards: usize,
    pub max_image_sources: usize,
}

impl AccountPolicy {
    /// The public build's policy: no commercial cap below the structure.
    pub const SELF_HOSTED: Self = Self {
        max_panels: None,
        max_cards: MAX_CONFIG_CARDS,
        max_image_sources: MAX_IMAGE_SOURCES,
    };

    #[must_use]
    pub fn effective_cards(&self) -> usize {
        self.max_cards.min(MAX_CONFIG_CARDS)
    }

    #[must_use]
    pub fn effective_image_sources(&self) -> usize {
        self.max_image_sources.min(MAX_IMAGE_SOURCES)
    }
}

pub trait Entitlements: Send + Sync + 'static {
    fn policy(&self, account: &AccountId) -> AccountPolicy;
}

#[derive(Debug, Default)]
pub struct SelfHosted;

impl Entitlements for SelfHosted {
    fn policy(&self, _account: &AccountId) -> AccountPolicy {
        AccountPolicy::SELF_HOSTED
    }
}
```

In `lib.rs:35`: `pub use entitlements::{AccountPolicy, Edition, Entitlements, SelfHosted};`

- [ ] **Step 4: Route the three callers through it**

`claim.rs` (inside `with_device_lifecycle`):
```rust
            if claim_state
                .entitlements()
                .policy(&account_id)
                .max_panels
                .is_some_and(|limit| owned.len() >= limit)
```

`images.rs` (keep the pre-check for now; Task 2 deletes it):
```rust
    let maximum = state
        .entitlements()
        .policy(&space.account_id)
        .effective_image_sources();
```

`app_api/mod.rs`:
```rust
    let maximum = state
        .entitlements()
        .policy(&space.account_id)
        .effective_cards();
    if config.cards.len() > maximum {
        return Err(AppApiError::Validation {
            message: "the configuration is not valid".into(),
            issues: vec![ValidationIssue {
                path: "cards".into(),
                code: app_core::ValidationCode::TooMany,
                message: format!("This panel can have at most {maximum} cards."),
            }],
        });
    }
```

`tests/accounts.rs` `OnePanel`:
```rust
impl Entitlements for OnePanel {
    fn policy(&self, _account: &server::identity::AccountId) -> server::AccountPolicy {
        server::AccountPolicy { max_panels: Some(1), ..server::AccountPolicy::SELF_HOSTED }
    }
}
```

- [ ] **Step 5: Add the card-limit route test** to `tests/companion_api.rs` (no existing test pins this message):

```rust
#[derive(Debug)]
struct OneCard;

impl server::Entitlements for OneCard {
    fn policy(&self, _account: &server::identity::AccountId) -> server::AccountPolicy {
        server::AccountPolicy { max_cards: 1, ..server::AccountPolicy::SELF_HOSTED }
    }
}

#[tokio::test]
async fn a_panel_over_its_card_limit_is_refused_per_panel() {
    let state = ServerState::in_memory_with_options(server::ServerOptions {
        entitlements: std::sync::Arc::new(OneCard),
        ..server::ServerOptions::default()
    });
    support::owner_account(&state);
    let server = spawn_with(state, None).await;
    let client = Client::new();
    let device = mint_device(&client, &server).await;

    let mut config = snapshot(&client, &server, &device.device_id).await["config"].clone();
    let mut second = config["cards"][0].clone();
    second["id"] = serde_json::json!("second-card");
    config["cards"].as_array_mut().expect("cards").push(second);

    let response = save_config(&client, &server, &device.device_id, &config).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = json_body(response).await;
    assert_eq!(body["issues"][0]["path"], "cards");
    assert_eq!(body["issues"][0]["message"], "This panel can have at most 1 cards.");
}
```

If the fresh config has more than one card already, drop the push and assert on the unmodified config instead; if the copied card fails validation for a reason other than the count (for example a duplicate source), use a second clock card built the way `companion_api.rs:1128` builds one. Check the status code the route actually returns for a `Validation` error (grep `AppApiError::Validation` in `app_api`) and use that.

- [ ] **Step 6: Run the narrow tests, then the server crate**

Run: `cd companion && cargo test -p server --lib entitlements > /tmp/t1.log 2>&1; echo $?` then `cargo test -p server --test companion_api --test accounts > /tmp/t1b.log 2>&1; echo $?`
Expected: both `0`.

- [ ] **Step 7: Commit**

```bash
git add companion/crates/server/src/entitlements.rs companion/crates/server/src/lib.rs companion/crates/server/src/claim.rs companion/crates/server/src/images.rs companion/crates/server/src/app_api/mod.rs companion/crates/server/tests/accounts.rs companion/crates/server/tests/companion_api.rs
git commit -m "refactor(server): one AccountPolicy snapshot behind every account cap"
```

---

### Task 2: the source cap is checked under the store's own lock

**Files:**
- Modify: `companion/crates/server/src/image_sources.rs:160-175` (error), `:202-205` (`mint`)
- Modify: `companion/crates/server/src/images.rs:259-275` (delete the pre-check), `:431-442` (`map_mint_error`)
- Test: `companion/crates/server/src/image_sources.rs` (unit tests module)

**Interfaces:**
- Consumes: `AccountPolicy::effective_image_sources()` from Task 1.
- Produces:
  ```rust
  impl ImageSourceStore {
      pub(crate) fn mint(&self, name: &str) -> Result<MintedSource, ImageSourceError>; // unchanged: mint_within(name, MAX_IMAGE_SOURCES)
      pub(crate) fn mint_within(&self, name: &str, limit: usize) -> Result<MintedSource, ImageSourceError>;
  }
  // New variant:
  ImageSourceError::PolicyCapacity { maximum: usize } // limit < MAX_IMAGE_SOURCES and reached
  ```
  `mint` stays so the many test callers in `data_cards.rs` need no edit.

- [ ] **Step 1: Write the failing tests** in `image_sources.rs`'s test module:

```rust
    #[test]
    fn a_lower_limit_is_its_own_refusal_and_the_ceiling_keeps_its_old_one() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = ImageSourceStore::new(temp.path().to_path_buf()).expect("store");
        for index in 0..4 {
            store.mint_within(&format!("Source {index}"), 4).expect("within four");
        }
        assert!(matches!(
            store.mint_within("Fifth", 4),
            Err(ImageSourceError::PolicyCapacity { maximum: 4 })
        ));
        for index in 4..MAX_IMAGE_SOURCES {
            store.mint(&format!("Source {index}")).expect("within the ceiling");
        }
        assert!(matches!(store.mint("Ninth"), Err(ImageSourceError::Capacity)));
    }

    #[test]
    fn a_cap_below_the_existing_count_refuses_and_keeps_every_source() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = ImageSourceStore::new(temp.path().to_path_buf()).expect("store");
        for index in 0..6 {
            store.mint(&format!("Source {index}")).expect("mint");
        }
        assert!(matches!(
            store.mint_within("After downgrade", 4),
            Err(ImageSourceError::PolicyCapacity { maximum: 4 })
        ));
        assert_eq!(store.summaries(chrono::Utc::now()).len(), 6);
    }

    #[test]
    fn the_last_slot_goes_to_exactly_one_of_two_concurrent_mints() {
        for limit in [4, MAX_IMAGE_SOURCES] {
            let temp = tempfile::tempdir().expect("temp dir");
            let store = std::sync::Arc::new(
                ImageSourceStore::new(temp.path().to_path_buf()).expect("store"),
            );
            for index in 0..limit - 1 {
                store.mint_within(&format!("Source {index}"), limit).expect("mint");
            }
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
            let racers: Vec<_> = (0..2)
                .map(|racer| {
                    let store = std::sync::Arc::clone(&store);
                    let barrier = std::sync::Arc::clone(&barrier);
                    std::thread::spawn(move || {
                        barrier.wait();
                        store.mint_within(&format!("Racer {racer}"), limit).is_ok()
                    })
                })
                .collect();
            let wins = racers
                .into_iter()
                .map(|racer| racer.join().expect("racer"))
                .filter(|won| *won)
                .count();
            assert_eq!(wins, 1, "limit {limit}");
            assert_eq!(store.summaries(chrono::Utc::now()).len(), limit);
        }
    }
```

- [ ] **Step 2: Run them and see them fail**

Run: `cd companion && cargo test -p server --lib image_sources > /tmp/t2.log 2>&1; echo $?; grep -E "error\[|test result" /tmp/t2.log | head`
Expected: non-zero; `no method named mint_within`, `no variant PolicyCapacity`.

- [ ] **Step 3: Implement** in `image_sources.rs`. Add the variant next to `Capacity`:

```rust
    #[error("this account can have at most {maximum} picture sources")]
    PolicyCapacity { maximum: usize },
```

Replace the start of `mint`:

```rust
    pub(crate) fn mint(&self, name: &str) -> Result<MintedSource, ImageSourceError> {
        self.mint_within(name, MAX_IMAGE_SOURCES)
    }

    /// Mints under `limit`, checked while holding the store's mutex so two
    /// callers racing for the last slot cannot both pass. A limit below the
    /// store's ceiling is the account's policy and refuses as such; at the
    /// ceiling the store answers with its own, unchanged refusal.
    pub(crate) fn mint_within(
        &self,
        name: &str,
        limit: usize,
    ) -> Result<MintedSource, ImageSourceError> {
        let mut state = self.lock();
        let limit = limit.min(MAX_IMAGE_SOURCES);
        if state.sources.len() >= limit {
            return Err(if limit < MAX_IMAGE_SOURCES {
                ImageSourceError::PolicyCapacity { maximum: limit }
            } else {
                ImageSourceError::Capacity
            });
        }
        // ... the rest of the former `mint` body, unchanged, from `let id = loop {`
```

Every `match` on `ImageSourceError` must now handle `PolicyCapacity`. In `images.rs`: `map_mint_error` maps `ImageSourceError::PolicyCapacity { maximum } => ImageRouteError::EntitlementCapacity { maximum: *maximum }`; the other mappers (`map_revoke_error` and the two after it) add `| ImageSourceError::PolicyCapacity { .. }` to their `Internal` arm.

Replace the pre-check in the create route (`images.rs` around 259-275) with:

```rust
    let face_kind = request.face_kind.clone();
    let space = account_space(&state, &session).await?;
    let limit = state
        .entitlements()
        .policy(&space.account_id)
        .effective_image_sources();
    let mint_space = Arc::clone(&space);
    let minted = tokio::task::spawn_blocking(move || {
        mint_space.image_sources.mint_within(&request.name, limit)
    })
    .await
    .map_err(|_| ImageRouteError::WorkerFailed)?
    .map_err(|error| map_mint_error(&error))?;
```

- [ ] **Step 4: Run the store tests and the image routes**

Run: `cd companion && cargo test -p server --lib image_sources > /tmp/t2.log 2>&1; echo $?` then `cargo test -p server --test image_routes > /tmp/t2b.log 2>&1; echo $?`
Expected: both `0`. `image_routes.rs:501` still sees `the image-source capacity has been reached` at the ceiling, which proves the self-host body is unchanged.

- [ ] **Step 5: Mutation probe.** Move the `if state.sources.len() >= limit` check to before `let mut state = self.lock();` (reading `self.lock().sources.len()` in a separate statement). Run the concurrent test 20 times (`for i in $(seq 20); do cargo test -p server --lib the_last_slot -q > /tmp/m2.log 2>&1 || echo FAIL; done`). A failure shows the test can catch the race; if it never fails, add a `std::thread::sleep(Duration::from_millis(5))` between the check and the push in the mutant only, confirm it fails, and record that in the commit message. Restore the code.

- [ ] **Step 6: Commit**

```bash
git add companion/crates/server/src/image_sources.rs companion/crates/server/src/images.rs
git commit -m "fix(server): check the picture-source cap under the store's lock"
```

---

### Task 3: `describe` says what each face can do

**Files:**
- Modify: `companion/faces/src/main.ts:36-56` (`describeCatalog`)
- Modify: `companion/faces/test/catalog-sample.test.ts`, `companion/faces/test/catalog-sample.json`
- Modify: `companion/crates/server/src/data_cards/faces_package.rs:121-140` (`CatalogFace`) and its tests (`:670-720`)
- Modify: `companion/crates/server/tests/support/fake-faces.sh:9-27` (the fake catalog)

**Interfaces:**
- Produces, on the catalog wire (snake_case, additive):
  `"origin": "builtin" | "plugin"`, `"views": bool`, `"selector": bool` on every entry.
- Produces, in Rust:
  ```rust
  pub(crate) struct CatalogFace { /* existing fields */,
      #[serde(default)] pub(crate) origin: Option<FaceOrigin>,
      #[serde(default)] pub(crate) views: Option<bool>,
      #[serde(default)] pub(crate) selector: Option<bool>,
  }
  #[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
  #[serde(rename_all = "lowercase")]
  pub(crate) enum FaceOrigin { Builtin, Plugin }
  impl CatalogFace {
      pub(crate) fn declares_views(&self) -> bool;    // views.unwrap_or(true)
      pub(crate) fn declares_selector(&self) -> bool; // selector.unwrap_or(true)
  }
  ```
  `origin` is parsed now and read by the plugin-source count in a later plan. An unknown `origin` string must not fail the whole catalog: deserialize it as `None` (see Step 3).

- [ ] **Step 1: Write the failing TypeScript test.** In `catalog-sample.test.ts`, give the first sample face a `views` and an `onTap`, keep the second bare, and add a test for origin and a withdrawn tombstone:

```ts
const SAMPLE_FACES: FaceDefinition[] = [
  {
    kind: "sample-a",
    label: "Sample A",
    fields: [{ type: "text", key: "user", label: "Username", placeholder: "octocat" }],
    tap: "Tap for detail.",
    refreshSeconds: 300,
    views: () => ["", "detail"],
    onTap: () => ({ view: "detail" }),
    render: async () => SQUARE_SVG,
  },
  {
    kind: "sample-b",
    label: "Sample B",
    fields: [],
    render: async () => SQUARE_SVG,
  },
];
```

and, after the existing test:

```ts
test("describe states origin, views and selector for every face", () => {
  const [a, b] = JSON.parse(describeCatalog(SAMPLE_FACES));
  expect([a.origin, a.views, a.selector]).toEqual(["plugin", true, true]);
  expect([b.origin, b.views, b.selector]).toEqual(["plugin", false, false]);
  const builtins = JSON.parse(describeCatalog());
  expect(builtins.every((face: { origin: string }) => face.origin === "builtin")).toBe(true);
  expect(builtins.every((face: { views: boolean }) => face.views)).toBe(true);
});

test("a withdrawn tombstone declares no views and no selector", () => {
  const [tomb] = JSON.parse(
    describeCatalog([
      { kind: "gone", label: "gone", fields: [], withdrawn: "withdrawn by the operator", render: async () => SQUARE_SVG },
    ]),
  );
  expect([tomb.origin, tomb.views, tomb.selector]).toEqual(["plugin", false, false]);
});
```

Check that every built-in in `FACES` really has `views` before keeping the second `expect` in the first test; if one lacks it, assert per kind instead.

- [ ] **Step 2: Run it and see it fail**

Run: `cd companion/faces && bun test test/catalog-sample.test.ts`
Expected: FAIL (`origin` undefined; sample JSON differs).

- [ ] **Step 3: Implement `describeCatalog`:**

```ts
export function describeCatalog(faces: readonly FaceDefinition[] = FACES): string {
  return JSON.stringify(
    faces.map((face) => {
      const { kind, label, fields, tap, refreshSeconds, withdrawn } = face;
      return {
        kind,
        label,
        fields,
        ...(withdrawn === undefined ? {} : { withdrawn }),
        ...(tap === undefined ? {} : { tap }),
        // (keep the existing comment about snake_case here)
        ...(refreshSeconds === undefined ? {} : { refresh_seconds: refreshSeconds }),
        // What the server may skip. A built-in is identified by identity, not by
        // kind, so a plugin folder named like a built-in cannot claim to be one.
        origin: faceOfKind(kind) === face ? "builtin" : "plugin",
        views: withdrawn === undefined && face.views !== undefined,
        selector: withdrawn === undefined && face.onTap !== undefined,
      };
    }),
    null,
    2,
  );
}
```

Regenerate the sample with the test file's own fixture (the existing test explains how the JSON is produced; write `describeCatalog(SAMPLE_FACES)` plus a trailing newline to `test/catalog-sample.json`), then review the diff: it must add exactly the three keys per entry and change nothing else.

In Rust `faces_package.rs`, add the three fields and the enum from **Interfaces**. Deserialize `origin` leniently so a future value does not drop the catalog:

```rust
fn lenient_origin<'de, D>(deserializer: D) -> Result<Option<FaceOrigin>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| serde_json::from_value(value).ok()))
}
```
with `#[serde(default, deserialize_with = "lenient_origin")]` on `origin`, and:

```rust
impl CatalogFace {
    /// Absent means "true": an older package that predates the key keeps
    /// today's dispatch, which always started a views child.
    pub(crate) fn declares_views(&self) -> bool {
        self.views.unwrap_or(true)
    }

    /// Absent means "true", for the same reason, for the selector exchange.
    pub(crate) fn declares_selector(&self) -> bool {
        self.selector.unwrap_or(true)
    }
}
```

Fix every struct literal of `CatalogFace` in tests (`cargo build --tests` names them) by adding `origin: None, views: None, selector: None`.

In `fake-faces.sh`'s `describe`, add `"origin": "builtin", "views": true, "selector": true` to the `headlines` entry only, so the fake keeps an un-flagged entry (`weather`, `rss`, `token`) that exercises the default.

- [ ] **Step 4: Add the Rust tests** in `faces_package.rs`'s test module, and extend the cross-language one:

```rust
    #[test]
    fn an_older_catalog_without_capability_keys_keeps_every_child() {
        let faces: Vec<CatalogFace> =
            serde_json::from_str(r#"[{"kind":"weather","label":"Weather","fields":[]}]"#)
                .expect("an older catalog still parses");
        assert!(faces[0].declares_views());
        assert!(faces[0].declares_selector());
        assert_eq!(faces[0].origin, None);
    }

    #[test]
    fn an_unknown_origin_does_not_drop_the_catalog() {
        let faces: Vec<CatalogFace> = serde_json::from_str(
            r#"[{"kind":"x","label":"X","fields":[],"origin":"partner","views":false,"selector":false}]"#,
        )
        .expect("a future origin value still parses");
        assert_eq!(faces[0].origin, None);
        assert!(!faces[0].declares_views());
        assert!(!faces[0].declares_selector());
    }
```

In `the_catalog_sample_committed_by_the_faces_package_still_deserializes`, add:

```rust
        assert_eq!(sample_a.origin, Some(FaceOrigin::Plugin));
        assert!(sample_a.declares_views() && sample_a.declares_selector());
        assert!(!sample_b.declares_views() && !sample_b.declares_selector());
```

- [ ] **Step 5: Run both sides**

Run: `cd companion/faces && bun test && bun run check && bun run lint && bun run format:check`
Then: `cd companion && cargo test -p server --lib faces_package > /tmp/t3.log 2>&1; echo $?`
Expected: all pass, `0`.

- [ ] **Step 6: Commit**

```bash
git add companion/faces/src/main.ts companion/faces/test/catalog-sample.test.ts companion/faces/test/catalog-sample.json companion/crates/server/src/data_cards/faces_package.rs companion/crates/server/tests/support/fake-faces.sh
git commit -m "feat(faces): describe states each face's origin, views and selector"
```

---

### Task 4: the server skips the children a face cannot use

**Files:**
- Modify: `companion/crates/server/src/data_cards.rs` (a capability lookup beside `face_takes_taps`, ~line 819; `select_staged_view`, ~line 881)
- Modify: `companion/crates/server/src/data_cards/worker.rs:339-365` (`stage_other_views`)
- Test: `companion/crates/server/src/data_cards.rs` (tests module)

**Interfaces:**
- Consumes: `CatalogFace::declares_views()`, `CatalogFace::declares_selector()` (Task 3).
- Produces:
  ```rust
  /// `None` when the catalog has no such kind (the caller keeps today's behaviour).
  pub(crate) fn face_capabilities(state: &ServerState, kind: &str) -> Option<(bool, bool)>; // (views, selector)
  ```

- [ ] **Step 1: Write the failing tests.** In `data_cards.rs`'s test module, write a temporary faces script that logs each verb and declares one plugin without views or selector and one built-in with both, then drive a refresh and a tap through the real code. Mirror the existing helpers in that module (`test_space`, `set_faces`, the refresher harness used by `warm_selector_hits_and_render_fallbacks_keep_the_owners_timezone`); the script:

```sh
#!/bin/sh
printf '%s\n' "$1" >> "$VERB_LOG"
case "$1" in
describe) cat <<'JSON'
[{"kind":"plain-plugin","label":"Plain","fields":[],"tap":"Tap for another.","origin":"plugin","views":false,"selector":false},
 {"kind":"headlines","label":"Headlines","fields":[],"tap":"Tap for more.","origin":"builtin","views":true,"selector":true}]
JSON
;;
render) cat "$FAKE_PNG" ;;
views) cat >/dev/null; printf '{"views":["","page-1"]}' ;;
tap) cat >/dev/null; printf '{"view":"page-1"}' ;;
esac
```

`FaceCommand::program` clears the environment, so bake `VERB_LOG` and `FAKE_PNG` into the script text as absolute paths (format them in with `format!`), with `FAKE_PNG` = `tests/support/fake-face.png`; mark it executable.

Tests (names are the contract; bodies follow the module's existing patterns):

- `a_plugin_without_views_starts_no_views_child`: run `worker::refresh_once` for a `plain-plugin` spec once; the verb log contains `render` and no `views`.
- `a_plugin_tap_skips_the_selector_and_falls_back_to_render`: with a running refresher for `plain-plugin`, call `tapped`; the log gets a second `render` and no `tap`.
- `a_builtin_still_stages_and_selects`: for `headlines`, one refresh logs `render`, `views`, `render`; a following `tapped` logs `tap` and causes no further `render`.

- [ ] **Step 2: Run them and see the first two fail**

Run: `cd companion && cargo test -p server --lib -- a_plugin_without_views a_plugin_tap_skips a_builtin_still > /tmp/t4.log 2>&1; echo $?; grep -E "panicked|test result" /tmp/t4.log`
Expected: the two plugin tests fail (the log shows `views` / `tap`); the built-in test passes.

- [ ] **Step 3: Implement.** Beside `face_takes_taps` in `data_cards.rs`:

```rust
/// What the catalog says `kind` can use: `(views, selector)`. `None` when the
/// catalog does not know the kind, and the caller keeps today's behaviour.
pub(crate) fn face_capabilities(state: &ServerState, kind: &str) -> Option<(bool, bool)> {
    state
        .inner
        .face_catalog
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .catalog()
        .iter()
        .find(|face| face.kind == kind)
        .map(|face| (face.declares_views(), face.declares_selector()))
}
```

`catalog()` (~line 492) takes `&mut self` and returns an `Arc<Vec<CatalogFace>>`, reloading at most once a minute exactly as `descriptor()` already does under the same lock; it is private to the module, which is where this function lives.

At the top of `stage_other_views` in `worker.rs`:

```rust
    // A face that declares no views has nothing to stage, and asking costs a
    // whole child (with plugin discovery). An older catalog says nothing, and
    // then the child runs exactly as before.
    if crate::data_cards::face_capabilities(state, &spec.face.kind)
        .is_some_and(|(views, _)| !views)
    {
        return;
    }
```

In `select_staged_view`, after the spec is found and before taking the transition lock:

```rust
    // A face with no selector cannot answer from a staged view; asking would
    // hold the shared selector worker while the package re-discovers every
    // plugin folder (91 ms median, 191 ms worst on the VM, 2026-10-04) only to
    // refuse. Go straight to the render fallback.
    if face_capabilities(state, &spec.face.kind).is_some_and(|(_, selector)| !selector) {
        return false;
    }
```

Do not hold the catalog mutex while taking the data-cards lock or the transition lock; `face_capabilities` releases it before returning.

- [ ] **Step 4: Run the new tests, then the whole server crate**

Run: `cd companion && cargo test -p server --lib -- a_plugin_without_views a_plugin_tap_skips a_builtin_still > /tmp/t4.log 2>&1; echo $?` then `cargo test -p server --all-targets > /tmp/t4b.log 2>&1; echo $?`
Expected: `0` and `0`.

- [ ] **Step 5: Mutation probes.** Invert each new `is_some_and(... !views)` / `!selector` condition in turn, and separately change `declares_views` to `unwrap_or(false)`; each mutant must fail at least one test (the older-catalog test or the built-in test catches the default). Restore.

- [ ] **Step 6: Commit**

```bash
git add companion/crates/server/src/data_cards.rs companion/crates/server/src/data_cards/worker.rs
git commit -m "perf(server): skip the views child and selector for faces that declare neither"
```

---

### Task 5: documents, gates, PR, deploy

**Files:**
- Modify: `docs/self-host.md:13-16`
- Modify: `docs/superpowers/specs/2026-10-04-deskmate-resource-limits-design.md` (section 9's M4 line; a status line under the title)
- Modify: `docs/roadmap.md` (Track A board line)

- [ ] **Step 1: Self-host paragraph.** Replace "one current frame per source," in `docs/self-host.md` with the spec's section 10 text: "External producers keep one frame per source. Server-rendered sources stage up to four views each, and an account's staged set is bounded at fifteen frames in RAM and on disk. That is a server bound, not a promise about what is resident on the panel." Keep the rest of the sentence's list intact.

- [ ] **Step 2: Record M4 in the spec.** Replace the M4 bullet with: the window ran from a restart at 2026-10-05 21:00:12 UTC to 2026-10-07 08:41:54 UTC (35 h 42 min): `CPUUsageNSec` 1,054.6 s = **.0082 CPU average**, `MemoryPeak` 514,973,696 B = **491 MiB**. Not a full 24 h-from-17:57 window because the service restarted again; it is the longest uninterrupted window available. Add under the title: "Status: approved by the owner 2026-10-07; phase 1 plan `docs/superpowers/plans/2026-10-07-deskmate-limits-phase-1.md`."

- [ ] **Step 3: Run the full gates** from `companion/` (each line separately, each redirected and its `$?` checked):

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo test --workspace --doc
cd apps/deskmate && bun test && bun run check && bun run format:check && bun run build
cd ../../faces && bun test && bun run check && bun run lint && bun run format:check
```

Expected: all exit `0`.

- [ ] **Step 4: Board line, PR, CI.** Update Track A's line: phase 1 implemented on this branch, PR number, what it changes (policy hook, source race, R1), what is next (phase 2 plan: ledger and cadence floor). Push, open the PR with the spec and plan linked, wait for `gh run list --branch track-a/limits-rev4` to show every job green, then merge.

- [ ] **Step 5: Deploy.** `deploy.sh --status`; confirm the live revision is an ancestor of `main` (`git merge-base --is-ancestor <live> origin/main`, or compare trees if it was squash-merged); deploy from `main`. Either order of faces and binary is safe: the old binary ignores the new keys, and the new binary defaults absent keys to today's behaviour. Then verify on the VM, read-only: the journal shows plugin refreshes with no views child (`journalctl -u deskmate-server --since <deploy>` has `server-rendered card refreshing` lines and the installed `describe` output, run as in the spec's section 9 harness, carries `"views": false` for every plugin). Record exactly what was observed in the board line; do not claim a panel observation.
