//! The browser companion's API.
//!
//! This is the surface the web app talks to, and it replaces the Mac app's Tauri
//! command set one-for-one. The shapes are deliberately unchanged: every response
//! here is the same DTO the `invoke()` bridge returned, so `src/lib/types.ts` and
//! the generated `types.contract.ts` fixture describe both the old IPC and this
//! HTTP API without a second vocabulary.
//!
//! Errors are the frontend's own `IpcError` discriminated union, carried in the
//! body with a matching HTTP status. That is why the app's `toIpcError` survived
//! the move: the category it switches on is still the category it receives.
//!
//! Every route is behind [`OperatorAuthenticated`]. That is the real gate. The
//! Caddy `basic_auth` in front of these paths is the edge gate the owner asked
//! for, but it cannot be the only one -- the server binds a LAN address because
//! Caddy is a bridge-network container and cannot reach loopback, so anything
//! trusting only the edge would be open to the LAN.

use std::sync::Arc;
use std::time::Duration;

use app_core::{
    AppConfig, AppSnapshot, CardField, CardFieldValue, CardSettings, ConnectionState,
    DeviceSnapshot, DisplayOrientation, PomodoroAction, RuntimeError, RuntimeHandle, SaveReceipt,
    ValidationIssue,
};
use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use base64::prelude::{BASE64_STANDARD, Engine as _};
use chrono::{Duration as ChronoDuration, Utc};
use futures_util::stream::{self, Stream};
use serde::{Deserialize, Serialize};

use crate::ServerState;
use crate::oauth::session::{OperatorAuthenticated, SessionSigner, set_cookie_header};

/// How long a companion session lasts. Longer than the OAuth-console session
/// (`oauth::routes::SESSION_TTL`, 12 h) on purpose: that one fronts a consent
/// flow an operator visits occasionally, this one fronts the window the owner
/// keeps open, and a daily re-login to read a clock face is friction with no
/// security to show for it. The credential it is traded for is unchanged.
const SESSION_TTL_DAYS: i64 = 30;

/// Largest configuration document accepted from the browser. Matches the app's
/// own `MAX_DRAFT_BYTES` so a draft that the UI refuses to send is also a draft
/// the server refuses to accept, rather than the two disagreeing by a byte.
const MAX_DRAFT_BYTES: usize = 64 * 1024;

/// How often the event stream re-reads the snapshot.
///
/// Deliberately a poll rather than a subscription. `RuntimeSubscription::recv_timeout`
/// is blocking, and threading it into an async SSE handler would need a blocking
/// thread per open browser tab. A poll costs one cheap read per second for a
/// single-operator deployment, works identically whether or not a runtime exists
/// (the offline case is the common one -- the board is normally powered off), and
/// only sends when something actually changed.
const EVENT_POLL: Duration = Duration::from_secs(1);

pub(crate) fn routes() -> Router<ServerState> {
    Router::new()
        .route("/v1/app/session", post(login).delete(logout))
        .route("/v1/app/devices", get(list_devices))
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

/// The frontend's `IpcError`, unchanged. `category` is what the app switches on;
/// the HTTP status carries the same decision for anything that is not the app.
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

#[derive(Deserialize)]
struct LoginRequest {
    token: String,
}

/// Trades the admin token for a session cookie.
///
/// The browser cannot put a bearer on a navigation, and keeping the admin token
/// in web storage would make every XSS a permanent credential leak. The cookie is
/// `HttpOnly` and `__Host-`-prefixed, so script cannot read it and no sibling host
/// can shadow it.
async fn login(State(state): State<ServerState>, payload: Json<LoginRequest>) -> Response {
    if !state.verify_admin_token(&payload.token) {
        // Bare 401, matching `auth.rs`: no body, no echo of what was presented.
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let ttl = ChronoDuration::days(SESSION_TTL_DAYS);
    let value = state
        .sessions()
        .mint(&SessionSigner::new_sid(), Utc::now(), ttl);
    let Ok(cookie) = HeaderValue::from_str(&set_cookie_header(&value, ttl)) else {
        return AppApiError::internal("cannot build the session cookie").into_response();
    };
    ([(header::SET_COOKIE, cookie)], StatusCode::NO_CONTENT).into_response()
}

/// Ends the session by expiring the cookie. Nothing server-side to revoke: the
/// cookie is a signed assertion, not a stored record.
async fn logout() -> Response {
    let Ok(cookie) = HeaderValue::from_str(&set_cookie_header("", ChronoDuration::zero())) else {
        return AppApiError::internal("cannot clear the session cookie").into_response();
    };
    ([(header::SET_COOKIE, cookie)], StatusCode::NO_CONTENT).into_response()
}

// ---------------------------------------------------------------------------
// Devices
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct DeviceRow {
    id: String,
    connected: bool,
}

async fn list_devices(
    State(state): State<ServerState>,
    _operator: OperatorAuthenticated,
) -> Json<Vec<DeviceRow>> {
    let rows = state
        .registry_device_ids()
        .into_iter()
        .map(|id| {
            let connected = state.device_is_linked(&id);
            DeviceRow { id, connected }
        })
        .collect();
    Json(rows)
}

// ---------------------------------------------------------------------------
// Snapshot
// ---------------------------------------------------------------------------

/// `AppSnapshot` plus the one fact the runtime deliberately does not own:
/// whether a settings document has ever existed on disk. Flattened, so the
/// frontend keeps parsing a single DTO exactly as it did over IPC.
#[derive(Serialize)]
pub(crate) struct CompanionSnapshot {
    #[serde(flatten)]
    pub(crate) app: AppSnapshot,
    pub(crate) has_saved_config: bool,
}

async fn snapshot(
    State(state): State<ServerState>,
    _operator: OperatorAuthenticated,
    Path(device_id): Path<String>,
) -> Result<Json<CompanionSnapshot>, AppApiError> {
    known_device(&state, &device_id)?;
    let snapshot = read_snapshot(&state, device_id).await?;
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
    device_id: String,
) -> Result<CompanionSnapshot, AppApiError> {
    if let Some(runtime) = live_runtime(state, &device_id) {
        let app = tokio::task::spawn_blocking(move || runtime.snapshot())
            .await
            .map_err(|_| worker_failed())??;
        return Ok(CompanionSnapshot {
            app,
            has_saved_config: true,
        });
    }

    let device_config = state.configs().for_device(&device_id);
    tokio::task::spawn_blocking(move || {
        let has_saved_config = device_config.store.path().exists();
        let outcome = device_config.store.load();
        let config = outcome.config().clone();
        let mut app = app_core::initial_snapshot(&config, app_core::RuntimeDiagnostics::default());
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
        }
    })
    .await
    .map_err(|_| worker_failed())
}

fn live_runtime(state: &ServerState, device_id: &str) -> Option<Arc<RuntimeHandle>> {
    state.device_link(device_id).and_then(|link| link.runtime())
}

fn known_device(state: &ServerState, device_id: &str) -> Result<(), AppApiError> {
    if state.registry().contains_device(device_id) {
        return Ok(());
    }
    Err(AppApiError::NotFound {
        message: format!("no device with id {device_id:?}"),
    })
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// A snapshot stream, replacing the Tauri `app-state` event.
///
/// Sends only on change: an idle panel produces SSE keep-alive comments and no
/// payloads, so an open tab costs nothing to hold.
async fn events(
    State(state): State<ServerState>,
    _operator: OperatorAuthenticated,
    Path(device_id): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, AppApiError> {
    known_device(&state, &device_id)?;
    let stream = stream::unfold(
        (state, device_id, None::<String>),
        |(state, device_id, last)| async move {
            loop {
                tokio::time::sleep(EVENT_POLL).await;
                let Ok(snapshot) = read_snapshot(&state, device_id.clone()).await else {
                    continue;
                };
                let Ok(encoded) = serde_json::to_string(&snapshot) else {
                    continue;
                };
                if last.as_ref() == Some(&encoded) {
                    continue;
                }
                let event = Event::default().event("app-state").data(encoded.clone());
                return Some((Ok(event), (state, device_id, Some(encoded))));
            }
        },
    );
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
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
/// Both halves of the preflight, as the Tauri command did: `compile()` -- a
/// strict superset of `validate()`, because it also catches a card with no
/// `wire_config()` lowering -- and the connected device's capability check. The
/// revision only matters for its `revision == 0` rejection, so any nonzero
/// placeholder is right for a draft that is never applied.
async fn validate_config(
    State(state): State<ServerState>,
    _operator: OperatorAuthenticated,
    Path(device_id): Path<String>,
    payload: Json<DraftPayload>,
) -> Result<Json<DraftValidation>, AppApiError> {
    known_device(&state, &device_id)?;
    let config = parse_draft(&payload)?;
    let device = read_snapshot(&state, device_id).await?.app.device;
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
///
/// Moved from the Mac app's `commands/config.rs` unchanged.
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

/// Names capabilities the way the person reading the settings window needs them.
/// The raw bitmask this used to print named nothing anyone could act on.
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
/// The body is `admin::put_config`'s, reached through a different gate: that
/// route authenticates a machine holding the admin bearer, this one an operator
/// holding a session. Both must serialize against a socket's load/start
/// transition, which is what `device_config.update` is for -- without it a save
/// can land after the socket has loaded but before its runtime is visible, and
/// the new configuration sits persisted-but-unapplied until the next reconnect.
async fn save_config(
    State(state): State<ServerState>,
    _operator: OperatorAuthenticated,
    Path(device_id): Path<String>,
    payload: Json<DraftPayload>,
) -> Result<Json<ConfigApplyResult>, AppApiError> {
    known_device(&state, &device_id)?;
    let config = parse_draft(&payload)?;

    let device_config = state.configs().for_device(&device_id);
    let _update = device_config.update.lock().await;

    let saved = config.clone();
    let store = Arc::clone(&device_config);
    let save = tokio::task::spawn_blocking(move || {
        saved.compile(1).map_err(|error| AppApiError::Validation {
            message: "the configuration is not valid".into(),
            issues: error.issues,
        })?;
        store
            .store
            .save(&saved)
            .map_err(|error| AppApiError::Persistence {
                message: error.to_string(),
            })
    })
    .await
    .map_err(|_| worker_failed())??;

    if let Some(runtime) = live_runtime(&state, &device_id) {
        tokio::task::spawn_blocking(move || runtime.apply_config(config))
            .await
            .map_err(|_| worker_failed())??;
    }
    device_config.record_current();

    Ok(Json(ConfigApplyResult { save }))
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
    _operator: OperatorAuthenticated,
    Path(device_id): Path<String>,
    payload: Json<PomodoroRequest>,
) -> Result<StatusCode, AppApiError> {
    known_device(&state, &device_id)?;
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

const PICTURE_PREVIEW_IS_PUSH_ONLY: &str =
    "Picture cards show the last frame pushed by their source";

/// Renders one card exactly as the firmware's own template would, from the same
/// scene `build_card_scene` would push.
///
/// This is the one renderer. `CLAUDE.md`'s rule holds: nothing here is a second
/// implementation to check the first against -- that was the retired parity gate,
/// and it kept three dead templates alive to have something to compare with.
async fn preview(
    State(state): State<ServerState>,
    _operator: OperatorAuthenticated,
    Path(device_id): Path<String>,
    payload: Json<PreviewRequest>,
) -> Result<Json<PreviewFrame>, AppApiError> {
    known_device(&state, &device_id)?;
    validate_card_id(&payload.card_id)?;
    let card_id = payload.0.card_id;
    let snapshot = read_snapshot(&state, device_id).await?.app;

    let card = snapshot
        .config
        .cards
        .iter()
        .find(|card| card.id() == card_id)
        .ok_or_else(|| AppApiError::NotFound {
            message: format!("no card with id {card_id:?}"),
        })?;

    // A picture card's frame belongs to its source, and is not reconstructed here.
    if matches!(card, CardSettings::Picture { .. }) {
        return Ok(Json(PreviewFrame {
            png_base64: None,
            sample: false,
            state: Some(PICTURE_PREVIEW_IS_PUSH_ONLY.to_owned()),
        }));
    }

    // A card with no published data renders with an empty field vector rather
    // than invented sample text, so the firmware's own per-template defaults show
    // through -- the device's actual unconfigured appearance. `sample` tells the
    // UI that happened without the renderer lying about what it drew.
    let data = snapshot
        .card_data
        .iter()
        .find(|data| data.card_id == card_id);
    let (fields, sample) = match data {
        Some(data) if !data.fields.is_empty() => (data.fields.clone(), false),
        _ => (Vec::new(), true),
    };

    let scene = app_core::preview_card_scene(&snapshot.config, &card_id, &fields)
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
        timer: preview_timer(&fields),
        orientation: preview_orientation(snapshot.config.preferences.orientation),
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

/// The orientation the preview renders at.
///
/// Deliberately NOT the configured mounting. `LandscapeFlipped` is the 180 degree
/// mount, and the simulator reproduces it by reversing the finished frame exactly
/// as the firmware's `LV_DISPLAY_ROTATION_270` does. On the panel that flip is
/// cancelled by the physical mounting, so a person always sees an upright face;
/// rendered into a window that is not itself upside down, it is just upside down.
/// The preview's job is to show what the person will see.
fn preview_orientation(configured: DisplayOrientation) -> lvgl_sim::SimOrientation {
    match configured {
        DisplayOrientation::Landscape | DisplayOrientation::LandscapeFlipped => {
            lvgl_sim::SimOrientation::Landscape
        }
    }
}

/// The timer a pomodoro scene's `timer.*` bindings resolve against. `None` for a
/// card that pushes no timer fields. The three keys are the ones
/// `firmware/main/link/protocol_task.c` reads by name, so the preview and the
/// panel derive the timer from the same inputs.
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
