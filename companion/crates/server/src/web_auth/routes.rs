use std::time::Instant;

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::ServerState;
use crate::auth::AdminAuthenticated;
use crate::identity::{Account, IdentityError, normalize_email};
use crate::web_auth::{
    AccountSession, ClientIp, InstanceOwner, SameOrigin, clear_session_cookie, set_session_cookie,
};

const EXPIRED_LINK: &str = "This sign-in link has expired.";
const WRONG_SETUP_CODE: &str = "That setup code is not right.";

pub(crate) fn routes() -> Router<ServerState> {
    Router::new()
        .route("/v1/app/instance", get(instance))
        .route("/v1/app/setup", post(setup))
        .route("/v1/app/auth/email", post(request_email_link))
        .route("/v1/app/auth/link", post(consume_email_link))
        .route("/v1/app/account", get(account))
        .route("/v1/app/sessions/revoke-all", post(revoke_all_sessions))
        .route("/v1/app/instance/signups", put(set_signups))
        .route("/v1/admin/signin-link", post(admin_signin_link))
}

#[derive(Serialize)]
struct InstanceResponse {
    setup_required: bool,
    google_enabled: bool,
    signups_open: bool,
    edition: crate::Edition,
}

async fn instance(State(state): State<ServerState>) -> Result<Json<InstanceResponse>, RouteError> {
    let lookup = state.clone();
    let (account_count, signups_open) = tokio::task::spawn_blocking(move || {
        let count = lookup.identity().account_count()?;
        let signups = lookup
            .identity()
            .signups_open()?
            .unwrap_or(lookup.signups_default());
        Ok::<_, IdentityError>((count, signups))
    })
    .await
    .map_err(|_| RouteError::WorkerFailed)??;
    Ok(Json(InstanceResponse {
        setup_required: account_count == 0 && state.inner.setup_code.is_active(),
        google_enabled: false,
        signups_open,
        edition: state.edition(),
    }))
}

#[derive(Deserialize)]
struct SetupRequest {
    code: String,
    email: String,
}

enum SetupResult {
    Created(Account, String),
    AlreadyComplete,
    WrongCode,
    InvalidEmail,
}

async fn setup(
    State(state): State<ServerState>,
    _origin: SameOrigin,
    client: ClientIp,
    Json(request): Json<SetupRequest>,
) -> Result<Response, RouteError> {
    let lookup = state.clone();
    let already_complete = tokio::task::spawn_blocking(move || lookup.identity().account_count())
        .await
        .map_err(|_| RouteError::WorkerFailed)??
        != 0;
    if already_complete {
        return Err(RouteError::NotFound);
    }
    ensure_failure_attempt_allowed(&state, client.0)?;
    let create = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let _guard = create
            .inner
            .setup_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if create.identity().account_count()? != 0 {
            return Ok(SetupResult::AlreadyComplete);
        }
        if !create.inner.setup_code.matches(&request.code) {
            return Ok(SetupResult::WrongCode);
        }
        let Some(email) = normalize_email(&request.email) else {
            return Ok(SetupResult::InvalidEmail);
        };
        let account = create
            .identity()
            .create_account(&email, false, true, Utc::now())?;
        let session = create.identity().create_session(&account.id, Utc::now())?;
        create.inner.setup_code.erase();
        Ok::<_, IdentityError>(SetupResult::Created(account, session))
    })
    .await
    .map_err(|_| RouteError::WorkerFailed)??;

    match result {
        SetupResult::Created(account, session) => Ok((
            [(header::SET_COOKIE, set_session_cookie(&session))],
            Json(AccountEnvelope::new(account)),
        )
            .into_response()),
        SetupResult::AlreadyComplete => Err(RouteError::NotFound),
        SetupResult::WrongCode => {
            record_failure(&state, client.0);
            Err(RouteError::bad_request(WRONG_SETUP_CODE))
        }
        SetupResult::InvalidEmail => {
            record_failure(&state, client.0);
            Err(RouteError::bad_request("Enter a valid email address."))
        }
    }
}

#[derive(Deserialize)]
struct EmailRequest {
    email: String,
}

async fn request_email_link(
    State(state): State<ServerState>,
    _origin: SameOrigin,
    client: ClientIp,
    Json(request): Json<EmailRequest>,
) -> Result<Response, RouteError> {
    let email = normalize_email(&request.email)
        .ok_or_else(|| RouteError::bad_request("Enter a valid email address."))?;
    check_and_record_email_limits(&state, &email, client.0)?;

    let issue = state.clone();
    let email_for_store = email.clone();
    let token = tokio::task::spawn_blocking(move || {
        let known = issue
            .identity()
            .account_by_email(&email_for_store)?
            .is_some();
        let signups_open = issue
            .identity()
            .signups_open()?
            .unwrap_or(issue.signups_default());
        if known || signups_open {
            issue
                .identity()
                .create_login_token(&email_for_store, Utc::now())
                .map(Some)
        } else {
            Ok(None)
        }
    })
    .await
    .map_err(|_| RouteError::WorkerFailed)??;

    if let Some(token) = token {
        let link = sign_in_link(state.public_url(), &token)?;
        spawn_mail(state.mailer(), email, link);
    }
    Ok((StatusCode::ACCEPTED, Json(serde_json::json!({}))).into_response())
}

#[derive(Deserialize)]
struct LinkRequest {
    token: String,
}

enum LinkResult {
    SignedIn(Account, String),
    Invalid,
}

async fn consume_email_link(
    State(state): State<ServerState>,
    _origin: SameOrigin,
    client: ClientIp,
    Json(request): Json<LinkRequest>,
) -> Result<Response, RouteError> {
    ensure_failure_attempt_allowed(&state, client.0)?;
    let consume = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let Some(email) = consume
            .identity()
            .consume_login_token(&request.token, Utc::now())?
        else {
            return Ok(LinkResult::Invalid);
        };
        let account = if let Some(account) = consume.identity().account_by_email(&email)? {
            consume.identity().mark_email_verified(&account.id)?;
            consume
                .identity()
                .account(&account.id)?
                .ok_or(IdentityError::NotFound)?
        } else {
            let signups_open = consume
                .identity()
                .signups_open()?
                .unwrap_or(consume.signups_default());
            if !signups_open {
                return Ok(LinkResult::Invalid);
            }
            consume
                .identity()
                .create_account(&email, true, false, Utc::now())?
        };
        let session = consume.identity().create_session(&account.id, Utc::now())?;
        Ok::<_, IdentityError>(LinkResult::SignedIn(account, session))
    })
    .await
    .map_err(|_| RouteError::WorkerFailed)??;

    match result {
        LinkResult::SignedIn(account, session) => Ok((
            [(header::SET_COOKIE, set_session_cookie(&session))],
            Json(AccountEnvelope::new(account)),
        )
            .into_response()),
        LinkResult::Invalid => {
            record_failure(&state, client.0);
            Err(RouteError::bad_request(EXPIRED_LINK))
        }
    }
}

async fn account(session: AccountSession) -> Json<AccountResponse> {
    Json(session.account.into())
}

async fn revoke_all_sessions(
    State(state): State<ServerState>,
    session: AccountSession,
) -> Result<Response, RouteError> {
    let account_id = session.account.id;
    tokio::task::spawn_blocking(move || state.identity().delete_sessions_for(&account_id))
        .await
        .map_err(|_| RouteError::WorkerFailed)??;
    Ok((
        [(header::SET_COOKIE, clear_session_cookie())],
        StatusCode::NO_CONTENT,
    )
        .into_response())
}

#[derive(Deserialize)]
struct SignupsRequest {
    open: bool,
}

#[derive(Serialize)]
struct SignupsResponse {
    signups_open: bool,
}

async fn set_signups(
    State(state): State<ServerState>,
    _owner: InstanceOwner,
    Json(request): Json<SignupsRequest>,
) -> Result<Json<SignupsResponse>, RouteError> {
    let open = request.open;
    tokio::task::spawn_blocking(move || state.identity().set_signups_open(open))
        .await
        .map_err(|_| RouteError::WorkerFailed)??;
    Ok(Json(SignupsResponse { signups_open: open }))
}

#[derive(Serialize)]
struct SignInLinkResponse {
    link: String,
}

async fn admin_signin_link(
    State(state): State<ServerState>,
    _admin: AdminAuthenticated,
    Json(request): Json<EmailRequest>,
) -> Result<Json<SignInLinkResponse>, RouteError> {
    let Some(email) = normalize_email(&request.email) else {
        return Err(RouteError::NotFound);
    };
    let issue = state.clone();
    let token = tokio::task::spawn_blocking(move || {
        if issue.identity().account_by_email(&email)?.is_none() {
            return Ok(None);
        }
        issue
            .identity()
            .create_login_token(&email, Utc::now())
            .map(Some)
    })
    .await
    .map_err(|_| RouteError::WorkerFailed)??
    .ok_or(RouteError::NotFound)?;
    Ok(Json(SignInLinkResponse {
        link: sign_in_link(state.public_url(), &token)?,
    }))
}

fn sign_in_link(public_url: &url::Url, token: &str) -> Result<String, RouteError> {
    let mut link = public_url
        .join("/signin")
        .map_err(|_| RouteError::Internal)?;
    link.query_pairs_mut().append_pair("token", token);
    Ok(link.into())
}

fn spawn_mail(mailer: std::sync::Arc<dyn crate::mailer::Mailer>, to: String, link: String) {
    tokio::spawn(async move {
        if let Err(error) = mailer.send_sign_in_link(&to, &link).await {
            tracing::warn!(target: "deskmate_server::mail", %error, "sign-in email delivery failed");
        }
    });
}

fn check_and_record_email_limits(
    state: &ServerState,
    email: &str,
    ip: std::net::IpAddr,
) -> Result<(), RouteError> {
    let now = Instant::now();
    let address_key = format!("email:{email}");
    let ip_key = format!("ip-email:{ip}");
    state
        .inner
        .email_address_limiter
        .check(&address_key, now)
        .map_err(RouteError::rate_limited)?;
    state
        .inner
        .email_ip_limiter
        .check(&ip_key, now)
        .map_err(RouteError::rate_limited)?;
    state.inner.email_address_limiter.record(&address_key, now);
    state.inner.email_ip_limiter.record(&ip_key, now);
    Ok(())
}

fn ensure_failure_attempt_allowed(
    state: &ServerState,
    ip: std::net::IpAddr,
) -> Result<(), RouteError> {
    state
        .inner
        .failure_limiter
        .check(&format!("ip-fail:{ip}"), Instant::now())
        .map_err(RouteError::rate_limited)
}

fn record_failure(state: &ServerState, ip: std::net::IpAddr) {
    state
        .inner
        .failure_limiter
        .record(&format!("ip-fail:{ip}"), Instant::now());
}

#[derive(Serialize)]
struct AccountEnvelope {
    account: AccountResponse,
}

impl AccountEnvelope {
    fn new(account: Account) -> Self {
        Self {
            account: account.into(),
        }
    }
}

#[derive(Serialize)]
struct AccountResponse {
    id: String,
    email: String,
    email_verified: bool,
    is_instance_owner: bool,
}

impl From<Account> for AccountResponse {
    fn from(account: Account) -> Self {
        Self {
            id: account.id.0,
            email: account.email,
            email_verified: account.email_verified,
            is_instance_owner: account.is_instance_owner,
        }
    }
}

#[derive(Debug)]
enum RouteError {
    BadRequest(&'static str),
    NotFound,
    RateLimited(u64),
    Store(IdentityError),
    WorkerFailed,
    Internal,
}

impl RouteError {
    fn bad_request(message: &'static str) -> Self {
        Self::BadRequest(message)
    }

    fn rate_limited(retry_after: std::time::Duration) -> Self {
        let partial_second = u64::from(retry_after.subsec_nanos() != 0);
        Self::RateLimited(retry_after.as_secs().saturating_add(partial_second).max(1))
    }
}

impl From<IdentityError> for RouteError {
    fn from(error: IdentityError) -> Self {
        Self::Store(error)
    }
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after_seconds: Option<u64>,
}

impl IntoResponse for RouteError {
    fn into_response(self) -> Response {
        match self {
            Self::BadRequest(message) => (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: message.to_owned(),
                    retry_after_seconds: None,
                }),
            )
                .into_response(),
            Self::NotFound => StatusCode::NOT_FOUND.into_response(),
            Self::RateLimited(seconds) => (
                StatusCode::TOO_MANY_REQUESTS,
                Json(ErrorResponse {
                    error: format!("Try again in {seconds} seconds."),
                    retry_after_seconds: Some(seconds),
                }),
            )
                .into_response(),
            Self::Store(error) => {
                tracing::error!(%error, "identity route failed");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
            Self::WorkerFailed => {
                tracing::error!("identity route worker failed");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    }
}
