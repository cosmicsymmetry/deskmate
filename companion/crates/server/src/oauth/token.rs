//! `TokenManager`: OAuth code exchange, skew-window refresh, revoke, and typed
//! integration health.
//!
//! Access tokens are cached in memory only. The durable `IntegrationStore` holds
//! just the refresh token and client secret, written at authorize and revoke --
//! never on refresh, because Google refresh tokens do not rotate. Every store
//! call hops through `spawn_blocking`: `IntegrationStore` holds a std Mutex
//! across its seal+write+fsync, which must not run on the async executor.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Utc};

use super::transport::{OAuthTransport, TokenEndpointError, classify_token_response};
use crate::secrets::{IntegrationSecret, IntegrationStore};

const REFRESH_SKEW_SECONDS: i64 = 60;

/// Google endpoint + client configuration. Defaults are Google's real URIs and
/// the read-only calendar scope; tests override the URIs with dummies because
/// the fake transport ignores them.
#[derive(Clone)]
pub struct GoogleOAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    pub auth_uri: String,
    pub token_uri: String,
    pub revoke_uri: String,
}

impl std::fmt::Debug for GoogleOAuthConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GoogleOAuthConfig")
            .field("client_id", &self.client_id)
            .field("client_secret", &"<redacted>")
            .field("redirect_uri", &self.redirect_uri)
            .field("scopes", &self.scopes)
            .field("auth_uri", &self.auth_uri)
            .field("token_uri", &self.token_uri)
            .field("revoke_uri", &self.revoke_uri)
            .finish()
    }
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
pub(crate) enum IntegrationHealth {
    Connected,
    NeedsReconnect,
    Error(String),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum TokenError {
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

impl std::fmt::Debug for CachedToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CachedToken")
            .field("access_token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

pub struct TokenManager {
    store: Arc<IntegrationStore>,
    transport: Arc<dyn OAuthTransport>,
    oauth: GoogleOAuthConfig,
    now: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    cache: Mutex<HashMap<String, CachedToken>>,
    health: Mutex<HashMap<String, IntegrationHealth>>,
    /// One async gate per integration, so exchange, refresh, and revoke publish
    /// their store/cache/health transitions in acquisition order. The outer
    /// lock is a std `Mutex` because it is only ever held long enough to clone
    /// an `Arc`; the inner one is a tokio `Mutex` because it IS held across an
    /// await.
    operation_gates: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl TokenManager {
    pub fn new(
        store: Arc<IntegrationStore>,
        transport: Arc<dyn OAuthTransport>,
        oauth: GoogleOAuthConfig,
    ) -> Self {
        Self::with_clock(store, transport, oauth, Arc::new(Utc::now))
    }

    pub(super) fn oauth_config(&self) -> &GoogleOAuthConfig {
        &self.oauth
    }

    fn with_clock(
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
            operation_gates: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) async fn exchange_code(
        &self,
        integration_id: &str,
        code: &str,
        code_verifier: &str,
    ) -> Result<(), TokenError> {
        let gate = self.operation_gate(integration_id);
        let _turn = gate.lock().await;
        let form = vec![
            ("grant_type".to_string(), "authorization_code".to_string()),
            ("code".to_string(), code.to_string()),
            ("redirect_uri".to_string(), self.oauth.redirect_uri.clone()),
            ("client_id".to_string(), self.oauth.client_id.clone()),
            (
                "client_secret".to_string(),
                self.oauth.client_secret.clone(),
            ),
            ("code_verifier".to_string(), code_verifier.to_string()),
        ];
        let tokens = self
            .post_tokens(integration_id, &self.oauth.token_uri, form)
            .await?;
        let refresh_token = tokens.refresh_token.ok_or_else(|| {
            let detail = "authorization response contained no refresh_token".to_string();
            self.set_health(integration_id, IntegrationHealth::Error(detail.clone()));
            TokenError::Provider {
                status: 200,
                detail,
            }
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

    /// Test convenience wrapper for callers that do not inspect expiry.
    ///
    /// # Errors
    /// See [`TokenManager::access_token_with_expiry`].
    #[cfg(test)]
    async fn access_token(&self, integration_id: &str) -> Result<String, TokenError> {
        self.access_token_with_expiry(integration_id)
            .await
            .map(|(token, _)| token)
    }

    /// The live access token and the expiry the provider gave it.
    ///
    /// The vend route needs both because it hands a producer the provider's own
    /// credential and expiry, not anything this server mints. The producer can
    /// cache until that instant, while the provider can still revoke it.
    ///
    /// # Errors
    /// [`TokenError::NotFound`] when no credentials are stored,
    /// [`TokenError::NeedsReconnect`] when the grant was revoked or expired,
    /// and the transport/provider variants otherwise.
    pub(crate) async fn access_token_with_expiry(
        &self,
        integration_id: &str,
    ) -> Result<(String, DateTime<Utc>), TokenError> {
        let gate = self.operation_gate(integration_id);
        let _turn = gate.lock().await;
        if let Some(pair) = self.cached_valid(integration_id) {
            return Ok(pair);
        }
        // Serialize refreshes per integration. Without this every caller that
        // missed the cache posts its own `grant_type=refresh_token`: they all
        // yield together at the `spawn_blocking` store read below, so N devices
        // sharing one integration means N simultaneous refreshes, N tokens
        // minted, N-1 of them immediately orphaned by `cache_token`'s
        // last-writer-wins, and Google rate-limiting the client.
        // Re-check under the gate: whoever held it before us has already
        // refreshed and cached, and their token is the one to return.
        if let Some(pair) = self.cached_valid(integration_id) {
            return Ok(pair);
        }
        let secret = self
            .store_get(integration_id.to_string())
            .await?
            .ok_or(TokenError::NotFound)?;
        // The CONFIGURED secret wins over the stored copy. The other way round --
        // which this was -- makes rotating the Google client secret fail in a way
        // nothing explains: a new integration authorizes fine while every existing
        // one keeps sending the retired secret, Google answers `invalid_client`,
        // and that classifies as `Provider`, not `InvalidGrant`, so health reads
        // `Error("invalid_client")` rather than `NeedsReconnect` and never tells
        // the operator that reconnecting is not even the fix. The stored copy
        // remains a fallback for a deployment with no configured secret.
        let client_secret = if self.oauth.client_secret.is_empty() {
            secret.client_secret.unwrap_or_default()
        } else {
            self.oauth.client_secret.clone()
        };
        let form = vec![
            ("grant_type".to_string(), "refresh_token".to_string()),
            ("refresh_token".to_string(), secret.refresh_token),
            ("client_id".to_string(), self.oauth.client_id.clone()),
            ("client_secret".to_string(), client_secret),
        ];
        let tokens = self
            .post_tokens(integration_id, &self.oauth.token_uri, form)
            .await?;
        // A refresh does NOT rotate the refresh token, so the store is untouched.
        self.cache_token(
            integration_id,
            tokens.access_token.clone(),
            tokens.expires_in,
        );
        self.set_health(integration_id, IntegrationHealth::Connected);
        // Read the expiry back out of the cache rather than recomputing it, so
        // there is exactly one place that decides when this token dies.
        let expires_at = self
            .cached_valid(integration_id)
            .map_or_else(|| (self.now)(), |(_, expires_at)| expires_at);
        Ok((tokens.access_token, expires_at))
    }

    pub(crate) async fn revoke(&self, integration_id: &str) -> Result<(), TokenError> {
        let gate = self.operation_gate(integration_id);
        let _turn = gate.lock().await;
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
        self.cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(integration_id);
        self.health
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(integration_id);
        if existed {
            Ok(())
        } else {
            Err(TokenError::NotFound)
        }
    }

    /// The integrations that have stored credentials. Ids only: presence,
    /// never token values.
    #[must_use]
    pub(crate) fn integration_ids(&self) -> Vec<String> {
        self.store.integration_ids()
    }

    pub(crate) fn health(&self, integration_id: &str) -> Option<IntegrationHealth> {
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
        let tokens =
            classify_token_response(response.status, &response.body).map_err(
                |error| match error {
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
                },
            )?;
        // `expires_in` is `#[serde(default)]`, so absent and zero arrive here
        // identically -- and either way the response is asserting a token that
        // has already expired. Caching it means `cached_valid` misses forever
        // and every single call refreshes; the token itself is usually fine, so
        // nothing else would ever surface the fault. Refuse it by name instead.
        //
        // Deliberately NOT bounded by `REFRESH_SKEW_SECONDS`: a token with, say,
        // 30 seconds left is legitimate and refreshing it on next use is the
        // correct, tested behaviour. Only a zero lifetime is nonsense.
        if tokens.expires_in == 0 {
            let detail =
                "token endpoint returned no usable expires_in (absent or zero)".to_string();
            self.set_health(integration_id, IntegrationHealth::Error(detail.clone()));
            return Err(TokenError::Malformed(detail));
        }
        Ok(tokens)
    }

    /// The per-integration operation gate, created on first use.
    fn operation_gate(&self, integration_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut gates = self
            .operation_gates
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Arc::clone(
            gates
                .entry(integration_id.to_string())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
        )
    }

    fn cached_valid(&self, integration_id: &str) -> Option<(String, DateTime<Utc>)> {
        let now = (self.now)();
        let cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = cache.get(integration_id)?;
        if now + Duration::seconds(REFRESH_SKEW_SECONDS) < entry.expires_at {
            Some((entry.access_token.clone(), entry.expires_at))
        } else {
            None
        }
    }

    fn cache_token(&self, integration_id: &str, access_token: String, expires_in: u64) {
        let now = (self.now)();
        let expires_at = i64::try_from(expires_in)
            .ok()
            .and_then(Duration::try_seconds)
            .and_then(|duration| now.checked_add_signed(duration))
            .unwrap_or(DateTime::<Utc>::MAX_UTC);
        self.cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                integration_id.to_string(),
                CachedToken {
                    access_token,
                    expires_at,
                },
            );
    }

    fn set_health(&self, integration_id: &str, health: IntegrationHealth) {
        self.health
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(integration_id.to_string(), health);
    }

    // IntegrationStore holds a std Mutex across disk I/O, so every store call
    // runs on the blocking pool and receives its own Arc clone.
    async fn store_get(
        &self,
        integration_id: String,
    ) -> Result<Option<IntegrationSecret>, TokenError> {
        let store = Arc::clone(&self.store);
        tokio::task::spawn_blocking(move || store.get(&integration_id))
            .await
            .map_err(|error| TokenError::Internal(error.to_string()))
    }

    async fn store_put(
        &self,
        integration_id: String,
        secret: IntegrationSecret,
    ) -> Result<(), TokenError> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::egress::FetchResponse;
    use crate::oauth::transport::{OAuthFuture, TransportError};
    use std::collections::VecDeque;
    use std::time::Duration as StdDuration;
    use tokio::sync::Notify;

    type Form = Vec<(String, String)>;
    type Call = (String, Form);

    struct FakeTransport {
        responses: Mutex<VecDeque<Result<FetchResponse, TransportError>>>,
        calls: Mutex<Vec<Call>>,
    }

    impl FakeTransport {
        fn new(responses: Vec<Result<FetchResponse, TransportError>>) -> Arc<Self> {
            Arc::new(Self {
                responses: Mutex::new(responses.into()),
                calls: Mutex::new(Vec::new()),
            })
        }
        fn calls(&self) -> Vec<Call> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl OAuthTransport for FakeTransport {
        fn post_form(
            &self,
            url: String,
            form: Form,
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

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum PausedOperation {
        Exchange,
        Refresh,
        Revoke,
    }

    struct ControlledTransport {
        paused: PausedOperation,
        started: Arc<Notify>,
        release: Arc<Notify>,
        calls: Mutex<Vec<Call>>,
    }

    impl ControlledTransport {
        fn new(paused: PausedOperation) -> Arc<Self> {
            Arc::new(Self {
                paused,
                started: Arc::new(Notify::new()),
                release: Arc::new(Notify::new()),
                calls: Mutex::new(Vec::new()),
            })
        }

        async fn wait_until_paused(&self) {
            self.started.notified().await;
        }

        fn release(&self) {
            self.release.notify_one();
        }
    }

    impl OAuthTransport for ControlledTransport {
        fn post_form(
            &self,
            url: String,
            form: Form,
        ) -> OAuthFuture<'_, Result<FetchResponse, TransportError>> {
            self.calls.lock().unwrap().push((url.clone(), form.clone()));
            let grant_type = form
                .iter()
                .find_map(|(key, value)| (key == "grant_type").then_some(value.as_str()));
            let operation = if url.contains("revoke") {
                Some(PausedOperation::Revoke)
            } else {
                match grant_type {
                    Some("authorization_code") => Some(PausedOperation::Exchange),
                    Some("refresh_token") => Some(PausedOperation::Refresh),
                    _ => None,
                }
            };
            let should_pause = operation == Some(self.paused);
            let paused = self.paused;
            let started = Arc::clone(&self.started);
            let release = Arc::clone(&self.release);
            Box::pin(async move {
                if should_pause {
                    started.notify_one();
                    release.notified().await;
                }
                match operation {
                    Some(PausedOperation::Exchange) if paused == PausedOperation::Refresh => {
                        ok(r#"{"access_token":"first","expires_in":30,"refresh_token":"old-rt"}"#)
                    }
                    Some(PausedOperation::Exchange) => ok(
                        r#"{"access_token":"exchanged","expires_in":3600,"refresh_token":"new-rt"}"#,
                    ),
                    Some(PausedOperation::Refresh) => {
                        ok(r#"{"access_token":"refreshed","expires_in":3600}"#)
                    }
                    Some(PausedOperation::Revoke) => ok(""),
                    None => panic!("unexpected controlled transport call: {url} {form:?}"),
                }
            })
        }
    }

    #[allow(clippy::unnecessary_wraps)] // Matches the fake's queued result type.
    fn ok(body: &str) -> Result<FetchResponse, TransportError> {
        Ok(FetchResponse {
            status: 200,
            body: body.as_bytes().to_vec(),
        })
    }
    #[allow(clippy::unnecessary_wraps)] // Matches the fake's queued result type.
    fn status(code: u16, body: &str) -> Result<FetchResponse, TransportError> {
        Ok(FetchResponse {
            status: code,
            body: body.as_bytes().to_vec(),
        })
    }

    fn store() -> (tempfile::TempDir, Arc<IntegrationStore>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(crate::secrets::SECRETS_STORE_FILE);
        let store = Arc::new(
            IntegrationStore::open(path, crate::secrets::SecretsKey::from_bytes([9u8; 32]))
                .expect("open store"),
        );
        (dir, store)
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

    #[test]
    fn oauth_fixture_directory_is_removed_when_its_guard_drops() {
        let (dir, store) = store();
        let path = dir.path().to_path_buf();
        let manager = manager(store, FakeTransport::new(vec![]), Utc::now());

        drop(manager);
        assert!(path.exists(), "the guard must own the fixture directory");
        drop(dir);
        assert!(!path.exists(), "dropping the guard must clean the fixture");
    }

    #[tokio::test]
    async fn exchange_persists_refresh_token_and_reports_connected() {
        let (_dir, store) = store();
        let transport = FakeTransport::new(vec![ok(
            r#"{"access_token":"at","expires_in":3600,"refresh_token":"rt"}"#,
        )]);
        let manager = manager(Arc::clone(&store), transport.clone(), Utc::now());

        manager
            .exchange_code("google-primary", "auth-code", "verifier-abc")
            .await
            .expect("exchange");

        let stored = tokio::task::spawn_blocking({
            let store = Arc::clone(&store);
            move || store.get("google-primary")
        })
        .await
        .unwrap();
        assert_eq!(stored.expect("stored").refresh_token, "rt");
        assert_eq!(
            manager.health("google-primary"),
            Some(IntegrationHealth::Connected)
        );

        let call = &transport.calls()[0];
        assert!(
            call.1
                .iter()
                .any(|(k, v)| k == "grant_type" && v == "authorization_code")
        );
        assert!(
            call.1
                .iter()
                .any(|(k, v)| k == "code_verifier" && v == "verifier-abc")
        );
    }

    #[tokio::test]
    async fn cached_access_token_is_reused_until_near_expiry() {
        let (_dir, store) = store();
        // Only ONE token response is queued: a second network call would panic.
        let transport = FakeTransport::new(vec![ok(
            r#"{"access_token":"at","expires_in":3600,"refresh_token":"rt"}"#,
        )]);
        let now = Utc::now();
        let manager = manager(store, transport, now);
        manager
            .exchange_code("id", "code", "v")
            .await
            .expect("exchange");

        // Well before expiry: served from cache, no second transport call.
        assert_eq!(
            manager
                .access_token_with_expiry("id")
                .await
                .expect("cached"),
            ("at".to_string(), now + Duration::seconds(3600))
        );
    }

    #[tokio::test]
    async fn maximum_token_lifetime_saturates_expiry_without_panicking() {
        let (_dir, store) = store();
        let transport = FakeTransport::new(vec![ok(
            r#"{"access_token":"at","expires_in":18446744073709551615,"refresh_token":"rt"}"#,
        )]);
        let now = DateTime::parse_from_rfc3339("2026-09-19T12:00:00Z")
            .expect("fixed time")
            .with_timezone(&Utc);
        let manager = manager(store, transport, now);

        manager
            .exchange_code("id", "code", "verifier")
            .await
            .expect("exchange");

        assert_eq!(
            manager
                .access_token_with_expiry("id")
                .await
                .expect("cached"),
            ("at".to_string(), DateTime::<Utc>::MAX_UTC)
        );
    }

    /// A refresh stampede: many callers want the same integration's token at the
    /// same instant, which is the ordinary case once one provider refresher serves
    /// several devices. Exactly ONE refresh must reach the token endpoint and every
    /// caller must get that token.
    ///
    /// The fake is queued with a single refresh response, so a second call to the
    /// endpoint panics the task that makes it -- which is what this asserts against.
    /// The race is real rather than theoretical: `access_token` awaits `store_get`,
    /// a `spawn_blocking` hop, so without coalescing every task passes the cache
    /// check, yields there together, and then posts.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_callers_share_one_refresh_instead_of_stampeding() {
        let (_dir, store) = store();
        let transport = FakeTransport::new(vec![
            ok(r#"{"access_token":"first","expires_in":30,"refresh_token":"rt"}"#),
            ok(r#"{"access_token":"refreshed","expires_in":3600}"#),
        ]);
        let now = Utc::now();
        let manager = Arc::new(manager(store, transport.clone(), now));
        manager
            .exchange_code("id", "code", "v")
            .await
            .expect("exchange");

        // Eight callers, all inside the 60s skew window, all racing.
        let mut handles = Vec::new();
        for _ in 0..8 {
            let manager = Arc::clone(&manager);
            handles.push(tokio::spawn(
                async move { manager.access_token("id").await },
            ));
        }
        for handle in handles {
            let token = handle
                .await
                .expect("no caller panicked -- a second refresh would exhaust the fake")
                .expect("every caller gets a token");
            assert_eq!(token, "refreshed");
        }

        // One exchange + exactly one refresh.
        assert_eq!(
            transport.calls().len(),
            2,
            "expected one exchange and one refresh, got {:?}",
            transport
                .calls()
                .iter()
                .map(|call| call.0.clone())
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn revoke_waits_for_in_flight_refresh_then_clears_all_local_state() {
        let (_dir, store) = store();
        let transport = ControlledTransport::new(PausedOperation::Refresh);
        let now = Utc::now();
        let manager = Arc::new(manager(Arc::clone(&store), transport.clone(), now));
        manager
            .exchange_code("id", "code", "verifier")
            .await
            .expect("initial exchange");

        let refresh = tokio::spawn({
            let manager = Arc::clone(&manager);
            async move { manager.access_token("id").await }
        });
        transport.wait_until_paused().await;
        let mut revoke = tokio::spawn({
            let manager = Arc::clone(&manager);
            async move { manager.revoke("id").await }
        });
        let early_revoke = tokio::time::timeout(StdDuration::from_millis(50), &mut revoke).await;
        transport.release();

        assert_eq!(
            refresh.await.expect("refresh task").expect("refresh"),
            "refreshed"
        );
        match early_revoke {
            Ok(result) => result.expect("revoke task").expect("revoke"),
            Err(_) => revoke.await.expect("revoke task").expect("revoke"),
        }
        let stored = tokio::task::spawn_blocking({
            let store = Arc::clone(&store);
            move || store.get("id")
        })
        .await
        .expect("store read");
        assert_eq!(stored, None, "revoke must win after the earlier refresh");
        assert!(matches!(
            manager.access_token("id").await,
            Err(TokenError::NotFound)
        ));
        assert_eq!(manager.health("id"), None);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cached_vending_waits_for_in_flight_revoke() {
        let (_dir, store) = store();
        let transport = ControlledTransport::new(PausedOperation::Revoke);
        let manager = Arc::new(manager(store, transport.clone(), Utc::now()));
        manager
            .exchange_code("id", "code", "verifier")
            .await
            .expect("initial exchange");

        let revoke = tokio::spawn({
            let manager = Arc::clone(&manager);
            async move { manager.revoke("id").await }
        });
        transport.wait_until_paused().await;
        let mut vend = tokio::spawn({
            let manager = Arc::clone(&manager);
            async move { manager.access_token_with_expiry("id").await }
        });

        tokio::time::timeout(StdDuration::from_millis(50), &mut vend)
            .await
            .expect_err("cached vending must wait behind an active revoke");
        transport.release();

        revoke.await.expect("revoke task").expect("revoke");
        assert!(matches!(
            vend.await.expect("vend task"),
            Err(TokenError::NotFound)
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn revoke_waits_for_earlier_exchange_and_removes_its_grant() {
        let (_dir, store) = store();
        store
            .put(
                "id".to_string(),
                IntegrationSecret {
                    provider: "google".to_string(),
                    refresh_token: "old-rt".to_string(),
                    client_secret: None,
                    scopes: vec![],
                    obtained_at: 1,
                },
            )
            .expect("seed old grant");
        let transport = ControlledTransport::new(PausedOperation::Exchange);
        let manager = Arc::new(manager(Arc::clone(&store), transport.clone(), Utc::now()));

        let exchange = tokio::spawn({
            let manager = Arc::clone(&manager);
            async move { manager.exchange_code("id", "code", "verifier").await }
        });
        transport.wait_until_paused().await;
        let mut revoke = tokio::spawn({
            let manager = Arc::clone(&manager);
            async move { manager.revoke("id").await }
        });
        let early_revoke = tokio::time::timeout(StdDuration::from_millis(50), &mut revoke).await;
        transport.release();

        exchange.await.expect("exchange task").expect("exchange");
        match early_revoke {
            Ok(result) => result.expect("revoke task").expect("revoke"),
            Err(_) => revoke.await.expect("revoke task").expect("revoke"),
        }
        let stored = tokio::task::spawn_blocking({
            let store = Arc::clone(&store);
            move || store.get("id")
        })
        .await
        .expect("store read");
        assert_eq!(stored, None, "revoke must win after the earlier exchange");
        assert!(matches!(
            manager.access_token("id").await,
            Err(TokenError::NotFound)
        ));
        assert_eq!(manager.health("id"), None);
    }

    /// A response with no usable `expires_in` would cache a token that
    /// `cached_valid` can never accept, so every later call refreshes forever
    /// while the token itself works fine. Refused by name rather than cached.
    #[tokio::test]
    async fn a_token_response_without_a_usable_expires_in_is_refused() {
        for body in [
            r#"{"access_token":"at","refresh_token":"rt"}"#,
            r#"{"access_token":"at","expires_in":0,"refresh_token":"rt"}"#,
        ] {
            let (_dir, store) = store();
            let transport = FakeTransport::new(vec![ok(body)]);
            let manager = manager(store, transport, Utc::now());
            let error = manager
                .exchange_code("id", "code", "v")
                .await
                .expect_err("a zero-lifetime token must be refused");
            assert!(
                matches!(&error, TokenError::Malformed(detail) if detail.contains("expires_in")),
                "unexpected error for {body}: {error}"
            );
            assert_eq!(
                manager.health("id"),
                Some(IntegrationHealth::Error(
                    "token endpoint returned no usable expires_in (absent or zero)".to_string()
                ))
            );
        }
    }

    /// Rotating the Google client secret must take effect for integrations that
    /// already authorized. With the stored copy winning, it did not: the refresh
    /// kept sending the retired secret, and the resulting health string pointed
    /// at the wrong cause.
    #[tokio::test]
    async fn a_rotated_client_secret_is_used_for_an_existing_integration() {
        let (_dir, store) = store();
        let transport = FakeTransport::new(vec![
            ok(r#"{"access_token":"first","expires_in":30,"refresh_token":"rt"}"#),
            ok(r#"{"access_token":"second","expires_in":3600}"#),
        ]);
        let now = Utc::now();

        let original = GoogleOAuthConfig {
            client_secret: "original-secret".to_string(),
            ..GoogleOAuthConfig::default()
        };
        let manager = TokenManager::with_clock(
            Arc::clone(&store),
            transport.clone(),
            original,
            Arc::new(move || now),
        );
        manager
            .exchange_code("id", "code", "v")
            .await
            .expect("exchange");

        // A second manager over the SAME store, configured with the rotated
        // secret: this is what a restart after rotation looks like.
        let rotated = GoogleOAuthConfig {
            client_secret: "rotated-secret".to_string(),
            ..GoogleOAuthConfig::default()
        };
        let manager =
            TokenManager::with_clock(store, transport.clone(), rotated, Arc::new(move || now));
        manager.access_token("id").await.expect("refresh");

        let refresh_form = &transport.calls()[1].1;
        assert!(
            refresh_form
                .iter()
                .any(|(key, value)| key == "client_secret" && value == "rotated-secret"),
            "refresh must use the rotated secret, got {refresh_form:?}"
        );
    }

    #[tokio::test]
    async fn near_expiry_triggers_a_refresh() {
        let (_dir, store) = store();
        let transport = FakeTransport::new(vec![
            ok(r#"{"access_token":"first","expires_in":30,"refresh_token":"rt"}"#),
            ok(r#"{"access_token":"second","expires_in":3600}"#),
        ]);
        let now = Utc::now();
        let manager = manager(store, transport.clone(), now);
        manager
            .exchange_code("id", "code", "v")
            .await
            .expect("exchange");

        // expires_in 30s is inside the 60s skew window, so this refreshes.
        assert_eq!(manager.access_token("id").await.expect("refresh"), "second");
        let refresh_call = &transport.calls()[1];
        assert!(
            refresh_call
                .1
                .iter()
                .any(|(k, v)| k == "grant_type" && v == "refresh_token")
        );
        assert!(
            refresh_call
                .1
                .iter()
                .any(|(k, v)| k == "refresh_token" && v == "rt")
        );
    }

    #[tokio::test]
    async fn invalid_grant_on_refresh_sets_needs_reconnect() {
        let (_dir, store) = store();
        let transport = FakeTransport::new(vec![
            ok(r#"{"access_token":"first","expires_in":10,"refresh_token":"rt"}"#),
            status(400, r#"{"error":"invalid_grant"}"#),
        ]);
        let manager = manager(store, transport, Utc::now());
        manager
            .exchange_code("id", "code", "v")
            .await
            .expect("exchange");

        assert!(matches!(
            manager.access_token("id").await,
            Err(TokenError::NeedsReconnect)
        ));
        assert_eq!(
            manager.health("id"),
            Some(IntegrationHealth::NeedsReconnect)
        );
    }

    #[tokio::test]
    async fn transport_failure_on_refresh_sets_error_health() {
        let (_dir, store) = store();
        let transport = FakeTransport::new(vec![
            ok(r#"{"access_token":"first","expires_in":10,"refresh_token":"rt"}"#),
            Err(TransportError("connection reset".to_string())),
        ]);
        let manager = manager(store, transport, Utc::now());
        manager
            .exchange_code("id", "code", "v")
            .await
            .expect("exchange");

        assert!(matches!(
            manager.access_token("id").await,
            Err(TokenError::Transport(_))
        ));
        assert!(matches!(
            manager.health("id"),
            Some(IntegrationHealth::Error(_))
        ));
    }

    #[tokio::test]
    async fn revoke_clears_the_stored_secret_and_calls_the_endpoint() {
        let (_dir, store) = store();
        let transport = FakeTransport::new(vec![
            ok(r#"{"access_token":"at","expires_in":3600,"refresh_token":"rt"}"#),
            ok(""), // revoke endpoint 200
        ]);
        let manager = manager(Arc::clone(&store), transport.clone(), Utc::now());
        manager
            .exchange_code("id", "code", "v")
            .await
            .expect("exchange");

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

    #[test]
    fn cached_token_debug_redacts_the_access_token() {
        let cached = CachedToken {
            access_token: "access-secret".to_string(),
            expires_at: Utc::now(),
        };

        let debug = format!("{cached:?}");
        assert!(!debug.contains("access-secret"));
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn google_oauth_config_debug_redacts_the_client_secret() {
        let config = GoogleOAuthConfig {
            client_id: "public-client-id".to_string(),
            client_secret: "private-client-secret".to_string(),
            ..GoogleOAuthConfig::default()
        };

        let debug = format!("{config:?}");
        assert!(!debug.contains("private-client-secret"));
        assert!(debug.contains("public-client-id"));
    }
}
