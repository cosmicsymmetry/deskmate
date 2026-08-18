//! `GET /v1/device/link` -- the networked-tier device's persistent,
//! single-owner runtime link.

use axum::extract::State;
use axum::extract::ws::{WebSocket, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tokio::sync::OwnedSemaphorePermit;

use app_core::{LoadOutcome, RuntimeHandle, RuntimeOptions, SystemCalendarRefresher};

use crate::auth::AuthenticatedDevice;
use crate::registry::DeviceId;
use crate::runtime_device::WebSocketRuntimeDevice;
use crate::{LinkLease, ServerState};

/// Coarse process protection for authenticated but long-lived sockets.
pub(crate) const MAX_CONCURRENT_LINKS: usize = 32;

/// One maximum COBS-framed protocol frame, delimiter included. A WebSocket
/// message may concatenate smaller complete frames, but their combined size
/// remains bounded by this protocol-native ceiling.
const MAX_WS_MESSAGE_SIZE: usize = protocol::MAX_WIRE_FRAME;

pub async fn handler(
    State(state): State<ServerState>,
    AuthenticatedDevice { device_id }: AuthenticatedDevice,
    ws: WebSocketUpgrade,
) -> Response {
    let Some(lease) = state.claim_link(device_id.clone()) else {
        tracing::warn!(device_id = %device_id, "device link refused: owner already live");
        return StatusCode::CONFLICT.into_response();
    };
    let Ok(permit) = state.link_slots().try_acquire_owned() else {
        tracing::warn!(
            device_id = %device_id,
            "device link refused: at MAX_CONCURRENT_LINKS capacity"
        );
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };

    ws.max_message_size(MAX_WS_MESSAGE_SIZE)
        .max_frame_size(MAX_WS_MESSAGE_SIZE)
        .on_upgrade(move |socket| run(socket, state, device_id, permit, lease))
}

async fn run(
    socket: WebSocket,
    state: ServerState,
    device_id: DeviceId,
    _permit: OwnedSemaphorePermit,
    lease: LinkLease,
) {
    tracing::info!(device_id = %device_id, "device link established");

    let device_config = state.configs().for_device(&device_id);
    let update = device_config.update.lock().await;
    let store = std::sync::Arc::clone(&device_config);
    let load = tokio::task::spawn_blocking(move || store.store.load()).await;
    let config = match load {
        Ok(LoadOutcome::Loaded { config, .. }) => config,
        Ok(LoadOutcome::Recovered { config, error, .. }) => {
            tracing::warn!(device_id = %device_id, %error, "device config recovered");
            config
        }
        Ok(LoadOutcome::ValidationFailed { config, issues, .. }) => {
            tracing::warn!(
                device_id = %device_id,
                issue_count = issues.len(),
                "device config validation failed; using genuine last-good config"
            );
            config
        }
        Err(_) => {
            tracing::error!(device_id = %device_id, "device config loader panicked");
            return;
        }
    };

    let (device, peer) = WebSocketRuntimeDevice::channel(device_id.clone());
    let runtime = tokio::task::spawn_blocking(move || {
        RuntimeHandle::start(
            config,
            Box::new(device),
            Box::<SystemCalendarRefresher>::default(),
            RuntimeOptions::default(),
        )
    })
    .await;
    let runtime = match runtime {
        Ok(Ok(runtime)) => std::sync::Arc::new(runtime),
        Ok(Err(app_core::RuntimeError::InvalidConfig { issues })) => {
            tracing::error!(
                device_id = %device_id,
                issue_count = issues.len(),
                "stored device config cannot start the runtime"
            );
            return;
        }
        Ok(Err(error)) => {
            tracing::error!(device_id = %device_id, %error, "device runtime failed to start");
            return;
        }
        Err(_) => {
            tracing::error!(device_id = %device_id, "device runtime starter panicked");
            return;
        }
    };
    lease.link().set_runtime(std::sync::Arc::clone(&runtime));
    drop(update);

    peer.run(socket, &device_id, lease.link().last_seen_counter())
        .await;

    let runtime = lease.link().take_runtime();
    if let Some(runtime) = runtime {
        match tokio::task::spawn_blocking(move || runtime.shutdown()).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::warn!(device_id = %device_id, %error, "device runtime shutdown failed");
            }
            Err(_) => {
                tracing::warn!(device_id = %device_id, "device runtime shutdown panicked");
            }
        }
    }
    tracing::info!(device_id = %device_id, "device link closed");
}
