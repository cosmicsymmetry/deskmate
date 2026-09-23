//! Route-level checks that need a full `ServerState` and Axum, exercised
//! through in-process `Router::oneshot` requests.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use server::oauth::GoogleOAuthConfig;
use server::oauth::transport::EgressTransport;
use server::secrets::IntegrationSecret;
use server::{ServerState, app};
use tower::ServiceExt;

mod oauth_support;
mod support;

use oauth_support::{FakeTransport, integration_state};

fn account_request(
    _state: &ServerState,
    account: &support::TestAccount,
    method: &str,
    uri: impl AsRef<str>,
) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri.as_ref())
        .header(header::COOKIE, &account.cookie)
        .header(header::ORIGIN, "https://deskmate.test")
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn callback_with_unknown_state_is_rejected_before_any_exchange() {
    let fixture = integration_state(Arc::new(EgressTransport), None);
    let owner = support::owner_account(&fixture.state);
    let app = app(fixture.state.clone());
    let response = app
        .oneshot(account_request(
            &fixture.state,
            &owner,
            "GET",
            "/v1/integrations/google/callback?code=abc&state=never-issued",
        ))
        .await
        .unwrap();
    // No stash for that state -> 400, and crucially no token exchange was attempted.
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn oauth_routes_reject_an_unauthenticated_caller_with_a_bare_401() {
    let fixture = integration_state(Arc::new(EgressTransport), None);
    let app = app(fixture.state.clone());
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
    let fixture = integration_state(Arc::new(EgressTransport), None);
    for (method, uri) in [
        (
            "GET",
            "/v1/integrations/google/callback?code=abc&state=state",
        ),
        ("POST", "/v1/integrations/google-primary/revoke"),
    ] {
        let response = app(fixture.state.clone())
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
    let fixture = integration_state(Arc::new(EgressTransport), None);
    let state = &fixture.state;
    let first = support::owner_account(state);
    let second = support::new_session(state, &first);
    let consent = app(state.clone())
        .oneshot(account_request(
            state,
            &first,
            "POST",
            "/v1/integrations/google",
        ))
        .await
        .unwrap();
    assert_eq!(consent.status(), StatusCode::SEE_OTHER);
    let consent_url = consent
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    let state_value = url::Url::parse(consent_url)
        .unwrap()
        .query_pairs()
        .find_map(|(key, value)| (key == "state").then(|| value.into_owned()))
        .expect("state query parameter");

    let response = app(state.clone())
        .oneshot(account_request(
            state,
            &second,
            "GET",
            format!("/v1/integrations/google/callback?code=abc&state={state_value}"),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .is_empty()
    );
    let retry = app(state.clone())
        .oneshot(account_request(
            state,
            &first,
            "GET",
            format!("/v1/integrations/google/callback?code=abc&state={state_value}"),
        ))
        .await
        .unwrap();
    assert_eq!(
        retry.status(),
        StatusCode::BAD_REQUEST,
        "a callback attempt consumes the state even when the session does not match"
    );
    assert_eq!(
        to_bytes(retry.into_body(), usize::MAX).await.unwrap(),
        "unknown or expired state"
    );
}

#[tokio::test]
async fn start_consent_redirects_to_google_for_the_instance_owner() {
    let fixture = integration_state(Arc::new(EgressTransport), None);
    let owner = support::owner_account(&fixture.state);
    let app = app(fixture.state.clone());
    let response = app
        .oneshot(account_request(
            &fixture.state,
            &owner,
            "POST",
            "/v1/integrations/google",
        ))
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

// --- Token vending --------------------------------------------------------

const STORED_REFRESH_TOKEN: &str = "stored-refresh-token-value";
const STORED_CLIENT_SECRET: &str = "stored-client-secret-value";

fn stored_grant() -> IntegrationSecret {
    IntegrationSecret {
        provider: "google".to_string(),
        refresh_token: STORED_REFRESH_TOKEN.to_string(),
        client_secret: Some(STORED_CLIENT_SECRET.to_string()),
        scopes: vec!["https://www.googleapis.com/auth/calendar.events.readonly".to_string()],
        obtained_at: 0,
    }
}

async fn mint_producer_credential(state: &ServerState, integration_id: &str) -> String {
    let owner = support::owner_account(state);
    let response = app(state.clone())
        .oneshot(account_request(
            state,
            &owner,
            "POST",
            format!("/v1/integrations/{integration_id}/producer"),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED, "mint must succeed");
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body).expect("json");
    parsed["token"].as_str().expect("token").to_string()
}

async fn vend(
    state: &ServerState,
    integration_id: &str,
    bearer: Option<&str>,
) -> (StatusCode, String) {
    let mut request = Request::builder()
        .method("POST")
        .uri(format!("/v1/integrations/{integration_id}/token"));
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

#[tokio::test]
async fn a_producer_credential_vends_the_providers_access_token() {
    let transport = FakeTransport::with(vec![(
        200,
        r#"{"access_token":"ya29.the-live-token","expires_in":3600}"#,
    )]);
    let fixture = integration_state(transport, Some(stored_grant()));
    let credential = mint_producer_credential(&fixture.state, "google").await;

    let (status, body) = vend(&fixture.state, "google", Some(&credential)).await;

    assert_eq!(status, StatusCode::OK);
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(parsed["access_token"], "ya29.the-live-token");
    assert!(
        parsed["expires_at"].is_string(),
        "the provider's own expiry must travel with the token"
    );
}

#[tokio::test]
async fn the_vend_response_carries_no_refresh_token_or_client_secret() {
    // The custodian boundary: a producer gets a credential the provider can
    // revoke, never the grant that mints them.
    let transport = FakeTransport::with(vec![(
        200,
        r#"{"access_token":"ya29.the-live-token","expires_in":3600,"refresh_token":"rotated-refresh"}"#,
    )]);
    let fixture = integration_state(transport, Some(stored_grant()));
    let credential = mint_producer_credential(&fixture.state, "google").await;

    let (_, body) = vend(&fixture.state, "google", Some(&credential)).await;

    assert!(!body.contains(STORED_REFRESH_TOKEN));
    assert!(!body.contains(STORED_CLIENT_SECRET));
    assert!(!body.contains("rotated-refresh"));
    assert!(!body.contains("refresh_token"));
}

#[tokio::test]
async fn an_unauthenticated_vend_is_refused() {
    let fixture = integration_state(FakeTransport::with(vec![]), Some(stored_grant()));
    let (status, _) = vend(&fixture.state, "google", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_admin_token_does_not_vend() {
    // An admin credential living in a producer's environment is exactly what
    // the separate producer credential exists to avoid.
    let fixture = integration_state(FakeTransport::with(vec![]), Some(stored_grant()));
    let (status, _) = vend(&fixture.state, "google", Some("in-memory-admin-token")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_credential_for_another_integration_cannot_vend() {
    // Uniform 401, not 403: a distinguishable rejection would enumerate which
    // integration ids exist.
    let fixture = integration_state(FakeTransport::with(vec![]), Some(stored_grant()));
    let credential = mint_producer_credential(&fixture.state, "google").await;
    let (status, _) = vend(&fixture.state, "dropbox", Some(&credential)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_revoked_grant_vends_conflict_naming_reconnection() {
    let transport = FakeTransport::with(vec![(
        400,
        r#"{"error":"invalid_grant","error_description":"expired"}"#,
    )]);
    let fixture = integration_state(transport, Some(stored_grant()));
    let credential = mint_producer_credential(&fixture.state, "google").await;

    let (status, body) = vend(&fixture.state, "google", Some(&credential)).await;

    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a revoked grant is not a bad request; retrying cannot fix it"
    );
    assert!(body.contains("reconnect"), "got {body}");
}

#[tokio::test]
async fn minting_a_producer_credential_requires_the_operator() {
    let fixture = integration_state(FakeTransport::with(vec![]), Some(stored_grant()));
    let response = app(fixture.state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/integrations/google/producer")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn revoking_a_producer_credential_stops_it_vending() {
    let transport = FakeTransport::with(vec![(
        200,
        r#"{"access_token":"ya29.the-live-token","expires_in":3600}"#,
    )]);
    let fixture = integration_state(transport, Some(stored_grant()));
    let credential = mint_producer_credential(&fixture.state, "google").await;
    let owner = support::owner_account(&fixture.state);

    let revoked = app(fixture.state.clone())
        .oneshot(account_request(
            &fixture.state,
            &owner,
            "DELETE",
            "/v1/integrations/google/producer",
        ))
        .await
        .unwrap();
    assert_eq!(revoked.status(), StatusCode::NO_CONTENT);

    let (status, _) = vend(&fixture.state, "google", Some(&credential)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_producer_supplied_target_is_ignored_entirely() {
    // Spec §5's named trap: a vend route that accepted a URL, host or scope
    // from its caller would recreate the SSRF surface the egress guard bounds,
    // on the one path that carries a credential. The handler takes no body and
    // no query, and this pins that the integration record stays the only
    // source of the target.
    let transport = FakeTransport::with(vec![(
        200,
        r#"{"access_token":"ya29.the-live-token","expires_in":3600}"#,
    )]);
    let fixture = integration_state(transport.clone(), Some(stored_grant()));
    let credential = mint_producer_credential(&fixture.state, "google").await;

    let response = app(fixture.state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/integrations/google/token?token_uri=https://attacker.example/token")
                .header(header::AUTHORIZATION, format!("Bearer {credential}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"token_uri":"https://attacker.example/token","scope":"https://www.googleapis.com/auth/drive"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        transport.hosts_called(),
        vec![
            url::Url::parse(&GoogleOAuthConfig::default().token_uri)
                .unwrap()
                .host_str()
                .unwrap()
                .to_string()
        ],
        "the producer's suggested target must never be reached"
    );
}

#[tokio::test]
async fn revoking_an_integration_also_revokes_its_producer_credential() {
    // Otherwise a producer keeps a credential against a dead integration and
    // the failure surfaces as a 401 loop, which reads like a broken producer
    // rather than the disconnection the operator actually performed.
    let transport = FakeTransport::with(vec![(200, r"{}")]);
    let fixture = integration_state(transport, Some(stored_grant()));
    let credential = mint_producer_credential(&fixture.state, "google").await;
    let owner = support::owner_account(&fixture.state);

    let revoked = app(fixture.state.clone())
        .oneshot(account_request(
            &fixture.state,
            &owner,
            "POST",
            "/v1/integrations/google/revoke",
        ))
        .await
        .unwrap();
    assert_eq!(revoked.status(), StatusCode::NO_CONTENT);

    let (status, _) = vend(&fixture.state, "google", Some(&credential)).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "the producer credential must not outlive the integration"
    );
}

#[tokio::test]
async fn revoking_an_integration_drops_the_credential_even_when_the_remote_revoke_fails() {
    // TokenManager::revoke already removes local state regardless of what the
    // provider says, because the operator asked to disconnect. Leaving a live
    // producer credential behind would contradict that.
    let transport = FakeTransport::with(vec![]); // every post_form errors
    let fixture = integration_state(transport, Some(stored_grant()));
    let credential = mint_producer_credential(&fixture.state, "google").await;
    let owner = support::owner_account(&fixture.state);

    let revoked = app(fixture.state.clone())
        .oneshot(account_request(
            &fixture.state,
            &owner,
            "POST",
            "/v1/integrations/google/revoke",
        ))
        .await
        .unwrap();
    assert_eq!(revoked.status(), StatusCode::NO_CONTENT);

    let (status, _) = vend(&fixture.state, "google", Some(&credential)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
