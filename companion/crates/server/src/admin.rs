//! Admin-token-protected provisioning, configuration, and status routes.

use app_core::{
    AdminConfigErrorBody, AppConfig, AppSnapshot, BakedFontMetrics, CardPreviewResponse, ClockCard,
    MAX_CONFIG_FILE_BYTES, PluginCatalog, PluginCatalogAsset, PluginCatalogEntry,
    PluginTemplateKind, RuntimeError, SaveReceipt, StoreError, ValidationIssue,
    build_digital_clock_scene,
};
use axum::Json;
use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{DefaultBodyLimit, FromRequestParts, Path, State};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use base64::Engine as _;
use chrono::NaiveDateTime;
use protocol::{Message, PushScene, validate_message};
use serde::{Deserialize, Serialize, Serializer};

use crate::ServerState;
use crate::auth::bearer_token;
use crate::plugin_registry::{LoadedPlugin, PluginRegistry};

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
        .route(
            "/v1/devices/{id}/cards/{card_id}/preview",
            get(get_card_preview),
        )
}

async fn get_plugins(
    State(state): State<ServerState>,
    _admin: AdminAuthenticated,
) -> Json<PluginCatalog> {
    let plugins = state
        .plugins()
        .ids()
        .filter_map(|id| state.plugins().get(id))
        .map(catalog_entry)
        .collect();
    let load_failures = state
        .plugin_load_failures()
        .iter()
        .map(|failure| app_core::admin::PluginLoadFailure {
            id: failure.id.clone(),
            error: failure.error.to_string(),
        })
        .collect();
    Json(PluginCatalog {
        plugins,
        load_failures,
    })
}

fn catalog_entry(loaded: &LoadedPlugin) -> PluginCatalogEntry {
    let mut assets: Vec<_> = loaded
        .assets
        .iter()
        .map(|(file, asset)| PluginCatalogAsset {
            file: file.to_string(),
            kind: asset_kind_name(asset.kind).to_owned(),
            byte_length: asset.bytes.len(),
            digest: protocol::digest_hex(&asset.digest),
        })
        .collect();
    assets.sort_by(|left, right| left.file.cmp(&right.file));
    let plugin::Source::Json {
        refresh_minutes, ..
    } = &loaded.manifest.source;
    PluginCatalogEntry {
        id: loaded.id.clone(),
        name: loaded.manifest.name.clone(),
        version: loaded.manifest.version.clone(),
        node_count: loaded.manifest.nodes.len()
            + loaded
                .manifest
                .repeats
                .iter()
                .map(|repeat| repeat.nodes.len())
                .sum::<usize>(),
        assets,
        display_name: loaded.manifest.display_name.clone(),
        description: loaded.manifest.description.clone(),
        manifest_version: match loaded.manifest.manifest_version {
            plugin::ManifestVersion::V1 => 1,
            plugin::ManifestVersion::V2 => 2,
        },
        template: match &loaded.manifest.template {
            plugin::Template::Scene => PluginTemplateKind::DisplayList,
            plugin::Template::Svg { .. } => PluginTemplateKind::Svg,
        },
        // `parse_manifest` bounds this to `plugin::MAX_REFRESH_MINUTES`
        // (1440), so the saturating arm is unreachable today; it is here so a
        // future bound change cannot silently wrap a cadence.
        refresh_minutes: u16::try_from(*refresh_minutes).unwrap_or(u16::MAX),
    }
}

fn asset_kind_name(kind: protocol::AssetKind) -> &'static str {
    match kind {
        protocol::AssetKind::Font => "font",
        protocol::AssetKind::IconFont => "icon-font",
        protocol::AssetKind::Image => "image",
    }
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

/// Renders one plugin card's current face as a PNG, without touching the
/// device. The runtime builds it from the snapshot the panel is already
/// showing, at revision 0: nothing is minted, nothing is sent, and the card
/// is not activated. Rate limiting is the Mac's polling cadence, not ours.
async fn get_card_preview(
    State(state): State<ServerState>,
    _admin: AdminAuthenticated,
    Path((device_id, card_id)): Path<(String, String)>,
) -> Result<Json<CardPreviewResponse>, AdminError> {
    if !state.registry().contains_device(&device_id) {
        return Err(AdminError::NotFound);
    }
    let runtime = state
        .device_link(&device_id)
        .and_then(|link| link.runtime())
        .ok_or_else(|| AdminError::Runtime {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: "this device has no runtime yet; it has never connected".into(),
        })?;
    let preview = tokio::task::spawn_blocking(move || runtime.render_card_preview(&card_id))
        .await
        .map_err(|_| AdminError::WorkerFailed)?
        .map_err(AdminError::from)?;

    // A frame this server rendered is canonical by construction, so this maps a
    // fault rather than a caller's mistake -- but it maps it, because a handler
    // that panics on a wrong length takes the connection with it.
    let png_base64 = match preview.frame.as_ref() {
        None => None,
        Some(frame) => Some(
            crate::rasterizer::frame_png(&frame.bytes)
                .map(|png| base64::engine::general_purpose::STANDARD.encode(png))
                .map_err(|error| AdminError::Runtime {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    message: error.to_string(),
                })?,
        ),
    };

    Ok(Json(CardPreviewResponse {
        png_base64,
        state: preview.state,
        message: preview.message,
        refreshed_at_unix_ms: preview.refreshed_at_unix_ms,
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
    use base64::Engine as _;
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
        let expected = protocol::digest_hex(
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

    async fn plugin_catalog_json(state: crate::ServerState) -> serde_json::Value {
        let response = crate::app(state)
            .oneshot(
                Request::builder()
                    .uri("/v1/plugins")
                    .header("authorization", "Bearer admin-secret")
                    .body(Body::empty())
                    .expect("catalog request"),
            )
            .await
            .expect("catalog response");
        assert_eq!(response.status(), StatusCode::OK);
        response_json(response).await
    }

    /// The tempdir is returned, not dropped: `ServerState` keeps the path and
    /// deleting the directory out from under it would be a different test.
    fn state_with_registry(
        registry: Arc<crate::plugin_registry::PluginRegistry>,
    ) -> (crate::ServerState, tempfile::TempDir) {
        let config_dir = tempfile::tempdir().expect("config tempdir");
        let state = crate::ServerState::new_with_plugins(
            "admin-secret".into(),
            crate::firmware::FirmwareCatalog::in_memory(),
            config_dir.path().to_path_buf(),
            registry,
            Vec::new(),
        );
        (state, config_dir)
    }

    #[tokio::test]
    async fn the_catalog_keeps_every_field_the_previous_response_carried() {
        // The five keys below are additive. A companion built before this
        // change reads the rest, so a renamed key or a moved value here is a
        // silent break rather than a compile error. Task 2's app-core test
        // pins the serialized key ORDER; this pins the values the server puts
        // in them for the real curated registry.
        let (state, _device_id, _config_dir) = state_with_curated_plugins();
        let mut legacy = plugin_catalog_json(state).await;
        for entry in legacy["plugins"].as_array_mut().expect("plugins array") {
            let entry = entry.as_object_mut().expect("plugin entry object");
            for added in [
                "display_name",
                "description",
                "manifest_version",
                "template",
                "refresh_minutes",
            ] {
                assert!(entry.remove(added).is_some(), "{added} is not in the entry");
            }
        }

        assert_eq!(
            legacy,
            serde_json::json!({
                "plugins": [
                    {"id": "agenda", "name": "agenda", "version": "1.0.0", "node_count": 4,
                     "assets": [{"file": "badge.rgb565", "kind": "image", "byte_length": 812,
                                 "digest": "e19db5d47bbdae46bf80e5a7df400795bce84973817c9f124642bc76bf41d33a"}]},
                    {"id": "aqi", "name": "aqi", "version": "1.0.0", "node_count": 7,
                     "assets": [{"file": "icons.ttf", "kind": "icon-font", "byte_length": 4320,
                                 "digest": "40bbbac715465adf7ba539f53a0cb16991a2c1f73a4fb57af209d39e0c62b327"}]},
                    {"id": "claude-limits", "name": "claude-limits", "version": "1.0.0",
                     "node_count": 12, "assets": []},
                    {"id": "svg-aqi", "name": "svg-aqi", "version": "1.0.0", "node_count": 0,
                     "assets": []}
                ],
                "load_failures": []
            })
        );
    }

    #[tokio::test]
    async fn the_catalog_reports_each_manifests_template_kind_and_cadence() {
        let (state, _device_id, _config_dir) = state_with_curated_plugins();
        let body = plugin_catalog_json(state).await;
        let entry = |id: &str| {
            body["plugins"]
                .as_array()
                .expect("plugins array")
                .iter()
                .find(|entry| entry["id"] == id)
                .unwrap_or_else(|| panic!("no catalog entry for {id}"))
                .clone()
        };

        // The editor's Plugin field and the picker read these; `svg-aqi` is
        // the only curated plugin the panel can never render natively.
        assert_eq!(entry("aqi")["template"], "display-list");
        assert_eq!(entry("svg-aqi")["template"], "svg");
        assert_eq!(entry("aqi")["refresh_minutes"], 15);
        assert_eq!(entry("agenda")["refresh_minutes"], 10);
    }

    #[tokio::test]
    async fn a_v2_manifests_own_display_name_and_description_reach_the_catalog() {
        let (registry, _base) =
            crate::test_plugins::fixture_registry(crate::test_plugins::V2_NAMED_MANIFEST);
        let (state, _config_dir) = state_with_registry(registry);

        let body = plugin_catalog_json(state).await;

        assert_eq!(body["plugins"][0]["display_name"], "Fixture plugin");
        assert_eq!(
            body["plugins"][0]["description"],
            "Names and cadence, threaded from the manifest"
        );
        assert_eq!(body["plugins"][0]["manifest_version"], 2);
        assert_eq!(body["plugins"][0]["refresh_minutes"], 7);
    }

    #[tokio::test]
    async fn a_v1_manifest_reports_version_one_and_no_names_rather_than_failing() {
        // v1 stays frozen: absence is the normal case, not a load failure.
        let (registry, _base) =
            crate::test_plugins::fixture_registry(crate::test_plugins::V1_MANIFEST);
        let (state, _config_dir) = state_with_registry(registry);

        let body = plugin_catalog_json(state).await;

        assert_eq!(body["plugins"][0]["manifest_version"], 1);
        assert!(body["plugins"][0]["display_name"].is_null());
        assert!(body["plugins"][0]["description"].is_null());
        assert_eq!(body["load_failures"].as_array().unwrap().len(), 0);
    }

    async fn preview_request(
        state: crate::ServerState,
        device_id: &str,
        card_id: &str,
        authorized: bool,
    ) -> Response {
        let mut builder = Request::builder()
            .method("GET")
            .uri(format!("/v1/devices/{device_id}/cards/{card_id}/preview"));
        if authorized {
            builder = builder.header("authorization", "Bearer admin-secret");
        }
        crate::app(state)
            .oneshot(builder.body(Body::empty()).expect("preview request"))
            .await
            .expect("preview response")
    }

    #[tokio::test]
    async fn a_preview_for_an_unknown_device_is_a_not_found() {
        let (state, _device_id, _config_dir) = state_with_curated_plugins();

        let response = preview_request(state, "dev-not-minted", "aqi-card", true).await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_preview_before_any_runtime_exists_is_service_unavailable() {
        // A minted device that has never connected has no runtime to ask.
        // That is temporary and retryable, so it is 503 -- distinct from the
        // 404 an addressing mistake gets.
        let (state, device_id, _config_dir) = state_with_curated_plugins();

        let response = preview_request(state, &device_id, "aqi-card", true).await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let error = response_json(response).await;
        assert_eq!(error["kind"], "runtime");
    }

    #[tokio::test]
    async fn the_preview_route_requires_admin_authentication() {
        let (state, device_id, _config_dir) = state_with_curated_plugins();

        let response = preview_request(state, &device_id, "aqi-card", false).await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    /// Serves fixture data to every plugin card except `never-fetched`, which
    /// yields no value at all. The worker caches a plugin snapshot only when
    /// `value` is `Some` (`app-core/src/runtime.rs:2056-2073`), so that card
    /// is permanently pre-first-fetch with no timing assumption.
    ///
    /// Payload selection is keyed by plugin id, not one fixed body: `aqi`'s
    /// curated manifest binds an icon-font glyph node to `icons.ttf`, which
    /// -- like the committed `Inter-subset.ttf` it is byte-identical to --
    /// has an empty `name` table (verified directly: `fontdb::Database` finds
    /// zero faces in it, because `fontdb` requires a Typographic-Family or
    /// Family name record to register a face at all, and this font has
    /// none). The device draws it fine by codepoint with no name lookup, but
    /// the *server's* raster fallback -- exercised for the first time by this
    /// preview route -- references a scene's fonts by family name in
    /// generated SVG, which is categorically impossible for a font `fontdb`
    /// never indexed. `claude-limits` is the display-list exemplar here
    /// instead: a real curated plugin with no asset fonts at all, so its
    /// preview exercises the same real compile-and-rasterize path without
    /// tripping over that pre-existing gap. `svg-aqi` is unaffected -- its
    /// SVG template declares no font asset either -- so it keeps the
    /// AQI-shaped payload.
    struct FixtureRefresher {
        aqi_payload: serde_json::Value,
        claude_limits_payload: serde_json::Value,
    }

    impl app_core::ProviderRefresher for FixtureRefresher {
        fn refresh(
            &mut self,
            request: app_core::ProviderRefreshRequest,
        ) -> app_core::ProviderRefreshResult {
            let fetched = request.widget_id != "never-fetched";
            let payload = match &request.provider {
                app_core::ProviderRequest::Plugin { plugin_id } if plugin_id == "claude-limits" => {
                    self.claude_limits_payload.clone()
                }
                _ => self.aqi_payload.clone(),
            };
            app_core::ProviderRefreshResult {
                generation: request.generation,
                widget_id: request.widget_id,
                fields: vec![protocol::Field {
                    key: "title".into(),
                    value: protocol::FieldValue::Text(request.title),
                }],
                value: fetched.then_some(payload),
                refreshed_at: fetched.then(chrono::Utc::now),
                age: fetched.then_some(std::time::Duration::ZERO),
                stale: !fetched,
                error: (!fetched).then(|| "the first refresh has not landed".to_owned()),
            }
        }
    }

    fn claude_limits_payload() -> serde_json::Value {
        serde_json::json!({
            "updated": "2026-09-08T12:00:00Z",
            "plan": "Max 20x",
            "windows": [
                {
                    "name": "session",
                    "used_pct": 42,
                    "resets_at_label": "Wed 6:09 PM",
                    "resets_in_label": "3 hours 48 minutes"
                },
                {
                    "name": "weekly",
                    "used_pct": 61,
                    "resets_at_label": "Fri 12:00 AM",
                    "resets_in_label": "2 days"
                }
            ]
        })
    }

    fn preview_config() -> app_core::AppConfig {
        // `AppConfig::default()` already carries the `clock` card, which is
        // the built-in the preview route must refuse.
        let mut config = app_core::AppConfig::default();
        for (id, plugin_id) in [
            ("aqi-card", "claude-limits"),
            ("svg-card", "svg-aqi"),
            ("never-fetched", "aqi"),
            ("absent-plugin", "not-installed"),
            ("aqi-known-gap", "aqi"),
        ] {
            config.cards.push(app_core::CardSettings::Plugin {
                id: id.into(),
                title: "Air quality".into(),
                plugin_id: plugin_id.into(),
                tap_action: app_core::WidgetTapAction::None,
                refresh: app_core::RefreshPolicy::Interval { minutes: 15 },
                alert: app_core::CardAlert::None,
            });
            config.playlists[0].entries.push(app_core::PlaylistEntry {
                card_id: id.into(),
                dwell_seconds: None,
            });
        }
        config
    }

    fn state_with_preview_runtime() -> (crate::ServerState, String, tempfile::TempDir) {
        let (state, device_id, config_dir) = state_with_curated_plugins();
        let (device, connector) =
            crate::runtime_device::WebSocketRuntimeDevice::channel(device_id.clone());
        let runtime = Arc::new(
            app_core::RuntimeHandle::start_with_plugin_host(
                preview_config(),
                Box::new(device),
                Box::new(FixtureRefresher {
                    aqi_payload: payload_from_envelope(AQI_FIXTURE),
                    claude_limits_payload: claude_limits_payload(),
                }),
                app_core::RuntimeOptions::default(),
                Some(Box::new(crate::plugin_host::ServerPluginHost::new(
                    curated_registry(),
                ))),
            )
            .expect("the preview runtime starts"),
        );
        // No socket is ever attached, and the lease is released immediately.
        // A preview must be served by a *retained* runtime, which is exactly
        // what a device between links leaves behind: `is_live()` is false and
        // `runtime()` is still `Some`.
        let lease = state
            .claim_link(device_id.clone())
            .expect("claim the link slot");
        lease.link().set_runtime(runtime, connector);
        drop(lease);
        (state, device_id, config_dir)
    }

    async fn preview_json(
        state: &crate::ServerState,
        device_id: &str,
        card_id: &str,
    ) -> serde_json::Value {
        let response = preview_request(state.clone(), device_id, card_id, true).await;
        assert_eq!(response.status(), StatusCode::OK);
        response_json(response).await
    }

    /// Polls until the first provider result has reached the worker. The
    /// runtime refreshes providers on its own thread, so "waiting" is a real
    /// transient state here rather than an outcome.
    async fn preview_once_settled(
        state: &crate::ServerState,
        device_id: &str,
        card_id: &str,
    ) -> serde_json::Value {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let body = preview_json(state, device_id, card_id).await;
            if body["state"] != "waiting" {
                return body;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the first plugin refresh never reached the runtime"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    fn assert_preview_png(body: &serde_json::Value) {
        // §4.2's invariant: a frame and a message are mutually exclusive.
        assert!(
            body["message"].is_null(),
            "a rendered frame carried a message"
        );
        let png = base64::engine::general_purpose::STANDARD
            .decode(body["png_base64"].as_str().expect("preview frame"))
            .expect("the preview frame is base64");
        let pixmap = resvg::tiny_skia::Pixmap::decode_png(&png).expect("the preview PNG decodes");
        assert_eq!((pixmap.width(), pixmap.height()), (448, 368));
    }

    #[tokio::test]
    async fn a_display_list_plugin_card_previews_as_a_decodable_448x368_png() {
        let (state, device_id, _config_dir) = state_with_preview_runtime();

        let body = preview_once_settled(&state, &device_id, "aqi-card").await;

        assert_eq!(body["state"], "fresh");
        assert_preview_png(&body);
        state.shutdown();
    }

    #[tokio::test]
    async fn an_svg_template_plugin_card_previews_through_the_same_route() {
        // The SVG arm is the one that is exact by construction: this raster
        // *is* what the panel shows.
        let (state, device_id, _config_dir) = state_with_preview_runtime();

        let body = preview_once_settled(&state, &device_id, "svg-card").await;

        assert_eq!(body["state"], "fresh");
        assert_preview_png(&body);
        state.shutdown();
    }

    #[tokio::test]
    async fn a_card_with_no_cached_snapshot_is_waiting_with_a_message_and_no_frame() {
        // The normal pre-first-fetch state, kept distinguishable from a fault.
        let (state, device_id, _config_dir) = state_with_preview_runtime();

        let body = preview_json(&state, &device_id, "never-fetched").await;

        assert_eq!(body["state"], "waiting");
        assert!(body["png_base64"].is_null());
        assert!(
            body["message"].as_str().is_some_and(|m| !m.is_empty()),
            "a stage with no frame must have a word to print"
        );
        state.shutdown();
    }

    #[tokio::test]
    async fn a_plugin_absent_from_the_registry_is_an_error_naming_only_that_plugin() {
        let (state, device_id, _config_dir) = state_with_preview_runtime();

        let body = preview_once_settled(&state, &device_id, "absent-plugin").await;

        assert_eq!(body["state"], "error");
        assert!(body["png_base64"].is_null());
        assert!(
            body["message"]
                .as_str()
                .expect("an error state carries a message")
                .contains("not-installed"),
            "the message must name the plugin: {}",
            body["message"]
        );
        state.shutdown();
    }

    /// KNOWN GAP, marked the same way `scene_parity.rs`'s `known_gap` marks a
    /// case the model provably cannot reproduce yet: closing the gap fails
    /// this assertion and forces the marker's removal, so the gap cannot rot
    /// into invisible missing coverage the way an unexplained skip would.
    ///
    /// The curated `aqi` plugin's icon-font asset (`icons.ttf`, byte-identical
    /// to the committed `crates/lvgl-sim/assets/Inter-subset.ttf`) has a
    /// `name` table with zero name records (verified directly against the
    /// TTF: the table is a bare 6-byte header, `count = 0`). `fontdb` 0.23.0
    /// requires at least one Typographic-Family or Family name record to
    /// register a face at all (`fontdb-0.23.0/src/lib.rs`'s `parse_names`),
    /// so `Database::load_font_data` on these exact bytes registers zero
    /// faces. The device draws this font fine -- LVGL resolves glyphs by
    /// codepoint, no name lookup needed -- but the server's raster fallback
    /// (this preview route) references a scene's fonts by family name in
    /// generated SVG, which is categorically impossible for a font `fontdb`
    /// never indexed. Do NOT edit `plugins/aqi/icons.ttf` to fix this: its
    /// bytes are digest-addressed and already committed for the device's own
    /// wire path, and changing them moves asset digests and hardware goldens
    /// well outside this task. When the rasterizer is changed to handle a
    /// name-table-less font, THIS TEST WILL FAIL: delete it then, and add
    /// `aqi` back to the ordinary display-list coverage above instead of
    /// `claude-limits`.
    #[tokio::test]
    async fn aqi_preview_is_a_known_font_name_table_gap_delete_this_test_when_fixed() {
        let (state, device_id, _config_dir) = state_with_preview_runtime();

        let body = preview_once_settled(&state, &device_id, "aqi-known-gap").await;

        assert_eq!(body["state"], "error");
        assert!(body["png_base64"].is_null());
        assert!(
            body["message"].as_str().is_some_and(|message| message
                .contains("font family is not in the explicit rasterizer font database")),
            "expected the known icons.ttf name-table gap, got: {}",
            body["message"]
        );
        state.shutdown();
    }

    #[tokio::test]
    async fn a_built_in_card_is_not_previewable_here_and_is_a_not_found() {
        // Built-in cards preview in the Mac's own simulator; this route is
        // the server-rendered path only. `RuntimeError::NotAPluginCard`.
        let (state, device_id, _config_dir) = state_with_preview_runtime();

        let response = preview_request(state.clone(), &device_id, "clock", true).await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        state.shutdown();
    }

    #[tokio::test]
    async fn a_card_absent_from_the_configuration_is_a_not_found() {
        // `RuntimeError::UnknownCard`.
        let (state, device_id, _config_dir) = state_with_preview_runtime();

        let response = preview_request(state.clone(), &device_id, "no-such-card", true).await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        state.shutdown();
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
            // Naming a card that is not configured, or one that is not a
            // plugin card, is an addressing mistake by the caller -- the same
            // bare 404 an unknown device gets from `get_device`, not a runtime
            // fault with a body. The unit `NotFound` already exists and is
            // what `IntoResponse` turns into a bare `404`.
            RuntimeError::UnknownCard { .. } | RuntimeError::NotAPluginCard { .. } => {
                Self::NotFound
            }
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
