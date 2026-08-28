//! The Deskmate V2 server: a single-tenant stub that owns networked-tier
//! devices over a persistent WebSocket link and answers firmware-update
//! checks for both tiers.
//!
//! This crate's device-facing surface -- `/v1/device/link`,
//! `/v1/device/firmware`, and `/v1/firmware/{version}.bin` -- is the whole
//! contract the firmware ever sees. V3 replaces everything behind it
//! (accounts, storage, multi-tenancy) without touching that surface.
//!
//! Task 8 puts this behind a public Cloudflare tunnel, so `app()` treats
//! every caller as internet-facing and anonymous by default: a single
//! process-wide concurrency limit, load shedding, and a request timeout
//! guard every route, on top of the per-route defenses in `device_link` and
//! `firmware`.

mod admin;
pub mod asset_sync;
mod auth;
mod device_link;
pub mod egress;
pub mod firmware;
pub mod registry;
pub mod runtime_device;
mod store;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use app_core::RuntimeHandle;
use axum::Router;
use axum::error_handling::HandleErrorLayer;
use axum::http::StatusCode;
use axum::routing::get;
use tower::ServiceBuilder;
use tower::limit::GlobalConcurrencyLimitLayer;
use tower::load_shed::error::Overloaded;

use firmware::FirmwareCatalog;
use registry::{DEVICE_IDENTITY_STORE_FILE, Registry};
use runtime_device::SocketConnector;

/// Caps how many requests this process handles at once, across *every*
/// route -- genuinely process-wide, via [`GlobalConcurrencyLimitLayer`]
/// sharing one semaphore. Plain `ServiceBuilder::concurrency_limit` would
/// look identical but mint a fresh semaphore each time `Layer::layer()` is
/// called, and axum calls it once per route *and* per method slot
/// (including the automatic 405 fallback) -- roughly six independent
/// limiters for this router's three routes, none of which would mean what
/// this constant's name says. This is coarse, blanket protection on top of
/// the download route's own streaming (which is what actually bounds its
/// memory use) and the device link's own connection cap.
///
/// Provisional: sized by judgment for a single-device V2 deployment, not
/// derived from real numbers. Revisit against actual traffic in V3.
/// `device_link::MAX_CONCURRENT_LINKS` (32) is `<=` this value by
/// construction, and an upgrade request releases its permit here at the
/// 101 response -- before the long-lived socket loop begins -- so the two
/// caps cannot deadlock each other.
const MAX_CONCURRENT_REQUESTS: usize = 64;

/// How long any single request may take to produce a response. Generous for
/// a firmware-check round trip; irrelevant to an established device link,
/// whose handler returns as soon as the WebSocket upgrade completes -- the
/// long-lived socket loop runs independently afterward.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Shared server state, cheap to clone: every field is behind an `Arc`
/// (directly or via the outer `Arc<StateInner>`). Per-device entries carry the
/// single-owner `app-core` runtime alongside the persistent registry and
/// config stores. They outlive individual WebSocket links so live app-core
/// state is not a property of a transport connection.
#[derive(Clone)]
pub struct ServerState {
    inner: Arc<StateInner>,
}

struct StateInner {
    registry: Registry,
    admin_token: String,
    firmware: FirmwareCatalog,
    configs: store::DeviceConfigStores,
    /// Keeps the dedicated config root alive for [`ServerState::in_memory`].
    /// Production paths are operator-owned and leave this as `None`.
    _config_temp_dir: Option<tempfile::TempDir>,
    device_links: Mutex<HashMap<registry::DeviceId, Arc<LiveLink>>>,
    /// Bounds concurrent `/v1/device/link` connections. `Arc`-wrapped
    /// separately from `StateInner` because `Semaphore::try_acquire_owned`
    /// needs an owned `Arc<Semaphore>` to hand a `'static` permit to the
    /// socket task that outlives the handler which acquired it.
    link_slots: Arc<tokio::sync::Semaphore>,
}

impl ServerState {
    #[must_use]
    pub fn new(admin_token: String, firmware: FirmwareCatalog, config_directory: PathBuf) -> Self {
        let registry = Registry::load(config_directory.join(DEVICE_IDENTITY_STORE_FILE));
        Self::with_config_temp_dir(admin_token, firmware, config_directory, registry, None)
    }

    fn with_config_temp_dir(
        admin_token: String,
        firmware: FirmwareCatalog,
        config_directory: PathBuf,
        registry: Registry,
        config_temp_dir: Option<tempfile::TempDir>,
    ) -> Self {
        Self {
            inner: Arc::new(StateInner {
                registry,
                admin_token,
                firmware,
                configs: store::DeviceConfigStores::new(config_directory),
                _config_temp_dir: config_temp_dir,
                device_links: Mutex::new(HashMap::new()),
                link_slots: Arc::new(tokio::sync::Semaphore::new(
                    device_link::MAX_CONCURRENT_LINKS,
                )),
            }),
        }
    }

    /// A state suitable for tests: an empty registry, a fixed admin token,
    /// a firmware catalog pinned at version `1.0.0`, and independent owned
    /// temp roots for firmware and device configuration.
    #[must_use]
    pub fn in_memory() -> Self {
        let firmware = FirmwareCatalog::in_memory();
        let config_temp_dir = tempfile::tempdir()
            .expect("failed to create a dedicated temp config directory for server state");
        let config_directory = config_temp_dir.path().to_path_buf();
        Self::with_config_temp_dir(
            "in-memory-admin-token".to_string(),
            firmware,
            config_directory,
            Registry::new(),
            Some(config_temp_dir),
        )
    }

    #[must_use]
    pub fn registry(&self) -> &Registry {
        &self.inner.registry
    }

    #[must_use]
    pub fn admin_token(&self) -> &str {
        &self.inner.admin_token
    }

    /// Compares `presented` against the admin token in constant time. This
    /// is the *only* sanctioned way to check the admin token -- callers
    /// (Task 9's Mac-facing routes) must not compare `admin_token()` with
    /// `==` themselves, which would quietly undo this crate's
    /// constant-time-comparison guarantee for the one secret that protects
    /// every write.
    #[must_use]
    pub fn verify_admin_token(&self, presented: &str) -> bool {
        registry::constant_time_eq(self.admin_token().as_bytes(), presented.as_bytes())
    }

    #[must_use]
    pub fn firmware(&self) -> &FirmwareCatalog {
        &self.inner.firmware
    }

    /// A fresh handle to the device-link concurrency cap. Returns an
    /// `Arc<Semaphore>` (rather than `&Semaphore`) because acquiring an
    /// owned permit from it is what lets a permit outlive the handler that
    /// acquired it, for the life of one socket.
    pub(crate) fn link_slots(&self) -> Arc<tokio::sync::Semaphore> {
        Arc::clone(&self.inner.link_slots)
    }

    pub(crate) fn configs(&self) -> &store::DeviceConfigStores {
        &self.inner.configs
    }

    /// Atomically reserves the one live ownership slot for `device_id`.
    /// The reservation happens before the 101 response, so two simultaneous
    /// upgrades cannot both believe they won.
    pub(crate) fn claim_link(&self, device_id: registry::DeviceId) -> Option<LinkLease> {
        let link = {
            let mut links = self
                .inner
                .device_links
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            Arc::clone(links.entry(device_id).or_default())
        };
        if !link.claim() {
            return None;
        }
        Some(LinkLease { link })
    }

    pub(crate) fn device_link(&self, device_id: &str) -> Option<Arc<LiveLink>> {
        self.inner
            .device_links
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(device_id)
            .cloned()
    }

    /// Stops every per-device runtime retained by this server. The operation
    /// is idempotent and is called after Axum drains on process shutdown.
    pub fn shutdown(&self) {
        let links: Vec<_> = self
            .inner
            .device_links
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .cloned()
            .collect();
        for link in links {
            let Some(runtime) = link.take_runtime() else {
                continue;
            };
            runtime.connector.detach();
            if let Err(error) = runtime.handle.shutdown() {
                tracing::warn!(%error, "device runtime shutdown failed");
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct LiveLink {
    live: AtomicBool,
    runtime: Mutex<Option<ManagedRuntime>>,
    last_seen_unix_ms: Arc<AtomicU64>,
}

struct ManagedRuntime {
    handle: Arc<RuntimeHandle>,
    connector: SocketConnector,
}

impl LiveLink {
    fn claim(&self) -> bool {
        self.live
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    fn release(&self) {
        self.live.store(false, Ordering::Release);
    }

    pub(crate) fn is_live(&self) -> bool {
        self.live.load(Ordering::Acquire)
    }

    pub(crate) fn set_runtime(&self, runtime: Arc<RuntimeHandle>, connector: SocketConnector) {
        *self
            .runtime
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(ManagedRuntime {
            handle: runtime,
            connector,
        });
    }

    pub(crate) fn runtime(&self) -> Option<Arc<RuntimeHandle>> {
        self.runtime
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(|runtime| Arc::clone(&runtime.handle))
    }

    pub(crate) fn attach_runtime(&self) -> Option<runtime_device::SocketPeer> {
        self.runtime
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(|runtime| runtime.connector.attach())
    }

    pub(crate) fn last_ota_error(&self) -> Option<String> {
        self.runtime
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .and_then(|runtime| runtime.connector.last_ota_error())
    }

    fn take_runtime(&self) -> Option<ManagedRuntime> {
        self.runtime
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    pub(crate) fn last_seen_counter(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.last_seen_unix_ms)
    }

    pub(crate) fn last_seen_unix_ms(&self) -> Option<u64> {
        match self.last_seen_unix_ms.load(Ordering::Relaxed) {
            0 => None,
            seen => Some(seen),
        }
    }
}

pub(crate) struct LinkLease {
    link: Arc<LiveLink>,
}

impl LinkLease {
    pub(crate) fn link(&self) -> &Arc<LiveLink> {
        &self.link
    }
}

impl Drop for LinkLease {
    fn drop(&mut self) {
        self.link.release();
    }
}

/// Builds both the device-facing surface and the admin-token-protected
/// Mac-facing routes over `state`.
///
/// Middleware order matters and is easy to get backwards: `poll_ready`
/// cascades down through the *whole* nested stack before `call` ever runs
/// on the outermost layer, so a plain concurrency limiter blocks
/// `poll_ready` itself while waiting for a permit -- invisibly to any
/// `Timeout` layered "outside" it, since `Timeout::call` (which is what
/// starts its clock) never even begins until `poll_ready` has already
/// resolved. `load_shed()` is what actually bounds that wait: it makes
/// `poll_ready` succeed unconditionally and turns "the inner service isn't
/// ready" into an immediate rejection at `call` time instead of a block.
/// `load_shed()` must therefore sit directly outside the concurrency
/// limiter (not just anywhere above it), which is why it is threaded
/// between `HandleErrorLayer` and `GlobalConcurrencyLimitLayer` below.
pub fn app(state: ServerState) -> Router {
    let middleware = ServiceBuilder::new()
        .layer(HandleErrorLayer::new(handle_middleware_error))
        .load_shed()
        .layer(GlobalConcurrencyLimitLayer::new(MAX_CONCURRENT_REQUESTS))
        .timeout(REQUEST_TIMEOUT);

    Router::new()
        .route("/v1/device/link", get(device_link::handler))
        .route("/v1/device/firmware", get(firmware::check))
        .route("/v1/firmware/{filename}", get(firmware::download))
        .merge(admin::routes())
        .layer(middleware)
        .with_state(state)
}

/// Converts whatever error the middleware stack above can produce into a
/// response, so the router as a whole stays an infallible `Service` as
/// `axum::serve` requires: an elapsed request timeout, or a request shed
/// because [`MAX_CONCURRENT_REQUESTS`] was already in flight. Both of
/// tower's error types are boxed by the layers that produce them, so this
/// downcasts rather than assuming which one occurred.
async fn handle_middleware_error(error: axum::BoxError) -> StatusCode {
    if error.is::<tower::timeout::error::Elapsed>() {
        StatusCode::REQUEST_TIMEOUT
    } else if error.is::<Overloaded>() {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_admin_token_accepts_the_real_token() {
        let state = ServerState::in_memory();
        assert!(state.verify_admin_token("in-memory-admin-token"));
    }

    #[test]
    fn verify_admin_token_rejects_a_wrong_token() {
        let state = ServerState::in_memory();
        assert!(!state.verify_admin_token("not-the-token"));
        assert!(!state.verify_admin_token(""));
    }

    #[test]
    fn in_memory_config_root_is_independent_from_firmware_storage() {
        // Catches reintroducing the production defect's path derivation in the
        // test constructor, where it would teach callers the wrong pattern.
        let state = ServerState::in_memory();
        let identity = state.registry().mint().expect("mint identity");
        let config_path = state
            .configs()
            .for_device(&identity.device_id)
            .store
            .path()
            .to_path_buf();
        let firmware_storage = state
            .firmware()
            .directory()
            .parent()
            .expect("in-memory firmware has an owned temp root");

        assert!(!config_path.starts_with(firmware_storage));
    }

    #[test]
    fn link_claim_refuses_a_second_live_owner_and_releases_on_drop() {
        let state = ServerState::in_memory();
        let first = state
            .claim_link("dev-0001".into())
            .expect("first owner claims the link");
        assert!(
            state.claim_link("dev-0001".into()).is_none(),
            "a second concurrent owner was accepted"
        );
        drop(first);
        assert!(
            state.claim_link("dev-0001".into()).is_some(),
            "the ownership slot was not released with its lease"
        );
    }
}
