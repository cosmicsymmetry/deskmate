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
use serde::Serialize;

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
) -> Json<MintDeviceResponse> {
    let identity = state.registry().mint();
    Json(MintDeviceResponse {
        device_id: identity.device_id,
        token: identity.token,
    })
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

    if let Some(runtime) = state.live_link(&device_id).and_then(|link| link.runtime()) {
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
    let live = state.live_link(&device_id);
    let connected = live.is_some();
    let last_seen_unix_ms = live.as_ref().and_then(|link| link.last_seen_unix_ms());
    let runtime = live.and_then(|link| link.runtime());
    let config = state.configs().for_device(&device_id).status();
    let snapshot = if let Some(runtime) = runtime {
        Some(
            tokio::task::spawn_blocking(move || runtime.snapshot())
                .await
                .map_err(|_| AdminError::WorkerFailed)?
                .map_err(AdminError::from)?,
        )
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
    snapshot: Option<AppSnapshot>,
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
            RuntimeError::Device { .. } | RuntimeError::Provider { .. } => Self::Runtime {
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
