//! Bearer-token extraction and authentication shared by every device-facing
//! route. A missing header, a malformed header, and an unknown token are all
//! indistinguishable to the caller: every one of them is a 401.

use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};

use crate::ServerState;
use crate::registry::DeviceId;

/// Globally caps the restart-diagnostic warning at one line per minute. A
/// single atomic timestamp has constant memory and avoids a per-source map
/// whose attacker-controlled cardinality would create a second resource
/// problem while trying to solve the first one.
const UNKNOWN_AUTH_WARNING_INTERVAL_SECS: u64 = 60;
static UNKNOWN_AUTH_WARNING_LIMITER: UnknownAuthWarningLimiter = UnknownAuthWarningLimiter::new();
static UNKNOWN_AUTH_WARNING_CLOCK: LazyLock<Instant> = LazyLock::new(Instant::now);

struct UnknownAuthWarningLimiter {
    last_logged_elapsed_seconds: AtomicU64,
}

impl UnknownAuthWarningLimiter {
    const fn new() -> Self {
        Self {
            last_logged_elapsed_seconds: AtomicU64::new(u64::MAX),
        }
    }

    fn should_log(&self, now_elapsed_seconds: u64) -> bool {
        let mut previous = self.last_logged_elapsed_seconds.load(Ordering::Relaxed);
        loop {
            if previous != u64::MAX
                && now_elapsed_seconds.saturating_sub(previous) < UNKNOWN_AUTH_WARNING_INTERVAL_SECS
            {
                return false;
            }
            match self.last_logged_elapsed_seconds.compare_exchange(
                previous,
                now_elapsed_seconds,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(current) => previous = current,
            }
        }
    }
}

/// Rejection returned for any bearer-auth failure. Deliberately featureless:
/// the caller learns nothing about *why* authentication failed beyond "try a
/// real token", and the server never logs the token value that was tried.
#[derive(Debug, thiserror::Error)]
#[error("missing, malformed, or unrecognized bearer token")]
pub(crate) struct AuthError;

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        StatusCode::UNAUTHORIZED.into_response()
    }
}

/// A request whose `Authorization: Bearer <token>` header named a device the
/// registry minted a token for.
#[derive(Debug, Clone)]
pub(crate) struct AuthenticatedDevice {
    pub device_id: DeviceId,
}

impl FromRequestParts<ServerState> for AuthenticatedDevice {
    type Rejection = AuthError;

    // axum's FromRequestParts declares `async fn`, so the signature is fixed by
    // the trait even though this body never awaits. Rewriting it to return
    // `std::future::ready` to satisfy the lint would obscure the extractor for
    // no behavioural gain.
    #[allow(clippy::unused_async_trait_impl)]
    async fn from_request_parts(
        parts: &mut Parts,
        state: &ServerState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(parts).ok_or(AuthError)?;
        let Some(device_id) = state.registry().authenticate(token) else {
            // Never include the presented token. Keep exactly one rate-limited
            // warning, choosing static wording that distinguishes ordinary
            // unknown credentials from discarded state after a failed load.
            if UNKNOWN_AUTH_WARNING_LIMITER.should_log(monotonic_time_seconds()) {
                if state.registry().store_load_failed() {
                    tracing::warn!(
                        "device authentication failed: bearer token is not recognized because \
                         the device identity store failed to load; existing devices must be re-minted"
                    );
                } else {
                    tracing::warn!("device authentication failed: bearer token is not recognized");
                }
            }
            return Err(AuthError);
        };
        Ok(Self { device_id })
    }
}

fn monotonic_time_seconds() -> u64 {
    UNKNOWN_AUTH_WARNING_CLOCK.elapsed().as_secs()
}

/// Extracts the token from a well-formed `Authorization: Bearer <token>`
/// header, or `None` for anything else (missing header, non-UTF-8 value,
/// wrong scheme). The scheme name is matched case-insensitively per RFC
/// 7235 §2.1 ("auth-scheme ... is case-insensitive") -- firmware or a proxy
/// sending `bearer`/`BEARER` is a protocol-legal client, not a malformed one.
pub(crate) fn bearer_token(parts: &Parts) -> Option<&str> {
    const SCHEME: &str = "Bearer ";
    let header = parts.headers.get(axum::http::header::AUTHORIZATION)?;
    let value = header.to_str().ok()?;
    if value.len() < SCHEME.len() || !value.is_char_boundary(SCHEME.len()) {
        return None;
    }
    let (scheme, token) = value.split_at(SCHEME.len());
    scheme.eq_ignore_ascii_case(SCHEME).then_some(token)
}

/// The one admin extractor. `admin.rs` and `images.rs` each grew their own
/// copy because the first was private to its module; the management surface
/// would have been the third, so they now share this.
///
/// Rejection is a bare 401 with no body -- which is what both copies already
/// produced -- so an unauthenticated caller learns nothing about whether the
/// route, the device, or the token was the problem.
pub(crate) struct AdminAuthenticated;

pub(crate) struct AdminUnauthorized;

impl IntoResponse for AdminUnauthorized {
    fn into_response(self) -> Response {
        StatusCode::UNAUTHORIZED.into_response()
    }
}

impl FromRequestParts<ServerState> for AdminAuthenticated {
    type Rejection = AdminUnauthorized;

    // axum's `FromRequestParts` declares `async fn`, so the signature is fixed
    // by the trait even though this body never awaits.
    #[allow(clippy::unused_async_trait_impl)]
    async fn from_request_parts(
        parts: &mut Parts,
        state: &ServerState,
    ) -> Result<Self, Self::Rejection> {
        let presented = bearer_token(parts).ok_or(AdminUnauthorized)?;
        state
            .verify_admin_token(presented)
            .then_some(Self)
            .ok_or(AdminUnauthorized)
    }
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;
    use axum::http::header::AUTHORIZATION;

    use super::{UNKNOWN_AUTH_WARNING_INTERVAL_SECS, UnknownAuthWarningLimiter, bearer_token};

    fn parts_with_authorization(value: Option<&str>) -> axum::http::request::Parts {
        let mut request = axum::http::Request::new(());
        if let Some(value) = value {
            request
                .headers_mut()
                .insert(AUTHORIZATION, HeaderValue::from_str(value).unwrap());
        }
        request.into_parts().0
    }

    #[test]
    fn extracts_a_well_formed_bearer_token() {
        let parts = parts_with_authorization(Some("Bearer abc123"));
        assert_eq!(bearer_token(&parts), Some("abc123"));
    }

    #[test]
    fn extracts_a_token_with_a_differently_cased_scheme() {
        let parts = parts_with_authorization(Some("bearer abc123"));
        assert_eq!(bearer_token(&parts), Some("abc123"));
        let parts = parts_with_authorization(Some("BEARER abc123"));
        assert_eq!(bearer_token(&parts), Some("abc123"));
    }

    #[test]
    fn rejects_a_missing_header() {
        let parts = parts_with_authorization(None);
        assert_eq!(bearer_token(&parts), None);
    }

    #[test]
    fn rejects_the_wrong_scheme() {
        let parts = parts_with_authorization(Some("Basic abc123"));
        assert_eq!(bearer_token(&parts), None);
    }

    #[test]
    fn rejects_a_value_shorter_than_the_scheme() {
        let parts = parts_with_authorization(Some("Bear"));
        assert_eq!(bearer_token(&parts), None);
    }

    #[test]
    fn unknown_auth_warnings_are_limited_process_wide() {
        // Catches logging every rejected public request: only the first call in
        // a window may emit, while the boundary opens the diagnostic again.
        // Zero is the real first value the process-monotonic clock can return.
        let limiter = UnknownAuthWarningLimiter::new();
        assert!(limiter.should_log(0));
        assert!(!limiter.should_log(0));
        assert!(!limiter.should_log(UNKNOWN_AUTH_WARNING_INTERVAL_SECS - 1));
        assert!(limiter.should_log(UNKNOWN_AUTH_WARNING_INTERVAL_SECS));
    }
}
