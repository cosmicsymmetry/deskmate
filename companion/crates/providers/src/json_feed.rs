use std::collections::HashSet;
use std::time::Duration;

use chrono::{DateTime, Utc};
use protocol::{Field, FieldValue, MAX_FIELD_COUNT, MAX_FIELD_TEXT_LEN};
use serde_json::Value;

use crate::http::{HttpClient, SystemHttpClient};
use crate::{LastGood, Provider, ProviderError, ProviderSnapshot, RefreshPolicy, truncate_utf8};

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
        // order deterministically; template validation narrows mappings further in Task 3.
        let mut fields = Vec::with_capacity(MAX_FIELD_COUNT);
        fields.push(text_field("title", truncate_utf8(title, 64)));
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
            error.map_or_else(String::new, |value| truncate_utf8(value, 96)),
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
            value: scalar_field_value(value)?,
        });
    }
    Ok(JsonFeed { fields })
}

fn scalar_field_value(value: &Value) -> Result<FieldValue, ProviderError> {
    match value {
        Value::Null => Ok(FieldValue::Text(String::new())),
        Value::Bool(value) => Ok(FieldValue::Boolean(*value)),
        Value::Number(value) => value.as_i64().map_or_else(
            || {
                Ok(FieldValue::Text(truncate_utf8(
                    &value.to_string(),
                    MAX_FIELD_TEXT_LEN,
                )))
            },
            |value| Ok(FieldValue::Integer(value)),
        ),
        Value::String(value) => Ok(FieldValue::Text(truncate_utf8(value, MAX_FIELD_TEXT_LEN))),
        Value::Array(_) | Value::Object(_) => Err(ProviderError::MalformedFeed(
            "JSON mapping must resolve to a scalar".into(),
        )),
    }
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
        assert_eq!(feed.fields[0].value, FieldValue::Integer(42));
        assert_eq!(feed.fields[1].value, FieldValue::Text("Builds".into()));
        assert_eq!(feed.fields[2].value, FieldValue::Boolean(true));
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
        assert_eq!(recovered.value.fields[0].value, FieldValue::Integer(43));
    }
}
