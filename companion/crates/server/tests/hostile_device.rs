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

async fn spawn() -> (
    String,
    server::registry::DeviceIdentity,
    String,
    support::TestAccount,
) {
    // Paced for a test. This file is about hostile frames closing the link and
    // the ownership slot being released afterwards -- not about how long a
    // reconnect waits. With production pacing each of the four reconnects cost a
    // real 3 seconds (one of `reconnect_interval`, two of `status_interval`) and
    // this one test was 14 seconds, a third of the entire workspace suite.
    let state = ServerState::in_memory_with_runtime_options(app_core::RuntimeOptions {
        reconnect_interval: Duration::from_millis(10),
        status_interval: Duration::from_millis(10),
        pomodoro_interval: Duration::from_millis(10),
        ..app_core::RuntimeOptions::default()
    });
    let owner = support::owner_account(&state);
    let identity = support::mint_owned_device(&state, &owner);
    let admin_token = support::IN_MEMORY_ADMIN_TOKEN.to_string();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    (
        format!("127.0.0.1:{}", address.port()),
        identity,
        admin_token,
        owner,
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
    socket.send(WsMessage::Binary(bytes.into())).await.unwrap();
    let closed = timeout(Duration::from_secs(1), async {
        loop {
            match socket.next().await {
                None | Some(Err(_) | Ok(WsMessage::Close(_))) => return true,
                Some(Ok(WsMessage::Ping(payload))) => {
                    socket.send(WsMessage::Pong(payload)).await.unwrap();
                }
                // Scheduled requests can already be in flight when the hostile
                // frame arrives. They do not cancel the bounded closure check.
                Some(Ok(_)) => {}
            }
        }
    })
    .await;
    assert_eq!(closed, Ok(true), "hostile frame did not close the link");
}

#[tokio::test]
async fn hostile_device_frames_are_bounded_and_concatenated_frames_decode() {
    let (host, identity, admin_token, _owner) = spawn().await;

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
            card_id: "clock".to_string(),
            action: EventAction::StartPause,
            interrupt_token: None,
        })
    };
    let mut concatenated = protocol::encode_message(0, &event(10)).unwrap();
    concatenated.extend(protocol::encode_message(0, &event(12)).unwrap());
    let mut socket = connect(&host, &identity).await;
    socket
        .send(WsMessage::Binary(concatenated.into()))
        .await
        .unwrap();
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

/// A producer's push must not wait on device delivery.
///
/// The durable outcome of `POST /v1/images/{token}` is "the frame is stored",
/// and it is stored before the runtime is told anything. Delivery happens over a
/// link the producer has no relationship with and cannot act on, so awaiting it
/// made a successful push answer 504 once the reconcile grew `AssetRelease`'s
/// twenty-second budget -- somebody holding a curl command saw an error for work
/// that had succeeded, and would reasonably retry it.
///
/// This lives here rather than beside the other image-route tests because it
/// needs a device that is CONNECTED and silent. With no device attached the
/// notification returns instantly either way, so the same test over there passed
/// against the awaiting version too, and proved nothing.
/// A 448x368 all-black PNG: the smallest thing the ingest route accepts.
fn exact_png() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, 448, 368);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("header");
        writer
            .write_image_data(&vec![0u8; 448 * 368 * 3])
            .expect("data");
    }
    out
}

#[tokio::test]
async fn an_image_push_does_not_wait_for_a_silent_device() {
    let (host, identity, _admin_token, owner) = spawn().await;
    // Connected, bootstrapped, and from here on it answers nothing.
    let _socket = connect(&host, &identity).await;

    let client = reqwest::Client::new();
    let minted: serde_json::Value = client
        .post(format!("http://{host}/v1/images"))
        .header("cookie", &owner.cookie)
        .header("origin", "https://deskmate.test")
        .header("Content-Type", "application/json")
        .body(r#"{"name":"Panel"}"#)
        .send()
        .await
        .expect("mint")
        .text()
        .await
        .map(|body| serde_json::from_str(&body).expect("mint body"))
        .expect("mint body text");
    let token = minted["token"].as_str().expect("token").to_string();

    let started = std::time::Instant::now();
    let response = client
        .post(format!("http://{host}/v1/images/{token}"))
        .header("Content-Type", "image/png")
        .body(exact_png())
        .send()
        .await
        .expect("push");
    let elapsed = started.elapsed();

    assert_eq!(response.status(), 200);
    assert!(
        elapsed < Duration::from_secs(5),
        "the push waited on a silent device: {elapsed:?}"
    );
}
