//! Operator session cookie: an HMAC-SHA256 token over `sid:exp`, keyed by the
//! existing `DESKMATE_ADMIN_TOKEN` (spec §1.2, §6). The stand-in for V3-deferred
//! accounts; it carries no privilege the admin token does not already carry.

use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use base64::prelude::{BASE64_URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Duration, Utc};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::ServerState;
use crate::auth::bearer_token;
use crate::registry::constant_time_eq;

type HmacSha256 = Hmac<Sha256>;

/// The `__Host-` prefix is not decoration. A browser refuses to accept a cookie
/// under this name unless it is `Secure`, has `Path=/`, and carries no `Domain`
/// attribute -- which this cookie already satisfies -- and, crucially, it stops
/// any sibling host under the registrable domain from setting a `Domain`-scoped
/// cookie of the same name. Without the prefix such a cookie would be sent
/// alongside the real one, and `session_cookie_value`'s first-match scan of the
/// `Cookie` header would happily pick whichever came first.
pub const SESSION_COOKIE: &str = "__Host-deskmate_session";

pub struct SessionSigner {
    key: Vec<u8>,
}

pub struct SessionClaims {
    pub sid: String,
    pub exp: i64,
}

impl SessionSigner {
    #[must_use]
    pub fn from_admin_token(token: &str) -> Self {
        Self {
            key: token.as_bytes().to_vec(),
        }
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
        Some(SessionClaims {
            sid: sid.to_string(),
            exp,
        })
    }

    /// A random session id embedded in the cookie so a `state` stash can be bound
    /// to the operator session that started it (spec §3).
    #[must_use]
    pub fn new_sid() -> String {
        use rand::RngCore as _;

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

/// Reads the session cookie value from the `Cookie` header, if present.
fn session_cookie_value(parts: &Parts) -> Option<String> {
    let header = parts
        .headers
        .get(axum::http::header::COOKIE)?
        .to_str()
        .ok()?;
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
        if let Some(token) = bearer_token(parts)
            && state.verify_admin_token(token)
        {
            return Ok(Self {
                sid: "bearer-admin".to_string(),
            });
        }
        // The state's own signer, not `integrations().sessions()`. Both are keyed
        // by the admin token and mint identical values, but the integration
        // runtime is absent on a deployment that configures no OAuth provider --
        // and reading the cookie through it meant no cookie was ever accepted
        // there, which would leave the browser companion permanently logged out.
        if let Some(value) = session_cookie_value(parts)
            && let Some(claims) = state.sessions().verify(&value, Utc::now())
        {
            return Ok(Self { sid: claims.sid });
        }
        Err(OperatorAuthError)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode, header};
    use axum::routing::get;
    use tower::ServiceExt as _;

    use crate::oauth::transport::EgressTransport;
    use crate::oauth::{GoogleOAuthConfig, IntegrationRuntime, TokenManager};
    use crate::{ServerState, app};

    use super::*;

    fn signer() -> SessionSigner {
        SessionSigner::from_admin_token("admin-token-value")
    }

    fn state_with_integrations() -> ServerState {
        let state = ServerState::in_memory();
        let directory = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(
            crate::secrets::IntegrationStore::open(
                directory.keep().join(crate::secrets::SECRETS_STORE_FILE),
                crate::secrets::SecretsKey::from_bytes([7u8; 32]),
            )
            .expect("store"),
        );
        let token_manager = Arc::new(TokenManager::new(
            store,
            Arc::new(EgressTransport),
            GoogleOAuthConfig::default(),
        ));
        state.set_integrations(Arc::new(IntegrationRuntime::new(
            token_manager,
            SessionSigner::from_admin_token("in-memory-admin-token"),
            GoogleOAuthConfig::default(),
        )));
        state
    }

    async fn operator_sid(operator: OperatorAuthenticated) -> String {
        operator.sid
    }

    fn protected_app(state: ServerState) -> Router {
        Router::new()
            .route("/operator-test", get(operator_sid))
            .with_state(state)
    }

    async fn assert_bare_unauthorized(response: axum::response::Response) {
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("read response body")
                .is_empty()
        );
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

    #[test]
    fn cookie_header_has_the_required_security_attributes() {
        let header = set_cookie_header("signed-value", Duration::hours(12));
        assert_eq!(
            header,
            "__Host-deskmate_session=signed-value; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=43200"
        );
    }

    #[tokio::test]
    async fn operator_gate_accepts_the_admin_bearer_token() {
        let response = protected_app(state_with_integrations())
            .oneshot(
                Request::builder()
                    .uri("/operator-test")
                    .header(header::AUTHORIZATION, "Bearer in-memory-admin-token")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("read response body"),
            "bearer-admin"
        );
    }

    #[tokio::test]
    async fn operator_gate_accepts_a_valid_session_cookie() {
        let state = state_with_integrations();
        let cookie = state.integrations().expect("integrations").sessions().mint(
            "cookie-sid",
            Utc::now(),
            Duration::hours(1),
        );
        let response = protected_app(state)
            .oneshot(
                Request::builder()
                    .uri("/operator-test")
                    .header(
                        header::COOKIE,
                        format!("other=1; {SESSION_COOKIE}={cookie}"),
                    )
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("read response body"),
            "cookie-sid"
        );
    }

    #[tokio::test]
    async fn operator_gate_rejects_absent_and_invalid_cookies_with_a_bare_401() {
        let state = state_with_integrations();
        let absent = protected_app(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/operator-test")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_bare_unauthorized(absent).await;

        let invalid = protected_app(state)
            .oneshot(
                Request::builder()
                    .uri("/operator-test")
                    .header(header::COOKIE, format!("{SESSION_COOKIE}=invalid"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_bare_unauthorized(invalid).await;
    }

    #[tokio::test]
    async fn login_exchanges_the_admin_bearer_for_a_signed_secure_cookie() {
        let response = app(state_with_integrations())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/session/login")
                    .header(header::AUTHORIZATION, "Bearer in-memory-admin-token")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let set_cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .expect("session cookie")
            .to_str()
            .expect("ASCII cookie");
        assert!(set_cookie.contains("; HttpOnly"));
        assert!(set_cookie.contains("; Secure"));
        assert!(set_cookie.contains("; SameSite=Lax"));
        assert!(set_cookie.contains("; Path=/"));
        assert!(set_cookie.contains("; Max-Age=43200"));

        let value = set_cookie
            .strip_prefix("__Host-deskmate_session=")
            .expect("session cookie name")
            .split(';')
            .next()
            .expect("cookie value");
        assert!(
            SessionSigner::from_admin_token("in-memory-admin-token")
                .verify(value, Utc::now())
                .is_some()
        );
    }

    #[tokio::test]
    async fn login_rejects_an_invalid_admin_token_with_a_bare_401() {
        let response = app(state_with_integrations())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/session/login")
                    .header(header::AUTHORIZATION, "Bearer wrong-admin-token")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_bare_unauthorized(response).await;
    }
}
