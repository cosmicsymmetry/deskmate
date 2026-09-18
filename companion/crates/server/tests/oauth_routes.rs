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
        .start_consent("different-session", "google-primary")
        .expect("under the pending limit");
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

// --- Token vending --------------------------------------------------------

use std::sync::Mutex;

use server::egress::FetchResponse;
use server::oauth::transport::{OAuthFuture, OAuthTransport, TransportError};
use server::secrets::{IntegrationSecret, IntegrationStore, SecretsKey};

const STORED_REFRESH_TOKEN: &str = "stored-refresh-token-value";
const STORED_CLIENT_SECRET: &str = "stored-client-secret-value";

/// Queued canned responses plus a record of every URL actually posted to, so a
/// test can assert the server reached exactly the hosts it should have.
struct FakeTransport {
    queued: Mutex<Vec<(u16, Vec<u8>)>>,
    urls: Mutex<Vec<String>>,
}

impl FakeTransport {
    fn with(responses: Vec<(u16, &str)>) -> Arc<Self> {
        Arc::new(Self {
            queued: Mutex::new(
                responses
                    .into_iter()
                    .rev()
                    .map(|(status, body)| (status, body.as_bytes().to_vec()))
                    .collect(),
            ),
            urls: Mutex::new(Vec::new()),
        })
    }

    fn hosts_called(&self) -> Vec<String> {
        self.urls
            .lock()
            .unwrap()
            .iter()
            .filter_map(|url| url::Url::parse(url).ok())
            .filter_map(|url| url.host_str().map(str::to_owned))
            .collect()
    }
}

impl OAuthTransport for FakeTransport {
    fn post_form(
        &self,
        url: String,
        _form: Vec<(String, String)>,
    ) -> OAuthFuture<'_, Result<FetchResponse, TransportError>> {
        self.urls.lock().unwrap().push(url);
        let next = self.queued.lock().unwrap().pop();
        Box::pin(async move {
            next.map(|(status, body)| FetchResponse { status, body })
                .ok_or_else(|| TransportError("no queued response".to_string()))
        })
    }
}

fn state_with_stored_grant(transport: Arc<FakeTransport>) -> ServerState {
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
                scopes: vec![
                    "https://www.googleapis.com/auth/calendar.events.readonly".to_string(),
                ],
                obtained_at: 0,
            },
        )
        .expect("put");
    let token_manager = Arc::new(TokenManager::new(
        store,
        transport,
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

async fn mint_producer_credential(state: &ServerState, integration_id: &str) -> String {
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/integrations/{integration_id}/producer"))
                .header(header::AUTHORIZATION, "Bearer in-memory-admin-token")
                .body(Body::empty())
                .unwrap(),
        )
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
    let state = state_with_stored_grant(transport);
    let credential = mint_producer_credential(&state, "google").await;

    let (status, body) = vend(&state, "google", Some(&credential)).await;

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
    let state = state_with_stored_grant(transport);
    let credential = mint_producer_credential(&state, "google").await;

    let (_, body) = vend(&state, "google", Some(&credential)).await;

    assert!(!body.contains(STORED_REFRESH_TOKEN));
    assert!(!body.contains(STORED_CLIENT_SECRET));
    assert!(!body.contains("rotated-refresh"));
    assert!(!body.contains("refresh_token"));
}

#[tokio::test]
async fn an_unauthenticated_vend_is_refused() {
    let state = state_with_stored_grant(FakeTransport::with(vec![]));
    let (status, _) = vend(&state, "google", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_admin_token_does_not_vend() {
    // An admin credential living in a producer's environment is exactly what
    // the separate producer credential exists to avoid.
    let state = state_with_stored_grant(FakeTransport::with(vec![]));
    let (status, _) = vend(&state, "google", Some("in-memory-admin-token")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_credential_for_another_integration_cannot_vend() {
    // Uniform 401, not 403: a distinguishable rejection would enumerate which
    // integration ids exist.
    let state = state_with_stored_grant(FakeTransport::with(vec![]));
    let credential = mint_producer_credential(&state, "google").await;
    let (status, _) = vend(&state, "dropbox", Some(&credential)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_revoked_grant_vends_conflict_naming_reconnection() {
    let transport = FakeTransport::with(vec![(
        400,
        r#"{"error":"invalid_grant","error_description":"expired"}"#,
    )]);
    let state = state_with_stored_grant(transport);
    let credential = mint_producer_credential(&state, "google").await;

    let (status, body) = vend(&state, "google", Some(&credential)).await;

    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a revoked grant is not a bad request; retrying cannot fix it"
    );
    assert!(body.contains("reconnect"), "got {body}");
}

#[tokio::test]
async fn minting_a_producer_credential_requires_the_operator() {
    let state = state_with_stored_grant(FakeTransport::with(vec![]));
    let response = app(state)
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
    let state = state_with_stored_grant(transport);
    let credential = mint_producer_credential(&state, "google").await;

    let revoked = app(state.clone())
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/v1/integrations/google/producer")
                .header(header::AUTHORIZATION, "Bearer in-memory-admin-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revoked.status(), StatusCode::NO_CONTENT);

    let (status, _) = vend(&state, "google", Some(&credential)).await;
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
    let state = state_with_stored_grant(transport.clone());
    let credential = mint_producer_credential(&state, "google").await;

    let response = app(state)
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
    let state = state_with_stored_grant(transport);
    let credential = mint_producer_credential(&state, "google").await;

    let revoked = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/integrations/google/revoke")
                .header(header::AUTHORIZATION, "Bearer in-memory-admin-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revoked.status(), StatusCode::NO_CONTENT);

    let (status, _) = vend(&state, "google", Some(&credential)).await;
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
    let state = state_with_stored_grant(transport);
    let credential = mint_producer_credential(&state, "google").await;

    let revoked = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/integrations/google/revoke")
                .header(header::AUTHORIZATION, "Bearer in-memory-admin-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revoked.status(), StatusCode::NO_CONTENT);

    let (status, _) = vend(&state, "google", Some(&credential)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
