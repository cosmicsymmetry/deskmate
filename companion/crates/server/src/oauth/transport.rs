//! The token-endpoint transport seam and Google response classification.
//!
//! Production sends through the egress guard (spec §6); tests inject a fake so
//! exchange/refresh/revoke are proven without touching Google or the network.

use std::pin::Pin;

use serde::Deserialize;

use crate::egress::{self, FetchResponse};

/// Boxed future returned by [`OAuthTransport`] so the trait stays object-safe
/// (`Arc<dyn OAuthTransport>` is what `TokenManager` holds).
pub type OAuthFuture<'a, T> = Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, thiserror::Error)]
#[error("oauth transport error: {0}")]
pub struct TransportError(pub String);

/// The seam a [`TokenManager`](super::token::TokenManager) uses to reach the
/// token/revoke endpoints. Production is [`EgressTransport`]; tests inject a fake.
pub trait OAuthTransport: Send + Sync {
    fn post_form(
        &self,
        url: String,
        form: Vec<(String, String)>,
    ) -> OAuthFuture<'_, Result<FetchResponse, TransportError>>;
}

/// Production transport: every call goes through the egress guard (spec §6).
pub struct EgressTransport;

impl OAuthTransport for EgressTransport {
    fn post_form(
        &self,
        url: String,
        form: Vec<(String, String)>,
    ) -> OAuthFuture<'_, Result<FetchResponse, TransportError>> {
        Box::pin(async move {
            let pairs: Vec<(&str, &str)> =
                form.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            egress::fetch_post_form(&url, &pairs)
                .await
                .map_err(|error| TransportError(error.to_string()))
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct GoogleTokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub expires_in: u64,
    #[serde(default)]
    pub refresh_token: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum TokenEndpointError {
    /// The grant was rejected as no longer valid (revoked, or refresh token
    /// expired). The operator must re-consent -- distinct from a transient error.
    #[error("authorization was revoked or expired (invalid_grant)")]
    InvalidGrant,
    /// Any other non-success status.
    #[error("token endpoint returned {status}: {detail}")]
    Provider { status: u16, detail: String },
    /// A 200 whose body could not be parsed as a token response.
    #[error("token endpoint returned an unparseable success body: {0}")]
    Malformed(String),
}

#[derive(Debug, Deserialize)]
struct TokenErrorBody {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

/// Classifies a token/refresh endpoint response. Pure over `(status, body)` so
/// the whole decision tree is unit-tested without a transport.
pub fn classify_token_response(
    status: u16,
    body: &[u8],
) -> Result<GoogleTokenResponse, TokenEndpointError> {
    if (200..300).contains(&status) {
        return serde_json::from_slice::<GoogleTokenResponse>(body)
            .map_err(|error| TokenEndpointError::Malformed(error.to_string()));
    }
    if let Ok(parsed) = serde_json::from_slice::<TokenErrorBody>(body) {
        if parsed.error == "invalid_grant" {
            return Err(TokenEndpointError::InvalidGrant);
        }
        let detail = match parsed.error_description {
            Some(description) => format!("{}: {description}", parsed.error),
            None => parsed.error,
        };
        return Err(TokenEndpointError::Provider { status, detail });
    }
    Err(TokenEndpointError::Provider {
        status,
        detail: String::from_utf8_lossy(body).chars().take(128).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_a_successful_exchange() {
        let body = br#"{"access_token":"at","expires_in":3600,"refresh_token":"rt"}"#;
        let parsed = classify_token_response(200, body).expect("ok");
        assert_eq!(parsed.access_token, "at");
        assert_eq!(parsed.expires_in, 3600);
        assert_eq!(parsed.refresh_token.as_deref(), Some("rt"));
    }

    #[test]
    fn classifies_a_refresh_without_a_new_refresh_token() {
        let body = br#"{"access_token":"at2","expires_in":3599}"#;
        let parsed = classify_token_response(200, body).expect("ok");
        assert_eq!(parsed.refresh_token, None);
    }

    #[test]
    fn a_200_without_access_token_is_malformed() {
        let body = br#"{"expires_in":3600}"#;
        assert!(matches!(
            classify_token_response(200, body),
            Err(TokenEndpointError::Malformed(_))
        ));
    }

    #[test]
    fn invalid_grant_is_its_own_variant() {
        let body = br#"{"error":"invalid_grant","error_description":"expired"}"#;
        assert!(matches!(
            classify_token_response(400, body),
            Err(TokenEndpointError::InvalidGrant)
        ));
    }

    #[test]
    fn other_4xx_errors_are_provider_errors() {
        let body = br#"{"error":"invalid_client"}"#;
        match classify_token_response(401, body) {
            Err(TokenEndpointError::Provider { status, detail }) => {
                assert_eq!(status, 401);
                assert!(detail.contains("invalid_client"));
            }
            other => panic!("expected Provider, got {other:?}"),
        }
    }

    #[test]
    fn a_5xx_is_a_provider_error_even_with_an_opaque_body() {
        assert!(matches!(
            classify_token_response(503, b"upstream down"),
            Err(TokenEndpointError::Provider { status: 503, .. })
        ));
    }
}
