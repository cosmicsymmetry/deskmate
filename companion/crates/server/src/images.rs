//! Producer-facing picture ingest and admin image-source lifecycle routes.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::rejection::JsonRejection;
use axum::extract::{DefaultBodyLimit, FromRequest, FromRequestParts, Path, Request, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::{AccountSpace, ImageNotificationOrigin, ServerState};
// These account routes accept the browser's session cookie. The admin bearer
// is deliberately not a browser session.
//
// The producer push route below is deliberately NOT covered by this: it
// authenticates a per-source producer credential, which is a different and much
// narrower thing than an operator.
use crate::auth::bearer_token;
use crate::image_ingest::{ImageIngestError, canonical_frame_from_png};
use crate::image_sources::{AcceptOutcome, ImageSourceError};
use crate::web_auth::AccountSession;

const MAX_IMAGE_BODY_BYTES: usize = 1024 * 1024;
const MAX_FACE_SETTINGS_BODY_BYTES: usize = 16 * 1024;

pub(crate) fn routes() -> Router<ServerState> {
    Router::new()
        .route("/v1/images", get(list_sources).post(mint_source))
        .route("/v1/faces", get(list_creatable_faces))
        .route(
            "/v1/images/{key}",
            post(push_image)
                .layer(DefaultBodyLimit::max(MAX_IMAGE_BODY_BYTES))
                .delete(revoke_source),
        )
        .route(
            "/v1/images/{id}/face",
            put(update_face).layer(DefaultBodyLimit::max(MAX_FACE_SETTINGS_BODY_BYTES)),
        )
}

/// Copies only a well-formed bearer credential from the request parts. The
/// path carrier stays available when the header is absent or malformed, and
/// the store still performs exactly one digest comparison for either form.
struct ProducerBearer(Option<String>);

impl FromRequestParts<ServerState> for ProducerBearer {
    type Rejection = std::convert::Infallible;

    #[allow(clippy::unused_async_trait_impl)]
    async fn from_request_parts(
        parts: &mut Parts,
        _state: &ServerState,
    ) -> Result<Self, Self::Rejection> {
        Ok(Self(bearer_token(parts).map(str::to_owned)))
    }
}

/// Checks the media type before asking Axum's bounded [`Bytes`] extractor to
/// touch the body. `DefaultBodyLimit` on the producer route supplies the
/// one-mebibyte ceiling that `Bytes::from_request` enforces.
struct PngBody(Bytes);

impl FromRequest<ServerState> for PngBody {
    type Rejection = ImageRouteError;

    async fn from_request(request: Request, state: &ServerState) -> Result<Self, Self::Rejection> {
        if !is_png_content_type(request.headers()) {
            return Err(ImageRouteError::UnsupportedMediaType);
        }
        Bytes::from_request(request, state)
            .await
            .map(Self)
            .map_err(|rejection| {
                if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    ImageRouteError::PayloadTooLarge
                } else {
                    ImageRouteError::BodyUnavailable
                }
            })
    }
}

fn is_png_content_type(headers: &HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|media_type| media_type.trim().eq_ignore_ascii_case("image/png"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MintSourceRequest {
    name: String,
    /// When present, the minted source is a face this server draws itself rather
    /// than one an external producer pushes to. The companion sends this when the
    /// owner picks "Weather" from the add menu, so that one click yields a card
    /// that is already wired up.
    #[serde(default)]
    face_kind: Option<String>,
}

#[derive(Serialize)]
struct MintSourceResponse {
    id: String,
    token: String,
}

#[derive(Serialize)]
struct ImageSourceDescriptor {
    id: String,
    name: String,
    face: Option<crate::data_cards::FaceDescriptor>,
    /// Null exactly when `face` is: an external producer's source has no server
    /// face to report on.
    face_status: Option<crate::data_cards::FaceStatus>,
}

#[cfg(test)]
pub(super) fn contract_mint_source_responses() -> Vec<serde_json::Value> {
    vec![
        serde_json::to_value(MintSourceResponse {
            id: "fixture-source".into(),
            token: "fixture-token".into(),
        })
        .expect("mint source response serializes"),
    ]
}

#[cfg(test)]
pub(super) fn contract_image_source_descriptors() -> Vec<serde_json::Value> {
    let face = crate::data_cards::creatable_faces(&ServerState::in_memory())
        .into_iter()
        .next()
        .expect("the server exposes at least one creatable face");
    [
        ImageSourceDescriptor {
            id: "external-source".into(),
            name: "External picture".into(),
            face: None,
            face_status: None,
        },
        ImageSourceDescriptor {
            id: "weather-source".into(),
            name: "Weather".into(),
            face: Some(face.clone()),
            face_status: Some(crate::data_cards::FaceStatus {
                state: crate::data_cards::FaceState::NeedsSettings,
                message: None,
                at_unix_seconds: None,
            }),
        },
        ImageSourceDescriptor {
            id: "token-source".into(),
            name: "Token price".into(),
            face: Some(face),
            face_status: Some(crate::data_cards::FaceStatus {
                state: crate::data_cards::FaceState::NeedsAttention,
                message: Some("the token was not found; check the coin ID".into()),
                at_unix_seconds: Some(1_790_000_000),
            }),
        },
    ]
    .into_iter()
    .map(|descriptor| serde_json::to_value(descriptor).expect("image source descriptor serializes"))
    .collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateFaceRequest {
    fields: BTreeMap<String, String>,
}

async fn list_sources(
    State(state): State<ServerState>,
    session: AccountSession,
) -> Result<Json<Vec<ImageSourceDescriptor>>, ImageRouteError> {
    let space = account_space(&state, &session).await?;
    let sources = space
        .image_sources
        .summaries(chrono::Utc::now())
        .into_iter()
        .map(|source| ImageSourceDescriptor {
            face: crate::data_cards::descriptor_for_source(&state, &space, &source.id),
            face_status: crate::data_cards::status_for_source(&state, &space, &source.id),
            id: source.id,
            name: source.name,
        })
        .collect();
    Ok(Json(sources))
}

async fn update_face(
    State(state): State<ServerState>,
    session: AccountSession,
    Path(source_id): Path<String>,
    payload: Result<Json<UpdateFaceRequest>, JsonRejection>,
) -> Result<Json<crate::data_cards::FaceDescriptor>, ImageRouteError> {
    let space = account_space(&state, &session).await?;
    let source_exists = space
        .image_sources
        .summaries(chrono::Utc::now())
        .iter()
        .any(|source| source.id == source_id);
    if !source_exists {
        return Err(ImageRouteError::NotFound);
    }
    let Json(request) = payload.map_err(|rejection| ImageRouteError::InvalidJson {
        status: rejection.status(),
        message: rejection.body_text(),
    })?;
    let runtime = tokio::runtime::Handle::current();
    let update_state = state.clone();
    let update_space = Arc::clone(&space);
    let updated = tokio::task::spawn_blocking(move || {
        crate::data_cards::update_face_fields(
            &update_state,
            &update_space,
            &runtime,
            &source_id,
            &request.fields,
        )
    })
    .await
    .map_err(|_| ImageRouteError::WorkerFailed)?
    .map_err(|error| map_face_update_error(&error))?;
    Ok(Json(updated))
}

/// The faces this server can draw, for the companion's add menu.
///
/// The app renders this list verbatim -- it is what lets "Weather" appear in the
/// window without the app knowing what weather is.
async fn list_creatable_faces(
    State(state): State<ServerState>,
    _session: AccountSession,
) -> Json<Vec<crate::data_cards::FaceDescriptor>> {
    Json(crate::data_cards::creatable_faces(&state))
}

async fn mint_source(
    State(state): State<ServerState>,
    session: AccountSession,
    payload: Result<Json<MintSourceRequest>, JsonRejection>,
) -> Result<Json<MintSourceResponse>, ImageRouteError> {
    let Json(request) = payload.map_err(|rejection| ImageRouteError::InvalidJson {
        status: rejection.status(),
        message: rejection.body_text(),
    })?;
    let face_kind = request.face_kind.clone();
    let space = account_space(&state, &session).await?;
    let maximum = state
        .entitlements()
        .max_image_sources(&space.account_id)
        .min(app_core::config::MAX_IMAGE_SOURCES);
    // Only a plan stricter than the store speaks for itself; at the store's own
    // ceiling the store answers, so self-hosting keeps its existing contract.
    if maximum < app_core::config::MAX_IMAGE_SOURCES
        && space.image_sources.summaries(chrono::Utc::now()).len() >= maximum
    {
        return Err(ImageRouteError::EntitlementCapacity { maximum });
    }
    let mint_state = state.clone();
    let mint_space = Arc::clone(&space);
    let minted = tokio::task::spawn_blocking(move || {
        let _ = mint_state;
        mint_space.image_sources.mint(&request.name)
    })
    .await
    .map_err(|_| ImageRouteError::WorkerFailed)?
    .map_err(|error| map_mint_error(&error))?;

    // A server-drawn face is attached in the same request. If attaching fails the
    // source is revoked rather than left behind: a half-made source shows up in
    // the add menu as something the owner never asked for and cannot explain.
    if let Some(kind) = face_kind {
        let face_state = state.clone();
        let face_space = Arc::clone(&space);
        let source_id = minted.id.clone();
        let attached = tokio::task::spawn_blocking(move || {
            crate::data_cards::create_face(&face_state, &face_space, &source_id, &kind)
        })
        .await
        .map_err(|_| ImageRouteError::WorkerFailed)?;
        if let Err(error) = attached {
            let revoke_state = state.clone();
            let revoke_space = Arc::clone(&space);
            let orphan = minted.id.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let _ = revoke_state;
                revoke_space.image_sources.revoke(&orphan)
            })
            .await;
            return Err(map_face_update_error(&error));
        }
    }

    Ok(Json(MintSourceResponse {
        id: minted.id,
        token: minted.token,
    }))
}

async fn revoke_source(
    State(state): State<ServerState>,
    session: AccountSession,
    Path(source_id): Path<String>,
) -> Result<StatusCode, ImageRouteError> {
    let space = account_space(&state, &session).await?;
    revoke_image_source(state, space, source_id)
        .await
        .map_err(|error| match error {
            RevokeImageSourceError::WorkerFailed => ImageRouteError::WorkerFailed,
            RevokeImageSourceError::Source(error) => map_revoke_error(&error),
            RevokeImageSourceError::Face(error) => map_face_update_error(&error),
        })?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum RevokeImageSourceError {
    #[error("a worker task failed")]
    WorkerFailed,
    #[error(transparent)]
    Source(ImageSourceError),
    #[error(transparent)]
    Face(crate::data_cards::FaceUpdateError),
}

pub(crate) async fn revoke_image_source(
    state: ServerState,
    space: Arc<AccountSpace>,
    source_id: String,
) -> Result<(), RevokeImageSourceError> {
    let revoking = Arc::clone(&space);
    let id = source_id.clone();
    tokio::task::spawn_blocking(move || revoking.image_sources.revoke(&id))
        .await
        .map_err(|_| RevokeImageSourceError::WorkerFailed)?
        .map_err(RevokeImageSourceError::Source)?;

    // Keep face removal in its own awaited phase. If the request is cancelled
    // while source persistence is pending, this write must never begin.
    tokio::task::spawn_blocking(move || {
        let _ = state;
        crate::data_cards::remove_face(&space, &source_id)
    })
    .await
    .map_err(|_| RevokeImageSourceError::WorkerFailed)?
    .map_err(RevokeImageSourceError::Face)
}

async fn push_image(
    State(state): State<ServerState>,
    Path(path_token): Path<String>,
    ProducerBearer(header_token): ProducerBearer,
    PngBody(body): PngBody,
) -> Result<StatusCode, ImageRouteError> {
    let (space, source_id) =
        authenticate_producer(&state, &path_token, header_token.as_deref()).await?;
    let frame = tokio::task::spawn_blocking(move || canonical_frame_from_png(&body))
        .await
        .map_err(|_| ImageRouteError::WorkerFailed)?
        .map_err(map_ingest_error)?;

    let accept_space = Arc::clone(&space);
    let accepted_source_id = source_id.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        accept_space
            .image_sources
            .accept(&accepted_source_id, frame, chrono::Utc::now())
    })
    .await
    .map_err(|_| ImageRouteError::WorkerFailed)?
    .map_err(|error| map_accept_error(&error))?;

    if let AcceptOutcome::Changed { digest } = outcome {
        // Deliberately not awaited. The producer's durable outcome is "the
        // frame is stored", and it is stored by the time we get here: delivery
        // to the device is this server's business, over a link the producer has
        // no relationship with and cannot act on.
        //
        // Awaiting it made a successful push answer 504. The reconcile drives a
        // full device synchronize -- chunk transfers plus AssetRelease's own
        // twenty-second budget -- so a slow or flapping link turned a stored
        // frame into an error for somebody holding a curl command, who would
        // then reasonably retry a push that had already succeeded.
        //
        // A failure here is not lost: it surfaces as a card error on the
        // device's own snapshot, which is where a device-delivery problem
        // belongs, and the next full synchronize reconciles the frame anyway.
        state.notify_image_source_changed(
            &space.account_id,
            source_id,
            digest,
            ImageNotificationOrigin::ExternalProducerPush,
        );
    }

    Ok(StatusCode::OK)
}

/// Selects one carrier and performs the sole token-digest verification.
async fn authenticate_producer(
    state: &ServerState,
    path_token: &str,
    header_token: Option<&str>,
) -> Result<(Arc<AccountSpace>, String), ImageRouteError> {
    let token = header_token.unwrap_or(path_token).to_owned();
    let lookup = state.clone();
    tokio::task::spawn_blocking(move || lookup.authenticate_image_producer(&token))
        .await
        .map_err(|_| ImageRouteError::WorkerFailed)?
        .ok_or(ImageRouteError::ProducerUnauthorized)
}

async fn account_space(
    state: &ServerState,
    session: &AccountSession,
) -> Result<Arc<AccountSpace>, ImageRouteError> {
    let lookup = state.clone();
    let account_id = session.account.id.clone();
    tokio::task::spawn_blocking(move || lookup.account_space(&account_id))
        .await
        .map_err(|_| ImageRouteError::WorkerFailed)
}

fn map_ingest_error(error: ImageIngestError) -> ImageRouteError {
    match error {
        ImageIngestError::NotPng => ImageRouteError::NotPng,
        error => ImageRouteError::InvalidImage(error.to_string()),
    }
}

fn map_mint_error(error: &ImageSourceError) -> ImageRouteError {
    match error {
        ImageSourceError::Capacity => ImageRouteError::Capacity,
        // Neither of these can reach a mint; they are folded in so the match
        // stays exhaustive without a wildcard that would hide a new variant.
        ImageSourceError::Io { .. }
        | ImageSourceError::UnknownToken
        | ImageSourceError::TooSoon => ImageRouteError::Internal,
    }
}

fn map_revoke_error(error: &ImageSourceError) -> ImageRouteError {
    match error {
        ImageSourceError::UnknownToken => ImageRouteError::NotFound,
        ImageSourceError::Io { .. } | ImageSourceError::Capacity | ImageSourceError::TooSoon => {
            ImageRouteError::Internal
        }
    }
}

fn map_accept_error(error: &ImageSourceError) -> ImageRouteError {
    match error {
        ImageSourceError::UnknownToken => ImageRouteError::ProducerUnauthorized,
        ImageSourceError::TooSoon => ImageRouteError::RateLimited,
        // The source ceiling is unreachable on an accept: minting already
        // refused the source that would have exceeded it.
        ImageSourceError::Io { .. } | ImageSourceError::Capacity => ImageRouteError::Internal,
    }
}

fn map_face_update_error(error: &crate::data_cards::FaceUpdateError) -> ImageRouteError {
    match error {
        crate::data_cards::FaceUpdateError::NoFace => ImageRouteError::FaceNotConfigurable,
        crate::data_cards::FaceUpdateError::UnknownField(_)
        | crate::data_cards::FaceUpdateError::InvalidField { .. } => {
            ImageRouteError::InvalidFaceFields(error.to_string())
        }
        crate::data_cards::FaceUpdateError::Encode(_)
        | crate::data_cards::FaceUpdateError::Persist(_) => ImageRouteError::Internal,
    }
}

#[derive(Debug)]
enum ImageRouteError {
    ProducerUnauthorized,
    NotFound,
    InvalidJson { status: StatusCode, message: String },
    UnsupportedMediaType,
    PayloadTooLarge,
    BodyUnavailable,
    NotPng,
    InvalidImage(String),
    RateLimited,
    Capacity,
    EntitlementCapacity { maximum: usize },
    FaceNotConfigurable,
    InvalidFaceFields(String),
    Internal,
    WorkerFailed,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum ErrorBody<'a> {
    Unauthorized { message: &'a str },
    InvalidJson { message: &'a str },
    UnsupportedMediaType { message: &'a str },
    PayloadTooLarge { message: &'a str },
    InvalidBody { message: &'a str },
    InvalidImage { message: &'a str },
    RateLimited { message: &'a str },
    Capacity { message: &'a str },
    FaceNotConfigurable { message: &'a str },
    InvalidFaceFields { message: &'a str },
    Internal,
}

impl IntoResponse for ImageRouteError {
    fn into_response(self) -> Response {
        match self {
            Self::ProducerUnauthorized => (
                StatusCode::UNAUTHORIZED,
                Json(ErrorBody::Unauthorized {
                    message: "the image source token is missing, unknown, or revoked",
                }),
            )
                .into_response(),
            Self::NotFound => StatusCode::NOT_FOUND.into_response(),
            Self::InvalidJson { status, message } => {
                (status, Json(ErrorBody::InvalidJson { message: &message })).into_response()
            }
            Self::UnsupportedMediaType => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                Json(ErrorBody::UnsupportedMediaType {
                    message: "send a PNG with Content-Type image/png",
                }),
            )
                .into_response(),
            Self::PayloadTooLarge => (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(ErrorBody::PayloadTooLarge {
                    message: "the PNG must be no larger than 1 MiB",
                }),
            )
                .into_response(),
            Self::BodyUnavailable => (
                StatusCode::BAD_REQUEST,
                Json(ErrorBody::InvalidBody {
                    message: "the PNG body could not be read",
                }),
            )
                .into_response(),
            Self::NotPng => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                Json(ErrorBody::UnsupportedMediaType {
                    message: "the body is not a PNG",
                }),
            )
                .into_response(),
            Self::InvalidImage(message) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(ErrorBody::InvalidImage { message: &message }),
            )
                .into_response(),
            Self::RateLimited => (
                StatusCode::TOO_MANY_REQUESTS,
                Json(ErrorBody::RateLimited {
                    message: "wait at least 5 seconds before pushing this source again",
                }),
            )
                .into_response(),
            Self::Capacity => (
                StatusCode::CONFLICT,
                Json(ErrorBody::Capacity {
                    message: "the image-source capacity has been reached",
                }),
            )
                .into_response(),
            // The same status and body kind as the store's own ceiling: a caller
            // sees one capacity contract whether the plan or the store said no.
            Self::EntitlementCapacity { maximum } => (
                StatusCode::CONFLICT,
                Json(ErrorBody::Capacity {
                    message: &format!(
                        "This account can have at most {maximum} picture sources."
                    ),
                }),
            )
                .into_response(),
            Self::FaceNotConfigurable => (
                StatusCode::CONFLICT,
                Json(ErrorBody::FaceNotConfigurable {
                    message: "this image source is fed by an external producer and has no server settings",
                }),
            )
                .into_response(),
            Self::InvalidFaceFields(message) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(ErrorBody::InvalidFaceFields { message: &message }),
            )
                .into_response(),
            Self::Internal | Self::WorkerFailed => {
                (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorBody::Internal)).into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::future;
    use std::path::Path;

    use futures_util::FutureExt as _;

    use super::*;
    use crate::firmware::FirmwareCatalog;

    fn state() -> (tempfile::TempDir, ServerState) {
        let root = tempfile::tempdir().expect("config root");
        let state = ServerState::new(
            "admin token".into(),
            FirmwareCatalog::in_memory(),
            root.path().to_path_buf(),
        );
        state
            .identity()
            .create_account("owner@example.com", true, true, chrono::Utc::now())
            .expect("owner account");
        (root, state)
    }

    fn mint_face(
        state: &ServerState,
        name: &str,
        kind: &str,
    ) -> crate::image_sources::MintedSource {
        let space = state.account_space(&state.instance_owner().expect("instance owner").id);
        let source = space.image_sources.mint(name).expect("mint source");
        crate::data_cards::create_face(state, &space, &source.id, kind).expect("create face");
        source
    }

    fn replace_file_with_directory(path: &Path) -> std::path::PathBuf {
        let backup = path.with_extension("backup");
        std::fs::rename(path, &backup).expect("preserve original file");
        std::fs::create_dir(path).expect("blocking directory");
        backup
    }

    struct NotifyOnDrop(Option<tokio::sync::oneshot::Sender<()>>);

    impl Drop for NotifyOnDrop {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }

    #[tokio::test]
    async fn helper_revokes_the_source_removes_only_its_face_and_cancels_its_task() {
        let (_root, state) = state();
        let space = state.account_space(&state.instance_owner().unwrap().id);
        let removed = mint_face(&state, "Weather", "weather");
        let retained = mint_face(&state, "News", "rss");
        let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _notify = NotifyOnDrop(Some(dropped_tx));
            future::pending::<()>().await;
        });
        space
            .data_cards
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert_task_for_test(removed.id.clone(), task);

        revoke_image_source(state.clone(), Arc::clone(&space), removed.id.clone())
            .await
            .expect("revoke source and face");

        assert_eq!(space.image_sources.authenticate(&removed.token), None);
        {
            let data_cards = space
                .data_cards
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            assert_eq!(
                data_cards.spec_source_ids_for_test().collect::<Vec<_>>(),
                [retained.id.as_str()]
            );
            assert!(!data_cards.has_task_for_test(&removed.id));
        }
        tokio::time::timeout(std::time::Duration::from_secs(1), dropped_rx)
            .await
            .expect("removed refresher was cancelled")
            .expect("drop notification");
    }

    #[tokio::test]
    async fn source_persistence_failure_skips_face_removal() {
        let (_root, state) = state();
        let space = state.account_space(&state.instance_owner().unwrap().id);
        let source = mint_face(&state, "Weather", "weather");
        let spec_path = space.root.join("data-cards.json");
        let before = std::fs::read(&spec_path).expect("face specs");
        replace_file_with_directory(
            &space
                .root
                .join(crate::image_sources::IMAGE_SOURCE_STORE_FILE),
        );

        let error = revoke_image_source(state.clone(), Arc::clone(&space), source.id.clone())
            .await
            .expect_err("source persistence must fail");

        assert!(matches!(error, RevokeImageSourceError::Source(_)));
        assert_eq!(
            space.image_sources.authenticate(&source.token),
            Some(source.id.clone())
        );
        assert_eq!(std::fs::read(&spec_path).expect("unchanged specs"), before);
        assert!(crate::data_cards::descriptor_for_source(&state, &space, &source.id).is_some());
    }

    #[tokio::test]
    async fn face_persistence_failure_leaves_the_source_revoked_and_face_state_unchanged() {
        let (_root, state) = state();
        let space = state.account_space(&state.instance_owner().unwrap().id);
        let source = mint_face(&state, "Weather", "weather");
        let spec_path = space.root.join("data-cards.json");
        let backup = replace_file_with_directory(&spec_path);
        let before = std::fs::read(&backup).expect("preserved specs");

        let error = revoke_image_source(state.clone(), Arc::clone(&space), source.id.clone())
            .await
            .expect_err("face persistence must fail");

        assert!(matches!(error, RevokeImageSourceError::Face(_)));
        assert_eq!(space.image_sources.authenticate(&source.token), None);
        assert_eq!(std::fs::read(&backup).expect("unchanged backup"), before);
        assert!(spec_path.is_dir(), "the failing target was not replaced");
        assert!(crate::data_cards::descriptor_for_source(&state, &space, &source.id).is_some());
    }

    #[test]
    fn cancellation_while_source_revocation_is_pending_never_starts_face_removal() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .max_blocking_threads(1)
            .enable_all()
            .build()
            .expect("test runtime");
        runtime.block_on(async {
            let (_root, state) = state();
            let space = state.account_space(&state.instance_owner().unwrap().id);
            let source = mint_face(&state, "Weather", "weather");
            let (started_tx, started_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            let blocker = tokio::task::spawn_blocking(move || {
                let _ = started_tx.send(());
                release_rx.recv().expect("release blocking worker");
            });
            started_rx.await.expect("blocking worker started");

            let operation =
                revoke_image_source(state.clone(), Arc::clone(&space), source.id.clone());
            assert!(
                operation.now_or_never().is_none(),
                "source revocation was not pending at its first await"
            );
            release_tx.send(()).expect("release worker");
            blocker.await.expect("blocking worker exits");
            tokio::time::timeout(std::time::Duration::from_secs(1), async {
                while space.image_sources.authenticate(&source.token).is_some() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("detached source revocation completed");

            assert!(
                crate::data_cards::descriptor_for_source(&state, &space, &source.id).is_some(),
                "the face phase began after its parent operation was cancelled"
            );
        });
    }
}
