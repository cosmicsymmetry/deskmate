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
mod auth;
mod device_link;
pub mod firmware;
pub mod registry;
pub mod runtime_device;
mod store;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
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
use registry::Registry;

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
/// (directly or via the outer `Arc<StateInner>`). Live links carry the
/// single-owner `app-core` runtime alongside the registry and config stores.
#[derive(Clone)]
pub struct ServerState {
    inner: Arc<StateInner>,
}

struct StateInner {
    registry: Registry,
    admin_token: String,
    firmware: FirmwareCatalog,
    configs: store::DeviceConfigStores,
    live_links: Mutex<HashMap<registry::DeviceId, Arc<LiveLink>>>,
    /// Bounds concurrent `/v1/device/link` connections. `Arc`-wrapped
    /// separately from `StateInner` because `Semaphore::try_acquire_owned`
    /// needs an owned `Arc<Semaphore>` to hand a `'static` permit to the
    /// socket task that outlives the handler which acquired it.
    link_slots: Arc<tokio::sync::Semaphore>,
}

impl ServerState {
    #[must_use]
    pub fn new(admin_token: String, firmware: FirmwareCatalog, config_directory: PathBuf) -> Self {
        Self {
            inner: Arc::new(StateInner {
                registry: Registry::new(),
                admin_token,
                firmware,
                configs: store::DeviceConfigStores::new(config_directory),
                live_links: Mutex::new(HashMap::new()),
                link_slots: Arc::new(tokio::sync::Semaphore::new(
                    device_link::MAX_CONCURRENT_LINKS,
                )),
            }),
        }
    }

    /// A state suitable for tests: an empty registry, a fixed admin token,
    /// and a firmware catalog pinned at version `1.0.0`.
    #[must_use]
    pub fn in_memory() -> Self {
        let firmware = FirmwareCatalog::in_memory();
        let config_directory = firmware
            .directory()
            .parent()
            .expect("in-memory firmware directory has an owned parent")
            .join("configs");
        Self::new(
            "in-memory-admin-token".to_string(),
            firmware,
            config_directory,
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
        let mut links = self
            .inner
            .live_links
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if links.contains_key(&device_id) {
            return None;
        }
        let link = Arc::new(LiveLink::default());
        links.insert(device_id.clone(), Arc::clone(&link));
        Some(LinkLease {
            state: self.clone(),
            device_id,
            link,
        })
    }

    pub(crate) fn live_link(&self, device_id: &str) -> Option<Arc<LiveLink>> {
        self.inner
            .live_links
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(device_id)
            .cloned()
    }
}

#[derive(Default)]
pub(crate) struct LiveLink {
    runtime: Mutex<Option<Arc<RuntimeHandle>>>,
    last_seen_unix_ms: Arc<AtomicU64>,
}

impl LiveLink {
    pub(crate) fn set_runtime(&self, runtime: Arc<RuntimeHandle>) {
        *self
            .runtime
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(runtime);
    }

    pub(crate) fn runtime(&self) -> Option<Arc<RuntimeHandle>> {
        self.runtime
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn take_runtime(&self) -> Option<Arc<RuntimeHandle>> {
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
    state: ServerState,
    device_id: registry::DeviceId,
    link: Arc<LiveLink>,
}

impl LinkLease {
    pub(crate) fn link(&self) -> &Arc<LiveLink> {
        &self.link
    }
}

impl Drop for LinkLease {
    fn drop(&mut self) {
        let mut links = self
            .state
            .inner
            .live_links
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if links
            .get(&self.device_id)
            .is_some_and(|current| Arc::ptr_eq(current, &self.link))
        {
            links.remove(&self.device_id);
        }
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
        let firmware = FirmwareCatalog::in_memory();
        let config_directory = firmware.directory().parent().unwrap().join("configs");
        let state = ServerState::new("the-real-token".to_string(), firmware, config_directory);
        assert!(state.verify_admin_token("the-real-token"));
    }

    #[test]
    fn verify_admin_token_rejects_a_wrong_token() {
        let firmware = FirmwareCatalog::in_memory();
        let config_directory = firmware.directory().parent().unwrap().join("configs");
        let state = ServerState::new("the-real-token".to_string(), firmware, config_directory);
        assert!(!state.verify_admin_token("not-the-token"));
        assert!(!state.verify_admin_token(""));
    }
}
