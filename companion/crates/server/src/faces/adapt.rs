//! Turning what a provider learned into what a face draws.
//!
//! This layer is thin and boring on purpose. Keeping it separate is what lets
//! the faces be tested with hand-written view models and the providers be
//! tested with fixture bodies, without either test having to know about the
//! other side. It is also where every lossy decision lives -- rounding tenths
//! to whole degrees, formatting an hour, choosing a currency mark -- so those
//! decisions are in one file rather than scattered through the drawing code.

use chrono::{DateTime, Utc};
use providers::rss::RssFeed;
use providers::token::TokenQuote;
use providers::weather::WeatherReading;

use super::relative_age;
use super::rss::{FeedEntry, RssFace};
use super::token::TokenFace;
use super::weather::{Condition, HourlyStep, WeatherFace};

/// Tenths of a degree to whole degrees, rounded away from zero.
///
/// `(t + 5) / 10` on a negative value rounds the wrong way -- -185 tenths
/// would become -18 rather than -19 -- so the sign is handled explicitly. A
/// panel showing -18 when it is -18.5 is a small lie the owner cannot check.
fn whole_degrees(tenths: i64) -> i32 {
    let rounded = if tenths >= 0 {
        (tenths + 5) / 10
    } else {
        (tenths - 5) / 10
    };
    i32::try_from(rounded).unwrap_or(if rounded < 0 { i32::MIN } else { i32::MAX })
}

pub(crate) fn weather_face(reading: &WeatherReading, hourly_columns: usize) -> WeatherFace {
    WeatherFace {
        place: reading.location.clone(),
        temperature: whole_degrees(reading.temperature_tenths),
        summary: reading.summary.clone(),
        condition: Condition::from_wmo(reading.weather_code, reading.is_day),
        high: whole_degrees(reading.high_tenths),
        low: whole_degrees(reading.low_tenths),
        hourly: reading
            .hourly
            .iter()
            .take(hourly_columns)
            .map(|hour| HourlyStep {
                // Two digits, zero-padded, in the location's own clock. Not
                // 12-hour: the strip's columns are 66px wide and "12PM" does
                // not fit beside a temperature at a legible size.
                label: format!("{:02}", hour.hour),
                temperature: whole_degrees(hour.temperature_tenths),
                condition: Condition::from_wmo(hour.weather_code, hour.is_day),
            })
            .collect(),
    }
}

pub(crate) fn rss_face(feed: &RssFeed, feed_title: &str, now: DateTime<Utc>) -> RssFace {
    RssFace {
        feed_title: feed_title.to_owned(),
        entries: feed
            .items
            .iter()
            .map(|item| FeedEntry {
                title: item.title.clone(),
                // An unparseable or absent date yields no age at all. The
                // alternative -- treating it as "now" -- would put a fresh
                // badge on an item from 2019.
                age: item
                    .published
                    .as_deref()
                    .and_then(parse_feed_date)
                    .map(|published| relative_age(published, now))
                    .unwrap_or_default(),
            })
            .collect(),
    }
}

/// Parses the two date formats feeds actually use.
///
/// RSS 2.0 specifies RFC 2822 (`Tue, 12 Sep 2026 14:03:00 +0000`) and Atom
/// specifies RFC 3339 (`2026-09-12T14:03:00Z`). Both appear in the wild
/// regardless of which format the document claims to be, so both are tried
/// rather than branching on the feed type.
fn parse_feed_date(raw: &str) -> Option<DateTime<Utc>> {
    let trimmed = raw.trim();
    DateTime::parse_from_rfc2822(trimmed)
        .or_else(|_| DateTime::parse_from_rfc3339(trimmed))
        .ok()
        .map(|parsed| parsed.with_timezone(&Utc))
}

pub(crate) fn token_face(quote: &TokenQuote, window: &str) -> TokenFace {
    TokenFace {
        symbol: quote.symbol.clone(),
        name: quote.name.clone(),
        currency: quote.currency.clone(),
        currency_mark: currency_mark(&quote.currency).to_owned(),
        price: quote.price,
        change_percent: quote.change_percent_24h,
        window: window.to_owned(),
        low: quote.low_24h,
        high: quote.high_24h,
        series: quote.series.clone(),
    }
}

/// The mark for a currency, or nothing.
///
/// Deliberately a short list rather than a dependency. An unknown currency
/// gets no mark at all, which is correct rather than merely safe: the face
/// already prints the ISO code top-right, so "142.37 ... KRW" reads fine while
/// a guessed or mojibake mark would not.
fn currency_mark(code: &str) -> &'static str {
    match code.to_ascii_uppercase().as_str() {
        "USD" => "$",
        "EUR" => "\u{20ac}",
        "GBP" => "\u{a3}",
        "JPY" | "CNY" => "\u{a5}",
        "AED" => "\u{62f}.\u{625}",
        "BTC" => "\u{20bf}",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone as _;
    use providers::rss::FeedItem;
    use providers::weather::WeatherHour;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 12, 14, 0, 0).unwrap()
    }

    #[test]
    fn tenths_round_away_from_zero_in_both_directions() {
        assert_eq!(whole_degrees(185), 19);
        assert_eq!(whole_degrees(-185), -19);
        assert_eq!(whole_degrees(184), 18);
        assert_eq!(whole_degrees(-184), -18);
        assert_eq!(whole_degrees(0), 0);
    }

    #[test]
    fn a_reading_becomes_a_face_with_its_condition_resolved() {
        let reading = WeatherReading {
            location: "Amsterdam, Netherlands".to_owned(),
            temperature_tenths: 112,
            apparent_temperature_tenths: 98,
            summary: "Rain".to_owned(),
            weather_code: 63,
            is_day: true,
            high_tenths: 134,
            low_tenths: 81,
            hourly: vec![
                WeatherHour {
                    hour: 14,
                    temperature_tenths: 112,
                    weather_code: 63,
                    is_day: true,
                },
                WeatherHour {
                    hour: 15,
                    temperature_tenths: 121,
                    weather_code: 0,
                    is_day: false,
                },
            ],
        };
        let face = weather_face(&reading, 6);
        assert_eq!(face.temperature, 11);
        assert_eq!(face.high, 13);
        assert_eq!(face.low, 8);
        assert_eq!(face.condition, Condition::Rain);
        assert_eq!(face.hourly[0].label, "14");
        assert_eq!(
            face.hourly[1].condition,
            Condition::ClearNight,
            "the hour's own is_day decides, not the current reading's"
        );
    }

    #[test]
    fn more_hours_than_columns_are_dropped_at_the_adapter() {
        let reading = WeatherReading {
            hourly: (0..8)
                .map(|hour| WeatherHour {
                    hour,
                    temperature_tenths: 100,
                    weather_code: 0,
                    is_day: true,
                })
                .collect(),
            ..WeatherReading::default()
        };
        assert_eq!(weather_face(&reading, 6).hourly.len(), 6);
    }

    #[test]
    fn both_feed_date_formats_are_understood() {
        let rfc2822 = parse_feed_date("Sat, 12 Sep 2026 13:46:00 +0000").expect("RSS 2.0 date");
        let rfc3339 = parse_feed_date("2026-09-12T13:46:00Z").expect("Atom date");
        assert_eq!(rfc2822, rfc3339);
        assert_eq!(relative_age(rfc2822, now()), "14m");
    }

    #[test]
    fn an_offset_date_is_normalized_rather_than_read_as_local() {
        // A feed in +04:00 publishing at 17:46 local is 13:46 UTC. Reading the
        // wall-clock time and ignoring the offset would show "4h ago".
        let offset = parse_feed_date("Sat, 12 Sep 2026 17:46:00 +0400").expect("an offset date");
        assert_eq!(relative_age(offset, now()), "14m");
    }

    #[test]
    fn an_unparseable_or_missing_date_yields_no_age_rather_than_now() {
        let feed = RssFeed {
            items: vec![
                FeedItem {
                    title: "No date".to_owned(),
                    link: None,
                    published: None,
                },
                FeedItem {
                    title: "Junk date".to_owned(),
                    link: None,
                    published: Some("last Tuesday-ish".to_owned()),
                },
            ],
        };
        let face = rss_face(&feed, "Feed", now());
        assert_eq!(face.entries[0].age, "");
        assert_eq!(
            face.entries[1].age, "",
            "an unparseable date must not be rendered as fresh"
        );
    }

    #[test]
    fn a_quote_becomes_a_face_with_its_currency_mark() {
        let quote = TokenQuote {
            symbol: "sol".to_owned(),
            name: "Solana".to_owned(),
            currency: "USD".to_owned(),
            price: 142.37,
            change_percent_24h: 2.41,
            high_24h: 144.12,
            low_24h: 136.9,
            series: vec![140.0, 142.0],
        };
        let face = token_face(&quote, "24h");
        assert_eq!(face.currency_mark, "$");
        assert_eq!(face.symbol, "sol");
        assert_eq!(face.series.len(), 2);
    }

    #[test]
    fn an_unknown_currency_gets_no_mark_rather_than_a_guessed_one() {
        assert_eq!(currency_mark("KRW"), "");
        assert_eq!(currency_mark("usd"), "$", "the lookup is case-insensitive");
    }
}
