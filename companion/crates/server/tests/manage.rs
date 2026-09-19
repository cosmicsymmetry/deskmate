//! Route-level checks for the management surface. These bind loopback and
//! therefore CANNOT run under a sandboxed implementer (project memory) -- the
//! controller runs them.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode, header};
use server::oauth::{GoogleOAuthConfig, IntegrationRuntime, TokenManager};
use server::secrets::{IntegrationSecret, IntegrationStore, SecretsKey};
use server::{ServerState, app};
use tower::ServiceExt;

const ADMIN: &str = "in-memory-admin-token";
const STORED_REFRESH_TOKEN: &str = "stored-refresh-token-value";
const STORED_CLIENT_SECRET: &str = "stored-client-secret-value";

struct NoTransport;
impl server::oauth::transport::OAuthTransport for NoTransport {
    fn post_form(
        &self,
        _url: String,
        _form: Vec<(String, String)>,
    ) -> server::oauth::transport::OAuthFuture<
        '_,
        Result<server::oauth::transport::FetchResponse, server::oauth::transport::TransportError>,
    > {
        Box::pin(async {
            Err(server::oauth::transport::TransportError(
                "no transport in this test".to_string(),
            ))
        })
    }
}

fn state() -> ServerState {
    let state = ServerState::in_memory();
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.keep().join("secrets.enc");
    let store =
        Arc::new(IntegrationStore::open(path, SecretsKey::from_bytes([1u8; 32])).expect("store"));
    let token_manager = Arc::new(TokenManager::new(
        store,
        Arc::new(NoTransport),
        GoogleOAuthConfig::default(),
    ));
    let runtime = Arc::new(IntegrationRuntime::new(token_manager));
    state.set_integrations(runtime);
    state
}

fn state_with_stored_grant() -> ServerState {
    let state = ServerState::in_memory();
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.keep().join("secrets.enc");
    let store =
        Arc::new(IntegrationStore::open(path, SecretsKey::from_bytes([1u8; 32])).expect("store"));
    store
        .put(
            "google".to_string(),
            IntegrationSecret {
                provider: "google".to_string(),
                refresh_token: STORED_REFRESH_TOKEN.to_string(),
                client_secret: Some(STORED_CLIENT_SECRET.to_string()),
                scopes: vec!["calendar.events.readonly".to_string()],
                obtained_at: 0,
            },
        )
        .expect("put");
    let token_manager = Arc::new(TokenManager::new(
        store,
        Arc::new(NoTransport),
        GoogleOAuthConfig::default(),
    ));
    let runtime = Arc::new(IntegrationRuntime::new(token_manager));
    state.set_integrations(runtime);
    state
}

async fn get(state: &ServerState, uri: &str, bearer: Option<&str>) -> (StatusCode, String) {
    let mut request = Request::builder().uri(uri);
    if let Some(bearer) = bearer {
        request = request.header(header::AUTHORIZATION, format!("Bearer {bearer}"));
    }
    let response = app(state.clone())
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

async fn post_form(
    state: &ServerState,
    uri: &str,
    body: &str,
    bearer: Option<&str>,
) -> (StatusCode, HeaderMap, String) {
    let mut request = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(bearer) = bearer {
        request = request.header(header::AUTHORIZATION, format!("Bearer {bearer}"));
    }
    let response = app(state.clone())
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

#[tokio::test]
async fn the_dashboard_refuses_an_unauthenticated_caller_with_a_bare_401() {
    let (status, body) = get(&state(), "/v1/manage", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.is_empty(), "no dashboard content may leak: {body}");
}

#[tokio::test]
async fn the_dashboard_renders_for_the_admin_bearer() {
    let (status, body) = get(&state(), "/v1/manage", Some(ADMIN)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.starts_with("<!doctype html>"), "got {body:.80}");
}

#[tokio::test]
async fn the_login_page_is_reachable_without_a_session() {
    // A 401 with no way to authenticate would make the page useless in a
    // browser, which is the one client it exists for.
    let (status, body) = get(&state(), "/v1/manage/login", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("<form"));
}

#[tokio::test]
async fn a_wrong_token_does_not_mint_a_session() {
    let (status, headers, _) = post_form(&state(), "/v1/manage/login", "token=wrong", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!headers.contains_key(header::SET_COOKIE));
}

#[tokio::test]
async fn login_without_integrations_checks_credentials_before_configuration() {
    let state = ServerState::in_memory();
    let (_, login_page) = get(&state, "/v1/manage/login", None).await;
    for (token, status, message) in [
        (
            ADMIN,
            StatusCode::SERVICE_UNAVAILABLE,
            "OAuth integrations are not configured on this server.",
        ),
        (
            "wrong",
            StatusCode::UNAUTHORIZED,
            "That token was not accepted.",
        ),
    ] {
        let (actual, headers, body) =
            post_form(&state, "/v1/manage/login", &format!("token={token}"), None).await;
        assert_eq!(actual, status);
        assert!(!headers.contains_key(header::SET_COOKIE));
        assert_eq!(
            body,
            login_page.replacen(
                "<h1>Deskmate</h1>",
                &format!("<h1>Deskmate</h1><p class=\"bad\">{message}</p>"),
                1,
            )
        );
    }
}

#[tokio::test]
async fn the_right_token_mints_a_session_cookie_that_script_cannot_read() {
    let (status, headers, _) = post_form(
        &state(),
        "/v1/manage/login",
        &format!("token={ADMIN}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let cookie = headers
        .get(header::SET_COOKIE)
        .expect("a session cookie")
        .to_str()
        .unwrap();
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("Secure"));
    assert!(cookie.starts_with("__Host-deskmate_session="));
    assert!(cookie.ends_with("; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=43200"));
    assert!(!cookie.to_ascii_lowercase().contains("domain="));
    assert_eq!(headers.get(header::LOCATION).unwrap(), "/v1/manage");
}

#[tokio::test]
async fn the_session_cookie_from_the_form_opens_the_dashboard() {
    // One cookie format, not two: the form login must mint what the extractor
    // already accepts.
    let state = state();
    let (_, headers, _) =
        post_form(&state, "/v1/manage/login", &format!("token={ADMIN}"), None).await;
    let set_cookie = headers.get(header::SET_COOKIE).unwrap().to_str().unwrap();
    let cookie = set_cookie.split(';').next().unwrap().to_string();

    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/v1/manage")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn the_dashboard_never_renders_a_stored_credential() {
    // The dashboard reports presence and health, never credential values.
    let (_, body) = get(&state_with_stored_grant(), "/v1/manage", Some(ADMIN)).await;
    assert!(!body.contains(STORED_REFRESH_TOKEN));
    assert!(!body.contains(STORED_CLIENT_SECRET));
}

#[tokio::test]
async fn the_dashboard_lists_a_stored_integration_by_id() {
    let (_, body) = get(&state_with_stored_grant(), "/v1/manage", Some(ADMIN)).await;
    assert!(body.contains("google"));
}

#[tokio::test]
async fn the_dashboard_actions_refuse_an_unauthenticated_caller() {
    for path in [
        "/v1/manage/integrations/google/connect",
        "/v1/manage/integrations/google/revoke",
        "/v1/manage/integrations/google/producer",
    ] {
        let (status, _, _) = post_form(&state(), path, "", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{path} must be gated");
    }
}

#[tokio::test]
async fn connecting_redirects_to_the_providers_consent_page() {
    let (status, headers, _) = post_form(
        &state(),
        "/v1/manage/integrations/google/connect",
        "",
        Some(ADMIN),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let location = headers.get(header::LOCATION).unwrap().to_str().unwrap();
    assert!(
        location.starts_with("https://accounts.google.com/"),
        "got {location}"
    );
}

#[tokio::test]
async fn revoking_from_the_dashboard_redirects_back_to_it() {
    let (status, headers, _) = post_form(
        &state_with_stored_grant(),
        "/v1/manage/integrations/google/revoke",
        "",
        Some(ADMIN),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers.get(header::LOCATION).unwrap(), "/v1/manage");
}

#[tokio::test]
async fn minting_shows_the_credential_inline_and_never_in_a_url() {
    // A redirect would carry the value in Location, which reaches browser
    // history and every access log between here and the operator.
    let (status, headers, body) = post_form(
        &state_with_stored_grant(),
        "/v1/manage/integrations/google/producer",
        "",
        Some(ADMIN),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "rendered inline, never redirected");
    assert!(!headers.contains_key(header::LOCATION));
    assert!(body.to_lowercase().contains("once"));
}

#[tokio::test]
async fn a_credential_minted_from_the_dashboard_actually_vends() {
    // The browser path and the API path must reach the same store; a dashboard
    // that minted into a different one would look fine and work nowhere.
    let state = state_with_stored_grant();
    let (_, _, body) = post_form(
        &state,
        "/v1/manage/integrations/google/producer",
        "",
        Some(ADMIN),
    )
    .await;
    // The credential is the only 64-hex-character run on the page.
    let token = body
        .split(|c: char| !c.is_ascii_hexdigit())
        .find(|candidate| candidate.len() == 64)
        .expect("a credential on the page");

    let (status, _, _) = post_form(&state, "/v1/integrations/google/token", "", Some(token)).await;
    assert_ne!(
        status,
        StatusCode::UNAUTHORIZED,
        "the dashboard-minted credential must authenticate against the API route"
    );
}
