use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::ServerState;
use crate::app_api::DeviceRow;
use crate::identity::{AccountId, DeviceState, IdentityError};
use crate::web_auth::{AccountSession, clear_session_cookie};

const PENDING_TTL: chrono::Duration = chrono::Duration::hours(24);
const HOUSEKEEPING_INTERVAL: Duration = Duration::from_hours(1);

pub(crate) fn routes() -> Router<ServerState> {
    Router::new()
        .route("/v1/app/devices", get(list_devices))
        .route("/v1/app/devices/claim", post(claim_device))
        .route("/v1/app/devices/{id}", delete(remove_device))
}

pub(crate) async fn list_devices(
    State(state): State<ServerState>,
    session: AccountSession,
) -> Result<Json<Vec<DeviceRow>>, ClaimError> {
    let account_id = session.account.id;
    let lookup = state.clone();
    let stored = tokio::task::spawn_blocking(move || {
        let space = lookup.account_space(&account_id);
        let owners = lookup.identity().devices_for(&account_id)?;
        let rows = owners
            .into_iter()
            .map(|owner| {
                let path = space
                    .configs
                    .for_device(&owner.device_id)
                    .store
                    .path()
                    .to_path_buf();
                let configured_at = std::fs::metadata(&path)
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .and_then(|modified| {
                        modified
                            .duration_since(std::time::UNIX_EPOCH)
                            .ok()
                            .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok())
                    });
                (owner.device_id, owner.state, path.exists(), configured_at)
            })
            .collect::<Vec<_>>();
        Ok::<_, IdentityError>(rows)
    })
    .await
    .map_err(|_| ClaimError::WorkerFailed)??;
    let rows = stored
        .into_iter()
        .map(
            |(id, state_value, has_saved_config, configured_at)| DeviceRow {
                connected: state.device_is_linked(&id),
                id,
                has_saved_config,
                configured_at,
                state: state_value.as_str(),
            },
        )
        .collect();
    Ok(Json(rows))
}

#[derive(Serialize)]
struct ClaimResponse {
    device_id: String,
    token: String,
    link_url: String,
}

async fn claim_device(
    State(state): State<ServerState>,
    session: AccountSession,
) -> Result<(StatusCode, Json<ClaimResponse>), ClaimError> {
    let account_id = session.account.id;
    let link_url = device_link_url(state.public_url())?;
    let claim_state = state.clone();
    let identity = tokio::task::spawn_blocking(move || {
        claim_state.with_device_lifecycle(|| {
            let owned = claim_state.identity().devices_for(&account_id)?;
            if claim_state
                .entitlements()
                .max_panels(&account_id)
                .is_some_and(|limit| owned.len() >= limit)
            {
                return Err(ClaimError::Conflict(
                    "Your account has reached its panel limit.",
                ));
            }
            let identity = claim_state.registry().mint()?;
            if let Err(error) = claim_state.identity().assign_device(
                &identity.device_id,
                &account_id,
                DeviceState::Pending,
                Utc::now(),
            ) {
                if let Err(revoke_error) = claim_state.registry().revoke(&identity.device_id) {
                    tracing::error!(
                        device_id = %identity.device_id,
                        %revoke_error,
                        "failed to roll back a device identity after claiming failed"
                    );
                }
                return Err(error.into());
            }
            Ok(identity)
        })
    })
    .await
    .map_err(|_| ClaimError::WorkerFailed)??;
    Ok((
        StatusCode::CREATED,
        Json(ClaimResponse {
            device_id: identity.device_id,
            token: identity.token,
            link_url,
        }),
    ))
}

async fn remove_device(
    State(state): State<ServerState>,
    session: AccountSession,
    Path(device_id): Path<String>,
) -> Result<StatusCode, ClaimError> {
    let account_id = session.account.id;
    tokio::task::spawn_blocking(move || {
        state.with_device_lifecycle(|| remove_owned_device_data(&state, &account_id, &device_id))
    })
    .await
    .map_err(|_| ClaimError::WorkerFailed)??;
    Ok(StatusCode::NO_CONTENT)
}

fn remove_owned_device_data(
    state: &ServerState,
    account_id: &AccountId,
    device_id: &str,
) -> Result<(), ClaimError> {
    let owner = state.identity().device_owner(device_id)?;
    if !owner.is_some_and(|owner| &owner.account_id == account_id) {
        return Err(ClaimError::NotFound);
    }
    state.registry().revoke(device_id)?;
    state.identity().release_device(device_id)?;
    let config = state
        .account_space(account_id)
        .configs
        .for_device(device_id)
        .store
        .path()
        .to_path_buf();
    let remove_result = remove_file_if_present(&config);
    state.close_link(device_id);
    remove_result?;
    Ok(())
}

pub(crate) async fn delete_account(
    State(state): State<ServerState>,
    session: AccountSession,
) -> Result<Response, ClaimError> {
    let account = session.account;
    tokio::task::spawn_blocking(move || {
        state.with_device_lifecycle(|| {
            delete_account_data(&state, &account.id, account.is_instance_owner)
        })
    })
    .await
    .map_err(|_| ClaimError::WorkerFailed)??;
    Ok((
        [(header::SET_COOKIE, clear_session_cookie())],
        StatusCode::NO_CONTENT,
    )
        .into_response())
}

fn delete_account_data(
    state: &ServerState,
    account_id: &AccountId,
    is_instance_owner: bool,
) -> Result<(), ClaimError> {
    if is_instance_owner && state.identity().account_count()? > 1 {
        return Err(ClaimError::Conflict(
            "Other accounts use this server. Remove them first.",
        ));
    }
    let devices = state.identity().devices_for(account_id)?;
    for device in &devices {
        state.registry().revoke(&device.device_id)?;
    }
    for device in &devices {
        state.close_link(&device.device_id);
    }

    let root = state.account_root(account_id);
    if let Some(space) = state.drop_account_space(account_id) {
        crate::data_cards::stop_account_refreshers(&space);
    }
    state.identity().delete_account(account_id)?;
    {
        let _guard = state
            .inner
            .setup_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.identity().account_count()? == 0 {
            let code = state.inner.setup_code.regenerate();
            crate::web_auth::setup_code::announce_setup_code(state.public_url(), &code);
        }
    }
    remove_directory_if_present(&root)?;
    Ok(())
}

#[doc(hidden)]
pub async fn collect_stale_claims(state: &ServerState, now: DateTime<Utc>) -> usize {
    let cutoff = now - PENDING_TTL;
    let lookup = state.clone();
    match tokio::task::spawn_blocking(move || {
        lookup.with_device_lifecycle(|| {
            let candidates = lookup.identity().stale_pending(cutoff)?;
            let collected = candidates.len();
            for device_id in candidates {
                lookup.registry().revoke(&device_id)?;
                lookup.identity().release_device(&device_id)?;
                lookup.close_link(&device_id);
            }
            Ok::<_, ClaimError>(collected)
        })
    })
    .await
    {
        Ok(Ok(collected)) => collected,
        Ok(Err(error)) => {
            tracing::error!(%error, "stale pending claims could not be collected");
            0
        }
        Err(error) => {
            tracing::error!(%error, "stale pending claim worker failed");
            0
        }
    }
}

#[doc(hidden)]
pub fn spawn_housekeeping(state: ServerState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut shutdown = state.subscribe_shutdown();
        if *shutdown.borrow() {
            return;
        }
        loop {
            tokio::select! {
                () = tokio::time::sleep(HOUSEKEEPING_INTERVAL) => {
                    collect_stale_claims(&state, Utc::now()).await;
                }
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        return;
                    }
                }
            }
        }
    })
}

fn device_link_url(public_url: &url::Url) -> Result<String, ClaimError> {
    let mut link = public_url.clone();
    let scheme = match link.scheme() {
        "https" => "wss",
        "http" => "ws",
        _ => return Err(ClaimError::Internal),
    };
    link.set_scheme(scheme).map_err(|()| ClaimError::Internal)?;
    link.set_path("/v1/device/link");
    link.set_query(None);
    link.set_fragment(None);
    Ok(link.into())
}

fn remove_file_if_present(path: &std::path::Path) -> Result<(), ClaimError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ClaimError::Filesystem(error.to_string())),
    }
}

fn remove_directory_if_present(path: &std::path::Path) -> Result<(), ClaimError> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ClaimError::Filesystem(error.to_string())),
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ClaimError {
    #[error("resource not found")]
    NotFound,
    #[error("{0}")]
    Conflict(&'static str),
    #[error(transparent)]
    Identity(#[from] IdentityError),
    #[error(transparent)]
    Registry(#[from] crate::registry::RegistryError),
    #[error("filesystem operation failed: {0}")]
    Filesystem(String),
    #[error("worker task failed")]
    WorkerFailed,
    #[error("internal error")]
    Internal,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: &'static str,
}

impl IntoResponse for ClaimError {
    fn into_response(self) -> Response {
        match self {
            Self::NotFound => StatusCode::NOT_FOUND.into_response(),
            Self::Conflict(message) => {
                (StatusCode::CONFLICT, Json(ErrorResponse { error: message })).into_response()
            }
            error => {
                tracing::error!(%error, "claim route failed");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removing_an_owned_device_revokes_registry_and_ownership() {
        let state = ServerState::in_memory();
        let account = state
            .identity()
            .create_account("owner@example.com", true, true, Utc::now())
            .unwrap();
        let device = state.registry().mint().unwrap();
        state
            .identity()
            .assign_device(
                &device.device_id,
                &account.id,
                DeviceState::Active,
                Utc::now(),
            )
            .unwrap();

        remove_owned_device_data(&state, &account.id, &device.device_id).unwrap();

        assert!(!state.registry().contains_device(&device.device_id));
        assert!(
            state
                .identity()
                .device_owner(&device.device_id)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn removing_another_accounts_device_is_not_an_identity_oracle() {
        let state = ServerState::in_memory();
        let owner = state
            .identity()
            .create_account("owner@example.com", true, true, Utc::now())
            .unwrap();
        let other = state
            .identity()
            .create_account("other@example.com", true, false, Utc::now())
            .unwrap();
        let device = state.registry().mint().unwrap();
        state
            .identity()
            .assign_device(
                &device.device_id,
                &owner.id,
                DeviceState::Active,
                Utc::now(),
            )
            .unwrap();

        assert!(matches!(
            remove_owned_device_data(&state, &other.id, &device.device_id),
            Err(ClaimError::NotFound)
        ));
        assert!(state.registry().contains_device(&device.device_id));
    }

    #[tokio::test]
    async fn housekeeping_stops_when_shutdown_already_began() {
        let state = ServerState::in_memory();
        state.begin_shutdown();
        tokio::time::timeout(Duration::from_secs(1), spawn_housekeeping(state))
            .await
            .expect("housekeeping ignored shutdown")
            .expect("housekeeping task panicked");
    }
}
