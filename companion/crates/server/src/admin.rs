//! Admin-token-protected provisioning, configuration, and status routes.

use app_core::{
    AdminConfigErrorBody, AppConfig, AppSnapshot, BakedFontMetrics, ClockCard,
    MAX_CONFIG_FILE_BYTES, RuntimeError, SaveReceipt, StoreError, ValidationIssue,
    build_digital_clock_scene,
};
use axum::Json;
use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use chrono::NaiveDateTime;
use protocol::{Message, PushScene, validate_message};
use serde::{Deserialize, Serialize, Serializer};

use crate::ServerState;
use crate::auth::AdminAuthenticated;

pub(crate) fn routes() -> Router<ServerState> {
    Router::new()
        .route("/v1/devices", post(create_device))
        .route("/v1/devices/{id}", get(get_device))
        .route(
            "/v1/devices/{id}/config",
            put(put_config).layer(DefaultBodyLimit::max(MAX_CONFIG_FILE_BYTES)),
        )
        .route("/v1/devices/{id}/scene", post(post_scene))
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

#[derive(Deserialize)]
struct PushSceneRequest {
    card_id: String,
    revision: Option<u32>,
    template: String,
    show_seconds: Option<bool>,
    local_now: Option<String>,
}

/// Builds and pushes an operator-selected scene through an existing device runtime.
async fn post_scene(
    State(state): State<ServerState>,
    _admin: AdminAuthenticated,
    Path(device_id): Path<String>,
    payload: Result<Json<PushSceneRequest>, JsonRejection>,
) -> Result<StatusCode, AdminError> {
    if !state.registry().contains_device(&device_id) {
        return Err(AdminError::NotFound);
    }
    let Json(request) = payload.map_err(|rejection| AdminError::InvalidJson {
        status: rejection.status(),
        message: rejection.body_text(),
    })?;
    let scene = build_operator_scene(request)?;
    let runtime = state
        .device_link(&device_id)
        .and_then(|link| link.runtime())
        .ok_or_else(|| AdminError::from(RuntimeError::DeviceDisconnected))?;
    let result = tokio::task::spawn_blocking(move || runtime.push_scene(scene))
        .await
        .map_err(|_| AdminError::WorkerFailed)?;
    result.map_err(AdminError::from)?;

    Ok(StatusCode::OK)
}

fn build_operator_scene(request: PushSceneRequest) -> Result<PushScene, AdminError> {
    let PushSceneRequest {
        card_id,
        revision,
        template,
        show_seconds,
        local_now,
    } = request;
    match template.as_str() {
        "digital_clock" => {
            let revision = revision.ok_or_else(|| AdminError::InvalidScene {
                message: "revision is required for scene template \"digital_clock\"".into(),
            })?;
            let show_seconds = show_seconds.ok_or_else(|| AdminError::InvalidScene {
                message: "show_seconds is required for scene template \"digital_clock\"".into(),
            })?;
            let local_now = local_now.ok_or_else(|| AdminError::InvalidScene {
                message: "local_now is required for scene template \"digital_clock\"".into(),
            })?;
            let local_now = NaiveDateTime::parse_from_str(&local_now, "%Y-%m-%dT%H:%M:%S")
                .map_err(|_| AdminError::InvalidScene {
                    message: "local_now must use YYYY-MM-DDTHH:MM:SS".into(),
                })?;
            let scene = build_digital_clock_scene(
                &ClockCard {
                    revision,
                    show_seconds,
                    local_now,
                },
                &BakedFontMetrics::SHIPPED,
            );
            let push = PushScene {
                card_id,
                revision,
                scene,
            };
            validate_message(&Message::PushScene(push.clone())).map_err(|error| {
                AdminError::InvalidScene {
                    message: error.to_string(),
                }
            })?;
            Ok(push)
        }
        _ => Err(AdminError::InvalidScene {
            message: format!("unknown scene template {template:?}"),
        }),
    }
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
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use axum::response::Response;
    use tower::ServiceExt as _;

    use super::{
        PushSceneRequest, build_operator_scene, insert_last_ota_error, insert_observed_age_seconds,
        observed_age_seconds,
    };

    fn request(value: serde_json::Value) -> PushSceneRequest {
        serde_json::from_value(value).expect("valid scene request fixture")
    }

    async fn post_scene_request(
        state: crate::ServerState,
        device_id: &str,
        body: serde_json::Value,
        authorized: bool,
    ) -> Response {
        let mut builder = Request::builder()
            .method("POST")
            .uri(format!("/v1/devices/{device_id}/scene"))
            .header("content-type", "application/json");
        if authorized {
            builder = builder.header("authorization", "Bearer in-memory-admin-token");
        }
        crate::app(state)
            .oneshot(
                builder
                    .body(Body::from(body.to_string()))
                    .expect("scene request"),
            )
            .await
            .expect("scene response")
    }

    #[test]
    fn observed_age_is_inserted_beside_the_values_it_qualifies() {
        let mut snapshot = serde_json::json!({"device": {"uptime_ms": 66761}});
        insert_observed_age_seconds(&mut snapshot, Some(412)).unwrap();
        assert_eq!(snapshot["device"]["observed_age_seconds"], 412);
        assert_eq!(snapshot["device"]["uptime_ms"], 66761);
    }

    #[test]
    fn a_device_never_heard_from_has_no_age_rather_than_a_zero_one() {
        assert_eq!(observed_age_seconds(None, 1_000_000), None);
        let mut snapshot = serde_json::json!({"device": {}});
        insert_observed_age_seconds(&mut snapshot, None).unwrap();
        assert!(snapshot["device"]["observed_age_seconds"].is_null());
    }

    #[test]
    fn observed_age_saturates_rather_than_wrapping_on_a_backwards_clock() {
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

    #[test]
    fn digital_clock_request_still_builds_the_exact_existing_scene() {
        let local_now = "2026-08-25T14:37:42";
        let push = build_operator_scene(request(serde_json::json!({
            "card_id": "clock-1",
            "revision": 17,
            "template": "digital_clock",
            "show_seconds": true,
            "local_now": local_now,
        })))
        .expect("build digital clock through the diagnostic path");

        assert_eq!(push.card_id, "clock-1");
        assert_eq!(push.revision, 17);
        assert_eq!(
            push.scene,
            app_core::build_digital_clock_scene(
                &app_core::ClockCard {
                    revision: 17,
                    show_seconds: true,
                    local_now: local_now.parse().expect("valid test instant"),
                },
                &app_core::BakedFontMetrics::SHIPPED,
            )
        );
    }

    #[tokio::test]
    async fn scene_route_requires_admin_authentication() {
        let response = post_scene_request(
            crate::ServerState::in_memory(),
            "dev-unknown",
            serde_json::json!({
                "card_id": "clock-1",
                "revision": 17,
                "template": "digital_clock",
                "show_seconds": true,
                "local_now": "2026-08-25T14:37:42",
            }),
            false,
        )
        .await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn unknown_scene_template_is_a_typed_bad_request() {
        let state = crate::ServerState::in_memory();
        let identity = state.registry().mint().expect("mint test device");
        let response = post_scene_request(
            state.clone(),
            &identity.device_id,
            serde_json::json!({
                "card_id": "old-card",
                "revision": 17,
                "template": "retired",
            }),
            true,
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let error: serde_json::Value = serde_json::from_slice(&body).expect("typed JSON response");
        assert_eq!(error["message"], "unknown scene template \"retired\"");
        state.shutdown();
    }
}

enum SaveConfigError {
    Invalid(Vec<ValidationIssue>),
    Store(StoreError),
}

#[derive(Debug)]
enum AdminError {
    NotFound,
    InvalidJson { status: StatusCode, message: String },
    InvalidScene { message: String },
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
            | RuntimeError::ImageSource { .. } => Self::Runtime {
                status: StatusCode::BAD_GATEWAY,
                message: error.to_string(),
            },
            RuntimeError::UnknownCard { .. } => Self::Runtime {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                message: error.to_string(),
            },
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum ErrorBody<'a> {
    InvalidJson { message: &'a str },
    InvalidScene { message: &'a str },
    Store { message: &'a str },
    Runtime { message: &'a str },
    Internal,
}

impl IntoResponse for AdminError {
    fn into_response(self) -> Response {
        match self {
            Self::NotFound => StatusCode::NOT_FOUND.into_response(),
            Self::InvalidJson { status, message } => {
                (status, Json(ErrorBody::InvalidJson { message: &message })).into_response()
            }
            Self::InvalidScene { message } => (
                StatusCode::BAD_REQUEST,
                Json(ErrorBody::InvalidScene { message: &message }),
            )
                .into_response(),
            Self::InvalidConfig { issues } => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(AdminConfigErrorBody::InvalidConfig { issues }),
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
