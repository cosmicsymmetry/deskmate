//! Malformed device traffic over the WebSocket transport. These tests run
//! entirely in-process; no board, tunnel, or external network is involved.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use protocol::{Frame, Message, decode_message, decode_wire_frame};
use server::{ServerState, app};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type DeviceSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

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

async fn connect(host: &str, identity: &server::registry::DeviceIdentity) -> DeviceSocket {
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
    let (socket, response) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(response.status(), 101);
    socket
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

async fn receive_two_status_responses(socket: &mut DeviceSocket) -> Vec<u32> {
    timeout(Duration::from_secs(1), async {
        let mut request_ids = Vec::new();
        while request_ids.len() < 2 {
            match socket.next().await {
                Some(Ok(WsMessage::Binary(wire))) => {
                    let frame = decode_wire_frame(&wire).unwrap();
                    assert!(matches!(
                        decode_message(&frame).unwrap(),
                        Message::StatusResponse(_)
                    ));
                    request_ids.push(frame.request_id);
                }
                Some(Ok(WsMessage::Ping(payload))) => {
                    socket.send(WsMessage::Pong(payload)).await.unwrap();
                }
                Some(Ok(other)) => panic!("unexpected WebSocket message: {other:?}"),
                Some(Err(error)) => panic!("WebSocket read failed: {error}"),
                None => panic!("WebSocket closed before both responses arrived"),
            }
        }
        request_ids
    })
    .await
    .expect("timed out waiting for two status responses")
}

#[tokio::test]
async fn hostile_device_frames_are_bounded_and_concatenated_frames_decode() {
    let (host, identity) = spawn().await;

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

    let mut concatenated = protocol::encode_message(10, &Message::StatusRequest).unwrap();
    concatenated.extend(protocol::encode_message(11, &Message::StatusRequest).unwrap());
    let mut socket = connect(&host, &identity).await;
    socket.send(WsMessage::Binary(concatenated)).await.unwrap();
    assert_eq!(receive_two_status_responses(&mut socket).await, [10, 11]);
}
