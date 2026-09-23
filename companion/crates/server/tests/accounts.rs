mod support;

use std::sync::Arc;

use reqwest::StatusCode;
use serde_json::{Value, json};
use server::mailer::RecordingMailer;
use server::{ServerOptions, ServerState, app};
use support::*;

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
