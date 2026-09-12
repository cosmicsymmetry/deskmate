//! OAuth consent, callback, and revoke routes plus the operator login route
//! (spec §3). Route handlers stay thin; the `state`/PKCE stash and the auth-URL
//! construction live on `IntegrationRuntime` so they are unit-testable.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::Router;
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::request::Parts;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use url::Url;

use super::session::{OperatorAuthenticated, SessionSigner, set_cookie_header};
use super::token::{GoogleOAuthConfig, TokenError, TokenManager};
use crate::ServerState;
use crate::auth::bearer_token;
use crate::registry::constant_time_eq;

const PENDING_TTL: Duration = Duration::seconds(600);
const SESSION_TTL: Duration = Duration::hours(12);
const DEFAULT_INTEGRATION_ID: &str = "google-primary";
/// Longest `integration_id` accepted from a caller. The id is a map key in three
/// long-lived maps (`pending`, the token cache, and health), and only `revoke`
/// ever removes an entry, so an unbounded id is an unbounded retention primitive
/// for anyone holding a session cookie. 64 bytes is far past any real id.
const MAX_INTEGRATION_ID_LEN: usize = 64;
/// Most simultaneously pending consent flows. One operator cannot legitimately
/// have more in flight than this within the 600 s TTL, and the sweep alone does
/// not bound the map between sweeps.
const MAX_PENDING_AUTHS: usize = 32;

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
    /// Returns `None` when too many consent flows are already pending, so the
    /// stash cannot be grown without bound by repeated calls.
    pub fn start_consent(&self, sid: &str, integration_id: &str) -> Option<String> {
        let state = super::pkce::generate_state();
        let pkce = super::pkce::generate_pkce();
        let now = Utc::now();

        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pending.retain(|_, stashed| now - stashed.created_at <= PENDING_TTL);
        if pending.len() >= MAX_PENDING_AUTHS {
            return None;
        }
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
        Some(url.into())
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
    /// A state the caller cannot retry out of -- today, a revoked or expired
    /// grant. Distinct from `BadRequest` because the request was fine.
    Conflict(String),
    Upstream(String),
    NotConfigured,
    Store(String),
}

impl IntoResponse for RouteError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthorized => StatusCode::UNAUTHORIZED.into_response(),
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message).into_response(),
            Self::Conflict(message) => (StatusCode::CONFLICT, message).into_response(),
            Self::Store(message) => (StatusCode::INTERNAL_SERVER_ERROR, message).into_response(),
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

/// Like [`token_error_to_route`], but a revoked grant is a 409 rather than a
/// 400: the producer's request was well-formed, and no amount of retrying will
/// fix a grant the owner revoked. The producer guide tells producers to stop
/// polling on this status and surface it to the operator.
fn vend_error_to_route(error: &TokenError) -> RouteError {
    match error {
        TokenError::NeedsReconnect => RouteError::Conflict(
            "authorization was revoked or expired; reconnect the integration".to_string(),
        ),
        TokenError::NotFound => RouteError::BadRequest("no such integration".to_string()),
        // Deliberately drops the provider's detail: a token-endpoint error body
        // can quote the request back, and this response goes to a producer.
        _ => RouteError::Upstream("the identity provider's token endpoint failed".to_string()),
    }
}

/// A producer credential, resolved to the integration it may vend for.
///
/// Separate from [`OperatorAuthenticated`] on purpose: an admin token must not
/// work here, or it would end up living in a producer's environment.
struct ProducerAuthenticated(String);

impl FromRequestParts<ServerState> for ProducerAuthenticated {
    type Rejection = RouteError;

    #[allow(clippy::unused_async_trait_impl)]
    async fn from_request_parts(
        parts: &mut Parts,
        state: &ServerState,
    ) -> Result<Self, Self::Rejection> {
        let presented = bearer_token(parts).ok_or(RouteError::Unauthorized)?;
        let integration_id = state
            .producer_credentials()
            .authenticate(presented)
            .ok_or(RouteError::Unauthorized)?;
        Ok(Self(integration_id))
    }
}

#[derive(Serialize)]
struct MintedCredentialResponse {
    integration_id: String,
    token: String,
}

#[derive(Serialize)]
struct VendedToken {
    access_token: String,
    expires_at: String,
}

/// Mints (or rotates) the producer credential for one integration.
async fn mint_producer(
    State(state): State<ServerState>,
    _operator: OperatorAuthenticated,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<MintedCredentialResponse>), RouteError> {
    validate_integration_id(&id)?;
    // The credential store holds a sync Mutex across disk I/O, so this must not
    // run on the async executor.
    let minted = tokio::task::spawn_blocking(move || state.producer_credentials().mint(&id))
        .await
        .map_err(|_| RouteError::Store("producer credential worker failed".to_string()))?
        .map_err(|error| RouteError::Store(error.to_string()))?;
    Ok((
        StatusCode::CREATED,
        Json(MintedCredentialResponse {
            integration_id: minted.integration_id,
            token: minted.token,
        }),
    ))
}

/// Revokes an integration's producer credential without touching the grant.
async fn revoke_producer(
    State(state): State<ServerState>,
    _operator: OperatorAuthenticated,
    Path(id): Path<String>,
) -> Result<StatusCode, RouteError> {
    validate_integration_id(&id)?;
    tokio::task::spawn_blocking(move || state.producer_credentials().revoke(&id))
        .await
        .map_err(|_| RouteError::Store("producer credential worker failed".to_string()))?
        .map_err(|error| RouteError::Store(error.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

/// Hands a producer the provider's own access token.
///
/// Takes no body and no query on purpose (spec §5): the integration record is
/// the only source of host and scope, so there is nothing a caller can supply
/// that would redirect the credential-bearing call.
async fn vend_token(
    State(state): State<ServerState>,
    ProducerAuthenticated(authenticated_id): ProducerAuthenticated,
    Path(id): Path<String>,
) -> Result<Json<VendedToken>, RouteError> {
    // Uniform 401 rather than 403: a distinguishable rejection would let a
    // caller enumerate which integration ids exist.
    if !constant_time_eq(authenticated_id.as_bytes(), id.as_bytes()) {
        return Err(RouteError::Unauthorized);
    }
    let runtime = state.integrations().ok_or(RouteError::NotConfigured)?;
    let (access_token, expires_at) = runtime
        .token_manager()
        .access_token_with_expiry(&authenticated_id)
        .await
        .map_err(|error| vend_error_to_route(&error))?;
    Ok(Json(VendedToken {
        access_token,
        expires_at: expires_at.to_rfc3339(),
    }))
}

pub(crate) fn routes() -> Router<ServerState> {
    Router::new()
        .route("/v1/session/login", post(login))
        .route("/v1/integrations/google", post(start_google))
        .route("/v1/integrations/google/callback", get(google_callback))
        .route("/v1/integrations/{id}/revoke", post(revoke_integration))
        .route(
            "/v1/integrations/{id}/producer",
            post(mint_producer).delete(revoke_producer),
        )
        .route("/v1/integrations/{id}/token", post(vend_token))
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

/// Bounds the one caller-supplied string that becomes a long-lived map key.
/// Restricting the charset as well keeps ids printable in logs and URLs and
/// leaves no room for a value that is technically valid UTF-8 but hostile to
/// read back.
fn validate_integration_id(value: &str) -> Result<(), RouteError> {
    if value.is_empty() || value.len() > MAX_INTEGRATION_ID_LEN {
        return Err(RouteError::BadRequest(format!(
            "integration_id must be 1..={MAX_INTEGRATION_ID_LEN} bytes"
        )));
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(RouteError::BadRequest(
            "integration_id may use only ASCII letters, digits, '-' and '_'".to_string(),
        ));
    }
    Ok(())
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
    validate_integration_id(&integration_id)?;
    let url = runtime
        .start_consent(&operator.sid, &integration_id)
        .ok_or_else(|| {
            RouteError::BadRequest(format!(
                "too many consent flows already pending (limit {MAX_PENDING_AUTHS}); \
                 finish or abandon one and retry"
            ))
        })?;
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
    // The producer credential goes FIRST, and unconditionally. It is scoped to
    // this integration and nothing else, so a credential that outlived the
    // grant could only fail -- and it would fail as a 401 loop, which reads
    // like a broken producer rather than the disconnection the operator just
    // performed. Ordering it first also means a grant revoke that errors (an
    // unknown id, a provider outage) still leaves no live credential behind,
    // matching `TokenManager::revoke`'s own rule that local state goes because
    // the operator asked to disconnect.
    let credential_state = state.clone();
    let credential_id = id.clone();
    tokio::task::spawn_blocking(move || {
        credential_state
            .producer_credentials()
            .revoke(&credential_id)
    })
    .await
    .map_err(|_| RouteError::Store("producer credential worker failed".to_string()))?
    .map_err(|error| RouteError::Store(error.to_string()))?;
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
        let url = runtime
            .start_consent("sid-1", "google-primary")
            .expect("under the pending limit");
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

    /// The single assertion that makes PKCE mean anything: the verifier we stashed
    /// must be the one whose challenge we sent to Google. Everything else about the
    /// flow can be correct and PKCE still be dead -- a stash holding an unrelated
    /// verifier produces a well-formed URL, a valid single-use `state`, and a
    /// permanent `invalid_grant` that only Google can see, in production.
    #[test]
    fn the_stashed_verifier_is_the_one_whose_challenge_was_sent_to_google() {
        let runtime = runtime();
        let url = runtime
            .start_consent("sid-1", "google-primary")
            .expect("under the pending limit");
        let parsed = Url::parse(&url).expect("valid url");
        let query: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        let state = query.get("state").expect("state is present");
        let challenge = query
            .get("code_challenge")
            .expect("code_challenge is present");

        let stashed = runtime
            .take_pending(state, Utc::now())
            .expect("the state we were handed is the state that was stashed");
        assert_eq!(
            &super::super::pkce::challenge_for(&stashed.code_verifier),
            challenge,
            "the stashed verifier must hash to the challenge sent to Google"
        );
    }

    /// `integration_id` is the one caller-supplied string that becomes a key in
    /// three long-lived maps, only one of which any code path ever removes from.
    /// It is bounded so a session-cookie holder cannot use it as a retention
    /// primitive.
    #[test]
    fn integration_id_is_bounded_in_length_and_charset() {
        assert!(validate_integration_id("google-primary").is_ok());
        assert!(validate_integration_id("a_b-9").is_ok());
        assert!(validate_integration_id(&"a".repeat(MAX_INTEGRATION_ID_LEN)).is_ok());

        for bad in [
            String::new(),
            "a".repeat(MAX_INTEGRATION_ID_LEN + 1),
            "has space".to_string(),
            "slash/path".to_string(),
            "dot.dot".to_string(),
            "unicode-\u{fffd}".to_string(),
        ] {
            assert!(
                validate_integration_id(&bad).is_err(),
                "{bad:?} must be refused"
            );
        }
    }

    /// The TTL sweep alone does not bound the pending map between sweeps: every
    /// stash lives 600 s, so a caller can accumulate them far faster than they
    /// expire. The count is what actually bounds it.
    #[test]
    fn pending_consent_flows_are_capped() {
        let runtime = runtime();
        for index in 0..MAX_PENDING_AUTHS {
            assert!(
                runtime.start_consent("sid-1", "google-primary").is_some(),
                "flow {index} should be admitted"
            );
        }
        assert!(
            runtime.start_consent("sid-1", "google-primary").is_none(),
            "the flow past the cap must be refused, not stashed"
        );
    }

    #[test]
    fn a_stashed_state_is_single_use() {
        let runtime = runtime();
        let url = runtime
            .start_consent("sid-1", "google-primary")
            .expect("under the pending limit");
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
        let url = runtime
            .start_consent("sid-1", "google-primary")
            .expect("under the pending limit");
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
