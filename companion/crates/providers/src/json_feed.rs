use std::collections::HashSet;
use std::time::Duration;

use chrono::{DateTime, Utc};
use protocol::{Field, FieldValue, MAX_FIELD_COUNT, MAX_FIELD_TEXT_LEN, truncate_utf8_to_bytes};
use serde_json::Value;

use crate::http::{HttpClient, SystemHttpClient};
use crate::{LastGood, Provider, ProviderError, ProviderSnapshot, RefreshPolicy, text_field};

pub const MAX_JSON_MAPPINGS: usize = 16;
pub const MAX_JSON_PATH_SEGMENTS: usize = 16;
pub const MAX_JSON_ARRAY_INDEX: usize = 1_023;

pub fn is_reserved_field(field: &str) -> bool {
    matches!(field, "title" | "stale" | "error")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonMapping {
    pub field: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonFeedOptions {
    pub url: String,
    pub mappings: Vec<JsonMapping>,
    pub refresh_interval: Duration,
    pub title: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JsonFeed {
    pub fields: Vec<Field>,
}

impl JsonFeed {
    pub fn wire_fields(&self, title: &str, stale: bool, error: Option<&str>) -> Vec<Field> {
        // Firmware v1 permits 16 fields. Reserve title/stale/error and retain mapping
        // order deterministically. Which template field a mapping lands on is the
        // user's choice; a mapping naming no declared field is counted as unknown by
        // the device and ignored, never rejected.
        let mut fields = Vec::with_capacity(MAX_FIELD_COUNT);
        fields.push(text_field("title", truncate_utf8_to_bytes(title, 64)));
        fields.extend(
            self.fields
                .iter()
                .filter(|field| !is_reserved_field(&field.key))
                .take(MAX_FIELD_COUNT - 3)
                .cloned(),
        );
        fields.push(Field {
            key: "stale".into(),
            value: FieldValue::Boolean(stale),
        });
        fields.push(text_field(
            "error",
            error.map_or("", |value| truncate_utf8_to_bytes(value, 96)),
        ));
        fields
    }
}

pub struct JsonFeedProvider<C> {
    client: C,
    options: JsonFeedOptions,
    state: LastGood<JsonFeed>,
}

impl<C: HttpClient> JsonFeedProvider<C> {
    pub fn new(client: C, options: JsonFeedOptions) -> Self {
        Self {
            client,
            options,
            state: LastGood::default(),
        }
    }

    pub fn fields(&self, snapshot: &ProviderSnapshot<JsonFeed>) -> Vec<Field> {
        snapshot.value.wire_fields(
            &self.options.title,
            snapshot.stale,
            snapshot.error.as_deref(),
        )
    }
}

impl JsonFeedProvider<SystemHttpClient> {
    pub fn system(options: JsonFeedOptions) -> Self {
        Self::new(SystemHttpClient::default(), options)
    }
}

impl<C: HttpClient> Provider for JsonFeedProvider<C> {
    type Output = JsonFeed;

    fn refresh_policy(&self) -> RefreshPolicy {
        RefreshPolicy::Interval(self.options.refresh_interval)
    }

    fn refresh(&mut self, now: DateTime<Utc>) -> ProviderSnapshot<Self::Output> {
        let result = self
            .client
            .get_text(&self.options.url)
            .and_then(|body| parse_json_feed(&body, &self.options.mappings));
        self.state.complete(now, result)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PathSegment {
    Key(String),
    Index(usize),
}

pub fn parse_json_feed(body: &str, mappings: &[JsonMapping]) -> Result<JsonFeed, ProviderError> {
    if mappings.len() > MAX_JSON_MAPPINGS {
        return Err(ProviderError::InvalidConfiguration(
            "too many JSON mappings".into(),
        ));
    }
    let document: Value = serde_json::from_str(body)
        .map_err(|_| ProviderError::MalformedFeed("JSON syntax is invalid".into()))?;
    let mut fields = Vec::with_capacity(mappings.len());
    let mut names = HashSet::with_capacity(mappings.len());
    for mapping in mappings {
        if mapping.field.is_empty()
            || is_reserved_field(&mapping.field)
            || !names.insert(mapping.field.as_str())
        {
            return Err(ProviderError::InvalidConfiguration(
                "JSON mapping fields must be unique, nonempty, and not reserved".into(),
            ));
        }
        let path = parse_path(&mapping.path)?;
        let value = path
            .iter()
            .try_fold(&document, |current, segment| match segment {
                PathSegment::Key(key) => current.get(key),
                PathSegment::Index(index) => current.get(*index),
            });
        let value = value.ok_or_else(|| {
            ProviderError::MalformedFeed("JSON mapping did not resolve to a value".into())
        })?;
        fields.push(Field {
            key: mapping.field.clone(),
            value: scalar_field_value(&mapping.field, value)?,
        });
    }
    Ok(JsonFeed { fields })
}

/// Every mapped scalar is emitted as **text**, whatever its JSON type.
///
/// No display template exposes a user-mappable integer or boolean field: the only
/// non-text fields the firmware declares are `stale` (reserved for the provider),
/// progress-ring's timer integers (device-local, never JSON-fed), and weather's
/// `*_temperature_tenths` (emitted by the weather provider, not by a mapping). The
/// firmware rejects the WHOLE push when an incoming field's type does not match the
/// declared one, so a `{"count": 42}` mapping arriving as an integer took every other
/// field on that card down with it. Text always matches.
fn scalar_field_value(key: &str, value: &Value) -> Result<FieldValue, ProviderError> {
    let text = match value {
        Value::Null => String::new(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(_) | Value::Object(_) => {
            return Err(ProviderError::MalformedFeed(
                "JSON mapping must resolve to a scalar".into(),
            ));
        }
    };
    Ok(FieldValue::Text(
        truncate_utf8_to_bytes(&text, declared_text_capacity(key)).to_owned(),
    ))
}

/// The text capacity the firmware's field registry declares for `key`
/// (`firmware/main/core/template_fields.c`).
///
/// Over-long text is rejected per push, not per field, so a 40-character feed string
/// mapped onto big-number-label's 16-byte `value` would discard the card's whole
/// update. Truncating host-side keeps the push legal and shows the user a clipped
/// value instead of nothing. Keys no template declares are ignored by the device as
/// unknown fields and can never trigger a rejection, so they only need the generic
/// wire bound. This can be a flat table because no key is declared with two different
/// capacities across templates.
fn declared_text_capacity(key: &str) -> usize {
    match key {
        // big-number-label `value`; icon-badge-text `value`/`icon`/`unit`.
        "value" | "icon" | "unit" => 16,
        // Every template's `title`, plus big-number-label/icon-badge-text `label`,
        // progress-ring `label`, and icon-badge-text `badge`.
        "title" | "label" | "badge" => 64,
        _ => match key
            .strip_prefix("row")
            .and_then(|rest| rest.split_once('_'))
        {
            // row-list declares five rows of `rowN_title` (96) and `rowN_time` (32).
            Some((index, "title")) if is_row_index(index) => 96,
            Some((index, "time")) if is_row_index(index) => 32,
            _ => MAX_FIELD_TEXT_LEN,
        },
    }
}

fn is_row_index(index: &str) -> bool {
    matches!(index, "0" | "1" | "2" | "3" | "4")
}

fn parse_path(raw: &str) -> Result<Vec<PathSegment>, ProviderError> {
    let bytes = raw.as_bytes();
    if bytes.first() != Some(&b'$') {
        return Err(invalid_path());
    }
    let mut segments = Vec::new();
    let mut index = 1;
    while index < bytes.len() {
        if segments.len() >= MAX_JSON_PATH_SEGMENTS {
            return Err(ProviderError::InvalidConfiguration(
                "JSON path has too many segments".into(),
            ));
        }
        match bytes[index] {
            b'.' => {
                index += 1;
                let start = index;
                while index < bytes.len()
                    && (bytes[index].is_ascii_alphanumeric() || matches!(bytes[index], b'_' | b'-'))
                {
                    index += 1;
                }
                if start == index {
                    return Err(invalid_path());
                }
                segments.push(PathSegment::Key(raw[start..index].into()));
            }
            b'[' => {
                index += 1;
                let start = index;
                while index < bytes.len() && bytes[index].is_ascii_digit() {
                    index += 1;
                }
                if start == index || bytes.get(index) != Some(&b']') {
                    return Err(invalid_path());
                }
                let array_index = raw[start..index]
                    .parse::<usize>()
                    .ok()
                    .filter(|value| *value <= MAX_JSON_ARRAY_INDEX)
                    .ok_or_else(invalid_path)?;
                index += 1;
                segments.push(PathSegment::Index(array_index));
            }
            _ => return Err(invalid_path()),
        }
    }
    Ok(segments)
}

fn invalid_path() -> ProviderError {
    ProviderError::InvalidConfiguration("JSON path must use bounded $.field[0] traversal".into())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use chrono::TimeZone;

    use super::*;

    struct FakeClient(VecDeque<Result<String, ProviderError>>);

    impl HttpClient for FakeClient {
        fn get_text(&mut self, _url: &str) -> Result<String, ProviderError> {
            self.0.pop_front().unwrap()
        }
    }

    fn mappings() -> Vec<JsonMapping> {
        vec![
            JsonMapping {
                field: "value".into(),
                path: "$.metrics[0].value".into(),
            },
            JsonMapping {
                field: "label".into(),
                path: "$.metrics[0].label".into(),
            },
            JsonMapping {
                field: "ready".into(),
                path: "$.ready".into(),
            },
        ]
    }

    #[test]
    fn extracts_only_bounded_declarative_scalar_paths() {
        let feed = parse_json_feed(
            include_str!("../tests/fixtures/json-feed.json"),
            &mappings(),
        )
        .unwrap();
        assert_eq!(feed.fields[0].value, FieldValue::Text("42".into()));
        assert_eq!(feed.fields[1].value, FieldValue::Text("Builds".into()));
        assert_eq!(feed.fields[2].value, FieldValue::Text("true".into()));
        assert!(
            parse_json_feed(
                r#"{"items":[1]}"#,
                &[JsonMapping {
                    field: "bad".into(),
                    path: "$..items".into(),
                }]
            )
            .is_err()
        );
        assert!(
            parse_json_feed(
                r#"{"items":[1]}"#,
                &[JsonMapping {
                    field: "bad".into(),
                    path: "$.items".into(),
                }]
            )
            .is_err()
        );
        assert!(
            parse_json_feed(
                r#"{"title":"shadowed"}"#,
                &[JsonMapping {
                    field: "title".into(),
                    path: "$.title".into(),
                }]
            )
            .is_err()
        );
    }

    #[test]
    fn preserves_last_good_fields_through_failure_and_recovery() {
        let mut provider = JsonFeedProvider::new(
            FakeClient(VecDeque::from([
                Ok(r#"{"metrics":[{"value":42,"label":"Builds"}],"ready":true}"#.into()),
                Err(ProviderError::Timeout),
                Ok(r#"{"metrics":[{"value":43,"label":"Builds"}],"ready":true}"#.into()),
            ])),
            JsonFeedOptions {
                url: "https://example.test/feed".into(),
                mappings: mappings(),
                refresh_interval: Duration::from_mins(5),
                title: "CI".into(),
            },
        );
        let first = provider.refresh(Utc.with_ymd_and_hms(2026, 8, 5, 10, 0, 0).unwrap());
        let stale = provider.refresh(Utc.with_ymd_and_hms(2026, 8, 5, 10, 2, 0).unwrap());
        assert!(stale.stale);
        assert_eq!(stale.value, first.value);
        assert_eq!(stale.age, Some(Duration::from_mins(2)));
        let recovered = provider.refresh(Utc.with_ymd_and_hms(2026, 8, 5, 10, 3, 0).unwrap());
        assert!(!recovered.stale);
        assert_eq!(
            recovered.value.fields[0].value,
            FieldValue::Text("43".into())
        );
    }

    /// Regression test for the final-review finding that the default json-feed card
    /// (`big-number-label`, whose only renderable data field is the 16-byte text
    /// `value`) could never render an integer or boolean mapping: the provider emitted
    /// `FieldValue::Integer`/`Boolean`, the firmware rejected the whole push on the type
    /// mismatch, and the runtime re-attempted it forever. Every mapped scalar is text
    /// now, and text landing on a declared field is truncated to that field's declared
    /// capacity so an over-long value clips instead of failing the push.
    #[test]
    fn every_mapped_scalar_is_emitted_as_bounded_text() {
        let body = format!(
            r#"{{"count":42,"ratio":3.5,"ready":false,"nothing":null,"long":"{}"}}"#,
            "x".repeat(64)
        );
        let feed = parse_json_feed(
            &body,
            &[
                mapping("value", "$.count"),
                mapping("label", "$.ratio"),
                mapping("badge", "$.ready"),
                mapping("icon", "$.nothing"),
                mapping("row0_time", "$.long"),
                mapping("undeclared", "$.long"),
            ],
        )
        .unwrap();

        assert_eq!(feed.fields[0].value, FieldValue::Text("42".into()));
        assert_eq!(feed.fields[1].value, FieldValue::Text("3.5".into()));
        assert_eq!(feed.fields[2].value, FieldValue::Text("false".into()));
        assert_eq!(feed.fields[3].value, FieldValue::Text(String::new()));
        // `row0_time` declares 32 bytes; an undeclared key is ignored by the device
        // and only needs the generic 128-byte wire bound.
        assert_eq!(feed.fields[4].value, FieldValue::Text("x".repeat(32)));
        assert_eq!(feed.fields[5].value, FieldValue::Text("x".repeat(64)));

        // An over-long value on big-number-label's 16-byte `value` clips rather than
        // making the firmware refuse the card's entire push.
        let long_value =
            parse_json_feed(r#"{"n":1234567890123456789}"#, &[mapping("value", "$.n")]).unwrap();
        assert_eq!(
            long_value.fields[0].value,
            FieldValue::Text("1234567890123456".into())
        );
    }

    fn mapping(field: &str, path: &str) -> JsonMapping {
        JsonMapping {
            field: field.into(),
            path: path.into(),
        }
    }
}
