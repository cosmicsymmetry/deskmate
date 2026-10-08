//! The browser companion's API.
//!
//! This is the surface the web app talks to. Its response DTOs are checked
//! against `src/lib/types.ts` through the generated `types.contract.ts` fixture,
//! so the Rust HTTP API and TypeScript client share one vocabulary.
//!
//! HTTP errors carry a discriminated JSON body matching the frontend's
//! `ApiError` union, plus the corresponding status. The web client switches on
//! the same category that non-browser callers can infer from the status.
//!
//! Protected data routes use the account carried by [`AccountSession`].
//! `DELETE /v1/app/session` revokes that stored session and clears its cookie.
//! The admin bearer is deliberately not accepted on this browser surface.
//! Device routes authenticate their own bearer tokens independently; firmware
//! downloads under `/v1/firmware/*` are deliberately unauthenticated.

use std::sync::Arc;
use std::time::Duration;

use app_core::{
    AppConfig, AppSnapshot, CardField, CardFieldValue, CardSettings, ConnectionState,
    DeviceSnapshot, LoadOutcome, PersistenceState, PomodoroAction, RuntimeError, RuntimeHandle,
    SAVED_SETTINGS_VALIDATION_FAILURE_MESSAGE, SaveReceipt, StoreError, ValidationIssue,
};
use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use base64::prelude::{BASE64_STANDARD, Engine as _};
use chrono::Utc;
use futures_util::stream::{self, Stream};
use serde::{Deserialize, Serialize};

use crate::web_auth::{AccountSession, clear_session_cookie};
use crate::{AccountSpace, ServerState};

/// Largest configuration document accepted from the browser. Matches the app's
/// own `MAX_DRAFT_BYTES` so a draft that the UI refuses to send is also a draft
/// the server refuses to accept, rather than the two disagreeing by a byte.
const MAX_DRAFT_BYTES: usize = 64 * 1024;

/// How often the event stream re-reads the snapshot.
///
/// Deliberately a poll rather than a subscription. `RuntimeSubscription::recv_timeout`
/// is blocking, and threading it into an async SSE handler would need a blocking
/// thread per open browser tab. A poll costs one cheap read per second for a
/// single-operator deployment and works identically whether or not a runtime exists
/// (the offline case is the common one -- the board is normally powered off).
const EVENT_POLL: Duration = Duration::from_secs(1);

/// The longest the stream will go without sending, once connected.
///
/// The change test below ignores telemetry, so without this floor the numbers it
/// ignores would never reach the web client at all. Half a minute is far finer than
/// anyone reads a signal strength and far coarser than a re-render costs.
const EVENT_HEARTBEAT: Duration = Duration::from_secs(30);

pub(crate) fn routes() -> Router<ServerState> {
    Router::new()
        .route("/v1/app/session", axum::routing::delete(logout))
        .route("/v1/app/{id}/snapshot", get(snapshot))
        .route("/v1/app/{id}/events", get(events))
        .route("/v1/app/{id}/config", put(save_config))
        .route("/v1/app/{id}/config/validate", post(validate_config))
        .route("/v1/app/{id}/preview", post(preview))
        .route("/v1/app/{id}/pomodoro", post(pomodoro))
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// An HTTP error response matching the frontend's `ApiError` union. `category`
/// is what the web app switches on; the status carries the same decision for
/// any other client.
#[derive(Debug, Serialize)]
#[serde(tag = "category", rename_all = "kebab-case")]
pub(crate) enum AppApiError {
    InvalidPayload {
        message: String,
    },
    PayloadTooLarge {
        message: String,
        maximum_bytes: usize,
    },
    Validation {
        message: String,
        issues: Vec<ValidationIssue>,
    },
    Persistence {
        message: String,
    },
    RuntimeUnavailable {
        message: String,
    },
    NotFound {
        message: String,
    },
    Device {
        message: String,
    },
    Internal {
        message: String,
    },
}

impl AppApiError {
    fn status(&self) -> StatusCode {
        match self {
            Self::InvalidPayload { .. } => StatusCode::BAD_REQUEST,
            Self::PayloadTooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE,
            Self::Validation { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            Self::NotFound { .. } => StatusCode::NOT_FOUND,
            Self::RuntimeUnavailable { .. } => StatusCode::SERVICE_UNAVAILABLE,
            Self::Device { .. } => StatusCode::BAD_GATEWAY,
            Self::Persistence { .. } | Self::Internal { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }
}

impl IntoResponse for AppApiError {
    fn into_response(self) -> Response {
        (self.status(), Json(self)).into_response()
    }
}

impl From<RuntimeError> for AppApiError {
    fn from(error: RuntimeError) -> Self {
        match error {
            RuntimeError::InvalidConfig { issues } => Self::Validation {
                message: "the configuration is not valid".into(),
                issues,
            },
            other => Self::Device {
                message: format!("{other:?}"),
            },
        }
    }
}

/// A worker thread that panicked or was cancelled. Every blocking call in this
/// module funnels here so the failure is reported once, in one shape.
fn worker_failed() -> AppApiError {
    AppApiError::internal("a worker task failed")
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

async fn logout(
    State(state): State<ServerState>,
    session: AccountSession,
) -> Result<Response, AppApiError> {
    let plaintext = session.session_plaintext;
    tokio::task::spawn_blocking(move || state.identity().delete_session(&plaintext))
        .await
        .map_err(|_| worker_failed())?
        .map_err(|error| AppApiError::internal(error.to_string()))?;
    Ok((
        [(header::SET_COOKIE, clear_session_cookie())],
        StatusCode::NO_CONTENT,
    )
        .into_response())
}

// ---------------------------------------------------------------------------
// Devices
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub(crate) struct DeviceRow {
    pub(crate) id: String,
    pub(crate) connected: bool,
    /// Whether this identity has ever been configured.
    ///
    /// Reported because the companion page has to select *a* display and the registry's
    /// order is mint order, not usefulness: a server that has minted spare
    /// identities over time lists several that were never configured, and
    /// opening on the first of those shows an empty loop for a display that is
    /// not the one on the desk.
    pub(crate) has_saved_config: bool,
    /// When that configuration was last written, in unix seconds.
    ///
    /// The tie-breaker `has_saved_config` alone could not provide. The live
    /// server has three configured identities and only one panel; the one being
    /// worked on is the one whose configuration was written most recently, and
    /// that is a fact the filesystem already knows. `None` when there is no
    /// saved configuration, or when the timestamp cannot be read -- a missing
    /// value ranks last rather than pretending to be old.
    pub(crate) configured_at: Option<i64>,
    pub(crate) state: &'static str,
}

// ---------------------------------------------------------------------------
// Snapshot
// ---------------------------------------------------------------------------

/// `AppSnapshot` plus the two facts the runtime deliberately does not own:
/// whether a settings document has ever existed on disk, and which protocol
/// version this host speaks. Flattened so the frontend receives one snapshot
/// object rather than a transport-specific envelope.
#[derive(Serialize)]
pub(crate) struct CompanionSnapshot {
    #[serde(flatten)]
    pub(crate) app: AppSnapshot,
    pub(crate) has_saved_config: bool,
    /// The wire version this server speaks, straight from `protocol`.
    ///
    /// Reported from the crate that defines the wire contract so the frontend's
    /// compatibility check cannot drift from the server's actual version.
    pub(crate) host_protocol_version: u8,
}

async fn snapshot(
    State(state): State<ServerState>,
    session: AccountSession,
    Path(device_id): Path<String>,
) -> Result<Json<CompanionSnapshot>, AppApiError> {
    let space = owned_device(&state, &session, &device_id).await?;
    let snapshot = read_snapshot(&state, space, device_id).await?;
    Ok(Json(snapshot))
}

/// The snapshot for one device, live if the board is linked and synthesized from
/// the stored configuration if it is not.
///
/// The offline branch is the common one -- the board is normally powered off --
/// and it is honest rather than optimistic: it reports the configuration the
/// server would push and a device that is plainly disconnected, never remembered
/// device facts that may no longer be true.
async fn read_snapshot(
    state: &ServerState,
    space: Arc<AccountSpace>,
    device_id: String,
) -> Result<CompanionSnapshot, AppApiError> {
    let device_config = space.configs.for_device(&device_id);
    let _update = device_config.update.lock().await;
    if let Some(runtime) = live_runtime(state, &device_id) {
        let mut app = tokio::task::spawn_blocking(move || runtime.snapshot())
            .await
            .map_err(|_| worker_failed())??;
        device_config.apply_load_diagnostic(&mut app);
        return Ok(CompanionSnapshot {
            app,
            has_saved_config: true,
            host_protocol_version: protocol::PROTOCOL_VERSION,
        });
    }

    let store = Arc::clone(&device_config);
    tokio::task::spawn_blocking(move || {
        let has_saved_config = store.store.path().exists();
        let (config, persistence) = match store.store.load() {
            LoadOutcome::Loaded { config, .. } => (config, PersistenceState::Clean),
            LoadOutcome::Recovered { config, error, .. } => (
                config,
                PersistenceState::RecoverableError {
                    message: error.to_string(),
                },
            ),
            LoadOutcome::ValidationFailed { config, issues, .. } => (
                config,
                PersistenceState::ValidationFailed {
                    message: SAVED_SETTINGS_VALIDATION_FAILURE_MESSAGE.into(),
                    issues,
                },
            ),
        };
        let mut app = app_core::initial_snapshot(&config, app_core::RuntimeDiagnostics::default());
        app.persistence = persistence;
        app.device = DeviceSnapshot {
            active_card_id: config.cards.first().map(|card| card.id().to_owned()),
            ..app_core::empty_device(ConnectionState::Disconnected {
                reason: Some("the display is not linked to the server".into()),
            })
        };
        app.runtime = if config.preferences.paused {
            app_core::RuntimeState::Paused
        } else {
            app_core::RuntimeState::Running
        };
        CompanionSnapshot {
            app,
            has_saved_config,
            host_protocol_version: protocol::PROTOCOL_VERSION,
        }
    })
    .await
    .map_err(|_| worker_failed())
}

fn live_runtime(state: &ServerState, device_id: &str) -> Option<Arc<RuntimeHandle>> {
    state.device_link(device_id).and_then(|link| link.runtime())
}

async fn owned_device(
    state: &ServerState,
    session: &AccountSession,
    device_id: &str,
) -> Result<Arc<AccountSpace>, AppApiError> {
    let lookup = state.clone();
    let account_id = session.account.id.clone();
    let device_id = device_id.to_owned();
    let lookup_device_id = device_id.clone();
    let space = tokio::task::spawn_blocking(move || {
        let owned = lookup
            .identity()
            .device_owner(&lookup_device_id)
            .map_err(|error| AppApiError::internal(error.to_string()))?
            .is_some_and(|owner| owner.account_id == account_id);
        Ok::<_, AppApiError>(owned.then(|| lookup.account_space(&account_id)))
    })
    .await
    .map_err(|_| worker_failed())??;
    space.ok_or_else(|| AppApiError::NotFound {
        message: format!("no device with id {device_id:?}"),
    })
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// An SSE snapshot stream using the `app-state` event consumed by the web companion.
///
/// Sends on meaningful state changes and with a 30-second payload heartbeat;
/// between those, an idle panel produces only SSE keep-alive comments.
async fn events(
    State(state): State<ServerState>,
    session: AccountSession,
    Path(device_id): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, AppApiError> {
    let space = owned_device(&state, &session, &device_id).await?;
    let shutdown = state.subscribe_shutdown();
    let start = (
        state,
        space,
        device_id,
        None::<String>,
        tokio::time::Instant::now(),
        shutdown,
    );
    let stream = stream::unfold(
        start,
        |(state, space, device_id, last, last_sent, mut shutdown)| async move {
            loop {
                if *shutdown.borrow() {
                    return None;
                }
                tokio::select! {
                    biased;
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() {
                            return None;
                        }
                        continue;
                    }
                    () = tokio::time::sleep(EVENT_POLL) => {}
                }
                let Ok(snapshot) =
                    read_snapshot(&state, Arc::clone(&space), device_id.clone()).await
                else {
                    continue;
                };
                let Ok(value) = serde_json::to_value(&snapshot) else {
                    continue;
                };
                let key = change_key(&value);
                let due = last_sent.elapsed() >= EVENT_HEARTBEAT;
                if !due && last.as_ref() == Some(&key) {
                    continue;
                }
                let event = Event::default().event("app-state").data(value.to_string());
                let now = tokio::time::Instant::now();
                return Some((
                    Ok(event),
                    (state, space, device_id, Some(key), now, shutdown),
                ));
            }
        },
    );
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// The part of a snapshot worth notifying the web client about.
///
/// A linked board reports its uptime, its signal strength and its frame counters
/// continuously, so a byte-for-byte comparison finds the snapshot different every
/// couple of seconds and re-renders the entire page for numbers nothing acts
/// on. Measured on the live server: six frames in twelve seconds, differing only
/// in `uptime_ms`, `wifi_rssi` and `counters.valid_frames`. The symptom was not
/// subtle -- the page never held still long enough for a browser to consider a
/// button clickable.
///
/// Telemetry is blanked rather than dropped, so a field appearing or disappearing
/// is still a change. The values themselves still reach the web client on
/// [`EVENT_HEARTBEAT`].
fn change_key(value: &serde_json::Value) -> String {
    let mut value = value.clone();
    if let Some(device) = value.get_mut("device").and_then(|d| d.as_object_mut()) {
        for volatile in [
            "uptime_ms",
            "free_heap",
            "wifi_rssi",
            "counters",
            // The store's byte tallies move whenever an asset is written, which
            // is every refresh of every picture card. The page renders none of
            // them, and letting them through would re-render it on each frame.
            "asset_store",
            "volatile_assets",
        ] {
            if let Some(field) = device.get_mut(volatile) {
                *field = serde_json::Value::Null;
            }
        }
    }
    // Pure host-side tallies; the web companion renders none of them.
    if let Some(diagnostics) = value.get_mut("diagnostics") {
        *diagnostics = serde_json::Value::Null;
    }
    value.to_string()
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// The draft envelope the app already sends: JSON-in-a-string, so the size limit
/// is checked against the bytes that will actually be stored rather than against
/// a re-serialization of them.
#[derive(Deserialize)]
struct DraftPayload {
    json: String,
}

#[derive(Serialize)]
pub(crate) struct DraftValidation {
    pub(crate) valid: bool,
    pub(crate) issues: Vec<ValidationIssue>,
}

#[derive(Serialize)]
pub(crate) struct ConfigApplyResult {
    pub(crate) save: SaveReceipt,
}

fn parse_draft(draft: &DraftPayload) -> Result<AppConfig, AppApiError> {
    if draft.json.len() > MAX_DRAFT_BYTES {
        return Err(AppApiError::PayloadTooLarge {
            message: format!("Configuration exceeds the {MAX_DRAFT_BYTES}-byte limit."),
            maximum_bytes: MAX_DRAFT_BYTES,
        });
    }
    serde_json::from_str(&draft.json).map_err(|error| AppApiError::InvalidPayload {
        message: error.to_string(),
    })
}

/// Reports exactly the issues a save would, so the UI can never call a draft
/// valid that Save then rejects.
///
/// The preflight runs both `compile()` -- a strict superset of `validate()`,
/// because it also catches a card with no `wire_config()` lowering -- and the
/// connected device's capability check. The revision only matters for its
/// `revision == 0` rejection, so any nonzero placeholder is right for a draft
/// that is never applied.
async fn validate_config(
    State(state): State<ServerState>,
    session: AccountSession,
    Path(device_id): Path<String>,
    payload: Json<DraftPayload>,
) -> Result<Json<DraftValidation>, AppApiError> {
    let space = owned_device(&state, &session, &device_id).await?;
    let config = parse_draft(&payload)?;
    let device = read_snapshot(&state, space, device_id).await?.app.device;
    let issues = match config.compile(1) {
        Ok(compiled) => missing_capability_issues(&device, compiled.required_capabilities),
        Err(error) => error.issues,
    };
    Ok(Json(DraftValidation {
        valid: issues.is_empty(),
        issues,
    }))
}

/// The capability issues a configuration would raise against `device`, or an
/// empty list when it is compatible -- or when no device is online to check
/// against, in which case an offline draft is saved for the next link and gated
/// then.
fn missing_capability_issues(
    device: &DeviceSnapshot,
    required_capabilities: u64,
) -> Vec<ValidationIssue> {
    if !matches!(device.connection, ConnectionState::Online) || device.protocol_version.is_none() {
        return Vec::new();
    }
    let available = device.capability_bits();
    if available & required_capabilities == required_capabilities {
        return Vec::new();
    }
    let missing = required_capabilities & !available;
    vec![ValidationIssue {
        path: "device.capabilities".into(),
        code: app_core::ValidationCode::RequiresCapability,
        message: format!(
            "the connected firmware does not support {}. Update the firmware, or remove the cards and settings that need it.",
            describe_capabilities(missing)
        ),
    }]
}

/// Names capabilities the way the person reading the settings page needs them.
/// A raw bitmask would name nothing the operator could act on.
fn describe_capabilities(bits: u64) -> String {
    let mut names: Vec<&'static str> = app_core::DeviceCapability::from_bits(bits)
        .into_iter()
        .map(app_core::DeviceCapability::label)
        .collect();
    // A bit this build does not know about can only be reported as itself.
    if bits & !app_core::DeviceCapability::known_bits() != 0 {
        names.push("an unrecognized device feature");
    }
    match names.len() {
        0 => "the features this configuration needs".to_owned(),
        1 => names[0].to_owned(),
        _ => {
            let last = names.pop().unwrap_or_default();
            format!("{} and {last}", names.join(", "))
        }
    }
}

/// Saves the configuration and applies it to the board if it is linked.
///
/// Compilation and atomic persistence use the same `DeviceConfig` primitive as
/// `admin::put_config`; this route then performs its browser-only source
/// reconciliation before applying the config. Both routes must serialize
/// against a socket's load/start transition, which is what
/// `device_config.update` is for -- without it a save can land after the socket
/// has loaded but before its runtime is visible, and the new configuration sits
/// persisted-but-unapplied until the next reconnect.
async fn save_config(
    State(state): State<ServerState>,
    session: AccountSession,
    Path(device_id): Path<String>,
    payload: Json<DraftPayload>,
) -> Result<Json<ConfigApplyResult>, AppApiError> {
    let space = owned_device(&state, &session, &device_id).await?;
    let config = parse_draft(&payload)?;
    let maximum = state
        .entitlements()
        .max_cards(&space.account_id)
        .min(app_core::MAX_CONFIG_CARDS);
    if config.cards.len() > maximum {
        return Err(AppApiError::Validation {
            message: "the configuration is not valid".into(),
            issues: vec![ValidationIssue {
                path: "cards".into(),
                code: app_core::ValidationCode::TooMany,
                message: format!("This account can have at most {maximum} cards."),
            }],
        });
    }

    let device_config = space.configs.for_device(&device_id);
    let _update = device_config.update.lock().await;

    let saved = config.clone();
    let store = Arc::clone(&device_config);
    let save = tokio::task::spawn_blocking(move || store.compile_and_save(&saved))
        .await
        .map_err(|_| worker_failed())?
        .map_err(|error| match error {
            StoreError::Validation { issues } => AppApiError::Validation {
                message: "the configuration is not valid".into(),
                issues,
            },
            error => AppApiError::Persistence {
                message: error.to_string(),
            },
        })?;

    // Sources are per account and panels are per device: another panel's cards
    // may still use a source this config does not name.
    if let Some(others) = sources_used_by_panels(&state, &space, Some(&device_id)).await? {
        reconcile_image_sources(Arc::clone(&space), &config, &others).await?;
    }

    if let Some(runtime) = live_runtime(&state, &device_id) {
        tokio::task::spawn_blocking(move || runtime.apply_config(config))
            .await
            .map_err(|_| worker_failed())??;
    }
    device_config.record_current();

    Ok(Json(ConfigApplyResult { save }))
}

/// Makes the server's image sources match the configuration that was just saved.
///
/// Minting happens when the owner picks a face from the add menu, which is a
/// *draft* action against a *server* resource -- two different transactions with
/// nothing joining them. Abandon the draft, close the tab, or remove the card,
/// and the source stayed on the server forever. The live deployment had
/// accumulated "Weather 2" and "Weather 3": credentialed sources no card
/// referenced, offered back in the add menu as things to reuse.
///
/// Revoking against the saved configuration closes that, including for existing
/// orphaned sources, because every save reconciles.
///
/// **This is a KEEP-set, and that is the shape that has bitten this project
/// before** (`AssetRelease.digests`, where an empty desired set meant "wipe
/// everything"). It is safe here only because `compile()` has already run and
/// rejects a configuration whose picture card names a source the document does
/// not declare -- so a config that authorises a deletion is one that provably
/// still names every source a card uses. Do not move this above the validation.
pub(crate) async fn reconcile_image_sources(
    space: Arc<AccountSpace>,
    config: &AppConfig,
    also_keep: &std::collections::BTreeSet<String>,
) -> Result<(), AppApiError> {
    let declared: std::collections::BTreeSet<&str> = config
        .image_sources
        .iter()
        .map(|source| source.id.as_str())
        .chain(also_keep.iter().map(String::as_str))
        .collect();
    let undeclared: Vec<String> = space
        .image_sources
        .summaries(Utc::now())
        .into_iter()
        .filter(|source| !declared.contains(source.id.as_str()))
        .map(|source| source.id)
        .collect();
    if undeclared.is_empty() {
        return Ok(());
    }

    for source_id in undeclared {
        match crate::images::revoke_image_source(Arc::clone(&space), source_id.clone()).await {
            Ok(()) => {}
            Err(crate::images::RevokeImageSourceError::WorkerFailed) => {
                return Err(worker_failed());
            }
            Err(crate::images::RevokeImageSourceError::Source(error)) => {
                // One source failing to revoke must not fail the save: the
                // configuration is already stored and is what the owner asked for.
                // Say so and carry on -- the next save reconciles again.
                tracing::warn!(source_id, %error, "could not revoke an undeclared image source");
                continue;
            }
            Err(crate::images::RevokeImageSourceError::Face(error)) => {
                tracing::warn!(source_id, %error, "could not drop the face of a revoked source");
            }
        }
        tracing::info!(
            source_id,
            "revoked an image source the configuration no longer declares"
        );
    }
    Ok(())
}

/// Every source declared by the saved configuration of the account's panels,
/// `except` one. `None` when any of them cannot be read cleanly: a source is
/// then possibly still in use, and nothing may be revoked on its account.
pub(crate) fn sources_used_by_saved_panels(
    state: &ServerState,
    space: &AccountSpace,
    except: Option<&str>,
) -> Option<std::collections::BTreeSet<String>> {
    let devices = state.identity().devices_for(&space.account_id).ok()?;
    let mut used = std::collections::BTreeSet::new();
    for device in devices {
        if except == Some(device.device_id.as_str()) {
            continue;
        }
        let store = space.configs.for_device(&device.device_id);
        if !store.store.path().exists() {
            continue;
        }
        let LoadOutcome::Loaded { config, .. } = store.store.load() else {
            return None;
        };
        used.extend(config.image_sources.iter().map(|source| source.id.clone()));
    }
    Some(used)
}

/// [`sources_used_by_saved_panels`] off the async runtime.
pub(crate) async fn sources_used_by_panels(
    state: &ServerState,
    space: &Arc<AccountSpace>,
    except: Option<&str>,
) -> Result<Option<std::collections::BTreeSet<String>>, AppApiError> {
    let state = state.clone();
    let space = Arc::clone(space);
    let except = except.map(str::to_owned);
    tokio::task::spawn_blocking(move || {
        sources_used_by_saved_panels(&state, &space, except.as_deref())
    })
    .await
    .map_err(|_| worker_failed())
}

// ---------------------------------------------------------------------------
// Pomodoro
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct PomodoroRequest {
    card_id: String,
    action: PomodoroAction,
}

/// Pomodoro control needs the board: the timer it moves lives on the device
/// between pushes. With no link there is nothing to command, and saying so is
/// better than silently accepting a tap that changes nothing.
async fn pomodoro(
    State(state): State<ServerState>,
    session: AccountSession,
    Path(device_id): Path<String>,
    payload: Json<PomodoroRequest>,
) -> Result<StatusCode, AppApiError> {
    let _space = owned_device(&state, &session, &device_id).await?;
    validate_card_id(&payload.card_id)?;
    let Some(runtime) = live_runtime(&state, &device_id) else {
        return Err(AppApiError::RuntimeUnavailable {
            message: "the display is not linked, so its timer cannot be controlled".into(),
        });
    };
    let Json(PomodoroRequest { card_id, action }) = payload;
    tokio::task::spawn_blocking(move || runtime.control_pomodoro(card_id, action))
        .await
        .map_err(|_| worker_failed())??;
    Ok(StatusCode::NO_CONTENT)
}

fn validate_card_id(card_id: &str) -> Result<(), AppApiError> {
    if card_id.is_empty() || card_id.len() > protocol::MAX_CARD_ID_LEN {
        return Err(AppApiError::InvalidPayload {
            message: "card ID is empty or too long".into(),
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Preview
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct PreviewRequest {
    card_id: String,
}

/// `png_base64` is null exactly when the renderer produced no pixels; `state`
/// then carries the word for why.
#[derive(Serialize)]
pub(crate) struct PreviewFrame {
    pub(crate) png_base64: Option<String>,
    pub(crate) sample: bool,
    pub(crate) state: Option<String>,
}

const PICTURE_HAS_NO_FRAME_YET: &str =
    "No frame yet \u{2014} this appears once its source draws one";

/// Renders the scene `build_card_scene` would push through the firmware's own
/// scene decoder and interpreter compiled into lvgl-sim.
///
/// Sharing the scene builder and renderer keeps the preview aligned with the
/// face sent to the panel.
async fn preview(
    State(state): State<ServerState>,
    session: AccountSession,
    Path(device_id): Path<String>,
    payload: Json<PreviewRequest>,
) -> Result<Json<PreviewFrame>, AppApiError> {
    let space = owned_device(&state, &session, &device_id).await?;
    validate_card_id(&payload.card_id)?;
    let card_id = payload.0.card_id;
    let snapshot = read_snapshot(&state, Arc::clone(&space), device_id)
        .await?
        .app;

    let card = snapshot
        .config
        .cards
        .iter()
        .find(|card| card.id() == card_id)
        .ok_or_else(|| AppApiError::NotFound {
            message: format!("no card with id {card_id:?}"),
        })?;

    // A picture card's face is whatever its source last pushed, so the preview is
    // that stored frame -- the very bytes the panel holds -- shown as a PNG. Nothing
    // is rendered or reconstructed here, which keeps "exactly one renderer" true: a
    // weather or token face is visible in the window without the window knowing how
    // to draw one.
    if let CardSettings::Picture { source_id, .. } = card {
        let source_id = source_id.clone();
        let image_space = Arc::clone(&space);
        let png = tokio::task::spawn_blocking(move || {
            image_space
                .image_sources
                .frame(&source_id, Utc::now())
                .and_then(|frame| crate::image_ingest::png_from_canonical_frame(&frame.bytes))
        })
        .await
        .map_err(|_| worker_failed())?;
        return Ok(Json(PreviewFrame {
            state: png.is_none().then(|| PICTURE_HAS_NO_FRAME_YET.to_owned()),
            png_base64: png.map(|png| BASE64_STANDARD.encode(png)),
            sample: false,
        }));
    }

    // A card with no published data passes an empty field vector to the host
    // scene builder, which uses its defaults for missing fields. `sample` tells
    // the UI those defaults were used.
    let data = snapshot
        .card_data
        .iter()
        .find(|data| data.card_id == card_id);
    let (fields, sample): (&[CardField], bool) = match data {
        Some(data) if !data.fields.is_empty() => (data.fields.as_slice(), false),
        _ => (&[], true),
    };

    let scene = app_core::preview_card_scene(&snapshot.config, &card_id, fields)
        .map_err(AppApiError::internal)?;
    let now = Utc::now();
    let utc_offset_minutes =
        app_core::utc_offset_minutes(&snapshot.config.preferences.timezone, now)
            .map_err(AppApiError::internal)?;

    let request = lvgl_sim::scene::SceneRenderRequest {
        scene,
        assets: Vec::new(),
        utc_offset_minutes,
        now_unix_seconds: now.timestamp(),
        timer: preview_timer(fields),
        // Browser previews show the person's upright view at either physical mounting.
        orientation: lvgl_sim::SimOrientation::Landscape,
    };
    let png = tokio::task::spawn_blocking(move || crate::preview_handle().render(request))
        .await
        .map_err(|_| worker_failed())?
        .map_err(|message| {
            if message == crate::preview::SUPERSEDED {
                AppApiError::RuntimeUnavailable { message }
            } else {
                AppApiError::internal(message)
            }
        })?;

    Ok(Json(PreviewFrame {
        png_base64: Some(BASE64_STANDARD.encode(png)),
        sample,
        state: None,
    }))
}

/// Builds the simulator's `SceneTimer` from the host-side `CardField` inputs a
/// pomodoro preview has available. The panel receives the corresponding typed
/// `PushTimer` state rather than reading these strings. `None` for a card that
/// supplies no timer fields.
fn preview_timer(fields: &[CardField]) -> Option<lvgl_sim::scene::SceneTimer> {
    let total = preview_field_integer(fields, "duration_seconds")?;
    let remaining = preview_field_integer(fields, "remaining_seconds").unwrap_or(total);
    Some(lvgl_sim::scene::SceneTimer {
        total_ms: preview_seconds_to_ms(total),
        remaining_ms: preview_seconds_to_ms(remaining),
        running: matches!(
            fields
                .iter()
                .find(|field| field.key == "running")
                .map(|field| &field.value),
            Some(CardFieldValue::Boolean { value: true })
        ),
    })
}

fn preview_field_integer(fields: &[CardField], key: &str) -> Option<i64> {
    fields
        .iter()
        .find_map(|field| match (&*field.key, &field.value) {
            (candidate, CardFieldValue::Integer { value }) if candidate == key => Some(*value),
            _ => None,
        })
}

fn preview_seconds_to_ms(seconds: i64) -> u32 {
    u32::try_from(seconds.max(0))
        .unwrap_or(u32::MAX)
        .saturating_mul(1_000)
}

#[cfg(test)]
mod contract;

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::firmware::FirmwareCatalog;

    #[test]
    fn the_event_stream_stays_quiet_while_only_telemetry_moves() {
        // A linked board reports uptime, signal strength and frame counters
        // continuously. Comparing whole snapshots made the stream fire every couple
        // of seconds and re-render the page for numbers nothing acts on -- measured
        // on the live server as six frames in twelve seconds, differing only in
        // `uptime_ms`, `wifi_rssi` and `counters.valid_frames`.
        //
        // This test drives the same predicate the stream uses, because reproducing a
        // live board's telemetry through the HTTP surface would need hardware.
        let base = serde_json::json!({
            "config": { "schema_version": 11 },
            "device": {
                "connection": { "kind": "online" },
                "uptime_ms": 1_000,
                "free_heap": 120_000,
                "wifi_rssi": -52,
                "counters": { "valid_frames": 10 },
            },
            "diagnostics": { "commands_processed": 1 },
        });
        let mut ticked = base.clone();
        ticked["device"]["uptime_ms"] = serde_json::json!(3_000);
        ticked["device"]["wifi_rssi"] = serde_json::json!(-55);
        ticked["device"]["counters"]["valid_frames"] = serde_json::json!(11);
        ticked["diagnostics"]["commands_processed"] = serde_json::json!(4);
        assert_eq!(
            change_key(&base),
            change_key(&ticked),
            "telemetry moving is not a reason to update the page"
        );

        let mut meaningful = base.clone();
        meaningful["config"]["schema_version"] = serde_json::json!(12);
        assert_ne!(
            change_key(&base),
            change_key(&meaningful),
            "a configuration change must still reach the page"
        );

        let mut went_offline = base.clone();
        went_offline["device"]["connection"] = serde_json::json!({ "kind": "disconnected" });
        assert_ne!(
            change_key(&base),
            change_key(&went_offline),
            "the display appearing or disappearing is the whole point of the stream"
        );

        // Blanked, not dropped: a telemetry field that vanishes is still a change.
        let mut lost_rssi = base.clone();
        lost_rssi["device"]
            .as_object_mut()
            .expect("device object")
            .remove("wifi_rssi");
        assert_ne!(
            change_key(&base),
            change_key(&lost_rssi),
            "a field disappearing is a change even when its value is ignored"
        );
    }

    fn state() -> (tempfile::TempDir, ServerState) {
        let root = tempfile::tempdir().expect("config root");
        let state = ServerState::new(
            "admin token".into(),
            FirmwareCatalog::in_memory(),
            root.path().to_path_buf(),
        );
        state
            .identity()
            .create_account("owner@example.com", true, true, Utc::now())
            .expect("owner account");
        (root, state)
    }

    fn mint_face(state: &ServerState) -> crate::image_sources::MintedSource {
        let space = state.account_space(&state.instance_owner().expect("instance owner").id);
        let source = space.image_sources.mint("Weather").expect("mint source");
        crate::data_cards::create_face(state, &space, &source.id, "weather").expect("create face");
        source
    }

    fn replace_file_with_directory(path: &Path) {
        let backup = path.with_extension("backup");
        std::fs::rename(path, backup).expect("preserve original file");
        std::fs::create_dir(path).expect("blocking directory");
    }

    #[tokio::test]
    async fn reconciliation_tolerates_source_failure_and_skips_its_face() {
        let (_root, state) = state();
        let space = state.account_space(&state.instance_owner().unwrap().id);
        let source = mint_face(&state);
        replace_file_with_directory(
            &space
                .root
                .join(crate::image_sources::IMAGE_SOURCE_STORE_FILE),
        );

        reconcile_image_sources(
            Arc::clone(&space),
            &AppConfig::default(),
            &std::collections::BTreeSet::new(),
        )
        .await
        .expect("ordinary source storage failure does not fail a saved config");

        assert_eq!(
            space.image_sources.authenticate(&source.token),
            Some(source.id.clone())
        );
        assert!(crate::data_cards::descriptor_for_source(&state, &space, &source.id).is_some());
    }

    #[tokio::test]
    async fn reconciliation_tolerates_face_failure_after_revoking_the_source() {
        let (_root, state) = state();
        let space = state.account_space(&state.instance_owner().unwrap().id);
        let source = mint_face(&state);
        replace_file_with_directory(&space.root.join("data-cards.json"));

        reconcile_image_sources(
            Arc::clone(&space),
            &AppConfig::default(),
            &std::collections::BTreeSet::new(),
        )
        .await
        .expect("ordinary face storage failure does not fail a saved config");

        assert_eq!(space.image_sources.authenticate(&source.token), None);
        assert!(crate::data_cards::descriptor_for_source(&state, &space, &source.id).is_some());
    }

    #[tokio::test]
    async fn reconciliation_keeps_a_source_another_panel_uses() {
        let (_root, state) = state();
        let space = state.account_space(&state.instance_owner().unwrap().id);
        let source = mint_face(&state);

        reconcile_image_sources(
            Arc::clone(&space),
            &AppConfig::default(),
            &std::collections::BTreeSet::from([source.id.clone()]),
        )
        .await
        .unwrap();

        assert_eq!(
            space.image_sources.authenticate(&source.token),
            Some(source.id.clone())
        );
    }

    #[test]
    fn startup_releases_sources_no_panel_declares() {
        // The live account after its only panel was removed: eight sources,
        // no panel naming any of them, and no room to add a picture card.
        let (_root, state) = state();
        let owner = state.instance_owner().unwrap().id;
        let space = state.account_space(&owner);
        let device = state.registry().mint().unwrap();
        state
            .identity()
            .assign_device(
                &device.device_id,
                &owner,
                crate::identity::DeviceState::Active,
                Utc::now(),
            )
            .unwrap();
        let orphan = space.image_sources.mint("Orphan").unwrap();

        assert_eq!(
            sources_used_by_saved_panels(&state, &space, None),
            Some(std::collections::BTreeSet::new())
        );
        crate::data_cards::start_account_data_cards(&state, &space).unwrap();

        assert_eq!(space.image_sources.authenticate(&orphan.token), None);
    }
}
