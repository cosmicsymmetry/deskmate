//! `GET /v1/device/link` -- the networked-tier device's persistent,
//! single-owner runtime link.

use axum::extract::State;
use axum::extract::ws::{WebSocket, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tokio::sync::OwnedSemaphorePermit;

use app_core::runtime::CardTapSink;
use app_core::{ConfigOrigin, LoadOutcome, RuntimeHandle};

use crate::AccountSpace;
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
const UNOWNED_LINK_WARNING_INTERVAL_SECS: u64 = 60;
static UNOWNED_LINK_WARNING_CLOCK: LazyLock<Instant> = LazyLock::new(Instant::now);
static LAST_UNOWNED_LINK_WARNING: AtomicU64 = AtomicU64::new(u64::MAX);

fn should_warn_unowned_link() -> bool {
    let now = UNOWNED_LINK_WARNING_CLOCK.elapsed().as_secs();
    let mut previous = LAST_UNOWNED_LINK_WARNING.load(Ordering::Relaxed);
    loop {
        if previous != u64::MAX && now.saturating_sub(previous) < UNOWNED_LINK_WARNING_INTERVAL_SECS
        {
            return false;
        }
        match LAST_UNOWNED_LINK_WARNING.compare_exchange(
            previous,
            now,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return true,
            Err(current) => previous = current,
        }
    }
}

/// Leaves the app-core runtime worker immediately and performs server routing on
/// Tokio. The runtime worker also owns the device link, so it must never wait on
/// the data-card mutex or catalog lookup.
struct ServerTapSink {
    state: ServerState,
    space: std::sync::Arc<AccountSpace>,
    runtime: tokio::runtime::Handle,
}

impl CardTapSink for ServerTapSink {
    fn tapped(&self, _card_id: &str, source_id: &str) {
        let state = self.state.clone();
        let space = std::sync::Arc::clone(&self.space);
        let source_id = source_id.to_owned();
        std::mem::drop(self.runtime.spawn(async move {
            crate::data_cards::tapped(&state, &space, &source_id);
        }));
    }
}

pub(crate) async fn handler(
    State(state): State<ServerState>,
    AuthenticatedDevice { device_id }: AuthenticatedDevice,
    ws: WebSocketUpgrade,
) -> Response {
    let ownership_state = state.clone();
    let ownership_device_id = device_id.clone();
    let owned = tokio::task::spawn_blocking(move || {
        ownership_state.with_device_lifecycle(|| {
            let space = ownership_state.space_for_device(&ownership_device_id);
            if space.is_some() {
                ownership_state
                    .identity()
                    .activate_device(&ownership_device_id)?;
            }
            Ok::<_, crate::identity::IdentityError>(space)
        })
    })
    .await;
    let space = match owned {
        Ok(Ok(Some(space))) => space,
        Ok(Ok(None)) => {
            if should_warn_unowned_link() {
                tracing::warn!(
                    target: "deskmate_server::device_link",
                    device_id = %device_id,
                    "device link refused: identity has no owning account"
                );
            }
            return StatusCode::UNAUTHORIZED.into_response();
        }
        Ok(Err(error)) => {
            tracing::error!(target: "deskmate_server::device_link", device_id = %device_id, %error,
                "device ownership could not be activated");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Err(error) => {
            tracing::error!(target: "deskmate_server::device_link", device_id = %device_id, %error,
                "device ownership lookup panicked");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let Some(lease) = state.claim_link(device_id.clone(), space.account_id.clone()) else {
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
        .on_upgrade(move |socket| run(socket, state, space, device_id, permit, lease))
}

async fn run(
    socket: WebSocket,
    state: ServerState,
    space: std::sync::Arc<AccountSpace>,
    device_id: DeviceId,
    _permit: OwnedSemaphorePermit,
    lease: LinkLease,
) {
    tracing::info!(device_id = %device_id, "device link established");

    let peer = if let Some(peer) = lease.link().attach_runtime() {
        peer
    } else {
        let device_config = space.configs.for_device(&device_id);
        let update = device_config.update.lock().await;
        let Some(config) = load_config(&device_config, &device_id).await else {
            return;
        };

        let (device, connector) = WebSocketRuntimeDevice::channel(device_id.clone());
        let peer = connector.attach();
        let image_sources = std::sync::Arc::clone(&space.image_sources);
        let options = state.runtime_options();
        let tap_sink: std::sync::Arc<dyn CardTapSink> = std::sync::Arc::new(ServerTapSink {
            state: state.clone(),
            space: std::sync::Arc::clone(&space),
            runtime: tokio::runtime::Handle::current(),
        });
        let runtime = tokio::task::spawn_blocking(move || {
            RuntimeHandle::start_with_ports(
                config,
                Box::new(device),
                options,
                Some(Box::new(ServerImageSourceHost::new(image_sources))),
                Some(tap_sink),
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
            device_config.record_load(origin, None, None);
            Some(config)
        }
        Ok(LoadOutcome::Recovered {
            config,
            origin,
            error,
        }) => {
            device_config.record_load(
                origin,
                Some(crate::store::ConfigFallbackReason::Recovery),
                Some(error.clone()),
            );
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
                Some(app_core::StoreError::Validation {
                    issues: issues.clone(),
                }),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_runtime_tap_callback_does_not_wait_for_server_routing() {
        let state = ServerState::in_memory();
        let account = state
            .identity()
            .create_account("owner@example.com", true, true, chrono::Utc::now())
            .expect("create account");
        let space = state.account_space(&account.id);
        let sink = std::sync::Arc::new(ServerTapSink {
            state: state.clone(),
            space: std::sync::Arc::clone(&space),
            runtime: tokio::runtime::Handle::current(),
        });
        let data_cards = space.data_cards.lock().expect("data cards");
        let (returned, observed) = std::sync::mpsc::channel();
        let caller = std::thread::spawn(move || {
            sink.tapped("picture", "external-producer");
            returned.send(()).expect("observe callback return");
        });

        let callback_returned = observed.recv_timeout(std::time::Duration::from_millis(200));
        drop(data_cards);
        caller.join().expect("callback thread");
        state.shutdown();

        assert!(
            callback_returned.is_ok(),
            "the app-core worker callback waited on the server's data-card mutex"
        );
    }
}
