pub mod http;
pub mod rss;
pub mod token;
pub mod weather;

use std::fmt;

use protocol::truncate_utf8_to_bytes;

pub const MAX_PROVIDER_RESPONSE_BYTES: usize = 1_048_576;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSnapshot<T> {
    pub value: Option<T>,
    pub error: Option<String>,
}

/// Shared bounded last-good state used by every network/file provider.
#[derive(Debug, Clone)]
pub(crate) struct LastGood<T> {
    value: Option<T>,
}

impl<T> Default for LastGood<T> {
    fn default() -> Self {
        Self { value: None }
    }
}

impl<T: Clone> LastGood<T> {
    pub(crate) fn complete(&mut self, result: Result<T, ProviderError>) -> ProviderSnapshot<T> {
        match result {
            Ok(value) => {
                self.value = Some(value.clone());
                ProviderSnapshot {
                    value: Some(value),
                    error: None,
                }
            }
            Err(error) => ProviderSnapshot {
                value: self.value.clone(),
                error: Some(truncate_utf8_to_bytes(&error.to_string(), 96).to_owned()),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    Io(String),
    Timeout,
    RedirectLimit,
    HttpStatus(u16),
    ResponseTooLarge,
    InvalidEncoding,
    MalformedFeed(String),
    UnsafeContent,
    InvalidConfiguration(String),
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "I/O: {error}"),
            Self::Timeout => formatter.write_str("provider request timed out"),
            Self::RedirectLimit => formatter.write_str("provider redirect limit exceeded"),
            Self::HttpStatus(status) => write!(formatter, "provider returned HTTP {status}"),
            Self::ResponseTooLarge => formatter.write_str("provider response is too large"),
            Self::InvalidEncoding => formatter.write_str("provider response is not UTF-8"),
            Self::MalformedFeed(error) => write!(formatter, "malformed provider data: {error}"),
            Self::UnsafeContent => formatter.write_str("provider response contains active content"),
            Self::InvalidConfiguration(error) => {
                write!(formatter, "invalid provider configuration: {error}")
            }
        }
    }
}

impl std::error::Error for ProviderError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_good_preserves_complete_values_through_failures_and_recovers() {
        let mut state = LastGood::<Vec<String>>::default();
        let first = state.complete(Err(ProviderError::Timeout));
        assert!(first.value.is_none());
        assert_eq!(first.error.as_deref(), Some("provider request timed out"));

        let value = vec!["headline".to_owned(), "second row".to_owned()];
        let success = state.complete(Ok(value.clone()));
        assert_eq!(success.value.as_ref(), Some(&value));
        assert!(success.error.is_none());

        for (error, expected) in [
            (ProviderError::Timeout, "provider request timed out"),
            (ProviderError::HttpStatus(429), "provider returned HTTP 429"),
        ] {
            let mut failure = state.complete(Err(error));
            assert_eq!(failure.value.as_ref(), Some(&value));
            assert_eq!(failure.error.as_deref(), Some(expected));
            failure.value.as_mut().unwrap().clear();
        }

        let recovered = vec!["replacement".to_owned()];
        let recovery = state.complete(Ok(recovered.clone()));
        assert_eq!(recovery.value, Some(recovered.clone()));
        assert!(recovery.error.is_none());
        assert_eq!(
            state.complete(Err(ProviderError::Timeout)).value,
            Some(recovered)
        );
    }

    #[test]
    fn provider_errors_are_truncated_at_96_bytes_on_a_utf8_boundary() {
        let mut state = LastGood::<String>::default();
        for (message, expected) in [
            (
                format!("{}érest", "a".repeat(89)),
                format!("I/O: {}é", "a".repeat(89)),
            ),
            (
                format!("{}érest", "a".repeat(90)),
                format!("I/O: {}", "a".repeat(90)),
            ),
        ] {
            let snapshot = state.complete(Err(ProviderError::Io(message)));
            assert_eq!(snapshot.error.as_deref(), Some(expected.as_str()));
            assert!(expected.len() <= 96);
        }
    }
}
