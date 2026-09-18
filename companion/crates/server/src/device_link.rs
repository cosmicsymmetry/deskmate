//! `GET /v1/device/link` -- the networked-tier device's persistent,
//! single-owner runtime link.

use axum::extract::State;
use axum::extract::ws::{WebSocket, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tokio::sync::OwnedSemaphorePermit;

use app_core::{ConfigOrigin, LoadOutcome, RuntimeHandle};

use crate::auth::AuthenticatedDevice;
use crate::image_sources::ServerImageSourceHost;
use crate::registry::DeviceId;
use crate::runtime_device::WebSocketRuntimeDevice;
use crate::{LinkLease, ServerState};

/// Caps concurrent device links. A deployment normally has very few devices,
/// but this endpoint sits on a public tunnel, so a caller with a
/// compromised token must not be able to pin unbounded socket tasks and
/// runtimes.
///
/// Sized conservatively rather than from production measurements. It is
/// `<= MAX_CONCURRENT_REQUESTS` (`lib.rs`) by construction. The link permit
/// is acquired with `try_acquire_owned` while the request permit is held, so
/// that step never waits; the 101 then releases the request permit before the
/// long-lived socket loop begins. With no circular wait, the two pools cannot
/// deadlock each other.
pub(crate) const MAX_CONCURRENT_LINKS: usize = 32;

/// One maximum COBS-framed protocol frame, delimiter included. A WebSocket
/// message may concatenate smaller complete frames, but their combined size
/// remains bounded by this protocol-native ceiling.
const MAX_WS_MESSAGE_SIZE: usize = protocol::MAX_WIRE_FRAME;

pub(crate) async fn handler(
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

    let peer = if let Some(peer) = lease.link().attach_runtime() {
        peer
    } else {
        let device_config = state.configs().for_device(&device_id);
        let update = device_config.update.lock().await;
        let Some(config) = load_config(&device_config, &device_id).await else {
            return;
        };

        let (device, connector) = WebSocketRuntimeDevice::channel(device_id.clone());
        let peer = connector.attach();
        let image_sources = std::sync::Arc::clone(state.image_sources());
        let options = state.runtime_options();
        let runtime = tokio::task::spawn_blocking(move || {
            RuntimeHandle::start_with_image_source_host(
                config,
                Box::new(device),
                options,
                Some(Box::new(ServerImageSourceHost::new(image_sources))),
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
        lease
            .link()
            .set_runtime(std::sync::Arc::clone(&runtime), connector);
        drop(update);
        peer
    };

    peer.run(socket, &device_id, lease.link().last_seen_counter())
        .await;

    tracing::info!(device_id = %device_id, "device link closed");
}

async fn load_config(
    device_config: &std::sync::Arc<crate::store::DeviceConfig>,
    device_id: &str,
) -> Option<app_core::AppConfig> {
    let store = std::sync::Arc::clone(device_config);
    let load = tokio::task::spawn_blocking(move || store.store.load()).await;
    match load {
        Ok(LoadOutcome::Loaded { config, origin }) => {
            device_config.record_load(origin, None);
            Some(config)
        }
        Ok(LoadOutcome::Recovered {
            config,
            origin,
            error,
        }) => {
            device_config.record_load(origin, Some(crate::store::ConfigFallbackReason::Recovery));
            tracing::warn!(
                device_id = %device_id,
                ?origin,
                %error,
                "device config recovery selected a fallback"
            );
            Some(config)
        }
        Ok(LoadOutcome::ValidationFailed {
            config,
            origin,
            issues,
        }) => {
            device_config.record_load(
                origin,
                Some(crate::store::ConfigFallbackReason::ValidationFailed),
            );
            if origin == ConfigOrigin::LastGood {
                tracing::warn!(
                    device_id = %device_id,
                    ?origin,
                    issue_count = issues.len(),
                    "device config validation failed; using genuine last-good config"
                );
            } else {
                tracing::warn!(
                    device_id = %device_id,
                    ?origin,
                    issue_count = issues.len(),
                    "device config validation failed; using fallback config"
                );
            }
            Some(config)
        }
        Err(_) => {
            tracing::error!(device_id = %device_id, "device config loader panicked");
            None
        }
    }
}
