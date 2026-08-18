//! `GET /v1/device/link` -- the networked-tier device's persistent link.
//!
//! This task's handler does the bare minimum to make the contract real: it
//! authenticates the upgrade, logs which device connected, and then holds
//! the socket open, discarding whatever the device sends. Task 9 replaces
//! the body of [`run`] with the `app-core` ownership runtime; nothing here
//! should grow logic that runtime will need to own instead.

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;

use crate::ServerState;
use crate::auth::AuthenticatedDevice;
use crate::registry::DeviceId;

pub async fn handler(
    State(_state): State<ServerState>,
    AuthenticatedDevice { device_id }: AuthenticatedDevice,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| run(socket, device_id))
}

/// Holds the link open. Does nothing else -- see the module doc.
async fn run(mut socket: WebSocket, device_id: DeviceId) {
    tracing::info!(device_id = %device_id, "device link established");

    while let Some(message) = socket.recv().await {
        match message {
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(_) => {
                // Task 9 hands inbound frames to the ownership runtime.
                // Until then the link is held open and nothing is echoed.
            }
        }
    }

    tracing::info!(device_id = %device_id, "device link closed");
}
