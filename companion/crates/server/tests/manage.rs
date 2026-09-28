//! Route-level checks for the management surface through in-process
//! `Router::oneshot` requests.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode, header};
use server::oauth::transport::OAuthTransport;
use server::secrets::IntegrationSecret;
use server::{ServerState, app};
use tower::ServiceExt;

mod oauth_support;
mod support;

use oauth_support::{FakeTransport, IntegrationFixture, integration_state};

const STORED_REFRESH_TOKEN: &str = "stored-refresh-token-value";
const STORED_CLIENT_SECRET: &str = "stored-client-secret-value";

fn stored_grant() -> IntegrationSecret {
    IntegrationSecret {
        provider: "google".to_string(),
        refresh_token: STORED_REFRESH_TOKEN.to_string(),
        client_secret: Some(STORED_CLIENT_SECRET.to_string()),
        scopes: vec!["calendar.events.readonly".to_string()],
        obtained_at: 0,
    }
}

fn state_with_stored_grant(transport: Arc<dyn OAuthTransport>) -> IntegrationFixture {
    integration_state(transport, Some(stored_grant()))
}

async fn get(
    state: &ServerState,
    uri: &str,
    account: Option<&support::TestAccount>,
) -> (StatusCode, String) {
    let mut request = Request::builder().uri(uri);
    if let Some(account) = account {
        request = request
            .header(header::COOKIE, &account.cookie)
            .header(header::ORIGIN, "https://deskmate.test");
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
    account: Option<&support::TestAccount>,
    bearer: Option<&str>,
) -> (StatusCode, HeaderMap, String) {
    let mut request = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(account) = account {
        request = request
            .header(header::COOKIE, &account.cookie)
            .header(header::ORIGIN, "https://deskmate.test");
    } else if let Some(bearer) = bearer {
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
async fn the_dashboard_redirects_an_unauthenticated_caller_to_the_app() {
    let fixture = integration_state(FakeTransport::with(vec![]), None);
    let (status, body) = get(&fixture.state, "/v1/manage", None).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(body.is_empty(), "no dashboard content may leak: {body}");
}

#[tokio::test]
async fn the_dashboard_renders_for_the_instance_owner() {
    let fixture = integration_state(FakeTransport::with(vec![]), None);
    let owner = support::owner_account(&fixture.state);
    let (status, body) = get(&fixture.state, "/v1/manage", Some(&owner)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.starts_with("<!doctype html>"), "got {body:.80}");
}

#[tokio::test]
async fn the_old_login_page_is_removed() {
    let fixture = integration_state(FakeTransport::with(vec![]), None);
    let (status, body) = get(&fixture.state, "/v1/manage/login", None).await;
    assert!(matches!(
        status,
        StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
    ));
    assert!(body.is_empty());
}

#[tokio::test]
async fn the_dashboard_never_renders_a_stored_credential() {
    // The dashboard reports presence and health, never credential values.
    let fixture = state_with_stored_grant(FakeTransport::with(vec![]));
    let owner = support::owner_account(&fixture.state);
    let (_, body) = get(&fixture.state, "/v1/manage", Some(&owner)).await;
    assert!(!body.contains(STORED_REFRESH_TOKEN));
    assert!(!body.contains(STORED_CLIENT_SECRET));
}

#[tokio::test]
async fn the_dashboard_lists_a_stored_integration_by_id() {
    let fixture = state_with_stored_grant(FakeTransport::with(vec![]));
    let owner = support::owner_account(&fixture.state);
    let (_, body) = get(&fixture.state, "/v1/manage", Some(&owner)).await;
    assert!(body.contains("google"));
}

#[tokio::test]
async fn the_dashboard_actions_refuse_an_unauthenticated_caller() {
    let fixture = integration_state(FakeTransport::with(vec![]), None);
    for path in [
        "/v1/manage/integrations/google/connect",
        "/v1/manage/integrations/google/revoke",
        "/v1/manage/integrations/google/producer",
    ] {
        let (status, _, _) = post_form(&fixture.state, path, "", None, None).await;
        assert_eq!(status, StatusCode::SEE_OTHER, "{path} must be gated");
    }
}

#[tokio::test]
async fn connecting_redirects_to_the_providers_consent_page() {
    let fixture = integration_state(FakeTransport::with(vec![]), None);
    let owner = support::owner_account(&fixture.state);
    let (status, headers, _) = post_form(
        &fixture.state,
        "/v1/manage/integrations/google/connect",
        "",
        Some(&owner),
        None,
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
    let fixture = state_with_stored_grant(FakeTransport::with(vec![]));
    let owner = support::owner_account(&fixture.state);
    let (status, headers, _) = post_form(
        &fixture.state,
        "/v1/manage/integrations/google/revoke",
        "",
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers.get(header::LOCATION).unwrap(), "/v1/manage");
}

#[tokio::test]
async fn minting_shows_the_credential_inline_and_never_in_a_url() {
    // A redirect would carry the value in Location, which reaches browser
    // history and every access log between here and the operator.
    let fixture = state_with_stored_grant(FakeTransport::with(vec![]));
    let owner = support::owner_account(&fixture.state);
    let (status, headers, body) = post_form(
        &fixture.state,
        "/v1/manage/integrations/google/producer",
        "",
        Some(&owner),
        None,
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
    let transport = FakeTransport::with(vec![(
        200,
        r#"{"access_token":"ya29.dashboard-token","expires_in":3600}"#,
    )]);
    let fixture = state_with_stored_grant(transport);
    let owner = support::owner_account(&fixture.state);
    let (mint_status, _, body) = post_form(
        &fixture.state,
        "/v1/manage/integrations/google/producer",
        "",
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(mint_status, StatusCode::OK);
    // The credential is the only 64-hex-character run on the page.
    let token = body
        .split(|c: char| !c.is_ascii_hexdigit())
        .find(|candidate| candidate.len() == 64)
        .expect("a credential on the page");

    let (status, _, body) = post_form(
        &fixture.state,
        "/v1/integrations/google/token",
        "",
        None,
        Some(token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&body).expect("vend response JSON");
    assert_eq!(body["access_token"], "ya29.dashboard-token");
    assert!(body["expires_at"].is_string());
}

#[tokio::test]
async fn a_provider_error_that_mentions_needs_reconnect_is_still_warning_health() {
    let fixture = state_with_stored_grant(FakeTransport::with(vec![(
        503,
        r#"{"error":"temporarily_unavailable","error_description":"NeedsReconnect <retry & wait>"}"#,
    )]));
    let owner = support::owner_account(&fixture.state);
    let (mint_status, _, body) = post_form(
        &fixture.state,
        "/v1/manage/integrations/google/producer",
        "",
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(mint_status, StatusCode::OK);
    let token = body
        .split(|c: char| !c.is_ascii_hexdigit())
        .find(|candidate| candidate.len() == 64)
        .expect("a credential on the page");

    let (vend_status, _, _) = post_form(
        &fixture.state,
        "/v1/integrations/google/token",
        "",
        None,
        Some(token),
    )
    .await;
    assert_eq!(vend_status, StatusCode::BAD_GATEWAY);

    let (dashboard_status, dashboard) = get(&fixture.state, "/v1/manage", Some(&owner)).await;
    assert_eq!(dashboard_status, StatusCode::OK);
    assert!(
        dashboard.contains(
            "<td class=\"warn\">Error(&quot;temporarily_unavailable: NeedsReconnect &lt;retry &amp; wait&gt;&quot;)</td>"
        ),
        "wrong health cell: {dashboard}"
    );
}
