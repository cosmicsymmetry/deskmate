//! The operator's management surface for networked devices and integrations.
//!
//! Browser-facing, and therefore separate from the `/v1` JSON API: these
//! handlers render HTML and redirect, while the API keeps its status codes for
//! machines. Where an action exists on both, the behaviour lives in one shared
//! function in `oauth::routes` and both call it, so the two paths cannot drift.

pub(crate) mod view;

use axum::Form;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use chrono::Utc;
use serde::Deserialize;

use crate::ServerState;
use crate::oauth::routes::{SESSION_TTL, mint_producer_action, revoke_integration_action};
use crate::oauth::session::{OperatorAuthenticated, SessionSigner, set_cookie_header};
use view::{DashboardModel, DeviceRow, IntegrationRow, SourceRow};

pub(crate) fn routes() -> Router<ServerState> {
    Router::new()
        .route("/v1/manage", get(dashboard))
        .route("/v1/manage/login", get(login_form).post(login_submit))
        .route(
            "/v1/manage/integrations/{id}/connect",
            post(connect_integration),
        )
        .route(
            "/v1/manage/integrations/{id}/revoke",
            post(revoke_from_dashboard),
        )
        .route(
            "/v1/manage/integrations/{id}/producer",
            post(mint_from_dashboard),
        )
}

/// Builds the page's model. Reports presence and health only; stored credential
/// values never leave their owning stores.
async fn dashboard(
    State(state): State<ServerState>,
    _operator: OperatorAuthenticated,
) -> Result<Html<String>, crate::app_api::AppApiError> {
    let lookup = state.clone();
    let (space, device_ids) = tokio::task::spawn_blocking(move || {
        let space = lookup.operator_space()?;
        let devices = lookup
            .identity()
            .devices_for(&space.account_id)
            .map_err(|error| crate::app_api::AppApiError::Internal {
                message: error.to_string(),
            })?
            .into_iter()
            .map(|owner| owner.device_id)
            .collect::<Vec<_>>();
        Ok::<_, crate::app_api::AppApiError>((space, devices))
    })
    .await
    .map_err(|_| crate::app_api::AppApiError::Internal {
        message: "a worker task failed".into(),
    })??;
    let devices = device_ids
        .into_iter()
        .map(|id| {
            let connected = state.device_is_linked(&id);
            DeviceRow { id, connected }
        })
        .collect();

    let integrations = match state.integrations() {
        Some(runtime) => runtime
            .token_manager()
            .integration_ids()
            .into_iter()
            .map(|id| {
                let health = runtime.token_manager().health(&id);
                IntegrationRow {
                    has_producer_credential: state.producer_credentials().has_credential(&id),
                    id,
                    health,
                }
            })
            .collect(),
        None => Vec::new(),
    };

    let sources = space
        .image_sources
        .summaries(Utc::now())
        .into_iter()
        .map(|summary| SourceRow {
            id: summary.id,
            name: summary.name,
            has_frame: summary.has_frame,
            stale: summary.stale,
            last_push: summary.last_push.map(|time| time.to_rfc3339()),
        })
        .collect();

    Ok(Html(view::render_dashboard(&DashboardModel {
        devices,
        integrations,
        sources,
    })))
}

/// Always reachable: a 401 with no way to authenticate would make the page
/// useless in a browser, and this form carries nothing worth protecting.
async fn login_form() -> Html<String> {
    Html(view::render_login(None))
}

#[derive(Deserialize)]
struct LoginForm {
    token: String,
}

/// Mints the same cookie, through the same signer and TTL, as the JSON login
/// route -- one cookie format, not two.
async fn login_submit(State(state): State<ServerState>, Form(form): Form<LoginForm>) -> Response {
    if !state.verify_admin_token(&form.token) {
        return (
            StatusCode::UNAUTHORIZED,
            Html(view::render_login(Some("That token was not accepted."))),
        )
            .into_response();
    }
    if state.integrations().is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Html(view::render_login(Some(
                "OAuth integrations are not configured on this server.",
            ))),
        )
            .into_response();
    }
    let sid = SessionSigner::new_sid();
    let cookie = state.sessions().mint(&sid, Utc::now(), SESSION_TTL);
    let Ok(header_value) = HeaderValue::from_str(&set_cookie_header(&cookie, SESSION_TTL)) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let mut response = Redirect::to("/v1/manage").into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, header_value);
    response
}

async fn connect_integration(
    State(state): State<ServerState>,
    operator: OperatorAuthenticated,
    Path(id): Path<String>,
) -> Response {
    let Some(runtime) = state.integrations() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match runtime.start_consent(&operator.sid, &id) {
        Some(url) => Redirect::to(&url).into_response(),
        None => (
            StatusCode::BAD_REQUEST,
            Html(view::render_error(
                "Too many consent flows are already pending; finish or abandon one.",
            )),
        )
            .into_response(),
    }
}

async fn revoke_from_dashboard(
    State(state): State<ServerState>,
    _operator: OperatorAuthenticated,
    Path(id): Path<String>,
) -> Response {
    match revoke_integration_action(&state, &id).await {
        Ok(()) => Redirect::to("/v1/manage").into_response(),
        Err(error) => error.into_response(),
    }
}

/// Renders the credential inline rather than redirecting.
///
/// A redirect would have to carry the value in its `Location`, which puts a
/// live credential into browser history and every access log between here and
/// the operator. It is shown once because only its digest is kept.
async fn mint_from_dashboard(
    State(state): State<ServerState>,
    _operator: OperatorAuthenticated,
    Path(id): Path<String>,
) -> Response {
    match mint_producer_action(&state, &id).await {
        Ok(minted) => Html(view::render_minted_credential(
            &minted.integration_id,
            &minted.token,
        ))
        .into_response(),
        Err(error) => error.into_response(),
    }
}
