//! The server-authored card faces.
//!
//! Each face is a pure function from a view model to an SVG document string.
//! Nothing here fetches, caches, or rasterizes: the provider layer decides what
//! is true, these modules decide what it looks like, and
//! [`crate::face_render`] turns the result into the frame the device receives.
//!
//! That split is what makes a face testable at all. A layout assertion here
//! needs no network and no golden image -- it renders a view model and reads
//! the document back.

pub(crate) mod adapt;
#[cfg(test)]
mod cases;
#[cfg(test)]
mod golden;
#[cfg(test)]
mod live;
pub(crate) mod rss;
pub(crate) mod svg;
pub(crate) mod theme;
pub(crate) mod token;
pub(crate) mod weather;

use chrono::{DateTime, Utc};

/// Renders an age the way a panel should state it: coarse, short, and never
/// more precise than it is honest about.
///
/// A panel refreshed every ten minutes cannot claim "3 minutes ago" and mean
/// it, so the units step up quickly and stop at days. An item dated in the
/// future -- which feeds do produce -- reads as "now" rather than as a negative
/// age.
pub(crate) fn relative_age(published: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let seconds = now.signed_duration_since(published).num_seconds();
    if seconds <= 60 {
        return "now".to_owned();
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours}h");
    }
    let days = hours / 24;
    if days < 7 {
        return format!("{days}d");
    }
    let weeks = days / 7;
    if weeks < 52 {
        return format!("{weeks}w");
    }
    format!("{}y", days / 365)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone as _;

    fn at(hours: i64, minutes: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 12, 12, 0, 0).unwrap()
            - chrono::Duration::hours(hours)
            - chrono::Duration::minutes(minutes)
    }

    #[test]
    fn ages_step_up_through_coarser_units() {
        let now = Utc.with_ymd_and_hms(2026, 9, 12, 12, 0, 0).unwrap();
        assert_eq!(relative_age(at(0, 0), now), "now");
        assert_eq!(relative_age(at(0, 14), now), "14m");
        assert_eq!(relative_age(at(3, 0), now), "3h");
        assert_eq!(relative_age(at(30, 0), now), "1d");
        assert_eq!(relative_age(at(24 * 10, 0), now), "1w");
    }

    #[test]
    fn an_item_dated_in_the_future_reads_as_now() {
        // Feeds really do this, usually via a timezone mistake. A panel
        // printing "-3h" would look broken in a way the feed caused.
        let now = Utc.with_ymd_and_hms(2026, 9, 12, 12, 0, 0).unwrap();
        let future = now + chrono::Duration::hours(5);
        assert_eq!(relative_age(future, now), "now");
    }
}
