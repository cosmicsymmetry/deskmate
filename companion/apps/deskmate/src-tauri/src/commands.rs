// Tauri injects command state and app handles by value; these signatures are part of
// its command macro contract even when the handler only borrows them internally.
#![allow(clippy::needless_pass_by_value)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use app_core::{
    AdminConfigErrorBody, AppConfig, CardField, CardFieldValue, CardSettings, ConfigStore,
    DisplayOrientation, DisplayTemplate, MAX_CONFIG_FILE_BYTES, MAX_DEVICE_ID_LEN,
    MAX_DEVICE_TOKEN_LEN, MAX_PSK_LEN, MAX_SERVER_URL_LEN, MAX_SSID_LEN, MAX_WIDGET_ID_LEN,
    NetworkConfig, NetworkSettings, NetworkSettingsStore, NetworkSettingsStoreError,
    NetworkSettingsUpdate, PomodoroAction, ProvisioningTier, RuntimeError, RuntimeHandle,
    SaveReceipt, StoreError, ValidationIssue, utc_offset_minutes,
};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;

use crate::{DesktopSnapshot, DesktopState, NetworkedConfigProjection};

pub(crate) const MAX_SERVER_ERROR_BYTES: usize = 64 * 1_024;
const MAX_IMAGE_SOURCE_MINT_BYTES: usize = 16 * 1_024;

/// The device-status read's own bound, which must EXCEED
/// [`app_core::MAX_CONFIG_FILE_BYTES`] rather than match it: `GET
/// /v1/devices/{id}` embeds the device's whole `AppConfig`, itself legal right up
/// to that limit, and then wraps it in a snapshot of providers, card data and card
/// errors. Sharing the 64 KiB error-body bound made a near-maximal config a
/// permanent "oversized response" -- a card-state poll that could never succeed,
/// reported with the same notice an unreachable server gets. Expressed as a
/// multiple so the relationship survives someone editing either number.
pub(crate) const MAX_SERVER_DEVICE_STATUS_BYTES: usize = 8 * MAX_CONFIG_FILE_BYTES;

/// The draft travels as a bounded JSON envelope so an IPC caller cannot make serde
/// allocate an arbitrarily deep application document before domain validation runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftPayload {
    pub json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WidgetTarget {
    pub widget_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftValidation {
    pub valid: bool,
    pub issues: Vec<ValidationIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigApplyResult {
    pub save: SaveReceipt,
}

#[derive(Clone)]
struct ConfigSaveContext {
    runtime: Arc<RuntimeHandle>,
    store: Arc<ConfigStore>,
    network_store: Arc<NetworkSettingsStore>,
    networked_config: Arc<NetworkedConfigProjection>,
    has_saved_config: Arc<AtomicBool>,
    mutation_lock: Arc<Mutex<()>>,
}

impl ConfigSaveContext {
    fn from_desktop(state: &DesktopState) -> Self {
        Self {
            runtime: Arc::clone(&state.runtime),
            store: Arc::clone(&state.store),
            network_store: Arc::clone(&state.network_store),
            networked_config: Arc::clone(&state.networked_config),
            has_saved_config: Arc::clone(&state.has_saved_config),
            mutation_lock: Arc::clone(&state.mutation_lock),
        }
    }
}

#[derive(Clone)]
struct ProvisionContext {
    runtime: Arc<RuntimeHandle>,
    network_store: Arc<NetworkSettingsStore>,
    networked_config: Arc<NetworkedConfigProjection>,
}

impl ProvisionContext {
    fn from_desktop(state: &DesktopState) -> Self {
        Self {
            runtime: Arc::clone(&state.runtime),
            network_store: Arc::clone(&state.network_store),
            networked_config: Arc::clone(&state.networked_config),
        }
    }
}

/// Everything a read-only server call needs, cloned out of `DesktopState` so the
/// blocking half can move onto a worker thread. No config, no runtime, no locks:
/// these calls never mutate anything the desktop owns.
#[derive(Clone)]
struct ServerQueryContext {
    agent: ureq::Agent,
    network_store: Arc<NetworkSettingsStore>,
}

impl ServerQueryContext {
    fn from_desktop(state: &DesktopState) -> Self {
        Self {
            agent: state.server_client.clone(),
            network_store: Arc::clone(&state.network_store),
        }
    }

    /// The store lends the token for one call and never returns it; a missing token
    /// is a typed instruction, not a transport failure.
    fn with_admin_token<T>(
        &self,
        operation: impl FnOnce(&str) -> Result<T, IpcError>,
    ) -> Result<T, IpcError> {
        self.network_store
            .with_admin_token(operation)
            .map_err(IpcError::from)?
            .ok_or_else(|| IpcError::InvalidPayload {
                message: "Enter the admin token in Network setup before reading server state."
                    .into(),
            })?
    }
}

fn fetch_server_plugins(
    context: &ServerQueryContext,
) -> Result<app_core::admin::PluginCatalog, IpcError> {
    let settings = context.network_store.load().settings().clone();
    let url =
        crate::server_client::server_url(&settings.server_url, &["v1", "plugins"])?.to_string();
    context.with_admin_token(|token| {
        crate::server_client::get_server_json(&context.agent, &url, token, MAX_SERVER_ERROR_BYTES)
    })
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct MintImageSourceRequest<'a> {
    name: &'a str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerMintedImageSource {
    id: String,
    token: String,
}

/// The one serialization boundary for the plaintext source credential. This value
/// lives only long enough to cross IPC once; neither the Rust state nor the config
/// retains it.
#[derive(Serialize)]
pub struct MintedImageSource {
    source_id: String,
    token: String,
    push_url: String,
}

#[derive(Serialize)]
#[serde(untagged)]
pub enum PluginCatalogOrMintedImageSource {
    Catalog(app_core::admin::PluginCatalog),
    Minted(MintedImageSource),
}

fn mint_server_image_source(
    context: &ServerQueryContext,
    name: &str,
) -> Result<MintedImageSource, IpcError> {
    validate_target(
        name,
        app_core::config::MAX_IMAGE_SOURCE_NAME_LEN,
        "picture source name",
    )?;
    let settings = context.network_store.load().settings().clone();
    let url = crate::server_client::server_url(&settings.server_url, &["v1", "images"])?;
    let body =
        serde_json::to_vec(&MintImageSourceRequest { name }).map_err(|_| IpcError::Internal {
            message: "the picture source request could not be encoded".into(),
        })?;
    let minted: ServerMintedImageSource = context.with_admin_token(|token| {
        let mut response = context
            .agent
            .post(url.as_str())
            .header("Authorization", format!("Bearer {token}"))
            .content_type("application/json")
            .send(&body)
            .map_err(|_| IpcError::RuntimeUnavailable {
                message: "the configured server could not be reached".into(),
            })?;
        let status = response.status().as_u16();
        let response_body = response
            .body_mut()
            .with_config()
            .limit(MAX_IMAGE_SOURCE_MINT_BYTES.saturating_add(1) as u64)
            .read_to_vec()
            .map_err(|error| match error {
                ureq::Error::BodyExceedsLimit(_) => IpcError::RuntimeUnavailable {
                    message: "the server returned an oversized response".into(),
                },
                _ => IpcError::RuntimeUnavailable {
                    message: "the server returned an unreadable response".into(),
                },
            })?;
        if !(200..300).contains(&status) {
            return Err(server_failure(status, &response_body));
        }
        serde_json::from_slice(&response_body).map_err(|_| IpcError::IncompatibleServer {
            message: "the server returned a picture source this app could not read".into(),
        })
    })?;
    validate_target(&minted.id, MAX_WIDGET_ID_LEN, "picture source ID")?;
    validate_secret(&minted.token, MAX_DEVICE_TOKEN_LEN, "picture source token")?;
    let push_url =
        crate::server_client::server_url(&settings.server_url, &["v1", "images", &minted.token])?
            .to_string();
    Ok(MintedImageSource {
        source_id: minted.id,
        token: minted.token,
        push_url,
    })
}

fn fetch_server_card_state(context: &ServerQueryContext) -> Result<Vec<ServerCardState>, IpcError> {
    let settings = context.network_store.load().settings().clone();
    validate_target(&settings.device_id, MAX_DEVICE_ID_LEN, "device ID")?;
    let url = crate::server_client::server_url(
        &settings.server_url,
        &["v1", "devices", &settings.device_id],
    )?
    .to_string();
    let status: ServerDeviceStatus = context.with_admin_token(|token| {
        crate::server_client::get_server_json(
            &context.agent,
            &url,
            token,
            MAX_SERVER_DEVICE_STATUS_BYTES,
        )
    })?;
    Ok(project_server_card_state(&status))
}

/// The tier decides where a server-rendered card's face comes from, and it decides
/// first: in local tier there is no server host, so this returns the kind-specific
/// sentence without opening a socket. Only in networked tier is a request made,
/// and only then can a 404 be read as `SERVER_PREVIEW_UNAVAILABLE`.
fn server_card_preview(
    context: &ServerQueryContext,
    tier: Option<app_core::DeviceTier>,
    card_id: &str,
    local_state: &str,
) -> Result<PreviewFrame, IpcError> {
    validate_target(card_id, MAX_WIDGET_ID_LEN, "card ID")?;
    let settings = context.network_store.load().settings().clone();
    if save_destination(tier, &settings) != SaveDestination::Server {
        return Ok(unrendered_server_frame(local_state));
    }
    validate_target(&settings.device_id, MAX_DEVICE_ID_LEN, "device ID")?;
    let url = crate::server_client::server_url(
        &settings.server_url,
        &[
            "v1",
            "devices",
            &settings.device_id,
            "cards",
            card_id,
            "preview",
        ],
    )?
    .to_string();
    let response = context.with_admin_token(|token| {
        crate::server_client::get_server_json::<app_core::admin::CardPreviewResponse>(
            &context.agent,
            &url,
            token,
            crate::server_client::MAX_SERVER_PREVIEW_BYTES,
        )
    });
    match response {
        Ok(response) => Ok(server_preview_frame(response)),
        // Spec section 10: additive routes fail closed. What a 404 means beyond
        // "no preview came back" is not knowable here, so the sentence does not
        // guess -- see `SERVER_PREVIEW_UNAVAILABLE`.
        Err(IpcError::NotFound { .. }) => Ok(unrendered_server_frame(SERVER_PREVIEW_UNAVAILABLE)),
        Err(error) => Err(error),
    }
}

async fn on_server_worker<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, IpcError> + Send + 'static,
) -> Result<T, IpcError> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|_| IpcError::RuntimeUnavailable {
            message: "the server request worker stopped unexpectedly".into(),
        })?
}

#[tauri::command]
pub async fn get_server_plugins(
    state: State<'_, DesktopState>,
    source_name: Option<String>,
) -> Result<PluginCatalogOrMintedImageSource, IpcError> {
    let context = ServerQueryContext::from_desktop(&state);
    on_server_worker(move || match source_name {
        Some(name) => {
            mint_server_image_source(&context, &name).map(PluginCatalogOrMintedImageSource::Minted)
        }
        None => fetch_server_plugins(&context).map(PluginCatalogOrMintedImageSource::Catalog),
    })
    .await
}

#[tauri::command]
pub async fn get_server_card_state(
    state: State<'_, DesktopState>,
) -> Result<Vec<ServerCardState>, IpcError> {
    let context = ServerQueryContext::from_desktop(&state);
    let states = on_server_worker(move || fetch_server_card_state(&context)).await?;
    // The snapshot stream is what a tile actually reads, so a successful poll
    // updates the projection before it answers the caller.
    state.server_card_state.replace(states.clone())?;
    Ok(states)
}

struct ServerSaveContext {
    config: ConfigSaveContext,
    server_client: ureq::Agent,
}

/// Secret-bearing IPC inputs intentionally implement neither `Debug` nor `Serialize`.
/// Their only outbound projection is `NetworkSettings`, which contains public fields.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerEndpointRequest {
    server_url: String,
    device_id: String,
    admin_token: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvisionDeviceRequest {
    ssid: String,
    passphrase: String,
    server_url: String,
    device_id: String,
    device_token: String,
    tier: app_core::DeviceTier,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfigRequest {
    draft: DraftPayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutostartStatus {
    pub enabled: bool,
    pub preference_enabled: bool,
}

/// A rendered card preview. `png_base64` is `None` when there is nothing to draw
/// and `state` then names why, in the server's own words: the stage prints that
/// sentence rather than "Preview unavailable", which stays reserved for a real
/// transport failure. Built-in cards always set `png_base64` and never `state`.
///
/// `sample` is set when the card has never published data (the runtime holds no
/// `CardDataSnapshot` for it): the request still renders, with an empty field set,
/// so the image is the firmware's own unconfigured appearance for that template
/// rather than an invented placeholder. It therefore only ever accompanies a real
/// frame -- a card with no frame is not a sample of anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewFrame {
    pub png_base64: Option<String>,
    pub sample: bool,
    pub state: Option<String>,
}

/// The server's view of one server-rendered card, projected onto the Mac's snapshot
/// so its tile carries the same value, freshness and error copy a built-in does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerCardState {
    pub card_id: String,
    pub provider: app_core::ProviderState,
    pub hero: Option<String>,
    pub errors: Vec<app_core::CardError>,
}

/// Deliberately partial. `AdminSnapshot` injects `last_ota_error` and
/// `observed_age_seconds` into `device` after serialization, so parsing the whole
/// `AppSnapshot` back would couple this app to a shape only the server writes.
/// These four collections are all a server-rendered tile needs.
#[derive(Deserialize)]
pub(crate) struct ServerDeviceStatus {
    snapshot: Option<ServerRuntimeSnapshot>,
}

#[derive(Deserialize)]
struct ServerRuntimeSnapshot {
    config: ServerSnapshotConfig,
    providers: Vec<app_core::ProviderSnapshot>,
    card_data: Vec<app_core::CardDataSnapshot>,
    card_errors: Vec<app_core::CardError>,
}

#[derive(Deserialize)]
struct ServerSnapshotConfig {
    cards: Vec<CardSettings>,
}

fn project_server_card_state(status: &ServerDeviceStatus) -> Vec<ServerCardState> {
    let Some(snapshot) = status.snapshot.as_ref() else {
        return Vec::new();
    };
    snapshot
        .config
        .cards
        .iter()
        .filter(|card| {
            matches!(
                card,
                CardSettings::Plugin { .. } | CardSettings::Picture { .. }
            )
        })
        .map(|card| {
            let card_id = card.id();
            ServerCardState {
                card_id: card_id.to_owned(),
                provider: snapshot
                    .providers
                    .iter()
                    .find(|provider| provider.widget_id == card_id)
                    .map_or(app_core::ProviderState::Idle, |provider| {
                        provider.state.clone()
                    }),
                // The summary the manifest declares arrives as `hero`; a non-text
                // value is not a headline, so it is not shown as one.
                hero: snapshot
                    .card_data
                    .iter()
                    .find(|data| data.card_id == card_id)
                    .and_then(|data| data.fields.iter().find(|field| field.key == "hero"))
                    .and_then(|field| match &field.value {
                        CardFieldValue::Text { value } => Some(value.clone()),
                        CardFieldValue::Integer { .. } | CardFieldValue::Boolean { .. } => None,
                    }),
                errors: snapshot
                    .card_errors
                    .iter()
                    .filter(|error| error.card_id == card_id)
                    .cloned()
                    .collect(),
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "category", rename_all = "kebab-case")]
pub enum IpcError {
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
    RuntimeBusy {
        message: String,
    },
    RuntimeUnavailable {
        message: String,
    },
    /// The server was reached, answered, and said something this app cannot read.
    ///
    /// Distinct from `RuntimeUnavailable` because the difference is the whole
    /// message: "couldn't reach the server" is simply false here. The reachable
    /// cause is the documented server-then-Mac rollout -- a server built before
    /// this app answers an additive route without the keys this app's DTOs
    /// require -- so the window's sentence for it means "update the server",
    /// never "check your network".
    IncompatibleServer {
        message: String,
    },
    NotFound {
        message: String,
    },
    Device {
        message: String,
    },
    Provider {
        message: String,
    },
    Autostart {
        message: String,
    },
    Window {
        message: String,
    },
    Internal {
        message: String,
    },
    /// A typed, visible refusal for input this build genuinely cannot act on yet --
    /// mirrors runtime.rs's `CardErrorKind::SceneRefused` for the same reason: say so
    /// explicitly rather than pretending to render something, or miscategorizing the
    /// refusal as an unexpected internal error.
    Unsupported {
        message: String,
    },
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPayload { message }
            | Self::PayloadTooLarge { message, .. }
            | Self::Validation { message, .. }
            | Self::Persistence { message }
            | Self::RuntimeBusy { message }
            | Self::RuntimeUnavailable { message }
            | Self::IncompatibleServer { message }
            | Self::NotFound { message }
            | Self::Device { message }
            | Self::Provider { message }
            | Self::Autostart { message }
            | Self::Window { message }
            | Self::Internal { message }
            | Self::Unsupported { message } => formatter.write_str(message),
        }
    }
}

impl std::error::Error for IpcError {}

impl IpcError {
    pub(crate) const fn log_label(&self) -> &'static str {
        match self {
            Self::InvalidPayload { .. } => "invalid-payload",
            Self::PayloadTooLarge { .. } => "payload-too-large",
            Self::Validation { .. } => "validation",
            Self::Persistence { .. } => "persistence",
            Self::RuntimeBusy { .. } => "runtime-busy",
            Self::RuntimeUnavailable { .. } => "runtime-unavailable",
            Self::IncompatibleServer { .. } => "incompatible-server",
            Self::NotFound { .. } => "not-found",
            Self::Device { .. } => "device",
            Self::Provider { .. } => "provider",
            Self::Autostart { .. } => "autostart",
            Self::Window { .. } => "window",
            Self::Internal { .. } => "internal",
            Self::Unsupported { .. } => "unsupported",
        }
    }

    fn map_message(mut self, map: impl FnOnce(String) -> String) -> Self {
        let message = match &mut self {
            Self::InvalidPayload { message }
            | Self::PayloadTooLarge { message, .. }
            | Self::Validation { message, .. }
            | Self::Persistence { message }
            | Self::RuntimeBusy { message }
            | Self::RuntimeUnavailable { message }
            | Self::IncompatibleServer { message }
            | Self::NotFound { message }
            | Self::Device { message }
            | Self::Provider { message }
            | Self::Autostart { message }
            | Self::Window { message }
            | Self::Internal { message }
            | Self::Unsupported { message } => message,
        };
        *message = map(std::mem::take(message));
        self
    }
}

#[tauri::command]
pub fn get_app_snapshot(state: State<'_, DesktopState>) -> Result<DesktopSnapshot, IpcError> {
    let snapshot = state.runtime.snapshot().map_err(IpcError::from)?;
    Ok(state.project_snapshot(snapshot))
}

#[tauri::command]
pub fn validate_config_draft(
    state: State<'_, DesktopState>,
    draft: DraftPayload,
) -> Result<DraftValidation, IpcError> {
    // The connected device's capabilities are half of what Save checks, so draft
    // validation must see them too — see `validate_draft_for_device`.
    let snapshot = state.runtime.snapshot().map_err(IpcError::from)?;
    validate_draft_for_device(&draft, &snapshot.device)
}

/// Reports exactly the issues `save_and_apply` would, so the settings UI can never
/// call a draft valid that Save then rejects.
///
/// That means both halves of Save's preflight: `compile()` (a strict superset of
/// `validate()` — it also catches a card with no `wire_config()` lowering) AND the
/// connected device's capability check. Reporting only the first was the narrower
/// contract this exists to prevent: a draft using a template the connected firmware
/// cannot render read as valid right up until Save refused it, with an error attached
/// to no field. The revision value only matters for `revision == 0` rejection, so any
/// nonzero placeholder is fine for a draft that is never actually applied.
fn validate_draft_for_device(
    draft: &DraftPayload,
    device: &app_core::DeviceSnapshot,
) -> Result<DraftValidation, IpcError> {
    let config = parse_draft(draft)?;
    let issues = match config.compile(1) {
        Ok(compiled) => missing_capability_issues(device, compiled.required_capabilities),
        Err(error) => error.issues,
    };
    Ok(DraftValidation {
        valid: issues.is_empty(),
        issues,
    })
}

#[tauri::command]
pub fn save_apply_config(
    state: State<'_, DesktopState>,
    draft: DraftPayload,
) -> Result<ConfigApplyResult, IpcError> {
    let config = parse_valid_draft(&draft)?;
    save_and_apply(ConfigSaveContext::from_desktop(&state), config)
}

#[tauri::command]
pub fn get_network_settings(state: State<'_, DesktopState>) -> NetworkSettings {
    state.network_store.load().settings().clone()
}

#[tauri::command]
pub fn set_server_endpoint(
    state: State<'_, DesktopState>,
    request: ServerEndpointRequest,
) -> Result<NetworkSettings, IpcError> {
    set_server_endpoint_with_context(&state.network_store, request)
}

fn set_server_endpoint_with_context(
    network_store: &NetworkSettingsStore,
    request: ServerEndpointRequest,
) -> Result<NetworkSettings, IpcError> {
    validate_server_url(&request.server_url)?;
    validate_secret(&request.admin_token, 4_096, "admin token")?;
    let current = network_store.load().settings().clone();
    // A blank box means "leave the stored id alone", which keeps the pre-pairing
    // endpoint save working. Anything typed is authoritative: without this, a Mac
    // that knows the tier but not the id has no path to the id at all, since pairing
    // demands a device token the server retains only as a digest.
    let device_id = if request.device_id.trim().is_empty() {
        current.device_id
    } else {
        validate_target(&request.device_id, MAX_DEVICE_ID_LEN, "device ID")?;
        request.device_id
    };
    let settings = NetworkSettings {
        server_url: request.server_url,
        device_id,
        tier: current.tier,
    };
    network_store
        .save(NetworkSettingsUpdate::new(
            settings.server_url.clone(),
            settings.device_id.clone(),
            None,
            Some(request.admin_token),
        ))
        .map_err(IpcError::from)?;
    Ok(settings)
}

#[tauri::command]
pub fn provision_device(
    state: State<'_, DesktopState>,
    request: ProvisionDeviceRequest,
) -> Result<NetworkSettings, IpcError> {
    provision_device_with_context(ProvisionContext::from_desktop(&state), request)
}

fn provision_device_with_context(
    context: ProvisionContext,
    request: ProvisionDeviceRequest,
) -> Result<NetworkSettings, IpcError> {
    validate_network_config_request(&request)?;
    let snapshot = context.runtime.snapshot().map_err(IpcError::from)?;
    let utc_offset_minutes =
        utc_offset_minutes(&snapshot.config.preferences.timezone, chrono::Utc::now()).map_err(
            |message| IpcError::Validation {
                message,
                issues: vec![ValidationIssue {
                    path: "preferences.timezone".into(),
                    code: app_core::ValidationCode::InvalidTimezone,
                    message: "Choose a valid display timezone before provisioning.".into(),
                }],
            },
        )?;
    let tier = match request.tier {
        app_core::DeviceTier::Local => ProvisioningTier::Local,
        app_core::DeviceTier::Networked => ProvisioningTier::Networked,
    };
    let public_settings = NetworkSettings {
        server_url: request.server_url.clone(),
        device_id: request.device_id.clone(),
        tier: Some(request.tier),
    };
    let config = NetworkConfig {
        ssid: request.ssid,
        psk: request.passphrase,
        server_url: if matches!(tier, ProvisioningTier::Networked) {
            device_link_url(&request.server_url)?
        } else {
            request.server_url
        },
        device_id: request.device_id,
        token: request.device_token.clone(),
        utc_offset_minutes,
        tier,
    };

    // Ownership changes only after the runtime-owned cable session confirms that the
    // device accepted provisioning. The device token has no desktop consumer after
    // this dispatch, so it is never retained on disk.
    context.runtime.provision(config).map_err(IpcError::from)?;
    context
        .network_store
        .save(NetworkSettingsUpdate::new(
            public_settings.server_url.clone(),
            public_settings.device_id.clone(),
            public_settings.tier,
            None,
        ))
        .map_err(IpcError::from)?;
    if matches!(tier, ProvisioningTier::Local) {
        context.networked_config.replace(None)?;
    }
    Ok(public_settings)
}

#[tauri::command]
pub fn factory_reset_device(state: State<'_, DesktopState>) -> Result<(), IpcError> {
    state.runtime.factory_reset().map_err(IpcError::from)?;
    let current = state.network_store.load().settings().clone();
    state
        .network_store
        .save(NetworkSettingsUpdate::new(
            current.server_url,
            current.device_id,
            Some(app_core::DeviceTier::Local),
            None,
        ))
        .map_err(IpcError::from)?;
    state.set_networked_config(None)
}

/// Explicit recovery for an endpoint saved before a device was successfully paired.
/// This changes only the Mac's routing decision; a later live Networked status still
/// wins and prevents the app from challenging server ownership over USB.
#[tauri::command]
pub fn use_local_ownership(state: State<'_, DesktopState>) -> Result<NetworkSettings, IpcError> {
    use_local_ownership_with_context(&state.network_store, &state.networked_config)
}

fn use_local_ownership_with_context(
    network_store: &NetworkSettingsStore,
    networked_config: &NetworkedConfigProjection,
) -> Result<NetworkSettings, IpcError> {
    let current = network_store.load().settings().clone();
    let settings = NetworkSettings {
        server_url: current.server_url,
        device_id: current.device_id,
        tier: Some(app_core::DeviceTier::Local),
    };
    network_store
        .save(NetworkSettingsUpdate::new(
            settings.server_url.clone(),
            settings.device_id.clone(),
            settings.tier,
            None,
        ))
        .map_err(IpcError::from)?;
    networked_config.replace(None)?;
    Ok(settings)
}

#[tauri::command]
pub async fn save_server_config(
    state: State<'_, DesktopState>,
    request: ServerConfigRequest,
) -> Result<ConfigApplyResult, IpcError> {
    let config = parse_valid_draft(&request.draft)?;
    let context = ServerSaveContext {
        config: ConfigSaveContext::from_desktop(&state),
        server_client: state.server_client.clone(),
    };
    tauri::async_runtime::spawn_blocking(move || save_server_config_blocking(context, config))
        .await
        .map_err(|_| IpcError::RuntimeUnavailable {
            message: "the server request worker stopped unexpectedly".into(),
        })?
}

fn save_server_config_blocking(
    context: ServerSaveContext,
    config: AppConfig,
) -> Result<ConfigApplyResult, IpcError> {
    let prepared = prepare_server_save(&context, config)?;

    // Network I/O must never hold `mutation_lock`: sync Tauri commands and tray
    // handlers use that same lock on the main thread. The local authoring mirror was
    // linearized above; a remote failure therefore reports both destinations honestly.
    let response = context
        .config
        .network_store
        .with_admin_token(|admin_token| {
            put_server_config(
                &context.server_client,
                &prepared.url,
                admin_token,
                &prepared.body,
            )
        })
        .map_err(IpcError::from)
        .and_then(|response| {
            response.ok_or_else(|| IpcError::InvalidPayload {
                message: "Enter the admin token in Network setup before saving to the server."
                    .into(),
            })
        })
        .and_then(|response| response)
        .map_err(server_error_after_local_save)?;
    let (status, response_body) = response;
    if !(200..300).contains(&status) {
        return Err(server_error_after_local_save(server_failure(
            status,
            &response_body,
        )));
    }

    Ok(ConfigApplyResult {
        save: prepared.save,
    })
}

struct PreparedServerSave {
    url: String,
    body: Vec<u8>,
    save: SaveReceipt,
}

fn prepare_server_save(
    context: &ServerSaveContext,
    mut config: AppConfig,
) -> Result<PreparedServerSave, IpcError> {
    let _mutation = context
        .config
        .mutation_lock
        .lock()
        .map_err(|_| IpcError::Internal {
            message: "desktop mutation lock is unavailable".into(),
        })?;
    let snapshot = context.config.runtime.snapshot().map_err(IpcError::from)?;
    merge_command_owned_preferences(&mut config, &snapshot.config);
    let compiled = config.compile(1).map_err(|error| IpcError::Validation {
        message: "configuration requires device features not implemented by this build".into(),
        issues: error.issues,
    })?;
    ensure_device_compatibility(&snapshot.device, compiled.required_capabilities)?;

    let settings = context.config.network_store.load().settings().clone();
    if save_destination(snapshot.device.tier, &settings) != SaveDestination::Server {
        return Err(IpcError::InvalidPayload {
            message: "display ownership is not known to be networked; connect it over USB to confirm ownership before saving".into(),
        });
    }
    let url = server_config_url(&settings.server_url, &settings.device_id)?.to_string();
    let body = serde_json::to_vec(&config).map_err(|_| IpcError::Internal {
        message: "configuration could not be serialized for the server".into(),
    })?;
    context
        .config
        .network_store
        .with_admin_token(|admin_token| validate_secret(admin_token, 4_096, "admin token"))
        .map_err(IpcError::from)?
        .ok_or_else(|| IpcError::InvalidPayload {
            message: "Enter the admin token in Network setup before saving to the server.".into(),
        })??;

    context
        .config
        .network_store
        .save(NetworkSettingsUpdate::new(
            settings.server_url,
            settings.device_id,
            Some(app_core::DeviceTier::Networked),
            None,
        ))
        .map_err(IpcError::from)?;

    // The server is authoritative in networked tier. Keep a local authoring mirror,
    // but deliberately do not call RuntimeHandle::apply_config: that would put a
    // config write onto the restricted USB link and challenge the server's ownership.
    let save = persist_config_parts(
        &context.config.runtime,
        &context.config.store,
        &context.config.has_saved_config,
        &config,
    )?;
    context.config.networked_config.replace(Some(config))?;
    Ok(PreparedServerSave { url, body, save })
}

fn server_error_after_local_save(error: IpcError) -> IpcError {
    error.map_message(|message| {
        format!(
            "The draft was saved on this Mac, but the server destination did not succeed: {message}"
        )
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SaveDestination {
    Local,
    Server,
}

fn save_destination(
    tier: Option<app_core::DeviceTier>,
    settings: &NetworkSettings,
) -> SaveDestination {
    match resolved_device_tier(tier, settings) {
        app_core::DeviceTier::Networked => SaveDestination::Server,
        app_core::DeviceTier::Local => SaveDestination::Local,
    }
}

/// The app's single ownership resolution: a live tier read over the cable wins,
/// otherwise the persisted tier decides, and a legacy server identity with no
/// recorded tier is conservatively read as networked so an unplugged save never
/// reaches for USB.
///
/// `resolveDeviceTier` in `useAppState.ts` is its TypeScript twin and must keep
/// answering the same way. Everything that depends on ownership -- where a save
/// goes, whether a preview asks the server, and whether the snapshot projector
/// overlays the server's plugin card state -- reads this one answer, because two
/// resolutions disagreeing is exactly the defect the projector had: the ordinary
/// networked case (cable out, so `device.tier` is `None`) routed saves to the
/// server while the projector treated the display as possibly local.
pub(crate) fn resolved_device_tier(
    tier: Option<app_core::DeviceTier>,
    settings: &NetworkSettings,
) -> app_core::DeviceTier {
    if let Some(tier) = tier {
        return tier;
    }
    if let Some(tier) = settings.tier {
        return tier;
    }
    if settings.server_url.is_empty() && settings.device_id.is_empty() {
        app_core::DeviceTier::Local
    } else {
        app_core::DeviceTier::Networked
    }
}

#[tauri::command]
pub fn resume_pushing(state: State<'_, DesktopState>) -> Result<(), IpcError> {
    set_paused(&state, false)
}

#[tauri::command]
pub fn control_pomodoro(
    state: State<'_, DesktopState>,
    target: WidgetTarget,
    action: PomodoroAction,
) -> Result<(), IpcError> {
    validate_target(&target.widget_id, MAX_WIDGET_ID_LEN, "widget ID")?;
    state
        .runtime
        .control_pomodoro(target.widget_id, action)
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn refresh_provider(
    state: State<'_, DesktopState>,
    target: WidgetTarget,
) -> Result<(), IpcError> {
    validate_target(&target.widget_id, MAX_WIDGET_ID_LEN, "widget ID")?;
    state
        .runtime
        .refresh_provider(target.widget_id)
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn get_autostart_status(
    app: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<AutostartStatus, IpcError> {
    autostart_status(&app, &state)
}

#[tauri::command]
pub fn set_autostart_enabled(
    app: AppHandle,
    state: State<'_, DesktopState>,
    enabled: bool,
) -> Result<AutostartStatus, IpcError> {
    set_autostart(&app, &state, enabled)
}

/// Renders one card exactly as the firmware's own template would: the same
/// `SimTemplate` `wire_config` would compile it to, the same last-good field values
/// the device holds (`AppSnapshot.card_data`), the same timezone offset the device's
/// clock is synced to, and the currently configured mounting orientation.
///
/// A card with no published data yet (no `CardDataSnapshot` entry, or one with an
/// empty field set) renders with an empty field vector instead of inventing sample
/// text: the firmware's own per-template defaults show through, which is the
/// device's actual unconfigured appearance. `sample` tells the caller this happened
/// so the settings UI can badge it, without the renderer itself lying about what it
/// drew.
///
/// Plugin and picture cards are the two kinds this simulator cannot draw: neither has
/// a `DisplayTemplate`. One comes from a manifest-compiled scene and the other from
/// a pushed frame; the server rasterizes both. Those previews are fetched rather than
/// rendered, which is why this command is `async` -- the fetch runs on a blocking worker,
/// never on the thread that would otherwise stall the settings window for the agent's timeout.
#[tauri::command]
pub async fn render_card_preview(
    state: State<'_, DesktopState>,
    card_id: String,
) -> Result<PreviewFrame, IpcError> {
    validate_target(&card_id, MAX_WIDGET_ID_LEN, "card ID")?;
    let snapshot = state.runtime.snapshot().map_err(IpcError::from)?;
    let card = snapshot
        .config
        .cards
        .iter()
        .find(|card| card.id() == card_id)
        .ok_or_else(|| IpcError::NotFound {
            message: format!("no card with id {card_id:?}"),
        })?;

    let server_rendered_state = match card {
        CardSettings::Clock { .. } | CardSettings::Pomodoro { .. } => None,
        CardSettings::Plugin { .. } => Some(PLUGIN_RENDERS_ON_THE_SERVER),
        CardSettings::Picture { .. } => Some(PICTURE_RENDERS_ON_THE_SERVER),
    };
    if let Some(local_state) = server_rendered_state {
        let context = ServerQueryContext::from_desktop(&state);
        let tier = snapshot.device.tier;
        let requested = card_id.clone();
        return on_server_worker(move || {
            server_card_preview(&context, tier, &requested, local_state)
        })
        .await;
    }

    let template = preview_template_for(card, &card_id)?;

    let data = snapshot
        .card_data
        .iter()
        .find(|data| data.card_id == card_id);
    let (fields, sample) = match data {
        Some(data) if !data.fields.is_empty() => {
            (data.fields.iter().map(sim_field).collect(), false)
        }
        _ => (Vec::new(), true),
    };

    let now = chrono::Utc::now();
    let utc_offset_minutes = utc_offset_minutes(&snapshot.config.preferences.timezone, now)
        .map_err(|message| IpcError::Internal { message })?;
    let orientation = preview_orientation(snapshot.config.preferences.orientation);

    let request = lvgl_sim::RenderRequest {
        template,
        fields,
        utc_offset_minutes,
        now_unix_seconds: now.timestamp(),
        orientation,
    };
    let png = state
        .preview
        .render(request)
        .map_err(|message| IpcError::Internal { message })?;
    Ok(PreviewFrame {
        png_base64: Some(BASE64_STANDARD.encode(png)),
        sample,
        state: None,
    })
}

/// The orientation the settings-window preview renders at.
///
/// Deliberately NOT the configured mounting. `LandscapeFlipped` is the 180 degree
/// mount, and the simulator reproduces it by reversing the finished frame
/// index-by-index (`lvgl-sim/csrc/sim_shim.c`'s `copy_frame_out`) exactly as the
/// firmware's `LV_DISPLAY_ROTATION_270` does. On the panel that flip is cancelled by
/// the physical mounting, so a person always sees an upright face; rendered into a
/// window that is not itself upside down, it is just upside down. The preview's job is
/// to show what the person will see, so it renders upright for both mountings.
///
/// Because the flip is a pure 180 degree rotation of identical content, nothing is
/// lost by this: the upright frame IS the flipped frame, read the way the viewer
/// reads it.
fn preview_orientation(configured: DisplayOrientation) -> lvgl_sim::SimOrientation {
    match configured {
        DisplayOrientation::Landscape | DisplayOrientation::LandscapeFlipped => {
            lvgl_sim::SimOrientation::Landscape
        }
    }
}

/// Mirrors the wire mapping `CardSettings::wire_config` uses for `TemplateKind`
/// (`app-core`'s `config.rs`), except targeting `lvgl_sim::SimTemplate` — the two
/// enums are exhaustively 1:1, so this can never fail to map a `DisplayTemplate` the
/// rest of the app accepts; there is no "unknown template" branch to fall back from.
/// Resolves the preview simulator's `SimTemplate` for one card, or a typed refusal.
///
/// A plugin or picture card has no `DisplayTemplate`: it renders on the server,
/// not from any of the six built-in templates this preview simulator knows how to
/// draw. `render_card_preview` sends both kinds to `server_card_preview` before
/// reaching here, so this arm is a GUARD, not a path
/// -- kept, and typed, so a future caller that forgets that routing is refused
/// visibly (mirroring `runtime.rs`'s `SceneRefused` handling for the same absence)
/// instead of silently drawing a plugin card as some unrelated built-in face.
fn preview_template_for(
    card: &CardSettings,
    card_id: &str,
) -> Result<lvgl_sim::SimTemplate, IpcError> {
    let Some(template) = card.template() else {
        return Err(IpcError::Unsupported {
            message: format!(
                "card {card_id:?} is server-rendered; its preview is not drawn by this simulator"
            ),
        });
    };
    Ok(sim_template(template))
}

fn sim_template(template: &DisplayTemplate) -> lvgl_sim::SimTemplate {
    match template {
        DisplayTemplate::DigitalClock => lvgl_sim::SimTemplate::DigitalClock,
        DisplayTemplate::ProgressRing => lvgl_sim::SimTemplate::ProgressRing,
        DisplayTemplate::RowList => lvgl_sim::SimTemplate::RowList,
        DisplayTemplate::AnalogClock => lvgl_sim::SimTemplate::AnalogClock,
        DisplayTemplate::BigNumberLabel => lvgl_sim::SimTemplate::BigNumberLabel,
        DisplayTemplate::IconBadgeText { .. } => lvgl_sim::SimTemplate::IconBadgeText,
    }
}

fn sim_field(field: &CardField) -> lvgl_sim::SimField {
    let value = match &field.value {
        CardFieldValue::Text { value } => lvgl_sim::SimFieldValue::Text(value.clone()),
        CardFieldValue::Integer { value } => lvgl_sim::SimFieldValue::Integer(*value),
        CardFieldValue::Boolean { value } => lvgl_sim::SimFieldValue::Boolean(*value),
    };
    lvgl_sim::SimField {
        name: field.key.clone(),
        value,
    }
}

/// Chosen from the tier the Mac already knows, before any socket is opened: in
/// local tier there is no server to render on and this app has no plugin host.
pub(crate) const PLUGIN_RENDERS_ON_THE_SERVER: &str = "Plugin cards render on the server";

/// The picture-specific local-tier sentence. Reusing the plugin sentence would make
/// the stage call this card something it is not.
pub(crate) const PICTURE_RENDERS_ON_THE_SERVER: &str = "Picture cards render on the server";

/// Chosen only after a networked-tier request came back 404.
///
/// Four different things answer 404 on this route -- a server built before it
/// exists, an unknown device, an unknown card, and a card that is not a plugin
/// card -- and the status code carries nothing that tells them apart. So the
/// sentence states what was observed and asserts no cause. It said "Plugin
/// previews need a newer server" until the whole-branch review, which was a
/// guess three quarters of the time.
pub(crate) const SERVER_PREVIEW_UNAVAILABLE: &str = "The server has no preview for this card";

fn unrendered_server_frame(state: &str) -> PreviewFrame {
    PreviewFrame {
        png_base64: None,
        sample: false,
        state: Some(state.to_owned()),
    }
}

/// Section 4.2 promises `png_base64` exactly when the state is `fresh` or `stale`
/// and `message` exactly when it is `error` or `waiting`. This keys off the frame
/// rather than the word, so a server that breaks that invariant still produces a
/// stage that is either an image or a sentence, never a blank black rectangle.
fn server_preview_frame(response: app_core::admin::CardPreviewResponse) -> PreviewFrame {
    match response.png_base64 {
        Some(png_base64) => PreviewFrame {
            png_base64: Some(png_base64),
            sample: false,
            state: None,
        },
        None => PreviewFrame {
            png_base64: None,
            sample: false,
            state: Some(
                response
                    .message
                    .unwrap_or_else(|| "The server sent no preview for this card".to_owned()),
            ),
        },
    }
}

pub(crate) fn set_paused(state: &DesktopState, paused: bool) -> Result<(), IpcError> {
    let _mutation = state.mutation_lock.lock().map_err(|_| IpcError::Internal {
        message: "desktop mutation lock is unavailable".into(),
    })?;
    let mut config = state.runtime.snapshot().map_err(IpcError::from)?.config;
    if config.preferences.paused == paused {
        return Ok(());
    }
    config.preferences.paused = paused;
    persist_config(state, &config)?;
    // If the worker cannot accept this change, the valid saved preference remains
    // authoritative on the next app start rather than being silently discarded.
    state.runtime.set_paused(paused).map_err(IpcError::from)
}

pub(crate) fn set_autostart(
    app: &AppHandle,
    state: &DesktopState,
    enabled: bool,
) -> Result<AutostartStatus, IpcError> {
    let _mutation = state.mutation_lock.lock().map_err(|_| IpcError::Internal {
        message: "desktop mutation lock is unavailable".into(),
    })?;
    let manager = app.autolaunch();
    let was_enabled = manager.is_enabled().map_err(autostart_error)?;
    let changed_os = was_enabled != enabled;
    if changed_os {
        if enabled {
            manager.enable().map_err(autostart_error)?;
        } else {
            manager.disable().map_err(autostart_error)?;
        }
    }

    let mut config = state.runtime.snapshot().map_err(IpcError::from)?.config;
    config.preferences.autostart = enabled;
    if let Err(error) = persist_config(state, &config) {
        if changed_os {
            if was_enabled {
                let _ = manager.enable();
            } else {
                let _ = manager.disable();
            }
        }
        return Err(error);
    }

    // This preference has no layout or provider effect, so update it without replacing
    // live pomodoro/provider state.
    state
        .runtime
        .set_autostart_preference(enabled)
        .map_err(IpcError::from)?;
    state
        .tray
        .autostart
        .set_checked(enabled)
        .map_err(|error| IpcError::Window {
            message: format!("cannot update settings window: {error}"),
        })?;
    Ok(AutostartStatus {
        enabled,
        preference_enabled: enabled,
    })
}

fn save_and_apply(
    context: ConfigSaveContext,
    mut config: AppConfig,
) -> Result<ConfigApplyResult, IpcError> {
    let _mutation = context
        .mutation_lock
        .lock()
        .map_err(|_| IpcError::Internal {
            message: "desktop mutation lock is unavailable".into(),
        })?;
    let snapshot = context.runtime.snapshot().map_err(IpcError::from)?;
    let settings = context.network_store.load().settings().clone();
    if save_destination(snapshot.device.tier, &settings) != SaveDestination::Local {
        return Err(IpcError::InvalidPayload {
            message: local_save_refusal(&settings).into(),
        });
    }
    merge_command_owned_preferences(&mut config, &snapshot.config);
    let compiled = config.compile(1).map_err(|error| IpcError::Validation {
        message: "configuration requires device features not implemented by this build".into(),
        issues: error.issues,
    })?;
    ensure_device_compatibility(&snapshot.device, compiled.required_capabilities)?;
    let save = persist_config_parts(
        &context.runtime,
        &context.store,
        &context.has_saved_config,
        &config,
    )?;
    context.networked_config.clear_for_local_save()?;
    // Keep a valid saved draft queued even when immediate device application reports
    // a transport failure. RuntimeHandle has already replaced its replay set then.
    context
        .runtime
        .apply_config(config)
        .map_err(IpcError::from)?;
    Ok(ConfigApplyResult { save })
}

fn local_save_refusal(settings: &NetworkSettings) -> &'static str {
    if !settings.server_url.is_empty() && settings.device_id.is_empty() {
        "a server endpoint is configured, but no device is paired; enter a device ID or connect over USB to confirm ownership"
    } else {
        "the display is owned by the server; save this configuration to the server instead"
    }
}

fn ensure_device_compatibility(
    device: &app_core::DeviceSnapshot,
    required_capabilities: u64,
) -> Result<(), IpcError> {
    let issues = missing_capability_issues(device, required_capabilities);
    if issues.is_empty() {
        return Ok(());
    }
    Err(IpcError::Validation {
        message: "the connected firmware cannot apply this configuration".into(),
        issues,
    })
}

/// The capability issues a configuration would raise against `device`, or an empty
/// list when it is compatible (or when no device is online to check against — an
/// offline draft is saved for the next connection and gated then).
fn missing_capability_issues(
    device: &app_core::DeviceSnapshot,
    required_capabilities: u64,
) -> Vec<ValidationIssue> {
    if !matches!(device.connection, app_core::ConnectionState::Online)
        || device.protocol_version.is_none()
    {
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

/// Names capabilities the way the person reading the settings window needs them. The
/// raw bitmask this used to print ("missing capability bits 0x0000000000000008") named
/// nothing the user could act on.
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

fn persist_config(state: &DesktopState, config: &AppConfig) -> Result<SaveReceipt, IpcError> {
    persist_config_parts(
        &state.runtime,
        &state.store,
        &state.has_saved_config,
        config,
    )
}

fn persist_config_parts(
    runtime: &RuntimeHandle,
    store: &ConfigStore,
    has_saved_config: &AtomicBool,
    config: &AppConfig,
) -> Result<SaveReceipt, IpcError> {
    runtime
        .set_persistence_state(app_core::PersistenceState::Saving)
        .map_err(IpcError::from)?;
    match store.save(config) {
        Ok(receipt) => {
            has_saved_config.store(true, Ordering::Release);
            runtime
                .set_persistence_state(app_core::PersistenceState::Clean)
                .map_err(IpcError::from)?;
            Ok(receipt)
        }
        Err(error) => {
            let message = error.to_string();
            let _ = runtime
                .set_persistence_state(app_core::PersistenceState::RecoverableError { message });
            Err(IpcError::from(error))
        }
    }
}

fn merge_command_owned_preferences(draft: &mut AppConfig, current: &AppConfig) {
    // Pause and start-at-login have dedicated commands with OS/runtime side effects.
    // Preserve their newest values when a settings draft was opened before a tray or
    // IPC toggle, while still allowing timezone edits through save/apply.
    draft.preferences.paused = current.preferences.paused;
    draft.preferences.autostart = current.preferences.autostart;
}

fn autostart_status(app: &AppHandle, state: &DesktopState) -> Result<AutostartStatus, IpcError> {
    let enabled = app.autolaunch().is_enabled().map_err(autostart_error)?;
    let preference_enabled = state
        .runtime
        .snapshot()
        .map_err(IpcError::from)?
        .config
        .preferences
        .autostart;
    Ok(AutostartStatus {
        enabled,
        preference_enabled,
    })
}

fn parse_valid_draft(draft: &DraftPayload) -> Result<AppConfig, IpcError> {
    let config = parse_draft(draft)?;
    config.validate().map_err(|error| IpcError::Validation {
        message: error.to_string(),
        issues: error.issues,
    })?;
    Ok(config)
}

fn parse_draft(draft: &DraftPayload) -> Result<AppConfig, IpcError> {
    let length = draft.json.len();
    if length > MAX_CONFIG_FILE_BYTES {
        return Err(IpcError::PayloadTooLarge {
            message: format!(
                "configuration draft exceeds the {MAX_CONFIG_FILE_BYTES}-byte IPC limit"
            ),
            maximum_bytes: MAX_CONFIG_FILE_BYTES,
        });
    }
    serde_json::from_str(&draft.json).map_err(|error| IpcError::InvalidPayload {
        message: format!("configuration draft is not valid: {error}"),
    })
}

fn validate_network_config_request(request: &ProvisionDeviceRequest) -> Result<(), IpcError> {
    validate_bounded(&request.ssid, MAX_SSID_LEN, "WiFi network")?;
    validate_bounded(&request.passphrase, MAX_PSK_LEN, "WiFi passphrase")?;
    validate_bounded(&request.server_url, MAX_SERVER_URL_LEN, "server URL")?;
    validate_bounded(&request.device_id, MAX_DEVICE_ID_LEN, "device ID")?;
    validate_bounded(&request.device_token, MAX_DEVICE_TOKEN_LEN, "device token")?;
    if matches!(request.tier, app_core::DeviceTier::Networked) {
        validate_target(&request.ssid, MAX_SSID_LEN, "WiFi network")?;
        validate_server_url(&request.server_url)?;
        validate_target(&request.device_id, MAX_DEVICE_ID_LEN, "device ID")?;
        validate_secret(&request.device_token, MAX_DEVICE_TOKEN_LEN, "device token")?;
    }
    Ok(())
}

fn validate_bounded(value: &str, maximum: usize, label: &str) -> Result<(), IpcError> {
    if value.len() > maximum {
        return Err(IpcError::InvalidPayload {
            message: format!("{label} must be at most {maximum} UTF-8 bytes"),
        });
    }
    Ok(())
}

fn validate_secret(value: &str, maximum: usize, label: &str) -> Result<(), IpcError> {
    if value.is_empty() {
        return Err(IpcError::InvalidPayload {
            message: format!("{label} must not be empty"),
        });
    }
    if value.contains(['\r', '\n']) {
        return Err(IpcError::InvalidPayload {
            message: format!("{label} contains an invalid line break"),
        });
    }
    validate_bounded(value, maximum, label)
}

pub(crate) fn validate_server_url(value: &str) -> Result<url::Url, IpcError> {
    validate_target(value, MAX_SERVER_URL_LEN, "server URL")?;
    let url = url::Url::parse(value).map_err(|_| IpcError::InvalidPayload {
        message: "server URL must be an absolute HTTP or HTTPS URL".into(),
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(IpcError::InvalidPayload {
            message: "server URL must be an absolute HTTP or HTTPS URL without credentials".into(),
        });
    }
    Ok(url)
}

/// Converts the operator-facing HTTPS base into the device's fixed WebSocket link.
/// Firmware derives HTTPS OTA routes back from the WSS authority, so paths, queries,
/// and fragments on the admin base deliberately do not cross onto the device URL.
fn device_link_url(server_url: &str) -> Result<String, IpcError> {
    let mut url = validate_server_url(server_url)?;
    if url.scheme() != "https" {
        return Err(IpcError::InvalidPayload {
            message: "server base URL must use HTTPS for secure device pairing".into(),
        });
    }
    url.set_scheme("wss")
        .map_err(|()| IpcError::InvalidPayload {
            message: "server base URL cannot be converted to a secure device link".into(),
        })?;
    url.set_path("/v1/device/link");
    url.set_query(None);
    url.set_fragment(None);
    let value = url.to_string();
    validate_bounded(&value, MAX_SERVER_URL_LEN, "derived device link URL")?;
    Ok(value)
}

fn server_config_url(server_url: &str, device_id: &str) -> Result<url::Url, IpcError> {
    validate_target(device_id, MAX_DEVICE_ID_LEN, "device ID")?;
    crate::server_client::server_url(server_url, &["v1", "devices", device_id, "config"])
}

fn put_server_config(
    agent: &ureq::Agent,
    url: &str,
    admin_token: &str,
    body: &[u8],
) -> Result<(u16, Vec<u8>), IpcError> {
    let mut response = agent
        .put(url)
        .header("Authorization", format!("Bearer {admin_token}"))
        .content_type("application/json")
        .send(body)
        .map_err(|_| IpcError::RuntimeUnavailable {
            message: "the configured server could not be reached".into(),
        })?;
    let status = response.status().as_u16();
    let response_body = if status == 422 {
        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_SERVER_ERROR_BYTES.saturating_add(1) as u64)
            .read_to_vec()
            .map_err(|_| IpcError::RuntimeUnavailable {
                message: "the server returned an unreadable error response".into(),
            })?;
        if body.len() > MAX_SERVER_ERROR_BYTES {
            return Err(IpcError::RuntimeUnavailable {
                message: "the server returned an oversized error response".into(),
            });
        }
        body
    } else {
        Vec::new()
    };
    Ok((status, response_body))
}

pub(crate) fn server_failure(status: u16, body: &[u8]) -> IpcError {
    match status {
        401 => IpcError::InvalidPayload {
            message: "the server rejected the admin token".into(),
        },
        404 => IpcError::NotFound {
            message: "the configured device was not found on the server".into(),
        },
        422 => match serde_json::from_slice::<AdminConfigErrorBody>(body) {
            Ok(AdminConfigErrorBody::InvalidConfig { issues }) => IpcError::Validation {
                message: format!(
                    "the server rejected this configuration with {} validation issue(s)",
                    issues.len()
                ),
                issues,
            },
            Err(_) => IpcError::RuntimeUnavailable {
                message: "the server rejected the configuration without validation details".into(),
            },
        },
        _ => IpcError::RuntimeUnavailable {
            message: format!("the server rejected the request (HTTP {status})"),
        },
    }
}

fn validate_target(value: &str, maximum: usize, label: &str) -> Result<(), IpcError> {
    if value.trim().is_empty() {
        return Err(IpcError::InvalidPayload {
            message: format!("{label} must not be empty"),
        });
    }
    if value.len() > maximum {
        return Err(IpcError::InvalidPayload {
            message: format!("{label} must be at most {maximum} UTF-8 bytes"),
        });
    }
    Ok(())
}

pub(crate) fn autostart_error(error: impl std::fmt::Display) -> IpcError {
    IpcError::Autostart {
        message: format!("cannot update start-at-login: {error}"),
    }
}

impl From<RuntimeError> for IpcError {
    fn from(error: RuntimeError) -> Self {
        match error {
            RuntimeError::InvalidConfig { issues } => Self::Validation {
                message: format!("configuration has {} validation issue(s)", issues.len()),
                issues,
            },
            RuntimeError::QueueFull => Self::RuntimeBusy {
                message: "the background runtime is busy; try again".into(),
            },
            RuntimeError::WorkerStopped | RuntimeError::ResponseTimeout => {
                Self::RuntimeUnavailable {
                    message: error.to_string(),
                }
            }
            RuntimeError::UnknownWidget { .. }
            | RuntimeError::UnknownScreen { .. }
            | RuntimeError::UnknownCard { .. }
            | RuntimeError::NotAPluginCard { .. } => Self::NotFound {
                message: error.to_string(),
            },
            RuntimeError::DeviceDisconnected => Self::Device {
                message: "device is disconnected".into(),
            },
            RuntimeError::Device { message } => Self::Device { message },
            RuntimeError::Provider { message } => Self::Provider { message },
        }
    }
}

impl From<StoreError> for IpcError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::Validation { issues } => Self::Validation {
                message: format!("configuration has {} validation issue(s)", issues.len()),
                issues,
            },
            other => Self::Persistence {
                message: other.to_string(),
            },
        }
    }
}

impl From<NetworkSettingsStoreError> for IpcError {
    fn from(error: NetworkSettingsStoreError) -> Self {
        Self::Persistence {
            message: error.to_string(),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A private directory named for the test that owns it, so parallel runs of these
    /// store-backed tests cannot collide on one path.
    fn scratch_directory(label: &str) -> std::path::PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};

        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("deskmate-{label}-{}-{serial}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        directory
    }
    use std::fs;
    use std::path::Path;

    use app_core::{
        AlertHold, AppConfig, AppPreferences, AppSnapshot, AssetKind, AssetSource,
        CURRENT_SCHEMA_VERSION, CardAlert, CardDataSnapshot, CardError, CardErrorKind, CardField,
        CardFieldValue, CardSettings, CarouselAdvance, ConnectionState, DeviceCapability,
        DeviceCounters, DeviceSnapshot, DisplayOrientation, DisplayTemplate, IconGlyphMapping,
        PersistenceState, Playlist, PlaylistEntry, PomodoroSnapshot, PomodoroState,
        ProviderSnapshot, ProviderState, RefreshPolicy, RuntimeDiagnostics, RuntimeError,
        RuntimeState, SAVED_SETTINGS_VALIDATION_FAILURE_MESSAGE, StoreWarning, UpdateChannel,
        UpdateCheckPolicy, UpdaterSettings, ValidationCode, WidgetTapAction,
    };
    use serde::Serialize;

    pub(crate) fn read_http_request(stream: &mut std::net::TcpStream) -> Vec<u8> {
        use std::io::Read as _;

        let mut request = Vec::new();
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let mut chunk = [0_u8; 1_024];
            let length = stream.read(&mut chunk).unwrap();
            assert_ne!(length, 0, "request ended before its HTTP headers");
            request.extend_from_slice(&chunk[..length]);
        }
        let header_end = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap()
            + 4;
        let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
        let content_length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length: "))
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        while request.len() < header_end + content_length {
            let mut chunk = [0_u8; 1_024];
            let length = stream.read(&mut chunk).unwrap();
            assert_ne!(length, 0, "request ended before its HTTP body");
            request.extend_from_slice(&chunk[..length]);
        }
        request
    }

    /// Mirrors `CardSettings::wire_config`'s `TemplateKind` mapping (`app-core`'s
    /// `config.rs`) 1:1, for every `DisplayTemplate` variant the app can construct.
    /// A future template variant that forgets to extend this match is a compile
    /// error, not a silent "unknown template" fallback — see `sim_template`'s docs.
    #[test]
    fn sim_template_mirrors_the_wire_mapping_for_every_display_template() {
        assert_eq!(
            sim_template(&DisplayTemplate::DigitalClock),
            lvgl_sim::SimTemplate::DigitalClock
        );
        assert_eq!(
            sim_template(&DisplayTemplate::ProgressRing),
            lvgl_sim::SimTemplate::ProgressRing
        );
        assert_eq!(
            sim_template(&DisplayTemplate::RowList),
            lvgl_sim::SimTemplate::RowList
        );
        assert_eq!(
            sim_template(&DisplayTemplate::AnalogClock),
            lvgl_sim::SimTemplate::AnalogClock
        );
        assert_eq!(
            sim_template(&DisplayTemplate::BigNumberLabel),
            lvgl_sim::SimTemplate::BigNumberLabel
        );
        assert_eq!(
            sim_template(&DisplayTemplate::IconBadgeText {
                icon_asset_id: Some("weather-icons".into())
            }),
            lvgl_sim::SimTemplate::IconBadgeText
        );
    }

    /// The preview shows what a person standing at the panel sees, which is upright
    /// at BOTH mountings -- the 180 degree mount is cancelled by the mounting itself.
    /// Restoring the pass-through this replaced (returning `LandscapeFlipped` for the
    /// flipped mounting) puts an upside-down clock in the settings window, and fails
    /// here.
    #[test]
    fn the_preview_renders_upright_whichever_way_the_panel_is_mounted() {
        for configured in [
            DisplayOrientation::Landscape,
            DisplayOrientation::LandscapeFlipped,
        ] {
            assert_eq!(
                preview_orientation(configured),
                lvgl_sim::SimOrientation::Landscape,
                "preview orientation for {configured:?} must be upright"
            );
        }
    }

    #[test]
    fn preview_template_for_resolves_every_built_in_template() {
        let card = CardSettings::Clock {
            id: "clock".into(),
            title: "Desk".into(),
            show_seconds: true,
            template: DisplayTemplate::DigitalClock,
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::DeviceLocal,
            alert: CardAlert::None,
        };
        assert_eq!(
            preview_template_for(&card, "clock").unwrap(),
            lvgl_sim::SimTemplate::DigitalClock
        );
    }

    /// The guard behind the routing, not the routing itself:
    /// `render_card_preview` never reaches this arm for a server-rendered card. It stays
    /// because a caller that forgets that must be refused, not served a built-in
    /// face at random.
    #[test]
    fn preview_template_for_a_server_rendered_card_is_a_typed_unsupported_refusal() {
        let card = CardSettings::Plugin {
            id: "aqi".into(),
            title: "Air quality".into(),
            plugin_id: "aqi".into(),
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::Interval { minutes: 15 },
            alert: CardAlert::None,
        };
        let error = preview_template_for(&card, "aqi").unwrap_err();
        assert!(
            matches!(error, IpcError::Unsupported { ref message } if message.contains("server-rendered")),
            "expected IpcError::Unsupported naming the server-rendered card, got {error:?}"
        );
    }

    #[test]
    fn sim_field_carries_the_key_and_maps_every_value_kind() {
        let text = sim_field(&CardField {
            key: "title".into(),
            value: CardFieldValue::Text {
                value: "Desk".into(),
            },
        });
        assert_eq!(text.name, "title");
        assert_eq!(text.value, lvgl_sim::SimFieldValue::Text("Desk".into()));

        let integer = sim_field(&CardField {
            key: "remaining_seconds".into(),
            value: CardFieldValue::Integer { value: 900 },
        });
        assert_eq!(integer.name, "remaining_seconds");
        assert_eq!(integer.value, lvgl_sim::SimFieldValue::Integer(900));

        let boolean = sim_field(&CardField {
            key: "stale".into(),
            value: CardFieldValue::Boolean { value: true },
        });
        assert_eq!(boolean.name, "stale");
        assert_eq!(boolean.value, lvgl_sim::SimFieldValue::Boolean(true));
    }

    #[test]
    fn draft_parser_bounds_and_strictly_decodes_the_envelope() {
        let default_json = serde_json::to_string(&AppConfig::default()).unwrap();
        assert_eq!(
            parse_valid_draft(&DraftPayload { json: default_json }).unwrap(),
            AppConfig::default()
        );

        let oversized = DraftPayload {
            json: "x".repeat(MAX_CONFIG_FILE_BYTES + 1),
        };
        assert!(matches!(
            parse_draft(&oversized),
            Err(IpcError::PayloadTooLarge { maximum_bytes, .. })
                if maximum_bytes == MAX_CONFIG_FILE_BYTES
        ));

        let unknown_field = DraftPayload {
            json: r#"{"schema_version":1,"preferences":{"timezone":"UTC","autostart":false,"paused":false},"widgets":[],"screens":[],"unexpected":true}"#.into(),
        };
        assert!(matches!(
            parse_draft(&unknown_field),
            Err(IpcError::InvalidPayload { .. })
        ));
    }

    /// A device snapshot with no connection, so draft validation exercises only the
    /// configuration's own issues.
    fn offline_device() -> AppSnapshot {
        contract_fixtures().snapshot.app
    }

    #[test]
    fn validation_is_non_mutating_and_returns_stable_issues() {
        let mut invalid = AppConfig::default();
        invalid.preferences.timezone = "Not/AZone".into();
        let result = validate_draft_for_device(
            &DraftPayload {
                json: serde_json::to_string(&invalid).unwrap(),
            },
            &offline_device().device,
        )
        .unwrap();
        assert!(!result.valid);
        assert_eq!(result.issues.len(), 1);
        assert_eq!(result.issues[0].code, ValidationCode::InvalidTimezone);
    }

    /// A draft can pass `validate()` cleanly (every field within its bounds) yet still
    /// be unsaveable because one of its fields has no `wire_config()` mapping. Before
    /// this fix, `validate_config_draft` ran only `validate()` and reported such a
    /// draft valid, so Save's separate `compile()` call was the first place the
    /// failure ever surfaced — with an error attached to no field, in a session where
    /// every other edit was now blocked too. `validate_config_draft` must catch this
    /// itself, on the card's own path, exactly like the save path does.
    ///
    /// `template` no longer fits this shape — `wire_config()` now lowers every
    /// `DisplayTemplate` (M4 task 3/8) — so this uses `tap_action: Dismiss`, which
    /// `wire_config()` still cannot lower (host tap actions are a later capability),
    /// to keep exercising the same "validates but cannot compile" gap.
    #[test]
    fn draft_validation_catches_a_field_with_no_wire_mapping_like_save_does() {
        let mut config = AppConfig::default();
        config.cards[0] = CardSettings::Clock {
            id: "clock".into(),
            title: "Desk".into(),
            show_seconds: true,
            template: DisplayTemplate::DigitalClock,
            tap_action: WidgetTapAction::Dismiss,
            refresh: RefreshPolicy::DeviceLocal,
            alert: CardAlert::None,
        };
        assert!(config.validate().is_ok(), "fixture must validate cleanly");

        let result = validate_draft_for_device(
            &DraftPayload {
                json: serde_json::to_string(&config).unwrap(),
            },
            &offline_device().device,
        )
        .unwrap();

        assert!(!result.valid);
        assert!(result.issues.iter().any(|issue| {
            issue.path == "cards[0]" && issue.code == ValidationCode::RequiresCapability
        }));
    }

    /// Final-review finding: `validate_config_draft` compiled the draft but never saw
    /// the connected device, so a configuration the attached firmware cannot render
    /// read as valid until Save's `ensure_device_compatibility` refused it. Draft
    /// validation must report the same issue Save would, on the same path.
    #[test]
    fn draft_validation_reports_the_capability_gap_save_would_reject() {
        let mut config = AppConfig::default();
        config.cards[0] = CardSettings::Clock {
            id: "clock".into(),
            title: "Desk".into(),
            show_seconds: true,
            template: DisplayTemplate::AnalogClock,
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::DeviceLocal,
            alert: CardAlert::None,
        };
        let draft = DraftPayload {
            json: serde_json::to_string(&config).unwrap(),
        };
        let required = config.compile(1).unwrap().required_capabilities;
        assert_ne!(
            required & DeviceCapability::ExtendedTemplates.bit(),
            0,
            "fixture must need the extended-templates capability"
        );

        // Offline: nothing to gate against, so the draft is reported as valid.
        let mut device = offline_device().device;
        assert!(validate_draft_for_device(&draft, &device).unwrap().valid);

        // Online, but on firmware without the capability: the same issue Save raises.
        device.connection = ConnectionState::Online;
        device.protocol_version = Some(1);
        device.capabilities = vec![DeviceCapability::CoreWidgets];
        device.unknown_capability_bits = 0;

        let result = validate_draft_for_device(&draft, &device).unwrap();
        assert!(!result.valid);
        assert_eq!(result.issues.len(), 1);
        assert_eq!(result.issues[0].path, "device.capabilities");
        assert_eq!(result.issues[0].code, ValidationCode::RequiresCapability);
        let IpcError::Validation {
            issues: save_issues,
            ..
        } = ensure_device_compatibility(&device, required).unwrap_err()
        else {
            panic!("save must reject an incompatible configuration as a validation error");
        };
        assert_eq!(
            result.issues, save_issues,
            "draft validation and save must report the identical issue"
        );
    }

    /// The capability gap is reported by name, not as a bitmask: "missing capability
    /// bits 0x0000000000000008" told the user nothing they could act on.
    #[test]
    fn capability_gaps_are_named_in_plain_language() {
        assert_eq!(
            describe_capabilities(DeviceCapability::ExtendedTemplates.bit()),
            "extended display templates"
        );
        assert_eq!(
            describe_capabilities(
                DeviceCapability::ConfigRotation.bit() | DeviceCapability::AssetTransfer.bit()
            ),
            "display rotation and icon and font asset transfer"
        );
        assert_eq!(
            describe_capabilities(DeviceCapability::ExtendedTemplates.bit() | 1 << 63),
            "extended display templates and an unrecognized device feature"
        );

        let mut device = offline_device().device;
        device.connection = ConnectionState::Online;
        device.protocol_version = Some(1);
        device.capabilities = vec![DeviceCapability::CoreWidgets];
        device.unknown_capability_bits = 0;
        let issues = missing_capability_issues(
            &device,
            DeviceCapability::CoreWidgets.bit() | DeviceCapability::ExtendedTemplates.bit(),
        );
        assert!(
            issues[0].message.contains("extended display templates"),
            "got {:?}",
            issues[0].message
        );
        assert!(
            !issues[0].message.contains("0x"),
            "no bitmask may reach the user: {:?}",
            issues[0].message
        );
    }

    #[test]
    fn action_targets_are_bounded_before_runtime_dispatch() {
        assert!(validate_target("clock", MAX_WIDGET_ID_LEN, "widget ID").is_ok());
        assert!(matches!(
            validate_target(" ", MAX_WIDGET_ID_LEN, "widget ID"),
            Err(IpcError::InvalidPayload { .. })
        ));
        assert!(matches!(
            validate_target(
                &"x".repeat(MAX_WIDGET_ID_LEN + 1),
                MAX_WIDGET_ID_LEN,
                "widget ID"
            ),
            Err(IpcError::InvalidPayload { .. })
        ));
    }

    #[test]
    fn runtime_errors_map_to_stable_ipc_categories() {
        assert!(matches!(
            IpcError::from(RuntimeError::QueueFull),
            IpcError::RuntimeBusy { .. }
        ));
        assert!(matches!(
            IpcError::from(RuntimeError::WorkerStopped),
            IpcError::RuntimeUnavailable { .. }
        ));
        assert!(matches!(
            IpcError::from(RuntimeError::UnknownWidget {
                widget_id: "missing".into()
            }),
            IpcError::NotFound { .. }
        ));
        assert!(matches!(
            IpcError::from(RuntimeError::Provider {
                message: "offline".into()
            }),
            IpcError::Provider { .. }
        ));
        assert!(matches!(
            IpcError::from(RuntimeError::DeviceDisconnected),
            IpcError::Device { .. }
        ));
    }

    #[test]
    fn stale_drafts_cannot_overwrite_command_owned_preferences() {
        let mut draft = AppConfig::default();
        draft.preferences.timezone = "Asia/Tbilisi".into();
        let mut current = AppConfig::default();
        current.preferences.paused = true;
        current.preferences.autostart = true;

        merge_command_owned_preferences(&mut draft, &current);

        assert_eq!(draft.preferences.timezone, "Asia/Tbilisi");
        assert!(draft.preferences.paused);
        assert!(draft.preferences.autostart);
    }

    #[test]
    fn server_config_url_preserves_the_base_path_and_encodes_the_device_id() {
        let url = server_config_url("https://desk.example/base/?discard=yes", "desk one").unwrap();
        assert_eq!(
            url.as_str(),
            "https://desk.example/base/v1/devices/desk%20one/config"
        );
    }

    #[test]
    fn operator_server_base_derives_the_secure_device_link_url() {
        assert_eq!(
            device_link_url("https://desk.example/base?discard=yes#fragment").unwrap(),
            "wss://desk.example/v1/device/link"
        );
        assert_eq!(
            device_link_url("https://[2001:db8::1]:8443").unwrap(),
            "wss://[2001:db8::1]:8443/v1/device/link"
        );
        let insecure = device_link_url("http://desk.example").unwrap_err();
        assert!(matches!(insecure, IpcError::InvalidPayload { .. }));
        assert!(insecure.to_string().contains("HTTPS"));
    }

    #[test]
    fn save_destination_follows_live_persisted_and_legacy_ownership() {
        use app_core::DeviceTier::{Local, Networked};

        let settings = |server_url: &str, device_id: &str, tier| NetworkSettings {
            server_url: server_url.into(),
            device_id: device_id.into(),
            tier,
        };
        let cases = [
            (
                Some(Networked),
                settings("", "", Some(Local)),
                SaveDestination::Server,
            ),
            (
                Some(Local),
                settings("https://desk.example", "desk-1", Some(Networked)),
                SaveDestination::Local,
            ),
            (
                None,
                settings("", "", Some(Networked)),
                SaveDestination::Server,
            ),
            (
                None,
                settings("https://desk.example", "desk-1", Some(Networked)),
                SaveDestination::Server,
            ),
            (None, settings("", "", Some(Local)), SaveDestination::Local),
            (
                None,
                settings("https://desk.example", "desk-1", Some(Local)),
                SaveDestination::Local,
            ),
            (None, settings("", "", None), SaveDestination::Local),
            (
                None,
                settings("https://desk.example", "", None),
                SaveDestination::Server,
            ),
            (None, settings("", "desk-1", None), SaveDestination::Server),
            (
                None,
                settings("https://desk.example", "desk-1", None),
                SaveDestination::Server,
            ),
        ];

        for (live_tier, settings, expected) in cases {
            assert_eq!(save_destination(live_tier, &settings), expected);
        }
    }

    /// A Mac that has only ever *observed* a networked board knows its tier but not
    /// its id: `remember_device_tier` persists the tier alone. Pairing cannot supply
    /// the id either, because that needs a plaintext device token the server keeps
    /// only as a digest. Saving server access is therefore the one path left, so the
    /// id typed beside the URL has to survive it.
    #[test]
    fn saving_server_access_persists_the_typed_device_id() {
        let directory = scratch_directory("server-access-device-id");
        let network_store = NetworkSettingsStore::new(directory.join("network-settings.json"));
        network_store
            .save(NetworkSettingsUpdate::new(
                "",
                "",
                Some(app_core::DeviceTier::Networked),
                None,
            ))
            .unwrap();

        let settings = set_server_endpoint_with_context(
            &network_store,
            ServerEndpointRequest {
                server_url: "https://desk.example".into(),
                device_id: "dev-0003".into(),
                admin_token: "admin-secret".into(),
            },
        )
        .unwrap();

        assert_eq!(settings.device_id, "dev-0003");
        assert_eq!(
            NetworkSettingsStore::new(directory.join("network-settings.json"))
                .load()
                .settings()
                .device_id,
            "dev-0003",
            "the typed device id never reached the settings file"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    /// Saving an endpoint before a device is paired is a supported flow -- it is what
    /// `use_local_ownership` exists to recover from -- so a blank box must mean "leave
    /// the id alone" rather than "erase the id I already have".
    #[test]
    fn saving_server_access_keeps_the_stored_device_id_when_the_box_is_blank() {
        let directory = scratch_directory("server-access-blank-device-id");
        let network_store = NetworkSettingsStore::new(directory.join("network-settings.json"));
        network_store
            .save(NetworkSettingsUpdate::new(
                "https://old.example",
                "dev-0003",
                Some(app_core::DeviceTier::Networked),
                None,
            ))
            .unwrap();

        let settings = set_server_endpoint_with_context(
            &network_store,
            ServerEndpointRequest {
                server_url: "https://desk.example".into(),
                device_id: "   ".into(),
                admin_token: "admin-secret".into(),
            },
        )
        .expect("a blank device id must not fail the save");

        assert_eq!(settings.device_id, "dev-0003");
        assert_eq!(settings.server_url, "https://desk.example");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn actual_local_save_refuses_persisted_network_ownership_before_writing() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "deskmate-local-ownership-{}-{serial}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let config_path = directory.join("config.json");
        let network_store = Arc::new(NetworkSettingsStore::new(
            directory.join("network-settings.json"),
        ));
        network_store
            .save(NetworkSettingsUpdate::new(
                "https://desk.example",
                "desk-1",
                Some(app_core::DeviceTier::Networked),
                Some("admin-secret".into()),
            ))
            .unwrap();
        let runtime = Arc::new(
            RuntimeHandle::start_serial(
                AppConfig::default(),
                Some("/definitely/not/a/deskmate-test-port".into()),
            )
            .unwrap(),
        );
        let context = ConfigSaveContext {
            runtime: Arc::clone(&runtime),
            store: Arc::new(ConfigStore::new(&config_path)),
            network_store,
            networked_config: Arc::new(NetworkedConfigProjection::default()),
            has_saved_config: Arc::new(AtomicBool::new(false)),
            mutation_lock: Arc::new(Mutex::new(())),
        };

        let result = save_and_apply(context, AppConfig::default());
        runtime.shutdown().unwrap();

        assert!(matches!(result, Err(IpcError::InvalidPayload { .. })));
        assert!(
            !config_path.exists(),
            "the refused local draft reached ConfigStore"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn failed_provision_does_not_record_an_ownership_transfer() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "deskmate-failed-provision-{}-{serial}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let network_store = Arc::new(NetworkSettingsStore::new(
            directory.join("network-settings.json"),
        ));
        network_store
            .save(NetworkSettingsUpdate::new(
                "https://old.example",
                "old-device",
                Some(app_core::DeviceTier::Local),
                None,
            ))
            .unwrap();
        let runtime = Arc::new(
            RuntimeHandle::start_serial(
                AppConfig::default(),
                Some("/definitely/not/a/deskmate-test-port".into()),
            )
            .unwrap(),
        );
        let context = ProvisionContext {
            runtime: Arc::clone(&runtime),
            network_store: Arc::clone(&network_store),
            networked_config: Arc::new(NetworkedConfigProjection::default()),
        };
        let request = ProvisionDeviceRequest {
            ssid: "home-network".into(),
            passphrase: "wifi-secret".into(),
            server_url: "https://desk.example".into(),
            device_id: "desk-1".into(),
            device_token: "device-secret".into(),
            tier: app_core::DeviceTier::Networked,
        };

        let result = provision_device_with_context(context, request);
        runtime.shutdown().unwrap();

        assert!(matches!(result, Err(IpcError::Device { .. })));
        assert_eq!(
            network_store.load().settings(),
            &NetworkSettings {
                server_url: "https://old.example".into(),
                device_id: "old-device".into(),
                tier: Some(app_core::DeviceTier::Local),
            },
            "a rejected device command changed the saved ownership"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn local_override_recovers_saved_routing_without_a_device_command() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "deskmate-local-override-{}-{serial}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let network_store = NetworkSettingsStore::new(directory.join("network-settings.json"));
        network_store
            .save(NetworkSettingsUpdate::new(
                "https://desk.example",
                "desk-1",
                Some(app_core::DeviceTier::Networked),
                Some("admin-secret".into()),
            ))
            .unwrap();
        let projection = NetworkedConfigProjection::default();
        let mut stale_server_config = AppConfig::default();
        stale_server_config.preferences.timezone = "Asia/Tbilisi".into();
        projection.replace(Some(stale_server_config)).unwrap();

        let settings = use_local_ownership_with_context(&network_store, &projection).unwrap();

        assert_eq!(settings.tier, Some(app_core::DeviceTier::Local));
        assert_eq!(
            network_store.load().settings().tier,
            Some(app_core::DeviceTier::Local)
        );
        let mut projected = AppConfig::default();
        projection.project(app_core::DeviceTier::Networked, &mut projected);
        assert_eq!(projected, AppConfig::default());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn incomplete_server_identity_refusal_does_not_claim_server_ownership() {
        let settings = NetworkSettings {
            server_url: "https://desk.example".into(),
            device_id: String::new(),
            tier: None,
        };

        assert_eq!(
            local_save_refusal(&settings),
            "a server endpoint is configured, but no device is paired; enter a device ID or connect over USB to confirm ownership"
        );
    }

    #[test]
    fn a_remote_failure_after_mirroring_names_both_destination_results() {
        let error = server_error_after_local_save(IpcError::Validation {
            message: "the server rejected this configuration".into(),
            issues: vec![ValidationIssue {
                path: "cards[0].title".into(),
                code: ValidationCode::Empty,
                message: "Choose a title.".into(),
            }],
        });

        assert!(matches!(
            error,
            IpcError::Validation { message, issues }
                if message.contains("saved on this Mac")
                    && message.contains("server destination did not succeed")
                    && issues.len() == 1
        ));
    }

    #[test]
    fn server_request_runs_after_the_mutation_lock_is_released() {
        use std::io::Write;
        use std::net::TcpListener;
        use std::time::{SystemTime, UNIX_EPOCH};

        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "deskmate-server-lock-{}-{serial}",
            std::process::id()
        ));
        fs::create_dir(&directory).unwrap();
        let config_path = directory.join("config.json");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let network_store = Arc::new(NetworkSettingsStore::new(
            directory.join("network-settings.json"),
        ));
        network_store
            .save(NetworkSettingsUpdate::new(
                format!("http://{address}"),
                "desk-1",
                Some(app_core::DeviceTier::Networked),
                Some("admin-secret".into()),
            ))
            .unwrap();
        let runtime = Arc::new(
            RuntimeHandle::start_serial(
                AppConfig::default(),
                Some("/definitely/not/a/deskmate-test-port".into()),
            )
            .unwrap(),
        );
        let mutation_lock = Arc::new(Mutex::new(()));
        let context = ServerSaveContext {
            config: ConfigSaveContext {
                runtime: Arc::clone(&runtime),
                store: Arc::new(ConfigStore::new(&config_path)),
                network_store,
                networked_config: Arc::new(NetworkedConfigProjection::default()),
                has_saved_config: Arc::new(AtomicBool::new(false)),
                mutation_lock: Arc::clone(&mutation_lock),
            },
            server_client: crate::server_http_agent(),
        };
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            assert!(
                mutation_lock.try_lock().is_ok(),
                "the server request started while the desktop mutation lock was held"
            );
            let request = read_http_request(&mut stream);
            assert!(String::from_utf8_lossy(&request).starts_with("PUT "));
            stream
                .write_all(
                    b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        });

        let result = save_server_config_blocking(context, AppConfig::default());
        server.join().unwrap();
        runtime.shutdown().unwrap();

        assert!(
            config_path.exists(),
            "the local authoring mirror was not saved"
        );
        assert!(matches!(
            result,
            Err(IpcError::RuntimeUnavailable { message })
                if message.contains("saved on this Mac")
                    && message.contains("server destination did not succeed")
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn server_invalid_config_is_the_existing_validation_ipc_category() {
        let error = server_failure(
            422,
            br#"{"kind":"invalid-config","issues":[{"path":"cards[0].title","code":"empty","message":"Choose a title."}]}"#,
        );
        assert!(matches!(
            &error,
            IpcError::Validation { message, issues }
                if issues.len() == 1
                    && issues[0].path == "cards[0].title"
                    && message.contains("server rejected")
                    && !message.contains("last working")
        ));
        assert_eq!(error.log_label(), "validation");

        let malformed = server_failure(422, b"not-json");
        assert!(matches!(malformed, IpcError::RuntimeUnavailable { .. }));
        assert!(!matches!(malformed, IpcError::Device { .. }));
    }

    #[test]
    fn server_transport_keeps_the_bounded_422_validation_body() {
        use std::io::Write;
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let body = br#"{"kind":"invalid-config","issues":[{"path":"cards","code":"empty","message":"Add a card."}]}"#;
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            let request = String::from_utf8_lossy(&request);
            assert!(request.starts_with("PUT /v1/devices/desk-1/config "));
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer admin-secret")
            );
            write!(
                stream,
                "HTTP/1.1 422 Unprocessable Entity\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
        });

        let (status, response_body) = put_server_config(
            &crate::server_http_agent(),
            &format!("http://{address}/v1/devices/desk-1/config"),
            "admin-secret",
            b"{}",
        )
        .unwrap();
        server.join().unwrap();
        assert_eq!(status, 422);
        assert!(matches!(
            server_failure(status, &response_body),
            IpcError::Validation { issues, .. }
                if issues.len() == 1 && issues[0].message == "Add a card."
        ));
    }

    #[test]
    fn networked_provisioning_requires_public_identity_and_write_only_token() {
        let request = ProvisionDeviceRequest {
            ssid: "home".into(),
            passphrase: String::new(),
            server_url: "https://desk.example".into(),
            device_id: "desk-1".into(),
            device_token: "device-secret".into(),
            tier: app_core::DeviceTier::Networked,
        };
        assert!(validate_network_config_request(&request).is_ok());

        let missing = ProvisionDeviceRequest {
            device_token: String::new(),
            ..request
        };
        let error = validate_network_config_request(&missing).unwrap_err();
        assert!(matches!(error, IpcError::InvalidPayload { .. }));
        assert!(!error.to_string().contains("device-secret"));
    }

    #[test]
    fn connected_legacy_firmware_is_rejected_during_persistence_preflight() {
        let mut device = contract_fixtures().snapshot.app.device;
        device.connection = ConnectionState::Online;
        device.protocol_version = Some(1);
        device.capabilities = vec![DeviceCapability::CoreWidgets];
        device.unknown_capability_bits = 0;
        let required = DeviceCapability::CoreWidgets.bit() | DeviceCapability::ConfigRotation.bit();

        let error = ensure_device_compatibility(&device, required).unwrap_err();
        assert!(matches!(
            error,
            IpcError::Validation { issues, .. }
                if issues.len() == 1
                    && issues[0].path == "device.capabilities"
                    && issues[0].code == ValidationCode::RequiresCapability
        ));

        device.connection = ConnectionState::Standalone;
        assert!(ensure_device_compatibility(&device, required).is_ok());
    }

    #[derive(Serialize)]
    struct ContractFixtures {
        snapshot: DesktopSnapshot,
        configs: Vec<AppConfig>,
        card_settings: Vec<CardSettings>,
        playlists: Vec<Playlist>,
        playlist_entries: Vec<PlaylistEntry>,
        card_alerts: Vec<CardAlert>,
        alert_holds: Vec<AlertHold>,
        carousel_advances: Vec<CarouselAdvance>,
        display_templates: Vec<DisplayTemplate>,
        tap_actions: Vec<WidgetTapAction>,
        refresh_policies: Vec<RefreshPolicy>,
        asset_sources: Vec<AssetSource>,
        asset_kinds: Vec<AssetKind>,
        update_channels: Vec<UpdateChannel>,
        update_check_policies: Vec<UpdateCheckPolicy>,
        display_orientations: Vec<DisplayOrientation>,
        device_capabilities: Vec<DeviceCapability>,
        runtime_states: Vec<RuntimeState>,
        connection_states: Vec<ConnectionState>,
        provider_states: Vec<ProviderState>,
        pomodoro_states: Vec<PomodoroState>,
        card_data: Vec<CardDataSnapshot>,
        persistence_states: Vec<PersistenceState>,
        validation_codes: Vec<ValidationCode>,
        pomodoro_actions: Vec<PomodoroAction>,
        errors: Vec<IpcError>,
        draft_validation: DraftValidation,
        config_apply_result: ConfigApplyResult,
        autostart_status: AutostartStatus,
        plugin_catalog: app_core::admin::PluginCatalog,
        server_card_state: Vec<ServerCardState>,
        preview_frame: PreviewFrame,
    }

    fn contract_card_kind(card: &CardSettings) -> &'static str {
        match card {
            CardSettings::Clock { .. } => "clock",
            CardSettings::Pomodoro { .. } => "pomodoro",
            CardSettings::Plugin { .. } => "plugin",
            CardSettings::Picture { .. } => "picture",
        }
    }

    #[allow(clippy::too_many_lines)]
    fn contract_fixtures() -> ContractFixtures {
        let issue = ValidationIssue {
            path: "active_playlist_id".into(),
            code: ValidationCode::MissingReference,
            message: "active playlist does not exist".into(),
        };
        let cards = vec![
            CardSettings::Clock {
                id: "clock".into(),
                title: "Desk".into(),
                show_seconds: true,
                template: DisplayTemplate::DigitalClock,
                tap_action: WidgetTapAction::None,
                refresh: RefreshPolicy::DeviceLocal,
                alert: CardAlert::None,
            },
            CardSettings::Pomodoro {
                id: "pomodoro".into(),
                label: "Focus".into(),
                duration_seconds: 1_500,
                template: DisplayTemplate::ProgressRing,
                tap_action: WidgetTapAction::StartPause,
                refresh: RefreshPolicy::DeviceLocal,
                alert: CardAlert::OnTimerFinish {
                    hold: AlertHold::UntilDismissed,
                },
            },
            CardSettings::Plugin {
                id: "air-quality".into(),
                title: "Office air".into(),
                plugin_id: "com.example.air-quality".into(),
                tap_action: WidgetTapAction::None,
                refresh: RefreshPolicy::Interval { minutes: 15 },
                alert: CardAlert::None,
            },
            CardSettings::Picture {
                id: "limits-picture".into(),
                title: "Limits".into(),
                source_id: "limits".into(),
                tap_action: WidgetTapAction::None,
                refresh: RefreshPolicy::Manual,
                alert: CardAlert::None,
            },
        ];
        let all_card_settings = cards.clone();
        let playlist_entries = vec![
            PlaylistEntry {
                card_id: "clock".into(),
                dwell_seconds: None,
            },
            PlaylistEntry {
                card_id: "pomodoro".into(),
                dwell_seconds: Some(20),
            },
            PlaylistEntry {
                card_id: "air-quality".into(),
                dwell_seconds: None,
            },
        ];
        let playlists = vec![
            Playlist {
                id: "workday".into(),
                name: "Workday".into(),
                advance: CarouselAdvance::Timed {
                    default_dwell_seconds: 30,
                },
                entries: playlist_entries.clone(),
            },
            Playlist {
                id: "manual".into(),
                name: "Manual".into(),
                advance: CarouselAdvance::Manual,
                entries: vec![playlist_entries[0].clone()],
            },
        ];
        let config = AppConfig {
            schema_version: CURRENT_SCHEMA_VERSION,
            preferences: AppPreferences {
                timezone: "Asia/Tbilisi".into(),
                autostart: true,
                paused: false,
                orientation: DisplayOrientation::LandscapeFlipped,
            },
            cards: cards.clone(),
            image_sources: vec![app_core::config::ImageSource {
                id: "limits".into(),
                name: "Claude limits".into(),
            }],
            assets: Vec::new(),
            playlists: playlists.clone(),
            active_playlist_id: "workday".into(),
            updater: UpdaterSettings::default(),
        };
        let card_data = vec![CardDataSnapshot {
            card_id: "air-quality".into(),
            fields: vec![
                CardField {
                    key: "summary".into(),
                    value: CardFieldValue::Text {
                        value: "42 · Good".into(),
                    },
                },
                CardField {
                    key: "stale".into(),
                    value: CardFieldValue::Boolean { value: false },
                },
            ],
        }];
        let snapshot = DesktopSnapshot {
            has_saved_config: true,
            app: AppSnapshot {
                config: config.clone(),
                runtime: RuntimeState::Error {
                    message: "example runtime error".into(),
                },
                device: DeviceSnapshot {
                    connection: ConnectionState::Disconnected {
                        reason: Some("USB disconnected".into()),
                    },
                    port_name: Some("/dev/cu.usbmodem1".into()),
                    firmware_version: Some("1.0.0".into()),
                    protocol_version: Some(1),
                    max_protocol_version: Some(1),
                    capabilities: vec![DeviceCapability::CoreWidgets],
                    unknown_capability_bits: 0,
                    uptime_ms: Some(42),
                    free_heap: Some(123_456),
                    rotation: Some(90),
                    tier: None,
                    wifi_state: None,
                    wifi_rssi: None,
                    ip: None,
                    last_network_error: None,
                    ota_state: None,
                    active_screen_id: Some("clock".into()),
                    counters: DeviceCounters {
                        host_reconnects: 1,
                        valid_frames: 2,
                        malformed_frames: 3,
                        crc_errors: 4,
                        overflow_frames: 5,
                        dropped_responses: 6,
                        rx_dropped_bytes: 7,
                        dropped_events: 8,
                        event_queue_high_water: 9,
                        dropped_ui_commands: 10,
                        ui_queue_high_water: 11,
                        host_dropped_events: 12,
                        detected_event_gaps: 13,
                    },
                },
                providers: vec![ProviderSnapshot {
                    widget_id: "air-quality".into(),
                    state: ProviderState::Stale {
                        message: "offline".into(),
                    },
                    last_success_unix_ms: Some(1_786_000_000_000),
                    age_seconds: Some(120),
                }],
                pomodoros: vec![PomodoroSnapshot {
                    widget_id: "pomodoro".into(),
                    state: PomodoroState::Running,
                    duration_seconds: 1_500,
                    remaining_seconds: 900,
                }],
                card_data: card_data.clone(),
                card_errors: vec![CardError {
                    kind: CardErrorKind::DataRefused,
                    card_id: "air-quality".into(),
                    message:
                        "the display refused this card's data (InvalidPayload): invalid push data"
                            .into(),
                }],
                persistence: PersistenceState::ValidationFailed {
                    message: SAVED_SETTINGS_VALIDATION_FAILURE_MESSAGE.into(),
                    issues: vec![issue.clone()],
                },
                diagnostics: RuntimeDiagnostics {
                    commands_processed: 1,
                    command_queue_full: 2,
                    provider_jobs_started: 3,
                    provider_queue_full: 4,
                    provider_results_discarded: 5,
                    subscriber_snapshots_overwritten: 6,
                    interrupt_dismissals_ignored: 7,
                },
            },
        };
        let runtime_states = vec![
            RuntimeState::Starting,
            RuntimeState::Running,
            RuntimeState::Paused,
            RuntimeState::Error {
                message: "error".into(),
            },
        ];
        let connection_states = vec![
            ConnectionState::Disconnected { reason: None },
            ConnectionState::Connecting,
            ConnectionState::Online,
            ConnectionState::Standalone,
        ];
        let provider_states = vec![
            ProviderState::Idle,
            ProviderState::Refreshing,
            ProviderState::Fresh,
            ProviderState::Stale {
                message: "stale".into(),
            },
            ProviderState::Error {
                message: "error".into(),
            },
        ];
        let pomodoro_states = vec![
            PomodoroState::Idle,
            PomodoroState::Running,
            PomodoroState::Paused,
            PomodoroState::Completed,
        ];
        let persistence_states = vec![
            PersistenceState::Clean,
            PersistenceState::Saving,
            PersistenceState::RecoverableError {
                message: "error".into(),
            },
            PersistenceState::ValidationFailed {
                message: SAVED_SETTINGS_VALIDATION_FAILURE_MESSAGE.into(),
                issues: vec![issue.clone()],
            },
        ];
        let validation_codes = vec![
            ValidationCode::UnsupportedVersion,
            ValidationCode::Empty,
            ValidationCode::TooLong,
            ValidationCode::TooMany,
            ValidationCode::DuplicateId,
            ValidationCode::MissingReference,
            ValidationCode::OutOfRange,
            ValidationCode::InvalidTimezone,
            ValidationCode::InvalidSource,
            ValidationCode::InvalidComposition,
            ValidationCode::TooLarge,
            ValidationCode::RequiresCapability,
        ];
        let pomodoro_actions = vec![
            PomodoroAction::Start,
            PomodoroAction::Pause,
            PomodoroAction::Toggle,
            PomodoroAction::Reset,
        ];
        let errors = vec![
            IpcError::InvalidPayload {
                message: "invalid".into(),
            },
            IpcError::PayloadTooLarge {
                message: "large".into(),
                maximum_bytes: MAX_CONFIG_FILE_BYTES,
            },
            IpcError::Validation {
                message: "validation".into(),
                issues: vec![issue.clone()],
            },
            IpcError::Persistence {
                message: "persistence".into(),
            },
            IpcError::RuntimeBusy {
                message: "busy".into(),
            },
            IpcError::RuntimeUnavailable {
                message: "unavailable".into(),
            },
            IpcError::IncompatibleServer {
                message: "incompatible".into(),
            },
            IpcError::NotFound {
                message: "missing".into(),
            },
            IpcError::Device {
                message: "device".into(),
            },
            IpcError::Provider {
                message: "provider".into(),
            },
            IpcError::Autostart {
                message: "autostart".into(),
            },
            IpcError::Window {
                message: "window".into(),
            },
            IpcError::Internal {
                message: "internal".into(),
            },
            IpcError::Unsupported {
                message: "unsupported".into(),
            },
        ];

        ContractFixtures {
            snapshot,
            configs: vec![config],
            card_settings: all_card_settings,
            playlists,
            playlist_entries,
            card_alerts: vec![
                CardAlert::None,
                CardAlert::OnTimerFinish {
                    hold: AlertHold::UntilDismissed,
                },
            ],
            alert_holds: vec![AlertHold::UntilDismissed, AlertHold::Seconds { value: 60 }],
            carousel_advances: vec![
                CarouselAdvance::Manual,
                CarouselAdvance::Timed {
                    default_dwell_seconds: 20,
                },
            ],
            display_templates: vec![
                DisplayTemplate::DigitalClock,
                DisplayTemplate::AnalogClock,
                DisplayTemplate::ProgressRing,
                DisplayTemplate::RowList,
                DisplayTemplate::BigNumberLabel,
                DisplayTemplate::IconBadgeText {
                    icon_asset_id: Some("weather-icons".into()),
                },
            ],
            tap_actions: vec![
                WidgetTapAction::None,
                WidgetTapAction::StartPause,
                WidgetTapAction::Reset,
                WidgetTapAction::Dismiss,
                WidgetTapAction::OpenUrl {
                    url: "https://example.test/action".into(),
                },
                WidgetTapAction::OpenApplication {
                    application_id: "com.example.app".into(),
                },
            ],
            refresh_policies: vec![
                RefreshPolicy::DeviceLocal,
                RefreshPolicy::Manual,
                RefreshPolicy::Interval { minutes: 15 },
            ],
            asset_sources: vec![AssetSource::File("/tmp/status-icons.ttf".into())],
            asset_kinds: vec![
                AssetKind::Font,
                AssetKind::IconFont {
                    glyphs: vec![IconGlyphMapping {
                        name: "cloud-rain".into(),
                        codepoint: 0xf729,
                    }],
                },
                AssetKind::Image,
            ],
            update_channels: vec![
                UpdateChannel::Stable,
                UpdateChannel::Beta,
                UpdateChannel::Manual,
            ],
            update_check_policies: vec![UpdateCheckPolicy::Disabled, UpdateCheckPolicy::Notify],
            display_orientations: vec![
                DisplayOrientation::Landscape,
                DisplayOrientation::LandscapeFlipped,
            ],
            device_capabilities: DeviceCapability::from_bits(DeviceCapability::known_bits()),
            runtime_states,
            connection_states,
            provider_states,
            pomodoro_states,
            card_data,
            persistence_states,
            validation_codes,
            pomodoro_actions,
            errors,
            draft_validation: DraftValidation {
                valid: false,
                issues: vec![issue],
            },
            config_apply_result: ConfigApplyResult {
                save: SaveReceipt {
                    generation: 7,
                    warning: Some(StoreWarning::Io {
                        operation: "sync config directory".into(),
                        message: "example warning".into(),
                    }),
                },
            },
            autostart_status: AutostartStatus {
                enabled: true,
                preference_enabled: false,
            },
            plugin_catalog: app_core::admin::PluginCatalog {
                plugins: vec![app_core::admin::PluginCatalogEntry {
                    id: "aqi".into(),
                    name: "aqi".into(),
                    version: "1.0.0".into(),
                    node_count: 4,
                    assets: vec![app_core::admin::PluginCatalogAsset {
                        file: "icons.ttf".into(),
                        kind: "icon-font".into(),
                        byte_length: 40_960,
                        digest: "0f1e2d3c".into(),
                    }],
                    display_name: Some("Air quality".into()),
                    description: Some("EPA index for a location".into()),
                    manifest_version: 2,
                    template: app_core::admin::PluginTemplateKind::DisplayList,
                    refresh_minutes: 15,
                }],
                load_failures: vec![app_core::admin::PluginLoadFailure {
                    id: "broken".into(),
                    error: "unknown key \"summry\"".into(),
                }],
            },
            server_card_state: vec![ServerCardState {
                card_id: "air-quality".into(),
                provider: ProviderState::Fresh,
                hero: Some("42".into()),
                errors: vec![CardError {
                    kind: CardErrorKind::SceneRefused,
                    card_id: "air-quality".into(),
                    message: "no snapshot cached yet".into(),
                }],
            }],
            preview_frame: PreviewFrame {
                png_base64: Some("iVBORw0KGgo=".into()),
                sample: true,
                state: None,
            },
        }
    }

    #[test]
    fn contract_fixture_covers_every_card_settings_variant() {
        let kinds = contract_fixtures()
            .card_settings
            .iter()
            .map(contract_card_kind)
            .collect::<Vec<_>>();
        assert_eq!(kinds, ["clock", "pomodoro", "plugin", "picture"]);
    }

    #[test]
    fn every_server_preview_outcome_maps_to_one_stage_state() {
        use app_core::admin::{CardPreviewResponse, CardPreviewState};

        let rendered = server_preview_frame(CardPreviewResponse {
            png_base64: Some("iVBORw0KGgo=".into()),
            state: CardPreviewState::Stale,
            message: None,
            refreshed_at_unix_ms: Some(1_787_000_000_000),
        });
        assert_eq!(
            rendered,
            PreviewFrame {
                png_base64: Some("iVBORw0KGgo=".into()),
                sample: false,
                state: None,
            }
        );

        // `sample` means "a real frame rendered, from an empty field set" -- it is
        // what puts the "No data yet" badge on a drawn image. A waiting plugin card
        // has no frame at all, so it is NOT sample: it prints the state sentence.
        let waiting = server_preview_frame(CardPreviewResponse {
            png_base64: None,
            state: CardPreviewState::Waiting,
            message: Some("Waiting for the first refresh".into()),
            refreshed_at_unix_ms: None,
        });
        assert_eq!(
            waiting,
            PreviewFrame {
                png_base64: None,
                sample: false,
                state: Some("Waiting for the first refresh".into()),
            }
        );

        let failed = server_preview_frame(CardPreviewResponse {
            png_base64: None,
            state: CardPreviewState::Error,
            message: Some("Plugin \"x\" is not loaded on the server".into()),
            refreshed_at_unix_ms: None,
        });
        assert_eq!(
            failed,
            PreviewFrame {
                png_base64: None,
                sample: false,
                state: Some("Plugin \"x\" is not loaded on the server".into()),
            }
        );

        // A server that says nothing still says something on the stage.
        let mute = server_preview_frame(CardPreviewResponse {
            png_base64: None,
            state: CardPreviewState::Error,
            message: None,
            refreshed_at_unix_ms: None,
        });
        assert!(matches!(mute.state, Some(message) if !message.is_empty()));
    }

    #[test]
    fn a_plugin_card_the_mac_cannot_render_says_where_it_renders() {
        assert_eq!(
            unrendered_server_frame(PLUGIN_RENDERS_ON_THE_SERVER),
            PreviewFrame {
                png_base64: None,
                sample: false,
                state: Some("Plugin cards render on the server".into()),
            }
        );
        assert_eq!(
            unrendered_server_frame(SERVER_PREVIEW_UNAVAILABLE).state,
            Some("The server has no preview for this card".into())
        );
    }

    #[test]
    fn a_picture_card_the_mac_cannot_render_says_where_it_renders_without_calling_it_a_plugin() {
        assert_eq!(
            unrendered_server_frame(PICTURE_RENDERS_ON_THE_SERVER),
            PreviewFrame {
                png_base64: None,
                sample: false,
                state: Some("Picture cards render on the server".into()),
            }
        );
        assert!(!PICTURE_RENDERS_ON_THE_SERVER.contains("Plugin"));
    }

    #[test]
    fn server_card_state_covers_plugin_and_picture_cards_and_reads_the_hero_field() {
        let body = serde_json::json!({
            "device_id": "desk-1",
            "connected": true,
            "last_seen_unix_ms": 1_787_000_000_000_u64,
            "config": { "origin": "current", "using_fallback": false, "fallback_reason": null },
            "snapshot": {
                "config": {
                    "cards": [
                        { "kind": "clock", "id": "clock", "title": "Desk",
                          "show_seconds": true, "template": { "kind": "digital-clock" },
                          "tap_action": { "kind": "none" },
                          "refresh": { "kind": "device-local" }, "alert": { "kind": "none" } },
                        { "kind": "plugin", "id": "air", "title": "", "plugin_id": "aqi",
                          "tap_action": { "kind": "none" },
                          "refresh": { "kind": "interval", "minutes": 15 },
                          "alert": { "kind": "none" } },
                        { "kind": "plugin", "id": "news", "title": "", "plugin_id": "agenda",
                          "tap_action": { "kind": "none" },
                          "refresh": { "kind": "interval", "minutes": 30 },
                          "alert": { "kind": "none" } },
                        { "kind": "picture", "id": "limits", "title": "Usage",
                          "source_id": "limits-source", "tap_action": { "kind": "none" },
                          "refresh": { "kind": "manual" },
                          "alert": { "kind": "none" } }
                    ]
                },
                "device": { "observed_age_seconds": 4, "last_ota_error": null },
                "providers": [
                    { "widget_id": "air", "state": { "kind": "fresh" },
                      "last_success_unix_ms": 1_787_000_000_000_i64, "age_seconds": 30 },
                    { "widget_id": "clock", "state": { "kind": "idle" },
                      "last_success_unix_ms": null, "age_seconds": null },
                    { "widget_id": "limits", "state": { "kind": "stale", "message": "late" },
                      "last_success_unix_ms": 1_787_000_000_000_i64, "age_seconds": 1800 }
                ],
                "card_data": [
                    { "card_id": "air", "fields": [
                        { "key": "title", "value": { "kind": "text", "value": "Air quality" } },
                        { "key": "hero", "value": { "kind": "text", "value": "42" } } ] },
                    { "card_id": "clock", "fields": [] }
                ],
                "card_errors": [
                    { "kind": "scene-refused", "card_id": "news",
                      "message": "no snapshot cached yet" },
                    { "kind": "scene-refused", "card_id": "limits",
                      "message": "waiting for the first picture" },
                    { "kind": "data-refused", "card_id": "clock", "message": "ignored" }
                ]
            }
        });

        let states = project_server_card_state(&serde_json::from_value(body).unwrap());

        assert_eq!(
            states,
            vec![
                ServerCardState {
                    card_id: "air".into(),
                    provider: ProviderState::Fresh,
                    hero: Some("42".into()),
                    errors: Vec::new(),
                },
                // No provider entry yet is Idle, not an invented staleness.
                ServerCardState {
                    card_id: "news".into(),
                    provider: ProviderState::Idle,
                    hero: None,
                    errors: vec![CardError {
                        kind: CardErrorKind::SceneRefused,
                        card_id: "news".into(),
                        message: "no snapshot cached yet".into(),
                    }],
                },
                ServerCardState {
                    card_id: "limits".into(),
                    provider: ProviderState::Stale {
                        message: "late".into(),
                    },
                    hero: None,
                    errors: vec![CardError {
                        kind: CardErrorKind::SceneRefused,
                        card_id: "limits".into(),
                        message: "waiting for the first picture".into(),
                    }],
                },
            ]
        );
    }

    #[test]
    fn a_device_with_no_runtime_projects_no_server_card_state() {
        let body = serde_json::json!({
            "device_id": "desk-1", "connected": false, "last_seen_unix_ms": null,
            "config": { "origin": "defaults", "using_fallback": false, "fallback_reason": null },
            "snapshot": null
        });
        assert!(project_server_card_state(&serde_json::from_value(body).unwrap()).is_empty());
    }

    /// Builds a `ServerQueryContext` pointed at a loopback listener with a stored
    /// admin token, in the tier the caller names.
    fn server_query_fixture(
        label: &str,
        tier: app_core::DeviceTier,
    ) -> (
        ServerQueryContext,
        std::net::TcpListener,
        std::path::PathBuf,
    ) {
        let directory = scratch_directory(label);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let network_store = Arc::new(NetworkSettingsStore::new(
            directory.join("network-settings.json"),
        ));
        network_store
            .save(NetworkSettingsUpdate::new(
                format!("http://{address}"),
                "desk-1",
                Some(tier),
                Some("admin-secret".into()),
            ))
            .unwrap();
        let context = ServerQueryContext {
            agent: crate::server_http_agent(),
            network_store,
        };
        (context, listener, directory)
    }

    fn answer_once(
        listener: std::net::TcpListener,
        status: &'static str,
        body: impl Into<String>,
    ) -> std::thread::JoinHandle<String> {
        use std::io::Write as _;

        let body = body.into();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = String::from_utf8_lossy(&read_http_request(&mut stream)).to_string();
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            request
        })
    }

    #[test]
    fn the_catalog_command_reads_the_shared_admin_dto() {
        let (context, listener, directory) =
            server_query_fixture("catalog", app_core::DeviceTier::Networked);
        let server = answer_once(
            listener,
            "200 OK",
            r#"{"plugins":[{"id":"aqi","name":"aqi","version":"1.0.0","node_count":4,
                "assets":[{"file":"icons.ttf","kind":"icon-font","byte_length":12,"digest":"ab"}],
                "display_name":"Air quality","description":"EPA index for a location",
                "manifest_version":2,"template":"display-list","refresh_minutes":15}],
              "load_failures":[{"id":"broken","error":"unknown key"}]}"#,
        );

        let catalog = fetch_server_plugins(&context).unwrap();

        let request = server.join().unwrap();
        assert!(request.starts_with("GET /v1/plugins "));
        assert_eq!(catalog.plugins.len(), 1);
        assert_eq!(
            catalog.plugins[0].display_name.as_deref(),
            Some("Air quality")
        );
        assert_eq!(catalog.plugins[0].refresh_minutes, 15);
        assert_eq!(
            catalog.plugins[0].template,
            app_core::admin::PluginTemplateKind::DisplayList
        );
        assert_eq!(catalog.load_failures[0].id, "broken");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn minting_a_picture_source_posts_its_name_and_returns_one_time_access() {
        let (context, listener, directory) =
            server_query_fixture("mint-picture-source", app_core::DeviceTier::Networked);
        let token = "11".repeat(32);
        let server = answer_once(
            listener,
            "200 OK",
            format!(r#"{{"id":"picture-source","token":"{token}"}}"#),
        );

        let minted = mint_server_image_source(&context, "Picture").unwrap();

        let request = server.join().unwrap();
        let normalized = request.to_ascii_lowercase();
        assert!(request.starts_with("POST /v1/images "));
        assert!(normalized.contains("authorization: bearer admin-secret\r\n"));
        assert!(normalized.contains("content-type: application/json\r\n"));
        assert!(request.ends_with(r#"{"name":"Picture"}"#));
        assert_eq!(minted.source_id, "picture-source");
        assert_eq!(minted.token, token);
        assert!(
            minted
                .push_url
                .ends_with(&format!("/v1/images/{}", minted.token))
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn the_card_state_command_asks_for_this_device_and_keeps_server_rendered_cards() {
        let (context, listener, directory) =
            server_query_fixture("card-state", app_core::DeviceTier::Networked);
        let server = answer_once(
            listener,
            "200 OK",
            r#"{"device_id":"desk-1","connected":true,"last_seen_unix_ms":1,
                "config":{"origin":"current","using_fallback":false,"fallback_reason":null},
                "snapshot":{"config":{"cards":[
                    {"kind":"plugin","id":"air","title":"","plugin_id":"aqi",
                     "tap_action":{"kind":"none"},"refresh":{"kind":"interval","minutes":15},
                     "alert":{"kind":"none"}},
                    {"kind":"picture","id":"limits","title":"Usage","source_id":"source",
                     "tap_action":{"kind":"none"},"refresh":{"kind":"manual"},
                     "alert":{"kind":"none"}}]},
                  "providers":[{"widget_id":"air","state":{"kind":"fresh"},
                    "last_success_unix_ms":null,"age_seconds":null},
                    {"widget_id":"limits","state":{"kind":"idle"},
                    "last_success_unix_ms":null,"age_seconds":null}],
                  "card_data":[{"card_id":"air","fields":[
                    {"key":"hero","value":{"kind":"text","value":"42"}}]}],
                  "card_errors":[]}}"#,
        );

        let states = fetch_server_card_state(&context).unwrap();

        let request = server.join().unwrap();
        assert!(request.starts_with("GET /v1/devices/desk-1 "));
        assert_eq!(states.len(), 2);
        assert_eq!(states[0].hero.as_deref(), Some("42"));
        assert_eq!(states[1].card_id, "limits");
        fs::remove_dir_all(directory).unwrap();
    }

    /// The device-status body embeds the device's whole `AppConfig`, which is
    /// itself legal up to `MAX_CONFIG_FILE_BYTES`. Reading it under the 64 KiB
    /// error-body bound made a near-maximal config a permanent "oversized
    /// response" -- a poll that could never succeed, reported as the same notice a
    /// dead server gets. The filler here is ordinary config the Mac's partial DTO
    /// ignores, sized past that old bound.
    #[test]
    fn a_device_status_body_larger_than_a_maximal_config_is_still_read() {
        let (context, listener, directory) =
            server_query_fixture("card-state-large", app_core::DeviceTier::Networked);
        let playlists = (0..800)
            .map(|index| {
                format!(
                    r#"{{"id":"loop-{index}","name":"Loop {index}","advance":{{"kind":"timed","default_dwell_seconds":20}},"entries":[]}}"#
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let body = format!(
            r#"{{"device_id":"desk-1","connected":true,"last_seen_unix_ms":1,
                "config":{{"origin":"current","using_fallback":false,"fallback_reason":null}},
                "snapshot":{{"config":{{"playlists":[{playlists}],"cards":[
                    {{"kind":"plugin","id":"air","title":"","plugin_id":"aqi",
                     "tap_action":{{"kind":"none"}},"refresh":{{"kind":"interval","minutes":15}},
                     "alert":{{"kind":"none"}}}}]}},
                  "providers":[],"card_data":[],"card_errors":[]}}}}"#
        );
        assert!(
            body.len() > MAX_SERVER_ERROR_BYTES,
            "the filler must exceed the error-body bound to prove anything"
        );
        assert!(body.len() < MAX_SERVER_DEVICE_STATUS_BYTES);
        let server = answer_once(listener, "200 OK", body);

        let states = fetch_server_card_state(&context).unwrap();

        server.join().unwrap();
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].card_id, "air");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_plugin_preview_asks_the_card_route_and_returns_its_frame() {
        let (context, listener, directory) =
            server_query_fixture("preview-ok", app_core::DeviceTier::Networked);
        let server = answer_once(
            listener,
            "200 OK",
            r#"{"png_base64":"iVBORw0KGgo=","state":"fresh","message":null,
                "refreshed_at_unix_ms":1787000000000}"#,
        );

        let frame = server_card_preview(
            &context,
            Some(app_core::DeviceTier::Networked),
            "air",
            PLUGIN_RENDERS_ON_THE_SERVER,
        )
        .unwrap();

        let request = server.join().unwrap();
        assert!(request.starts_with("GET /v1/devices/desk-1/cards/air/preview "));
        assert_eq!(frame.png_base64.as_deref(), Some("iVBORw0KGgo="));
        assert_eq!(frame.state, None);
        assert!(!frame.sample);
        fs::remove_dir_all(directory).unwrap();
    }

    /// Spec section 10: the Mac's GETs are additive, so a server that has never
    /// heard of this route 404s. That degrades to a printed sentence rather than a
    /// fault, and it is not the local-tier sentence, which is chosen before any
    /// request is made.
    ///
    /// An unknown device, an unknown card and a built-in card also 404, and the
    /// status code cannot tell any of the four apart -- so the sentence claims only
    /// that no preview came back. An earlier version said "Plugin previews need a
    /// newer server", which asserted a cause this code cannot know.
    #[test]
    fn a_preview_the_server_does_not_return_degrades_instead_of_erroring() {
        let (context, listener, directory) =
            server_query_fixture("preview-404", app_core::DeviceTier::Networked);
        let server = answer_once(listener, "404 Not Found", "");

        let frame = server_card_preview(
            &context,
            Some(app_core::DeviceTier::Networked),
            "air",
            PLUGIN_RENDERS_ON_THE_SERVER,
        )
        .unwrap();

        server.join().unwrap();
        assert_eq!(frame.state.as_deref(), Some(SERVER_PREVIEW_UNAVAILABLE));
        assert_eq!(frame.png_base64, None);
        fs::remove_dir_all(directory).unwrap();
    }

    /// Local tier has no server to render on and the Mac has no plugin host, so it
    /// says so from the tier alone rather than opening a socket at all.
    #[test]
    fn a_local_tier_plugin_preview_never_reaches_the_network() {
        let (context, listener, directory) =
            server_query_fixture("preview-local", app_core::DeviceTier::Local);

        let frame = server_card_preview(
            &context,
            Some(app_core::DeviceTier::Local),
            "air",
            PLUGIN_RENDERS_ON_THE_SERVER,
        )
        .unwrap();

        assert_eq!(frame.state.as_deref(), Some(PLUGIN_RENDERS_ON_THE_SERVER));
        assert_eq!(frame.png_base64, None);
        drop(listener);
        fs::remove_dir_all(directory).unwrap();
    }

    fn typescript_contract_source() -> String {
        let json = serde_json::to_string_pretty(&contract_fixtures()).unwrap();
        format!(
            "// Generated by the Rust IPC contract test; edit the DTOs, not this fixture.\n\
             import type {{ IpcContractFixtures }} from \"./types\";\n\n\
             export const ipcContractFixtures = {json} as const satisfies IpcContractFixtures;\n"
        )
    }

    #[test]
    fn typescript_contract_fixture_stays_in_sync() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/lib/types.contract.ts");
        let checked = fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!(
                "cannot read checked TypeScript contract {}: {error}",
                path.display()
            )
        });
        assert_eq!(checked, typescript_contract_source());
    }

    #[test]
    #[ignore = "prints the checked TypeScript fixture for intentional regeneration"]
    fn print_typescript_contract_fixture() {
        print!("{}", typescript_contract_source());
    }
}
