//! Operator login route and the integration-runtime seam it uses. Task 6 adds
//! the consent, callback, revoke, and pending-state behavior to this module.

use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::request::Parts;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use chrono::{Duration, Utc};

use super::session::{SessionSigner, set_cookie_header};
use super::token::{GoogleOAuthConfig, TokenManager};
use crate::ServerState;
use crate::auth::bearer_token;

const SESSION_TTL: Duration = Duration::hours(12);

/// Shared OAuth runtime. Task 5 needs its session signer; Task 6 extends this
/// same runtime with the provider configuration and pending consent state.
pub struct IntegrationRuntime {
    token_manager: Arc<TokenManager>,
    sessions: SessionSigner,
    // Retained now so Task 6 can add consent URL construction without changing
    // this constructor; the leading underscore keeps the standalone Task 5
    // build warning-free until those routes consume it.
    _oauth: GoogleOAuthConfig,
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
            _oauth: oauth,
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
}

pub(crate) fn routes() -> Router<ServerState> {
    Router::new().route("/v1/session/login", post(login))
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
