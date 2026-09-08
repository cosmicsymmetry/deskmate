//! OAuth consent, callback, and revoke routes plus the operator login route
//! (spec §3). Route handlers stay thin; the `state`/PKCE stash and the auth-URL
//! construction live on `IntegrationRuntime` so they are unit-testable.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::request::Parts;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use url::Url;

use super::session::{OperatorAuthenticated, SessionSigner, set_cookie_header};
use super::token::{GoogleOAuthConfig, TokenError, TokenManager};
use crate::ServerState;
use crate::auth::bearer_token;
use crate::registry::constant_time_eq;

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

impl IntegrationRuntime {
    #[must_use]
    pub fn new(
        token_manager: Arc<TokenManager>,
        sessions: SessionSigner,
        oauth: GoogleOAuthConfig,
    ) -> Self {
        Self {
            token_manager,
            sessions,
            oauth,
            pending: Mutex::new(HashMap::new()),
        }
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
        let now = Utc::now();

        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pending.retain(|_, stashed| now - stashed.created_at <= PENDING_TTL);
        pending.insert(
            state.clone(),
            StashedAuth {
                pending: PendingAuth {
                    integration_id: integration_id.to_string(),
                    code_verifier: pkce.verifier,
                    sid: sid.to_string(),
                },
                created_at: now,
            },
        );
        drop(pending);

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
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        // Search every candidate and compare every byte. Avoid HashMap::remove(state):
        // its ordinary equality can short-circuit and disclose a matching prefix.
        let mut matched_state = None;
        for candidate in pending.keys() {
            if constant_time_eq(candidate.as_bytes(), state.as_bytes()) {
                matched_state = Some(candidate.clone());
            }
        }

        let stashed = pending.remove(&matched_state?)?;
        if now - stashed.created_at > PENDING_TTL {
            return None;
        }
        Some(stashed.pending)
    }
}

enum RouteError {
    Unauthorized,
    BadRequest(String),
    Upstream(String),
    NotConfigured,
}

impl IntoResponse for RouteError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthorized => StatusCode::UNAUTHORIZED.into_response(),
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message).into_response(),
            Self::Upstream(message) => (StatusCode::BAD_GATEWAY, message).into_response(),
            Self::NotConfigured => (
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
async fn login(State(state): State<ServerState>, parts: Parts) -> Response {
    match bearer_token(&parts) {
        Some(token) if state.verify_admin_token(token) => {}
        _ => return StatusCode::UNAUTHORIZED.into_response(),
    }

    let Some(runtime) = state.integrations() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "OAuth integrations are not configured on this server",
        )
            .into_response();
    };
    let sid = SessionSigner::new_sid();
    let cookie = runtime.sessions().mint(&sid, Utc::now(), SESSION_TTL);
    let Ok(header_value) = HeaderValue::from_str(&set_cookie_header(&cookie, SESSION_TTL)) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let mut response = StatusCode::NO_CONTENT.into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, header_value);
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
    let integration_id = query
        .integration_id
        .unwrap_or_else(|| DEFAULT_INTEGRATION_ID.to_string());
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
    let state_value = query
        .state
        .ok_or_else(|| RouteError::BadRequest("missing state".to_string()))?;
    let pending = runtime
        .take_pending(&state_value, Utc::now())
        .ok_or_else(|| RouteError::BadRequest("unknown or expired state".to_string()))?;
    if !constant_time_eq(pending.sid.as_bytes(), operator.sid.as_bytes()) {
        return Err(RouteError::Unauthorized);
    }
    if let Some(error) = query.error {
        return Err(RouteError::BadRequest(format!(
            "authorization was declined: {error}"
        )));
    }
    let code = query
        .code
        .ok_or_else(|| RouteError::BadRequest("missing code".to_string()))?;

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
    runtime
        .token_manager()
        .revoke(&id)
        .await
        .map_err(token_error_to_route)?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::egress::FetchResponse;
    use crate::oauth::transport::{OAuthFuture, OAuthTransport, TransportError};
    use url::Url;

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
        let token_manager = Arc::new(TokenManager::new(
            store,
            Arc::new(NoTransport),
            GoogleOAuthConfig::default(),
        ));
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
        let query: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(
            query.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
        assert!(query.contains_key("code_challenge"));
        assert_eq!(
            query.get("scope").map(String::as_str),
            Some("https://www.googleapis.com/auth/calendar.events.readonly")
        );
        assert_eq!(
            query.get("access_type").map(String::as_str),
            Some("offline")
        );
        assert_eq!(query.get("prompt").map(String::as_str), Some("consent"));
        assert!(query.contains_key("state"));
    }

    #[test]
    fn a_stashed_state_is_single_use() {
        let runtime = runtime();
        let url = runtime.start_consent("sid-1", "google-primary");
        let state = Url::parse(&url)
            .unwrap()
            .query_pairs()
            .into_owned()
            .find(|(k, _)| k == "state")
            .unwrap()
            .1;

        let first = runtime
            .take_pending(&state, Utc::now())
            .expect("first take must succeed");
        assert_eq!(first.integration_id, "google-primary");
        assert_eq!(first.sid, "sid-1");
        assert!(
            runtime.take_pending(&state, Utc::now()).is_none(),
            "state must be single-use"
        );
    }

    #[test]
    fn an_unknown_state_returns_none() {
        assert!(runtime().take_pending("never-issued", Utc::now()).is_none());
    }

    #[test]
    fn an_expired_stash_returns_none() {
        let runtime = runtime();
        let url = runtime.start_consent("sid-1", "google-primary");
        let state = Url::parse(&url)
            .unwrap()
            .query_pairs()
            .into_owned()
            .find(|(k, _)| k == "state")
            .unwrap()
            .1;
        let much_later = Utc::now() + Duration::seconds(601);
        assert!(runtime.take_pending(&state, much_later).is_none());
    }
}
