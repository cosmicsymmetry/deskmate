//! The device-facing surface. These tests never touch a board: they exercise
//! the same three endpoints the firmware will call, over a loopback listener.

use server::{ServerState, app};

async fn spawn() -> (String, server::registry::DeviceIdentity) {
    let state = ServerState::in_memory();
    let identity = state.registry().mint().expect("mint identity");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(state)).await.unwrap();
    });
    (format!("127.0.0.1:{}", address.port()), identity)
}

fn link_request(host: &str, authorization: Option<&str>) -> http::Request<()> {
    let mut request = http::Request::builder()
        .uri(format!("ws://{host}/v1/device/link"))
        .header("Host", host)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tokio_tungstenite::tungstenite::handshake::client::generate_key(),
        );
    if let Some(authorization) = authorization {
        request = request.header("Authorization", authorization);
    }
    request.body(()).unwrap()
}

#[tokio::test]
async fn device_link_accepts_a_minted_token() {
    let (host, identity) = spawn().await;
    let authorization = format!("Bearer {}", identity.token);
    let request = link_request(&host, Some(&authorization));
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
        let request = link_request(&host, authorization);
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
