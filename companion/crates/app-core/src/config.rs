use std::collections::{HashMap, HashSet};
use std::fmt;

use chrono::{DateTime, Offset, Utc};
pub use protocol::MAX_WIDGET_ID_LEN;
use protocol::{
    ApplyConfig, Field, FieldValue, InterruptPolicy, PushData, ScreenConfig, TapAction,
    TemplateKind, WidgetConfig,
};
use serde::{Deserialize, Deserializer, Serialize};

// Helper module for strict deserialization of internally tagged enums
mod strict_tagged_enum {
    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    #[derive(Debug, Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "kebab-case")]
    pub enum CardPresenceInner {
        InRotation { dwell_seconds: Option<u16> },
        AlertOnly,
        Off,
    }

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
        OnTimerFinish {
            hold: super::AlertHold,
        },
        BeforeEvent {
            lead_minutes: u16,
            hold: super::AlertHold,
        },
    }

    #[derive(Debug, Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "kebab-case")]
    pub enum CarouselAdvanceInner {
        Manual,
        Timed { default_dwell_seconds: u16 },
    }

    /// Serde's `deny_unknown_fields` no-op on internally tagged enums affects unit
    /// variants just as much as struct variants (`DigitalClock`, not just
    /// `IconBadgeText`), so `DisplayTemplate` needs the same treatment as the
    /// card-behaviour types above.
    #[derive(Debug, Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "kebab-case")]
    pub enum DisplayTemplateInner {
        DigitalClock,
        AnalogClock,
        ProgressRing,
        RowList,
        BigNumberLabel,
        IconBadgeText { icon_asset_id: Option<String> },
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

    /// Mirrors `super::CardSettings`. Fields that are themselves internally
    /// tagged enums are typed as the outer, validating public type (e.g.
    /// `super::CardPresence`, not a raw inner enum) so that deserializing a
    /// card recursively re-validates every nested tagged object. Typing a
    /// nested field as a raw inner type here would silently defeat the
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
            presence: super::CardPresence,
            alert: super::CardAlert,
        },
        Pomodoro {
            id: String,
            label: String,
            duration_seconds: u32,
            template: super::DisplayTemplate,
            tap_action: super::WidgetTapAction,
            refresh: super::RefreshPolicy,
            presence: super::CardPresence,
            alert: super::CardAlert,
        },
        Calendar {
            id: String,
            title: String,
            source: super::CalendarSource,
            template: super::DisplayTemplate,
            tap_action: super::WidgetTapAction,
            refresh: super::RefreshPolicy,
            presence: super::CardPresence,
            alert: super::CardAlert,
        },
        Weather {
            id: String,
            title: String,
            location: String,
            units: super::WeatherUnits,
            template: super::DisplayTemplate,
            tap_action: super::WidgetTapAction,
            refresh: super::RefreshPolicy,
            presence: super::CardPresence,
            alert: super::CardAlert,
        },
        JsonFeed {
            id: String,
            title: String,
            url: String,
            mappings: Vec<super::JsonFieldMapping>,
            template: super::DisplayTemplate,
            tap_action: super::WidgetTapAction,
            refresh: super::RefreshPolicy,
            presence: super::CardPresence,
            alert: super::CardAlert,
        },
        Rss {
            id: String,
            title: String,
            url: String,
            max_items: u8,
            template: super::DisplayTemplate,
            tap_action: super::WidgetTapAction,
            refresh: super::RefreshPolicy,
            presence: super::CardPresence,
            alert: super::CardAlert,
        },
    }

    /// Type-aware validation of allowed fields for each enum type.
    #[allow(private_bounds)]
    trait ValidatingDeserialize: for<'de> serde::Deserialize<'de> {
        fn allowed_fields(kind: &str) -> Option<&'static [&'static str]>;
    }

    impl ValidatingDeserialize for CardPresenceInner {
        fn allowed_fields(kind: &str) -> Option<&'static [&'static str]> {
            match kind {
                "in-rotation" => Some(&["kind", "dwell_seconds"]),
                "alert-only" | "off" => Some(&["kind"]),
                _ => None,
            }
        }
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
                "before-event" => Some(&["kind", "lead_minutes", "hold"]),
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
                "digital-clock" | "analog-clock" | "progress-ring" | "row-list"
                | "big-number-label" => Some(&["kind"]),
                "icon-badge-text" => Some(&["kind", "icon_asset_id"]),
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
                    "presence",
                    "alert",
                ]),
                "pomodoro" => Some(&[
                    "kind",
                    "id",
                    "label",
                    "duration_seconds",
                    "template",
                    "tap_action",
                    "refresh",
                    "presence",
                    "alert",
                ]),
                "calendar" => Some(&[
                    "kind",
                    "id",
                    "title",
                    "source",
                    "template",
                    "tap_action",
                    "refresh",
                    "presence",
                    "alert",
                ]),
                "weather" => Some(&[
                    "kind",
                    "id",
                    "title",
                    "location",
                    "units",
                    "template",
                    "tap_action",
                    "refresh",
                    "presence",
                    "alert",
                ]),
                "json-feed" => Some(&[
                    "kind",
                    "id",
                    "title",
                    "url",
                    "mappings",
                    "template",
                    "tap_action",
                    "refresh",
                    "presence",
                    "alert",
                ]),
                "rss" => Some(&[
                    "kind",
                    "id",
                    "title",
                    "url",
                    "max_items",
                    "template",
                    "tap_action",
                    "refresh",
                    "presence",
                    "alert",
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

pub const CURRENT_SCHEMA_VERSION: u32 = 3;
pub const MAX_WIDGET_TITLE_LEN: usize = 64;
pub const MAX_TIMEZONE_LEN: usize = 64;
pub const MAX_ICS_SOURCE_LEN: usize = 2_048;
pub const MAX_PROVIDER_URL_LEN: usize = 2_048;
pub const MAX_PROVIDER_RESPONSE_BYTES: usize = providers::MAX_PROVIDER_RESPONSE_BYTES;
pub const MAX_PROVIDER_REDIRECTS: u8 = providers::MAX_PROVIDER_REDIRECTS;
pub const PROVIDER_REQUEST_TIMEOUT_SECONDS: u64 = providers::PROVIDER_REQUEST_TIMEOUT.as_secs();
pub const MAX_LOCATION_LEN: usize = 128;
pub const MAX_JSON_PATH_LEN: usize = 256;
pub const MAX_JSON_MAPPINGS: usize = 16;
pub const MAX_ASSETS: usize = 16;
pub const MAX_ASSET_SOURCE_LEN: usize = 2_048;
pub const MAX_ASSET_BYTES: u32 = 262_144;
pub const MAX_TOTAL_ASSET_BYTES: u32 = 1_048_576;
pub const MAX_ICON_DIMENSION: u16 = 128;
pub const MAX_FONT_GLYPHS: u16 = 512;
pub const MAX_GLYPH_RANGES: usize = 16;
pub const MAX_HOST_ACTION_TARGET_LEN: usize = 2_048;
pub const MAX_UPDATE_ARTIFACT_BYTES: u32 = 1_048_576;
pub const MAX_UPDATE_VERSION_LEN: usize = 64;
pub const MAX_UPDATE_MODEL_LEN: usize = 64;
pub const MAX_SIGNING_KEY_ID_LEN: usize = 64;
pub const ED25519_SIGNATURE_BASE64_LEN: usize = 88;
pub const MAX_RSS_ITEMS: u8 = 5;
pub const MIN_POMODORO_SECONDS: u32 = 1;
pub const MAX_POMODORO_SECONDS: u32 = 86_400;
pub const MIN_CALENDAR_REFRESH_MINUTES: u16 = 1;
pub const MIN_WEATHER_REFRESH_MINUTES: u16 = 10;
pub const MAX_CALENDAR_REFRESH_MINUTES: u16 = 1_440;
pub const MAX_CONFIG_CARDS: usize = 8;
// Every in-rotation-or-alert-only card can lower to one wire widget, so the card cap
// must never exceed what the protocol's `ApplyConfig` encoder accepts.
const _: () = assert!(MAX_CONFIG_CARDS <= protocol::MAX_CONFIG_WIDGETS);
pub const MIN_DWELL_SECONDS: u16 = 5;
pub const MAX_DWELL_SECONDS: u16 = 3_600;
pub const MIN_ALERT_LEAD_MINUTES: u16 = 1;
pub const MAX_ALERT_LEAD_MINUTES: u16 = 60;
pub const MIN_ALERT_HOLD_SECONDS: u16 = 5;
pub const MAX_ALERT_HOLD_SECONDS: u16 = 600;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub schema_version: u32,
    pub preferences: AppPreferences,
    pub cards: Vec<CardSettings>,
    pub assets: Vec<AssetSettings>,
    pub carousel: CarouselSettings,
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
                presence: CardPresence::InRotation {
                    dwell_seconds: None,
                },
                alert: CardAlert::None,
            }],
            assets: Vec::new(),
            carousel: CarouselSettings::default(),
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
        let mut in_rotation = 0_usize;
        for (index, card) in self.cards.iter().enumerate() {
            let path = format!("cards[{index}]");
            validate_identifier(
                &format!("{path}.id"),
                card.id(),
                MAX_WIDGET_ID_LEN,
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
            if card.presence().is_in_rotation() {
                in_rotation += 1;
            }
        }
        if in_rotation == 0 && !self.cards.is_empty() {
            issues.push(ValidationIssue::new(
                "cards",
                ValidationCode::OutOfRange,
                "at least one card must be in the rotation",
            ));
        }
        if let CarouselAdvance::Timed {
            default_dwell_seconds,
        } = self.carousel.advance
        {
            validate_range(
                "carousel.advance.default_dwell_seconds",
                u32::from(default_dwell_seconds),
                u32::from(MIN_DWELL_SECONDS),
                u32::from(MAX_DWELL_SECONDS),
                &mut issues,
            );
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
        for (index, card) in self.cards.iter().enumerate() {
            if let DisplayTemplate::IconBadgeText {
                icon_asset_id: Some(asset_id),
            } = card.template()
            {
                match assets_by_id.get(asset_id.as_str()) {
                    None => issues.push(ValidationIssue::new(
                        format!("cards[{index}].template.icon_asset_id"),
                        ValidationCode::MissingReference,
                        format!("asset {asset_id:?} does not exist"),
                    )),
                    Some(kind) if !matches!(kind, AssetKind::Icon { .. }) => {
                        issues.push(ValidationIssue::new(
                            format!("cards[{index}].template.icon_asset_id"),
                            ValidationCode::InvalidComposition,
                            "icon template must reference an icon asset",
                        ));
                    }
                    Some(_) => {}
                }
            }
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
        let mut widgets = Vec::with_capacity(self.cards.len());
        let mut screens = Vec::with_capacity(self.cards.len());
        for (index, card) in self.cards.iter().enumerate() {
            if card.presence().is_off() {
                continue;
            }
            let Some(widget) = card.wire_config() else {
                compatibility_issues.push(ValidationIssue::new(
                    format!("cards[{index}]"),
                    ValidationCode::RequiresCapability,
                    "card provider/template/action is not implemented by this build",
                ));
                continue;
            };
            if card.presence().is_in_rotation() {
                // The screen ID is the card ID. The protocol treats widget and screen
                // IDs as distinct namespaces, so reuse is legal, and compilation still
                // invents no identifiers.
                screens.push(ScreenConfig {
                    screen_id: card.id().to_owned(),
                    widget_id: card.id().to_owned(),
                });
            }
            widgets.push(widget);
        }
        if !self.assets.is_empty() {
            compatibility_issues.push(ValidationIssue::new(
                "assets",
                ValidationCode::RequiresCapability,
                "asset transfer is not implemented by this build",
            ));
        }
        if !compatibility_issues.is_empty() {
            return Err(ConfigValidationError {
                issues: compatibility_issues,
            });
        }
        let initial_pushes = self
            .cards
            .iter()
            .filter(|card| !card.presence().is_off())
            .map(|card| PushData {
                widget_id: card.id().into(),
                revision,
                fields: card.initial_fields(),
            })
            .collect();

        Ok(CompiledAppConfig {
            layout: ApplyConfig {
                revision,
                rotation: self.preferences.orientation.rotation_degrees(),
                widgets,
                screens,
            },
            initial_pushes,
            required_capabilities: self.required_device_capabilities(),
        })
    }

    pub fn required_device_capabilities(&self) -> u64 {
        let mut required = protocol::CAPABILITY_CORE_WIDGETS;
        if self.preferences.orientation == DisplayOrientation::LandscapeFlipped {
            required |= protocol::CAPABILITY_CONFIG_ROTATION;
        }
        if self.cards.iter().any(|card| {
            !matches!(
                card.template(),
                DisplayTemplate::DigitalClock
                    | DisplayTemplate::ProgressRing
                    | DisplayTemplate::RowList
            )
        }) {
            required |= protocol::CAPABILITY_EXTENDED_TEMPLATES;
        }
        if self.cards.iter().any(|card| {
            matches!(
                card.tap_action(),
                WidgetTapAction::Dismiss
                    | WidgetTapAction::OpenUrl { .. }
                    | WidgetTapAction::OpenApplication { .. }
            )
        }) {
            required |= protocol::CAPABILITY_HOST_TAP_ACTIONS;
        }
        if !self.assets.is_empty() {
            required |= protocol::CAPABILITY_ASSET_TRANSFER;
        }
        required
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
/// runtime's device time-sync path (`runtime::send_time_sync`) and the desktop
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
    RowList,
    BigNumberLabel,
    IconBadgeText { icon_asset_id: Option<String> },
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
            strict_tagged_enum::DisplayTemplateInner::RowList => Self::RowList,
            strict_tagged_enum::DisplayTemplateInner::BigNumberLabel => Self::BigNumberLabel,
            strict_tagged_enum::DisplayTemplateInner::IconBadgeText { icon_asset_id } => {
                Self::IconBadgeText { icon_asset_id }
            }
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

impl RefreshPolicy {
    pub const fn interval_minutes(self) -> Option<u16> {
        match self {
            Self::Interval { minutes } => Some(minutes),
            Self::DeviceLocal | Self::Manual => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CardPresence {
    InRotation { dwell_seconds: Option<u16> },
    AlertOnly,
    Off,
}

impl<'de> Deserialize<'de> for CardPresence {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let inner: strict_tagged_enum::CardPresenceInner =
            strict_tagged_enum::validate_and_deserialize(&value)
                .map_err(serde::de::Error::custom)?;
        Ok(match inner {
            strict_tagged_enum::CardPresenceInner::InRotation { dwell_seconds } => {
                CardPresence::InRotation { dwell_seconds }
            }
            strict_tagged_enum::CardPresenceInner::AlertOnly => CardPresence::AlertOnly,
            strict_tagged_enum::CardPresenceInner::Off => CardPresence::Off,
        })
    }
}

impl CardPresence {
    pub const fn dwell_seconds(self, default: u16) -> Option<u16> {
        match self {
            Self::InRotation { dwell_seconds } => Some(match dwell_seconds {
                Some(seconds) => seconds,
                None => default,
            }),
            Self::AlertOnly | Self::Off => None,
        }
    }

    pub const fn is_in_rotation(self) -> bool {
        matches!(self, Self::InRotation { .. })
    }

    pub const fn is_off(self) -> bool {
        matches!(self, Self::Off)
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
    BeforeEvent { lead_minutes: u16, hold: AlertHold },
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
            strict_tagged_enum::CardAlertInner::BeforeEvent { lead_minutes, hold } => {
                CardAlert::BeforeEvent { lead_minutes, hold }
            }
        })
    }
}

impl CardAlert {
    pub const fn is_none(self) -> bool {
        matches!(self, Self::None)
    }

    pub const fn hold(self) -> Option<AlertHold> {
        match self {
            Self::None => None,
            Self::OnTimerFinish { hold } | Self::BeforeEvent { hold, .. } => Some(hold),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WeatherUnits {
    Metric,
    Imperial,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JsonFieldMapping {
    pub field: String,
    pub path: String,
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
        presence: CardPresence,
        alert: CardAlert,
    },
    Pomodoro {
        id: String,
        label: String,
        duration_seconds: u32,
        template: DisplayTemplate,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
        presence: CardPresence,
        alert: CardAlert,
    },
    Calendar {
        id: String,
        title: String,
        source: CalendarSource,
        template: DisplayTemplate,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
        presence: CardPresence,
        alert: CardAlert,
    },
    Weather {
        id: String,
        title: String,
        location: String,
        units: WeatherUnits,
        template: DisplayTemplate,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
        presence: CardPresence,
        alert: CardAlert,
    },
    JsonFeed {
        id: String,
        title: String,
        url: String,
        mappings: Vec<JsonFieldMapping>,
        template: DisplayTemplate,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
        presence: CardPresence,
        alert: CardAlert,
    },
    Rss {
        id: String,
        title: String,
        url: String,
        max_items: u8,
        template: DisplayTemplate,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
        presence: CardPresence,
        alert: CardAlert,
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
                presence,
                alert,
            } => CardSettings::Clock {
                id,
                title,
                show_seconds,
                template,
                tap_action,
                refresh,
                presence,
                alert,
            },
            strict_tagged_enum::CardSettingsInner::Pomodoro {
                id,
                label,
                duration_seconds,
                template,
                tap_action,
                refresh,
                presence,
                alert,
            } => CardSettings::Pomodoro {
                id,
                label,
                duration_seconds,
                template,
                tap_action,
                refresh,
                presence,
                alert,
            },
            strict_tagged_enum::CardSettingsInner::Calendar {
                id,
                title,
                source,
                template,
                tap_action,
                refresh,
                presence,
                alert,
            } => CardSettings::Calendar {
                id,
                title,
                source,
                template,
                tap_action,
                refresh,
                presence,
                alert,
            },
            strict_tagged_enum::CardSettingsInner::Weather {
                id,
                title,
                location,
                units,
                template,
                tap_action,
                refresh,
                presence,
                alert,
            } => CardSettings::Weather {
                id,
                title,
                location,
                units,
                template,
                tap_action,
                refresh,
                presence,
                alert,
            },
            strict_tagged_enum::CardSettingsInner::JsonFeed {
                id,
                title,
                url,
                mappings,
                template,
                tap_action,
                refresh,
                presence,
                alert,
            } => CardSettings::JsonFeed {
                id,
                title,
                url,
                mappings,
                template,
                tap_action,
                refresh,
                presence,
                alert,
            },
            strict_tagged_enum::CardSettingsInner::Rss {
                id,
                title,
                url,
                max_items,
                template,
                tap_action,
                refresh,
                presence,
                alert,
            } => CardSettings::Rss {
                id,
                title,
                url,
                max_items,
                template,
                tap_action,
                refresh,
                presence,
                alert,
            },
        })
    }
}

impl CardSettings {
    pub fn id(&self) -> &str {
        match self {
            Self::Clock { id, .. }
            | Self::Pomodoro { id, .. }
            | Self::Calendar { id, .. }
            | Self::Weather { id, .. }
            | Self::JsonFeed { id, .. }
            | Self::Rss { id, .. } => id,
        }
    }

    pub const fn presence(&self) -> CardPresence {
        match self {
            Self::Clock { presence, .. }
            | Self::Pomodoro { presence, .. }
            | Self::Calendar { presence, .. }
            | Self::Weather { presence, .. }
            | Self::JsonFeed { presence, .. }
            | Self::Rss { presence, .. } => *presence,
        }
    }

    pub const fn alert(&self) -> CardAlert {
        match self {
            Self::Clock { alert, .. }
            | Self::Pomodoro { alert, .. }
            | Self::Calendar { alert, .. }
            | Self::Weather { alert, .. }
            | Self::JsonFeed { alert, .. }
            | Self::Rss { alert, .. } => *alert,
        }
    }

    pub const fn refresh(&self) -> RefreshPolicy {
        match self {
            Self::Clock { refresh, .. }
            | Self::Pomodoro { refresh, .. }
            | Self::Calendar { refresh, .. }
            | Self::Weather { refresh, .. }
            | Self::JsonFeed { refresh, .. }
            | Self::Rss { refresh, .. } => *refresh,
        }
    }

    pub fn template(&self) -> &DisplayTemplate {
        match self {
            Self::Clock { template, .. }
            | Self::Pomodoro { template, .. }
            | Self::Calendar { template, .. }
            | Self::Weather { template, .. }
            | Self::JsonFeed { template, .. }
            | Self::Rss { template, .. } => template,
        }
    }

    pub fn tap_action(&self) -> &WidgetTapAction {
        match self {
            Self::Clock { tap_action, .. }
            | Self::Pomodoro { tap_action, .. }
            | Self::Calendar { tap_action, .. }
            | Self::Weather { tap_action, .. }
            | Self::JsonFeed { tap_action, .. }
            | Self::Rss { tap_action, .. } => tap_action,
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
                    ProviderKind::Clock,
                    template,
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
                    ProviderKind::Pomodoro,
                    template,
                    tap_action,
                    *refresh,
                    issues,
                );
            }
            Self::Calendar {
                title,
                source,
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
                source.validate(&format!("{path}.source"), issues);
                validate_composition(
                    path,
                    ProviderKind::Calendar,
                    template,
                    tap_action,
                    *refresh,
                    issues,
                );
            }
            Self::Weather {
                title,
                location,
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
                validate_text(
                    &format!("{path}.location"),
                    location,
                    MAX_LOCATION_LEN,
                    true,
                    issues,
                );
                validate_composition(
                    path,
                    ProviderKind::Weather,
                    template,
                    tap_action,
                    *refresh,
                    issues,
                );
                if let RefreshPolicy::Interval { minutes } = refresh
                    && *minutes < MIN_WEATHER_REFRESH_MINUTES
                {
                    issues.push(ValidationIssue::new(
                        format!("{path}.refresh.minutes"),
                        ValidationCode::OutOfRange,
                        format!(
                            "weather refresh interval must be at least {MIN_WEATHER_REFRESH_MINUTES} minutes"
                        ),
                    ));
                }
            }
            Self::JsonFeed {
                title,
                url,
                mappings,
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
                validate_http_url(&format!("{path}.url"), url, issues);
                validate_collection_bounds(
                    &format!("{path}.mappings"),
                    mappings.len(),
                    MAX_JSON_MAPPINGS,
                    issues,
                );
                let mut fields = HashSet::with_capacity(mappings.len());
                for (index, mapping) in mappings.iter().enumerate() {
                    let mapping_path = format!("{path}.mappings[{index}]");
                    validate_identifier(
                        &format!("{mapping_path}.field"),
                        &mapping.field,
                        protocol::MAX_FIELD_KEY_LEN,
                        issues,
                    );
                    validate_text(
                        &format!("{mapping_path}.path"),
                        &mapping.path,
                        MAX_JSON_PATH_LEN,
                        true,
                        issues,
                    );
                    if providers::json_feed::is_reserved_field(&mapping.field) {
                        issues.push(ValidationIssue::new(
                            format!("{mapping_path}.field"),
                            ValidationCode::InvalidComposition,
                            "JSON mapping field is reserved for provider metadata",
                        ));
                    }
                    if !fields.insert(mapping.field.as_str()) {
                        issues.push(ValidationIssue::new(
                            format!("{mapping_path}.field"),
                            ValidationCode::DuplicateId,
                            "JSON mapping field is duplicated",
                        ));
                    }
                }
                validate_composition(
                    path,
                    ProviderKind::JsonFeed,
                    template,
                    tap_action,
                    *refresh,
                    issues,
                );
            }
            Self::Rss {
                title,
                url,
                max_items,
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
                validate_http_url(&format!("{path}.url"), url, issues);
                if !(1..=MAX_RSS_ITEMS).contains(max_items) {
                    issues.push(ValidationIssue::new(
                        format!("{path}.max_items"),
                        ValidationCode::OutOfRange,
                        format!("RSS item count must be 1..={MAX_RSS_ITEMS}"),
                    ));
                }
                validate_composition(
                    path,
                    ProviderKind::Rss,
                    template,
                    tap_action,
                    *refresh,
                    issues,
                );
            }
        }
        self.tap_action()
            .validate(&format!("{path}.tap_action"), issues);
        if let DisplayTemplate::IconBadgeText {
            icon_asset_id: Some(asset_id),
        } = self.template()
        {
            validate_identifier(
                &format!("{path}.template.icon_asset_id"),
                asset_id,
                MAX_WIDGET_ID_LEN,
                issues,
            );
        }
    }

    fn wire_config(&self) -> Option<WidgetConfig> {
        let template = match self.template() {
            DisplayTemplate::DigitalClock => TemplateKind::DigitalClock,
            DisplayTemplate::ProgressRing => TemplateKind::ProgressRing,
            DisplayTemplate::RowList => TemplateKind::RowList,
            DisplayTemplate::AnalogClock => TemplateKind::AnalogClock,
            DisplayTemplate::BigNumberLabel => TemplateKind::BigNumberLabel,
            DisplayTemplate::IconBadgeText { .. } => TemplateKind::IconBadgeText,
        };
        let tap_action = match self.tap_action() {
            WidgetTapAction::None => TapAction::None,
            WidgetTapAction::StartPause => TapAction::StartPause,
            WidgetTapAction::Reset => TapAction::Reset,
            WidgetTapAction::Dismiss
            | WidgetTapAction::OpenUrl { .. }
            | WidgetTapAction::OpenApplication { .. } => return None,
        };
        Some(WidgetConfig {
            widget_id: self.id().into(),
            template,
            size_class: protocol::SizeClass::Full,
            tap_action,
            interrupt_policy: if self.alert().is_none() {
                InterruptPolicy::Disabled
            } else {
                InterruptPolicy::Enabled
            },
        })
    }

    fn initial_fields(&self) -> Vec<Field> {
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
            Self::Calendar { title, .. } => {
                let mut fields = Vec::with_capacity(13);
                fields.push(text_field("title", title));
                for row in 0..5 {
                    fields.push(text_field(&format!("row{row}_title"), ""));
                    fields.push(text_field(&format!("row{row}_time"), ""));
                }
                fields.push(bool_field("stale", true));
                fields.push(text_field("error", "Waiting for calendar refresh"));
                fields
            }
            Self::Weather { title, .. } => vec![
                text_field("title", title),
                bool_field("stale", true),
                text_field("error", "Waiting for provider refresh"),
            ],
            Self::JsonFeed {
                title, mappings, ..
            } => {
                let mut fields = Vec::with_capacity(protocol::MAX_FIELD_COUNT);
                fields.push(text_field("title", title));
                fields.extend(
                    mappings
                        .iter()
                        .filter(|mapping| {
                            !matches!(mapping.field.as_str(), "title" | "stale" | "error")
                        })
                        .take(protocol::MAX_FIELD_COUNT - 3)
                        .map(|mapping| text_field(&mapping.field, "")),
                );
                fields.push(bool_field("stale", true));
                fields.push(text_field("error", "Waiting for provider refresh"));
                fields
            }
            Self::Rss { title, .. } => {
                let mut fields = Vec::with_capacity(13);
                fields.push(text_field("title", title));
                for row in 0..5 {
                    fields.push(text_field(&format!("row{row}_title"), ""));
                    fields.push(text_field(&format!("row{row}_time"), ""));
                }
                fields.push(bool_field("stale", true));
                fields.push(text_field("error", "Waiting for provider refresh"));
                fields
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "kebab-case",
    deny_unknown_fields
)]
pub enum CalendarSource {
    File(String),
    Url(String),
}

impl CalendarSource {
    pub fn value(&self) -> &str {
        match self {
            Self::File(value) | Self::Url(value) => value,
        }
    }

    fn validate(&self, path: &str, issues: &mut Vec<ValidationIssue>) {
        match self {
            Self::File(file) => validate_text(path, file, MAX_ICS_SOURCE_LEN, true, issues),
            Self::Url(url) => validate_http_url(path, url, issues),
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum AssetKind {
    Icon {
        width: u16,
        height: u16,
    },
    Font {
        pixel_size: u16,
        glyph_ranges: Vec<GlyphRange>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlyphRange {
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CarouselSettings {
    pub advance: CarouselAdvance,
}

impl Default for CarouselSettings {
    fn default() -> Self {
        Self {
            advance: CarouselAdvance::Manual,
        }
    }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FirmwareArtifactMetadata {
    pub version: String,
    pub model: String,
    pub byte_length: u32,
    pub sha256_hex: String,
    pub signing_key_id: String,
    pub signature_base64: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledAppConfig {
    pub layout: ApplyConfig,
    pub initial_pushes: Vec<PushData>,
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
    MissingScreen,
    DuplicateReference,
    UnsupportedSize,
    OutOfRange,
    InvalidTimezone,
    InvalidSource,
    InvalidComposition,
    Overlap,
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
enum ProviderKind {
    Clock,
    Pomodoro,
    Calendar,
    Weather,
    JsonFeed,
    Rss,
}

fn validate_composition(
    path: &str,
    provider: ProviderKind,
    template: &DisplayTemplate,
    tap_action: &WidgetTapAction,
    refresh: RefreshPolicy,
    issues: &mut Vec<ValidationIssue>,
) {
    // A pairing is allowed only when the provider actually populates the fields the
    // template declares (`firmware/main/core/template_fields.c`). A template whose
    // renderable fields the provider never sends draws its placeholders forever, and
    // every field the provider sends that the template does not declare is counted in
    // the device's `unknown_field_count` on EVERY refresh — degrading the diagnostic
    // that exists to catch real host/firmware schema drift.
    let template_supported = match provider {
        // Clock sends only title/show_seconds; `big-number-label`'s `value` would
        // never be written and the card would show a permanent "--".
        ProviderKind::Clock => matches!(
            template,
            DisplayTemplate::DigitalClock | DisplayTemplate::AnalogClock
        ),
        // Pomodoro sends `label`, `duration_seconds`, `remaining_seconds` and
        // `running`; `big-number-label` declares only `label` out of those, so its
        // hero `value` stayed "--" forever while the other three counted as unknown
        // on EVERY tick — a continuous drip, worse than the calendar case above.
        // Unlike weather's `row-list` this strands no saved configuration: v0/v1
        // migration hard-codes pomodoro to `ProgressRing`, and while v2 migration
        // copies `template` verbatim, no v2 file could hold `big-number-label` on a
        // pomodoro card because `save_and_apply` compiled before persisting and
        // `wire_config()` refused to lower that template at the time.
        ProviderKind::Pomodoro => matches!(template, DisplayTemplate::ProgressRing),
        // Calendar and RSS send `title` plus ten `rowN_*` fields, which only
        // `row-list` declares. On `icon-badge-text` all ten counted as unknown on
        // every refresh and the card rendered the hollow `unknown` ring and "--".
        ProviderKind::Calendar | ProviderKind::Rss => {
            matches!(template, DisplayTemplate::RowList)
        }
        // `icon-badge-text` is what weather's field set was designed for and
        // `big-number-label` renders its `title`/`value`/`label` subset. `RowList`
        // must stay: `validate()` runs on config LOAD, cards are never migrated to a
        // new template, and every weather card saved before the extended templates
        // shipped is still on `row-list` (it was the only weather-legal template
        // `wire_config()` could lower back then). Removing it would make those saved
        // configurations fail to load.
        ProviderKind::Weather => matches!(
            template,
            DisplayTemplate::BigNumberLabel
                | DisplayTemplate::IconBadgeText { .. }
                | DisplayTemplate::RowList
        ),
        // Every json-feed field is user-mapped by name, so the user can populate any
        // template's declared text fields, including `row-list`'s `rowN_*` set.
        ProviderKind::JsonFeed => matches!(
            template,
            DisplayTemplate::BigNumberLabel
                | DisplayTemplate::IconBadgeText { .. }
                | DisplayTemplate::RowList
        ),
    };
    if !template_supported {
        issues.push(ValidationIssue::new(
            format!("{path}.template"),
            ValidationCode::InvalidComposition,
            "display template is incompatible with this provider",
        ));
    }

    if matches!(
        tap_action,
        WidgetTapAction::StartPause | WidgetTapAction::Reset
    ) && !matches!(provider, ProviderKind::Pomodoro)
    {
        issues.push(ValidationIssue::new(
            format!("{path}.tap_action"),
            ValidationCode::InvalidComposition,
            "start/pause and reset actions require a pomodoro provider",
        ));
    }

    // `widget_model.c` refuses any widget whose template is not `PROGRESS_RING` while
    // carrying a non-`NONE` tap action, and `validate_config` is all-or-nothing: one
    // such widget makes the device reject the entire `ApplyConfig`, so no card updates
    // at all. Mirror that rule host-side instead of letting a saveable configuration
    // take the whole layout down.
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

    let refresh_supported = match provider {
        ProviderKind::Clock | ProviderKind::Pomodoro => {
            matches!(refresh, RefreshPolicy::DeviceLocal)
        }
        ProviderKind::Calendar
        | ProviderKind::Weather
        | ProviderKind::JsonFeed
        | ProviderKind::Rss => matches!(
            refresh,
            RefreshPolicy::Manual | RefreshPolicy::Interval { .. }
        ),
    };
    if !refresh_supported {
        issues.push(ValidationIssue::new(
            format!("{path}.refresh"),
            ValidationCode::InvalidComposition,
            "refresh policy is incompatible with this provider",
        ));
    }
    if let RefreshPolicy::Interval { minutes } = refresh
        && !(MIN_CALENDAR_REFRESH_MINUTES..=MAX_CALENDAR_REFRESH_MINUTES).contains(&minutes)
    {
        issues.push(ValidationIssue::new(
            format!("{path}.refresh.minutes"),
            ValidationCode::OutOfRange,
            format!(
                "refresh interval must be {MIN_CALENDAR_REFRESH_MINUTES}..={MAX_CALENDAR_REFRESH_MINUTES} minutes"
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
    validate_text(path, url, MAX_PROVIDER_URL_LEN, true, issues);
    if !url.is_empty() && providers::http::validate_http_url(url).is_err() {
        issues.push(ValidationIssue::new(
            path,
            ValidationCode::InvalidSource,
            "URL must be HTTP(S), include a host, and contain no credentials",
        ));
    }
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
    let presence = card.presence();
    let alert = card.alert();

    if let CardPresence::InRotation {
        dwell_seconds: Some(seconds),
    } = presence
    {
        validate_range(
            &format!("{path}.presence.dwell_seconds"),
            u32::from(seconds),
            u32::from(MIN_DWELL_SECONDS),
            u32::from(MAX_DWELL_SECONDS),
            issues,
        );
    }

    // An alert-only card with no trigger could never appear on screen.
    if matches!(presence, CardPresence::AlertOnly) && alert.is_none() {
        issues.push(ValidationIssue::new(
            format!("{path}.presence"),
            ValidationCode::OutOfRange,
            "an alert-only card must configure an alert",
        ));
    }

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
        CardAlert::BeforeEvent { lead_minutes, hold } => {
            if !matches!(card, CardSettings::Calendar { .. }) {
                issues.push(ValidationIssue::new(
                    format!("{path}.alert"),
                    ValidationCode::OutOfRange,
                    "before-event alerts are only valid on calendar cards",
                ));
            }
            validate_range(
                &format!("{path}.alert.lead_minutes"),
                u32::from(lead_minutes),
                u32::from(MIN_ALERT_LEAD_MINUTES),
                u32::from(MAX_ALERT_LEAD_MINUTES),
                issues,
            );
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
        validate_identifier(&format!("{path}.id"), &self.id, MAX_WIDGET_ID_LEN, issues);
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
            AssetKind::Icon { width, height } => {
                if !(1..=MAX_ICON_DIMENSION).contains(width)
                    || !(1..=MAX_ICON_DIMENSION).contains(height)
                {
                    issues.push(ValidationIssue::new(
                        format!("{path}.kind"),
                        ValidationCode::OutOfRange,
                        format!("icon dimensions must be 1..={MAX_ICON_DIMENSION} pixels"),
                    ));
                }
            }
            AssetKind::Font {
                pixel_size,
                glyph_ranges,
            } => {
                if !(8..=96).contains(pixel_size) {
                    issues.push(ValidationIssue::new(
                        format!("{path}.kind.pixel_size"),
                        ValidationCode::OutOfRange,
                        "font pixel size must be 8..=96",
                    ));
                }
                if glyph_ranges.is_empty() || glyph_ranges.len() > MAX_GLYPH_RANGES {
                    issues.push(ValidationIssue::new(
                        format!("{path}.kind.glyph_ranges"),
                        ValidationCode::OutOfRange,
                        format!("font must contain 1..={MAX_GLYPH_RANGES} glyph ranges"),
                    ));
                }
                let mut glyph_count = 0_u32;
                let mut previous_end = None;
                for (index, range) in glyph_ranges.iter().enumerate() {
                    let includes_surrogate = range.start <= 0xdfff && range.end >= 0xd800;
                    let overlaps_or_unsorted = previous_end.is_some_and(|end| range.start <= end);
                    if range.start > range.end
                        || range.end > 0x0010_ffff
                        || includes_surrogate
                        || overlaps_or_unsorted
                    {
                        issues.push(ValidationIssue::new(
                            format!("{path}.kind.glyph_ranges[{index}]"),
                            ValidationCode::OutOfRange,
                            "glyph ranges must be ascending, non-overlapping Unicode scalar values",
                        ));
                    } else {
                        glyph_count =
                            glyph_count.saturating_add(range.end.saturating_sub(range.start) + 1);
                        previous_end = Some(range.end);
                    }
                }
                if glyph_count > u32::from(MAX_FONT_GLYPHS) {
                    issues.push(ValidationIssue::new(
                        format!("{path}.kind.glyph_ranges"),
                        ValidationCode::TooMany,
                        format!("font must contain at most {MAX_FONT_GLYPHS} glyphs"),
                    ));
                }
            }
        }
    }
}

impl FirmwareArtifactMetadata {
    pub fn validate(&self) -> Result<(), ConfigValidationError> {
        let mut issues = Vec::new();
        validate_text(
            "version",
            &self.version,
            MAX_UPDATE_VERSION_LEN,
            true,
            &mut issues,
        );
        validate_text(
            "model",
            &self.model,
            MAX_UPDATE_MODEL_LEN,
            true,
            &mut issues,
        );
        if !(1..=MAX_UPDATE_ARTIFACT_BYTES).contains(&self.byte_length) {
            issues.push(ValidationIssue::new(
                "byte_length",
                ValidationCode::OutOfRange,
                format!("firmware artifact must be 1..={MAX_UPDATE_ARTIFACT_BYTES} bytes"),
            ));
        }
        if self.sha256_hex.len() != 64
            || !self.sha256_hex.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            issues.push(ValidationIssue::new(
                "sha256_hex",
                ValidationCode::InvalidSource,
                "firmware SHA-256 must contain exactly 64 hexadecimal characters",
            ));
        }
        validate_text(
            "signing_key_id",
            &self.signing_key_id,
            MAX_SIGNING_KEY_ID_LEN,
            true,
            &mut issues,
        );
        let signature = self.signature_base64.as_bytes();
        if signature.len() != ED25519_SIGNATURE_BASE64_LEN
            || signature[ED25519_SIGNATURE_BASE64_LEN - 2..] != *b"=="
            || !signature[..ED25519_SIGNATURE_BASE64_LEN - 2]
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/'))
        {
            issues.push(ValidationIssue::new(
                "signature_base64",
                ValidationCode::InvalidSource,
                "firmware Ed25519 signature must be an 88-character base64 value",
            ));
        }
        if issues.is_empty() {
            Ok(())
        } else {
            Err(ConfigValidationError { issues })
        }
    }
}

fn text_field(key: &str, value: &str) -> Field {
    Field {
        key: key.into(),
        value: FieldValue::Text(value.into()),
    }
}

fn int_field(key: &str, value: u32) -> Field {
    Field {
        key: key.into(),
        value: FieldValue::Integer(i64::from(value)),
    }
}

fn bool_field(key: &str, value: bool) -> Field {
    Field {
        key: key.into(),
        value: FieldValue::Boolean(value),
    }
}
