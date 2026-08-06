use std::time::Duration;

use chrono::{DateTime, Utc};
use protocol::{Field, FieldValue};
use serde_json::Value;

use crate::http::{HttpClient, SystemHttpClient};
use crate::{LastGood, Provider, ProviderError, ProviderSnapshot, RefreshPolicy, truncate_utf8};

pub const MIN_WEATHER_REFRESH_INTERVAL: Duration = Duration::from_mins(10);
const GEOCODING_ENDPOINT: &str = "https://geocoding-api.open-meteo.com/v1/search";
const FORECAST_ENDPOINT: &str = "https://api.open-meteo.com/v1/forecast";

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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WeatherReading {
    pub location: String,
    pub temperature_tenths: i64,
    pub apparent_temperature_tenths: i64,
    pub weather_code: u16,
    pub is_day: bool,
    pub summary: String,
    pub icon: String,
    pub unit: String,
}

impl WeatherReading {
    pub fn fields(&self, title: &str, stale: bool, error: Option<&str>) -> Vec<Field> {
        let rounded = if self.temperature_tenths >= 0 {
            (self.temperature_tenths + 5) / 10
        } else {
            (self.temperature_tenths - 5) / 10
        };
        vec![
            text_field("title", truncate_utf8(title, 64)),
            text_field("value", format!("{rounded}°")),
            text_field("label", truncate_utf8(&self.summary, 64)),
            text_field("badge", truncate_utf8(&self.location, 64)),
            text_field("icon", self.icon.clone()),
            Field {
                key: "temperature_tenths".into(),
                value: FieldValue::Integer(self.temperature_tenths),
            },
            Field {
                key: "apparent_temperature_tenths".into(),
                value: FieldValue::Integer(self.apparent_temperature_tenths),
            },
            text_field("unit", self.unit.clone()),
            Field {
                key: "stale".into(),
                value: FieldValue::Boolean(stale),
            },
            text_field(
                "error",
                error.map_or_else(String::new, |value| truncate_utf8(value, 96)),
            ),
        ]
    }
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

    pub fn fields(&self, snapshot: &ProviderSnapshot<WeatherReading>) -> Vec<Field> {
        snapshot.value.fields(
            &self.options.title,
            snapshot.stale,
            snapshot.error.as_deref(),
        )
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
        ("forecast_days", "1"),
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
    let document: Value = serde_json::from_str(body)
        .map_err(|_| ProviderError::MalformedFeed("weather forecast JSON is invalid".into()))?;
    let current = document.get("current").ok_or_else(|| {
        ProviderError::MalformedFeed("weather forecast has no current data".into())
    })?;
    let temperature = finite_number(current, "temperature_2m")?;
    let apparent = finite_number(current, "apparent_temperature")?;
    if !(-250.0..=250.0).contains(&temperature) || !(-250.0..=250.0).contains(&apparent) {
        return Err(ProviderError::MalformedFeed(
            "weather temperature is outside supported bounds".into(),
        ));
    }
    let weather_code = current
        .get("weather_code")
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .ok_or_else(|| ProviderError::MalformedFeed("weather code is invalid".into()))?;
    let is_day = match current.get("is_day") {
        Some(Value::Bool(value)) => *value,
        Some(value) => value.as_i64() == Some(1),
        None => false,
    };
    let (summary, icon) = describe_weather(weather_code, is_day);
    Ok(WeatherReading {
        location: truncate_utf8(location, 64),
        temperature_tenths: to_tenths(temperature)?,
        apparent_temperature_tenths: to_tenths(apparent)?,
        weather_code,
        is_day,
        summary: summary.into(),
        icon: icon.into(),
        unit: match units {
            WeatherUnits::Metric => "celsius",
            WeatherUnits::Imperial => "fahrenheit",
        }
        .into(),
    })
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

fn text_field(key: impl Into<String>, value: String) -> Field {
    Field {
        key: key.into(),
        value: FieldValue::Text(value),
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
}
