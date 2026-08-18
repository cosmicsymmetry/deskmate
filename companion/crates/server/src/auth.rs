//! Bearer-token extraction and authentication shared by every device-facing
//! route. A missing header, a malformed header, and an unknown token are all
//! indistinguishable to the caller: every one of them is a 401.

use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};

use crate::ServerState;
use crate::registry::DeviceId;

/// Rejection returned for any bearer-auth failure. Deliberately featureless:
/// the caller learns nothing about *why* authentication failed beyond "try a
/// real token", and the server never logs the token value that was tried.
#[derive(Debug, thiserror::Error)]
#[error("missing, malformed, or unrecognized bearer token")]
pub struct AuthError;

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        StatusCode::UNAUTHORIZED.into_response()
    }
}

/// A request whose `Authorization: Bearer <token>` header named a device the
/// registry minted a token for.
#[derive(Debug, Clone)]
pub struct AuthenticatedDevice {
    pub device_id: DeviceId,
}

impl FromRequestParts<ServerState> for AuthenticatedDevice {
    type Rejection = AuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &ServerState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(parts).ok_or(AuthError)?;
        let device_id = state.registry().authenticate(token).ok_or(AuthError)?;
        Ok(Self { device_id })
    }
}

/// Extracts the token from a well-formed `Authorization: Bearer <token>`
/// header, or `None` for anything else (missing header, non-UTF-8 value,
/// wrong scheme). The scheme name is matched case-insensitively per RFC
/// 7235 §2.1 ("auth-scheme ... is case-insensitive") -- firmware or a proxy
/// sending `bearer`/`BEARER` is a protocol-legal client, not a malformed one.
fn bearer_token(parts: &Parts) -> Option<&str> {
    const SCHEME: &str = "Bearer ";
    let header = parts.headers.get(axum::http::header::AUTHORIZATION)?;
    let value = header.to_str().ok()?;
    if value.len() < SCHEME.len() || !value.is_char_boundary(SCHEME.len()) {
        return None;
    }
    let (scheme, token) = value.split_at(SCHEME.len());
    scheme.eq_ignore_ascii_case(SCHEME).then_some(token)
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;
    use axum::http::header::AUTHORIZATION;

    use super::bearer_token;

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
}
