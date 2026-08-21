//! Admin-token-protected provisioning, configuration, and status routes.

use app_core::{
    AppConfig, AppSnapshot, MAX_CONFIG_FILE_BYTES, RuntimeError, SaveReceipt, StoreError,
    ValidationIssue,
};
use axum::Json;
use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{DefaultBodyLimit, FromRequestParts, Path, State};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use serde::{Serialize, Serializer};

use crate::ServerState;
use crate::auth::bearer_token;

pub(crate) fn routes() -> Router<ServerState> {
    Router::new()
        .route("/v1/devices", post(create_device))
        .route("/v1/devices/{id}", get(get_device))
        .route(
            "/v1/devices/{id}/config",
            put(put_config).layer(DefaultBodyLimit::max(MAX_CONFIG_FILE_BYTES)),
        )
}

struct AdminAuthenticated;

impl FromRequestParts<ServerState> for AdminAuthenticated {
    type Rejection = AdminError;

    // axum's FromRequestParts declares `async fn`, so the signature is fixed by
    // the trait even though this body never awaits. Rewriting it to return
    // `std::future::ready` to satisfy the lint would obscure the extractor for
    // no behavioural gain.
    #[allow(clippy::unused_async_trait_impl)]
    async fn from_request_parts(
        parts: &mut Parts,
        state: &ServerState,
    ) -> Result<Self, Self::Rejection> {
        let presented = bearer_token(parts).ok_or(AdminError::Unauthorized)?;
        state
            .verify_admin_token(presented)
            .then_some(Self)
            .ok_or(AdminError::Unauthorized)
    }
}

async fn create_device(
    State(state): State<ServerState>,
    _admin: AdminAuthenticated,
) -> Result<Json<MintDeviceResponse>, AdminError> {
    let identity = tokio::task::spawn_blocking(move || state.registry().mint())
        .await
        .map_err(|_| AdminError::WorkerFailed)?
        .map_err(|error| AdminError::Store {
            message: error.to_string(),
        })?;
    Ok(Json(MintDeviceResponse {
        device_id: identity.device_id,
        token: identity.token,
    }))
}

/// The sole serialization boundary for a newly minted device token. Keeping
/// it separate from the registry identity prevents that secret-bearing type
/// from becoming casually serializable on status or diagnostic paths.
#[derive(Serialize)]
struct MintDeviceResponse {
    device_id: String,
    token: String,
}

async fn put_config(
    State(state): State<ServerState>,
    _admin: AdminAuthenticated,
    Path(device_id): Path<String>,
    payload: Result<Json<AppConfig>, JsonRejection>,
) -> Result<Json<SaveReceipt>, AdminError> {
    if !state.registry().contains_device(&device_id) {
        return Err(AdminError::NotFound);
    }
    let Json(config) = payload.map_err(|rejection| AdminError::InvalidJson {
        status: rejection.status(),
        message: rejection.body_text(),
    })?;
    let device_config = state.configs().for_device(&device_id);
    let _update = device_config.update.lock().await;

    let saved_config = config.clone();
    let store = std::sync::Arc::clone(&device_config);
    let receipt = tokio::task::spawn_blocking(move || {
        saved_config
            .compile(1)
            .map_err(|error| SaveConfigError::Invalid(error.issues))?;
        store
            .store
            .save(&saved_config)
            .map_err(SaveConfigError::Store)
    })
    .await
    .map_err(|_| AdminError::WorkerFailed)?
    .map_err(AdminError::from)?;

    if let Some(runtime) = state
        .device_link(&device_id)
        .and_then(|link| link.runtime())
    {
        tokio::task::spawn_blocking(move || runtime.apply_config(config))
            .await
            .map_err(|_| AdminError::WorkerFailed)?
            .map_err(AdminError::from)?;
    }
    device_config.record_current();

    Ok(Json(receipt))
}

async fn get_device(
    State(state): State<ServerState>,
    _admin: AdminAuthenticated,
    Path(device_id): Path<String>,
) -> Result<Json<DeviceStatus>, AdminError> {
    if !state.registry().contains_device(&device_id) {
        return Err(AdminError::NotFound);
    }
    let link = state.device_link(&device_id);
    let connected = link.as_ref().is_some_and(|link| link.is_live());
    let last_seen_unix_ms = link.as_ref().and_then(|link| link.last_seen_unix_ms());
    let last_ota_error = link.as_ref().and_then(|link| link.last_ota_error());
    let runtime = link.and_then(|link| link.runtime());
    let config = state.configs().for_device(&device_id).status();
    let snapshot = if let Some(runtime) = runtime {
        Some(AdminSnapshot {
            snapshot: tokio::task::spawn_blocking(move || runtime.snapshot())
                .await
                .map_err(|_| AdminError::WorkerFailed)?
                .map_err(AdminError::from)?,
            last_ota_error,
            observed_age_seconds: observed_age_seconds(last_seen_unix_ms, now_unix_ms()),
        })
    } else {
        None
    };

    Ok(Json(DeviceStatus {
        device_id,
        connected,
        last_seen_unix_ms,
        config,
        snapshot,
    }))
}

#[derive(Debug, Serialize)]
struct DeviceStatus {
    device_id: String,
    connected: bool,
    last_seen_unix_ms: Option<u64>,
    config: crate::store::DeviceConfigStatus,
    snapshot: Option<AdminSnapshot>,
}

#[derive(Debug)]
struct AdminSnapshot {
    snapshot: AppSnapshot,
    last_ota_error: Option<String>,
    /// How long ago the device was last heard from, in seconds.
    ///
    /// The runtime is retained across a link drop by design, so this endpoint
    /// still serves a snapshot while the device is gone -- and every field under
    /// `device` is then the *last value received*, not a current one. A frozen
    /// `uptime_ms` reads exactly like a live one from a freshly booted board,
    /// which nearly cost the 2026-08-19 session a wrong conclusion. `connected`
    /// says so, but it sits at the top level, several screens away from the
    /// numbers it qualifies. This rides *inside* `device`, next to them.
    observed_age_seconds: Option<u64>,
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

/// Seconds between `last_seen` and `now`, or `None` when the device has never
/// been heard from. Saturating: a clock that moves backwards between the two
/// samples reports 0 rather than wrapping to a nonsense age.
fn observed_age_seconds(last_seen_unix_ms: Option<u64>, now_unix_ms: u64) -> Option<u64> {
    last_seen_unix_ms.map(|last_seen| now_unix_ms.saturating_sub(last_seen) / 1000)
}

fn insert_last_ota_error(
    snapshot: &mut serde_json::Value,
    last_ota_error: Option<&str>,
) -> Result<(), &'static str> {
    let device = snapshot
        .get_mut("device")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("snapshot device is not an object")?;
    device.insert(
        "last_ota_error".to_owned(),
        serde_json::to_value(last_ota_error).map_err(|_| "last OTA error is not serializable")?,
    );
    Ok(())
}

fn insert_observed_age_seconds(
    snapshot: &mut serde_json::Value,
    observed_age_seconds: Option<u64>,
) -> Result<(), &'static str> {
    let device = snapshot
        .get_mut("device")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("snapshot device is not an object")?;
    device.insert(
        "observed_age_seconds".to_owned(),
        serde_json::to_value(observed_age_seconds)
            .map_err(|_| "observed age is not serializable")?,
    );
    Ok(())
}

impl Serialize for AdminSnapshot {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut value = serde_json::to_value(&self.snapshot).map_err(serde::ser::Error::custom)?;
        insert_last_ota_error(&mut value, self.last_ota_error.as_deref())
            .map_err(serde::ser::Error::custom)?;
        insert_observed_age_seconds(&mut value, self.observed_age_seconds)
            .map_err(serde::ser::Error::custom)?;
        value.serialize(serializer)
    }
}

#[cfg(test)]
mod tests {
    use super::{insert_last_ota_error, insert_observed_age_seconds, observed_age_seconds};

    #[test]
    fn observed_age_is_inserted_beside_the_values_it_qualifies() {
        let mut snapshot = serde_json::json!({"device": {"uptime_ms": 66761}});
        insert_observed_age_seconds(&mut snapshot, Some(412)).unwrap();
        // Inside `device`, next to the frozen uptime -- not at the top level,
        // where it would be as easy to miss as `connected` was.
        assert_eq!(snapshot["device"]["observed_age_seconds"], 412);
        assert_eq!(snapshot["device"]["uptime_ms"], 66761);
    }

    #[test]
    fn a_device_never_heard_from_has_no_age_rather_than_a_zero_one() {
        // Zero would read as "heard from just now", which is the opposite of
        // the truth for a device that has never connected.
        assert_eq!(observed_age_seconds(None, 1_000_000), None);
        let mut snapshot = serde_json::json!({"device": {}});
        insert_observed_age_seconds(&mut snapshot, None).unwrap();
        assert!(snapshot["device"]["observed_age_seconds"].is_null());
    }

    #[test]
    fn observed_age_saturates_rather_than_wrapping_on_a_backwards_clock() {
        // SystemTime is not monotonic: NTP can step it backwards between the
        // sample and the read. Wrapping would turn a fresh device into one last
        // seen half a billion years ago.
        assert_eq!(observed_age_seconds(Some(2_000), 1_000), Some(0));
        assert_eq!(observed_age_seconds(Some(1_000), 413_000), Some(412));
    }

    #[test]
    fn ota_error_is_inserted_inside_snapshot_device() {
        let mut snapshot = serde_json::json!({"device": {"ota_state": "failed"}});
        insert_last_ota_error(&mut snapshot, Some("download: ESP_ERR_NO_MEM")).unwrap();
        assert_eq!(
            snapshot["device"]["last_ota_error"],
            "download: ESP_ERR_NO_MEM"
        );
    }
}

enum SaveConfigError {
    Invalid(Vec<ValidationIssue>),
    Store(StoreError),
}

#[derive(Debug)]
enum AdminError {
    Unauthorized,
    NotFound,
    InvalidJson { status: StatusCode, message: String },
    InvalidConfig { issues: Vec<ValidationIssue> },
    Store { message: String },
    Runtime { status: StatusCode, message: String },
    WorkerFailed,
}

impl From<SaveConfigError> for AdminError {
    fn from(error: SaveConfigError) -> Self {
        match error {
            SaveConfigError::Invalid(issues) => Self::InvalidConfig { issues },
            SaveConfigError::Store(StoreError::Validation { issues }) => {
                Self::InvalidConfig { issues }
            }
            SaveConfigError::Store(error) => Self::Store {
                message: error.to_string(),
            },
        }
    }
}

impl From<RuntimeError> for AdminError {
    fn from(error: RuntimeError) -> Self {
        match error {
            RuntimeError::InvalidConfig { issues } => Self::InvalidConfig { issues },
            RuntimeError::QueueFull | RuntimeError::WorkerStopped => Self::Runtime {
                status: StatusCode::SERVICE_UNAVAILABLE,
                message: error.to_string(),
            },
            RuntimeError::ResponseTimeout => Self::Runtime {
                status: StatusCode::GATEWAY_TIMEOUT,
                message: error.to_string(),
            },
            RuntimeError::DeviceDisconnected
            | RuntimeError::Device { .. }
            | RuntimeError::Provider { .. } => Self::Runtime {
                status: StatusCode::BAD_GATEWAY,
                message: error.to_string(),
            },
            RuntimeError::UnknownWidget { .. } | RuntimeError::UnknownScreen { .. } => {
                Self::Runtime {
                    status: StatusCode::UNPROCESSABLE_ENTITY,
                    message: error.to_string(),
                }
            }
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum ErrorBody<'a> {
    InvalidJson { message: &'a str },
    InvalidConfig { issues: &'a [ValidationIssue] },
    Store { message: &'a str },
    Runtime { message: &'a str },
    Internal,
}

impl IntoResponse for AdminError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthorized => StatusCode::UNAUTHORIZED.into_response(),
            Self::NotFound => StatusCode::NOT_FOUND.into_response(),
            Self::InvalidJson { status, message } => {
                (status, Json(ErrorBody::InvalidJson { message: &message })).into_response()
            }
            Self::InvalidConfig { issues } => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(ErrorBody::InvalidConfig { issues: &issues }),
            )
                .into_response(),
            Self::Store { message } => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::Store { message: &message }),
            )
                .into_response(),
            Self::Runtime { status, message } => {
                (status, Json(ErrorBody::Runtime { message: &message })).into_response()
            }
            Self::WorkerFailed => {
                (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorBody::Internal)).into_response()
            }
        }
    }
}
