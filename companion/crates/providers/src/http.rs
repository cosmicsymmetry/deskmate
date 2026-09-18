use crate::ProviderError;

pub trait HttpClient {
    fn get_text(&mut self, url: &str) -> Result<String, ProviderError>;
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

#[cfg(test)]
mod tests {
    use super::*;

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
