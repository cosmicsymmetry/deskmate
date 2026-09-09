//! Inferring whether a picture source has gone quiet, from its own observed
//! cadence rather than from a declared interval.
//!
//! A declared interval is friction every external producer pays forever, and
//! half of them would declare it wrong. What makes inference safe rather than
//! guesswork is the clamp: a per-minute producer is not flagged after three
//! minutes of silence, and a daily one is not given three days of rope.

use std::time::Duration;

use chrono::{DateTime, Utc};

/// How many accepted push times one source remembers.
pub(crate) const PUSH_TIME_RING: usize = 8;

/// How many baseline intervals of silence mean "gone quiet".
pub(crate) const STALE_MULTIPLE: u32 = 3;

/// No source is flagged sooner than this, however fast it normally pushes.
pub(crate) const STALE_FLOOR: Duration = Duration::from_secs(15 * 60);

/// No source is given more rope than this, however slowly it normally pushes.
pub(crate) const STALE_CEILING: Duration = Duration::from_secs(48 * 60 * 60);

/// The smallest number of *intervals* an inference needs. Three intervals means
/// four pushes: one gap is not a cadence, and two cannot outvote an outlier.
const MIN_INTERVALS: usize = 3;

/// How long this source may stay silent before it counts as stale, or `None`
/// when too few pushes have been seen to infer anything.
pub(crate) fn stale_deadline(recent_pushes: &[DateTime<Utc>]) -> Option<Duration> {
    if recent_pushes.len() < MIN_INTERVALS + 1 {
        return None;
    }
    let mut intervals: Vec<i64> = recent_pushes
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).num_seconds().max(0))
        .collect();
    intervals.sort_unstable();
    // Median, not mean: one late push must not inflate the deadline for good.
    let middle = intervals.len() / 2;
    let baseline = if intervals.len() % 2 == 0 {
        (intervals[middle - 1] + intervals[middle]) / 2
    } else {
        intervals[middle]
    };
    let baseline = u64::try_from(baseline).unwrap_or(0);
    let deadline = Duration::from_secs(baseline.saturating_mul(u64::from(STALE_MULTIPLE)));
    Some(deadline.clamp(STALE_FLOOR, STALE_CEILING))
}

/// Whether this source has been silent past its inferred deadline.
pub(crate) fn is_stale(recent_pushes: &[DateTime<Utc>], now: DateTime<Utc>) -> bool {
    let Some(deadline) = stale_deadline(recent_pushes) else {
        return false;
    };
    let Some(last) = recent_pushes.last() else {
        return false;
    };
    let silence = (now - *last).num_seconds();
    if silence <= 0 {
        return false;
    }
    u64::try_from(silence).unwrap_or(0) > deadline.as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + seconds, 0).unwrap()
    }

    /// Pushes every `interval` seconds, oldest first, `count` of them.
    fn cadence(count: usize, interval: i64) -> Vec<DateTime<Utc>> {
        (0..count)
            .map(|index| at(index as i64 * interval))
            .collect()
    }

    #[test]
    fn fewer_than_three_intervals_is_never_stale() {
        // Three pushes give two intervals. A cadence cannot be inferred yet, so
        // no amount of silence flags it.
        let ring = cadence(3, 60);
        let last = *ring.last().unwrap();
        assert!(!is_stale(&ring, last + chrono::Duration::days(30)));
        assert_eq!(stale_deadline(&ring), None);
    }

    #[test]
    fn four_pushes_are_enough_to_infer_a_cadence() {
        let ring = cadence(4, 60);
        assert!(stale_deadline(&ring).is_some());
    }

    #[test]
    fn a_fast_producer_gets_the_floor_not_three_minutes() {
        // One-minute cadence: 3 x 60s = 180s, which the floor lifts to 15 min.
        let ring = cadence(8, 60);
        assert_eq!(stale_deadline(&ring), Some(STALE_FLOOR));
        let last = *ring.last().unwrap();
        assert!(!is_stale(&ring, last + chrono::Duration::minutes(14)));
        assert!(is_stale(&ring, last + chrono::Duration::minutes(16)));
    }

    #[test]
    fn a_daily_producer_gets_the_ceiling_not_three_days() {
        let ring = cadence(8, 24 * 60 * 60);
        assert_eq!(stale_deadline(&ring), Some(STALE_CEILING));
        let last = *ring.last().unwrap();
        assert!(!is_stale(&ring, last + chrono::Duration::hours(47)));
        assert!(is_stale(&ring, last + chrono::Duration::hours(49)));
    }

    #[test]
    fn a_ten_minute_producer_gets_three_times_its_baseline() {
        // 10 min baseline -> 30 min, inside both bounds, so neither clamp applies.
        let ring = cadence(8, 10 * 60);
        assert_eq!(stale_deadline(&ring), Some(Duration::from_secs(30 * 60)));
    }

    #[test]
    fn the_median_resists_one_late_push() {
        // Seven ~10-minute gaps and one enormous one. A mean would inflate the
        // deadline permanently; the median must not move much.
        let mut ring = cadence(8, 10 * 60);
        let last = *ring.last().unwrap();
        ring.push(last + chrono::Duration::hours(6));
        let deadline = stale_deadline(&ring).expect("enough samples");
        assert_eq!(deadline, Duration::from_secs(30 * 60));
    }

    #[test]
    fn any_push_clears_stale_immediately() {
        let ring = cadence(8, 10 * 60);
        let last = *ring.last().unwrap();
        let long_after = last + chrono::Duration::hours(5);
        assert!(is_stale(&ring, long_after), "silent for five hours");

        let mut recovered = ring.clone();
        recovered.push(long_after);
        assert!(
            !is_stale(&recovered, long_after),
            "a push clears it at once"
        );
    }

    #[test]
    fn an_empty_ring_is_never_stale() {
        assert!(!is_stale(&[], at(0)));
        assert_eq!(stale_deadline(&[]), None);
    }
}
