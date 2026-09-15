//! The only HTTP client the provider layer is given.
//!
//! # Why this adapter exists at all
//!
//! `providers::http::SystemHttpClient` validates a URL's scheme and refuses
//! embedded credentials, and that is all. It performs no address checks, so it
//! will happily fetch `http://192.168.1.1/` or the cloud metadata endpoint.
//! That was survivable when the providers ran inside the owner's own Mac app;
//! it is not survivable in the server, which sits on the homelab beside
//! unrelated services.
//!
//! [`crate::egress`] already implements the whole guard -- deny-list,
//! resolve-then-pin against DNS rebinding, per-hop re-validation across
//! redirects, body cap, wall-clock budget. This type is the seam that puts the
//! providers behind it, by implementing the trait they already depend on.
//! **No server-side card may be constructed with any other client**, which is
//! what keeps the SSRF surface to one function.
//!
//! # The sync/async seam
//!
//! `providers::Provider::refresh` is synchronous and [`crate::egress::fetch`]
//! is not. The bridge is [`tokio::runtime::Handle::block_on`], which is legal
//! from a `spawn_blocking` thread and panics from an async one -- so this
//! client must only ever be driven from inside `spawn_blocking`. The
//! constructor captures the handle rather than taking one per call so that
//! requirement is stated once, here, instead of at every call site.

use providers::ProviderError;
use providers::http::{HttpClient, validate_http_url};

use crate::egress::{self, EgressError};

/// The response-body ceiling the providers themselves declare. The egress
/// guard's own [`egress::MAX_RESPONSE_BODY_BYTES`] is larger; the tighter of
/// the two wins, and it is this one.
const MAX_BODY_BYTES: usize = providers::MAX_PROVIDER_RESPONSE_BYTES;

/// An [`HttpClient`] that fetches through the egress guard.
pub(crate) struct EgressHttpClient {
    runtime: tokio::runtime::Handle,
}

impl EgressHttpClient {
    /// Captures the current runtime handle.
    ///
    /// # Panics
    ///
    /// If there is no Tokio runtime to attach to. Every caller is inside the
    /// server's own runtime, so this is a programming error rather than a
    /// condition to handle.
    pub(crate) fn new() -> Self {
        Self {
            runtime: tokio::runtime::Handle::current(),
        }
    }
}

impl HttpClient for EgressHttpClient {
    fn get_text(&mut self, url: &str) -> Result<String, ProviderError> {
        // The providers' own cheap checks first -- scheme, no credentials --
        // because they produce the error the provider layer's configuration
        // messages are written for. The guard then does the address work.
        validate_http_url(url)?;

        let response = self
            .runtime
            .block_on(egress::fetch(url))
            .map_err(map_egress_error)?;

        // A non-2xx body is not data. The providers report the status so the
        // card can say "HTTP 429" rather than "malformed feed", which is the
        // difference between a rate limit somebody can wait out and a feed
        // somebody has to go and fix.
        if !(200..300).contains(&response.status) {
            return Err(ProviderError::HttpStatus(response.status));
        }
        if response.body.len() > MAX_BODY_BYTES {
            return Err(ProviderError::ResponseTooLarge);
        }
        String::from_utf8(response.body).map_err(|_| ProviderError::InvalidEncoding)
    }
}

/// Maps a guard refusal onto the provider vocabulary.
///
/// A denied address becomes `InvalidConfiguration` rather than an I/O error:
/// nothing about it will change on a retry, and the card's message should
/// send the owner to the URL they typed rather than to their network.
fn map_egress_error(error: EgressError) -> ProviderError {
    match error {
        // The reason alone, not the guard's full message: the card has 96
        // bytes for an error and the host is already on the screen the owner
        // typed it into.
        EgressError::Denied { reason, .. } => {
            ProviderError::InvalidConfiguration(reason.to_string())
        }
        EgressError::InvalidUrl(detail) | EgressError::UnsupportedScheme(detail) => {
            ProviderError::InvalidConfiguration(detail)
        }
        EgressError::MissingHost => {
            ProviderError::InvalidConfiguration("the URL has no host".into())
        }
        // Unreachable from a data card: the one-host allowlist belongs to the
        // credential path (`egress::fetch_post_form`), which this client never
        // calls. Mapped rather than left to `unreachable!()` so that adding a
        // caller can never turn a policy refusal into a panic.
        EgressError::HostNotPermitted { host } => {
            ProviderError::InvalidConfiguration(format!("host {host} is not permitted"))
        }
        EgressError::Timeout => ProviderError::Timeout,
        EgressError::TooManyRedirects => ProviderError::RedirectLimit,
        EgressError::ResponseTooLarge { .. } => ProviderError::ResponseTooLarge,
        // A DNS failure, a bad redirect and a connect failure are all "the
        // network did not cooperate", which is what the stale badge says.
        EgressError::ResolutionFailed { detail, .. }
        | EgressError::BadRedirect(detail)
        | EgressError::Request(detail) => ProviderError::Io(detail),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The guard's own module owns the exhaustive address-policy tests; what
    /// matters here is only that a refusal survives the mapping into the
    /// provider vocabulary, and that it does so *without a request*.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_private_address_is_refused_as_a_configuration_error() {
        let error = tokio::task::spawn_blocking(|| {
            EgressHttpClient::new().get_text("http://192.168.1.1/feed.xml")
        })
        .await
        .expect("the blocking task runs")
        .expect_err("a private address is denied");
        assert_eq!(
            error.category(),
            providers::ProviderErrorCategory::InvalidConfiguration,
            "a denied address is the owner's URL to fix, not a transient fault"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_cloud_metadata_endpoint_is_refused() {
        let error = tokio::task::spawn_blocking(|| {
            EgressHttpClient::new().get_text("http://169.254.169.254/latest/meta-data/")
        })
        .await
        .expect("the blocking task runs")
        .expect_err("the metadata endpoint is denied");
        assert_eq!(
            error.category(),
            providers::ProviderErrorCategory::InvalidConfiguration
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_non_http_scheme_is_refused_before_the_guard_is_consulted() {
        let error =
            tokio::task::spawn_blocking(|| EgressHttpClient::new().get_text("file:///etc/passwd"))
                .await
                .expect("the blocking task runs")
                .expect_err("a file URL is refused");
        assert_eq!(
            error.category(),
            providers::ProviderErrorCategory::InvalidConfiguration
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_url_carrying_credentials_is_refused_without_echoing_them() {
        let error = tokio::task::spawn_blocking(|| {
            EgressHttpClient::new().get_text("https://user:token@example.test/feed")
        })
        .await
        .expect("the blocking task runs")
        .expect_err("credentials are refused");
        assert!(!error.to_string().contains("token"));
    }
}
