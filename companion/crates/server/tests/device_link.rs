//! The device-facing surface. These tests never touch a board: they exercise
//! the same three endpoints the firmware will call, over a loopback listener.

use server::{ServerState, app};

async fn spawn() -> (String, server::registry::DeviceIdentity) {
    let state = ServerState::in_memory();
    let identity = state.registry().mint();
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
    let request = http::Request::builder()
        .uri(format!("ws://{host}/v1/device/link"))
        .header("Authorization", format!("Bearer {}", identity.token))
        .header("Host", &host)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tokio_tungstenite::tungstenite::handshake::client::generate_key(),
        )
        .body(())
        .unwrap();
    let (_stream, response) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(response.status(), 101);
}

#[tokio::test]
async fn device_link_refuses_an_unknown_token() {
    let (host, _identity) = spawn().await;
    let request = http::Request::builder()
        .uri(format!("ws://{host}/v1/device/link"))
        .header("Authorization", "Bearer not-a-real-token")
        .header("Host", &host)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tokio_tungstenite::tungstenite::handshake::client::generate_key(),
        )
        .body(())
        .unwrap();
    assert!(tokio_tungstenite::connect_async(request).await.is_err());
}

#[tokio::test]
async fn device_link_refuses_a_missing_authorization_header() {
    let (host, _identity) = spawn().await;
    let request = http::Request::builder()
        .uri(format!("ws://{host}/v1/device/link"))
        .header("Host", &host)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tokio_tungstenite::tungstenite::handshake::client::generate_key(),
        )
        .body(())
        .unwrap();
    assert!(tokio_tungstenite::connect_async(request).await.is_err());
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
}
