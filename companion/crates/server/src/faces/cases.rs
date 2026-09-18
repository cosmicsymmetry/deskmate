//! The named faces the golden harness renders.
//!
//! One list, used two ways: as the fixtures every layout test asserts against,
//! and as the set a human reviews. Keeping them here rather than inline in each
//! test means a case added to check a bug is automatically also a case somebody
//! looks at.
//!
//! # How to look at these
//!
//! ```sh
//! DESKMATE_FACE_DUMP=/tmp/faces cargo test -p server faces::golden -- --nocapture
//! ```
//!
//! That writes one PNG per case. The PNGs are deliberately **not** committed:
//! anti-aliased pixel diffs are brittle, and a face is reviewed by eye on a
//! real panel. What *is* asserted in CI is that every case
//! renders, that its text stays inside the canvas, and that the layout facts
//! each case was added for still hold.

use super::rss::{FeedEntry, RssFace};
use super::svg::count;
use super::token::TokenFace;
use super::weather::{Condition, HourlyStep, WeatherFace};

/// One reviewable face.
pub(crate) struct Case {
    /// Stable, filesystem-safe, and descriptive of the *situation*, not the
    /// data: `rss--three-line-lead`, never `rss--hacker-news`.
    pub name: &'static str,
    pub svg: String,
}

fn entry(title: &str, age: &str) -> FeedEntry {
    FeedEntry {
        title: title.to_owned(),
        age: age.to_owned(),
    }
}

fn hour(label: &str, temperature: i32, condition: Condition) -> HourlyStep {
    HourlyStep {
        label: label.to_owned(),
        temperature,
        condition,
    }
}

/// A plausible 24-hour price series with a visible trend and some noise, so the
/// sparkline is reviewed against something shaped like real data rather than a
/// clean sine wave.
fn series(start: f64, drift: f64, samples: usize) -> Vec<f64> {
    (0..samples)
        .map(|index| {
            let step = count(index);
            let progress = step / (count(samples) - 1.0);
            let wobble = (step / 6.0).sin() * 1.4 + (step / 2.3).cos() * 0.6;
            start + drift * progress + wobble
        })
        .collect()
}

pub(crate) fn all() -> Vec<Case> {
    let mut cases = rss_cases();
    cases.extend(token_cases());
    cases.extend(weather_cases());
    cases
}

fn rss_cases() -> Vec<Case> {
    let mut cases = vec![Case {
        name: "rss--three-line-lead",
        svg: super::rss::render(&RssFace {
            feed_title: "Hacker News".to_owned(),
            entries: vec![
                entry(
                    "Rust 1.98 stabilises const generics and ships a much faster linker",
                    "14m",
                ),
                entry("A postmortem of the eu-west-1 control plane outage", "1h"),
                entry("Writing a toy TCP stack in 400 lines of Zig", "3h"),
                entry("The case against microservices, revisited", "5h"),
            ],
        }),
    }];
    cases.push(Case {
        name: "rss--one-line-lead",
        svg: super::rss::render(&RssFace {
            feed_title: "Changelog".to_owned(),
            entries: vec![
                entry("Postgres 19 is out", "6m"),
                entry("SQLite adds strict table checks by default", "2h"),
                entry("Zig 0.16 release notes", "8h"),
                entry("A new ARM backend for LLVM", "1d"),
            ],
        }),
    });
    cases.push(Case {
        name: "rss--empty-feed",
        svg: super::rss::render(&RssFace {
            feed_title: "Quiet feed".to_owned(),
            entries: Vec::new(),
        }),
    });
    cases.push(Case {
        name: "rss--single-item",
        svg: super::rss::render(&RssFace {
            feed_title: "Status".to_owned(),
            entries: vec![entry("All systems operational", "30m")],
        }),
    });
    cases.push(Case {
        name: "rss--undated-items",
        svg: super::rss::render(&RssFace {
            feed_title: "No dates".to_owned(),
            entries: vec![
                entry("A feed whose items carry no pubDate at all", ""),
                entry("So none of these rows print an age", ""),
                entry("And the lead prints none either", ""),
            ],
        }),
    });

    cases
}

fn token_cases() -> Vec<Case> {
    let mut cases = vec![Case {
        name: "token--solana-rising",
        svg: super::token::render(&TokenFace {
            symbol: "SOL".to_owned(),
            name: "Solana".to_owned(),
            currency: "USD".to_owned(),
            currency_mark: "$".to_owned(),
            price: 142.37,
            change_percent: 2.41,
            window: "24h".to_owned(),
            low: 136.90,
            high: 144.12,
            series: series(137.0, 5.2, 96),
        }),
    }];
    cases.push(Case {
        name: "token--solana-falling",
        svg: super::token::render(&TokenFace {
            symbol: "SOL".to_owned(),
            name: "Solana".to_owned(),
            currency: "USD".to_owned(),
            currency_mark: "$".to_owned(),
            price: 118.04,
            change_percent: -6.83,
            window: "24h".to_owned(),
            low: 116.50,
            high: 127.80,
            series: series(127.0, -9.0, 96),
        }),
    });
    cases.push(Case {
        name: "token--five-figure-price",
        svg: super::token::render(&TokenFace {
            symbol: "BTC".to_owned(),
            name: "Bitcoin".to_owned(),
            currency: "USD".to_owned(),
            currency_mark: "$".to_owned(),
            price: 104_235.50,
            change_percent: 0.42,
            window: "24h".to_owned(),
            low: 103_010.0,
            high: 105_700.0,
            series: series(103_500.0, 700.0, 96),
        }),
    });
    cases.push(Case {
        name: "token--sub-cent-price",
        svg: super::token::render(&TokenFace {
            symbol: "BONK".to_owned(),
            name: "Bonk".to_owned(),
            currency: "USD".to_owned(),
            currency_mark: "$".to_owned(),
            price: 0.000_041_82,
            change_percent: 11.6,
            window: "24h".to_owned(),
            low: 0.000_037_1,
            high: 0.000_043_9,
            series: series(0.000_037, 0.000_005, 96),
        }),
    });
    cases.push(Case {
        name: "token--no-series-yet",
        svg: super::token::render(&TokenFace {
            symbol: "SOL".to_owned(),
            name: "Solana".to_owned(),
            currency: "USD".to_owned(),
            currency_mark: "$".to_owned(),
            price: 142.37,
            change_percent: 0.0,
            window: "24h".to_owned(),
            low: 142.37,
            high: 142.37,
            series: Vec::new(),
        }),
    });

    cases
}

fn weather_cases() -> Vec<Case> {
    let mut cases = vec![Case {
        name: "weather--clear-day",
        svg: super::weather::render(&WeatherFace {
            place: "Dubai".to_owned(),
            temperature: 34,
            summary: "Mostly clear".to_owned(),
            condition: Condition::ClearDay,
            high: 38,
            low: 27,
            hourly: vec![
                hour("14", 34, Condition::ClearDay),
                hour("15", 35, Condition::ClearDay),
                hour("16", 34, Condition::PartlyCloudyDay),
                hour("17", 32, Condition::PartlyCloudyDay),
                hour("18", 30, Condition::Cloudy),
                hour("19", 29, Condition::ClearNight),
            ],
        }),
    }];
    cases.push(Case {
        name: "weather--rain",
        svg: super::weather::render(&WeatherFace {
            place: "Amsterdam".to_owned(),
            temperature: 11,
            summary: "Moderate rain".to_owned(),
            condition: Condition::Rain,
            high: 13,
            low: 8,
            hourly: vec![
                hour("09", 11, Condition::Rain),
                hour("10", 11, Condition::Rain),
                hour("11", 12, Condition::Drizzle),
                hour("12", 12, Condition::Cloudy),
                hour("13", 13, Condition::PartlyCloudyDay),
                hour("14", 12, Condition::Rain),
            ],
        }),
    });
    cases.push(Case {
        name: "weather--snow-sub-zero",
        svg: super::weather::render(&WeatherFace {
            place: "Tromso".to_owned(),
            temperature: -18,
            summary: "Heavy snow".to_owned(),
            condition: Condition::Snow,
            high: -9,
            low: -24,
            hourly: vec![
                hour("06", -18, Condition::Snow),
                hour("07", -17, Condition::Snow),
                hour("08", -15, Condition::Sleet),
                hour("09", -13, Condition::Cloudy),
                hour("10", -11, Condition::PartlyCloudyDay),
                hour("11", -9, Condition::ClearDay),
            ],
        }),
    });
    cases.push(Case {
        name: "weather--thunderstorm-night",
        svg: super::weather::render(&WeatherFace {
            place: "Singapore".to_owned(),
            temperature: 27,
            summary: "Thunderstorms".to_owned(),
            condition: Condition::Thunderstorm,
            high: 31,
            low: 26,
            hourly: vec![
                hour("22", 27, Condition::Thunderstorm),
                hour("23", 27, Condition::Rain),
                hour("00", 26, Condition::Rain),
                hour("01", 26, Condition::Drizzle),
                hour("02", 26, Condition::PartlyCloudyNight),
                hour("03", 26, Condition::ClearNight),
            ],
        }),
    });
    cases.extend(weather_edge_cases());
    cases
}

/// The weather faces that exist to pin a behaviour rather than to show a
/// condition: the long place name, the missing strip, the extremes.
fn weather_edge_cases() -> Vec<Case> {
    let mut cases = vec![Case {
        name: "weather--fog",
        svg: super::weather::render(&WeatherFace {
            place: "San Francisco".to_owned(),
            temperature: 13,
            summary: "Depositing rime fog".to_owned(),
            condition: Condition::Fog,
            high: 17,
            low: 11,
            hourly: vec![
                hour("07", 13, Condition::Fog),
                hour("08", 13, Condition::Fog),
                hour("09", 14, Condition::Cloudy),
                hour("10", 15, Condition::PartlyCloudyDay),
                hour("11", 16, Condition::ClearDay),
                hour("12", 17, Condition::ClearDay),
            ],
        }),
    }];
    cases.push(Case {
        name: "weather--long-place-and-range",
        // The shape the live geocoder returns: "city, country", which is long
        // enough to reach the high/low beside it. A tracked eyebrow measured
        // without its tracking drew straight through it.
        svg: super::weather::render(&WeatherFace {
            place: "Dubai, United Arab Emirates".to_owned(),
            temperature: 37,
            summary: "Clear".to_owned(),
            condition: Condition::ClearNight,
            high: 44,
            low: 29,
            hourly: vec![
                hour("19", 37, Condition::ClearNight),
                hour("20", 37, Condition::ClearNight),
                hour("21", 35, Condition::ClearNight),
                hour("22", 34, Condition::ClearNight),
                hour("23", 33, Condition::ClearNight),
                hour("00", 32, Condition::ClearNight),
            ],
        }),
    });
    cases.push(Case {
        name: "weather--long-place-name",
        svg: super::weather::render(&WeatherFace {
            place: "Llanfairpwllgwyngyllgogerychwyrndrobwllllantysiliogogogoch".to_owned(),
            temperature: 9,
            summary: "Freezing drizzle and blowing snow later".to_owned(),
            condition: Condition::Sleet,
            high: 10,
            low: 4,
            hourly: vec![
                hour("12", 9, Condition::Sleet),
                hour("13", 9, Condition::Sleet),
                hour("14", 8, Condition::Rain),
                hour("15", 7, Condition::Rain),
                hour("16", 6, Condition::Cloudy),
                hour("17", 5, Condition::PartlyCloudyNight),
            ],
        }),
    });
    cases.push(Case {
        name: "weather--no-hourly",
        svg: super::weather::render(&WeatherFace {
            place: "Reykjavik".to_owned(),
            temperature: 4,
            summary: "Overcast".to_owned(),
            condition: Condition::Cloudy,
            high: 6,
            low: 1,
            hourly: Vec::new(),
        }),
    });

    cases
}
