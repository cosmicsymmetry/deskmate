use crate::{
    MAX_PROVIDER_REDIRECTS, MAX_PROVIDER_RESPONSE_BYTES, PROVIDER_REQUEST_TIMEOUT, ProviderError,
};

pub trait HttpClient {
    fn get_text(&mut self, url: &str) -> Result<String, ProviderError>;
}

pub struct SystemHttpClient {
    agent: ureq::Agent,
}

impl Default for SystemHttpClient {
    fn default() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(PROVIDER_REQUEST_TIMEOUT))
            .max_redirects(u32::from(MAX_PROVIDER_REDIRECTS))
            .max_redirects_will_error(true)
            .build()
            .into();
        Self { agent }
    }
}

impl HttpClient for SystemHttpClient {
    fn get_text(&mut self, url: &str) -> Result<String, ProviderError> {
        validate_http_url(url)?;
        let mut response = self.agent.get(url).call().map_err(classify_ureq_error)?;
        response
            .body_mut()
            .with_config()
            .limit(u64::try_from(MAX_PROVIDER_RESPONSE_BYTES.saturating_add(1)).unwrap_or(u64::MAX))
            .lossy_utf8(false)
            .read_to_string()
            .map_err(classify_ureq_error)
            .and_then(|body| bound_body(body, MAX_PROVIDER_RESPONSE_BYTES))
    }
}

pub fn validate_http_url(raw: &str) -> Result<(), ProviderError> {
    let parsed = url::Url::parse(raw)
        .map_err(|_| ProviderError::InvalidConfiguration("URL is invalid".into()))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(ProviderError::InvalidConfiguration(
            "URL must use HTTP or HTTPS".into(),
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(ProviderError::InvalidConfiguration(
            "URL must not contain credentials".into(),
        ));
    }
    Ok(())
}

fn classify_ureq_error(error: ureq::Error) -> ProviderError {
    match error {
        ureq::Error::Timeout(_) => ProviderError::Timeout,
        ureq::Error::TooManyRedirects | ureq::Error::RedirectFailed => ProviderError::RedirectLimit,
        ureq::Error::StatusCode(status) => ProviderError::HttpStatus(status),
        ureq::Error::BodyExceedsLimit(_) => ProviderError::ResponseTooLarge,
        ureq::Error::Io(error) if error.kind() == std::io::ErrorKind::InvalidData => {
            ProviderError::InvalidEncoding
        }
        _ => ProviderError::Io("provider transport failed".into()),
    }
}

fn bound_body(body: String, maximum_response_bytes: usize) -> Result<String, ProviderError> {
    if body.len() > maximum_response_bytes {
        Err(ProviderError::ResponseTooLarge)
    } else {
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_response_size_and_classifies_transport_failures() {
        assert_eq!(
            bound_body("x".repeat(33), 32).unwrap_err().category(),
            crate::ProviderErrorCategory::Oversized
        );
        assert_eq!(
            classify_ureq_error(ureq::Error::Timeout(ureq::Timeout::Global)).category(),
            crate::ProviderErrorCategory::Timeout
        );
        assert_eq!(
            classify_ureq_error(ureq::Error::TooManyRedirects).category(),
            crate::ProviderErrorCategory::Redirect
        );
        assert_eq!(
            classify_ureq_error(ureq::Error::StatusCode(503)),
            ProviderError::HttpStatus(503)
        );
        assert_eq!(PROVIDER_REQUEST_TIMEOUT.as_secs(), 10);
        assert_eq!(MAX_PROVIDER_REDIRECTS, 3);
        assert_eq!(MAX_PROVIDER_RESPONSE_BYTES, 1_048_576);
    }

    #[test]
    fn rejects_non_http_and_credential_bearing_urls_without_echoing_them() {
        assert!(validate_http_url("file:///etc/passwd").is_err());
        let error = validate_http_url("https://secret:token@example.test/feed").unwrap_err();
        assert_eq!(
            error.category(),
            crate::ProviderErrorCategory::InvalidConfiguration
        );
        assert!(!error.to_string().contains("secret"));
        assert!(!error.to_string().contains("token"));
    }
}
