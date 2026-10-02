//! The device-facing surface. These tests never touch a board: they exercise
//! the same three endpoints the firmware will call, over a loopback listener.

use server::{ServerState, app};

mod support;

async fn spawn() -> (String, server::registry::DeviceIdentity) {
    let state = ServerState::in_memory();
    let owner = support::owner_account(&state);
    let identity = support::mint_owned_device(&state, &owner);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(state)).await.unwrap();
    });
    (format!("127.0.0.1:{}", address.port()), identity)
}

#[tokio::test]
async fn device_link_accepts_a_minted_token() {
    let (host, identity) = spawn().await;
    let authorization = format!("Bearer {}", identity.token);
    let request = support::link_request(&host, Some(&authorization));
    let (_stream, response) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(response.status(), 101);
}

#[tokio::test]
async fn device_link_refuses_invalid_authorization() {
    for (name, authorization) in [
        ("unknown-token", Some("Bearer not-a-real-token")),
        ("missing-authorization-header", None),
    ] {
        let (host, _identity) = spawn().await;
        let request = support::link_request(&host, authorization);
        assert!(
            tokio_tungstenite::connect_async(request).await.is_err(),
            "{name}"
        );
    }
}

#[tokio::test]
async fn firmware_check_reports_no_update_when_current() {
    let (host, identity) = spawn().await;
    let body = reqwest::Client::new()
        .get(format!("http://{host}/v1/device/firmware?current=1.0.0"))
        .bearer_auth(&identity.token)
        .send()
        .await
        .unwrap();
    assert_eq!(body.status(), 204);
}

#[tokio::test]
async fn firmware_check_refuses_an_unknown_token() {
    let (host, _identity) = spawn().await;
    let response = reqwest::Client::new()
        .get(format!("http://{host}/v1/device/firmware?current=1.0.0"))
        .bearer_auth("not-a-real-token")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    assert!(response.bytes().await.unwrap().is_empty());
}

/// A valid token whose device no account owns is refused: nothing may write a
/// config into an account folder that does not exist, which is what makes
/// deleting an account safe against a panel reconnecting mid-deletion.
#[tokio::test]
async fn device_link_refuses_a_device_no_account_owns() {
    let state = ServerState::in_memory();
    support::owner_account(&state);
    let orphan = state.registry().mint().expect("mint without an owner");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
    tokio::spawn(async move {
        axum::serve(listener, app(state)).await.unwrap();
    });
    let authorization = format!("Bearer {}", orphan.token);
    let error =
        tokio_tungstenite::connect_async(support::link_request(&host, Some(&authorization)))
            .await
            .expect_err("an unowned device must not link");
    assert!(error.to_string().contains("401"), "{error}");
}
