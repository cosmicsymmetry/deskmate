//! Route-level checks that need a full `ServerState` and Axum. These bind
//! loopback and therefore CANNOT run under Codex's sandbox (project memory:
//! sandbox denies loopback binds) -- the controller runs them.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use server::oauth::session::SessionSigner;
use server::oauth::{GoogleOAuthConfig, IntegrationRuntime, TokenManager};
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
    assert!(
        to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn callback_and_revoke_also_reject_unauthenticated_callers_with_bare_401s() {
    for (method, uri) in [
        (
            "GET",
            "/v1/integrations/google/callback?code=abc&state=state",
        ),
        ("POST", "/v1/integrations/google-primary/revoke"),
    ] {
        let response = app(state_with_integrations())
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .is_empty()
        );
    }
}

#[tokio::test]
async fn callback_rejects_a_state_from_another_session_before_exchange() {
    let state = state_with_integrations();
    let consent_url = state
        .integrations()
        .expect("integrations")
        .start_consent("different-session", "google-primary");
    let state_value = url::Url::parse(&consent_url)
        .unwrap()
        .query_pairs()
        .find_map(|(key, value)| (key == "state").then(|| value.into_owned()))
        .expect("state query parameter");

    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/integrations/google/callback?code=abc&state={state_value}"
                ))
                .header(header::AUTHORIZATION, "Bearer in-memory-admin-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        state
            .integrations()
            .expect("integrations")
            .take_pending(&state_value, chrono::Utc::now())
            .is_none(),
        "a callback attempt consumes the state even when the session does not match"
    );
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
    let location = response
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(location.starts_with("https://accounts.google.com/o/oauth2/v2/auth"));
}
