mod support;

use std::sync::{Arc, Mutex};

use base64::prelude::{BASE64_URL_SAFE_NO_PAD as B, Engine as _};
use chrono::Utc;
use reqwest::StatusCode;
use serde_json::{Value, json};
use server::mailer::RecordingMailer;
use server::oauth::GoogleOAuthConfig;
use server::oauth::transport::{FetchResponse, OAuthFuture, OAuthTransport, TransportError};
use server::{ServerOptions, ServerState, app};
use sha2::{Digest, Sha256};
use support::*;

#[derive(Default)]
struct FakeGoogleTransport {
    id_token: Mutex<Option<String>>,
    calls: Mutex<Vec<TransportCall>>,
}

type TransportCall = (String, Vec<(String, String)>);

impl FakeGoogleTransport {
    fn answer_id_token(&self, claims: &Value) {
        *self.id_token.lock().unwrap() = Some(id_token(claims));
    }
}

impl OAuthTransport for FakeGoogleTransport {
    fn post_form(
        &self,
        url: String,
        form: Vec<(String, String)>,
    ) -> OAuthFuture<'_, Result<FetchResponse, TransportError>> {
        self.calls.lock().unwrap().push((url, form));
        let token = self.id_token.lock().unwrap().clone();
        Box::pin(async move {
            let token = token.ok_or_else(|| TransportError("no fake response".to_string()))?;
            Ok(FetchResponse {
                status: 200,
                body: json!({
                    "id_token": token,
                    "access_token": "x",
                    "expires_in": 3600,
                })
                .to_string()
                .into_bytes(),
            })
        })
    }
}

fn id_token(claims: &Value) -> String {
    format!(
        "{}.{}.sig",
        B.encode(br#"{"alg":"RS256"}"#),
        B.encode(claims.to_string())
    )
}

fn google_claims(sub: &str, email: &str, verified: bool) -> Value {
    json!({
        "iss": "https://accounts.google.com",
        "aud": "test-google-client",
        "sub": sub,
        "email": email,
        "email_verified": verified,
        "exp": Utc::now().timestamp() + 60,
    })
}

fn google_world(signups_open: bool) -> (ServerState, Arc<FakeGoogleTransport>) {
    let state = ServerState::in_memory_with_options(ServerOptions {
        signups_default: signups_open,
        ..ServerOptions::default()
    });
    owner_account(&state);
    let transport = Arc::new(FakeGoogleTransport::default());
    state.set_google_sign_in(
        GoogleOAuthConfig {
            client_id: "test-google-client".to_string(),
            client_secret: "test-google-secret".to_string(),
            auth_uri: "https://accounts.google.test/authorize".to_string(),
            token_uri: "https://oauth2.googleapis.com/token".to_string(),
            ..GoogleOAuthConfig::default()
        },
        transport.clone(),
    );
    (state, transport)
}

async fn no_redirect_get(server: &HttpTestServer, path: &str) -> reqwest::Response {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
        .get(format!("{}{path}", server.base_url))
        .send()
        .await
        .expect("GET without redirects")
}

fn query_param(url: &str, name: &str) -> String {
    url::Url::parse(url)
        .expect("valid URL")
        .query_pairs()
        .find_map(|(key, value)| (key == name).then(|| value.into_owned()))
        .unwrap_or_else(|| panic!("missing {name} query parameter"))
}

fn state_with_mailer() -> (ServerState, Arc<RecordingMailer>) {
    let mailer = Arc::new(RecordingMailer::default());
    let state = ServerState::in_memory_with_options(ServerOptions {
        mailer: mailer.clone(),
        ..ServerOptions::default()
    });
    (state, mailer)
}

async fn post_json(server: &HttpTestServer, path: &str, body: Value) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("{}{path}", server.base_url))
        .header("origin", "https://deskmate.test")
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .expect("JSON request")
}

async fn get_json(server: &HttpTestServer, path: &str) -> Value {
    let response = reqwest::Client::new()
        .get(format!("{}{path}", server.base_url))
        .send()
        .await
        .expect("GET request");
    json_body(response).await
}

fn token_from_link(link: &str) -> String {
    url::Url::parse(link)
        .expect("valid sign-in URL")
        .query_pairs()
        .find_map(|(key, value)| (key == "token").then(|| value.into_owned()))
        .expect("sign-in URL token")
}

async fn wait_for_mail(mailer: &RecordingMailer, count: usize) {
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if mailer.sent.lock().unwrap().len() >= count {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("mailer did not record the message");
}

#[tokio::test]
async fn instance_reports_google_only_when_google_sign_in_is_set() {
    let disabled = ServerState::in_memory();
    owner_account(&disabled);
    let disabled_server = spawn_http(app(disabled)).await;
    assert_eq!(
        get_json(&disabled_server, "/v1/app/instance").await["google_enabled"],
        false
    );
    assert_eq!(
        no_redirect_get(&disabled_server, "/v1/app/auth/google/start")
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        no_redirect_get(
            &disabled_server,
            "/v1/app/auth/google/callback?state=nope&code=c",
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );

    let (enabled, _transport) = google_world(false);
    let enabled_server = spawn_http(app(enabled)).await;
    assert_eq!(
        get_json(&enabled_server, "/v1/app/instance").await["google_enabled"],
        true
    );
}

#[tokio::test]
async fn google_sign_in_links_to_the_account_with_the_same_verified_email() {
    let (state, transport) = google_world(false);
    let server = spawn_http(app(state.clone())).await;
    let start = no_redirect_get(&server, "/v1/app/auth/google/start").await;
    assert_eq!(start.status(), StatusCode::SEE_OTHER);
    let location = start.headers()["location"].to_str().unwrap();
    let state_param = query_param(location, "state");
    let challenge = query_param(location, "code_challenge");
    assert_eq!(query_param(location, "scope"), "openid email");
    assert_eq!(
        query_param(location, "redirect_uri"),
        "https://deskmate.test/v1/app/auth/google/callback"
    );

    transport.answer_id_token(&google_claims("g-1", "owner@example.com", true));
    let callback = no_redirect_get(
        &server,
        &format!("/v1/app/auth/google/callback?state={state_param}&code=c"),
    )
    .await;
    assert_eq!(callback.status(), StatusCode::SEE_OTHER);
    assert_eq!(callback.headers()["location"], "/");
    assert!(
        callback.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .starts_with("__Host-deskmate_session=")
    );
    assert!(
        state
            .identity()
            .account_for_google("g-1")
            .unwrap()
            .is_some()
    );

    let calls = transport.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "https://oauth2.googleapis.com/token");
    let form = calls[0]
        .1
        .iter()
        .cloned()
        .collect::<std::collections::HashMap<_, _>>();
    assert_eq!(form.get("code").map(String::as_str), Some("c"));
    assert_eq!(
        form.get("redirect_uri").map(String::as_str),
        Some("https://deskmate.test/v1/app/auth/google/callback")
    );
    let verifier = form.get("code_verifier").expect("PKCE verifier");
    assert_eq!(B.encode(Sha256::digest(verifier.as_bytes())), challenge);
}

#[tokio::test]
async fn google_sign_in_does_not_create_accounts_while_signups_are_closed() {
    let (state, transport) = google_world(false);
    let server = spawn_http(app(state.clone())).await;
    let start = no_redirect_get(&server, "/v1/app/auth/google/start").await;
    let state_param = query_param(start.headers()["location"].to_str().unwrap(), "state");
    transport.answer_id_token(&google_claims("g-stranger", "stranger@example.com", true));

    let callback = no_redirect_get(
        &server,
        &format!("/v1/app/auth/google/callback?state={state_param}&code=c"),
    )
    .await;
    assert_eq!(callback.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        callback.headers()["location"],
        "/?signin_error=signups-closed"
    );
    assert!(callback.headers().get("set-cookie").is_none());
    assert!(
        state
            .identity()
            .account_by_email("stranger@example.com")
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn google_sign_in_refuses_an_unverified_email() {
    let (state, transport) = google_world(true);
    let server = spawn_http(app(state.clone())).await;
    let start = no_redirect_get(&server, "/v1/app/auth/google/start").await;
    let state_param = query_param(start.headers()["location"].to_str().unwrap(), "state");
    transport.answer_id_token(&google_claims("g-unverified", "person@example.com", false));

    let callback = no_redirect_get(
        &server,
        &format!("/v1/app/auth/google/callback?state={state_param}&code=c"),
    )
    .await;
    assert_eq!(callback.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        callback.headers()["location"],
        "/?signin_error=email-unverified"
    );
    assert!(
        state
            .identity()
            .account_by_email("person@example.com")
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn a_google_callback_with_an_unknown_state_is_refused_without_a_token_call() {
    let (state, transport) = google_world(false);
    let server = spawn_http(app(state)).await;
    let callback = no_redirect_get(&server, "/v1/app/auth/google/callback?state=nope&code=c").await;
    assert_eq!(callback.status(), StatusCode::SEE_OTHER);
    assert_eq!(callback.headers()["location"], "/?signin_error=expired");
    assert!(transport.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn google_callback_failures_are_rate_limited_per_ip() {
    let (state, _transport) = google_world(false);
    let server = spawn_http(app(state)).await;
    for _ in 0..5 {
        let response =
            no_redirect_get(&server, "/v1/app/auth/google/callback?state=nope&code=c").await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
    }
    let limited = no_redirect_get(&server, "/v1/app/auth/google/callback?state=nope&code=c").await;
    assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn first_run_setup_creates_the_owner_once() {
    let (state, _mail) = state_with_mailer();
    let code = state.setup_code_for_tests().unwrap();
    let server = spawn_http(app(state)).await;

    let instance = get_json(&server, "/v1/app/instance").await;
    assert_eq!(
        instance,
        json!({
            "setup_required": true,
            "google_enabled": false,
            "signups_open": false,
            "edition": "self-hosted",
        })
    );
    let wrong = post_json(
        &server,
        "/v1/app/setup",
        json!({"code": "WRONG-CODE", "email": "o@example.com"}),
    )
    .await;
    assert_eq!(wrong.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json_body(wrong).await,
        json!({"error": "That setup code is not right."})
    );

    let ok = post_json(
        &server,
        "/v1/app/setup",
        json!({"code": code.to_lowercase(), "email": "o@example.com"}),
    )
    .await;
    assert_eq!(ok.status(), StatusCode::OK);
    assert!(
        ok.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .starts_with("__Host-deskmate_session=")
    );
    let account = json_body(ok).await["account"].clone();
    assert!(account["id"].as_str().unwrap().starts_with("acc_"));
    assert_eq!(account["email"], "o@example.com");
    assert_eq!(account["email_verified"], false);
    assert_eq!(account["is_instance_owner"], true);
    assert_eq!(account.as_object().unwrap().len(), 4);
    assert_eq!(
        post_json(
            &server,
            "/v1/app/setup",
            json!({"code": code, "email": "x@example.com"}),
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get_json(&server, "/v1/app/instance").await["setup_required"],
        false
    );
}

#[tokio::test]
async fn setup_code_guesses_are_rate_limited() {
    let (state, _mail) = state_with_mailer();
    let server = spawn_http(app(state)).await;
    for _ in 0..5 {
        assert_eq!(
            post_json(
                &server,
                "/v1/app/setup",
                json!({"code": "AAAA-AAAA", "email": "o@example.com"}),
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        post_json(
            &server,
            "/v1/app/setup",
            json!({"code": "AAAA-AAAA", "email": "o@example.com"}),
        )
        .await
        .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn sign_in_email_does_not_reveal_accounts_and_respects_closed_signups() {
    let (state, mailer) = state_with_mailer();
    owner_account(&state);
    let server = spawn_http(app(state)).await;

    let known = post_json(
        &server,
        "/v1/app/auth/email",
        json!({"email": "  OWNER@example.com "}),
    )
    .await;
    assert_eq!(known.status(), StatusCode::ACCEPTED);
    let known_body = known.text().await.unwrap();
    let unknown = post_json(
        &server,
        "/v1/app/auth/email",
        json!({"email": "stranger@example.com"}),
    )
    .await;
    assert_eq!(unknown.status(), StatusCode::ACCEPTED);
    let unknown_body = unknown.text().await.unwrap();

    assert_eq!(known_body, unknown_body);
    wait_for_mail(&mailer, 1).await;
    let sent = mailer.sent.lock().unwrap();
    assert_eq!(sent.len(), 1, "only the existing account is mailed");
    assert_eq!(sent[0].0, "owner@example.com");
    assert!(sent[0].1.starts_with("https://deskmate.test/signin?token="));
}

#[tokio::test]
async fn a_link_signs_in_once_and_verifies_the_email() {
    let (state, mailer) = state_with_mailer();
    owner_account(&state);
    let server = spawn_http(app(state)).await;
    post_json(
        &server,
        "/v1/app/auth/email",
        json!({"email": "owner@example.com"}),
    )
    .await;
    wait_for_mail(&mailer, 1).await;
    let token = token_from_link(&mailer.sent.lock().unwrap()[0].1);

    let first = post_json(&server, "/v1/app/auth/link", json!({"token": token})).await;
    assert_eq!(first.status(), StatusCode::OK);
    let body = json_body(first).await;
    assert_eq!(body["account"]["email_verified"], true);

    let second = post_json(&server, "/v1/app/auth/link", json!({"token": token})).await;
    assert_eq!(second.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json_body(second).await["error"],
        "This sign-in link has expired."
    );
}

#[tokio::test]
async fn an_older_unused_link_still_works_after_a_newer_one() {
    let (state, mailer) = state_with_mailer();
    owner_account(&state);
    let server = spawn_http(app(state)).await;
    post_json(
        &server,
        "/v1/app/auth/email",
        json!({"email": "owner@example.com"}),
    )
    .await;
    wait_for_mail(&mailer, 1).await;
    let first = token_from_link(&mailer.sent.lock().unwrap()[0].1);
    post_json(
        &server,
        "/v1/app/auth/email",
        json!({"email": "owner@example.com"}),
    )
    .await;
    wait_for_mail(&mailer, 2).await;

    assert_eq!(
        post_json(&server, "/v1/app/auth/link", json!({"token": first}),)
            .await
            .status(),
        StatusCode::OK
    );
}

/// A link mailed while sign-ups were open must not create an account after the
/// owner has closed them again: the switch governs account creation, not only
/// who gets mailed.
#[tokio::test]
async fn closing_signups_voids_an_unused_link_for_a_new_account() {
    let (state, mailer) = state_with_mailer();
    let owner = owner_account(&state);
    let server = spawn_http(app(state.clone())).await;
    {
        let response = cookie_request(
            &server,
            &owner,
            "PUT",
            "/v1/app/instance/signups",
            Some(json!({"open": true})),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }
    post_json(
        &server,
        "/v1/app/auth/email",
        json!({"email": "friend@example.com"}),
    )
    .await;
    wait_for_mail(&mailer, 1).await;
    let token = token_from_link(&mailer.sent.lock().unwrap()[0].1);
    let closed = cookie_request(
        &server,
        &owner,
        "PUT",
        "/v1/app/instance/signups",
        Some(json!({"open": false})),
    )
    .await;
    assert_eq!(closed.status(), StatusCode::OK);

    let response = post_json(&server, "/v1/app/auth/link", json!({"token": token})).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json_body(response).await["error"],
        "This sign-in link has expired."
    );
    assert!(
        state
            .identity()
            .account_by_email("friend@example.com")
            .unwrap()
            .is_none(),
        "no account was created"
    );
}

#[tokio::test]
async fn open_signups_create_an_account_from_a_link() {
    let (state, mailer) = state_with_mailer();
    let owner = owner_account(&state);
    let server = spawn_http(app(state)).await;
    let opened = cookie_request(
        &server,
        &owner,
        "PUT",
        "/v1/app/instance/signups",
        Some(json!({"open": true})),
    )
    .await;
    assert_eq!(opened.status(), StatusCode::OK);
    assert_eq!(json_body(opened).await, json!({"signups_open": true}));
    post_json(
        &server,
        "/v1/app/auth/email",
        json!({"email": "friend@example.com"}),
    )
    .await;
    wait_for_mail(&mailer, 1).await;
    let token = token_from_link(&mailer.sent.lock().unwrap()[0].1);
    let response = post_json(&server, "/v1/app/auth/link", json!({"token": token})).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        json_body(response).await["account"]["is_instance_owner"],
        false
    );
}

#[tokio::test]
async fn sign_in_emails_are_rate_limited_per_address() {
    let (state, mailer) = state_with_mailer();
    owner_account(&state);
    let server = spawn_http(app(state)).await;
    for _ in 0..5 {
        assert_eq!(
            post_json(
                &server,
                "/v1/app/auth/email",
                json!({"email": "owner@example.com"}),
            )
            .await
            .status(),
            StatusCode::ACCEPTED
        );
    }
    let response = post_json(
        &server,
        "/v1/app/auth/email",
        json!({"email": "owner@example.com"}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    wait_for_mail(&mailer, 5).await;
    assert_eq!(mailer.sent.lock().unwrap().len(), 5);
}

#[tokio::test]
async fn recovery_link_needs_the_admin_bearer() {
    let (state, _mailer) = state_with_mailer();
    owner_account(&state);
    let server = spawn_http(app(state)).await;
    let client = reqwest::Client::new();
    let url = format!("{}/v1/admin/signin-link", server.base_url);

    assert_eq!(
        client
            .post(&url)
            .header("content-type", "application/json")
            .body(json!({"email": "owner@example.com"}).to_string())
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let response = client
        .post(&url)
        .bearer_auth(IN_MEMORY_ADMIN_TOKEN)
        .header("content-type", "application/json")
        .body(json!({"email": "owner@example.com"}).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let link = json_body(response).await["link"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        post_json(
            &server,
            "/v1/app/auth/link",
            json!({"token": token_from_link(&link)}),
        )
        .await
        .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn account_shape_is_exact_and_revoke_all_invalidates_every_session() {
    let (state, _mailer) = state_with_mailer();
    let owner = owner_account(&state);
    let other_session = new_session(&state, &owner);
    let owner_id = owner.id.0.clone();
    let server = spawn_http(app(state)).await;

    let response = cookie_request(&server, &owner, "GET", "/v1/app/account", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        json_body(response).await,
        json!({
            "id": owner_id,
            "email": "owner@example.com",
            "email_verified": true,
            "is_instance_owner": true,
        })
    );

    let revoked =
        cookie_request(&server, &owner, "POST", "/v1/app/sessions/revoke-all", None).await;
    assert_eq!(revoked.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        revoked.headers()["set-cookie"],
        "__Host-deskmate_session=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0"
    );
    for session in [&owner, &other_session] {
        assert_eq!(
            cookie_request(&server, session, "GET", "/v1/app/account", None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
}

#[tokio::test]
async fn no_session_is_401_and_a_wrong_origin_is_403() {
    let state = ServerState::in_memory();
    let account = owner_account(&state);
    let server = spawn_http(app(state)).await;
    let client = reqwest::Client::new();

    let anonymous = client
        .get(format!("{}/v1/app/devices", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    assert!(anonymous.bytes().await.unwrap().is_empty());

    let wrong = client
        .delete(format!("{}/v1/app/session", server.base_url))
        .header("cookie", &account.cookie)
        .header("origin", "https://evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), StatusCode::FORBIDDEN);

    let missing = client
        .delete(format!("{}/v1/app/session", server.base_url))
        .header("cookie", &account.cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(
        missing.status(),
        StatusCode::FORBIDDEN,
        "missing Origin on an unsafe method"
    );
}

#[tokio::test]
async fn sign_out_deletes_the_session_server_side() {
    let state = ServerState::in_memory();
    let account = owner_account(&state);
    let server = spawn_http(app(state)).await;

    let signed_out = cookie_request(&server, &account, "DELETE", "/v1/app/session", None).await;
    assert_eq!(signed_out.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        signed_out.headers()["set-cookie"],
        "__Host-deskmate_session=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0"
    );
    assert_eq!(
        cookie_request(&server, &account, "GET", "/v1/app/devices", None)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn the_admin_bearer_is_not_a_browser_session() {
    let state = ServerState::in_memory();
    owner_account(&state);
    let server = spawn_http(app(state)).await;
    let client = reqwest::Client::new();

    let response = client
        .get(format!("{}/v1/app/devices", server.base_url))
        .bearer_auth(IN_MEMORY_ADMIN_TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    for path in ["/v1/app/session", "/v1/session/login", "/v1/manage/login"] {
        let response = client
            .post(format!("{}{path}", server.base_url))
            .header("origin", "https://deskmate.test")
            .header("content-type", "application/json")
            .body(serde_json::json!({"token": IN_MEMORY_ADMIN_TOKEN}).to_string())
            .send()
            .await
            .unwrap();
        assert!(
            matches!(response.status().as_u16(), 404 | 405),
            "{path} still trades the admin token"
        );
    }
}

#[tokio::test]
async fn a_second_sign_in_does_not_invalidate_the_first_session() {
    let state = ServerState::in_memory();
    let first = owner_account(&state);
    let second = new_session(&state, &first);
    let server = spawn_http(app(state)).await;

    assert_eq!(
        cookie_request(&server, &first, "GET", "/v1/app/devices", None)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        cookie_request(&server, &second, "GET", "/v1/app/devices", None)
            .await
            .status(),
        StatusCode::OK
    );
}
