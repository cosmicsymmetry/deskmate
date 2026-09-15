use std::time::Duration;

use chrono::{DateTime, Utc};
use protocol::truncate_utf8_to_bytes;
use serde_json::Value;

use crate::http::{HttpClient, SystemHttpClient};
use crate::{LastGood, Provider, ProviderError, ProviderSnapshot, RefreshPolicy};

pub const MIN_WEATHER_REFRESH_INTERVAL: Duration = Duration::from_mins(10);
const GEOCODING_ENDPOINT: &str = "https://geocoding-api.open-meteo.com/v1/search";
const FORECAST_ENDPOINT: &str = "https://api.open-meteo.com/v1/forecast";
/// Mirrors the ±2000 tenths bound the firmware declares for the temperature integers
/// in `firmware/main/core/template_fields.c`. The device is the narrower of the two
/// and rejects a whole push over it, so the host must not admit a wider window.
/// How many hours the strip can show. The face draws six columns; asking
/// for a couple more costs nothing and lets a narrower future layout use
/// them without another round trip.
pub const MAX_HOURLY_STEPS: usize = 8;
const MIN_TEMPERATURE_DEGREES: f64 = -200.0;
const MAX_TEMPERATURE_DEGREES: f64 = 200.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeatherUnits {
    Metric,
    Imperial,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeatherOptions {
    pub location: String,
    pub units: WeatherUnits,
    pub refresh_interval: Duration,
    pub title: String,
}

/// One hour of the forecast strip.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WeatherHour {
    /// Hour of day in the location's own timezone, 0..=23.
    pub hour: u32,
    pub temperature_tenths: i64,
    pub weather_code: u16,
    pub is_day: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WeatherReading {
    pub location: String,
    pub temperature_tenths: i64,
    pub apparent_temperature_tenths: i64,
    pub summary: String,
    /// The WMO code itself, not a pre-chosen icon name.
    ///
    /// The retired device templates needed a string they could look up in a
    /// baked sprite table, so this provider used to resolve the code to an
    /// icon name and throw the code away. A server-side face draws the
    /// condition itself and wants the raw code: it distinguishes freezing rain
    /// from rain, which no icon name in the old table did.
    pub weather_code: u16,
    pub is_day: bool,
    pub high_tenths: i64,
    pub low_tenths: i64,
    /// Successive hours starting at the current one. Empty when the provider
    /// returned no usable hourly series; the face says so rather than
    /// inventing one.
    pub hourly: Vec<WeatherHour>,
}

pub struct WeatherProvider<C> {
    client: C,
    options: WeatherOptions,
    state: LastGood<WeatherReading>,
}

impl<C: HttpClient> WeatherProvider<C> {
    pub fn new(client: C, options: WeatherOptions) -> Self {
        Self {
            client,
            options,
            state: LastGood::default(),
        }
    }
}

impl WeatherProvider<SystemHttpClient> {
    pub fn system(options: WeatherOptions) -> Self {
        Self::new(SystemHttpClient::default(), options)
    }
}

impl<C: HttpClient> Provider for WeatherProvider<C> {
    type Output = WeatherReading;

    fn refresh_policy(&self) -> RefreshPolicy {
        RefreshPolicy::Interval(
            self.options
                .refresh_interval
                .max(MIN_WEATHER_REFRESH_INTERVAL),
        )
    }

    fn refresh(&mut self, now: DateTime<Utc>) -> ProviderSnapshot<Self::Output> {
        let result = fetch_weather(&mut self.client, &self.options);
        self.state.complete(now, result)
    }
}

fn fetch_weather(
    client: &mut impl HttpClient,
    options: &WeatherOptions,
) -> Result<WeatherReading, ProviderError> {
    if options.location.trim().is_empty() {
        return Err(ProviderError::InvalidConfiguration(
            "weather location is empty".into(),
        ));
    }
    let geocoding_url = endpoint_with_query(
        GEOCODING_ENDPOINT,
        &[
            ("name", options.location.trim()),
            ("count", "1"),
            ("language", "en"),
            ("format", "json"),
        ],
    )?;
    let geocoding = client.get_text(&geocoding_url)?;
    let geocoding: Value = serde_json::from_str(&geocoding)
        .map_err(|_| ProviderError::MalformedFeed("weather location JSON is invalid".into()))?;
    let location = geocoding
        .get("results")
        .and_then(Value::as_array)
        .and_then(|results| results.first())
        .ok_or_else(|| ProviderError::MalformedFeed("weather location was not found".into()))?;
    let latitude = finite_number(location, "latitude")?;
    let longitude = finite_number(location, "longitude")?;
    if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
        return Err(ProviderError::MalformedFeed(
            "weather location coordinates are invalid".into(),
        ));
    }
    let name = location
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(options.location.trim());
    let country = location.get("country").and_then(Value::as_str);
    let display_location =
        country.map_or_else(|| name.to_owned(), |country| format!("{name}, {country}"));

    let latitude = latitude.to_string();
    let longitude = longitude.to_string();
    let mut query = vec![
        ("latitude", latitude.as_str()),
        ("longitude", longitude.as_str()),
        (
            "current",
            "temperature_2m,apparent_temperature,weather_code,is_day",
        ),
        ("daily", "temperature_2m_max,temperature_2m_min"),
        ("hourly", "temperature_2m,weather_code,is_day"),
        // Two days, because the strip starts at the current hour and has to
        // keep filling across local midnight. One day leaves the 21:00 panel
        // with three columns.
        ("forecast_days", "2"),
        // The hourly labels are hours of the day on a physical panel in a
        // room, so they are the location's hours. Asking for UTC and
        // converting here would mean shipping a timezone database to solve a
        // problem the API already solves.
        ("timezone", "auto"),
    ];
    if options.units == WeatherUnits::Imperial {
        query.push(("temperature_unit", "fahrenheit"));
    }
    let forecast_url = endpoint_with_query(FORECAST_ENDPOINT, &query)?;
    let forecast = client.get_text(&forecast_url)?;
    parse_forecast(&forecast, &display_location, options.units)
}

fn parse_forecast(
    body: &str,
    location: &str,
    units: WeatherUnits,
) -> Result<WeatherReading, ProviderError> {
    let _ = units;
    let document: Value = serde_json::from_str(body)
        .map_err(|_| ProviderError::MalformedFeed("weather forecast JSON is invalid".into()))?;
    let current = document.get("current").ok_or_else(|| {
        ProviderError::MalformedFeed("weather forecast has no current data".into())
    })?;
    let temperature = bounded_temperature(finite_number(current, "temperature_2m")?)?;
    let apparent = bounded_temperature(finite_number(current, "apparent_temperature")?)?;
    let weather_code = weather_code_of(current)?;
    let is_day = is_day_of(current);
    let (summary, _) = describe_weather(weather_code, is_day);

    // The daily extremes are the first entry of the two-day window, which is
    // today. A forecast that omits them is not an error: the face falls back
    // to the current reading for both, which is true if uninformative.
    let (high, low) = daily_extremes(&document).unwrap_or((temperature, temperature));

    Ok(WeatherReading {
        location: truncate_utf8_to_bytes(location, 64).to_owned(),
        temperature_tenths: to_tenths(temperature)?,
        apparent_temperature_tenths: to_tenths(apparent)?,
        summary: summary.into(),
        weather_code,
        is_day,
        high_tenths: to_tenths(high)?,
        low_tenths: to_tenths(low)?,
        hourly: hourly_series(&document, current.get("time").and_then(Value::as_str)),
    })
}

/// The temperature window this provider admits.
///
/// It was inherited from the retired `icon-badge-text` firmware schema, which
/// declared these integers as -2000..=2000 tenths and refused a whole push
/// outside it. That schema is gone with protocol v2's template registry, so
/// nothing on the wire enforces it now -- but a sanity bound is still worth
/// keeping, because the alternative to refusing a nonsense reading is drawing
/// it two metres wide on a panel.
fn bounded_temperature(value: f64) -> Result<f64, ProviderError> {
    if (MIN_TEMPERATURE_DEGREES..=MAX_TEMPERATURE_DEGREES).contains(&value) {
        Ok(value)
    } else {
        Err(ProviderError::MalformedFeed(
            "weather temperature is outside supported bounds".into(),
        ))
    }
}

fn weather_code_of(object: &Value) -> Result<u16, ProviderError> {
    object
        .get("weather_code")
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .ok_or_else(|| ProviderError::MalformedFeed("weather code is invalid".into()))
}

fn is_day_of(object: &Value) -> bool {
    match object.get("is_day") {
        Some(Value::Bool(value)) => *value,
        Some(value) => value.as_i64() == Some(1),
        None => false,
    }
}

fn daily_extremes(document: &Value) -> Option<(f64, f64)> {
    let daily = document.get("daily")?;
    let high = daily
        .get("temperature_2m_max")?
        .as_array()?
        .first()?
        .as_f64()
        .filter(|value| value.is_finite())?;
    let low = daily
        .get("temperature_2m_min")?
        .as_array()?
        .first()?
        .as_f64()
        .filter(|value| value.is_finite())?;
    // A provider that swapped them would put the low above the high on the
    // panel, which reads as our bug rather than theirs.
    Some((high.max(low), high.min(low)))
}

/// The successive hours starting at the current one.
///
/// Open-Meteo returns the whole two-day window, so the series has to be
/// aligned to now: the first entry whose timestamp matches the current hour.
/// Falling back to the start of the array would show a panel at 21:00 the
/// forecast for midnight last night.
fn hourly_series(document: &Value, current_time: Option<&str>) -> Vec<WeatherHour> {
    let Some(hourly) = document.get("hourly") else {
        return Vec::new();
    };
    let (Some(times), Some(temperatures), Some(codes)) = (
        hourly.get("time").and_then(Value::as_array),
        hourly.get("temperature_2m").and_then(Value::as_array),
        hourly.get("weather_code").and_then(Value::as_array),
    ) else {
        return Vec::new();
    };
    let days = hourly.get("is_day").and_then(Value::as_array);

    let start = current_time
        .and_then(|now| {
            let hour_of_now = now.get(..13)?;
            times.iter().position(|time| {
                time.as_str()
                    .is_some_and(|time| time.starts_with(hour_of_now))
            })
        })
        .unwrap_or(0);

    times
        .iter()
        .enumerate()
        .skip(start)
        .take(MAX_HOURLY_STEPS)
        .filter_map(|(index, time)| {
            let hour = hour_of(time.as_str()?)?;
            let temperature = temperatures
                .get(index)?
                .as_f64()
                .filter(|value| value.is_finite())
                .and_then(|value| bounded_temperature(value).ok())?;
            let code = codes
                .get(index)?
                .as_u64()
                .and_then(|value| u16::try_from(value).ok())?;
            let is_day = days.and_then(|days| days.get(index)).is_none_or(|value| {
                matches!(value, Value::Bool(true)) || value.as_i64() == Some(1)
            });
            Some(WeatherHour {
                hour,
                temperature_tenths: to_tenths(temperature).ok()?,
                weather_code: code,
                is_day,
            })
        })
        .collect()
}

/// Reads the hour out of an ISO-8601 local timestamp such as
/// `2026-09-12T14:00`.
fn hour_of(timestamp: &str) -> Option<u32> {
    timestamp
        .get(11..13)?
        .parse()
        .ok()
        .filter(|hour| *hour < 24)
}

fn endpoint_with_query(endpoint: &str, values: &[(&str, &str)]) -> Result<String, ProviderError> {
    let mut url = url::Url::parse(endpoint)
        .map_err(|_| ProviderError::InvalidConfiguration("weather endpoint is invalid".into()))?;
    url.query_pairs_mut().extend_pairs(values.iter().copied());
    Ok(url.into())
}

fn finite_number(object: &Value, key: &str) -> Result<f64, ProviderError> {
    object
        .get(key)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .ok_or_else(|| ProviderError::MalformedFeed("weather numeric field is invalid".into()))
}

fn to_tenths(value: f64) -> Result<i64, ProviderError> {
    let scaled = (value * 10.0).round();
    format!("{scaled:.0}").parse().map_err(|_| {
        ProviderError::MalformedFeed("weather numeric field is outside supported bounds".into())
    })
}

fn describe_weather(code: u16, is_day: bool) -> (&'static str, &'static str) {
    match code {
        0 if is_day => ("Clear", "sun"),
        0 => ("Clear", "moon"),
        1 | 2 if is_day => ("Partly cloudy", "cloud-sun"),
        1 | 2 => ("Partly cloudy", "cloud-moon"),
        3 => ("Overcast", "cloud"),
        45 | 48 => ("Fog", "fog"),
        51 | 53 | 55 | 56 | 57 => ("Drizzle", "drizzle"),
        61 | 63 | 65 | 66 | 67 | 80 | 81 | 82 => ("Rain", "rain"),
        71 | 73 | 75 | 77 | 85 | 86 => ("Snow", "snow"),
        95 | 96 | 99 => ("Thunderstorm", "storm"),
        _ => ("Unknown", "unknown"),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use chrono::TimeZone;

    use super::*;

    struct FakeClient {
        responses: VecDeque<Result<String, ProviderError>>,
        urls: Vec<String>,
    }

    impl HttpClient for FakeClient {
        fn get_text(&mut self, url: &str) -> Result<String, ProviderError> {
            self.urls.push(url.into());
            self.responses.pop_front().unwrap()
        }
    }

    fn options(units: WeatherUnits) -> WeatherOptions {
        WeatherOptions {
            location: "Tbilisi".into(),
            units,
            refresh_interval: Duration::from_mins(1),
            title: "Weather".into(),
        }
    }

    #[test]
    fn typed_adapter_geocodes_and_requests_explicit_units() {
        let client = FakeClient {
            responses: VecDeque::from([
                Ok(include_str!("../tests/fixtures/weather-location.json").into()),
                Ok(include_str!("../tests/fixtures/weather-current.json").into()),
            ]),
            urls: Vec::new(),
        };
        let mut provider = WeatherProvider::new(client, options(WeatherUnits::Imperial));
        assert_eq!(
            provider.refresh_policy(),
            RefreshPolicy::Interval(MIN_WEATHER_REFRESH_INTERVAL)
        );
        let snapshot = provider.refresh(Utc.with_ymd_and_hms(2026, 8, 5, 10, 0, 0).unwrap());
        assert!(!snapshot.stale);
        assert_eq!(snapshot.value.location, "Tbilisi, Georgia");
        assert_eq!(snapshot.value.temperature_tenths, 725);
        assert_eq!(snapshot.value.summary, "Partly cloudy");
        assert!(provider.client.urls[0].contains("name=Tbilisi"));
        assert!(provider.client.urls[1].contains("temperature_unit=fahrenheit"));
        assert!(
            !provider
                .client
                .urls
                .iter()
                .any(|url| url.contains("apikey"))
        );
    }

    #[test]
    fn malformed_or_missing_data_has_a_stable_category() {
        for malformed in ["not json", r#"{"results":[]}"#] {
            let client = FakeClient {
                responses: VecDeque::from([Ok(malformed.into())]),
                urls: Vec::new(),
            };
            let mut provider = WeatherProvider::new(client, options(WeatherUnits::Metric));
            let snapshot = provider.refresh(Utc::now());
            assert!(snapshot.stale);
            assert!(
                snapshot
                    .error
                    .unwrap()
                    .starts_with("malformed provider data:")
            );
        }
    }

    /// The host temperature window must match the ±2000 tenths the firmware declares.
    /// It used to be ±250.0°, so a reading between 200.0° and 250.0° passed every host
    /// check and then made the device reject the weather card's entire push.
    #[test]
    fn temperature_window_matches_the_firmware_declared_range() {
        let forecast = |celsius: f64| {
            format!(
                r#"{{"current":{{"temperature_2m":{celsius},"apparent_temperature":{celsius},"weather_code":3,"is_day":1}}}}"#
            )
        };
        let accepted = parse_forecast(&forecast(200.0), "Nowhere", WeatherUnits::Metric).unwrap();
        assert_eq!(accepted.temperature_tenths, 2000);
        assert_eq!(accepted.apparent_temperature_tenths, 2000);

        for outside in [200.1, -200.1, 240.0] {
            assert!(
                parse_forecast(&forecast(outside), "Nowhere", WeatherUnits::Metric).is_err(),
                "{outside} is outside the firmware's declared range and must be rejected"
            );
        }
    }
}
