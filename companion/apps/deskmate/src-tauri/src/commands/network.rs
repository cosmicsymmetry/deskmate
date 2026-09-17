//! Everything that talks to something outside this process: the server
//! endpoint, provisioning and factory reset, ownership transfer, mirroring a
//! saved config to the server, and minting an image source.
//!
//! Split out of `commands.rs` unchanged on 2026-09-11. The glob keeps name
//! resolution identical to when this was one file.

#[allow(clippy::wildcard_imports)]
use super::*;

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MintImageSourceRequest<'a> {
    pub(super) name: &'a str,
    /// Set when the owner picked a server-drawn face from the add menu. The
    /// server mints the source and attaches the face in one request, so a card
    /// arrives ready rather than needing a second round trip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) face_kind: Option<&'a str>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ServerMintedImageSource {
    pub(super) id: String,
    pub(super) token: String,
}

/// The one serialization boundary for the plaintext source credential. This value
/// lives only long enough to cross IPC once; neither the Rust state nor the config
/// retains it.
#[derive(Serialize)]
pub struct MintedImageSource {
    pub(crate) source_id: String,
    pub(crate) token: String,
    pub(crate) push_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageSourceDescriptor {
    pub id: String,
    pub name: String,
    pub face: Option<FaceDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaceDescriptor {
    pub kind: String,
    pub label: String,
    pub fields: Vec<FaceFieldDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum FaceFieldDescriptor {
    Text {
        key: String,
        label: String,
        value: String,
        placeholder: String,
    },
    Url {
        key: String,
        label: String,
        value: String,
        placeholder: String,
    },
    Enum {
        key: String,
        label: String,
        value: String,
        options: Vec<FaceFieldOption>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaceFieldOption {
    pub value: String,
    pub label: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateImageSourceFaceRequest {
    pub(super) source_id: String,
    pub(super) fields: BTreeMap<String, String>,
}

#[derive(Serialize)]
struct ServerUpdateFaceRequest<'a> {
    fields: &'a BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct ImageRouteErrorBody {
    message: Option<String>,
}

pub(super) fn mint_server_image_source(
    context: &ServerQueryContext,
    name: &str,
    face_kind: Option<&str>,
) -> Result<MintedImageSource, IpcError> {
    validate_target(
        name,
        app_core::config::MAX_IMAGE_SOURCE_NAME_LEN,
        "picture source name",
    )?;
    let settings = context.network_store.load().settings().clone();
    let url = crate::server_client::server_url(&settings.server_url, &["v1", "images"])?;
    let body = serde_json::to_vec(&MintImageSourceRequest { name, face_kind }).map_err(|_| {
        IpcError::Internal {
            message: "the picture source request could not be encoded".into(),
        }
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
            .limit(MAX_IMAGE_SOURCE_RESPONSE_BYTES.saturating_add(1) as u64)
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
    validate_target(&minted.id, MAX_CARD_ID_LEN, "picture source ID")?;
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
pub async fn mint_image_source(
    state: State<'_, DesktopState>,
    source_name: String,
    face_kind: Option<String>,
) -> Result<MintedImageSource, IpcError> {
    let context = ServerQueryContext::from_desktop(&state);
    on_server_worker(move || mint_server_image_source(&context, &source_name, face_kind.as_deref()))
        .await
}

#[tauri::command]
pub async fn list_creatable_faces(
    state: State<'_, DesktopState>,
) -> Result<Vec<FaceDescriptor>, IpcError> {
    let context = ServerQueryContext::from_desktop(&state);
    on_server_worker(move || list_server_creatable_faces(&context)).await
}

pub(super) fn list_server_creatable_faces(
    context: &ServerQueryContext,
) -> Result<Vec<FaceDescriptor>, IpcError> {
    let settings = context.network_store.load().settings().clone();
    let url = crate::server_client::server_url(&settings.server_url, &["v1", "faces"])?;
    let body = context.with_admin_token(|token| {
        let response = context
            .agent
            .get(url.as_str())
            .header("Authorization", format!("Bearer {token}"))
            .call()
            .map_err(|_| unreachable_server())?;
        read_image_source_response(response)
    })?;
    let faces: Vec<FaceDescriptor> =
        serde_json::from_slice(&body).map_err(|_| IpcError::IncompatibleServer {
            message: "the server listed faces this app could not read".into(),
        })?;
    Ok(faces)
}

#[tauri::command]
pub async fn list_image_sources(
    state: State<'_, DesktopState>,
) -> Result<Vec<ImageSourceDescriptor>, IpcError> {
    let context = ServerQueryContext::from_desktop(&state);
    on_server_worker(move || list_server_image_sources(&context)).await
}

#[tauri::command]
pub async fn update_image_source_face(
    state: State<'_, DesktopState>,
    request: UpdateImageSourceFaceRequest,
) -> Result<FaceDescriptor, IpcError> {
    let context = ServerQueryContext::from_desktop(&state);
    on_server_worker(move || update_server_image_source_face(&context, request)).await
}

pub(super) fn list_server_image_sources(
    context: &ServerQueryContext,
) -> Result<Vec<ImageSourceDescriptor>, IpcError> {
    let settings = context.network_store.load().settings().clone();
    let url = crate::server_client::server_url(&settings.server_url, &["v1", "images"])?;
    let body = context.with_admin_token(|token| {
        let response = context
            .agent
            .get(url.as_str())
            .header("Authorization", format!("Bearer {token}"))
            .call()
            .map_err(|_| unreachable_server())?;
        read_image_source_response(response)
    })?;
    let sources: Vec<ImageSourceDescriptor> =
        serde_json::from_slice(&body).map_err(|_| IpcError::IncompatibleServer {
            message: "the server returned image-source settings this app could not read".into(),
        })?;
    validate_image_source_descriptors(&sources)?;
    Ok(sources)
}

pub(super) fn update_server_image_source_face(
    context: &ServerQueryContext,
    request: UpdateImageSourceFaceRequest,
) -> Result<FaceDescriptor, IpcError> {
    validate_target(&request.source_id, MAX_CARD_ID_LEN, "picture source ID")?;
    validate_face_field_values(&request.fields)?;
    let settings = context.network_store.load().settings().clone();
    let url = crate::server_client::server_url(
        &settings.server_url,
        &["v1", "images", &request.source_id, "face"],
    )?;
    let body = serde_json::to_vec(&ServerUpdateFaceRequest {
        fields: &request.fields,
    })
    .map_err(|_| IpcError::Internal {
        message: "the image-source settings request could not be encoded".into(),
    })?;
    let response_body = context.with_admin_token(|token| {
        let response = context
            .agent
            .put(url.as_str())
            .header("Authorization", format!("Bearer {token}"))
            .content_type("application/json")
            .send(&body)
            .map_err(|_| unreachable_server())?;
        read_image_source_response(response)
    })?;
    let descriptor: FaceDescriptor =
        serde_json::from_slice(&response_body).map_err(|_| IpcError::IncompatibleServer {
            message: "the server returned source settings this app could not read".into(),
        })?;
    validate_face_descriptor(&descriptor)?;
    Ok(descriptor)
}

fn read_image_source_response(
    mut response: ureq::http::Response<ureq::Body>,
) -> Result<Vec<u8>, IpcError> {
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_IMAGE_SOURCE_RESPONSE_BYTES.saturating_add(1) as u64)
        .read_to_vec()
        .map_err(|error| match error {
            ureq::Error::BodyExceedsLimit(_) => IpcError::RuntimeUnavailable {
                message: "the server returned an oversized image-source response".into(),
            },
            _ => IpcError::RuntimeUnavailable {
                message: "the server returned an unreadable image-source response".into(),
            },
        })?;
    if body.len() > MAX_IMAGE_SOURCE_RESPONSE_BYTES {
        return Err(IpcError::RuntimeUnavailable {
            message: "the server returned an oversized image-source response".into(),
        });
    }
    if !(200..300).contains(&status) {
        return Err(image_source_server_failure(status, &body));
    }
    Ok(body)
}

fn image_source_server_failure(status: u16, body: &[u8]) -> IpcError {
    if status == 401 {
        return IpcError::InvalidPayload {
            message: "the server rejected the admin token".into(),
        };
    }
    let detail = serde_json::from_slice::<ImageRouteErrorBody>(body)
        .ok()
        .and_then(|error| error.message)
        .filter(|message| !message.trim().is_empty());
    match status {
        404 => IpcError::NotFound {
            message: detail
                .unwrap_or_else(|| "the picture source was not found on the server".into()),
        },
        400..=499 => IpcError::InvalidPayload {
            message: detail.unwrap_or_else(|| {
                format!("the server refused the source settings (HTTP {status})")
            }),
        },
        _ => IpcError::RuntimeUnavailable {
            message: format!("the server rejected the source settings request (HTTP {status})"),
        },
    }
}

fn unreachable_server() -> IpcError {
    IpcError::RuntimeUnavailable {
        message: "the configured server could not be reached".into(),
    }
}

fn validate_face_field_values(fields: &BTreeMap<String, String>) -> Result<(), IpcError> {
    if fields.len() > MAX_FACE_FIELDS {
        return Err(IpcError::InvalidPayload {
            message: format!("a source may have at most {MAX_FACE_FIELDS} settings fields"),
        });
    }
    for (key, value) in fields {
        validate_target(key, MAX_FACE_FIELD_KEY_BYTES, "source setting key")?;
        validate_bounded(value, MAX_FACE_FIELD_VALUE_BYTES, "source setting value")?;
        if value.chars().any(char::is_control) {
            return Err(IpcError::InvalidPayload {
                message: format!("source setting {key:?} contains a control character"),
            });
        }
    }
    Ok(())
}

fn validate_image_source_descriptors(sources: &[ImageSourceDescriptor]) -> Result<(), IpcError> {
    if sources.len() > app_core::config::MAX_IMAGE_SOURCES {
        return Err(IpcError::IncompatibleServer {
            message: "the server returned more image sources than this app supports".into(),
        });
    }
    for source in sources {
        validate_target(&source.id, MAX_CARD_ID_LEN, "picture source ID")?;
        validate_target(
            &source.name,
            app_core::config::MAX_IMAGE_SOURCE_NAME_LEN,
            "picture source name",
        )?;
        if let Some(face) = &source.face {
            validate_face_descriptor(face)?;
        }
    }
    Ok(())
}

fn validate_face_descriptor(descriptor: &FaceDescriptor) -> Result<(), IpcError> {
    validate_target(&descriptor.kind, MAX_FACE_FIELD_KEY_BYTES, "face kind")?;
    validate_target(&descriptor.label, 128, "face label")?;
    if descriptor.fields.len() > MAX_FACE_FIELDS {
        return Err(IpcError::IncompatibleServer {
            message: "the server returned too many source settings fields".into(),
        });
    }
    let mut keys = HashSet::with_capacity(descriptor.fields.len());
    for field in &descriptor.fields {
        let (key, label, value, placeholder) = match field {
            FaceFieldDescriptor::Text {
                key,
                label,
                value,
                placeholder,
            }
            | FaceFieldDescriptor::Url {
                key,
                label,
                value,
                placeholder,
            } => (key, label, value, Some(placeholder)),
            FaceFieldDescriptor::Enum {
                key, label, value, ..
            } => (key, label, value, None),
        };
        validate_target(key, MAX_FACE_FIELD_KEY_BYTES, "source setting key")?;
        if !keys.insert(key) {
            return Err(IpcError::IncompatibleServer {
                message: "the server returned duplicate source setting keys".into(),
            });
        }
        validate_target(label, 128, "source setting label")?;
        validate_bounded(value, MAX_FACE_FIELD_VALUE_BYTES, "source setting value")?;
        if let Some(placeholder) = placeholder {
            validate_bounded(
                placeholder,
                MAX_FACE_FIELD_VALUE_BYTES,
                "source setting placeholder",
            )?;
        }
        if let FaceFieldDescriptor::Enum { options, value, .. } = field {
            if options.is_empty() || options.len() > MAX_FACE_FIELDS {
                return Err(IpcError::IncompatibleServer {
                    message: "the server returned an invalid segmented setting".into(),
                });
            }
            for option in options {
                validate_target(
                    &option.value,
                    MAX_FACE_FIELD_VALUE_BYTES,
                    "source setting option",
                )?;
                validate_target(&option.label, 128, "source setting option label")?;
            }
            if !options.iter().any(|option| option.value == *value) {
                return Err(IpcError::IncompatibleServer {
                    message: "the server returned an unsupported selected setting".into(),
                });
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub fn set_server_endpoint(
    state: State<'_, DesktopState>,
    request: ServerEndpointRequest,
) -> Result<NetworkSettings, IpcError> {
    set_server_endpoint_with_context(&state.network_store, request)
}

pub(super) fn set_server_endpoint_with_context(
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

pub(super) fn provision_device_with_context(
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

pub(super) fn use_local_ownership_with_context(
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

pub(super) fn save_server_config_blocking(
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

pub(super) struct PreparedServerSave {
    pub(super) url: String,
    pub(super) body: Vec<u8>,
    pub(super) save: SaveReceipt,
}

pub(super) fn prepare_server_save(
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

pub(super) fn server_error_after_local_save(error: IpcError) -> IpcError {
    error.map_message(|message| {
        format!(
            "The draft was saved on this Mac, but the server destination did not succeed: {message}"
        )
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SaveDestination {
    Local,
    Server,
}

pub(super) fn save_destination(
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
/// goes and whether the networked config projection applies -- reads this one
/// answer. Two resolutions disagreeing once let an unplugged network-owned display
/// route saves to the server while the projection treated it as possibly local.
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

/// Converts the operator-facing HTTPS base into the device's fixed WebSocket link.
/// Firmware derives HTTPS OTA routes back from the WSS authority, so paths, queries,
/// and fragments on the admin base deliberately do not cross onto the device URL.
pub(super) fn device_link_url(server_url: &str) -> Result<String, IpcError> {
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

pub(super) fn server_config_url(server_url: &str, device_id: &str) -> Result<url::Url, IpcError> {
    validate_target(device_id, MAX_DEVICE_ID_LEN, "device ID")?;
    crate::server_client::server_url(server_url, &["v1", "devices", device_id, "config"])
}

pub(super) fn put_server_config(
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

pub(super) fn validate_network_config_request(
    request: &ProvisionDeviceRequest,
) -> Result<(), IpcError> {
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
