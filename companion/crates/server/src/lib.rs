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
//! every caller as internet-facing and anonymous by default: a global
//! concurrency limit and request timeout guard every route, on top of the
//! per-route defenses in `device_link` and `firmware`.

mod auth;
mod device_link;
pub mod firmware;
pub mod registry;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::error_handling::HandleErrorLayer;
use axum::http::StatusCode;
use axum::routing::get;
use tower::ServiceBuilder;

use firmware::FirmwareCatalog;
use registry::Registry;

/// Caps how many requests this process handles at once, across every route.
/// This is coarse, blanket protection on top of the download route's own
/// streaming (which is what actually bounds its memory use) and the device
/// link's own connection cap.
const MAX_CONCURRENT_REQUESTS: usize = 64;

/// How long any single request may take to produce a response. Generous for
/// a firmware-check round trip; irrelevant to an established device link,
/// whose handler returns as soon as the WebSocket upgrade completes -- the
/// long-lived socket loop runs independently afterward.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Shared server state, cheap to clone: every field is behind an `Arc`
/// (directly or via the outer `Arc<StateInner>`). Task 9 adds the
/// single-owner `app-core` runtime alongside the registry.
#[derive(Clone)]
pub struct ServerState {
    inner: Arc<StateInner>,
}

struct StateInner {
    registry: Registry,
    admin_token: String,
    firmware: FirmwareCatalog,
    /// Bounds concurrent `/v1/device/link` connections. `Arc`-wrapped
    /// separately from `StateInner` because `Semaphore::try_acquire_owned`
    /// needs an owned `Arc<Semaphore>` to hand a `'static` permit to the
    /// socket task that outlives the handler which acquired it.
    link_slots: Arc<tokio::sync::Semaphore>,
}

impl ServerState {
    #[must_use]
    pub fn new(admin_token: String, firmware: FirmwareCatalog) -> Self {
        Self {
            inner: Arc::new(StateInner {
                registry: Registry::new(),
                admin_token,
                firmware,
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
        Self::new(
            "in-memory-admin-token".to_string(),
            FirmwareCatalog::in_memory(),
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
}

/// Builds the full device-facing router over `state`. Task 9 adds a
/// Mac-facing router behind the admin token; this task only wires the
/// surface firmware will ever call.
pub fn app(state: ServerState) -> Router {
    let middleware = ServiceBuilder::new()
        .layer(HandleErrorLayer::new(handle_middleware_error))
        .timeout(REQUEST_TIMEOUT)
        .concurrency_limit(MAX_CONCURRENT_REQUESTS);

    Router::new()
        .route("/v1/device/link", get(device_link::handler))
        .route("/v1/device/firmware", get(firmware::check))
        .route("/v1/firmware/{filename}", get(firmware::download))
        .layer(middleware)
        .with_state(state)
}

/// Converts whatever error the middleware stack above can produce into a
/// response, so the router as a whole stays an infallible `Service` as
/// `axum::serve` requires. `tower::timeout::Timeout` boxes its error (it
/// forwards the inner service's error or its own `Elapsed`, both converted
/// via `Into<BoxError>`), so the only error this stack can actually produce
/// today is an elapsed timeout -- checked explicitly rather than assumed,
/// so a future layer's genuine error still gets a distinct response.
async fn handle_middleware_error(error: axum::BoxError) -> StatusCode {
    if error.is::<tower::timeout::error::Elapsed>() {
        StatusCode::REQUEST_TIMEOUT
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_admin_token_accepts_the_real_token() {
        let state = ServerState::new("the-real-token".to_string(), FirmwareCatalog::in_memory());
        assert!(state.verify_admin_token("the-real-token"));
    }

    #[test]
    fn verify_admin_token_rejects_a_wrong_token() {
        let state = ServerState::new("the-real-token".to_string(), FirmwareCatalog::in_memory());
        assert!(!state.verify_admin_token("not-the-token"));
        assert!(!state.verify_admin_token(""));
    }
}
