#![allow(dead_code)]

use futures_util::{SinkExt, StreamExt};
use protocol::{Ack, ApplyConfig, HeartbeatAck, Message, StatusResponse};
use tokio_tungstenite::tungstenite::Message as WsMessage;

pub type DeviceSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

pub fn link_request(host: &str, authorization: Option<&str>) -> http::Request<()> {
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

// The error belongs to tungstenite; retain it so callers can assert HTTP refusals.
#[allow(clippy::result_large_err)]
pub async fn connect_device(
    host: &str,
    token: &str,
) -> Result<DeviceSocket, tokio_tungstenite::tungstenite::Error> {
    let request = link_request(host, Some(&format!("Bearer {token}")));
    tokio_tungstenite::connect_async(request)
        .await
        .map(|(socket, _)| socket)
}

#[allow(clippy::result_large_err)]
pub async fn connect_server_device(
    server: &HttpTestServer,
    token: &str,
) -> Result<DeviceSocket, tokio_tungstenite::tungstenite::Error> {
    let host = server
        .base_url
        .strip_prefix("http://")
        .expect("test server uses HTTP");
    connect_device(host, token).await
}

pub const IN_MEMORY_ADMIN_TOKEN: &str = "in-memory-admin-token";

#[derive(Clone)]
pub struct TestAccount {
    pub id: server::identity::AccountId,
    pub cookie: String,
}

pub fn owner_account(state: &server::ServerState) -> TestAccount {
    let account = state
        .identity()
        .accounts()
        .expect("list test accounts")
        .into_iter()
        .find(|account| account.is_instance_owner)
        .unwrap_or_else(|| {
            state
                .identity()
                .create_account("owner@example.com", true, true, chrono::Utc::now())
                .expect("create instance owner")
        });
    test_account(state, account)
}

pub fn second_account(state: &server::ServerState, email: &str) -> TestAccount {
    let account = state
        .identity()
        .create_account(email, true, false, chrono::Utc::now())
        .expect("create second account");
    test_account(state, account)
}

fn test_account(state: &server::ServerState, account: server::identity::Account) -> TestAccount {
    let sid = state
        .identity()
        .create_session(&account.id, chrono::Utc::now())
        .expect("create account session");
    TestAccount {
        id: account.id,
        cookie: format!("__Host-deskmate_session={sid}"),
    }
}

pub fn new_session(state: &server::ServerState, account: &TestAccount) -> TestAccount {
    let sid = state
        .identity()
        .create_session(&account.id, chrono::Utc::now())
        .expect("create another account session");
    TestAccount {
        id: account.id.clone(),
        cookie: format!("__Host-deskmate_session={sid}"),
    }
}

pub async fn cookie_request(
    server: &HttpTestServer,
    account: &TestAccount,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> reqwest::Response {
    let method = reqwest::Method::from_bytes(method.as_bytes()).expect("HTTP method");
    let mut request = reqwest::Client::new()
        .request(method, format!("{}{path}", server.base_url))
        .header("cookie", &account.cookie)
        .header("origin", "https://deskmate.test");
    if let Some(body) = body {
        request = request
            .header("content-type", "application/json")
            .body(body.to_string());
    }
    request.send().await.expect("account request")
}

pub fn mint_owned_device(
    state: &server::ServerState,
    account: &TestAccount,
) -> server::registry::DeviceIdentity {
    let identity = state.registry().mint().expect("mint device identity");
    state
        .identity()
        .assign_device(
            &identity.device_id,
            &account.id,
            server::identity::DeviceState::Active,
            chrono::Utc::now(),
        )
        .expect("assign device owner");
    identity
}

pub struct HttpTestServer {
    pub base_url: String,
    account: Option<TestAccount>,
}

impl HttpTestServer {
    pub fn at(base_url: String) -> Self {
        Self {
            base_url,
            account: None,
        }
    }

    pub fn with_account(mut self, account: TestAccount) -> Self {
        self.account = Some(account);
        self
    }

    pub fn account(&self) -> &TestAccount {
        self.account.as_ref().expect("test server account")
    }
}

pub async fn spawn_http(router: axum::Router) -> HttpTestServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let address = listener.local_addr().expect("test server address");
    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .expect("serve test routes");
    });
    HttpTestServer::at(format!("http://127.0.0.1:{}", address.port()))
}

/// Installs the fake faces package (`fake-faces.sh` beside this file), so a test of
/// the face routes exercises the real subprocess seam without Bun or the network.
/// The crate's own unit tests get this automatically; an integration target builds
/// the library without `cfg(test)`, so it has to ask.
pub fn with_fake_faces(state: server::ServerState) -> server::ServerState {
    let script =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/fake-faces.sh");
    server::set_faces(&state, server::FaceCommand::program(script));
    state
}

pub async fn json_body(response: reqwest::Response) -> serde_json::Value {
    let text = response.text().await.expect("response body");
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("body is not JSON ({error}): {text}"))
}

pub fn replace_file_with_directory(path: &std::path::Path) {
    std::fs::rename(path, path.with_extension("backup")).expect("preserve original file");
    std::fs::create_dir(path).expect("blocking directory");
}

async fn drive_until<T>(
    socket: &mut DeviceSocket,
    what: &str,
    mut step: impl FnMut(Message) -> Option<T>,
) -> T {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match socket.next().await {
                Some(Ok(WsMessage::Binary(bytes))) => {
                    let frame = protocol::decode_wire_frame(&bytes)
                        .expect("device received a malformed frame");
                    let message = protocol::decode_message(&frame)
                        .expect("device received an undecodable message");
                    reply(socket, frame.request_id, &message).await;
                    if let Some(result) = step(message) {
                        return result;
                    }
                }
                Some(Ok(WsMessage::Ping(payload))) => {
                    socket.send(WsMessage::Pong(payload)).await.unwrap();
                }
                Some(Ok(other)) => panic!("unexpected WebSocket message: {other:?}"),
                Some(Err(error)) => panic!("WebSocket read failed: {error}"),
                None => panic!("socket closed before the expected {what} arrived"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for the expected {what}"))
}

pub async fn drive_until_config(socket: &mut DeviceSocket, card_id: &str) -> ApplyConfig {
    let mut target_config = None;
    drive_until(socket, "config", |message| {
        match message {
            Message::ApplyConfig(config)
                if config.cards.iter().any(|card| card.card_id == card_id) =>
            {
                target_config = Some(config);
            }
            Message::ActivateCard(_) => return target_config.take(),
            _ => {}
        }
        None
    })
    .await
}

pub async fn drive_until_push(socket: &mut DeviceSocket, card_id: &str) -> protocol::PushTimer {
    drive_until(socket, "timer push", |message| match message {
        Message::PushTimer(push) if push.card_id == card_id => Some(push),
        _ => None,
    })
    .await
}

pub async fn drive_until_scene(socket: &mut DeviceSocket) -> protocol::PushScene {
    drive_until(socket, "scene", |message| match message {
        Message::PushScene(push) => Some(push),
        _ => None,
    })
    .await
}

/// Completes runtime synchronization after either an initial connection or a
/// reconnect. A reconnect replays retained state, then performs the same
/// scheduled status/time synchronization as the initial connection.
pub async fn bootstrap_runtime(socket: &mut DeviceSocket) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match socket.next().await {
                Some(Ok(WsMessage::Binary(bytes))) => {
                    let frame = protocol::decode_wire_frame(&bytes)
                        .expect("device received a malformed frame during bootstrap");
                    let message = protocol::decode_message(&frame)
                        .expect("device received an undecodable message during bootstrap");
                    let complete = matches!(message, Message::ActivateCard(_));
                    reply(socket, frame.request_id, &message).await;
                    if complete {
                        finish_initial_schedule(socket).await;
                        flush_socket(socket).await;
                        return;
                    }
                }
                Some(Ok(WsMessage::Ping(payload))) => {
                    socket.send(WsMessage::Pong(payload)).await.unwrap();
                }
                Some(Ok(other)) => panic!("unexpected bootstrap WebSocket message: {other:?}"),
                Some(Err(error)) => panic!("bootstrap WebSocket read failed: {error}"),
                None => panic!("socket closed before runtime bootstrap completed"),
            }
        }
    })
    .await
    .expect("timed out bootstrapping the runtime");
}

async fn finish_initial_schedule(socket: &mut DeviceSocket) {
    let mut saw_status = false;
    let mut saw_time_sync = false;
    while !saw_status || !saw_time_sync {
        match socket.next().await {
            Some(Ok(WsMessage::Binary(bytes))) => {
                let frame = protocol::decode_wire_frame(&bytes).expect("initial schedule frame");
                let message = protocol::decode_message(&frame).expect("initial schedule message");
                saw_status |= matches!(message, Message::StatusRequest);
                saw_time_sync |= matches!(message, Message::TimeSync(_));
                reply(socket, frame.request_id, &message).await;
            }
            Some(Ok(WsMessage::Ping(payload))) => {
                socket.send(WsMessage::Pong(payload)).await.unwrap();
            }
            Some(Ok(other)) => panic!("unexpected initial-schedule message: {other:?}"),
            Some(Err(error)) => panic!("initial-schedule read failed: {error}"),
            None => panic!("socket closed before initial scheduled work completed"),
        }
    }
}

pub async fn answer_next_runtime_status(socket: &mut DeviceSocket) {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            match socket.next().await {
                Some(Ok(WsMessage::Binary(bytes))) => {
                    let frame = protocol::decode_wire_frame(&bytes).expect("runtime status frame");
                    let message = protocol::decode_message(&frame).expect("runtime status message");
                    let is_status = matches!(message, Message::StatusRequest);
                    reply(socket, frame.request_id, &message).await;
                    if is_status {
                        return;
                    }
                }
                Some(Ok(WsMessage::Ping(payload))) => {
                    socket.send(WsMessage::Pong(payload)).await.unwrap();
                }
                Some(Ok(other)) => panic!("unexpected status WebSocket message: {other:?}"),
                Some(Err(error)) => panic!("status WebSocket read failed: {error}"),
                None => panic!("socket closed before the runtime status request"),
            }
        }
    })
    .await
    .expect("timed out waiting for app-core's status request");
}

pub async fn flush_socket(socket: &mut DeviceSocket) {
    const PROBE: &[u8] = b"deskmate-test-flush";
    socket.send(WsMessage::Ping(PROBE.into())).await.unwrap();
    loop {
        match socket.next().await {
            Some(Ok(WsMessage::Pong(payload))) if payload.as_ref() == PROBE => return,
            Some(Ok(WsMessage::Binary(bytes))) => {
                let frame = protocol::decode_wire_frame(&bytes).expect("probe frame");
                let message = protocol::decode_message(&frame).expect("probe message");
                reply(socket, frame.request_id, &message).await;
            }
            Some(Ok(WsMessage::Ping(payload))) => {
                socket.send(WsMessage::Pong(payload)).await.unwrap();
            }
            Some(Ok(other)) => panic!("unexpected probe WebSocket message: {other:?}"),
            Some(Err(error)) => panic!("probe WebSocket read failed: {error}"),
            None => panic!("socket closed before flush completed"),
        }
    }
}

async fn reply(socket: &mut DeviceSocket, request_id: u32, request: &Message) {
    let response = match request {
        Message::StatusRequest => Message::StatusResponse(sample_status()),
        Message::TimeSync(_) => Message::Ack(Ack {
            acknowledged_type: protocol::TYPE_TIME_SYNC,
            revision: None,
            already_present: None,
        }),
        Message::ApplyConfig(config) => Message::Ack(Ack {
            acknowledged_type: protocol::TYPE_APPLY_CONFIG,
            revision: Some(config.revision),
            already_present: None,
        }),
        Message::PushTimer(push) => Message::Ack(Ack {
            acknowledged_type: protocol::TYPE_PUSH_TIMER,
            revision: Some(push.revision),
            already_present: None,
        }),
        Message::ActivateCard(_) => Message::Ack(Ack {
            acknowledged_type: protocol::TYPE_ACTIVATE_CARD,
            revision: None,
            already_present: None,
        }),
        Message::TriggerInterrupt(_) => Message::Ack(Ack {
            acknowledged_type: protocol::TYPE_TRIGGER_INTERRUPT,
            revision: None,
            already_present: None,
        }),
        Message::PushScene(push) => Message::Ack(Ack {
            acknowledged_type: protocol::TYPE_PUSH_SCENE,
            revision: Some(push.revision),
            already_present: None,
        }),
        Message::Heartbeat => Message::HeartbeatAck(HeartbeatAck { uptime_ms: 1_234 }),
        other => panic!("server sent an unexpected device request: {other:?}"),
    };
    socket
        .send(WsMessage::Binary(
            protocol::encode_message(request_id, &response)
                .unwrap()
                .into(),
        ))
        .await
        .unwrap();
}

fn sample_status() -> StatusResponse {
    StatusResponse {
        // A device that reached this server over WSS advertised networking
        // capability to get here. Use the full shipping set so this shared
        // fixture cannot describe an impossible tunnel peer or mask the next
        // host-side capability gate.
        capabilities: protocol::CURRENT_CAPABILITIES,
        firmware_version: "test-device".to_owned(),
        uptime_ms: 1_234,
        free_heap: 5_678,
        brightness: 128,
        valid_frames: 1,
        tier: protocol::Tier::Networked,
        wifi_state: protocol::WifiState::Connected,
        wifi_rssi: -42,
        ip: "192.0.2.10".to_owned(),
        last_ota_error: Some("download: ESP_ERR_NO_MEM".to_owned()),
        ..protocol::test_support::sample_status_response()
    }
}
