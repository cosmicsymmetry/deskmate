//! The Deskmate V2 server: a single-tenant stub that owns networked-tier
//! devices over a persistent WebSocket link and answers firmware-update
//! checks for both tiers.
//!
//! This crate's device-facing surface -- `/v1/device/link`,
//! `/v1/device/firmware`, and `/v1/firmware/{version}.bin` -- is the whole
//! contract the firmware ever sees. V3 replaces everything behind it
//! (accounts, storage, multi-tenancy) without touching that surface.

mod auth;
mod device_link;
pub mod firmware;
pub mod registry;

use std::sync::Arc;

use axum::Router;
use axum::routing::get;

use firmware::FirmwareCatalog;
use registry::Registry;

/// Shared server state, cheap to clone: every field is behind an `Arc`.
/// Task 9 adds the single-owner `app-core` runtime alongside the registry.
#[derive(Clone)]
pub struct ServerState {
    inner: Arc<StateInner>,
}

struct StateInner {
    registry: Registry,
    admin_token: String,
    firmware: FirmwareCatalog,
}

impl ServerState {
    #[must_use]
    pub fn new(admin_token: String, firmware: FirmwareCatalog) -> Self {
        Self {
            inner: Arc::new(StateInner {
                registry: Registry::new(),
                admin_token,
                firmware,
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

    #[must_use]
    pub fn firmware(&self) -> &FirmwareCatalog {
        &self.inner.firmware
    }
}

/// Builds the full device-facing router over `state`. Task 9 adds a
/// Mac-facing router behind the admin token; this task only wires the
/// surface firmware will ever call.
pub fn app(state: ServerState) -> Router {
    Router::new()
        .route("/v1/device/link", get(device_link::handler))
        .route("/v1/device/firmware", get(firmware::check))
        .route("/v1/firmware/{filename}", get(firmware::download))
        .with_state(state)
}
