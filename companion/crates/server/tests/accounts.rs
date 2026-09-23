mod support;

use reqwest::StatusCode;
use server::{ServerState, app};
use support::*;

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
