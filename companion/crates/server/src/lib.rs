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

mod accounts;
mod admin;
pub use accounts::AccountSpace;
// The browser companion's authenticated HTTP API.
mod app_api;
mod auth;
mod claim;
#[doc(hidden)]
pub use claim::{collect_stale_claims, spawn_housekeeping};
mod credential;
// Server-rendered data cards: the server pushing frames to its own image
// sources, so a weather/RSS/token face reaches the device through the same
// picture-card path an external producer's PNG does.
mod data_cards;
pub use data_cards::{FaceCommand, set_faces, start_data_cards};
mod device_link;
// The SSRF egress guard for OAuth token POSTs.
mod egress;
mod entitlements;
pub use entitlements::{Edition, Entitlements, SelfHosted};
pub use web_auth::TrustedProxies;
pub mod firmware;
#[doc(hidden)]
pub mod identity;
mod image_ingest;
mod image_sources;
mod image_staleness;
mod images;
pub mod mailer;
mod manage;
#[doc(hidden)]
pub mod migrate;
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
mod web_auth;

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
    identity: identity::IdentityStore,
    config_directory: PathBuf,
    accounts: Mutex<HashMap<identity::AccountId, Arc<AccountSpace>>>,
    face_catalog: Mutex<data_cards::FaceCatalogState>,
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
    firmware: FirmwareCatalog,
    options: ServerOptions,
    setup_code: web_auth::setup_code::SetupCode,
    setup_lock: Mutex<()>,
    email_address_limiter: web_auth::RateLimiter,
    email_ip_limiter: web_auth::RateLimiter,
    failure_limiter: web_auth::RateLimiter,
    /// Google account sign-in, attached only when the Google client is
    /// configured. Separate from the instance-owner integration runtime: the
    /// two flows have different redirects, scopes and pending state.
    google_sign_in: OnceLock<web_auth::google::GoogleSignIn>,
    /// Proxies whose `X-Forwarded-For` is believed besides loopback, set at startup
    /// from `DESKMATE_TRUSTED_PROXIES`; unset means loopback only.
    trusted_proxies: OnceLock<web_auth::TrustedProxies>,
    /// The OAuth integration runtime, attached at startup by `set_integrations`
    /// when integrations are configured. `OnceLock` so existing constructors are
    /// untouched and a deployment without integrations simply never sets it.
    integrations: OnceLock<Arc<oauth::IntegrationRuntime>>,
    /// Keeps the dedicated config root alive for [`ServerState::in_memory`].
    /// Production paths are operator-owned and leave this as `None`.
    _config_temp_dir: Option<tempfile::TempDir>,
    shutdown: tokio::sync::watch::Sender<bool>,
    /// Serializes device admission with identity revocation. Authentication
    /// happens before the WebSocket handler, so this closes the gap where a
    /// request authenticated just before deletion could otherwise claim a link
    /// after deletion had already looked for live links to close.
    device_lifecycle: Mutex<()>,
    device_links: Mutex<HashMap<registry::DeviceId, Arc<LiveLink>>>,
    /// Bounds concurrent `/v1/device/link` connections. `Arc`-wrapped
    /// separately from `StateInner` because `Semaphore::try_acquire_owned`
    /// needs an owned `Arc<Semaphore>` to hand a `'static` permit to the
    /// socket task that outlives the handler which acquired it.
    link_slots: Arc<tokio::sync::Semaphore>,
    #[cfg(test)]
    image_notifications: Mutex<Vec<ImageNotification>>,
}

#[cfg(test)]
type ImageNotification = (
    identity::AccountId,
    String,
    [u8; protocol::ASSET_DIGEST_LEN],
    ImageNotificationOrigin,
    Vec<registry::DeviceId>,
);

pub struct ServerOptions {
    pub public_url: url::Url,
    pub edition: Edition,
    pub entitlements: Arc<dyn Entitlements>,
    pub signups_default: bool,
    pub mailer: Arc<dyn mailer::Mailer>,
    pub extra_routes: Option<Router<ServerState>>,
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            public_url: url::Url::parse("https://deskmate.test/")
                .expect("the default public URL is valid"),
            edition: Edition::SelfHosted,
            entitlements: Arc::new(SelfHosted),
            signups_default: false,
            mailer: Arc::new(mailer::LogMailer),
            extra_routes: None,
        }
    }
}

/// The faces package a freshly built state starts with.
///
/// None: production is told at startup ([`set_faces`]), and a deployment that is
/// never told has no server faces. Inside this crate's own unit tests every state
/// gets the fake package instead, so a test about faces need not install one.
#[cfg(not(test))]
const fn default_faces() -> Option<FaceCommand> {
    None
}

#[cfg(test)]
#[allow(clippy::unnecessary_wraps)]
fn default_faces() -> Option<FaceCommand> {
    Some(data_cards::fake_faces())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImageNotificationOrigin {
    ExternalProducerPush,
    ServerRenderedRefresh,
}

impl ServerState {
    #[must_use]
    pub fn new(admin_token: String, firmware: FirmwareCatalog, config_directory: PathBuf) -> Self {
        Self::new_with_options(
            admin_token,
            firmware,
            config_directory,
            ServerOptions::default(),
        )
    }

    #[must_use]
    pub fn new_with_options(
        admin_token: String,
        firmware: FirmwareCatalog,
        config_directory: PathBuf,
        options: ServerOptions,
    ) -> Self {
        let registry = Registry::load(config_directory.join(DEVICE_IDENTITY_STORE_FILE));
        Self::with_config_temp_dir(
            admin_token,
            firmware,
            config_directory,
            registry,
            None,
            app_core::RuntimeOptions::default(),
            options,
        )
    }

    fn with_config_temp_dir(
        admin_token: String,
        firmware: FirmwareCatalog,
        config_directory: PathBuf,
        registry: Registry,
        config_temp_dir: Option<tempfile::TempDir>,
        runtime_options: app_core::RuntimeOptions,
        options: ServerOptions,
    ) -> Self {
        let identity = identity::IdentityStore::open(&config_directory.join("identity.db"))
            .expect("failed to open the identity store");
        let setup_code = if identity
            .account_count()
            .expect("failed to inspect the identity store")
            == 0
        {
            let (setup_code, code) = web_auth::setup_code::SetupCode::generate();
            web_auth::setup_code::announce_setup_code(&options.public_url, &code);
            setup_code
        } else {
            web_auth::setup_code::SetupCode::inactive()
        };
        let producer_credentials =
            producer_credentials::ProducerCredentialStore::open(config_directory.clone())
                .expect("failed to load the producer credential store");
        Self {
            inner: Arc::new(StateInner {
                registry,
                identity,
                config_directory,
                accounts: Mutex::new(HashMap::new()),
                face_catalog: Mutex::new(data_cards::FaceCatalogState::new(default_faces())),
                producer_credentials: Arc::new(producer_credentials),
                admin_token,
                runtime_options,
                firmware,
                options,
                setup_code,
                setup_lock: Mutex::new(()),
                email_address_limiter: web_auth::RateLimiter::new(5, Duration::from_hours(1)),
                email_ip_limiter: web_auth::RateLimiter::new(20, Duration::from_hours(1)),
                failure_limiter: web_auth::RateLimiter::new(5, Duration::from_mins(15)),
                google_sign_in: OnceLock::new(),
                trusted_proxies: OnceLock::new(),
                integrations: OnceLock::new(),
                _config_temp_dir: config_temp_dir,
                shutdown: tokio::sync::watch::channel(false).0,
                device_lifecycle: Mutex::new(()),
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
        Self::in_memory_with_runtime_and_options(runtime_options, ServerOptions::default())
    }

    #[doc(hidden)]
    #[must_use]
    pub fn in_memory_with_options(options: ServerOptions) -> Self {
        Self::in_memory_with_runtime_and_options(app_core::RuntimeOptions::default(), options)
    }

    fn in_memory_with_runtime_and_options(
        runtime_options: app_core::RuntimeOptions,
        options: ServerOptions,
    ) -> Self {
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
            options,
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

    #[doc(hidden)]
    #[must_use]
    pub fn identity(&self) -> &identity::IdentityStore {
        &self.inner.identity
    }

    #[doc(hidden)]
    pub fn account_space(&self, account: &identity::AccountId) -> Arc<AccountSpace> {
        let (space, opened) = self.acquire_account_space(account);
        if opened && let Err(error) = data_cards::start_account_data_cards(self, &space) {
            tracing::error!(
                account_id = %account,
                %error,
                "the account's server-rendered cards could not be started"
            );
        }
        space
    }

    /// Acquires the cached space without starting cards; startup propagates load errors.
    pub(crate) fn acquire_account_space(
        &self,
        account: &identity::AccountId,
    ) -> (Arc<AccountSpace>, bool) {
        let mut spaces = self
            .inner
            .accounts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(space) = spaces.get(account) {
            (Arc::clone(space), false)
        } else {
            let space = Arc::new(AccountSpace::open(
                self.account_root(account),
                account.clone(),
            ));
            spaces.insert(account.clone(), Arc::clone(&space));
            (space, true)
        }
    }

    pub(crate) fn drop_account_space(
        &self,
        account: &identity::AccountId,
    ) -> Option<Arc<AccountSpace>> {
        self.inner
            .accounts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(account)
    }

    pub(crate) fn account_root(&self, account: &identity::AccountId) -> PathBuf {
        self.inner
            .config_directory
            .join("accounts")
            .join(&account.0)
    }

    pub(crate) fn with_device_lifecycle<T>(&self, operation: impl FnOnce() -> T) -> T {
        let _guard = self
            .inner
            .device_lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        operation()
    }

    pub(crate) fn space_for_device(&self, device_id: &str) -> Option<Arc<AccountSpace>> {
        let owner = self.identity().device_owner(device_id).ok()??;
        Some(self.account_space(&owner.account_id))
    }

    pub(crate) fn instance_owner(&self) -> Result<identity::Account, identity::IdentityError> {
        self.identity()
            .accounts()?
            .into_iter()
            .find(|account| account.is_instance_owner)
            .ok_or(identity::IdentityError::NotFound)
    }

    pub(crate) fn public_url(&self) -> &url::Url {
        &self.inner.options.public_url
    }

    pub(crate) fn signups_default(&self) -> bool {
        self.inner.options.signups_default
    }

    pub(crate) fn mailer(&self) -> Arc<dyn mailer::Mailer> {
        Arc::clone(&self.inner.options.mailer)
    }

    #[doc(hidden)]
    #[must_use]
    pub fn setup_code_for_tests(&self) -> Option<String> {
        self.inner.setup_code.display_for_tests()
    }

    pub(crate) fn entitlements(&self) -> &dyn Entitlements {
        self.inner.options.entitlements.as_ref()
    }

    pub(crate) fn edition(&self) -> Edition {
        self.inner.options.edition
    }

    pub(crate) fn producer_credentials(
        &self,
    ) -> &Arc<producer_credentials::ProducerCredentialStore> {
        &self.inner.producer_credentials
    }

    pub(crate) fn authenticate_image_producer(
        &self,
        token: &str,
    ) -> Option<(Arc<AccountSpace>, String)> {
        for account in self.identity().accounts().ok()? {
            let space = self.account_space(&account.id);
            if let Some(source_id) = space.image_sources.authenticate(token) {
                return Some((space, source_id));
            }
        }
        None
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

    /// Enables Google account sign-in with the same client and guarded POST
    /// transport used by Google integrations. The account callback always comes
    /// from this server's public URL, regardless of the integration redirect.
    pub fn set_google_sign_in(
        &self,
        mut config: oauth::GoogleOAuthConfig,
        transport: Arc<dyn oauth::transport::OAuthTransport>,
    ) {
        config.redirect_uri = self
            .public_url()
            .join("/v1/app/auth/google/callback")
            .expect("the Google sign-in callback path joins the public URL")
            .into();
        let _ = self
            .inner
            .google_sign_in
            .set(web_auth::google::GoogleSignIn::new(config, transport));
    }

    pub(crate) fn google_sign_in(&self) -> Option<&web_auth::google::GoogleSignIn> {
        self.inner.google_sign_in.get()
    }

    /// Trusts `X-Forwarded-For` from these proxies as well as loopback. Startup only.
    pub fn set_trusted_proxies(&self, proxies: TrustedProxies) {
        let _ = self.inner.trusted_proxies.set(proxies);
    }

    pub(crate) fn trusted_proxies(&self) -> &TrustedProxies {
        static LOOPBACK_ONLY: TrustedProxies = TrustedProxies::NONE;
        self.inner.trusted_proxies.get().unwrap_or(&LOOPBACK_ONLY)
    }

    /// Snapshot live handles without holding the server's link-map lock while a
    /// runtime command waits for its worker reply. Every device gets the update:
    /// image sources are reusable across devices and the runtime itself decides
    /// whether the visible card subscribes to this source.
    pub(crate) fn notify_image_source_changed(
        &self,
        account: &identity::AccountId,
        source_id: String,
        digest: [u8; protocol::ASSET_DIGEST_LEN],
        origin: ImageNotificationOrigin,
    ) {
        let (device_ids, runtimes) = {
            let links = self
                .inner
                .device_links
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let matching = links
                .iter()
                .filter(|(_, link)| link.account_id == *account && link.is_live())
                .collect::<Vec<_>>();
            let device_ids = matching
                .iter()
                .map(|(device_id, _)| (*device_id).clone())
                .collect::<Vec<_>>();
            let runtimes = matching
                .into_iter()
                .filter_map(|(_, link)| link.runtime())
                .collect::<Vec<_>>();
            (device_ids, runtimes)
        };
        tracing::info!(target: "server::tap_latency", account_id = %account,
            source_id, digest = %protocol::digest_hex(&digest), devices = ?device_ids,
            unix_us = chrono::Utc::now().timestamp_micros(), "image update queued for runtimes");
        #[cfg(test)]
        self.inner
            .image_notifications
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((
                account.clone(),
                source_id.clone(),
                digest,
                origin,
                device_ids,
            ));
        #[cfg(not(test))]
        let _ = device_ids;
        tokio::task::spawn_blocking(move || {
            if let Err(error) = notify_runtimes(runtimes, |runtime| {
                runtime.image_source_updated(&source_id, digest)
            }) {
                warn_image_notification_failure(origin, &source_id, &error);
            }
        });
    }

    #[cfg(test)]
    fn image_notifications_for_test(&self) -> Vec<ImageNotification> {
        self.inner
            .image_notifications
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Atomically reserves the one live ownership slot for `device_id`.
    /// The reservation happens before the 101 response, so two simultaneous
    /// upgrades cannot both believe they won.
    pub(crate) fn claim_link(
        &self,
        device_id: registry::DeviceId,
        account_id: identity::AccountId,
    ) -> Option<LinkLease> {
        let _lifecycle = self
            .inner
            .device_lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.registry().contains_device(&device_id)
            || !self
                .identity()
                .device_owner(&device_id)
                .ok()
                .flatten()
                .is_some_and(|owner| owner.account_id == account_id)
        {
            return None;
        }
        let link = {
            let mut links = self
                .inner
                .device_links
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match links.entry(device_id) {
                std::collections::hash_map::Entry::Occupied(entry) => {
                    if entry.get().account_id != account_id {
                        return None;
                    }
                    Arc::clone(entry.get())
                }
                std::collections::hash_map::Entry::Vacant(entry) => {
                    Arc::clone(entry.insert(Arc::new(LiveLink::new(account_id))))
                }
            }
        };
        if !link.claim() {
            return None;
        }
        Some(LinkLease { link })
    }

    pub(crate) fn close_link(&self, device_id: &str) {
        let link = self
            .inner
            .device_links
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(device_id);
        let Some(link) = link else {
            return;
        };
        let Some(runtime) = link.close() else {
            return;
        };
        runtime.connector.detach();
        if let Err(error) = runtime.handle.shutdown() {
            tracing::warn!(device_id = %device_id, %error, "revoked device runtime shutdown failed");
        }
    }

    pub(crate) fn device_link(&self, device_id: &str) -> Option<Arc<LiveLink>> {
        self.inner
            .device_links
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(device_id)
            .cloned()
    }

    /// Ends long-lived browser streams before Axum waits for connections to drain.
    pub fn begin_shutdown(&self) {
        self.inner.shutdown.send_replace(true);
    }

    pub(crate) fn subscribe_shutdown(&self) -> tokio::sync::watch::Receiver<bool> {
        self.inner.shutdown.subscribe()
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

pub(crate) struct LiveLink {
    account_id: identity::AccountId,
    live: AtomicBool,
    closed: AtomicBool,
    runtime: Mutex<Option<ManagedRuntime>>,
    last_seen_unix_ms: Arc<AtomicU64>,
}

struct ManagedRuntime {
    handle: Arc<RuntimeHandle>,
    connector: SocketConnector,
}

impl LiveLink {
    fn new(account_id: identity::AccountId) -> Self {
        Self {
            account_id,
            live: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            runtime: Mutex::new(None),
            last_seen_unix_ms: Arc::new(AtomicU64::new(0)),
        }
    }

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
        let mut retained = self
            .runtime
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.closed.load(Ordering::Acquire) {
            connector.detach();
            if let Err(error) = runtime.shutdown() {
                tracing::warn!(%error, "revoked device runtime shutdown failed during startup");
            }
            return;
        }
        *retained = Some(ManagedRuntime {
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

    fn close(&self) -> Option<ManagedRuntime> {
        self.closed.store(true, Ordering::Release);
        self.live.store(false, Ordering::Release);
        self.take_runtime()
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

    let mut router = Router::new()
        .route("/v1/device/link", get(device_link::handler))
        .route("/v1/device/firmware", get(firmware::check))
        .route("/v1/firmware/{filename}", get(firmware::download))
        .merge(admin::routes())
        .merge(claim::routes())
        .merge(app_api::routes())
        .merge(images::routes())
        .merge(manage::routes())
        .merge(oauth::routes::routes())
        .merge(web_auth::routes::routes());
    if let Some(extra_routes) = state.inner.options.extra_routes.clone() {
        router = router.merge(extra_routes);
    }
    let router = router.layer(middleware).with_state(state);
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
        let account = state
            .identity()
            .create_account("owner@example.com", true, true, chrono::Utc::now())
            .unwrap();
        let space = state.account_space(&account.id);
        let identity = state.registry().mint().expect("mint identity");
        let config_path = space
            .configs
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
        let account = state
            .identity()
            .create_account("owner@example.com", true, true, chrono::Utc::now())
            .unwrap();
        let device = state.registry().mint().unwrap();
        state
            .identity()
            .assign_device(
                &device.device_id,
                &account.id,
                identity::DeviceState::Active,
                chrono::Utc::now(),
            )
            .unwrap();
        let first = state
            .claim_link(device.device_id.clone(), account.id.clone())
            .expect("first owner claims the link");
        assert!(
            state
                .claim_link(device.device_id.clone(), account.id.clone())
                .is_none(),
            "a second concurrent owner was accepted"
        );
        drop(first);
        assert!(
            state.claim_link(device.device_id, account.id).is_some(),
            "the ownership slot was not released with its lease"
        );
    }

    #[test]
    fn link_claim_rechecks_registry_identity_and_account_ownership() {
        let state = ServerState::in_memory();
        let account = state
            .identity()
            .create_account("owner@example.com", true, true, chrono::Utc::now())
            .unwrap();

        let revoked = state.registry().mint().unwrap();
        state
            .identity()
            .assign_device(
                &revoked.device_id,
                &account.id,
                identity::DeviceState::Active,
                chrono::Utc::now(),
            )
            .unwrap();
        state.registry().revoke(&revoked.device_id).unwrap();
        assert!(
            state
                .claim_link(revoked.device_id, account.id.clone())
                .is_none(),
            "a request authenticated just before revocation claimed a link"
        );

        let released = state.registry().mint().unwrap();
        state
            .identity()
            .assign_device(
                &released.device_id,
                &account.id,
                identity::DeviceState::Active,
                chrono::Utc::now(),
            )
            .unwrap();
        state
            .identity()
            .release_device(&released.device_id)
            .unwrap();
        assert!(
            state.claim_link(released.device_id, account.id).is_none(),
            "a device whose ownership row was deleted claimed a link"
        );
    }

    #[tokio::test]
    async fn image_notifications_name_only_live_devices_in_the_owning_account() {
        let state = ServerState::in_memory();
        let account_a = state
            .identity()
            .create_account("a@example.com", true, true, chrono::Utc::now())
            .unwrap();
        let account_b = state
            .identity()
            .create_account("b@example.com", true, false, chrono::Utc::now())
            .unwrap();
        let device_a = state.registry().mint().unwrap();
        let device_b = state.registry().mint().unwrap();
        state
            .identity()
            .assign_device(
                &device_a.device_id,
                &account_a.id,
                identity::DeviceState::Active,
                chrono::Utc::now(),
            )
            .unwrap();
        state
            .identity()
            .assign_device(
                &device_b.device_id,
                &account_b.id,
                identity::DeviceState::Active,
                chrono::Utc::now(),
            )
            .unwrap();
        let _lease_a = state
            .claim_link(device_a.device_id.clone(), account_a.id.clone())
            .unwrap();
        let _lease_b = state.claim_link(device_b.device_id, account_b.id).unwrap();
        let digest = [0x42; protocol::ASSET_DIGEST_LEN];

        state.notify_image_source_changed(
            &account_a.id,
            "weather".into(),
            digest,
            ImageNotificationOrigin::ExternalProducerPush,
        );

        assert_eq!(
            state.image_notifications_for_test(),
            [(
                account_a.id,
                "weather".into(),
                digest,
                ImageNotificationOrigin::ExternalProducerPush,
                vec![device_a.device_id],
            )]
        );
    }
}
