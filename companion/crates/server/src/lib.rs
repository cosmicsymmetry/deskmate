//! The Deskmate server owns networked devices over persistent WebSocket links
//! and answers their firmware-update checks.
//!
//! This crate's device-facing surface -- `/v1/device/link`,
//! `/v1/device/firmware`, and `/v1/firmware/{version}.bin` -- is the whole
//! contract the firmware sees. Its route and payload shapes are frozen by the
//! device protocol, while server-internal storage and account concerns remain
//! behind that boundary.
//!
//! Production exposes this behind a public Cloudflare tunnel, so `app()` treats
//! every caller as internet-facing and anonymous by default: a single
//! process-wide concurrency limit, load shedding, and a request timeout
//! guard every route, on top of the per-route defenses in `device_link` and
//! `firmware`.

mod admin;
// The browser companion's authenticated HTTP API.
mod app_api;
mod auth;
mod credential;
// Server-rendered data cards: the server pushing frames to its own image
// sources, so a weather/RSS/token face reaches the device through the same
// picture-card path an external producer's PNG does.
mod data_cards;
pub use data_cards::start_data_cards;
mod device_link;
// The SSRF egress guard, and the one HTTP client the provider layer is
// allowed to use.
mod egress;
mod egress_client;
// The server-authored data-card faces: the SVG authoring (`faces`) and the
// rasterizer that turns one into the frame a picture producer would have
// pushed (`face_render`).
mod face_render;
mod faces;
pub mod firmware;
mod image_ingest;
mod image_sources;
mod image_staleness;
mod images;
mod manage;
pub mod oauth;
// The one LVGL simulator this process owns, and the card previews it renders.
mod preview;
mod producer_credentials;
pub mod registry;
mod runtime_device;
pub mod secrets;
mod store;
// Serves the companion's built assets from DESKMATE_WEB_DIR.
mod web;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
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
/// Sized conservatively for the expected small deployment rather than derived
/// from production traffic; keep it under review as traffic data accumulates.
/// `device_link::MAX_CONCURRENT_LINKS` (32) is `<=` this value by
/// construction, and an upgrade request releases its permit here at the
/// 101 response -- before the long-lived socket loop begins -- so the two
/// caps cannot deadlock each other.
const MAX_CONCURRENT_REQUESTS: usize = 64;

/// The process's single card-preview renderer, started on first use.
///
/// One per process, not one per `ServerState`: `lvgl_sim::Simulator` owns LVGL's
/// global state, so a second one is not a second renderer but a corruption of the
/// first. Tests construct many states and would otherwise start many simulators.
static PREVIEW: OnceLock<preview::PreviewHandle> = OnceLock::new();

pub(crate) fn preview_handle() -> &'static preview::PreviewHandle {
    PREVIEW.get_or_init(preview::spawn)
}

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
    image_sources: Arc<image_sources::ImageSourceStore>,
    data_cards: Mutex<data_cards::DataCardState>,
    producer_credentials: Arc<producer_credentials::ProducerCredentialStore>,
    admin_token: String,
    /// How the per-device `app-core` runtime is paced.
    ///
    /// A field rather than `RuntimeOptions::default()` at the construction site,
    /// because the defaults are production pacing and a test that is not about
    /// pacing should not pay for it. `app-core`'s own tests already inject
    /// 10-millisecond intervals; the server had no way to, so every reconnect in
    /// `tests/hostile_device.rs` cost a real 3 seconds -- one second of
    /// `reconnect_interval` plus a two-second `status_interval` -- and four
    /// reconnects made one test 14 seconds, a third of the whole suite.
    runtime_options: app_core::RuntimeOptions,
    /// Signs and verifies the operator session cookie, keyed by the admin token.
    /// Held here so deployments without OAuth integrations still have browser
    /// sessions: the companion authenticates once with the admin token and then
    /// carries a cookie, independently of any configured OAuth provider.
    sessions: oauth::session::SessionSigner,
    firmware: FirmwareCatalog,
    configs: store::DeviceConfigStores,
    /// The OAuth integration runtime, attached at startup by `set_integrations`
    /// when integrations are configured. `OnceLock` so existing constructors are
    /// untouched and a deployment without integrations simply never sets it.
    integrations: OnceLock<Arc<oauth::IntegrationRuntime>>,
    /// Keeps the dedicated config root alive for [`ServerState::in_memory`].
    /// Production paths are operator-owned and leave this as `None`.
    _config_temp_dir: Option<tempfile::TempDir>,
    device_links: Mutex<HashMap<registry::DeviceId, Arc<LiveLink>>>,
    /// Bounds concurrent `/v1/device/link` connections. `Arc`-wrapped
    /// separately from `StateInner` because `Semaphore::try_acquire_owned`
    /// needs an owned `Arc<Semaphore>` to hand a `'static` permit to the
    /// socket task that outlives the handler which acquired it.
    link_slots: Arc<tokio::sync::Semaphore>,
    #[cfg(test)]
    image_notifications: Mutex<
        Vec<(
            String,
            [u8; protocol::ASSET_DIGEST_LEN],
            ImageNotificationOrigin,
        )>,
    >,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImageNotificationOrigin {
    ExternalProducerPush,
    ServerRenderedRefresh,
}

impl ServerState {
    #[must_use]
    pub fn new(admin_token: String, firmware: FirmwareCatalog, config_directory: PathBuf) -> Self {
        let registry = Registry::load(config_directory.join(DEVICE_IDENTITY_STORE_FILE));
        Self::with_config_temp_dir(
            admin_token,
            firmware,
            config_directory,
            registry,
            None,
            app_core::RuntimeOptions::default(),
        )
    }

    fn with_config_temp_dir(
        admin_token: String,
        firmware: FirmwareCatalog,
        config_directory: PathBuf,
        registry: Registry,
        config_temp_dir: Option<tempfile::TempDir>,
        runtime_options: app_core::RuntimeOptions,
    ) -> Self {
        let data_card_spec_path = config_directory.join("data-cards.json");
        let image_sources = image_sources::ImageSourceStore::new(config_directory.clone())
            .expect("failed to load the image-source store");
        let producer_credentials =
            producer_credentials::ProducerCredentialStore::open(config_directory.clone())
                .expect("failed to load the producer credential store");
        Self {
            inner: Arc::new(StateInner {
                registry,
                image_sources: Arc::new(image_sources),
                data_cards: Mutex::new(data_cards::DataCardState::new(data_card_spec_path)),
                producer_credentials: Arc::new(producer_credentials),
                sessions: oauth::session::SessionSigner::from_admin_token(&admin_token),
                admin_token,
                runtime_options,
                firmware,
                configs: store::DeviceConfigStores::new(config_directory),
                integrations: OnceLock::new(),
                _config_temp_dir: config_temp_dir,
                device_links: Mutex::new(HashMap::new()),
                link_slots: Arc::new(tokio::sync::Semaphore::new(
                    device_link::MAX_CONCURRENT_LINKS,
                )),
                #[cfg(test)]
                image_notifications: Mutex::new(Vec::new()),
            }),
        }
    }

    /// A state suitable for tests: an empty registry, a fixed admin token,
    /// a firmware catalog pinned at version `1.0.0`, and independent owned
    /// temp roots for firmware and device configuration.
    #[must_use]
    pub fn in_memory() -> Self {
        Self::in_memory_with_runtime_options(app_core::RuntimeOptions::default())
    }

    /// [`Self::in_memory`] with the device runtime paced for a test.
    ///
    /// Opt-in rather than the default for `in_memory`, because most tests want
    /// production pacing and shortening it under them would change what they
    /// exercise. Reach for this only when the wait is incidental to the thing
    /// being tested.
    #[must_use]
    pub fn in_memory_with_runtime_options(runtime_options: app_core::RuntimeOptions) -> Self {
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
            runtime_options,
        )
    }

    /// The pacing a newly linked device's runtime is started with.
    pub(crate) fn runtime_options(&self) -> app_core::RuntimeOptions {
        self.inner.runtime_options
    }

    #[must_use]
    pub fn registry(&self) -> &Registry {
        &self.inner.registry
    }

    /// The operator session signer. Always present -- see the field's own note.
    pub(crate) fn sessions(&self) -> &oauth::session::SessionSigner {
        &self.inner.sessions
    }

    pub(crate) fn image_sources(&self) -> &Arc<image_sources::ImageSourceStore> {
        &self.inner.image_sources
    }

    pub(crate) fn producer_credentials(
        &self,
    ) -> &Arc<producer_credentials::ProducerCredentialStore> {
        &self.inner.producer_credentials
    }

    /// Every minted device id, for the management surface's device table.
    pub(crate) fn registry_device_ids(&self) -> Vec<String> {
        self.inner.registry.device_ids()
    }

    /// Whether a device currently holds a live link. A board at rest is the
    /// resting state here, not a fault, so this is reported as "not connected"
    /// rather than as an error.
    pub(crate) fn device_is_linked(&self, device_id: &str) -> bool {
        self.device_link(device_id)
            .is_some_and(|link| link.is_live())
    }

    /// Compares `presented` against the admin token in constant time. This
    /// is the *only* sanctioned way to check the admin token, preserving the
    /// constant-time-comparison guarantee for the one secret that protects
    /// every write.
    #[must_use]
    pub fn verify_admin_token(&self, presented: &str) -> bool {
        registry::constant_time_eq(self.inner.admin_token.as_bytes(), presented.as_bytes())
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

    /// The OAuth integration runtime, if one was attached at startup.
    #[must_use]
    pub fn integrations(&self) -> Option<Arc<oauth::IntegrationRuntime>> {
        self.inner.integrations.get().map(Arc::clone)
    }

    /// Attaches the OAuth integration runtime once, at startup. Subsequent calls
    /// are ignored (the first attachment wins).
    pub fn set_integrations(&self, runtime: Arc<oauth::IntegrationRuntime>) {
        let _ = self.inner.integrations.set(runtime);
    }

    /// Snapshot live handles without holding the server's link-map lock while a
    /// runtime command waits for its worker reply. Every device gets the update:
    /// image sources are reusable across devices and the runtime itself decides
    /// whether the visible card subscribes to this source.
    pub(crate) fn notify_image_source_changed(
        &self,
        source_id: String,
        digest: [u8; protocol::ASSET_DIGEST_LEN],
        origin: ImageNotificationOrigin,
    ) {
        #[cfg(test)]
        self.inner
            .image_notifications
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((source_id.clone(), digest, origin));
        let runtimes: Vec<_> = {
            let links = self
                .inner
                .device_links
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            links
                .values()
                .filter(|link| link.is_live())
                .filter_map(|link| link.runtime())
                .collect()
        };
        tokio::task::spawn_blocking(move || {
            if let Err(error) = notify_runtimes(runtimes, |runtime| {
                runtime.image_source_updated(&source_id, digest)
            }) {
                warn_image_notification_failure(origin, &source_id, &error);
            }
        });
    }

    #[cfg(test)]
    fn image_notifications_for_test(
        &self,
    ) -> Vec<(
        String,
        [u8; protocol::ASSET_DIGEST_LEN],
        ImageNotificationOrigin,
    )> {
        self.inner
            .image_notifications
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
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
        data_cards::stop_refreshers(self);
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

fn notify_runtimes<T>(
    runtimes: impl IntoIterator<Item = T>,
    mut notify: impl FnMut(T) -> Result<(), app_core::RuntimeError>,
) -> Result<(), app_core::RuntimeError> {
    let mut first_error = None;
    for runtime in runtimes {
        if let Err(error) = notify(runtime)
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn warn_image_notification_failure(
    origin: ImageNotificationOrigin,
    source_id: &str,
    error: &app_core::RuntimeError,
) {
    match origin {
        ImageNotificationOrigin::ExternalProducerPush => {
            tracing::warn!(target: "server::images",
                source_id = %source_id,
                %error,
                "image source stored but the device was not notified; the next                      synchronize will reconcile it"
            );
        }
        ImageNotificationOrigin::ServerRenderedRefresh => {
            tracing::warn!(target: "server::data_cards",
                source_id = %source_id,
                %error,
                "the frame is stored but the device was not notified; the next synchronize reconciles it"
            );
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

/// Builds the device-facing surface and the authenticated browser-companion
/// routes over `state`.
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
/// The router, with no companion UI mounted. `app_with_web` adds it.
pub fn app(state: ServerState) -> Router {
    app_with_web(state, None)
}

/// The router, optionally serving the browser companion from `web_root`.
///
/// The UI is mounted as a fallback *outside* the `/v1` namespace, so adding it
/// cannot shadow an API route: `web::asset` answers 404 for any `/v1` path it is
/// handed, and every real API route matches before the fallback is consulted.
pub fn app_with_web(state: ServerState, web_root: Option<PathBuf>) -> Router {
    let middleware = ServiceBuilder::new()
        .layer(HandleErrorLayer::new(handle_middleware_error))
        .load_shed()
        .layer(GlobalConcurrencyLimitLayer::new(MAX_CONCURRENT_REQUESTS))
        .timeout(REQUEST_TIMEOUT);

    let router = Router::new()
        .route("/v1/device/link", get(device_link::handler))
        .route("/v1/device/firmware", get(firmware::check))
        .route("/v1/firmware/{filename}", get(firmware::download))
        .merge(admin::routes())
        .merge(app_api::routes())
        .merge(images::routes())
        .merge(manage::routes())
        .merge(oauth::routes::routes())
        .layer(middleware)
        .with_state(state);
    match web_root {
        Some(root) => router.merge(web::routes(web::WebRoot::new(root))),
        None => router,
    }
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
    use std::collections::BTreeMap;

    use tracing::field::{Field, Visit};
    use tracing_subscriber::layer::{Context, SubscriberExt as _};
    use tracing_subscriber::{Layer, Registry};

    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    struct CapturedEvent {
        target: String,
        level: String,
        fields: BTreeMap<String, String>,
    }

    #[derive(Clone, Default)]
    struct EventCollector(Arc<Mutex<Vec<CapturedEvent>>>);

    #[derive(Default)]
    struct FieldVisitor {
        fields: BTreeMap<String, String>,
    }

    impl Visit for FieldVisitor {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.fields
                .insert(field.name().to_owned(), format!("{value:?}"));
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            self.fields
                .insert(field.name().to_owned(), value.to_owned());
        }
    }

    impl Layer<Registry> for EventCollector {
        fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, Registry>) {
            let mut visitor = FieldVisitor::default();
            event.record(&mut visitor);
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(CapturedEvent {
                    target: event.metadata().target().to_owned(),
                    level: event.metadata().level().to_string(),
                    fields: visitor.fields,
                });
        }
    }

    #[test]
    fn notification_fanout_visits_later_runtimes_after_an_error() {
        let mut visited = Vec::new();

        let result = notify_runtimes([0, 1, 2], |runtime| {
            visited.push(runtime);
            match runtime {
                0 => Err(app_core::RuntimeError::WorkerStopped),
                1 => Err(app_core::RuntimeError::ResponseTimeout),
                _ => Ok(()),
            }
        });

        assert_eq!(visited, [0, 1, 2]);
        assert_eq!(result, Err(app_core::RuntimeError::WorkerStopped));
    }

    #[test]
    fn external_notification_warning_keeps_its_operator_facing_contract() {
        let events = EventCollector::default();
        let captured = Arc::clone(&events.0);
        let subscriber = tracing_subscriber::registry().with(events);

        tracing::subscriber::with_default(subscriber, || {
            warn_image_notification_failure(
                ImageNotificationOrigin::ExternalProducerPush,
                "source-1",
                &app_core::RuntimeError::WorkerStopped,
            );
        });

        assert_eq!(
            *captured
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            [CapturedEvent {
                target: "server::images".into(),
                level: "WARN".into(),
                fields: BTreeMap::from([
                    ("error".into(), "runtime worker has stopped".into()),
                    (
                        "message".into(),
                        "image source stored but the device was not notified; the next                      synchronize will reconcile it".into(),
                    ),
                    ("source_id".into(), "source-1".into()),
                ]),
            }]
        );
    }

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
