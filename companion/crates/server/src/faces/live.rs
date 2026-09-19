//! The end-to-end check: real providers, real network, real faces.
//!
//! Off by default. CI must not depend on `CoinGecko`'s rate limit or somebody
//! else's RSS host being up, and a test that fails for those reasons trains
//! people to ignore failures. It runs when asked:
//!
//! ```sh
//! DESKMATE_LIVE_FACES=1 \
//! DESKMATE_FACE_DUMP=/tmp/faces-live \
//!   cargo test -p server faces::live -- --nocapture --test-threads 1
//! ```
//!
//! What it proves that no other test can: that the real API shapes still parse
//! into the view models the faces expect, and that the whole chain --
//! egress guard, provider, adapter, face, rasterizer -- produces a frame. A
//! fixture test cannot prove the first of those, because the fixture was
//! written from the same reading of the API as the parser.

#![cfg(test)]

use std::time::Duration;

use chrono::Utc;
use providers::Provider as _;
use providers::rss::{RssOptions, RssProvider};
use providers::token::{TokenOptions, TokenProvider};
use providers::weather::{WeatherOptions, WeatherProvider, WeatherUnits};

use super::{adapt, rss, token, weather};
use crate::egress_client::EgressHttpClient;
use crate::face_render::{frame_from_svg, pixmap_from_svg};

fn enabled() -> bool {
    std::env::var_os("DESKMATE_LIVE_FACES").is_some()
}

/// Writes a face for review and asserts it produces a frame.
fn accept(name: &str, svg: &str) {
    let frame =
        frame_from_svg(svg).unwrap_or_else(|error| panic!("{name} did not render: {error}"));
    let expected = 12 + (448 * 368 * 2);
    assert_eq!(frame.bytes.len(), expected, "{name} framed wrongly");

    if let Some(directory) = std::env::var_os("DESKMATE_FACE_DUMP") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).expect("the dump directory is creatable");
        let path = directory.join(format!("{name}.png"));
        pixmap_from_svg(svg)
            .expect("the face renders")
            .save_png(&path)
            .expect("the PNG is written");
        println!("wrote {}", path.display());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_live_weather_card_renders() {
    if !enabled() {
        return;
    }
    let snapshot = tokio::task::spawn_blocking(|| {
        WeatherProvider::new(
            EgressHttpClient::new(),
            WeatherOptions {
                location: std::env::var("DESKMATE_LIVE_PLACE")
                    .unwrap_or_else(|_| "Dubai".to_owned()),
                units: WeatherUnits::Metric,
                refresh_interval: Duration::from_mins(15),
            },
        )
        .refresh(Utc::now())
    })
    .await
    .expect("the blocking refresh runs");

    assert!(
        snapshot.error.is_none(),
        "the live weather fetch failed: {:?}",
        snapshot.error
    );
    let reading = &snapshot.value;
    println!(
        "weather: {} {}\u{b0} code {} day {} H{} L{} hours {}",
        reading.location,
        reading.temperature_tenths / 10,
        reading.weather_code,
        reading.is_day,
        reading.high_tenths / 10,
        reading.low_tenths / 10,
        reading.hourly.len(),
    );
    assert!(
        !reading.hourly.is_empty(),
        "the live forecast carried no hourly series, so the strip would be empty"
    );
    // The strip starts at the current hour, which is the alignment no fixture
    // can prove is still right against the live response shape.
    assert!(
        reading.hourly.windows(2).all(|pair| {
            let step = (pair[1].hour + 24 - pair[0].hour) % 24;
            step == 1
        }),
        "the hourly series is not consecutive: {:?}",
        reading
            .hourly
            .iter()
            .map(|hour| hour.hour)
            .collect::<Vec<_>>()
    );

    let face = adapt::weather_face(reading);
    accept("live--weather", &weather::render(&face));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_live_token_card_renders() {
    if !enabled() {
        return;
    }
    let snapshot = tokio::task::spawn_blocking(|| {
        TokenProvider::new(
            EgressHttpClient::new(),
            TokenOptions {
                coin_id: std::env::var("DESKMATE_LIVE_COIN")
                    .unwrap_or_else(|_| "solana".to_owned()),
                currency: "usd".to_owned(),
                refresh_interval: Duration::from_mins(5),
                api_key: std::env::var("DESKMATE_COINGECKO_KEY").ok(),
            },
        )
        .refresh(Utc::now())
    })
    .await
    .expect("the blocking refresh runs");

    assert!(
        snapshot.error.is_none(),
        "the live token fetch failed: {:?}",
        snapshot.error
    );
    let quote = &snapshot.value;
    println!(
        "token: {} ({}) {} {:.6} {:+.2}% L{:.6} H{:.6} samples {}",
        quote.symbol,
        quote.name,
        quote.currency,
        quote.price,
        quote.change_percent_24h,
        quote.low_24h,
        quote.high_24h,
        quote.series.len(),
    );
    assert!(quote.price > 0.0, "a live price is positive");
    assert!(!quote.symbol.is_empty(), "the ticker arrived");
    assert!(
        quote.low_24h <= quote.price && quote.price <= quote.high_24h,
        "the price sits outside its own 24h range: {} not in {}..={}",
        quote.price,
        quote.low_24h,
        quote.high_24h
    );

    let face = adapt::token_face(quote);
    accept("live--token", &token::render(&face));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_live_rss_card_renders() {
    if !enabled() {
        return;
    }
    let url = std::env::var("DESKMATE_LIVE_FEED")
        .unwrap_or_else(|_| "https://news.ycombinator.com/rss".to_owned());
    let title =
        std::env::var("DESKMATE_LIVE_FEED_TITLE").unwrap_or_else(|_| "Hacker News".to_owned());
    let feed_url = url.clone();
    let snapshot = tokio::task::spawn_blocking(move || {
        RssProvider::new(
            EgressHttpClient::new(),
            RssOptions {
                url: feed_url,
                maximum_items: 5,
                refresh_interval: Duration::from_mins(15),
            },
        )
        .refresh(Utc::now())
    })
    .await
    .expect("the blocking refresh runs");

    assert!(
        snapshot.error.is_none(),
        "the live feed fetch failed: {:?}",
        snapshot.error
    );
    let feed = &snapshot.value;
    println!("rss: {} items from {url}", feed.items.len());
    for item in &feed.items {
        println!("  {:?} {:?}", item.published, item.title);
    }
    assert!(!feed.items.is_empty(), "the live feed carried no items");
    // Dates are the part most likely to be wrong against a real feed: the
    // fixtures use one format and the wild uses both.
    let dated = feed
        .items
        .iter()
        .filter(|item| item.published.is_some())
        .count();
    assert!(dated > 0, "no item carried a date, so no age can be shown");

    let face = adapt::rss_face(feed, &title, Utc::now());
    let aged = face
        .entries
        .iter()
        .filter(|entry| !entry.age.is_empty())
        .count();
    assert_eq!(
        aged, dated,
        "every dated item should have produced an age; {dated} dated but {aged} rendered"
    );
    accept("live--rss", &rss::render(&face));
}
