pub mod ics;

use std::fmt;
use std::time::Duration;

use chrono::{DateTime, Utc};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    Io(String),
    Http(String),
    FeedTooLarge,
    MalformedFeed(String),
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "I/O: {error}"),
            Self::Http(error) => write!(formatter, "HTTP: {error}"),
            Self::FeedTooLarge => formatter.write_str("calendar feed is too large"),
            Self::MalformedFeed(error) => write!(formatter, "malformed calendar feed: {error}"),
        }
    }
}

impl std::error::Error for ProviderError {}
