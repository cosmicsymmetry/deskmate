use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub(crate) struct RateLimiter {
    limit: usize,
    window: Duration,
    attempts: Mutex<Attempts>,
}

#[derive(Default)]
struct Attempts {
    by_key: HashMap<String, VecDeque<Instant>>,
    last_sweep: Option<Instant>,
}

impl RateLimiter {
    pub(crate) fn new(limit: usize, window: Duration) -> Self {
        Self {
            limit,
            window,
            attempts: Mutex::new(Attempts::default()),
        }
    }

    pub(crate) fn check(&self, key: &str, now: Instant) -> Result<(), Duration> {
        let mut attempts = self.expire(now);
        let Some(entries) = attempts.by_key.get_mut(key) else {
            return Ok(());
        };
        prune(entries, now, self.window);
        if entries.is_empty() {
            attempts.by_key.remove(key);
            return Ok(());
        }
        self.check_entries(entries, now)
    }

    /// Admit and record under one lock so concurrent starts share one budget.
    pub(crate) fn check_and_record(&self, key: &str, now: Instant) -> Result<(), Duration> {
        let mut attempts = self.expire(now);
        let entries = attempts.by_key.entry(key.to_owned()).or_default();
        prune(entries, now, self.window);
        self.check_entries(entries, now)?;
        entries.push_back(now);
        Ok(())
    }

    fn check_entries(&self, entries: &VecDeque<Instant>, now: Instant) -> Result<(), Duration> {
        if entries.len() < self.limit {
            return Ok(());
        }
        let retry_at = entries.front().copied().unwrap_or(now) + self.window;
        Err(retry_at.saturating_duration_since(now))
    }

    pub(crate) fn record(&self, key: &str, now: Instant) {
        let mut attempts = self.expire(now);
        let entries = attempts.by_key.entry(key.to_owned()).or_default();
        prune(entries, now, self.window);
        entries.push_back(now);
    }

    fn expire(&self, now: Instant) -> MutexGuard<'_, Attempts> {
        let mut attempts = self
            .attempts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Reclaim even keys that are never looked up again, without scanning
        // the entire map on every request. Idle limiters are swept on next use.
        let interval = self.window.min(Duration::from_secs(60));
        if attempts
            .last_sweep
            .is_none_or(|last| now.saturating_duration_since(last) >= interval)
        {
            attempts.by_key.retain(|_, entries| {
                prune(entries, now, self.window);
                !entries.is_empty()
            });
            attempts.last_sweep = Some(now);
        }
        attempts
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
    fn checks_do_not_allocate_unrecorded_keys() {
        let limiter = RateLimiter::new(2, Duration::from_secs(60));
        for i in 0..100 {
            limiter
                .check(&format!("unrecorded-{i}"), Instant::now())
                .unwrap();
        }
        assert!(limiter.attempts.lock().unwrap().by_key.is_empty());
    }

    #[test]
    fn checks_and_records_expire_untouched_keys_without_losing_active_attempts() {
        for record in [false, true] {
            let limiter = RateLimiter::new(1, Duration::from_secs(60));
            let t0 = Instant::now();
            limiter.record("expired", t0);
            limiter.record("active", t0 + Duration::from_secs(59));
            let now = t0 + Duration::from_secs(80);
            if record {
                limiter.record("new", now);
            } else {
                limiter.check("new", now).unwrap();
            }
            assert!(
                !limiter
                    .attempts
                    .lock()
                    .unwrap()
                    .by_key
                    .contains_key("expired")
            );
            assert_eq!(limiter.check("active", now), Err(Duration::from_secs(39)));
            assert_eq!(
                limiter.check("active", t0 + Duration::from_secs(119)),
                Ok(())
            );
            assert!(
                !limiter
                    .attempts
                    .lock()
                    .unwrap()
                    .by_key
                    .contains_key("active")
            );
            // No lookup of "new": subsequent unrelated traffic must reclaim it too.
            limiter
                .check("unrelated", t0 + Duration::from_secs(3600))
                .unwrap();
            assert!(limiter.attempts.lock().unwrap().by_key.is_empty());
        }
    }

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
