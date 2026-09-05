pub mod http;
pub mod ics;
pub mod json_feed;
pub mod rss;
pub mod weather;

use std::fmt;
use std::time::Duration;

use chrono::{DateTime, Utc};
use protocol::{Field, FieldValue, truncate_utf8_to_bytes};

pub const MAX_PROVIDER_RESPONSE_BYTES: usize = 1_048_576;
pub const MAX_PROVIDER_REDIRECTS: u8 = 3;
pub const PROVIDER_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshPolicy {
    Interval(Duration),
    OnDemand,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSnapshot<T> {
    pub value: T,
    pub refreshed_at: Option<DateTime<Utc>>,
    pub age: Option<Duration>,
    pub stale: bool,
    pub error: Option<String>,
}

pub trait Provider {
    type Output;

    fn refresh_policy(&self) -> RefreshPolicy;

    fn refresh(&mut self, now: DateTime<Utc>) -> ProviderSnapshot<Self::Output>;
}

/// Shared bounded last-good state used by every network/file provider.
#[derive(Debug, Clone)]
pub struct LastGood<T> {
    value: Option<T>,
    refreshed_at: Option<DateTime<Utc>>,
}

impl<T> Default for LastGood<T> {
    fn default() -> Self {
        Self {
            value: None,
            refreshed_at: None,
        }
    }
}

impl<T: Clone + Default> LastGood<T> {
    pub fn complete(
        &mut self,
        now: DateTime<Utc>,
        result: Result<T, ProviderError>,
    ) -> ProviderSnapshot<T> {
        match result {
            Ok(value) => {
                self.value = Some(value.clone());
                self.refreshed_at = Some(now);
                ProviderSnapshot {
                    value,
                    refreshed_at: Some(now),
                    age: Some(Duration::ZERO),
                    stale: false,
                    error: None,
                }
            }
            Err(error) => {
                let age = self
                    .refreshed_at
                    .and_then(|success| now.signed_duration_since(success).to_std().ok());
                ProviderSnapshot {
                    value: self.value.clone().unwrap_or_default(),
                    refreshed_at: self.refreshed_at,
                    age,
                    stale: true,
                    error: Some(truncate_utf8_to_bytes(&error.to_string(), 96).to_owned()),
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderErrorCategory {
    Io,
    Timeout,
    Redirect,
    HttpStatus,
    Oversized,
    Encoding,
    Malformed,
    UnsafeContent,
    InvalidConfiguration,
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

impl ProviderError {
    pub const fn category(&self) -> ProviderErrorCategory {
        match self {
            Self::Io(_) => ProviderErrorCategory::Io,
            Self::Timeout => ProviderErrorCategory::Timeout,
            Self::RedirectLimit => ProviderErrorCategory::Redirect,
            Self::HttpStatus(_) => ProviderErrorCategory::HttpStatus,
            Self::ResponseTooLarge => ProviderErrorCategory::Oversized,
            Self::InvalidEncoding => ProviderErrorCategory::Encoding,
            Self::MalformedFeed(_) => ProviderErrorCategory::Malformed,
            Self::UnsafeContent => ProviderErrorCategory::UnsafeContent,
            Self::InvalidConfiguration(_) => ProviderErrorCategory::InvalidConfiguration,
        }
    }
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

pub(crate) fn text_field(key: impl Into<String>, value: impl Into<String>) -> Field {
    Field {
        key: key.into(),
        value: FieldValue::Text(value.into()),
    }
}
