use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;

use crate::state::{CardField, CardFieldValue};
use chrono::{DateTime, Offset, Utc};
pub use protocol::MAX_CARD_ID_LEN;
use protocol::{ApplyConfig, CardConfig, PushTimer, TapAction};
use serde::{Deserialize, Deserializer, Serialize};

// Helper module for strict deserialization of internally tagged enums
mod strict_tagged_enum {
    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    #[derive(Debug, Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "kebab-case")]
    pub enum AlertHoldInner {
        UntilDismissed,
        Seconds { value: u16 },
    }

    #[derive(Debug, Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "kebab-case")]
    pub enum CardAlertInner {
        None,
        OnTimerFinish { hold: super::AlertHold },
    }

    #[derive(Debug, Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "kebab-case")]
    pub enum CarouselAdvanceInner {
        Manual,
        Timed { default_dwell_seconds: u16 },
    }

    /// Serde's `deny_unknown_fields` no-op on internally tagged enums affects unit
    /// variants just as much as struct variants (`DigitalClock`, not just
    /// `AnalogClock`), so `DisplayTemplate` needs the same treatment as the
    /// card-behaviour types above.
    #[derive(Debug, Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "kebab-case")]
    pub enum DisplayTemplateInner {
        DigitalClock,
        AnalogClock,
        ProgressRing,
    }

    #[derive(Debug, Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "kebab-case")]
    pub enum WidgetTapActionInner {
        None,
        StartPause,
        Reset,
        Dismiss,
        OpenUrl { url: String },
        OpenApplication { application_id: String },
    }

    #[derive(Debug, Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "kebab-case")]
    pub enum RefreshPolicyInner {
        DeviceLocal,
        Manual,
        Interval { minutes: u16 },
    }

    /// Mirrors `super::AssetKind`. The device now rasterizes any size from a TTF
    /// at runtime, so `font` carries no pixel size and no glyph ranges; `icon-font`
    /// carries a TTF plus a name→codepoint map so a scene can write a symbolic
    /// icon name instead of a raw codepoint; `image` carries no extra fields
    /// because the host converts the source to an LVGL binary image itself.
    #[derive(Debug, Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "kebab-case")]
    pub enum AssetKindInner {
        Font,
        IconFont {
            glyphs: Vec<super::IconGlyphMapping>,
        },
        Image,
    }

    /// Mirrors `super::CardSettings`. Fields that are themselves internally
    /// tagged enums are typed as the outer, validating public type so that
    /// deserializing a card recursively re-validates every nested tagged object.
    /// Typing a nested field as a raw inner type here would silently defeat the
    /// unknown-field check for that nested object.
    #[derive(Debug, Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "kebab-case")]
    pub enum CardSettingsInner {
        Clock {
            id: String,
            title: String,
            show_seconds: bool,
            template: super::DisplayTemplate,
            tap_action: super::WidgetTapAction,
            refresh: super::RefreshPolicy,
            alert: super::CardAlert,
            dwell_seconds: Option<u16>,
        },
        Pomodoro {
            id: String,
            label: String,
            duration_seconds: u32,
            template: super::DisplayTemplate,
            tap_action: super::WidgetTapAction,
            refresh: super::RefreshPolicy,
            alert: super::CardAlert,
            dwell_seconds: Option<u16>,
        },
        Picture {
            id: String,
            title: String,
            source_id: String,
            tap_action: super::WidgetTapAction,
            refresh: super::RefreshPolicy,
            alert: super::CardAlert,
            dwell_seconds: Option<u16>,
        },
    }

    /// Type-aware validation of allowed fields for each enum type.
    #[allow(private_bounds)]
    trait ValidatingDeserialize: for<'de> serde::Deserialize<'de> {
        fn allowed_fields(kind: &str) -> Option<&'static [&'static str]>;
    }

    impl ValidatingDeserialize for AlertHoldInner {
        fn allowed_fields(kind: &str) -> Option<&'static [&'static str]> {
            match kind {
                "until-dismissed" => Some(&["kind"]),
                "seconds" => Some(&["kind", "value"]),
                _ => None,
            }
        }
    }

    impl ValidatingDeserialize for CardAlertInner {
        fn allowed_fields(kind: &str) -> Option<&'static [&'static str]> {
            match kind {
                "none" => Some(&["kind"]),
                "on-timer-finish" => Some(&["kind", "hold"]),
                _ => None,
            }
        }
    }

    impl ValidatingDeserialize for CarouselAdvanceInner {
        fn allowed_fields(kind: &str) -> Option<&'static [&'static str]> {
            match kind {
                "manual" => Some(&["kind"]),
                "timed" => Some(&["kind", "default_dwell_seconds"]),
                _ => None,
            }
        }
    }

    impl ValidatingDeserialize for DisplayTemplateInner {
        fn allowed_fields(kind: &str) -> Option<&'static [&'static str]> {
            match kind {
                "digital-clock" | "analog-clock" | "progress-ring" => Some(&["kind"]),
                _ => None,
            }
        }
    }

    impl ValidatingDeserialize for WidgetTapActionInner {
        fn allowed_fields(kind: &str) -> Option<&'static [&'static str]> {
            match kind {
                "none" | "start-pause" | "reset" | "dismiss" => Some(&["kind"]),
                "open-url" => Some(&["kind", "url"]),
                "open-application" => Some(&["kind", "application_id"]),
                _ => None,
            }
        }
    }

    impl ValidatingDeserialize for RefreshPolicyInner {
        fn allowed_fields(kind: &str) -> Option<&'static [&'static str]> {
            match kind {
                "device-local" | "manual" => Some(&["kind"]),
                "interval" => Some(&["kind", "minutes"]),
                _ => None,
            }
        }
    }

    impl ValidatingDeserialize for AssetKindInner {
        fn allowed_fields(kind: &str) -> Option<&'static [&'static str]> {
            match kind {
                "font" | "image" => Some(&["kind"]),
                "icon-font" => Some(&["kind", "glyphs"]),
                _ => None,
            }
        }
    }

    impl ValidatingDeserialize for CardSettingsInner {
        fn allowed_fields(kind: &str) -> Option<&'static [&'static str]> {
            match kind {
                "clock" => Some(&[
                    "kind",
                    "id",
                    "title",
                    "show_seconds",
                    "template",
                    "tap_action",
                    "refresh",
                    "alert",
                    "dwell_seconds",
                ]),
                "pomodoro" => Some(&[
                    "kind",
                    "id",
                    "label",
                    "duration_seconds",
                    "template",
                    "tap_action",
                    "refresh",
                    "alert",
                    "dwell_seconds",
                ]),
                "picture" => Some(&[
                    "kind",
                    "id",
                    "title",
                    "source_id",
                    "tap_action",
                    "refresh",
                    "alert",
                    "dwell_seconds",
                ]),
                _ => None,
            }
        }
    }

    #[allow(private_bounds)]
    pub fn validate_and_deserialize<T: ValidatingDeserialize>(value: &Value) -> Result<T, String> {
        // Validate unknown fields using type-aware field list
        if let Value::Object(map) = value {
            let kind_str = match map.get("kind") {
                Some(Value::String(s)) => s.as_str(),
                _ => return Err("missing or invalid 'kind' field".to_string()),
            };

            let allowed = T::allowed_fields(kind_str)
                .ok_or_else(|| format!("unknown variant: {kind_str}"))?;

            for key in map.keys() {
                if !allowed.contains(&key.as_str()) {
                    return Err(format!("unknown field: {key}"));
                }
            }
        }

        // Use the inner deserialize
        serde_json::from_value(value.clone()).map_err(|e| e.to_string())
    }
}

pub const CURRENT_SCHEMA_VERSION: u32 = 10;
pub const MAX_WIDGET_TITLE_LEN: usize = 64;
pub const MAX_TIMEZONE_LEN: usize = 64;
pub const MAX_ACTION_URL_LEN: usize = 2_048;
pub const MAX_ASSETS: usize = 16;
pub const MAX_ASSET_SOURCE_LEN: usize = 2_048;
pub const MAX_ASSET_BYTES: u32 = 262_144;
pub const MAX_TOTAL_ASSET_BYTES: u32 = 1_048_576;
pub const MAX_ICON_GLYPHS: usize = 256;
pub const MAX_ICON_GLYPH_NAME_LEN: usize = 64;
pub const MAX_HOST_ACTION_TARGET_LEN: usize = 2_048;
pub(crate) const MIN_POMODORO_SECONDS: u32 = 1;
pub(crate) const MAX_POMODORO_SECONDS: u32 = 86_400;
pub const MIN_CARD_REFRESH_MINUTES: u16 = 1;
pub const MAX_CARD_REFRESH_MINUTES: u16 = 1_440;
pub const MAX_CONFIG_CARDS: usize = 8;
/// How many named image sources one configuration may declare.
///
/// Eight frames is about 2.6 MB of the 6 MB `assets` partition and eight of the
/// device's durable digest budget. Raising it needs the flash budget re-checked,
/// not just this number.
pub const MAX_IMAGE_SOURCES: usize = 8;
const MAX_IMAGE_SOURCE_NAME_LEN: usize = 48;
// Every compiled card can lower to one wire widget, so the card cap must never exceed
// what the protocol's `ApplyConfig` encoder accepts.
const _: () = assert!(MAX_CONFIG_CARDS <= protocol::MAX_CONFIG_CARDS);
pub const MIN_DWELL_SECONDS: u16 = 5;
pub const MAX_DWELL_SECONDS: u16 = 3_600;
pub const MIN_ALERT_HOLD_SECONDS: u16 = 5;
pub const MAX_ALERT_HOLD_SECONDS: u16 = 600;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub schema_version: u32,
    pub preferences: AppPreferences,
    pub cards: Vec<CardSettings>,
    /// Named image sources. `#[serde(default)]` is load-bearing: the migration
    /// path parses a v4/v5/v6 document straight into this struct, and those
    /// documents have no such key. Without the default every saved config on
    /// earth fails to parse and the user is told their settings are invalid.
    #[serde(default)]
    pub image_sources: Vec<ImageSource>,
    pub assets: Vec<AssetSettings>,
    /// How the loop advances. `cards` IS the loop, in order, so this is one
    /// setting for the whole document rather than a property of a named
    /// grouping. The v10 schema has one ordered card loop.
    pub advance: CarouselAdvance,
    pub updater: UpdaterSettings,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            preferences: AppPreferences::default(),
            cards: vec![CardSettings::Clock {
                id: "clock".into(),
                title: "Desk".into(),
                show_seconds: true,
                template: DisplayTemplate::DigitalClock,
                tap_action: WidgetTapAction::None,
                refresh: RefreshPolicy::DeviceLocal,
                alert: CardAlert::None,
                dwell_seconds: None,
            }],
            image_sources: Vec::new(),
            assets: Vec::new(),
            advance: CarouselAdvance::Manual,
            updater: UpdaterSettings::default(),
        }
    }
}

impl AppConfig {
    #[allow(clippy::too_many_lines)]
    pub fn validate(&self) -> Result<(), ConfigValidationError> {
        let mut issues = Vec::new();
        if self.schema_version != CURRENT_SCHEMA_VERSION {
            issues.push(ValidationIssue::new(
                "schema_version",
                ValidationCode::UnsupportedVersion,
                format!(
                    "schema version {} is not supported; expected {CURRENT_SCHEMA_VERSION}",
                    self.schema_version
                ),
            ));
        }
        validate_text(
            "preferences.timezone",
            &self.preferences.timezone,
            MAX_TIMEZONE_LEN,
            true,
            &mut issues,
        );
        validate_timezone(&self.preferences.timezone, &mut issues);
        validate_collection_bounds("cards", self.cards.len(), MAX_CONFIG_CARDS, &mut issues);

        let mut card_ids = HashSet::with_capacity(self.cards.len());
        for (index, card) in self.cards.iter().enumerate() {
            let path = format!("cards[{index}]");
            validate_identifier(
                &format!("{path}.id"),
                card.id(),
                MAX_CARD_ID_LEN,
                &mut issues,
            );
            if !card_ids.insert(card.id()) {
                issues.push(ValidationIssue::new(
                    format!("{path}.id"),
                    ValidationCode::DuplicateId,
                    format!("card ID {:?} is duplicated", card.id()),
                ));
            }
            card.validate(&path, &mut issues);
            validate_card_behaviour(&path, card, &mut issues);
        }

        if self.image_sources.len() > MAX_IMAGE_SOURCES {
            issues.push(ValidationIssue::new(
                "image_sources",
                ValidationCode::TooMany,
                format!("at most {MAX_IMAGE_SOURCES} image_sources entries are supported"),
            ));
        }
        let mut seen_source_ids: BTreeSet<&str> = BTreeSet::new();
        for (index, source) in self.image_sources.iter().enumerate() {
            let path = format!("image_sources[{index}]");
            validate_identifier(
                &format!("{path}.id"),
                &source.id,
                MAX_CARD_ID_LEN,
                &mut issues,
            );
            validate_text(
                &format!("{path}.name"),
                &source.name,
                MAX_IMAGE_SOURCE_NAME_LEN,
                true,
                &mut issues,
            );
            if !seen_source_ids.insert(source.id.as_str()) {
                issues.push(ValidationIssue::new(
                    format!("{path}.id"),
                    ValidationCode::DuplicateId,
                    format!("image source {:?} is declared more than once", source.id),
                ));
            }
        }
        for (index, card) in self.cards.iter().enumerate() {
            if let CardSettings::Picture { source_id, .. } = card
                && !seen_source_ids.contains(source_id.as_str())
            {
                issues.push(ValidationIssue::new(
                    format!("cards[{index}].source_id"),
                    ValidationCode::MissingReference,
                    format!("image source {source_id:?} does not exist"),
                ));
            }
        }

        if let CarouselAdvance::Timed {
            default_dwell_seconds,
        } = self.advance
            && !(MIN_DWELL_SECONDS..=MAX_DWELL_SECONDS).contains(&default_dwell_seconds)
        {
            issues.push(ValidationIssue::new(
                "advance.default_dwell_seconds",
                ValidationCode::OutOfRange,
                format!("default dwell must be {MIN_DWELL_SECONDS}..={MAX_DWELL_SECONDS} seconds"),
            ));
        }
        for (index, card) in self.cards.iter().enumerate() {
            if let Some(dwell_seconds) = card.dwell_seconds()
                && !(MIN_DWELL_SECONDS..=MAX_DWELL_SECONDS).contains(&dwell_seconds)
            {
                issues.push(ValidationIssue::new(
                    format!("cards[{index}].dwell_seconds"),
                    ValidationCode::OutOfRange,
                    format!(
                        "card {:?} must dwell for {MIN_DWELL_SECONDS}..={MAX_DWELL_SECONDS} seconds",
                        card.id()
                    ),
                ));
            }
        }
        // Keep this second issue: the server's 422 response exposes the full issue list.
        if self.cards.len() > MAX_CONFIG_CARDS {
            issues.push(ValidationIssue::new(
                "cards",
                ValidationCode::TooMany,
                format!("compiled widget set must contain at most {MAX_CONFIG_CARDS} cards"),
            ));
        }

        if self.assets.len() > MAX_ASSETS {
            issues.push(ValidationIssue::new(
                "assets",
                ValidationCode::TooMany,
                format!("at most {MAX_ASSETS} assets are supported"),
            ));
        }
        let mut assets_by_id = HashMap::with_capacity(self.assets.len());
        let mut total_asset_bytes = 0_u32;
        for (index, asset) in self.assets.iter().enumerate() {
            let path = format!("assets[{index}]");
            asset.validate(&path, &mut issues);
            if assets_by_id
                .insert(asset.id.as_str(), &asset.kind)
                .is_some()
            {
                issues.push(ValidationIssue::new(
                    format!("{path}.id"),
                    ValidationCode::DuplicateId,
                    format!("asset ID {:?} is duplicated", asset.id),
                ));
            }
            total_asset_bytes = total_asset_bytes.saturating_add(asset.maximum_bytes);
        }
        if total_asset_bytes > MAX_TOTAL_ASSET_BYTES {
            issues.push(ValidationIssue::new(
                "assets",
                ValidationCode::TooLarge,
                format!("asset budget must not exceed {MAX_TOTAL_ASSET_BYTES} bytes"),
            ));
        }
        if issues.is_empty() {
            Ok(())
        } else {
            Err(ConfigValidationError { issues })
        }
    }

    pub fn compile(&self, revision: u32) -> Result<CompiledAppConfig, ConfigValidationError> {
        self.validate()?;
        if revision == 0 {
            return Err(ConfigValidationError {
                issues: vec![ValidationIssue::new(
                    "revision",
                    ValidationCode::OutOfRange,
                    "device revision must be greater than zero",
                )],
            });
        }

        let mut compatibility_issues = Vec::new();
        let mut cards = Vec::with_capacity(self.cards.len());
        for (index, card) in self.cards.iter().enumerate() {
            let Some(wire) = card.wire_config() else {
                compatibility_issues.push(ValidationIssue::new(
                    format!("cards[{index}]"),
                    ValidationCode::RequiresCapability,
                    "card kind/action is not implemented by this build",
                ));
                continue;
            };
            cards.push(wire);
        }
        if !compatibility_issues.is_empty() {
            return Err(ConfigValidationError {
                issues: compatibility_issues,
            });
        }
        // Protocol v2 replaced the initial field bag with a timer, which only a
        // pomodoro has. Everything else a face shows arrives as a scene.
        let initial_timers = self
            .cards
            .iter()
            .filter_map(|card| card.initial_timer(revision))
            .collect();

        Ok(CompiledAppConfig {
            layout: ApplyConfig {
                revision,
                rotation: self.preferences.orientation.rotation_degrees(),
                cards,
            },
            initial_timers,
            assets: self.assets.clone(),
            required_capabilities: self.required_device_capabilities(),
        })
    }

    /// Capability bits this configuration needs from the device.
    ///
    /// Protocol v2 retired the bits that described template rendering, so what
    /// is left is asset transfer -- the only thing a configuration can still
    /// ask a device to be able to do. Rotation, tap actions and templates all
    /// went with the concepts behind them.
    pub fn required_device_capabilities(&self) -> u64 {
        if self.assets.is_empty() {
            0
        } else {
            protocol::CAPABILITY_ASSET_TRANSFER
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppPreferences {
    pub timezone: String,
    pub autostart: bool,
    pub paused: bool,
    #[serde(default)]
    pub orientation: DisplayOrientation,
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            timezone: "UTC".into(),
            autostart: false,
            paused: false,
            orientation: DisplayOrientation::default(),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DisplayOrientation {
    #[default]
    Landscape,
    LandscapeFlipped,
}

impl DisplayOrientation {
    pub const fn rotation_degrees(self) -> u16 {
        match self {
            Self::Landscape => 90,
            Self::LandscapeFlipped => 270,
        }
    }
}

/// Computes the UTC offset in minutes for `timezone` at `now`. Shared by the
/// runtime's device time-sync path (`runtime::send_time_sync`) and the web
/// preview renderer so both derive the offset from the same configured timezone
/// through the same formula, rather than each doing its own `chrono_tz` lookup that
/// could drift from the other.
///
/// # Errors
///
/// Returns an error message when `timezone` is not a recognized IANA timezone, or
/// when the computed offset does not fit the protocol's `i16` minutes field (not
/// reachable by any real-world timezone, but `AppConfig::validate` already rejects
/// unrecognized timezones before this is ever called on a saved configuration).
pub fn utc_offset_minutes(timezone: &str, now: DateTime<Utc>) -> Result<i16, String> {
    let parsed: chrono_tz::Tz = timezone
        .parse()
        .map_err(|_| format!("{timezone:?} is not a recognized IANA timezone"))?;
    let offset_seconds = now.with_timezone(&parsed).offset().fix().local_minus_utc();
    i16::try_from(offset_seconds / 60)
        .map_err(|_| "timezone offset exceeds protocol bounds".to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum DisplayTemplate {
    DigitalClock,
    AnalogClock,
    ProgressRing,
}

impl<'de> Deserialize<'de> for DisplayTemplate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let inner: strict_tagged_enum::DisplayTemplateInner =
            strict_tagged_enum::validate_and_deserialize(&value)
                .map_err(serde::de::Error::custom)?;
        Ok(match inner {
            strict_tagged_enum::DisplayTemplateInner::DigitalClock => Self::DigitalClock,
            strict_tagged_enum::DisplayTemplateInner::AnalogClock => Self::AnalogClock,
            strict_tagged_enum::DisplayTemplateInner::ProgressRing => Self::ProgressRing,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum WidgetTapAction {
    None,
    StartPause,
    Reset,
    Dismiss,
    OpenUrl { url: String },
    OpenApplication { application_id: String },
}

impl<'de> Deserialize<'de> for WidgetTapAction {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let inner: strict_tagged_enum::WidgetTapActionInner =
            strict_tagged_enum::validate_and_deserialize(&value)
                .map_err(serde::de::Error::custom)?;
        Ok(match inner {
            strict_tagged_enum::WidgetTapActionInner::None => Self::None,
            strict_tagged_enum::WidgetTapActionInner::StartPause => Self::StartPause,
            strict_tagged_enum::WidgetTapActionInner::Reset => Self::Reset,
            strict_tagged_enum::WidgetTapActionInner::Dismiss => Self::Dismiss,
            strict_tagged_enum::WidgetTapActionInner::OpenUrl { url } => Self::OpenUrl { url },
            strict_tagged_enum::WidgetTapActionInner::OpenApplication { application_id } => {
                Self::OpenApplication { application_id }
            }
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RefreshPolicy {
    DeviceLocal,
    Manual,
    Interval { minutes: u16 },
}

impl<'de> Deserialize<'de> for RefreshPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let inner: strict_tagged_enum::RefreshPolicyInner =
            strict_tagged_enum::validate_and_deserialize(&value)
                .map_err(serde::de::Error::custom)?;
        Ok(match inner {
            strict_tagged_enum::RefreshPolicyInner::DeviceLocal => Self::DeviceLocal,
            strict_tagged_enum::RefreshPolicyInner::Manual => Self::Manual,
            strict_tagged_enum::RefreshPolicyInner::Interval { minutes } => {
                Self::Interval { minutes }
            }
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AlertHold {
    UntilDismissed,
    Seconds { value: u16 },
}

impl<'de> Deserialize<'de> for AlertHold {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let inner: strict_tagged_enum::AlertHoldInner =
            strict_tagged_enum::validate_and_deserialize(&value)
                .map_err(serde::de::Error::custom)?;
        Ok(match inner {
            strict_tagged_enum::AlertHoldInner::UntilDismissed => AlertHold::UntilDismissed,
            strict_tagged_enum::AlertHoldInner::Seconds { value } => AlertHold::Seconds { value },
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CardAlert {
    None,
    OnTimerFinish { hold: AlertHold },
}

impl<'de> Deserialize<'de> for CardAlert {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let inner: strict_tagged_enum::CardAlertInner =
            strict_tagged_enum::validate_and_deserialize(&value)
                .map_err(serde::de::Error::custom)?;
        Ok(match inner {
            strict_tagged_enum::CardAlertInner::None => CardAlert::None,
            strict_tagged_enum::CardAlertInner::OnTimerFinish { hold } => {
                CardAlert::OnTimerFinish { hold }
            }
        })
    }
}

impl CardAlert {
    pub const fn hold(self) -> Option<AlertHold> {
        match self {
            Self::None => None,
            Self::OnTimerFinish { hold } => Some(hold),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CarouselAdvance {
    Manual,
    Timed { default_dwell_seconds: u16 },
}

impl<'de> Deserialize<'de> for CarouselAdvance {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let inner: strict_tagged_enum::CarouselAdvanceInner =
            strict_tagged_enum::validate_and_deserialize(&value)
                .map_err(serde::de::Error::custom)?;
        Ok(match inner {
            strict_tagged_enum::CarouselAdvanceInner::Manual => CarouselAdvance::Manual,
            strict_tagged_enum::CarouselAdvanceInner::Timed {
                default_dwell_seconds,
            } => CarouselAdvance::Timed {
                default_dwell_seconds,
            },
        })
    }
}

impl CarouselAdvance {
    pub const fn default_dwell_seconds(self) -> Option<u16> {
        match self {
            Self::Manual => None,
            Self::Timed {
                default_dwell_seconds,
            } => Some(default_dwell_seconds),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CardSettings {
    Clock {
        id: String,
        title: String,
        show_seconds: bool,
        template: DisplayTemplate,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
        alert: CardAlert,
        dwell_seconds: Option<u16>,
    },
    Pomodoro {
        id: String,
        label: String,
        duration_seconds: u32,
        template: DisplayTemplate,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
        alert: CardAlert,
        dwell_seconds: Option<u16>,
    },
    /// A card whose face is a PNG an external producer pushed to a named image
    /// source. It carries no `template` field: the picture *is*
    /// the layout, so there is nothing to select among. `source_id` names the
    /// source; the server resolves it to a resident asset digest and pushes a
    /// single full-canvas image scene.
    Picture {
        id: String,
        title: String,
        source_id: String,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
        alert: CardAlert,
        dwell_seconds: Option<u16>,
    },
}

impl<'de> Deserialize<'de> for CardSettings {
    #[allow(clippy::too_many_lines)]
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let inner: strict_tagged_enum::CardSettingsInner =
            strict_tagged_enum::validate_and_deserialize(&value)
                .map_err(serde::de::Error::custom)?;
        Ok(match inner {
            strict_tagged_enum::CardSettingsInner::Clock {
                id,
                title,
                show_seconds,
                template,
                tap_action,
                refresh,
                alert,
                dwell_seconds,
            } => CardSettings::Clock {
                id,
                title,
                show_seconds,
                template,
                tap_action,
                refresh,
                alert,
                dwell_seconds,
            },
            strict_tagged_enum::CardSettingsInner::Pomodoro {
                id,
                label,
                duration_seconds,
                template,
                tap_action,
                refresh,
                alert,
                dwell_seconds,
            } => CardSettings::Pomodoro {
                id,
                label,
                duration_seconds,
                template,
                tap_action,
                refresh,
                alert,
                dwell_seconds,
            },
            strict_tagged_enum::CardSettingsInner::Picture {
                id,
                title,
                source_id,
                tap_action,
                refresh,
                alert,
                dwell_seconds,
            } => CardSettings::Picture {
                id,
                title,
                source_id,
                tap_action,
                refresh,
                alert,
                dwell_seconds,
            },
        })
    }
}

impl CardSettings {
    pub fn id(&self) -> &str {
        match self {
            Self::Clock { id, .. } | Self::Pomodoro { id, .. } | Self::Picture { id, .. } => id,
        }
    }

    pub const fn alert(&self) -> CardAlert {
        match self {
            Self::Clock { alert, .. }
            | Self::Pomodoro { alert, .. }
            | Self::Picture { alert, .. } => *alert,
        }
    }

    /// `None` for picture cards: the picture is already the complete face, so
    /// there is no `DisplayTemplate` to select.
    ///
    /// Since protocol v2 this is purely host state -- it selects which scene
    /// builder draws the card -- and the wire carries no template at all.
    pub fn template(&self) -> Option<&DisplayTemplate> {
        match self {
            Self::Clock { template, .. } | Self::Pomodoro { template, .. } => Some(template),
            Self::Picture { .. } => None,
        }
    }

    /// How long this card stays on the panel before the loop advances, or
    /// `None` to use `AppConfig::advance`'s default.
    pub const fn dwell_seconds(&self) -> Option<u16> {
        match self {
            Self::Clock { dwell_seconds, .. }
            | Self::Pomodoro { dwell_seconds, .. }
            | Self::Picture { dwell_seconds, .. } => *dwell_seconds,
        }
    }

    pub fn tap_action(&self) -> &WidgetTapAction {
        match self {
            Self::Clock { tap_action, .. }
            | Self::Pomodoro { tap_action, .. }
            | Self::Picture { tap_action, .. } => tap_action,
        }
    }

    #[allow(clippy::too_many_lines)]
    fn validate(&self, path: &str, issues: &mut Vec<ValidationIssue>) {
        match self {
            Self::Clock {
                title,
                template,
                tap_action,
                refresh,
                ..
            } => {
                validate_text(
                    &format!("{path}.title"),
                    title,
                    MAX_WIDGET_TITLE_LEN,
                    false,
                    issues,
                );
                validate_composition(
                    path,
                    CardBehavior::Clock,
                    Some(template),
                    tap_action,
                    *refresh,
                    issues,
                );
            }
            Self::Pomodoro {
                label,
                duration_seconds,
                template,
                tap_action,
                refresh,
                ..
            } => {
                validate_text(
                    &format!("{path}.label"),
                    label,
                    MAX_WIDGET_TITLE_LEN,
                    false,
                    issues,
                );
                if !(MIN_POMODORO_SECONDS..=MAX_POMODORO_SECONDS).contains(duration_seconds) {
                    issues.push(ValidationIssue::new(
                        format!("{path}.duration_seconds"),
                        ValidationCode::OutOfRange,
                        format!(
                            "duration must be {MIN_POMODORO_SECONDS}..={MAX_POMODORO_SECONDS} seconds"
                        ),
                    ));
                }
                validate_composition(
                    path,
                    CardBehavior::Pomodoro,
                    Some(template),
                    tap_action,
                    *refresh,
                    issues,
                );
            }
            Self::Picture {
                title,
                source_id,
                tap_action,
                refresh,
                ..
            } => {
                validate_text(
                    &format!("{path}.title"),
                    title,
                    MAX_WIDGET_TITLE_LEN,
                    false,
                    issues,
                );
                validate_identifier(
                    &format!("{path}.source_id"),
                    source_id,
                    MAX_CARD_ID_LEN,
                    issues,
                );
                validate_composition(
                    path,
                    CardBehavior::Picture,
                    None,
                    tap_action,
                    *refresh,
                    issues,
                );
            }
        }
        self.tap_action()
            .validate(&format!("{path}.tap_action"), issues);
    }

    /// This card as protocol v2's `CardConfig`.
    ///
    /// v2 carries only what the device decides for itself: card identity and
    /// tap behavior. Faces arrive as host-built scenes, and alert policy stays
    /// on the host.
    fn wire_config(&self) -> Option<CardConfig> {
        let tap_action = match self.tap_action() {
            WidgetTapAction::None => TapAction::None,
            WidgetTapAction::StartPause => TapAction::StartPause,
            WidgetTapAction::Reset => TapAction::Reset,
            WidgetTapAction::Dismiss
            | WidgetTapAction::OpenUrl { .. }
            | WidgetTapAction::OpenApplication { .. } => return None,
        };
        Some(CardConfig {
            card_id: self.id().into(),
            tap_action,
        })
    }

    /// The host-side data a card's face starts from, before any provider or
    /// timer has run.
    ///
    /// Protocol v1 sent this bag to the device as `PushData` and seeded the
    /// host's own copy from the same value. Protocol v2's device has no field
    /// model, so this is now purely host state: the input the scene builders
    /// read, and what the settings UI shows as the card's data.
    pub(crate) fn initial_fields(&self) -> Vec<CardField> {
        match self {
            Self::Clock {
                title,
                show_seconds,
                ..
            } => vec![
                text_field("title", title),
                bool_field("show_seconds", *show_seconds),
                bool_field("stale", false),
                text_field("error", ""),
            ],
            Self::Pomodoro {
                label,
                duration_seconds,
                ..
            } => vec![
                text_field("label", label),
                int_field("duration_seconds", *duration_seconds),
                int_field("remaining_seconds", *duration_seconds),
                bool_field("running", false),
                bool_field("stale", false),
                text_field("error", ""),
            ],
            // Picture uses this placeholder shape until its first frame arrives.
            Self::Picture { title, .. } => vec![
                text_field("title", title),
                bool_field("stale", true),
                text_field("error", "Waiting for picture"),
            ],
        }
    }

    /// The timer this card's `timer.*` scene bindings start from, or `None`
    /// for a card that has no timer. Replaces protocol v1's initial field bag.
    fn initial_timer(&self, revision: u32) -> Option<PushTimer> {
        match self {
            Self::Pomodoro {
                id,
                duration_seconds,
                ..
            } => Some(PushTimer {
                card_id: id.clone(),
                revision,
                total_ms: duration_seconds.saturating_mul(1_000),
                remaining_ms: duration_seconds.saturating_mul(1_000),
                running: false,
            }),
            Self::Clock { .. } | Self::Picture { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetSettings {
    pub id: String,
    pub source: AssetSource,
    pub kind: AssetKind,
    pub maximum_bytes: u32,
}

/// A named destination an external producer pushes pictures to.
///
/// Authoring identity and nothing else. The credential that authorizes a push
/// is **not** here and never enters a configuration: the server keeps a SHA-256
/// digest keyed by `id`, exactly as it does for device identities, and the
/// plaintext token is returned once at mint and never persisted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageSource {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "kebab-case",
    deny_unknown_fields
)]
pub enum AssetSource {
    File(String),
}

/// The device rasterizes glyphs at any size from a TTF/OTF at runtime (`tiny_ttf`),
/// so unlike v4's `Icon { width, height }` / `Font { pixel_size, glyph_ranges }`, no
/// v5 variant pins a size or a pre-baked glyph set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AssetKind {
    /// A TTF/OTF text font. Size is chosen per text node at render time.
    Font,
    /// A TTF/OTF icon font plus a name→codepoint map, so a scene can write
    /// `icon: "cloud-rain"` instead of a raw codepoint.
    IconFont { glyphs: Vec<IconGlyphMapping> },
    /// An LVGL binary image, converted host-side so the device needs no PNG
    /// decoder.
    Image,
}

impl<'de> Deserialize<'de> for AssetKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let inner: strict_tagged_enum::AssetKindInner =
            strict_tagged_enum::validate_and_deserialize(&value)
                .map_err(serde::de::Error::custom)?;
        Ok(match inner {
            strict_tagged_enum::AssetKindInner::Font => Self::Font,
            strict_tagged_enum::AssetKindInner::IconFont { glyphs } => Self::IconFont { glyphs },
            strict_tagged_enum::AssetKindInner::Image => Self::Image,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IconGlyphMapping {
    pub name: String,
    pub codepoint: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdaterSettings {
    pub channel: UpdateChannel,
    pub checks: UpdateCheckPolicy,
}

impl Default for UpdaterSettings {
    fn default() -> Self {
        Self {
            channel: UpdateChannel::Stable,
            checks: UpdateCheckPolicy::Notify,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UpdateChannel {
    Stable,
    Beta,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UpdateCheckPolicy {
    Disabled,
    Notify,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledAppConfig {
    pub layout: ApplyConfig,
    pub initial_timers: Vec<PushTimer>,
    pub assets: Vec<AssetSettings>,
    pub required_capabilities: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ValidationCode {
    UnsupportedVersion,
    Empty,
    TooLong,
    TooMany,
    DuplicateId,
    MissingReference,
    OutOfRange,
    InvalidTimezone,
    InvalidSource,
    InvalidComposition,
    TooLarge,
    RequiresCapability,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub path: String,
    pub code: ValidationCode,
    pub message: String,
}

impl ValidationIssue {
    fn new(path: impl Into<String>, code: ValidationCode, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            code,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigValidationError {
    pub issues: Vec<ValidationIssue>,
}

impl fmt::Display for ConfigValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "configuration has {} validation issue(s)",
            self.issues.len()
        )
    }
}

impl std::error::Error for ConfigValidationError {}

fn validate_collection_bounds(
    path: &str,
    count: usize,
    maximum: usize,
    issues: &mut Vec<ValidationIssue>,
) {
    if count == 0 {
        issues.push(ValidationIssue::new(
            path,
            ValidationCode::Empty,
            format!("at least one {path} entry is required"),
        ));
    } else if count > maximum {
        issues.push(ValidationIssue::new(
            path,
            ValidationCode::TooMany,
            format!("at most {maximum} {path} entries are supported"),
        ));
    }
}

fn validate_identifier(path: &str, value: &str, maximum: usize, issues: &mut Vec<ValidationIssue>) {
    validate_text(path, value, maximum, true, issues);
}

fn validate_text(
    path: &str,
    value: &str,
    maximum: usize,
    required: bool,
    issues: &mut Vec<ValidationIssue>,
) {
    if required && value.trim().is_empty() {
        issues.push(ValidationIssue::new(
            path,
            ValidationCode::Empty,
            "value must not be empty",
        ));
    }
    if value.len() > maximum {
        issues.push(ValidationIssue::new(
            path,
            ValidationCode::TooLong,
            format!("value must be at most {maximum} UTF-8 bytes"),
        ));
    }
}

fn validate_timezone(timezone: &str, issues: &mut Vec<ValidationIssue>) {
    if timezone.parse::<chrono_tz::Tz>().is_err() {
        issues.push(ValidationIssue::new(
            "preferences.timezone",
            ValidationCode::InvalidTimezone,
            format!("{timezone:?} is not a recognized IANA timezone"),
        ));
    }
}

#[derive(Clone, Copy)]
enum CardBehavior {
    Clock,
    Pomodoro,
    Picture,
}

fn validate_composition(
    path: &str,
    behavior: CardBehavior,
    template: Option<&DisplayTemplate>,
    tap_action: &WidgetTapAction,
    refresh: RefreshPolicy,
    issues: &mut Vec<ValidationIssue>,
) {
    // `template` is `None` only for picture cards: they render from a host-built
    // scene, not a `DisplayTemplate`, so there
    // is no template/card field-compatibility pairing to check and no
    // template-gated tap-action rule to enforce here. The card-gated tap-action
    // and refresh-policy checks below still apply to every kind.
    if let Some(template) = template {
        // A pairing is allowed only when the card produces the fields the
        // template's host scene builder reads. In
        // `runtime/scene.rs::build_template_card_scene`, the clock builders read
        // `show_seconds`, and the progress-ring builder reads `label` and
        // `duration_seconds`.
        let template_supported = match behavior {
            // Clock scenes can render either clock face.
            CardBehavior::Clock => matches!(
                template,
                DisplayTemplate::DigitalClock | DisplayTemplate::AnalogClock
            ),
            // Pomodoro bindings are defined by the progress-ring scene.
            CardBehavior::Pomodoro => matches!(template, DisplayTemplate::ProgressRing),
            // Picture cards never pass `Some(template)` — see the comment
            // above. This arm is unreachable by construction today, but the function
            // validates untrusted config content, so it returns a safe `false` (an
            // `InvalidComposition` issue) rather than panicking if that ever changes.
            CardBehavior::Picture => false,
        };
        if !template_supported {
            issues.push(ValidationIssue::new(
                format!("{path}.template"),
                ValidationCode::InvalidComposition,
                "display template is incompatible with this card kind",
            ));
        }

        // Timer actions require the timer bindings supplied by the
        // progress-ring scene. Reject the composition before applying the
        // all-or-nothing device layout.
        if matches!(
            tap_action,
            WidgetTapAction::StartPause | WidgetTapAction::Reset
        ) && !matches!(template, DisplayTemplate::ProgressRing)
        {
            issues.push(ValidationIssue::new(
                format!("{path}.tap_action"),
                ValidationCode::InvalidComposition,
                "start/pause and reset actions require the progress-ring template",
            ));
        }
    }

    if matches!(
        tap_action,
        WidgetTapAction::StartPause | WidgetTapAction::Reset
    ) && !matches!(behavior, CardBehavior::Pomodoro)
    {
        issues.push(ValidationIssue::new(
            format!("{path}.tap_action"),
            ValidationCode::InvalidComposition,
            "start/pause and reset actions require a pomodoro card",
        ));
    }

    let refresh_supported = match behavior {
        CardBehavior::Clock | CardBehavior::Pomodoro => {
            matches!(refresh, RefreshPolicy::DeviceLocal)
        }
        CardBehavior::Picture => matches!(
            refresh,
            RefreshPolicy::Manual | RefreshPolicy::Interval { .. }
        ),
    };
    if !refresh_supported {
        issues.push(ValidationIssue::new(
            format!("{path}.refresh"),
            ValidationCode::InvalidComposition,
            "refresh policy is incompatible with this card kind",
        ));
    }
    if let RefreshPolicy::Interval { minutes } = refresh
        && !(MIN_CARD_REFRESH_MINUTES..=MAX_CARD_REFRESH_MINUTES).contains(&minutes)
    {
        issues.push(ValidationIssue::new(
            format!("{path}.refresh.minutes"),
            ValidationCode::OutOfRange,
            format!(
                "refresh interval must be {MIN_CARD_REFRESH_MINUTES}..={MAX_CARD_REFRESH_MINUTES} minutes"
            ),
        ));
    }
}

impl WidgetTapAction {
    fn validate(&self, path: &str, issues: &mut Vec<ValidationIssue>) {
        match self {
            Self::OpenUrl { url } => validate_http_url(&format!("{path}.url"), url, issues),
            Self::OpenApplication { application_id } => {
                validate_text(
                    &format!("{path}.application_id"),
                    application_id,
                    MAX_HOST_ACTION_TARGET_LEN,
                    true,
                    issues,
                );
                if application_id.contains(['/', '\\'])
                    || application_id.chars().any(char::is_control)
                {
                    issues.push(ValidationIssue::new(
                        format!("{path}.application_id"),
                        ValidationCode::InvalidSource,
                        "application target must be an OS application identifier, not a path",
                    ));
                }
            }
            Self::None | Self::StartPause | Self::Reset | Self::Dismiss => {}
        }
    }
}

fn validate_http_url(path: &str, url: &str, issues: &mut Vec<ValidationIssue>) {
    validate_text(path, url, MAX_ACTION_URL_LEN, true, issues);
    if !url.is_empty() && !is_valid_http_url(url) {
        issues.push(ValidationIssue::new(
            path,
            ValidationCode::InvalidSource,
            "URL must be HTTP(S), include a host, and contain no credentials",
        ));
    }
}

fn is_valid_http_url(raw: &str) -> bool {
    let Ok(parsed) = url::Url::parse(raw) else {
        return false;
    };
    matches!(parsed.scheme(), "http" | "https")
        && parsed.host_str().is_some()
        && parsed.username().is_empty()
        && parsed.password().is_none()
}

fn validate_range(
    path: &str,
    value: u32,
    minimum: u32,
    maximum: u32,
    issues: &mut Vec<ValidationIssue>,
) {
    if !(minimum..=maximum).contains(&value) {
        issues.push(ValidationIssue::new(
            path,
            ValidationCode::OutOfRange,
            format!("value must be {minimum}..={maximum}"),
        ));
    }
}

fn validate_card_behaviour(path: &str, card: &CardSettings, issues: &mut Vec<ValidationIssue>) {
    let alert = card.alert();

    match alert {
        CardAlert::None => {}
        CardAlert::OnTimerFinish { hold } => {
            if !matches!(card, CardSettings::Pomodoro { .. }) {
                issues.push(ValidationIssue::new(
                    format!("{path}.alert"),
                    ValidationCode::OutOfRange,
                    "on-timer-finish alerts are only valid on pomodoro cards",
                ));
            }
            validate_alert_hold(path, hold, issues);
        }
    }
}

fn validate_alert_hold(path: &str, hold: AlertHold, issues: &mut Vec<ValidationIssue>) {
    if let AlertHold::Seconds { value } = hold {
        validate_range(
            &format!("{path}.alert.hold.value"),
            u32::from(value),
            u32::from(MIN_ALERT_HOLD_SECONDS),
            u32::from(MAX_ALERT_HOLD_SECONDS),
            issues,
        );
    }
}

impl AssetSettings {
    fn validate(&self, path: &str, issues: &mut Vec<ValidationIssue>) {
        validate_identifier(&format!("{path}.id"), &self.id, MAX_CARD_ID_LEN, issues);
        let AssetSource::File(source) = &self.source;
        validate_text(
            &format!("{path}.source"),
            source,
            MAX_ASSET_SOURCE_LEN,
            true,
            issues,
        );
        if !(1..=MAX_ASSET_BYTES).contains(&self.maximum_bytes) {
            issues.push(ValidationIssue::new(
                format!("{path}.maximum_bytes"),
                ValidationCode::OutOfRange,
                format!("asset size budget must be 1..={MAX_ASSET_BYTES} bytes"),
            ));
        }
        match &self.kind {
            AssetKind::Font | AssetKind::Image => {}
            AssetKind::IconFont { glyphs } => {
                if glyphs.is_empty() || glyphs.len() > MAX_ICON_GLYPHS {
                    issues.push(ValidationIssue::new(
                        format!("{path}.kind.glyphs"),
                        ValidationCode::OutOfRange,
                        format!("icon-font must contain 1..={MAX_ICON_GLYPHS} glyph mappings"),
                    ));
                }
                let mut seen_names = HashSet::with_capacity(glyphs.len());
                for (index, glyph) in glyphs.iter().enumerate() {
                    validate_identifier(
                        &format!("{path}.kind.glyphs[{index}].name"),
                        &glyph.name,
                        MAX_ICON_GLYPH_NAME_LEN,
                        issues,
                    );
                    if !seen_names.insert(glyph.name.as_str()) {
                        issues.push(ValidationIssue::new(
                            format!("{path}.kind.glyphs[{index}].name"),
                            ValidationCode::DuplicateId,
                            format!("icon name {:?} is duplicated", glyph.name),
                        ));
                    }
                    let is_surrogate = (0xd800..=0xdfff).contains(&glyph.codepoint);
                    if glyph.codepoint > 0x0010_ffff || is_surrogate {
                        issues.push(ValidationIssue::new(
                            format!("{path}.kind.glyphs[{index}].codepoint"),
                            ValidationCode::OutOfRange,
                            "codepoint must be a valid Unicode scalar value",
                        ));
                    }
                }
            }
        }
    }
}

fn text_field(key: &str, value: &str) -> CardField {
    CardField {
        key: key.into(),
        value: CardFieldValue::Text {
            value: value.into(),
        },
    }
}

fn int_field(key: &str, value: u32) -> CardField {
    CardField {
        key: key.into(),
        value: CardFieldValue::Integer {
            value: i64::from(value),
        },
    }
}

fn bool_field(key: &str, value: bool) -> CardField {
    CardField {
        key: key.into(),
        value: CardFieldValue::Boolean { value },
    }
}
