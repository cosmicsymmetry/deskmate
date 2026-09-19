#![allow(dead_code)]

use futures_util::{SinkExt, StreamExt};
use protocol::{Ack, ApplyConfig, HeartbeatAck, Message, StatusResponse};
use tokio_tungstenite::tungstenite::Message as WsMessage;

pub type DeviceSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

pub const IN_MEMORY_ADMIN_TOKEN: &str = "in-memory-admin-token";

pub async fn drive_until_config(socket: &mut DeviceSocket, card_id: &str) -> ApplyConfig {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut target_config = None;
        loop {
            match socket.next().await {
                Some(Ok(WsMessage::Binary(bytes))) => {
                    let frame = protocol::decode_wire_frame(&bytes)
                        .expect("device received a malformed frame");
                    let message = protocol::decode_message(&frame)
                        .expect("device received an undecodable message");
                    let target = match &message {
                        Message::ApplyConfig(config) => {
                            config.cards.iter().any(|card| card.card_id == card_id)
                        }
                        _ => false,
                    };
                    let completes_target_sync =
                        target_config.is_some() && matches!(&message, Message::ActivateCard(_));
                    reply(socket, frame.request_id, &message).await;
                    if target {
                        let Message::ApplyConfig(config) = message else {
                            unreachable!("target is true only for ApplyConfig");
                        };
                        target_config = Some(config);
                    } else if completes_target_sync {
                        return target_config
                            .take()
                            .expect("target config was recorded before activation");
                    }
                }
                Some(Ok(WsMessage::Ping(payload))) => {
                    socket.send(WsMessage::Pong(payload)).await.unwrap();
                }
                Some(Ok(other)) => panic!("unexpected WebSocket message: {other:?}"),
                Some(Err(error)) => panic!("WebSocket read failed: {error}"),
                None => panic!("socket closed before the expected config arrived"),
            }
        }
    })
    .await
    .expect("timed out waiting for the expected config")
}

pub async fn drive_until_push(socket: &mut DeviceSocket, card_id: &str) -> protocol::PushTimer {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match socket.next().await {
                Some(Ok(WsMessage::Binary(bytes))) => {
                    let frame = protocol::decode_wire_frame(&bytes)
                        .expect("device received a malformed frame");
                    let message = protocol::decode_message(&frame)
                        .expect("device received an undecodable message");
                    let target = matches!(
                        &message,
                        Message::PushTimer(push) if push.card_id == card_id
                    );
                    reply(socket, frame.request_id, &message).await;
                    if target {
                        let Message::PushTimer(push) = message else {
                            unreachable!("target is true only for PushTimer");
                        };
                        return push;
                    }
                }
                Some(Ok(WsMessage::Ping(payload))) => {
                    socket.send(WsMessage::Pong(payload)).await.unwrap();
                }
                Some(Ok(other)) => panic!("unexpected WebSocket message: {other:?}"),
                Some(Err(error)) => panic!("WebSocket read failed: {error}"),
                None => panic!("socket closed before the expected timer push arrived"),
            }
        }
    })
    .await
    .expect("timed out waiting for the expected timer push")
}

pub async fn drive_until_scene(socket: &mut DeviceSocket) -> protocol::PushScene {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match socket.next().await {
                Some(Ok(WsMessage::Binary(bytes))) => {
                    let frame = protocol::decode_wire_frame(&bytes)
                        .expect("device received a malformed frame");
                    let message = protocol::decode_message(&frame)
                        .expect("device received an undecodable message");
                    let target = matches!(&message, Message::PushScene(_));
                    reply(socket, frame.request_id, &message).await;
                    if target {
                        let Message::PushScene(push) = message else {
                            unreachable!("target is true only for PushScene");
                        };
                        return push;
                    }
                }
                Some(Ok(WsMessage::Ping(payload))) => {
                    socket.send(WsMessage::Pong(payload)).await.unwrap();
                }
                Some(Ok(other)) => panic!("unexpected WebSocket message: {other:?}"),
                Some(Err(error)) => panic!("WebSocket read failed: {error}"),
                None => panic!("socket closed before the expected scene arrived"),
            }
        }
    })
    .await
    .expect("timed out waiting for the expected scene")
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
