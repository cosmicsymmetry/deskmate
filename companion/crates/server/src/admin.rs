//! Admin-token-protected provisioning, configuration, and status routes.

use app_core::{
    AppConfig, AppSnapshot, BakedFontMetrics, ClockCard, MAX_CONFIG_FILE_BYTES, RuntimeError,
    SaveReceipt, StoreError, ValidationIssue, build_digital_clock_scene,
};
use axum::Json;
use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{DefaultBodyLimit, FromRequestParts, Path, State};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use chrono::NaiveDateTime;
use protocol::{Message, PushScene, validate_message};
use serde::{Deserialize, Serialize, Serializer};

use crate::ServerState;
use crate::auth::bearer_token;
use crate::plugin_registry::PluginRegistry;

/// Matches the standard provider response ceiling and stays below Axum's
/// independent 2 MiB default limit for the complete JSON request body.
const MAX_OPERATOR_PLUGIN_DATA_BYTES: usize = providers::MAX_PROVIDER_RESPONSE_BYTES;

pub(crate) fn routes() -> Router<ServerState> {
    Router::new()
        .route("/v1/devices", post(create_device))
        .route("/v1/plugins", get(get_plugins))
        .route("/v1/devices/{id}", get(get_device))
        .route(
            "/v1/devices/{id}/config",
            put(put_config).layer(DefaultBodyLimit::max(MAX_CONFIG_FILE_BYTES)),
        )
        .route("/v1/devices/{id}/scene", post(post_scene))
}

async fn get_plugins(
    State(state): State<ServerState>,
    _admin: AdminAuthenticated,
) -> Json<PluginCatalogResponse> {
    let plugins = state
        .plugins()
        .ids()
        .filter_map(|id| state.plugins().get(id))
        .map(|plugin| {
            let mut assets: Vec<_> = plugin
                .assets
                .iter()
                .map(|(file, asset)| PluginAssetResponse {
                    file: file.to_string(),
                    kind: asset_kind_name(asset.kind),
                    byte_length: asset.bytes.len(),
                    digest: digest_hex(&asset.digest),
                })
                .collect();
            assets.sort_by(|left, right| left.file.cmp(&right.file));
            PluginResponse {
                id: plugin.id.clone(),
                name: plugin.manifest.name.clone(),
                version: plugin.manifest.version.clone(),
                node_count: plugin.manifest.nodes.len()
                    + plugin
                        .manifest
                        .repeats
                        .iter()
                        .map(|repeat| repeat.nodes.len())
                        .sum::<usize>(),
                assets,
            }
        })
        .collect();
    let load_failures = state
        .plugin_load_failures()
        .iter()
        .map(|failure| PluginLoadFailureResponse {
            id: failure.id.clone(),
            error: failure.error.to_string(),
        })
        .collect();
    Json(PluginCatalogResponse {
        plugins,
        load_failures,
    })
}

#[derive(Debug, Serialize)]
struct PluginCatalogResponse {
    plugins: Vec<PluginResponse>,
    load_failures: Vec<PluginLoadFailureResponse>,
}

#[derive(Debug, Serialize)]
struct PluginResponse {
    id: String,
    name: String,
    version: String,
    node_count: usize,
    assets: Vec<PluginAssetResponse>,
}

#[derive(Debug, Serialize)]
struct PluginAssetResponse {
    file: String,
    kind: &'static str,
    byte_length: usize,
    digest: String,
}

#[derive(Debug, Serialize)]
struct PluginLoadFailureResponse {
    id: String,
    error: String,
}

fn asset_kind_name(kind: protocol::AssetKind) -> &'static str {
    match kind {
        protocol::AssetKind::Font => "font",
        protocol::AssetKind::IconFont => "icon-font",
        protocol::AssetKind::Image => "image",
    }
}

fn digest_hex(digest: &[u8; protocol::ASSET_DIGEST_LEN]) -> String {
    use std::fmt::Write as _;

    let mut encoded = String::with_capacity(protocol::ASSET_DIGEST_LEN * 2);
    for byte in digest {
        write!(encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
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

#[derive(Deserialize)]
struct PushSceneRequest {
    card_id: String,
    revision: Option<u32>,
    template: String,
    show_seconds: Option<bool>,
    local_now: Option<String>,
    plugin_id: Option<String>,
    #[serde(default)]
    data: OptionalSceneData,
    #[serde(default)]
    stale: bool,
    error: Option<String>,
}

#[derive(Default)]
enum OptionalSceneData {
    #[default]
    Missing,
    Present(serde_json::Value),
}

impl<'de> Deserialize<'de> for OptionalSceneData {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        serde_json::Value::deserialize(deserializer).map(Self::Present)
    }
}

#[derive(Debug)]
enum OperatorScene {
    Push(PushScene),
    Plugin {
        card_id: String,
        plugin_id: String,
        snapshot: providers::ProviderSnapshot<serde_json::Value>,
    },
}

/// Builds and pushes an operator-selected scene through an existing device runtime.
/// Plugin input is cached in that runtime and rendered by its normal negotiated
/// executor; only the legacy digital-clock diagnostic keeps the direct `PushScene` form.
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
    let scene = build_operator_scene(request, state.plugins())?;
    let runtime = state
        .device_link(&device_id)
        .and_then(|link| link.runtime())
        .ok_or_else(|| AdminError::from(RuntimeError::DeviceDisconnected))?;
    let result = tokio::task::spawn_blocking(move || match scene {
        OperatorScene::Push(push) => runtime.push_scene(push),
        OperatorScene::Plugin {
            card_id,
            plugin_id,
            snapshot,
        } => runtime.inject_plugin_snapshot(card_id, plugin_id, snapshot),
    })
    .await
    .map_err(|_| AdminError::WorkerFailed)?;
    map_operator_runtime_result(result)?;

    Ok(StatusCode::OK)
}

fn map_operator_runtime_result(result: Result<(), RuntimeError>) -> Result<(), AdminError> {
    if let Err(RuntimeError::Provider { message }) = &result
        && message.starts_with("operator-plugin-mismatch:")
    {
        return Err(AdminError::InvalidScene {
            message: message
                .strip_prefix("operator-plugin-mismatch: ")
                .unwrap_or(message)
                .to_owned(),
        });
    }
    result.map_err(AdminError::from)
}

fn build_operator_scene(
    request: PushSceneRequest,
    plugins: &std::sync::Arc<PluginRegistry>,
) -> Result<OperatorScene, AdminError> {
    let PushSceneRequest {
        card_id,
        revision,
        template,
        show_seconds,
        local_now,
        plugin_id,
        data,
        stale,
        error,
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
            Ok(OperatorScene::Push(push))
        }
        "plugin" => {
            let plugin_id = plugin_id.ok_or_else(|| AdminError::InvalidScene {
                message: "plugin_id is required for scene template \"plugin\"".into(),
            })?;
            let OptionalSceneData::Present(data) = data else {
                return Err(AdminError::InvalidScene {
                    message: "data is required for scene template \"plugin\"".into(),
                });
            };
            let data_length = serde_json::to_vec(&data)
                .map_err(|error| AdminError::InvalidScene {
                    message: format!("plugin data could not be measured: {error}"),
                })?
                .len();
            if data_length > MAX_OPERATOR_PLUGIN_DATA_BYTES {
                return Err(AdminError::InvalidScene {
                    message: format!(
                        "plugin data is {data_length} bytes; the limit is {MAX_OPERATOR_PLUGIN_DATA_BYTES}"
                    ),
                });
            }
            if plugins.get(&plugin_id).is_none() {
                return Err(AdminError::InvalidScene {
                    message: format!("unknown plugin id {plugin_id:?}"),
                });
            }
            let snapshot = providers::ProviderSnapshot {
                value: data,
                refreshed_at: None,
                age: None,
                stale,
                error,
            };
            Ok(OperatorScene::Plugin {
                card_id,
                plugin_id,
                snapshot,
            })
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
    use std::path::Path;
    use std::sync::Arc;

    use app_core::RuntimeError;
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use axum::response::Response;
    use tower::ServiceExt as _;

    use super::{
        AdminError, MAX_OPERATOR_PLUGIN_DATA_BYTES, OperatorScene, PushSceneRequest,
        build_operator_scene, insert_last_ota_error, insert_observed_age_seconds,
        map_operator_runtime_result, observed_age_seconds,
    };

    const AQI_FIXTURE: &str = include_str!("../../plugin/tests/fixtures/aqi_response.json");
    const AGENDA_FIXTURE: &str = include_str!("../../plugin/tests/fixtures/agenda_response.json");

    fn curated_plugins_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins")
    }

    fn curated_registry() -> Arc<crate::plugin_registry::PluginRegistry> {
        let (registry, failures) =
            crate::plugin_registry::PluginRegistry::load(&curated_plugins_dir())
                .expect("load curated plugins");
        assert!(failures.is_empty(), "unexpected failures: {failures:?}");
        Arc::new(registry)
    }

    fn payload_from_envelope(raw: &str) -> serde_json::Value {
        let root: serde_json::Value = serde_json::from_str(raw).expect("fixture is valid JSON");
        assert_eq!(root["status"], "ok");
        root["payload"].clone()
    }

    fn request(value: serde_json::Value) -> PushSceneRequest {
        serde_json::from_value(value).expect("valid scene request fixture")
    }

    fn plugin_request(
        plugin_id: &str,
        data: &serde_json::Value,
        stale: bool,
        error: Option<&str>,
    ) -> PushSceneRequest {
        request(serde_json::json!({
            "card_id": format!("{plugin_id}-card"),
            "revision": 17,
            "template": "plugin",
            "plugin_id": plugin_id,
            "data": data,
            "stale": stale,
            "error": error,
        }))
    }

    fn state_with_curated_plugins() -> (crate::ServerState, String, tempfile::TempDir) {
        let config_dir = tempfile::tempdir().expect("config tempdir");
        let state = crate::ServerState::new_with_plugins(
            "admin-secret".into(),
            crate::firmware::FirmwareCatalog::in_memory(),
            config_dir.path().to_path_buf(),
            curated_registry(),
            Vec::new(),
        );
        let identity = state.registry().mint().expect("mint test device");
        (state, identity.device_id, config_dir)
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
            builder = builder.header("authorization", "Bearer admin-secret");
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

    async fn response_json(response: Response) -> serde_json::Value {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        serde_json::from_slice(&body).expect("typed JSON response")
    }

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

    #[test]
    fn aqi_plugin_request_queues_the_real_unwrapped_fixture_for_the_runtime() {
        let registry = curated_registry();
        let action = build_operator_scene(
            plugin_request("aqi", &payload_from_envelope(AQI_FIXTURE), false, None),
            &registry,
        )
        .expect("prepare AQI fixture through the admin path");

        assert!(
            matches!(action, OperatorScene::Plugin { card_id, plugin_id, snapshot } if card_id == "aqi-card" && plugin_id == "aqi" && snapshot.value == payload_from_envelope(AQI_FIXTURE))
        );
    }

    #[test]
    fn agenda_plugin_request_queues_the_real_unwrapped_fixture_for_the_runtime() {
        let registry = curated_registry();
        let action = build_operator_scene(
            plugin_request(
                "agenda",
                &payload_from_envelope(AGENDA_FIXTURE),
                false,
                None,
            ),
            &registry,
        )
        .expect("prepare agenda fixture through the admin path");

        assert!(
            matches!(action, OperatorScene::Plugin { plugin_id, snapshot, .. } if plugin_id == "agenda" && snapshot.value == payload_from_envelope(AGENDA_FIXTURE))
        );
    }

    #[test]
    fn explicit_json_null_is_present_plugin_data_not_a_missing_field() {
        let registry = curated_registry();
        let action = build_operator_scene(
            plugin_request("aqi", &serde_json::Value::Null, false, None),
            &registry,
        )
        .expect("JSON null is an arbitrary JSON value, not an absent data field");

        assert!(
            matches!(action, OperatorScene::Plugin { snapshot, .. } if snapshot.value.is_null())
        );
    }

    #[test]
    fn plugin_stale_and_error_state_each_reach_the_runtime_snapshot() {
        let registry = curated_registry();
        let data = payload_from_envelope(AQI_FIXTURE);
        let clean = build_operator_scene(plugin_request("aqi", &data, false, None), &registry)
            .expect("prepare clean snapshot");
        let stale = build_operator_scene(plugin_request("aqi", &data, true, None), &registry)
            .expect("prepare stale snapshot");
        let errored = build_operator_scene(
            plugin_request("aqi", &data, false, Some("operator supplied error")),
            &registry,
        )
        .expect("prepare errored snapshot");

        let OperatorScene::Plugin {
            snapshot: clean, ..
        } = clean
        else {
            panic!()
        };
        let OperatorScene::Plugin {
            snapshot: stale, ..
        } = stale
        else {
            panic!()
        };
        let OperatorScene::Plugin {
            snapshot: errored, ..
        } = errored
        else {
            panic!()
        };
        assert!(!clean.stale && clean.error.is_none());
        assert!(stale.stale && stale.error.is_none());
        assert_eq!(errored.error.as_deref(), Some("operator supplied error"));
    }

    #[test]
    fn digital_clock_request_still_builds_the_exact_existing_scene() {
        let registry = curated_registry();
        let local_now = "2026-08-25T14:37:42";
        let action = build_operator_scene(
            request(serde_json::json!({
                "card_id": "clock-1",
                "revision": 17,
                "template": "digital_clock",
                "show_seconds": true,
                "local_now": local_now,
            })),
            &registry,
        )
        .expect("build digital clock through the refactored path");
        let OperatorScene::Push(push) = action else {
            panic!("digital clock must remain a direct push")
        };

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

    #[test]
    fn operator_plugin_data_over_the_cap_is_rejected_even_when_the_manifest_ignores_it() {
        let registry = curated_registry();
        let mut data = payload_from_envelope(AQI_FIXTURE);
        data["unused"] = serde_json::Value::String("x".repeat(MAX_OPERATOR_PLUGIN_DATA_BYTES));

        let error = build_operator_scene(plugin_request("aqi", &data, false, None), &registry)
            .expect_err("oversized operator data must be refused before compilation");
        let AdminError::InvalidScene { message } = error else {
            panic!("oversized data returned the wrong error type: {error:?}");
        };
        assert!(message.contains("plugin data is"));
        assert!(message.contains(&MAX_OPERATOR_PLUGIN_DATA_BYTES.to_string()));
    }

    #[tokio::test]
    async fn unknown_plugin_id_is_a_typed_bad_request_naming_only_that_id() {
        let (state, device_id, _config_dir) = state_with_curated_plugins();
        let response = post_scene_request(
            state,
            &device_id,
            serde_json::json!({
                "card_id": "missing-card",
                "revision": 17,
                "template": "plugin",
                "plugin_id": "not-installed",
                "data": {},
            }),
            true,
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let error = response_json(response).await;
        assert_eq!(error["kind"], "invalid-scene");
        assert_eq!(error["message"], "unknown plugin id \"not-installed\"");
    }

    #[test]
    fn plugin_payload_is_not_compiled_in_the_route_before_the_runtime_executor() {
        let registry = curated_registry();
        let mut data = payload_from_envelope(AQI_FIXTURE);
        data["current"]["category"] = serde_json::Value::String("x".repeat(4097));
        let action = build_operator_scene(plugin_request("aqi", &data, false, None), &registry)
            .expect("route preparation must defer plugin compilation");

        assert!(
            matches!(action, OperatorScene::Plugin { plugin_id, snapshot, .. } if plugin_id == "aqi" && snapshot.value == data)
        );
    }

    #[test]
    fn plugin_operator_request_needs_no_caller_revision_because_runtime_mints_it() {
        let registry = curated_registry();
        let action = build_operator_scene(
            request(serde_json::json!({
                "card_id": "aqi-card",
                "template": "plugin",
                "plugin_id": "aqi",
                "data": payload_from_envelope(AQI_FIXTURE),
            })),
            &registry,
        )
        .expect("plugin request without the revision footgun");

        assert!(
            matches!(action, OperatorScene::Plugin { card_id, plugin_id, .. } if card_id == "aqi-card" && plugin_id == "aqi")
        );
    }

    #[test]
    fn runtime_plugin_card_mismatch_maps_to_the_routes_typed_invalid_scene_error() {
        let error = map_operator_runtime_result(Err(RuntimeError::Provider {
            message: "operator-plugin-mismatch: card \"air\" is configured for plugin \"aqi\", not \"agenda\"".into(),
        }))
        .expect_err("mismatch must remain a typed scene request error");

        assert!(
            matches!(error, AdminError::InvalidScene { message } if message.contains("configured for plugin") && message.contains("aqi") && message.contains("agenda"))
        );
    }

    #[tokio::test]
    async fn plugin_template_without_data_is_a_typed_bad_request() {
        let (state, device_id, _config_dir) = state_with_curated_plugins();
        let response = post_scene_request(
            state,
            &device_id,
            serde_json::json!({
                "card_id": "aqi-card",
                "revision": 17,
                "template": "plugin",
                "plugin_id": "aqi",
            }),
            true,
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let error = response_json(response).await;
        assert_eq!(error["kind"], "invalid-scene");
        assert_eq!(
            error["message"],
            "data is required for scene template \"plugin\""
        );
    }

    #[tokio::test]
    async fn plugin_template_without_plugin_id_is_a_typed_bad_request() {
        let (state, device_id, _config_dir) = state_with_curated_plugins();
        let response = post_scene_request(
            state,
            &device_id,
            serde_json::json!({
                "card_id": "plugin-card",
                "revision": 17,
                "template": "plugin",
                "data": {},
            }),
            true,
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let error = response_json(response).await;
        assert_eq!(error["kind"], "invalid-scene");
        assert_eq!(
            error["message"],
            "plugin_id is required for scene template \"plugin\""
        );
    }

    #[tokio::test]
    async fn scene_route_requires_admin_authentication() {
        let response = post_scene_request(
            crate::ServerState::in_memory(),
            "dev-unknown",
            serde_json::json!({
                "card_id": "aqi-card",
                "revision": 17,
                "template": "plugin",
                "plugin_id": "aqi",
                "data": {},
            }),
            false,
        )
        .await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn plugin_catalog_digest_matches_resolve_assets_for_the_same_file() {
        let plugins_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins");
        let (registry, failures) = crate::plugin_registry::PluginRegistry::load(&plugins_dir)
            .expect("load curated plugins");
        assert!(failures.is_empty());
        let manifest_source = std::fs::read_to_string(plugins_dir.join("aqi/manifest.toml"))
            .expect("read AQI manifest");
        let manifest = plugin::parse_manifest(&manifest_source).expect("parse AQI manifest");
        let independently_resolved = plugin::resolve_assets(&manifest, &plugins_dir.join("aqi"))
            .expect("resolve AQI assets");
        let expected = super::digest_hex(
            &independently_resolved
                .get("icons.ttf")
                .expect("AQI icon font")
                .digest,
        );
        let config_dir = tempfile::tempdir().expect("config tempdir");
        let state = crate::ServerState::new_with_plugins(
            "admin-secret".into(),
            crate::firmware::FirmwareCatalog::in_memory(),
            config_dir.path().to_path_buf(),
            Arc::new(registry),
            vec![crate::plugin_registry::PluginLoadFailure {
                id: "broken".into(),
                error: crate::plugin_registry::PluginLoadError::ManifestRead {
                    path: plugins_dir.join("broken/manifest.toml"),
                    message: "fixture failure".into(),
                },
            }],
        );

        let router = crate::app(state);
        let unauthorized = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/plugins")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("unauthorized route response");
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/v1/plugins")
                    .header("authorization", "Bearer admin-secret")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("route response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("parse response JSON");
        let aqi = body["plugins"]
            .as_array()
            .expect("plugins array")
            .iter()
            .find(|plugin| plugin["id"] == "aqi")
            .expect("AQI response entry");
        let icons = aqi["assets"]
            .as_array()
            .expect("assets array")
            .iter()
            .find(|asset| asset["file"] == "icons.ttf")
            .expect("AQI icon response entry");

        assert_eq!(icons["digest"], expected);
        assert_eq!(
            icons["byte_length"],
            std::fs::metadata(plugins_dir.join("aqi/icons.ttf"))
                .unwrap()
                .len()
        );
        assert_eq!(body["load_failures"][0]["id"], "broken");
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
    InvalidScene { message: &'a str },
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
            Self::InvalidScene { message } => (
                StatusCode::BAD_REQUEST,
                Json(ErrorBody::InvalidScene { message: &message }),
            )
                .into_response(),
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
