//! Malformed device traffic over the WebSocket transport. These tests run
//! entirely in-process; no board, tunnel, or external network is involved.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use protocol::{DeviceEvent, EventAction, EventKind, Frame, Message};
use server::{ServerState, app};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

mod support;

type DeviceSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn spawn() -> (String, server::registry::DeviceIdentity, String) {
    let state = ServerState::in_memory();
    let identity = state.registry().mint();
    let admin_token = state.admin_token().to_string();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app(state)).await.unwrap();
    });
    (
        format!("127.0.0.1:{}", address.port()),
        identity,
        admin_token,
    )
}

async fn connect(host: &str, identity: &server::registry::DeviceIdentity) -> DeviceSocket {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    loop {
        let request = http::Request::builder()
            .uri(format!("ws://{host}/v1/device/link"))
            .header("Authorization", format!("Bearer {}", identity.token))
            .header("Host", host)
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header(
                "Sec-WebSocket-Key",
                tokio_tungstenite::tungstenite::handshake::client::generate_key(),
            )
            .body(())
            .unwrap();
        if let Ok((mut socket, response)) = tokio_tungstenite::connect_async(request).await {
            assert_eq!(response.status(), 101);
            support::bootstrap_runtime(&mut socket).await;
            return socket;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "device ownership slot was not released after hostile disconnect"
        );
        tokio::task::yield_now().await;
    }
}

async fn assert_rejected(host: &str, identity: &server::registry::DeviceIdentity, bytes: Vec<u8>) {
    let mut socket = connect(host, identity).await;
    socket.send(WsMessage::Binary(bytes)).await.unwrap();
    let closed = timeout(Duration::from_secs(1), async {
        loop {
            match socket.next().await {
                None | Some(Err(_) | Ok(WsMessage::Close(_))) => return true,
                Some(Ok(WsMessage::Ping(payload))) => {
                    socket.send(WsMessage::Pong(payload)).await.unwrap();
                }
                Some(Ok(_)) => return false,
            }
        }
    })
    .await;
    assert_eq!(closed, Ok(true), "hostile frame did not close the link");
}

#[tokio::test]
async fn hostile_device_frames_are_bounded_and_concatenated_frames_decode() {
    let (host, identity, admin_token) = spawn().await;

    let mut corrupt_delimiter = protocol::encode_message(1, &Message::StatusRequest).unwrap();
    corrupt_delimiter.insert(corrupt_delimiter.len() / 2, 0);
    assert_rejected(&host, &identity, corrupt_delimiter).await;

    assert_rejected(
        &host,
        &identity,
        protocol::test_support::oversized_declared_payload(2),
    )
    .await;

    let unknown_type = protocol::encode_frame(&Frame::new(u8::MAX, 3, vec![0xa0])).unwrap();
    assert_rejected(&host, &identity, unknown_type).await;

    let mut truncated = protocol::encode_message(4, &Message::StatusRequest).unwrap();
    assert_eq!(truncated.pop(), Some(0));
    assert_rejected(&host, &identity, truncated).await;

    let event = |sequence| {
        Message::DeviceEvent(DeviceEvent {
            sequence,
            kind: EventKind::Tap,
            widget_id: "clock".to_string(),
            screen_id: "clock".to_string(),
            action: EventAction::StartPause,
            interrupt_token: None,
        })
    };
    let mut concatenated = protocol::encode_message(0, &event(10)).unwrap();
    concatenated.extend(protocol::encode_message(0, &event(12)).unwrap());
    let mut socket = connect(&host, &identity).await;
    socket.send(WsMessage::Binary(concatenated)).await.unwrap();
    support::answer_next_runtime_status(&mut socket).await;
    support::flush_socket(&mut socket).await;

    let client = reqwest::Client::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    loop {
        let response = client
            .get(format!("http://{host}/v1/devices/{}", identity.device_id))
            .bearer_auth(&admin_token)
            .send()
            .await
            .expect("device status request");
        let status: serde_json::Value =
            serde_json::from_str(&response.text().await.expect("device status response body"))
                .expect("device status JSON");
        if status["snapshot"]["device"]["counters"]["detected_event_gaps"] == 1 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "both concatenated DeviceEvents were not routed in sequence"
        );
        tokio::task::yield_now().await;
    }
}
