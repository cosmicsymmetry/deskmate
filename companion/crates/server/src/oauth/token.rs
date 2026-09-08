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
        let cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = cache.get(integration_id)?;
        if now + Duration::seconds(REFRESH_SKEW_SECONDS) < entry.expires_at {
            Some(entry.access_token.clone())
        } else {
            None
        }
    }

    fn cache_token(&self, integration_id: &str, access_token: String, expires_in: u64) {
        let expires_in = i64::try_from(expires_in).unwrap_or(i64::MAX);
        let expires_at = (self.now)()
            .checked_add_signed(Duration::seconds(expires_in))
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
        let store = store();
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
        let store = store();
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
        let store = store();
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
        let store = store();
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
