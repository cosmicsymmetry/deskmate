use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub(crate) struct RateLimiter {
    limit: usize,
    window: Duration,
    attempts: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl RateLimiter {
    pub(crate) fn new(limit: usize, window: Duration) -> Self {
        Self {
            limit,
            window,
            attempts: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn check(&self, key: &str, now: Instant) -> Result<(), Duration> {
        let mut attempts = self
            .attempts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entries = attempts.entry(key.to_owned()).or_default();
        prune(entries, now, self.window);
        if entries.len() < self.limit {
            return Ok(());
        }
        let retry_at = entries.front().copied().unwrap_or(now) + self.window;
        Err(retry_at.saturating_duration_since(now))
    }

    pub(crate) fn record(&self, key: &str, now: Instant) {
        let mut attempts = self
            .attempts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entries = attempts.entry(key.to_owned()).or_default();
        prune(entries, now, self.window);
        entries.push_back(now);
    }
}

fn prune(entries: &mut VecDeque<Instant>, now: Instant, window: Duration) {
    while entries
        .front()
        .is_some_and(|attempt| now.saturating_duration_since(*attempt) >= window)
    {
        entries.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sliding_window_allows_limit_then_reports_retry_after() {
        let limiter = RateLimiter::new(2, Duration::from_secs(60));
        let t0 = Instant::now();
        for _ in 0..2 {
            limiter.check("k", t0).unwrap();
            limiter.record("k", t0);
        }
        let wait = limiter
            .check("k", t0 + Duration::from_secs(10))
            .unwrap_err();
        assert_eq!(wait, Duration::from_secs(50));
        assert!(limiter.check("k", t0 + Duration::from_secs(61)).is_ok());
        assert!(limiter.check("other", t0).is_ok());
    }
}
