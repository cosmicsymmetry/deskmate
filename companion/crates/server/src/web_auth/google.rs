use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use base64::prelude::{BASE64_URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use url::Url;

use crate::ServerState;
use crate::identity::{IdentityError, normalize_email};
use crate::oauth::GoogleOAuthConfig;
use crate::oauth::transport::OAuthTransport;
use crate::registry::constant_time_eq;
use crate::web_auth::routes::{RouteError, ensure_failure_attempt_allowed, record_failure};
use crate::web_auth::{ClientIp, RateLimiter, set_session_cookie};

const PENDING_TTL: Duration = Duration::seconds(600);
const MAX_PENDING_AUTHS: usize = 32;
/// Google sign-in starts one client may make per ten minutes. Per client only because
/// `ClientIp` is the visitor's address; see `TrustedProxies`.
const STARTS_PER_CLIENT: usize = 10;
const STATE_COOKIE: &str = "__Host-deskmate_google";

struct PendingSignIn {
    verifier: String,
    nonce: String,
    created_at: DateTime<Utc>,
}

pub(crate) struct GoogleSignIn {
    config: GoogleOAuthConfig,
    transport: Arc<dyn OAuthTransport>,
    pending: Mutex<HashMap<String, PendingSignIn>>,
    start_limiter: RateLimiter,
}

impl GoogleSignIn {
    pub(crate) fn new(config: GoogleOAuthConfig, transport: Arc<dyn OAuthTransport>) -> Self {
        Self {
            config,
            transport,
            pending: Mutex::new(HashMap::new()),
            start_limiter: RateLimiter::new(STARTS_PER_CLIENT, std::time::Duration::from_secs(600)),
        }
    }

    fn start(&self, now: DateTime<Utc>) -> Result<(String, String), StartError> {
        let state = crate::oauth::pkce::generate_state();
        let nonce = crate::oauth::pkce::generate_state();
        let pkce = crate::oauth::pkce::generate_pkce();
        let mut url = Url::parse(&self.config.auth_uri).map_err(|_| StartError::InvalidUrl)?;
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pending.retain(|_, entry| now - entry.created_at < PENDING_TTL);
        if pending.len() >= MAX_PENDING_AUTHS {
            return Err(StartError::AtCapacity);
        }
        pending.insert(
            state.clone(),
            PendingSignIn {
                verifier: pkce.verifier,
                nonce: nonce.clone(),
                created_at: now,
            },
        );
        drop(pending);

        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.config.client_id)
            .append_pair("redirect_uri", &self.config.redirect_uri)
            .append_pair("scope", "openid email")
            .append_pair("code_challenge", &pkce.challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("nonce", &nonce)
            .append_pair("state", &state);
        Ok((url.into(), state))
    }

    fn take_pending(&self, state: &str, now: DateTime<Utc>) -> Option<PendingSignIn> {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut matched = None;
        for candidate in pending.keys() {
            if constant_time_eq(candidate.as_bytes(), state.as_bytes()) {
                matched = Some(candidate.clone());
            }
        }
        let matched = matched?;
        let entry = pending.remove(&matched)?;
        (now - entry.created_at < PENDING_TTL).then_some(entry)
    }

    async fn exchange_code(
        &self,
        code: &str,
        pending: &PendingSignIn,
        now: DateTime<Utc>,
    ) -> Result<GoogleClaims, ExchangeError> {
        let form = vec![
            ("grant_type".to_string(), "authorization_code".to_string()),
            ("code".to_string(), code.to_string()),
            ("redirect_uri".to_string(), self.config.redirect_uri.clone()),
            ("client_id".to_string(), self.config.client_id.clone()),
            (
                "client_secret".to_string(),
                self.config.client_secret.clone(),
            ),
            ("code_verifier".to_string(), pending.verifier.clone()),
        ];
        let response = self
            .transport
            .post_form(self.config.token_uri.clone(), form)
            .await
            .map_err(|_| ExchangeError::Transport)?;
        if !(200..300).contains(&response.status) {
            return Err(ExchangeError::Provider);
        }
        let response: TokenResponse =
            serde_json::from_slice(&response.body).map_err(|_| ExchangeError::Provider)?;
        claims_from_id_token(
            &response.id_token,
            &self.config.client_id,
            &pending.nonce,
            now,
        )
        .map_err(ExchangeError::Claims)
    }
}

#[derive(Debug)]
enum StartError {
    AtCapacity,
    InvalidUrl,
}

#[derive(Debug)]
enum ExchangeError {
    Transport,
    Provider,
    Claims(&'static str),
}

#[derive(Deserialize)]
struct TokenResponse {
    id_token: String,
}

pub(crate) struct GoogleClaims {
    pub(crate) sub: String,
    pub(crate) email: String,
    pub(crate) email_authoritative: bool,
}

/// Reads identity claims from the ID token returned directly by Google's token
/// endpoint. There is deliberately no signature verification here: `OpenID`
/// Connect Core 3.1.3.7 permits relying on TLS server validation when an ID
/// token is received directly from the token endpoint. The request reaches only
/// `oauth2.googleapis.com` through the server's guarded OAuth POST transport.
pub(crate) fn claims_from_id_token(
    id_token: &str,
    client_id: &str,
    nonce: &str,
    now: DateTime<Utc>,
) -> Result<GoogleClaims, &'static str> {
    let mut parts = id_token.split('.');
    let _header = parts.next().ok_or("invalid-token")?;
    let payload = parts.next().ok_or("invalid-token")?;
    let _signature = parts.next().ok_or("invalid-token")?;
    if parts.next().is_some() {
        return Err("invalid-token");
    }
    let payload = BASE64_URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| "invalid-token")?;
    let claims: serde_json::Value =
        serde_json::from_slice(&payload).map_err(|_| "invalid-token")?;

    let issuer = claims
        .get("iss")
        .and_then(serde_json::Value::as_str)
        .ok_or("invalid-token")?;
    if !matches!(
        issuer,
        "https://accounts.google.com" | "accounts.google.com"
    ) {
        return Err("invalid-token");
    }
    let audience_matches = match claims.get("aud") {
        Some(serde_json::Value::String(audience)) => audience == client_id,
        Some(serde_json::Value::Array(audiences)) => audiences
            .iter()
            .any(|audience| audience.as_str() == Some(client_id)),
        _ => false,
    };
    if !audience_matches {
        return Err("invalid-token");
    }
    let multiple_audiences = claims
        .get("aud")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|audiences| audiences.len() > 1);
    if (multiple_audiences || claims.get("azp").is_some())
        && claims.get("azp").and_then(serde_json::Value::as_str) != Some(client_id)
    {
        return Err("invalid-token");
    }
    if !claims
        .get("nonce")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|claim| constant_time_eq(claim.as_bytes(), nonce.as_bytes()))
    {
        return Err("invalid-token");
    }
    let expires_at = claims
        .get("exp")
        .and_then(serde_json::Value::as_i64)
        .ok_or("invalid-token")?;
    if expires_at <= now.timestamp() {
        return Err("invalid-token");
    }
    let email_verified = match claims.get("email_verified") {
        Some(serde_json::Value::Bool(value)) => *value,
        Some(serde_json::Value::String(value)) => value == "true",
        _ => false,
    };
    if !email_verified {
        return Err("email-unverified");
    }
    let sub = claims
        .get("sub")
        .and_then(serde_json::Value::as_str)
        .filter(|sub| !sub.is_empty())
        .ok_or("invalid-token")?
        .to_string();
    let email = claims
        .get("email")
        .and_then(serde_json::Value::as_str)
        .and_then(normalize_email)
        .ok_or("invalid-token")?;
    // Google can verify current mailbox ownership for Gmail and Workspace.
    // A third-party address may have changed owners since Google verified it.
    let email_authoritative = email.ends_with("@gmail.com")
        || claims
            .get("hd")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|domain| !domain.is_empty());
    Ok(GoogleClaims {
        sub,
        email,
        email_authoritative,
    })
}

pub(super) fn routes() -> Router<ServerState> {
    Router::new()
        .route("/v1/app/auth/google/start", get(start_google))
        .route("/v1/app/auth/google/callback", get(google_callback))
}

async fn start_google(State(state): State<ServerState>, client: ClientIp) -> Response {
    let Some(google) = state.google_sign_in() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if let Err(retry) = google
        .start_limiter
        .check_and_record(&client.0.to_string(), std::time::Instant::now())
    {
        return RouteError::rate_limited(retry).into_response();
    }
    match google.start(Utc::now()) {
        Ok((url, state_value)) => (
            [(
                header::SET_COOKIE,
                state_cookie(&state_value, PENDING_TTL.num_seconds()),
            )],
            Redirect::to(&url),
        )
            .into_response(),
        Err(StartError::AtCapacity) => StatusCode::TOO_MANY_REQUESTS.into_response(),
        Err(StartError::InvalidUrl) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[derive(Deserialize)]
struct CallbackQuery {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
}

enum SignInResult {
    SignedIn(String),
    SignupsClosed,
    EmailLinkRequired,
    SetupRequired,
}

async fn google_callback(
    State(state): State<ServerState>,
    client: ClientIp,
    headers: HeaderMap,
    Query(query): Query<CallbackQuery>,
) -> Response {
    if let Err(error) = ensure_failure_attempt_allowed(&state, client.0) {
        return error.into_response();
    }
    let Some(google) = state.google_sign_in() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(state_value) = query.state.clone() else {
        return callback_failure(&state, client.0, "expired");
    };
    // A server-side state lookup alone does not prevent login CSRF: the
    // callback must arrive in the same browser that initiated this flow.
    let cookie_matches = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|header| header.to_str().ok())
        .flat_map(|header| header.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .any(|(name, value)| {
            name == STATE_COOKIE && constant_time_eq(value.as_bytes(), state_value.as_bytes())
        });
    if !cookie_matches {
        return callback_failure(&state, client.0, "expired");
    }
    let mut response = finish_google_callback(&state, client, google, state_value, query).await;
    response
        .headers_mut()
        .append(header::SET_COOKIE, state_cookie("", 0));
    response
}

fn state_cookie(value: &str, max_age: i64) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{STATE_COOKIE}={value}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age={max_age}"
    ))
    .expect("random OAuth state is a valid cookie value")
}

async fn finish_google_callback(
    state: &ServerState,
    client: ClientIp,
    google: &GoogleSignIn,
    state_value: String,
    query: CallbackQuery,
) -> Response {
    let Some(pending) = google.take_pending(&state_value, Utc::now()) else {
        return callback_failure(state, client.0, "expired");
    };
    if query.error.is_some() {
        return callback_failure(state, client.0, "declined");
    }
    let Some(code) = query.code else {
        return callback_failure(state, client.0, "provider");
    };
    let claims = match google.exchange_code(&code, &pending, Utc::now()).await {
        Ok(claims) => claims,
        Err(ExchangeError::Claims("email-unverified")) => {
            return callback_failure(state, client.0, "email-unverified");
        }
        Err(error) => {
            tracing::warn!(target: "deskmate_server::google_sign_in", ?error, "Google sign-in token exchange failed");
            return callback_failure(state, client.0, "provider");
        }
    };

    let sign_in = state.clone();
    let result = tokio::task::spawn_blocking(move || resolve_account(&sign_in, &claims)).await;
    match result {
        Ok(Ok(SignInResult::SignedIn(session))) => (
            [(header::SET_COOKIE, set_session_cookie(&session))],
            Redirect::to("/"),
        )
            .into_response(),
        Ok(Ok(SignInResult::SignupsClosed)) => callback_failure(state, client.0, "signups-closed"),
        Ok(Ok(SignInResult::EmailLinkRequired)) => {
            callback_failure(state, client.0, "email-link-required")
        }
        Ok(Ok(SignInResult::SetupRequired)) => callback_failure(state, client.0, "setup-required"),
        Ok(Err(error)) => {
            tracing::error!(target: "deskmate_server::google_sign_in", %error, "Google sign-in identity update failed");
            callback_failure(state, client.0, "provider")
        }
        Err(error) => {
            tracing::error!(target: "deskmate_server::google_sign_in", %error, "Google sign-in identity worker failed");
            callback_failure(state, client.0, "provider")
        }
    }
}

fn resolve_account(
    state: &ServerState,
    claims: &GoogleClaims,
) -> Result<SignInResult, IdentityError> {
    if state.identity().account_count()? == 0 {
        return Ok(SignInResult::SetupRequired);
    }
    let account = if let Some(account) = state.identity().account_for_google(&claims.sub)? {
        account
    } else if let Some(account) = state.identity().account_by_email(&claims.email)? {
        if !claims.email_authoritative {
            return Ok(SignInResult::EmailLinkRequired);
        }
        state.identity().link_google(&claims.sub, &account.id)?;
        account
    } else {
        let signups_open = state
            .identity()
            .signups_open()?
            .unwrap_or(state.signups_default());
        if !signups_open {
            return Ok(SignInResult::SignupsClosed);
        }
        let account = state
            .identity()
            .create_account(&claims.email, true, false, Utc::now())?;
        state.identity().link_google(&claims.sub, &account.id)?;
        account
    };
    state.identity().mark_email_verified(&account.id)?;
    let session = state.identity().create_session(&account.id, Utc::now())?;
    Ok(SignInResult::SignedIn(session))
}

fn callback_failure(state: &ServerState, ip: std::net::IpAddr, reason: &'static str) -> Response {
    record_failure(state, ip);
    Redirect::to(&format!("/?signin_error={reason}")).into_response()
}

#[cfg(test)]
mod tests {
    use base64::prelude::BASE64_URL_SAFE_NO_PAD as B;
    use serde_json::{Value, json};

    use super::*;
    use crate::egress::FetchResponse;
    use crate::oauth::transport::{OAuthFuture, TransportError};

    struct NoTransport;

    impl OAuthTransport for NoTransport {
        fn post_form(
            &self,
            _url: String,
            _form: Vec<(String, String)>,
        ) -> OAuthFuture<'_, Result<FetchResponse, TransportError>> {
            Box::pin(async { Err(TransportError("unused".to_string())) })
        }
    }

    fn id_token(claims: &Value) -> String {
        format!(
            "{}.{}.sig",
            B.encode(br#"{"alg":"RS256"}"#),
            B.encode(claims.to_string())
        )
    }

    fn runtime() -> GoogleSignIn {
        GoogleSignIn::new(
            GoogleOAuthConfig {
                client_id: "cid".to_string(),
                client_secret: "secret".to_string(),
                redirect_uri: "https://deskmate.test/v1/app/auth/google/callback".to_string(),
                ..GoogleOAuthConfig::default()
            },
            Arc::new(NoTransport),
        )
    }

    #[test]
    fn accepts_a_verified_email_for_this_client_only() {
        let now = Utc::now();
        let good = json!({
            "iss": "https://accounts.google.com",
            "aud": "cid",
            "sub": "123",
            "email": "A@Example.com",
            "email_verified": true,
            "exp": now.timestamp() + 60,
            "nonce": "test-nonce",
        });
        let claims = claims_from_id_token(&id_token(&good), "cid", "test-nonce", now).unwrap();
        assert_eq!(
            (claims.sub.as_str(), claims.email.as_str()),
            ("123", "a@example.com")
        );

        for (field, value) in [
            ("aud", json!("other")),
            ("aud", json!(["cid", "other"])),
            ("azp", json!("other")),
            ("nonce", json!(null)),
            ("nonce", json!("another-sign-in")),
            ("iss", json!("https://evil.example")),
            ("email_verified", json!(false)),
            ("exp", json!(now.timestamp() - 1)),
        ] {
            let mut bad = good.clone();
            bad[field] = value;
            assert!(
                claims_from_id_token(&id_token(&bad), "cid", "test-nonce", now).is_err(),
                "{field}"
            );
        }
        assert!(claims_from_id_token("not-a-jwt", "cid", "test-nonce", now).is_err());
    }

    #[test]
    fn accepts_google_alternate_claim_encodings() {
        let now = Utc::now();
        let claims = json!({
            "iss": "accounts.google.com",
            "aud": ["another-client", "cid"],
            "azp": "cid",
            "sub": "google-sub",
            "email": " PERSON@Example.COM ",
            "email_verified": "true",
            "exp": now.timestamp() + 60,
            "nonce": "test-nonce",
        });
        let parsed = claims_from_id_token(&id_token(&claims), "cid", "test-nonce", now).unwrap();
        assert_eq!(parsed.sub, "google-sub");
        assert_eq!(parsed.email, "person@example.com");
    }

    #[test]
    fn rejects_empty_identity_claims_and_tokens_at_the_expiry_boundary() {
        let now = Utc::now();
        let base = json!({
            "iss": "https://accounts.google.com",
            "aud": "cid",
            "sub": "123",
            "email": "a@example.com",
            "email_verified": true,
            "exp": now.timestamp() + 60,
            "nonce": "test-nonce",
        });
        for (field, value) in [
            ("sub", json!("")),
            ("email", json!("")),
            ("exp", json!(now.timestamp())),
        ] {
            let mut bad = base.clone();
            bad[field] = value;
            assert!(
                claims_from_id_token(&id_token(&bad), "cid", "test-nonce", now).is_err(),
                "{field}"
            );
        }
    }

    #[test]
    fn start_budget_expires_without_extending_the_window_on_rejection() {
        let runtime = runtime();
        let now = std::time::Instant::now();
        for _ in 0..STARTS_PER_CLIENT {
            runtime
                .start_limiter
                .check_and_record("client", now)
                .unwrap();
        }
        assert_eq!(
            runtime
                .start_limiter
                .check_and_record("client", now + std::time::Duration::from_secs(599)),
            Err(std::time::Duration::from_secs(1))
        );
        for _ in 0..STARTS_PER_CLIENT {
            runtime
                .start_limiter
                .check_and_record("client", now + std::time::Duration::from_secs(600))
                .unwrap();
        }
        assert_eq!(
            runtime
                .start_limiter
                .check_and_record("client", now + std::time::Duration::from_secs(600)),
            Err(std::time::Duration::from_secs(600))
        );
    }

    #[test]
    fn pending_states_are_single_use_expire_and_are_capped() {
        let runtime = runtime();
        let now = Utc::now();
        let (first_url, _) = runtime.start(now).unwrap();
        let first_state = Url::parse(&first_url)
            .unwrap()
            .query_pairs()
            .find_map(|(key, value)| (key == "state").then(|| value.into_owned()))
            .unwrap();
        assert!(runtime.take_pending(&first_state, now).is_some());
        assert!(runtime.take_pending(&first_state, now).is_none());

        let (expired_url, _) = runtime.start(now).unwrap();
        let expired_state = Url::parse(&expired_url)
            .unwrap()
            .query_pairs()
            .find_map(|(key, value)| (key == "state").then(|| value.into_owned()))
            .unwrap();
        assert!(
            runtime
                .take_pending(&expired_state, now + PENDING_TTL)
                .is_none()
        );

        for _ in 0..MAX_PENDING_AUTHS {
            assert!(runtime.start(now).is_ok());
        }
        assert!(matches!(runtime.start(now), Err(StartError::AtCapacity)));
    }

    #[test]
    fn verified_email_match_is_linked_and_open_signups_create_an_account() {
        let existing_state = ServerState::in_memory();
        let existing = existing_state
            .identity()
            .create_account("person@example.com", false, true, Utc::now())
            .unwrap();
        let existing_claims = GoogleClaims {
            sub: "google-existing".to_string(),
            email: "person@example.com".to_string(),
            email_authoritative: true,
        };
        let SignInResult::SignedIn(session) =
            resolve_account(&existing_state, &existing_claims).unwrap()
        else {
            panic!("existing email must sign in");
        };
        assert_eq!(
            existing_state
                .identity()
                .session_account(&session, Utc::now())
                .unwrap()
                .unwrap()
                .id,
            existing.id
        );
        assert!(
            existing_state
                .identity()
                .account(&existing.id)
                .unwrap()
                .unwrap()
                .email_verified
        );
        assert_eq!(
            existing_state
                .identity()
                .account_for_google("google-existing")
                .unwrap()
                .unwrap()
                .id,
            existing.id
        );

        let open_state = ServerState::in_memory_with_options(crate::ServerOptions {
            signups_default: true,
            ..crate::ServerOptions::default()
        });
        open_state
            .identity()
            .create_account("owner@example.com", false, true, Utc::now())
            .unwrap();
        let new_claims = GoogleClaims {
            sub: "google-new".to_string(),
            email: "new@example.com".to_string(),
            email_authoritative: true,
        };
        assert!(matches!(
            resolve_account(&open_state, &new_claims),
            Ok(SignInResult::SignedIn(_))
        ));
        assert!(
            open_state
                .identity()
                .account_for_google("google-new")
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn an_existing_google_subject_wins_over_an_email_match() {
        let state = ServerState::in_memory();
        let subject_account = state
            .identity()
            .create_account("subject@example.com", true, true, Utc::now())
            .unwrap();
        state
            .identity()
            .link_google("stable-subject", &subject_account.id)
            .unwrap();
        let email_account = state
            .identity()
            .create_account("current@example.com", true, false, Utc::now())
            .unwrap();
        let claims = GoogleClaims {
            sub: "stable-subject".to_string(),
            email: "current@example.com".to_string(),
            email_authoritative: false,
        };
        let SignInResult::SignedIn(session) = resolve_account(&state, &claims).unwrap() else {
            panic!("known Google subject must sign in");
        };
        let signed_in = state
            .identity()
            .session_account(&session, Utc::now())
            .unwrap()
            .unwrap();
        assert_eq!(signed_in.id, subject_account.id);
        assert_ne!(signed_in.id, email_account.id);
    }
}
