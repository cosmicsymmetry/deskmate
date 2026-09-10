//! Producer-facing picture ingest and admin image-source lifecycle routes.

use std::sync::Arc;

use app_core::RuntimeError;
use axum::body::Bytes;
use axum::extract::rejection::JsonRejection;
use axum::extract::{DefaultBodyLimit, FromRequest, FromRequestParts, Path, Request, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::ServerState;
use crate::auth::bearer_token;
use crate::image_ingest::{ImageIngestError, canonical_frame_from_png};
use crate::image_sources::{AcceptOutcome, ImageSourceError};

const MAX_IMAGE_BODY_BYTES: usize = 1024 * 1024;

pub(crate) fn routes() -> Router<ServerState> {
    Router::new().route("/v1/images", post(mint_source)).route(
        "/v1/images/{key}",
        post(push_image)
            .layer(DefaultBodyLimit::max(MAX_IMAGE_BODY_BYTES))
            .delete(revoke_source),
    )
}

/// The existing admin extractor is private to `admin.rs`; keep the same
/// extractor position and the same constant-time verification contract here.
struct AdminAuthenticated;

impl FromRequestParts<ServerState> for AdminAuthenticated {
    type Rejection = ImageRouteError;

    #[allow(clippy::unused_async_trait_impl)]
    async fn from_request_parts(
        parts: &mut Parts,
        state: &ServerState,
    ) -> Result<Self, Self::Rejection> {
        let presented = bearer_token(parts).ok_or(ImageRouteError::AdminUnauthorized)?;
        state
            .verify_admin_token(presented)
            .then_some(Self)
            .ok_or(ImageRouteError::AdminUnauthorized)
    }
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
}

#[derive(Serialize)]
struct MintSourceResponse {
    id: String,
    token: String,
}

async fn mint_source(
    State(state): State<ServerState>,
    _admin: AdminAuthenticated,
    payload: Result<Json<MintSourceRequest>, JsonRejection>,
) -> Result<Json<MintSourceResponse>, ImageRouteError> {
    let Json(request) = payload.map_err(|rejection| ImageRouteError::InvalidJson {
        status: rejection.status(),
        message: rejection.body_text(),
    })?;
    // Plugin assets and picture frames draw from one device-wide digest budget,
    // and the registry's share is already committed, so what is left is what a
    // new source may claim.
    let available = crate::plugin_registry::MAX_DURABLE_REGISTRY_ASSETS
        .saturating_sub(state.plugins().all_assets().len());
    let minted =
        tokio::task::spawn_blocking(move || state.image_sources().mint(&request.name, available))
            .await
            .map_err(|_| ImageRouteError::WorkerFailed)?
            .map_err(|error| map_mint_error(&error))?;
    Ok(Json(MintSourceResponse {
        id: minted.id,
        token: minted.token,
    }))
}

async fn revoke_source(
    State(state): State<ServerState>,
    _admin: AdminAuthenticated,
    Path(source_id): Path<String>,
) -> Result<StatusCode, ImageRouteError> {
    tokio::task::spawn_blocking(move || state.image_sources().revoke(&source_id))
        .await
        .map_err(|_| ImageRouteError::WorkerFailed)?
        .map_err(|error| map_revoke_error(&error))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn push_image(
    State(state): State<ServerState>,
    Path(path_token): Path<String>,
    ProducerBearer(header_token): ProducerBearer,
    PngBody(body): PngBody,
) -> Result<StatusCode, ImageRouteError> {
    let source_id = authenticate_producer(&state, &path_token, header_token.as_deref())?;
    let frame = tokio::task::spawn_blocking(move || canonical_frame_from_png(&body))
        .await
        .map_err(|_| ImageRouteError::WorkerFailed)?
        .map_err(map_ingest_error)?;

    let accept_state = state.clone();
    let accepted_source_id = source_id.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        accept_state
            .image_sources()
            .accept(&accepted_source_id, frame, chrono::Utc::now())
    })
    .await
    .map_err(|_| ImageRouteError::WorkerFailed)?
    .map_err(|error| map_accept_error(&error))?;

    if let AcceptOutcome::Changed { digest } = outcome {
        let runtimes = live_runtimes(&state);
        tokio::task::spawn_blocking(move || notify_runtimes(runtimes, &source_id, digest))
            .await
            .map_err(|_| ImageRouteError::WorkerFailed)?
            .map_err(|error| map_runtime_error(&error))?;
    }

    Ok(StatusCode::OK)
}

/// Selects one carrier and performs the sole token-digest verification.
fn authenticate_producer(
    state: &ServerState,
    path_token: &str,
    header_token: Option<&str>,
) -> Result<String, ImageRouteError> {
    state
        .image_sources()
        .authenticate(header_token.unwrap_or(path_token))
        .ok_or(ImageRouteError::ProducerUnauthorized)
}

/// Snapshot live handles without holding the server's link-map lock while a
/// runtime command waits for its worker reply. Every device gets the update:
/// image sources are reusable across devices and the runtime itself decides
/// whether the visible card subscribes to this source.
fn live_runtimes(state: &ServerState) -> Vec<Arc<app_core::RuntimeHandle>> {
    state
        .inner
        .device_links
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .values()
        .filter(|link| link.is_live())
        .filter_map(|link| link.runtime())
        .collect()
}

fn notify_runtimes(
    runtimes: Vec<Arc<app_core::RuntimeHandle>>,
    source_id: &str,
    digest: [u8; protocol::ASSET_DIGEST_LEN],
) -> Result<(), RuntimeError> {
    let mut first_error = None;
    for runtime in runtimes {
        if let Err(error) = runtime.image_source_updated(source_id, digest)
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn map_ingest_error(error: ImageIngestError) -> ImageRouteError {
    match error {
        ImageIngestError::NotPng => ImageRouteError::NotPng,
        error => ImageRouteError::InvalidImage(error.to_string()),
    }
}

fn map_mint_error(error: &ImageSourceError) -> ImageRouteError {
    match error {
        // Both ceilings read the same way to the caller -- there is no room for
        // another source -- and differ only in which budget ran out, which the
        // error's own message says.
        ImageSourceError::Capacity | ImageSourceError::DigestBudget { .. } => {
            ImageRouteError::Capacity
        }
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
        ImageSourceError::Io { .. }
        | ImageSourceError::Capacity
        | ImageSourceError::DigestBudget { .. }
        | ImageSourceError::TooSoon => ImageRouteError::Internal,
    }
}

fn map_accept_error(error: &ImageSourceError) -> ImageRouteError {
    match error {
        ImageSourceError::UnknownToken => ImageRouteError::ProducerUnauthorized,
        ImageSourceError::TooSoon => ImageRouteError::RateLimited,
        // Neither ceiling is reachable on an accept: minting already refused
        // the source that would have exceeded one.
        ImageSourceError::Io { .. }
        | ImageSourceError::Capacity
        | ImageSourceError::DigestBudget { .. } => ImageRouteError::Internal,
    }
}

fn map_runtime_error(error: &RuntimeError) -> ImageRouteError {
    let status = match error {
        RuntimeError::QueueFull | RuntimeError::WorkerStopped => StatusCode::SERVICE_UNAVAILABLE,
        RuntimeError::ResponseTimeout => StatusCode::GATEWAY_TIMEOUT,
        _ => StatusCode::BAD_GATEWAY,
    };
    ImageRouteError::Runtime { status }
}

#[derive(Debug)]
enum ImageRouteError {
    AdminUnauthorized,
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
    Runtime { status: StatusCode },
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
    Runtime { message: &'a str },
    Internal,
}

impl IntoResponse for ImageRouteError {
    fn into_response(self) -> Response {
        match self {
            Self::AdminUnauthorized => StatusCode::UNAUTHORIZED.into_response(),
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
            Self::Runtime { status } => (
                status,
                Json(ErrorBody::Runtime {
                    message: "the picture was stored, but the display update could not be queued",
                }),
            )
                .into_response(),
            Self::Internal | Self::WorkerFailed => {
                (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorBody::Internal)).into_response()
            }
        }
    }
}
