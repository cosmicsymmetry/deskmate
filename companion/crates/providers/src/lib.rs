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

impl<T> ProviderSnapshot<T> {
    pub(crate) fn from_result(result: Result<T, ProviderError>) -> Self {
        match result {
            Ok(value) => Self {
                value: Some(value),
                error: None,
            },
            Err(error) => Self {
                value: None,
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
    fn provider_errors_are_truncated_at_96_bytes_on_a_utf8_boundary() {
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
            let snapshot = ProviderSnapshot::<String>::from_result(Err(ProviderError::Io(message)));
            assert!(snapshot.value.is_none());
            assert_eq!(snapshot.error.as_deref(), Some(expected.as_str()));
            assert!(expected.len() <= 96);
        }
    }
}
