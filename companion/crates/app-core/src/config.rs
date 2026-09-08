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

    /// Mirrors `super::AssetKind`. The device now rasterizes any size from a TTF
    /// at runtime, so `font` carries no pixel size and no glyph ranges; `icon-font`
    /// carries a TTF plus a name→codepoint map so a plugin can write a symbolic
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
        },
        Pomodoro {
            id: String,
            label: String,
            duration_seconds: u32,
            template: super::DisplayTemplate,
            tap_action: super::WidgetTapAction,
            refresh: super::RefreshPolicy,
            alert: super::CardAlert,
        },
        Calendar {
            id: String,
            title: String,
            source: super::CalendarSource,
            template: super::DisplayTemplate,
            tap_action: super::WidgetTapAction,
            refresh: super::RefreshPolicy,
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
            alert: super::CardAlert,
        },
        /// A plugin card names a curated plugin by id and renders whatever its
        /// manifest compiles to (a host-pushed scene), not one of the six
        /// built-in `DisplayTemplate`s -- deliberately absent here, see
        /// `super::CardSettings::Plugin`.
        Plugin {
            id: String,
            title: String,
            plugin_id: String,
            tap_action: super::WidgetTapAction,
            refresh: super::RefreshPolicy,
            alert: super::CardAlert,
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
                ]),
                "calendar" => Some(&[
                    "kind",
                    "id",
                    "title",
                    "source",
                    "template",
                    "tap_action",
                    "refresh",
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
                    "alert",
                ]),
                "plugin" => Some(&[
                    "kind",
                    "id",
                    "title",
                    "plugin_id",
                    "tap_action",
                    "refresh",
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

pub const CURRENT_SCHEMA_VERSION: u32 = 6;
pub(crate) const DEFAULT_PLAYLIST_ID: &str = "my-playlist";
pub(crate) const DEFAULT_PLAYLIST_NAME: &str = "My playlist";
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
pub const MAX_ICON_GLYPHS: usize = 256;
pub const MAX_ICON_GLYPH_NAME_LEN: usize = 64;
pub const MAX_HOST_ACTION_TARGET_LEN: usize = 2_048;
pub const MAX_RSS_ITEMS: u8 = 5;
/// Bound for a `plugin` card's `plugin_id`, which names a curated plugin by its
/// manifest's own `name` field. This mirrors `plugin::manifest::MAX_NAME_LEN`
/// (64 bytes) rather than importing it: the `plugin` crate depends on `app-core`
/// (it compiles a manifest against `providers`/`app-core` types), so the reverse
/// dependency would be circular. Keep this value equal to that one by hand.
pub const MAX_PLUGIN_ID_LEN: usize = 64;
pub const MIN_POMODORO_SECONDS: u32 = 1;
pub const MAX_POMODORO_SECONDS: u32 = 86_400;
pub const MIN_CALENDAR_REFRESH_MINUTES: u16 = 1;
pub const MIN_WEATHER_REFRESH_MINUTES: u16 = 10;
pub const MAX_CALENDAR_REFRESH_MINUTES: u16 = 1_440;
pub const MAX_CONFIG_CARDS: usize = 8;
pub const MAX_PLAYLISTS: usize = 8;
pub const MAX_PLAYLIST_ENTRIES: usize = 8;
pub const MAX_PLAYLIST_NAME_LEN: usize = 48;
// Every compiled card can lower to one wire widget, so the card cap must never exceed
// what the protocol's `ApplyConfig` encoder accepts.
const _: () = assert!(MAX_CONFIG_CARDS <= protocol::MAX_CONFIG_WIDGETS);
pub const MIN_DWELL_SECONDS: u16 = 5;
pub const MAX_DWELL_SECONDS: u16 = 3_600;
pub const MIN_ALERT_LEAD_MINUTES: u16 = 1;
pub const MAX_ALERT_LEAD_MINUTES: u16 = 60;
pub const MIN_ALERT_HOLD_SECONDS: u16 = 5;
pub const MAX_ALERT_HOLD_SECONDS: u16 = 600;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub advance: CarouselAdvance,
    pub entries: Vec<PlaylistEntry>,
}

impl<'de> Deserialize<'de> for Playlist {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Fields {
            id: String,
            name: String,
            advance: CarouselAdvance,
            entries: Vec<PlaylistEntry>,
        }

        let value = serde_json::Value::deserialize(deserializer)?;
        let object = value
            .as_object()
            .ok_or_else(|| serde::de::Error::custom("playlist must be an object"))?;
        for key in object.keys() {
            if !["id", "name", "advance", "entries"].contains(&key.as_str()) {
                return Err(serde::de::Error::custom(format!(
                    "unknown playlist field: {key}"
                )));
            }
        }
        let fields: Fields = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        Ok(Self {
            id: fields.id,
            name: fields.name,
            advance: fields.advance,
            entries: fields.entries,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlaylistEntry {
    pub card_id: String,
    pub dwell_seconds: Option<u16>,
}

impl<'de> Deserialize<'de> for PlaylistEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Fields {
            card_id: String,
            dwell_seconds: Option<u16>,
        }

        let value = serde_json::Value::deserialize(deserializer)?;
        let object = value
            .as_object()
            .ok_or_else(|| serde::de::Error::custom("playlist entry must be an object"))?;
        for key in object.keys() {
            if !["card_id", "dwell_seconds"].contains(&key.as_str()) {
                return Err(serde::de::Error::custom(format!(
                    "unknown playlist entry field: {key}"
                )));
            }
        }
        let fields: Fields = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        Ok(Self {
            card_id: fields.card_id,
            dwell_seconds: fields.dwell_seconds,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub schema_version: u32,
    pub preferences: AppPreferences,
    pub cards: Vec<CardSettings>,
    pub assets: Vec<AssetSettings>,
    pub playlists: Vec<Playlist>,
    pub active_playlist_id: String,
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
            }],
            assets: Vec::new(),
            playlists: vec![Playlist {
                id: DEFAULT_PLAYLIST_ID.into(),
                name: DEFAULT_PLAYLIST_NAME.into(),
                advance: CarouselAdvance::Manual,
                entries: vec![PlaylistEntry {
                    card_id: "clock".into(),
                    dwell_seconds: None,
                }],
            }],
            active_playlist_id: DEFAULT_PLAYLIST_ID.into(),
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
        }

        validate_collection_bounds(
            "playlists",
            self.playlists.len(),
            MAX_PLAYLISTS,
            &mut issues,
        );
        let mut playlist_ids = HashSet::with_capacity(self.playlists.len());
        let mut playlist_names = HashSet::with_capacity(self.playlists.len());
        for (index, playlist) in self.playlists.iter().enumerate() {
            let path = format!("playlists[{index}]");
            validate_identifier_with_context(
                &format!("{path}.id"),
                &playlist.id,
                MAX_WIDGET_ID_LEN,
                &format!("playlist ID {:?}", playlist.id),
                &mut issues,
            );
            if !playlist_ids.insert(playlist.id.as_str()) {
                issues.push(ValidationIssue::new(
                    format!("{path}.id"),
                    ValidationCode::DuplicateId,
                    format!("playlist ID {:?} is duplicated", playlist.id),
                ));
            }
            if playlist.name.trim().is_empty() {
                issues.push(ValidationIssue::new(
                    format!("{path}.name"),
                    ValidationCode::Empty,
                    format!("playlist {:?} name must not be empty", playlist.id),
                ));
            }
            if playlist.name.chars().count() > MAX_PLAYLIST_NAME_LEN {
                issues.push(ValidationIssue::new(
                    format!("{path}.name"),
                    ValidationCode::TooLong,
                    format!(
                        "playlist {:?} name must be at most {MAX_PLAYLIST_NAME_LEN} characters",
                        playlist.id
                    ),
                ));
            }
            if !playlist_names.insert(playlist.name.trim()) {
                issues.push(ValidationIssue::new(
                    format!("{path}.name"),
                    ValidationCode::DuplicateId,
                    format!(
                        "playlist {:?} duplicates the name {:?}",
                        playlist.id, playlist.name
                    ),
                ));
            }

            if playlist.entries.is_empty() {
                let (code, message) =
                    if playlist.id == self.active_playlist_id && !self.cards.is_empty() {
                        (
                            ValidationCode::OutOfRange,
                            format!(
                                "active playlist {:?} must contain at least one card",
                                playlist.id
                            ),
                        )
                    } else {
                        (
                            ValidationCode::Empty,
                            format!("playlist {:?} must contain at least one entry", playlist.id),
                        )
                    };
                issues.push(ValidationIssue::new(
                    format!("{path}.entries"),
                    code,
                    message,
                ));
            } else if playlist.entries.len() > MAX_PLAYLIST_ENTRIES {
                issues.push(ValidationIssue::new(
                    format!("{path}.entries"),
                    ValidationCode::TooMany,
                    format!(
                        "playlist {:?} supports at most {MAX_PLAYLIST_ENTRIES} entries",
                        playlist.id
                    ),
                ));
            }

            if let CarouselAdvance::Timed {
                default_dwell_seconds,
            } = playlist.advance
                && !(MIN_DWELL_SECONDS..=MAX_DWELL_SECONDS).contains(&default_dwell_seconds)
            {
                issues.push(ValidationIssue::new(
                    format!("{path}.advance.default_dwell_seconds"),
                    ValidationCode::OutOfRange,
                    format!(
                        "playlist {:?} default dwell must be {MIN_DWELL_SECONDS}..={MAX_DWELL_SECONDS} seconds",
                        playlist.id
                    ),
                ));
            }

            let mut entry_card_ids = HashSet::with_capacity(playlist.entries.len());
            for (entry_index, entry) in playlist.entries.iter().enumerate() {
                let entry_path = format!("{path}.entries[{entry_index}]");
                validate_identifier_with_context(
                    &format!("{entry_path}.card_id"),
                    &entry.card_id,
                    MAX_WIDGET_ID_LEN,
                    &format!("card ID {:?} in playlist {:?}", entry.card_id, playlist.id),
                    &mut issues,
                );
                if !card_ids.contains(entry.card_id.as_str()) {
                    issues.push(ValidationIssue::new(
                        format!("{entry_path}.card_id"),
                        ValidationCode::MissingReference,
                        format!(
                            "card {:?} referenced by playlist {:?} does not exist",
                            entry.card_id, playlist.id
                        ),
                    ));
                }
                if !entry_card_ids.insert(entry.card_id.as_str()) {
                    issues.push(ValidationIssue::new(
                        format!("{entry_path}.card_id"),
                        ValidationCode::DuplicateId,
                        format!(
                            "card {:?} appears more than once in playlist {:?}",
                            entry.card_id, playlist.id
                        ),
                    ));
                }
                if let Some(dwell_seconds) = entry.dwell_seconds
                    && !(MIN_DWELL_SECONDS..=MAX_DWELL_SECONDS).contains(&dwell_seconds)
                {
                    issues.push(ValidationIssue::new(
                        format!("{entry_path}.dwell_seconds"),
                        ValidationCode::OutOfRange,
                        format!(
                            "card {:?} in playlist {:?} must dwell for {MIN_DWELL_SECONDS}..={MAX_DWELL_SECONDS} seconds",
                            entry.card_id, playlist.id
                        ),
                    ));
                }
            }
        }
        if self.active_playlist().is_none() {
            issues.push(ValidationIssue::new(
                "active_playlist_id",
                ValidationCode::MissingReference,
                format!(
                    "active playlist {:?} does not exist",
                    self.active_playlist_id
                ),
            ));
        }
        if self.compiled_card_ids().len() > MAX_CONFIG_CARDS {
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
        for (index, card) in self.cards.iter().enumerate() {
            if let Some(DisplayTemplate::IconBadgeText {
                icon_asset_id: Some(asset_id),
            }) = card.template()
            {
                match assets_by_id.get(asset_id.as_str()) {
                    None => issues.push(ValidationIssue::new(
                        format!("cards[{index}].template.icon_asset_id"),
                        ValidationCode::MissingReference,
                        format!("asset {asset_id:?} does not exist"),
                    )),
                    Some(kind) if !matches!(kind, AssetKind::IconFont { .. }) => {
                        issues.push(ValidationIssue::new(
                            format!("cards[{index}].template.icon_asset_id"),
                            ValidationCode::InvalidComposition,
                            "icon template must reference an icon-font asset",
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

    pub fn active_playlist(&self) -> Option<&Playlist> {
        self.playlists
            .iter()
            .find(|playlist| playlist.id == self.active_playlist_id)
    }

    /// Card ids the compiled widget set will contain: active-playlist entries
    /// (in entry order) followed by alert-capable cards outside it (by id).
    pub fn compiled_card_ids(&self) -> Vec<&str> {
        let mut card_ids = Vec::with_capacity(self.cards.len());
        let mut active_card_ids = HashSet::new();
        if let Some(playlist) = self.active_playlist() {
            for entry in &playlist.entries {
                card_ids.push(entry.card_id.as_str());
                active_card_ids.insert(entry.card_id.as_str());
            }
        }

        let mut alert_outsiders: Vec<&CardSettings> = self
            .cards
            .iter()
            .filter(|card| !card.alert().is_none() && !active_card_ids.contains(card.id()))
            .collect();
        alert_outsiders.sort_by(|left, right| left.id().cmp(right.id()));
        card_ids.extend(alert_outsiders.into_iter().map(CardSettings::id));
        card_ids
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
        let active_playlist = self
            .active_playlist()
            .expect("validated active playlist must exist");
        let active_card_ids: HashSet<&str> = active_playlist
            .entries
            .iter()
            .map(|entry| entry.card_id.as_str())
            .collect();
        let compiled_card_ids = self.compiled_card_ids();
        for card_id in &compiled_card_ids {
            let (index, card) = self
                .cards
                .iter()
                .enumerate()
                .find(|(_, card)| card.id() == *card_id)
                .expect("validated compiled card must exist");
            let Some(widget) = card.wire_config() else {
                compatibility_issues.push(ValidationIssue::new(
                    format!("cards[{index}]"),
                    ValidationCode::RequiresCapability,
                    "card provider/template/action is not implemented by this build",
                ));
                continue;
            };
            if active_card_ids.contains(*card_id) {
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
        if !compatibility_issues.is_empty() {
            return Err(ConfigValidationError {
                issues: compatibility_issues,
            });
        }
        let initial_pushes = compiled_card_ids
            .into_iter()
            .map(|card_id| {
                self.cards
                    .iter()
                    .find(|card| card.id() == card_id)
                    .expect("validated compiled card must exist")
            })
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
            assets: self.assets.clone(),
            required_capabilities: self.required_device_capabilities(),
        })
    }

    pub fn required_device_capabilities(&self) -> u64 {
        let mut required = protocol::CAPABILITY_CORE_WIDGETS;
        if self.preferences.orientation == DisplayOrientation::LandscapeFlipped {
            required |= protocol::CAPABILITY_CONFIG_ROTATION;
        }
        if self.cards.iter().any(|card| {
            // A plugin card's `template()` is `None`: it does not use any built-in
            // template, extended or otherwise, so it does not itself require this
            // capability (see `wire_config`'s comment for what it puts on the wire).
            matches!(
                card.template(),
                Some(t) if !matches!(
                    t,
                    DisplayTemplate::DigitalClock
                        | DisplayTemplate::ProgressRing
                        | DisplayTemplate::RowList
                )
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
        alert: CardAlert,
    },
    Pomodoro {
        id: String,
        label: String,
        duration_seconds: u32,
        template: DisplayTemplate,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
        alert: CardAlert,
    },
    Calendar {
        id: String,
        title: String,
        source: CalendarSource,
        template: DisplayTemplate,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
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
        alert: CardAlert,
    },
    /// Renders from a curated plugin's manifest-compiled scene rather than one
    /// of the six built-in `DisplayTemplate`s, so it carries no `template`
    /// field: there is nothing to select among. `plugin_id` names the plugin
    /// (matching its manifest's own `name`); the server resolves it to a
    /// manifest, fetches its data source, and pushes the resulting scene
    /// (Task 7). `title` is the card's own label, same as every other kind.
    Plugin {
        id: String,
        title: String,
        plugin_id: String,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
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
                alert,
            } => CardSettings::Clock {
                id,
                title,
                show_seconds,
                template,
                tap_action,
                refresh,
                alert,
            },
            strict_tagged_enum::CardSettingsInner::Pomodoro {
                id,
                label,
                duration_seconds,
                template,
                tap_action,
                refresh,
                alert,
            } => CardSettings::Pomodoro {
                id,
                label,
                duration_seconds,
                template,
                tap_action,
                refresh,
                alert,
            },
            strict_tagged_enum::CardSettingsInner::Calendar {
                id,
                title,
                source,
                template,
                tap_action,
                refresh,
                alert,
            } => CardSettings::Calendar {
                id,
                title,
                source,
                template,
                tap_action,
                refresh,
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
                alert,
            } => CardSettings::Weather {
                id,
                title,
                location,
                units,
                template,
                tap_action,
                refresh,
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
                alert,
            } => CardSettings::JsonFeed {
                id,
                title,
                url,
                mappings,
                template,
                tap_action,
                refresh,
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
                alert,
            } => CardSettings::Rss {
                id,
                title,
                url,
                max_items,
                template,
                tap_action,
                refresh,
                alert,
            },
            strict_tagged_enum::CardSettingsInner::Plugin {
                id,
                title,
                plugin_id,
                tap_action,
                refresh,
                alert,
            } => CardSettings::Plugin {
                id,
                title,
                plugin_id,
                tap_action,
                refresh,
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
            | Self::Rss { id, .. }
            | Self::Plugin { id, .. } => id,
        }
    }

    pub const fn alert(&self) -> CardAlert {
        match self {
            Self::Clock { alert, .. }
            | Self::Pomodoro { alert, .. }
            | Self::Calendar { alert, .. }
            | Self::Weather { alert, .. }
            | Self::JsonFeed { alert, .. }
            | Self::Rss { alert, .. }
            | Self::Plugin { alert, .. } => *alert,
        }
    }

    pub const fn refresh(&self) -> RefreshPolicy {
        match self {
            Self::Clock { refresh, .. }
            | Self::Pomodoro { refresh, .. }
            | Self::Calendar { refresh, .. }
            | Self::Weather { refresh, .. }
            | Self::JsonFeed { refresh, .. }
            | Self::Rss { refresh, .. }
            | Self::Plugin { refresh, .. } => *refresh,
        }
    }

    /// `None` for a plugin card: it has no `DisplayTemplate` to select among
    /// because it renders from its manifest-compiled scene, not one of the six
    /// built-in templates. Callers that only care about the built-in surface
    /// (the `IconBadgeText` asset check, the extended-templates capability
    /// gate) already treat `None` as "nothing to check here"; callers that
    /// need a wire `TemplateKind` regardless (`wire_config`) pick an inert
    /// placeholder explicitly, with their own comment.
    pub fn template(&self) -> Option<&DisplayTemplate> {
        match self {
            Self::Clock { template, .. }
            | Self::Pomodoro { template, .. }
            | Self::Calendar { template, .. }
            | Self::Weather { template, .. }
            | Self::JsonFeed { template, .. }
            | Self::Rss { template, .. } => Some(template),
            Self::Plugin { .. } => None,
        }
    }

    pub fn tap_action(&self) -> &WidgetTapAction {
        match self {
            Self::Clock { tap_action, .. }
            | Self::Pomodoro { tap_action, .. }
            | Self::Calendar { tap_action, .. }
            | Self::Weather { tap_action, .. }
            | Self::JsonFeed { tap_action, .. }
            | Self::Rss { tap_action, .. }
            | Self::Plugin { tap_action, .. } => tap_action,
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
                    ProviderKind::Pomodoro,
                    Some(template),
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
                    Some(template),
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
                    Some(template),
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
                    Some(template),
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
                    Some(template),
                    tap_action,
                    *refresh,
                    issues,
                );
            }
            Self::Plugin {
                title,
                plugin_id,
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
                    &format!("{path}.plugin_id"),
                    plugin_id,
                    MAX_PLUGIN_ID_LEN,
                    issues,
                );
                validate_composition(
                    path,
                    ProviderKind::Plugin,
                    None,
                    tap_action,
                    *refresh,
                    issues,
                );
            }
        }
        self.tap_action()
            .validate(&format!("{path}.tap_action"), issues);
        if let Some(DisplayTemplate::IconBadgeText {
            icon_asset_id: Some(asset_id),
        }) = self.template()
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
            // A plugin card (`None`) carries no `DisplayTemplate`: it renders from a
            // host-pushed scene (`PushScene`), not any of the six built-in C
            // templates. Firmware no longer switches on this field to choose a
            // renderer at all (stage 3a retired every built-in template in favour of
            // scenes for every card), so this byte is inert for a plugin card;
            // `DigitalClock` is picked arbitrarily to keep `WidgetConfig` fully
            // populated for older tooling that still reads it. Do not read rendering
            // meaning into it for a plugin card, and do not read the merge with the
            // real digital-clock arm below as anything but that shared byte value.
            Some(DisplayTemplate::DigitalClock) | None => TemplateKind::DigitalClock,
            Some(DisplayTemplate::ProgressRing) => TemplateKind::ProgressRing,
            Some(DisplayTemplate::RowList) => TemplateKind::RowList,
            Some(DisplayTemplate::AnalogClock) => TemplateKind::AnalogClock,
            Some(DisplayTemplate::BigNumberLabel) => TemplateKind::BigNumberLabel,
            Some(DisplayTemplate::IconBadgeText { .. }) => TemplateKind::IconBadgeText,
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
            // A plugin's own fields are named by its manifest (`field.*` bindings),
            // which app-core does not parse -- that is the plugin crate's job, and
            // the manifest lives in a file this crate never reads. Only the
            // housekeeping fields every provider-backed card carries are known here
            // (the same set weather uses); the server pushes the plugin's real
            // fields once it resolves the manifest and fetches the first snapshot
            // (Task 7).
            Self::Weather { title, .. } | Self::Plugin { title, .. } => vec![
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

/// The device rasterizes glyphs at any size from a TTF/OTF at runtime (`tiny_ttf`),
/// so unlike v4's `Icon { width, height }` / `Font { pixel_size, glyph_ranges }`, no
/// v5 variant pins a size or a pre-baked glyph set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AssetKind {
    /// A TTF/OTF text font. Size is chosen per text node at render time.
    Font,
    /// A TTF/OTF icon font plus a name→codepoint map, so a plugin can write
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
    pub initial_pushes: Vec<PushData>,
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

fn validate_identifier_with_context(
    path: &str,
    value: &str,
    maximum: usize,
    context: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    if value.trim().is_empty() {
        issues.push(ValidationIssue::new(
            path,
            ValidationCode::Empty,
            format!("{context} must not be empty"),
        ));
    }
    if value.len() > maximum {
        issues.push(ValidationIssue::new(
            path,
            ValidationCode::TooLong,
            format!("{context} must be at most {maximum} UTF-8 bytes"),
        ));
    }
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
    Plugin,
}

fn validate_composition(
    path: &str,
    provider: ProviderKind,
    template: Option<&DisplayTemplate>,
    tap_action: &WidgetTapAction,
    refresh: RefreshPolicy,
    issues: &mut Vec<ValidationIssue>,
) {
    // `template` is `None` only for `ProviderKind::Plugin`: a plugin card renders
    // from its manifest-compiled scene, not one of the six built-in
    // `DisplayTemplate`s, so there is no template/provider field-compatibility
    // pairing to check and no template-gated tap-action rule to enforce here. The
    // provider-gated tap-action check and the refresh-policy checks below still
    // apply to every provider, plugin included.
    if let Some(template) = template {
        // A pairing is allowed only when the provider actually populates the fields
        // the template declares (`firmware/main/core/template_fields.c`). A template
        // whose renderable fields the provider never sends draws its placeholders
        // forever, and every field the provider sends that the template does not
        // declare is counted in the device's `unknown_field_count` on EVERY refresh —
        // degrading the diagnostic that exists to catch real host/firmware schema
        // drift.
        let template_supported = match provider {
            // Clock sends only title/show_seconds; `big-number-label`'s `value` would
            // never be written and the card would show a permanent "--".
            ProviderKind::Clock => matches!(
                template,
                DisplayTemplate::DigitalClock | DisplayTemplate::AnalogClock
            ),
            // Pomodoro sends `label`, `duration_seconds`, `remaining_seconds` and
            // `running`; `big-number-label` declares only `label` out of those, so its
            // hero `value` stayed "--" forever while the other three counted as
            // unknown on EVERY tick — a continuous drip, worse than the calendar case
            // above. Unlike weather's `row-list` this strands no saved configuration:
            // v0/v1 migration hard-codes pomodoro to `ProgressRing`, and while v2
            // migration copies `template` verbatim, no v2 file could hold
            // `big-number-label` on a pomodoro card because `save_and_apply` compiled
            // before persisting and `wire_config()` refused to lower that template at
            // the time.
            ProviderKind::Pomodoro => matches!(template, DisplayTemplate::ProgressRing),
            // Calendar and RSS send `title` plus ten `rowN_*` fields, which only
            // `row-list` declares. On `icon-badge-text` all ten counted as unknown on
            // every refresh and the card rendered the hollow `unknown` ring and "--".
            ProviderKind::Calendar | ProviderKind::Rss => {
                matches!(template, DisplayTemplate::RowList)
            }
            // `icon-badge-text` is what weather's field set was designed for and
            // `big-number-label` renders its `title`/`value`/`label` subset. `RowList`
            // must stay: `validate()` runs on config LOAD, cards are never migrated to
            // a new template, and every weather card saved before the extended
            // templates shipped is still on `row-list` (it was the only weather-legal
            // template `wire_config()` could lower back then). Removing it would make
            // those saved configurations fail to load.
            ProviderKind::Weather => matches!(
                template,
                DisplayTemplate::BigNumberLabel
                    | DisplayTemplate::IconBadgeText { .. }
                    | DisplayTemplate::RowList
            ),
            // Every json-feed field is user-mapped by name, so the user can populate
            // any template's declared text fields, including `row-list`'s `rowN_*`
            // set.
            ProviderKind::JsonFeed => matches!(
                template,
                DisplayTemplate::BigNumberLabel
                    | DisplayTemplate::IconBadgeText { .. }
                    | DisplayTemplate::RowList
            ),
            // A plugin card never passes `Some(template)` — see the comment above. This
            // arm is unreachable by construction today, but the function validates
            // untrusted config content, so it returns a safe `false` (an
            // `InvalidComposition` issue) rather than panicking if that ever stops
            // being true.
            ProviderKind::Plugin => false,
        };
        if !template_supported {
            issues.push(ValidationIssue::new(
                format!("{path}.template"),
                ValidationCode::InvalidComposition,
                "display template is incompatible with this provider",
            ));
        }

        // `widget_model.c` refuses any widget whose template is not `PROGRESS_RING`
        // while carrying a non-`NONE` tap action, and `validate_config` is
        // all-or-nothing: one such widget makes the device reject the entire
        // `ApplyConfig`, so no card updates at all. Mirror that rule host-side
        // instead of letting a saveable configuration take the whole layout down.
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
    ) && !matches!(provider, ProviderKind::Pomodoro)
    {
        issues.push(ValidationIssue::new(
            format!("{path}.tap_action"),
            ValidationCode::InvalidComposition,
            "start/pause and reset actions require a pomodoro provider",
        ));
    }

    let refresh_supported = match provider {
        ProviderKind::Clock | ProviderKind::Pomodoro => {
            matches!(refresh, RefreshPolicy::DeviceLocal)
        }
        ProviderKind::Calendar
        | ProviderKind::Weather
        | ProviderKind::JsonFeed
        | ProviderKind::Rss
        | ProviderKind::Plugin => matches!(
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
