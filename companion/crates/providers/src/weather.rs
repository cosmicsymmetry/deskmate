use protocol::truncate_utf8_to_bytes;
use serde_json::Value;

use crate::http::HttpClient;
use crate::{ProviderError, ProviderSnapshot};

const GEOCODING_ENDPOINT: &str = "https://geocoding-api.open-meteo.com/v1/search";
const FORECAST_ENDPOINT: &str = "https://api.open-meteo.com/v1/forecast";
/// Rejects implausible provider data before it enters the application state.
const MIN_TEMPERATURE_DEGREES: f64 = -200.0;
const MAX_TEMPERATURE_DEGREES: f64 = 200.0;
/// How many hours the strip can show. The face draws six columns; asking
/// for a couple more costs nothing and lets a narrower future layout use
/// them without another round trip.
const MAX_HOURLY_STEPS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeatherUnits {
    Metric,
    Imperial,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeatherOptions {
    pub location: String,
    pub units: WeatherUnits,
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
    pub summary: String,
    /// The raw WMO code lets the face choose day/night visuals while preserving
    /// distinctions such as freezing rain versus rain.
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
}

impl<C: HttpClient> WeatherProvider<C> {
    pub fn new(client: C, options: WeatherOptions) -> Self {
        Self { client, options }
    }

    pub fn refresh(&mut self) -> ProviderSnapshot<WeatherReading> {
        let result = fetch_weather(&mut self.client, &self.options);
        ProviderSnapshot::from_result(result)
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
    parse_forecast(&forecast, &display_location)
}

fn parse_forecast(body: &str, location: &str) -> Result<WeatherReading, ProviderError> {
    let document: Value = serde_json::from_str(body)
        .map_err(|_| ProviderError::MalformedFeed("weather forecast JSON is invalid".into()))?;
    let current = document.get("current").ok_or_else(|| {
        ProviderError::MalformedFeed("weather forecast has no current data".into())
    })?;
    let temperature = bounded_temperature(finite_number(current, "temperature_2m")?)?;
    let apparent = bounded_temperature(finite_number(current, "apparent_temperature")?)?;
    let weather_code = weather_code_of(current)?;
    let is_day = is_day_of(current);
    let summary = weather_summary(weather_code);

    // The daily extremes are the first entry of the two-day window, which is
    // today. A forecast that omits them is not an error: the face falls back
    // to the current reading for both, which is true if uninformative.
    let (high, low) = daily_extremes(&document).unwrap_or((temperature, temperature));
    let high = bounded_temperature(high)?;
    let low = bounded_temperature(low)?;
    let temperature_tenths = to_tenths(temperature)?;
    // Apparent temperature still validates the forecast even though the face
    // does not display it; malformed data must fail the refresh.
    to_tenths(apparent)?;

    Ok(WeatherReading {
        location: truncate_utf8_to_bytes(location, 64).to_owned(),
        temperature_tenths,
        summary: summary.into(),
        weather_code,
        is_day,
        high_tenths: to_tenths(high)?,
        low_tenths: to_tenths(low)?,
        hourly: hourly_series(&document, current.get("time").and_then(Value::as_str)),
    })
}

/// Applies a broad semantic sanity bound to temperatures returned by the provider.
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

    let Some(start) = current_time.and_then(|now| {
        hour_of(now)?;
        let hour_of_now = now.get(..13)?;
        times.iter().position(|time| {
            time.as_str()
                .is_some_and(|time| time.starts_with(hour_of_now))
        })
    }) else {
        return Vec::new();
    };

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

fn weather_summary(code: u16) -> &'static str {
    match code {
        0 => "Clear",
        1 | 2 => "Partly cloudy",
        3 => "Overcast",
        45 | 48 => "Fog",
        51 | 53 | 55 | 56 | 57 => "Drizzle",
        61 | 63 | 65 | 66 | 67 | 80 | 81 | 82 => "Rain",
        71 | 73 | 75 | 77 | 85 | 86 => "Snow",
        95 | 96 | 99 => "Thunderstorm",
        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

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
        let snapshot = provider.refresh();
        assert!(snapshot.error.is_none());
        let reading = snapshot.value.expect("the forecast arrived");
        assert_eq!(reading.location, "Tbilisi, Georgia");
        assert_eq!(reading.temperature_tenths, 725);
        assert_eq!(reading.summary, "Partly cloudy");
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
            let snapshot = provider.refresh();
            assert!(snapshot.value.is_none());
            assert!(
                snapshot
                    .error
                    .unwrap()
                    .starts_with("malformed provider data:")
            );
        }
    }

    /// Temperatures outside the provider's semantic sanity bounds are rejected.
    #[test]
    fn temperature_window_rejects_implausible_values() {
        let forecast = |celsius: f64| {
            format!(
                r#"{{"current":{{"temperature_2m":{celsius},"apparent_temperature":{celsius},"weather_code":3,"is_day":1}}}}"#
            )
        };
        let accepted = parse_forecast(&forecast(200.0), "Nowhere").unwrap();
        assert_eq!(accepted.temperature_tenths, 2000);

        for outside in [200.1, -200.1, 240.0] {
            assert!(
                parse_forecast(&forecast(outside), "Nowhere").is_err(),
                "{outside} is implausible provider data and must be rejected"
            );
        }
    }

    #[test]
    fn apparent_temperature_is_required_numeric_and_bounded() {
        let forecast = |apparent: Option<Value>| {
            let mut document: Value =
                serde_json::from_str(include_str!("../tests/fixtures/weather-current.json"))
                    .unwrap();
            let current = document["current"].as_object_mut().unwrap();
            current.insert("temperature_2m".into(), Value::from(20));
            if let Some(value) = apparent {
                current.insert("apparent_temperature".into(), value);
            } else {
                current.remove("apparent_temperature");
            }
            document.to_string()
        };
        for (apparent, message) in [
            (None, "weather numeric field is invalid"),
            (Some(Value::Null), "weather numeric field is invalid"),
            (
                Some(Value::from("warm")),
                "weather numeric field is invalid",
            ),
            (
                Some(Value::from(200.1)),
                "weather temperature is outside supported bounds",
            ),
            (
                Some(Value::from(-200.1)),
                "weather temperature is outside supported bounds",
            ),
        ] {
            let error = parse_forecast(&forecast(apparent), "Nowhere").unwrap_err();
            assert_eq!(error, ProviderError::MalformedFeed(message.into()));
            assert_eq!(
                error.to_string(),
                format!("malformed provider data: {message}")
            );
        }
        for apparent in [-200.0, 18.5, 200.0] {
            let reading = parse_forecast(&forecast(Some(Value::from(apparent))), "Nowhere")
                .expect("a valid apparent temperature is accepted");
            assert_eq!(reading.temperature_tenths, 200);
        }
    }

    fn forecast_with_daily(high: f64, low: f64) -> String {
        serde_json::json!({
            "current": {
                "temperature_2m": 12.3,
                "apparent_temperature": 11.0,
                "weather_code": 3,
                "is_day": 1
            },
            "daily": {
                "temperature_2m_max": [high],
                "temperature_2m_min": [low]
            }
        })
        .to_string()
    }

    #[test]
    fn complete_daily_extrema_must_each_be_within_temperature_bounds() {
        for (high, low) in [(200.1, 0.0), (0.0, -200.1)] {
            let error = parse_forecast(&forecast_with_daily(high, low), "Nowhere")
                .expect_err("an out-of-bounds daily extreme must fail the reading");
            assert_eq!(
                error,
                ProviderError::MalformedFeed(
                    "weather temperature is outside supported bounds".into()
                )
            );
        }
    }

    #[test]
    fn daily_extrema_accept_boundaries_and_sort_reversed_values() {
        let boundaries = parse_forecast(&forecast_with_daily(200.0, -200.0), "Nowhere")
            .expect("inclusive temperature bounds are accepted");
        assert_eq!(boundaries.high_tenths, 2000);
        assert_eq!(boundaries.low_tenths, -2000);

        let reversed = parse_forecast(&forecast_with_daily(-10.0, 25.0), "Nowhere")
            .expect("valid reversed extrema are sorted");
        assert_eq!(reversed.high_tenths, 250);
        assert_eq!(reversed.low_tenths, -100);
    }

    #[test]
    fn incomplete_or_malformed_daily_data_still_falls_back_to_current() {
        let current = serde_json::json!({
            "temperature_2m": 12.3,
            "apparent_temperature": 11.0,
            "weather_code": 3,
            "is_day": 1
        });
        let daily_values = [
            None,
            Some(serde_json::json!({"temperature_2m_max": [20.0]})),
            Some(serde_json::json!({
                "temperature_2m_max": [],
                "temperature_2m_min": []
            })),
            Some(serde_json::json!({
                "temperature_2m_max": ["warm"],
                "temperature_2m_min": [5.0]
            })),
        ];
        for daily in daily_values {
            let mut document = serde_json::json!({"current": current});
            if let Some(daily) = daily {
                document["daily"] = daily;
            }
            let reading = parse_forecast(&document.to_string(), "Nowhere")
                .expect("optional malformed daily data falls back to current");
            assert_eq!((reading.high_tenths, reading.low_tenths), (123, 123));
        }
    }

    #[test]
    fn invalid_complete_daily_extrema_fail_without_a_value() {
        let location = include_str!("../tests/fixtures/weather-location.json");
        let client = FakeClient {
            responses: VecDeque::from([
                Ok(location.into()),
                Ok(forecast_with_daily(20.0, 5.0)),
                Ok(location.into()),
                Ok(forecast_with_daily(200.1, 5.0)),
            ]),
            urls: Vec::new(),
        };
        let mut provider = WeatherProvider::new(client, options(WeatherUnits::Metric));
        let first = provider.refresh();
        assert!(first.error.is_none());
        let second = provider.refresh();
        assert!(second.value.is_none());
        assert_eq!(
            second.error.as_deref(),
            Some("malformed provider data: weather temperature is outside supported bounds")
        );
    }

    fn hourly_document() -> Value {
        serde_json::json!({
            "hourly": {
                "time": [
                    "2026-09-12T22:00",
                    "2026-09-12T23:00",
                    "2026-09-13T00:00",
                    "2026-09-13T01:00"
                ],
                "temperature_2m": [18.0, 17.0, 16.0, 15.0],
                "weather_code": [1, 2, 3, 45],
                "is_day": [1, 0, 0, 0]
            }
        })
    }

    #[test]
    fn hourly_series_requires_current_time_to_match() {
        let document = hourly_document();
        for current_time in [None, Some("not-a-time"), Some("2026-09-12T21:30")] {
            assert!(
                hourly_series(&document, current_time).is_empty(),
                "{current_time:?} must not fall back to the first hourly entry"
            );
        }
    }

    #[test]
    fn hourly_series_starts_at_current_local_hour_and_crosses_midnight() {
        let hours = hourly_series(&hourly_document(), Some("2026-09-12T23:45"));
        assert_eq!(hours.first().map(|hour| hour.hour), Some(23));
        assert_eq!(
            hours.iter().map(|hour| hour.hour).collect::<Vec<_>>(),
            vec![23, 0, 1]
        );
    }
}
