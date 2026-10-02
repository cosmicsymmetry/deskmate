mod support;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::prelude::{BASE64_URL_SAFE_NO_PAD as B, Engine as _};
use chrono::Utc;
use reqwest::StatusCode;
use serde_json::{Value, json};
use server::mailer::RecordingMailer;
use server::oauth::GoogleOAuthConfig;
use server::oauth::transport::{FetchResponse, OAuthFuture, OAuthTransport, TransportError};
use server::{Entitlements, ServerOptions, ServerState, app};
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

fn google_claims(sub: &str, email: &str, verified: bool, nonce: &str) -> Value {
    json!({
        "iss": "https://accounts.google.com",
        "aud": "test-google-client",
        "sub": sub,
        "email": email,
        "email_verified": verified,
        "nonce": nonce,
        "hd": "example.com",
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
    get_with_cookie(server, path, "").await
}

fn google_cookie(start: &reqwest::Response) -> String {
    start.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

async fn get_with_cookie(server: &HttpTestServer, path: &str, cookie: &str) -> reqwest::Response {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
        .get(format!("{}{path}", server.base_url))
        .header("cookie", cookie)
        .send()
        .await
        .expect("GET without redirects")
}

async fn google_sign_in(
    server: &HttpTestServer,
    transport: &FakeGoogleTransport,
    claims: impl FnOnce(&str) -> Value,
) -> (reqwest::Response, reqwest::Response) {
    let start = no_redirect_get(server, "/v1/app/auth/google/start").await;
    let location = start.headers()["location"].to_str().unwrap();
    let state = query_param(location, "state");
    transport.answer_id_token(&claims(&query_param(location, "nonce")));
    let callback = get_with_cookie(
        server,
        &format!("/v1/app/auth/google/callback?state={state}&code=c"),
        &google_cookie(&start),
    )
    .await;
    (start, callback)
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

use support::connect_server_device as connect_device;

async fn expect_socket_closed(socket: &mut support::DeviceSocket) {
    use futures_util::StreamExt as _;

    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match socket.next().await {
                None | Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_)) | Err(_)) => {
                    return;
                }
                Some(Ok(_)) => {}
            }
        }
    })
    .await
    .expect("device socket stayed open after revocation");
}

#[derive(Debug)]
struct OnePanel;

impl Entitlements for OnePanel {
    fn max_cards(&self, _account: &server::identity::AccountId) -> usize {
        app_core::MAX_CONFIG_CARDS
    }

    fn max_image_sources(&self, _account: &server::identity::AccountId) -> usize {
        app_core::config::MAX_IMAGE_SOURCES
    }

    fn max_panels(&self, _account: &server::identity::AccountId) -> Option<usize> {
        Some(1)
    }

    fn feature_enabled(&self, _account: &server::identity::AccountId, _feature: &str) -> bool {
        true
    }
}

#[tokio::test]
async fn a_claimed_panel_is_pending_until_it_links_then_active() {
    let state = ServerState::in_memory();
    let account = owner_account(&state);
    let server = spawn_http(app(state)).await;

    let claimed = cookie_request(&server, &account, "POST", "/v1/app/devices/claim", None).await;
    assert_eq!(claimed.status(), StatusCode::CREATED);
    let claimed = json_body(claimed).await;
    assert_eq!(claimed["link_url"], "wss://deskmate.test/v1/device/link");
    let rows =
        json_body(cookie_request(&server, &account, "GET", "/v1/app/devices", None).await).await;
    assert_eq!(rows[0]["state"], "pending");

    let _socket = connect_device(&server, claimed["token"].as_str().unwrap())
        .await
        .expect("claimed panel connects");
    let rows =
        json_body(cookie_request(&server, &account, "GET", "/v1/app/devices", None).await).await;
    assert_eq!(rows[0]["state"], "active");
}

#[tokio::test]
async fn claiming_respects_the_accounts_panel_limit() {
    let state = ServerState::in_memory_with_options(ServerOptions {
        entitlements: Arc::new(OnePanel),
        ..ServerOptions::default()
    });
    let account = owner_account(&state);
    let server = spawn_http(app(state)).await;

    assert_eq!(
        cookie_request(&server, &account, "POST", "/v1/app/devices/claim", None,)
            .await
            .status(),
        StatusCode::CREATED
    );
    let refused = cookie_request(&server, &account, "POST", "/v1/app/devices/claim", None).await;
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_body(refused).await,
        json!({"error": "Your account has reached its panel limit."})
    );
}

#[tokio::test]
async fn stale_pending_claims_are_revoked_after_a_day() {
    let state = ServerState::in_memory();
    let account = owner_account(&state);
    let pending = state.registry().mint().expect("mint pending panel");
    state
        .identity()
        .assign_device(
            &pending.device_id,
            &account.id,
            server::identity::DeviceState::Pending,
            Utc::now() - chrono::Duration::hours(25),
        )
        .expect("assign pending panel");

    assert_eq!(server::collect_stale_claims(&state, Utc::now()).await, 1);
    assert!(!state.registry().contains_device(&pending.device_id));
    assert!(
        state
            .identity()
            .device_owner(&pending.device_id)
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn removing_a_panel_revokes_it_and_drops_its_live_link() {
    let state = ServerState::in_memory();
    let account = owner_account(&state);
    let panel = mint_owned_device(&state, &account);
    let config_path = state
        .account_space(&account.id)
        .root
        .join("devices")
        .join(format!("{}.json", panel.device_id));
    std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    std::fs::write(&config_path, b"saved config").unwrap();
    let server = spawn_http(app(state.clone())).await;
    let mut socket = connect_device(&server, &panel.token)
        .await
        .expect("owned panel connects");

    assert_eq!(
        cookie_request(
            &server,
            &account,
            "DELETE",
            &format!("/v1/app/devices/{}", panel.device_id),
            None,
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    expect_socket_closed(&mut socket).await;
    assert!(!state.registry().contains_device(&panel.device_id));
    assert!(!config_path.exists());
    assert!(connect_device(&server, &panel.token).await.is_err());
}

#[tokio::test]
async fn deleting_an_account_removes_its_folder_and_refuses_its_panel() {
    let state = ServerState::in_memory();
    let owner = owner_account(&state);
    let account = second_account(&state, "b@example.com");
    let panel = mint_owned_device(&state, &account);
    let root = state.account_space(&account.id).root.clone();
    let server = spawn_http(app(state)).await;

    let owner_refused = cookie_request(&server, &owner, "DELETE", "/v1/app/account", None).await;
    assert_eq!(owner_refused.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_body(owner_refused).await,
        json!({"error": "Other accounts use this server. Remove them first."})
    );
    let deleted = cookie_request(&server, &account, "DELETE", "/v1/app/account", None).await;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        deleted.headers()["set-cookie"],
        "__Host-deskmate_session=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0"
    );
    assert!(!root.exists());
    assert!(connect_device(&server, &panel.token).await.is_err());
    assert_eq!(
        cookie_request(&server, &account, "GET", "/v1/app/devices", None)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_panel_linking_during_account_deletion_writes_nothing_into_the_deleted_folder() {
    let state = ServerState::in_memory();
    let _owner = owner_account(&state);
    let account = second_account(&state, "racing@example.com");
    let panel = mint_owned_device(&state, &account);
    let root = state.account_space(&account.id).root.clone();
    let server = spawn_http(app(state.clone())).await;
    let stop = Arc::new(AtomicBool::new(false));
    let attempts = Arc::new(AtomicUsize::new(0));

    let reconnect_server = HttpTestServer::at(server.base_url.clone());
    let reconnect_token = panel.token.clone();
    let reconnect_stop = Arc::clone(&stop);
    let reconnect_attempts = Arc::clone(&attempts);
    let reconnects = tokio::spawn(async move {
        while !reconnect_stop.load(Ordering::Acquire) {
            reconnect_attempts.fetch_add(1, Ordering::Release);
            if let Ok(socket) = connect_device(&reconnect_server, &reconnect_token).await {
                drop(socket);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });

    tokio::time::timeout(Duration::from_secs(1), async {
        while attempts.load(Ordering::Acquire) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("reconnect task did not start");
    assert_eq!(
        cookie_request(&server, &account, "DELETE", "/v1/app/account", None)
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    stop.store(true, Ordering::Release);
    reconnects.await.expect("reconnect task");

    assert!(!root.exists(), "a racing link recreated the account folder");
    assert!(
        !state.registry().contains_device(&panel.device_id),
        "the deleted account's device identity survived"
    );
    assert!(connect_device(&server, &panel.token).await.is_err());
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
    let (start, callback) = google_sign_in(&server, &transport, |nonce| {
        google_claims("g-1", "owner@example.com", true, nonce)
    })
    .await;
    assert_eq!(start.status(), StatusCode::SEE_OTHER);
    let location = start.headers()["location"].to_str().unwrap();
    let challenge = query_param(location, "code_challenge");
    assert_eq!(query_param(location, "scope"), "openid email");
    assert_eq!(
        query_param(location, "redirect_uri"),
        "https://deskmate.test/v1/app/auth/google/callback"
    );
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
    let (_, callback) = google_sign_in(&server, &transport, |nonce| {
        google_claims("g-stranger", "stranger@example.com", true, nonce)
    })
    .await;
    assert_eq!(callback.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        callback.headers()["location"],
        "/?signin_error=signups-closed"
    );
    assert!(
        callback
            .headers()
            .get_all("set-cookie")
            .iter()
            .all(|cookie| !cookie
                .to_str()
                .unwrap()
                .starts_with("__Host-deskmate_session="))
    );
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
    let (_, callback) = google_sign_in(&server, &transport, |nonce| {
        google_claims("g-unverified", "person@example.com", false, nonce)
    })
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
            "email_delivery": "email",
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
async fn deleting_the_last_account_restores_first_run_setup() {
    let state = ServerState::in_memory();
    let code = state.setup_code_for_tests().unwrap();
    let server = spawn_http(app(state.clone())).await;
    let setup = post_json(
        &server,
        "/v1/app/setup",
        json!({"code": code, "email": "first@example.com"}),
    )
    .await;
    assert_eq!(setup.status(), StatusCode::OK);
    let cookie = google_cookie(&setup);
    assert!(state.setup_code_for_tests().is_none());
    assert_eq!(
        get_json(&server, "/v1/app/instance").await["setup_required"],
        false
    );

    let deleted = reqwest::Client::new()
        .delete(format!("{}/v1/app/account", server.base_url))
        .header("origin", "https://deskmate.test")
        .header("cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    assert_eq!(state.identity().account_count().unwrap(), 0);
    assert_eq!(
        get_json(&server, "/v1/app/instance").await["setup_required"],
        true
    );
    let replacement = state
        .setup_code_for_tests()
        .expect("regenerated setup code");
    let setup_again = post_json(
        &server,
        "/v1/app/setup",
        json!({"code": replacement, "email": "next@example.com"}),
    )
    .await;
    assert_eq!(setup_again.status(), StatusCode::OK);
    let body = json_body(setup_again).await;
    assert_eq!(body["account"]["email"], "next@example.com");
    assert_eq!(body["account"]["is_instance_owner"], true);
    assert!(state.setup_code_for_tests().is_none());
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
    let account = state
        .identity()
        .create_account("owner@example.com", false, true, Utc::now())
        .unwrap();
    assert!(!account.email_verified);
    let server = spawn_http(app(state.clone())).await;
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
    assert!(
        state
            .identity()
            .account(&account.id)
            .unwrap()
            .unwrap()
            .email_verified
    );

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

async fn google_start_from(
    client: &reqwest::Client,
    server: &HttpTestServer,
    ip: &str,
) -> reqwest::Response {
    client
        .get(format!("{}/v1/app/auth/google/start", server.base_url))
        .header("x-forwarded-for", ip)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn google_starts_are_limited_per_client_without_spending_the_callback_budget() {
    let (state, transport) = google_world(false);
    let server = spawn_http(app(state)).await;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let first = google_start_from(&client, &server, "198.51.100.7").await;
    assert_eq!(first.status(), StatusCode::SEE_OTHER);
    for _ in 1..32 {
        assert_eq!(
            google_start_from(&client, &server, "198.51.100.7")
                .await
                .status(),
            StatusCode::SEE_OTHER
        );
    }
    for _ in 0..30 {
        let refused = google_start_from(&client, &server, "198.51.100.7").await;
        assert_eq!(refused.status(), StatusCode::TOO_MANY_REQUESTS);
        let retry = json_body(refused).await["retry_after_seconds"]
            .as_u64()
            .unwrap();
        assert!((1..=600).contains(&retry));
    }
    let location = first.headers()["location"].to_str().unwrap();
    let flow = query_param(location, "state");
    let nonce = query_param(location, "nonce");
    transport.answer_id_token(&google_claims("owner", "owner@example.com", true, &nonce));
    let callback = client
        .get(format!(
            "{}/v1/app/auth/google/callback?state={flow}&code=c",
            server.base_url
        ))
        .header("x-forwarded-for", "198.51.100.7")
        .header("cookie", google_cookie(&first))
        .send()
        .await
        .unwrap();
    assert_eq!(callback.status(), StatusCode::SEE_OTHER);
    assert_eq!(callback.headers()["location"], "/");
    assert!(
        callback
            .headers()
            .get_all("set-cookie")
            .iter()
            .any(|cookie| cookie
                .to_str()
                .unwrap()
                .starts_with("__Host-deskmate_session="))
    );
    // The successful callback frees a global pending slot, but not this client's budget.
    assert_eq!(
        google_start_from(&client, &server, "198.51.100.7")
            .await
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        google_start_from(&client, &server, "203.0.113.9")
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_google_starts_admit_only_the_client_limit() {
    let (state, transport) = google_world(false);
    let server = spawn_http(app(state)).await;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let barrier = Arc::new(tokio::sync::Barrier::new(40));
    let mut starts = tokio::task::JoinSet::new();
    for _ in 0..40 {
        let barrier = barrier.clone();
        let client = client.clone();
        let server = HttpTestServer::at(server.base_url.clone());
        starts.spawn(async move {
            barrier.wait().await;
            google_start_from(&client, &server, "198.51.100.7").await
        });
    }
    let mut admitted = Vec::new();
    while let Some(response) = starts.join_next().await {
        let response = response.unwrap();
        match response.status() {
            StatusCode::SEE_OTHER => admitted.push(response),
            StatusCode::TOO_MANY_REQUESTS => {
                // A bare 429 from the global pending cap would hide a missing client limiter.
                let retry = json_body(response).await["retry_after_seconds"]
                    .as_u64()
                    .unwrap();
                assert!((1..=600).contains(&retry));
            }
            status => panic!("unexpected start status: {status}"),
        }
    }
    assert_eq!(admitted.len(), 32);
    let first = &admitted[0];
    let location = first.headers()["location"].to_str().unwrap();
    let flow = query_param(location, "state");
    let nonce = query_param(location, "nonce");
    transport.answer_id_token(&google_claims("owner", "owner@example.com", true, &nonce));
    let callback = client
        .get(format!(
            "{}/v1/app/auth/google/callback?state={flow}&code=c",
            server.base_url
        ))
        .header("x-forwarded-for", "198.51.100.7")
        .header("cookie", google_cookie(first))
        .send()
        .await
        .unwrap();
    assert_eq!(callback.status(), StatusCode::SEE_OTHER);
    assert_eq!(callback.headers()["location"], "/");
    assert_eq!(
        google_start_from(&client, &server, "198.51.100.7")
            .await
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        google_start_from(&client, &server, "203.0.113.9")
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
}

#[tokio::test]
async fn google_callback_is_bound_to_the_starting_browser_and_cannot_be_replayed() {
    let (state, transport) = google_world(false);
    let server = spawn_http(app(state)).await;
    let start = no_redirect_get(&server, "/v1/app/auth/google/start").await;
    let location = start.headers()["location"].to_str().unwrap();
    let state_param = query_param(location, "state");
    let path = format!("/v1/app/auth/google/callback?state={state_param}&code=c");
    let cookie = google_cookie(&start);
    let set_cookie = start.headers()["set-cookie"].to_str().unwrap();
    for attribute in [
        "HttpOnly",
        "Secure",
        "SameSite=Lax",
        "Path=/",
        "Max-Age=600",
    ] {
        assert!(set_cookie.contains(attribute));
    }
    transport.answer_id_token(&google_claims(
        "g-1",
        "owner@example.com",
        true,
        &query_param(location, "nonce"),
    ));
    for other_cookie in ["", "__Host-deskmate_google=other-browser"] {
        let response = get_with_cookie(&server, &path, other_cookie).await;
        assert_eq!(response.headers()["location"], "/?signin_error=expired");
        assert!(response.headers().get("set-cookie").is_none());
        assert!(transport.calls.lock().unwrap().is_empty());
    }
    // A mismatched browser must not burn the real browser's pending state.
    let response = get_with_cookie(&server, &path, &cookie).await;
    assert_eq!(response.headers()["location"], "/");
    assert!(
        response
            .headers()
            .get_all("set-cookie")
            .iter()
            .any(|value| {
                let value = value.to_str().unwrap();
                value.starts_with("__Host-deskmate_google=;") && value.contains("Max-Age=0")
            })
    );
    let replay = get_with_cookie(&server, &path, &cookie).await;
    assert_eq!(replay.headers()["location"], "/?signin_error=expired");
    assert_eq!(transport.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn google_callback_rejects_a_nonce_from_another_sign_in() {
    let (state, transport) = google_world(true);
    let server = spawn_http(app(state.clone())).await;
    let (_, response) = google_sign_in(&server, &transport, |_| {
        google_claims("g-new", "new@example.com", true, "wrong-nonce")
    })
    .await;
    assert_eq!(response.headers()["location"], "/?signin_error=provider");
    assert!(
        state
            .identity()
            .account_for_google("g-new")
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn instance_reports_the_actual_mail_delivery_mode() {
    let server = spawn_http(app(ServerState::in_memory())).await;
    assert_eq!(
        get_json(&server, "/v1/app/instance").await["email_delivery"],
        "server-log"
    );
    let (state, _) = state_with_mailer();
    let server = spawn_http(app(state)).await;
    assert_eq!(
        get_json(&server, "/v1/app/instance").await["email_delivery"],
        "email"
    );
}

#[tokio::test]
async fn google_does_not_link_a_third_party_email_without_current_mailbox_proof() {
    let (state, transport) = google_world(false);
    let server = spawn_http(app(state.clone())).await;
    let (_, response) = google_sign_in(&server, &transport, |nonce| {
        let mut claims = google_claims("unlinked", "owner@example.com", true, nonce);
        claims.as_object_mut().unwrap().remove("hd");
        claims
    })
    .await;
    assert_eq!(
        response.headers()["location"],
        "/?signin_error=email-link-required"
    );
    assert!(
        state
            .identity()
            .account_for_google("unlinked")
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn google_creates_an_account_when_open_and_returns_to_it_after_registration_closes() {
    let (state, transport) = google_world(true);
    let server = spawn_http(app(state.clone())).await;
    for (email, signups_open) in [("new@gmail.com", true), ("changed@example.org", false)] {
        state.identity().set_signups_open(signups_open).unwrap();
        let (_, response) = google_sign_in(&server, &transport, |nonce| {
            let mut claims = google_claims("stable-google-user", email, true, nonce);
            claims.as_object_mut().unwrap().remove("hd");
            claims
        })
        .await;
        assert_eq!(response.headers()["location"], "/");
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(response.headers()["referrer-policy"], "no-referrer");
        assert_eq!(
            state
                .identity()
                .account_for_google("stable-google-user")
                .unwrap()
                .unwrap()
                .email,
            "new@gmail.com"
        );
    }
    assert_eq!(state.identity().account_count().unwrap(), 2);
}

#[tokio::test]
async fn email_registration_cannot_bypass_first_run_owner_setup() {
    let mailer = Arc::new(RecordingMailer::default());
    let state = ServerState::in_memory_with_options(ServerOptions {
        signups_default: true,
        mailer: mailer.clone(),
        ..ServerOptions::default()
    });
    let code = state.setup_code_for_tests().unwrap();
    let server = spawn_http(app(state.clone())).await;
    let response = post_json(
        &server,
        "/v1/app/auth/email",
        json!({"email": "early@example.com"}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    tokio::task::yield_now().await;
    assert!(
        mailer.sent.lock().unwrap().is_empty(),
        "registration must wait for the setup code owner"
    );
    // Also reject a previously issued or administrator-created token until setup completes.
    let token = state
        .identity()
        .create_login_token("early@example.com", Utc::now())
        .unwrap();
    let response = post_json(&server, "/v1/app/auth/link", json!({"token": token})).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(state.identity().account_count().unwrap(), 0);
    let setup = post_json(
        &server,
        "/v1/app/setup",
        json!({"code": code, "email": "owner@example.com"}),
    )
    .await;
    assert_eq!(setup.status(), StatusCode::OK);
    assert_eq!(json_body(setup).await["account"]["is_instance_owner"], true);
}

#[tokio::test]
async fn google_registration_cannot_bypass_first_run_owner_setup() {
    let (state, transport) = google_world(true);
    let owner = state
        .identity()
        .account_by_email("owner@example.com")
        .unwrap()
        .unwrap();
    state.identity().delete_account(&owner.id).unwrap();
    let server = spawn_http(app(state.clone())).await;
    let (_, response) = google_sign_in(&server, &transport, |nonce| {
        google_claims("early-google", "early@gmail.com", true, nonce)
    })
    .await;
    assert_eq!(
        response.headers()["location"],
        "/?signin_error=setup-required"
    );
    assert_eq!(state.identity().account_count().unwrap(), 0);
    assert_eq!(
        get_json(&server, "/v1/app/instance").await["setup_required"],
        true
    );
}
