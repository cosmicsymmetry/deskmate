//! The device-facing surface. These tests never touch a board: they exercise
//! the same three endpoints the firmware will call, over a loopback listener.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use protocol::{Message, OtaState, StatusResponse, Tier, WifiState};
use server::{ServerState, app};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message as WsMessage;

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

fn sample_status() -> StatusResponse {
    StatusResponse {
        protocol_version: protocol::PROTOCOL_VERSION,
        max_protocol_version: protocol::MAX_PROTOCOL_VERSION,
        capabilities: 0,
        firmware_version: "test-device".to_owned(),
        uptime_ms: 1234,
        free_heap: 5678,
        display_width: 320,
        display_height: 240,
        brightness: 128,
        rotation: 0,
        online: true,
        latest_revision: 0,
        valid_frames: 1,
        malformed_frames: 0,
        crc_errors: 0,
        overflow_frames: 0,
        dropped_responses: 0,
        rx_dropped_bytes: 0,
        dropped_events: 0,
        event_queue_high_water: 0,
        dropped_ui_commands: 0,
        ui_queue_high_water: 0,
        config_revision: 0,
        latest_interrupt_token: 0,
        tier: Tier::Networked,
        wifi_state: WifiState::Connected,
        wifi_rssi: -42,
        ip: "192.0.2.10".to_owned(),
        ota_state: OtaState::Idle,
        last_network_error: None,
    }
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
async fn device_link_periodically_round_trips_status() {
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
    let (mut socket, response) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(response.status(), 101);

    let request_id = timeout(Duration::from_secs(6), async {
        loop {
            match socket.next().await {
                Some(Ok(WsMessage::Binary(wire))) => {
                    let frame = protocol::decode_wire_frame(&wire).unwrap();
                    assert!(matches!(
                        protocol::decode_message(&frame).unwrap(),
                        Message::StatusRequest
                    ));
                    break frame.request_id;
                }
                Some(Ok(WsMessage::Ping(payload))) => {
                    socket.send(WsMessage::Pong(payload)).await.unwrap();
                }
                Some(Ok(other)) => panic!("unexpected WebSocket message: {other:?}"),
                Some(Err(error)) => panic!("WebSocket read failed: {error}"),
                None => panic!("device link closed before the periodic status request"),
            }
        }
    })
    .await
    .expect("timed out waiting for periodic status request");
    assert_ne!(request_id, 0);

    socket
        .send(WsMessage::Binary(
            protocol::encode_message(request_id, &Message::StatusResponse(sample_status()))
                .unwrap(),
        ))
        .await
        .unwrap();

    // A following request proves the decoded StatusResponse did not cause the
    // server to reject and close the device link.
    socket
        .send(WsMessage::Binary(
            protocol::encode_message(99, &Message::StatusRequest).unwrap(),
        ))
        .await
        .unwrap();
    timeout(Duration::from_secs(1), async {
        loop {
            match socket.next().await {
                Some(Ok(WsMessage::Binary(wire))) => {
                    let frame = protocol::decode_wire_frame(&wire).unwrap();
                    if frame.request_id == 99 {
                        assert!(matches!(
                            protocol::decode_message(&frame).unwrap(),
                            Message::StatusResponse(_)
                        ));
                        break;
                    }
                }
                Some(Ok(WsMessage::Ping(payload))) => {
                    socket.send(WsMessage::Pong(payload)).await.unwrap();
                }
                Some(Ok(other)) => panic!("unexpected WebSocket message: {other:?}"),
                Some(Err(error)) => panic!("WebSocket read failed: {error}"),
                None => panic!("device link closed after the status response"),
            }
        }
    })
    .await
    .expect("timed out waiting for status echo after periodic round trip");
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
