use std::collections::{HashMap, HashSet};
use std::fmt;

pub use protocol::MAX_WIDGET_ID_LEN;
use protocol::{
    ApplyConfig, Field, FieldValue, InterruptPolicy, MAX_CONFIG_SCREENS, MAX_CONFIG_WIDGETS,
    MAX_SCREEN_ID_LEN, PushData, ScreenConfig, SizeClass, TapAction, TemplateKind, WidgetConfig,
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
            hold: AlertHoldInner,
        },
        BeforeEvent {
            lead_minutes: u16,
            hold: AlertHoldInner,
        },
    }

    #[derive(Debug, Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "kebab-case")]
    pub enum CarouselAdvanceInner {
        Manual,
        Timed { default_dwell_seconds: u16 },
    }

    pub fn validate_and_deserialize<T: for<'de> serde::Deserialize<'de>>(
        value: &Value,
    ) -> Result<T, String> {
        // Validate unknown fields
        if let Value::Object(map) = value {
            let allowed = match map.get("kind") {
                Some(Value::String(s)) => match s.as_str() {
                    "in-rotation" => vec!["kind", "dwell_seconds"],
                    "alert-only" | "off" | "until-dismissed" | "none" | "manual" => {
                        vec!["kind"]
                    }
                    "seconds" => vec!["kind", "value"],
                    "on-timer-finish" => vec!["kind", "hold"],
                    "before-event" => vec!["kind", "lead_minutes", "hold"],
                    "timed" => vec!["kind", "default_dwell_seconds"],
                    _ => return Err(format!("unknown variant: {s}")),
                },
                _ => return Err("missing or invalid 'kind' field".to_string()),
            };

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

pub const CURRENT_SCHEMA_VERSION: u32 = 2;
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
pub const MAX_TILES_PER_SCREEN: usize = 4;
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
pub const MAX_DASHBOARD_COLUMNS: u8 = 2;
pub const MAX_DASHBOARD_ROWS: u8 = 2;
pub const MAX_RSS_ITEMS: u8 = 5;
pub const MIN_AUTO_ADVANCE_SECONDS: u16 = 5;
pub const MAX_AUTO_ADVANCE_SECONDS: u16 = 3_600;
pub const MIN_POMODORO_SECONDS: u32 = 1;
pub const MAX_POMODORO_SECONDS: u32 = 86_400;
pub const MIN_CALENDAR_REFRESH_MINUTES: u16 = 1;
pub const MIN_WEATHER_REFRESH_MINUTES: u16 = 10;
pub const MAX_CALENDAR_REFRESH_MINUTES: u16 = 1_440;
pub const MAX_CONFIG_CARDS: usize = 8;
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
    pub widgets: Vec<WidgetSettings>,
    pub screens: Vec<ScreenSettings>,
    pub assets: Vec<AssetSettings>,
    pub carousel: CarouselSettings,
    pub updater: UpdaterSettings,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            preferences: AppPreferences::default(),
            widgets: vec![WidgetSettings::Clock {
                id: "clock".into(),
                size: WidgetSize::Full,
                title: "Desk".into(),
                show_seconds: true,
                template: DisplayTemplate::DigitalClock,
                tap_action: WidgetTapAction::None,
                refresh: RefreshPolicy::DeviceLocal,
                interrupt_policy: WidgetInterruptPolicy::Disabled,
            }],
            screens: vec![ScreenSettings {
                id: "clock-screen".into(),
                layout: ScreenLayout::Single {
                    widget_id: "clock".into(),
                },
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
        validate_collection_bounds(
            "widgets",
            self.widgets.len(),
            MAX_CONFIG_WIDGETS,
            &mut issues,
        );
        validate_collection_bounds(
            "screens",
            self.screens.len(),
            MAX_CONFIG_SCREENS,
            &mut issues,
        );

        let mut widget_ids = HashSet::with_capacity(self.widgets.len());
        for (index, widget) in self.widgets.iter().enumerate() {
            let path = format!("widgets[{index}]");
            validate_identifier(
                &format!("{path}.id"),
                widget.id(),
                MAX_WIDGET_ID_LEN,
                &mut issues,
            );
            if !widget_ids.insert(widget.id()) {
                issues.push(ValidationIssue::new(
                    format!("{path}.id"),
                    ValidationCode::DuplicateId,
                    format!("widget ID {:?} is duplicated", widget.id()),
                ));
            }
            widget.validate(&path, &mut issues);
        }

        let mut screen_ids = HashSet::with_capacity(self.screens.len());
        let mut references: HashMap<&str, usize> = HashMap::with_capacity(self.widgets.len());
        let widget_sizes: HashMap<&str, WidgetSize> = self
            .widgets
            .iter()
            .map(|widget| (widget.id(), widget.size()))
            .collect();
        for (index, screen) in self.screens.iter().enumerate() {
            let path = format!("screens[{index}]");
            validate_identifier(
                &format!("{path}.id"),
                &screen.id,
                MAX_SCREEN_ID_LEN,
                &mut issues,
            );
            if !screen_ids.insert(screen.id.as_str()) {
                issues.push(ValidationIssue::new(
                    format!("{path}.id"),
                    ValidationCode::DuplicateId,
                    format!("screen ID {:?} is duplicated", screen.id),
                ));
            }
            match &screen.layout {
                ScreenLayout::Single { widget_id } => {
                    validate_widget_reference(
                        &format!("{path}.layout.widget_id"),
                        widget_id,
                        &widget_sizes,
                        WidgetSize::Tile,
                        false,
                        &mut references,
                        &mut issues,
                    );
                }
                ScreenLayout::Dashboard {
                    columns,
                    rows,
                    tiles,
                } => {
                    if !(1..=MAX_DASHBOARD_COLUMNS).contains(columns)
                        || !(1..=MAX_DASHBOARD_ROWS).contains(rows)
                    {
                        issues.push(ValidationIssue::new(
                            format!("{path}.layout"),
                            ValidationCode::OutOfRange,
                            format!(
                                "dashboard grid must be within {MAX_DASHBOARD_COLUMNS} columns and {MAX_DASHBOARD_ROWS} rows"
                            ),
                        ));
                    }
                    if !(2..=MAX_TILES_PER_SCREEN).contains(&tiles.len()) {
                        issues.push(ValidationIssue::new(
                            format!("{path}.layout.tiles"),
                            ValidationCode::OutOfRange,
                            format!("dashboard must contain 2..={MAX_TILES_PER_SCREEN} tiles"),
                        ));
                    }
                    let mut occupied = HashSet::new();
                    for (tile_index, tile) in tiles.iter().enumerate() {
                        let tile_path = format!("{path}.layout.tiles[{tile_index}]");
                        validate_widget_reference(
                            &format!("{tile_path}.widget_id"),
                            &tile.widget_id,
                            &widget_sizes,
                            WidgetSize::Tile,
                            true,
                            &mut references,
                            &mut issues,
                        );
                        let valid_geometry = tile.column_span > 0
                            && tile.row_span > 0
                            && tile.column < *columns
                            && tile.row < *rows
                            && tile.column.saturating_add(tile.column_span) <= *columns
                            && tile.row.saturating_add(tile.row_span) <= *rows;
                        if !valid_geometry {
                            issues.push(ValidationIssue::new(
                                tile_path.clone(),
                                ValidationCode::OutOfRange,
                                "tile must fit entirely inside its dashboard grid",
                            ));
                            continue;
                        }
                        for column in tile.column..tile.column + tile.column_span {
                            for row in tile.row..tile.row + tile.row_span {
                                if !occupied.insert((column, row)) {
                                    issues.push(ValidationIssue::new(
                                        tile_path.clone(),
                                        ValidationCode::Overlap,
                                        "dashboard tiles must not overlap",
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }

        for (index, widget) in self.widgets.iter().enumerate() {
            match references.get(widget.id()).copied().unwrap_or_default() {
                0 => issues.push(ValidationIssue::new(
                    format!("widgets[{index}].id"),
                    ValidationCode::MissingScreen,
                    format!("widget {:?} is not assigned to a screen", widget.id()),
                )),
                1 => {}
                _ => issues.push(ValidationIssue::new(
                    format!("widgets[{index}].id"),
                    ValidationCode::DuplicateReference,
                    format!(
                        "widget {:?} is assigned to more than one screen",
                        widget.id()
                    ),
                )),
            }
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
        for (index, widget) in self.widgets.iter().enumerate() {
            if let DisplayTemplate::IconBadgeText {
                icon_asset_id: Some(asset_id),
            } = widget.template()
            {
                match assets_by_id.get(asset_id.as_str()) {
                    None => issues.push(ValidationIssue::new(
                        format!("widgets[{index}].template.icon_asset_id"),
                        ValidationCode::MissingReference,
                        format!("asset {asset_id:?} does not exist"),
                    )),
                    Some(kind) if !matches!(kind, AssetKind::Icon { .. }) => {
                        issues.push(ValidationIssue::new(
                            format!("widgets[{index}].template.icon_asset_id"),
                            ValidationCode::InvalidComposition,
                            "icon template must reference an icon asset",
                        ));
                    }
                    Some(_) => {}
                }
            }
        }
        if self.carousel.auto_advance_seconds.is_some_and(|seconds| {
            !(MIN_AUTO_ADVANCE_SECONDS..=MAX_AUTO_ADVANCE_SECONDS).contains(&seconds)
        }) {
            issues.push(ValidationIssue::new(
                "carousel.auto_advance_seconds",
                ValidationCode::OutOfRange,
                format!(
                    "auto advance must be {MIN_AUTO_ADVANCE_SECONDS}..={MAX_AUTO_ADVANCE_SECONDS} seconds"
                ),
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
        let widgets = self
            .widgets
            .iter()
            .enumerate()
            .filter_map(|(index, widget)| {
                widget.wire_config().or_else(|| {
                    compatibility_issues.push(ValidationIssue::new(
                        format!("widgets[{index}]"),
                        ValidationCode::RequiresCapability,
                        "widget provider/template/action is not implemented by this build",
                    ));
                    None
                })
            })
            .collect();
        let screens = self
            .screens
            .iter()
            .enumerate()
            .filter_map(|(index, screen)| match &screen.layout {
                ScreenLayout::Single { widget_id } => Some(ScreenConfig {
                    screen_id: screen.id.clone(),
                    widget_id: widget_id.clone(),
                }),
                ScreenLayout::Dashboard { .. } => {
                    compatibility_issues.push(ValidationIssue::new(
                        format!("screens[{index}].layout"),
                        ValidationCode::RequiresCapability,
                        "dashboard layout requires protocol v2 firmware",
                    ));
                    None
                }
            })
            .collect();
        if !self.assets.is_empty() {
            compatibility_issues.push(ValidationIssue::new(
                "assets",
                ValidationCode::RequiresCapability,
                "asset transfer is not implemented by this build",
            ));
        }
        if self.carousel.auto_advance_seconds.is_some() {
            compatibility_issues.push(ValidationIssue::new(
                "carousel.auto_advance_seconds",
                ValidationCode::RequiresCapability,
                "timed carousel advance is not implemented by this build",
            ));
        }
        if !compatibility_issues.is_empty() {
            return Err(ConfigValidationError {
                issues: compatibility_issues,
            });
        }
        let initial_pushes = self
            .widgets
            .iter()
            .map(|widget| PushData {
                widget_id: widget.id().into(),
                revision,
                fields: widget.initial_fields(),
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
        if self
            .screens
            .iter()
            .any(|screen| matches!(&screen.layout, ScreenLayout::Dashboard { .. }))
        {
            required |= protocol::CAPABILITY_DASHBOARD_LAYOUTS;
        }
        if self.widgets.iter().any(|widget| {
            !matches!(
                widget.template(),
                DisplayTemplate::DigitalClock
                    | DisplayTemplate::ProgressRing
                    | DisplayTemplate::RowList
            )
        }) {
            required |= protocol::CAPABILITY_EXTENDED_TEMPLATES;
        }
        if self.widgets.iter().any(|widget| {
            matches!(
                widget.tap_action(),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WidgetSize {
    Full,
    Standard,
    Tile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum DisplayTemplate {
    DigitalClock,
    AnalogClock,
    ProgressRing,
    RowList,
    BigNumberLabel,
    IconBadgeText { icon_asset_id: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum WidgetTapAction {
    None,
    StartPause,
    Reset,
    Dismiss,
    OpenUrl { url: String },
    OpenApplication { application_id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum RefreshPolicy {
    DeviceLocal,
    Manual,
    Interval { minutes: u16 },
}

impl RefreshPolicy {
    pub const fn interval_minutes(self) -> Option<u16> {
        match self {
            Self::Interval { minutes } => Some(minutes),
            Self::DeviceLocal | Self::Manual => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WidgetInterruptPolicy {
    Disabled,
    Enabled,
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
            strict_tagged_enum::CardAlertInner::OnTimerFinish { hold: inner_hold } => {
                let hold = match inner_hold {
                    strict_tagged_enum::AlertHoldInner::UntilDismissed => AlertHold::UntilDismissed,
                    strict_tagged_enum::AlertHoldInner::Seconds { value } => {
                        AlertHold::Seconds { value }
                    }
                };
                CardAlert::OnTimerFinish { hold }
            }
            strict_tagged_enum::CardAlertInner::BeforeEvent {
                lead_minutes,
                hold: inner_hold,
            } => {
                let hold = match inner_hold {
                    strict_tagged_enum::AlertHoldInner::UntilDismissed => AlertHold::UntilDismissed,
                    strict_tagged_enum::AlertHoldInner::Seconds { value } => {
                        AlertHold::Seconds { value }
                    }
                };
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum WidgetSettings {
    Clock {
        id: String,
        size: WidgetSize,
        title: String,
        show_seconds: bool,
        template: DisplayTemplate,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
        interrupt_policy: WidgetInterruptPolicy,
    },
    Pomodoro {
        id: String,
        size: WidgetSize,
        label: String,
        duration_seconds: u32,
        template: DisplayTemplate,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
        interrupt_policy: WidgetInterruptPolicy,
    },
    Calendar {
        id: String,
        size: WidgetSize,
        title: String,
        source: CalendarSource,
        template: DisplayTemplate,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
        interrupt_policy: WidgetInterruptPolicy,
    },
    Weather {
        id: String,
        size: WidgetSize,
        title: String,
        location: String,
        units: WeatherUnits,
        template: DisplayTemplate,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
        interrupt_policy: WidgetInterruptPolicy,
    },
    JsonFeed {
        id: String,
        size: WidgetSize,
        title: String,
        url: String,
        mappings: Vec<JsonFieldMapping>,
        template: DisplayTemplate,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
        interrupt_policy: WidgetInterruptPolicy,
    },
    Rss {
        id: String,
        size: WidgetSize,
        title: String,
        url: String,
        max_items: u8,
        template: DisplayTemplate,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
        interrupt_policy: WidgetInterruptPolicy,
    },
}

impl WidgetSettings {
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

    pub const fn size(&self) -> WidgetSize {
        match self {
            Self::Clock { size, .. }
            | Self::Pomodoro { size, .. }
            | Self::Calendar { size, .. }
            | Self::Weather { size, .. }
            | Self::JsonFeed { size, .. }
            | Self::Rss { size, .. } => *size,
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

    pub const fn interrupt_policy(&self) -> WidgetInterruptPolicy {
        match self {
            Self::Clock {
                interrupt_policy, ..
            }
            | Self::Pomodoro {
                interrupt_policy, ..
            }
            | Self::Calendar {
                interrupt_policy, ..
            }
            | Self::Weather {
                interrupt_policy, ..
            }
            | Self::JsonFeed {
                interrupt_policy, ..
            }
            | Self::Rss {
                interrupt_policy, ..
            } => *interrupt_policy,
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
            DisplayTemplate::AnalogClock
            | DisplayTemplate::BigNumberLabel
            | DisplayTemplate::IconBadgeText { .. } => return None,
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
            size_class: self.size().to_wire(),
            tap_action,
            interrupt_policy: match self.interrupt_policy() {
                WidgetInterruptPolicy::Disabled => InterruptPolicy::Disabled,
                WidgetInterruptPolicy::Enabled => InterruptPolicy::Enabled,
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

impl WidgetSize {
    const fn to_wire(self) -> SizeClass {
        match self {
            Self::Full => SizeClass::Full,
            Self::Standard => SizeClass::Standard,
            Self::Tile => SizeClass::Tile,
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
pub struct ScreenSettings {
    pub id: String,
    pub layout: ScreenLayout,
}

impl ScreenSettings {
    pub fn widget_ids(&self) -> impl Iterator<Item = &str> {
        let mut ids = [None, None, None, None];
        match &self.layout {
            ScreenLayout::Single { widget_id } => ids[0] = Some(widget_id.as_str()),
            ScreenLayout::Dashboard { tiles, .. } => {
                for (slot, tile) in ids.iter_mut().zip(tiles) {
                    *slot = Some(tile.widget_id.as_str());
                }
            }
        }
        ids.into_iter().flatten()
    }

    pub fn primary_widget_id(&self) -> Option<&str> {
        self.widget_ids().next()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ScreenLayout {
    Single {
        widget_id: String,
    },
    Dashboard {
        columns: u8,
        rows: u8,
        tiles: Vec<TileSettings>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TileSettings {
    pub widget_id: String,
    pub column: u8,
    pub row: u8,
    pub column_span: u8,
    pub row_span: u8,
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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CarouselSettings {
    pub auto_advance_seconds: Option<u16>,
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
    let template_supported = match provider {
        ProviderKind::Clock => matches!(
            template,
            DisplayTemplate::DigitalClock
                | DisplayTemplate::AnalogClock
                | DisplayTemplate::BigNumberLabel
        ),
        ProviderKind::Pomodoro => matches!(
            template,
            DisplayTemplate::ProgressRing | DisplayTemplate::BigNumberLabel
        ),
        ProviderKind::Calendar | ProviderKind::Rss => matches!(
            template,
            DisplayTemplate::RowList | DisplayTemplate::IconBadgeText { .. }
        ),
        ProviderKind::Weather => matches!(
            template,
            DisplayTemplate::BigNumberLabel | DisplayTemplate::IconBadgeText { .. }
        ),
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

fn validate_widget_reference<'a>(
    path: &str,
    widget_id: &'a str,
    widget_sizes: &HashMap<&'a str, WidgetSize>,
    compared_size: WidgetSize,
    require_equal: bool,
    references: &mut HashMap<&'a str, usize>,
    issues: &mut Vec<ValidationIssue>,
) {
    validate_identifier(path, widget_id, MAX_WIDGET_ID_LEN, issues);
    let Some(size) = widget_sizes.get(widget_id) else {
        issues.push(ValidationIssue::new(
            path,
            ValidationCode::MissingReference,
            format!("widget {widget_id:?} does not exist"),
        ));
        return;
    };
    *references.entry(widget_id).or_default() += 1;
    if (*size == compared_size) != require_equal {
        issues.push(ValidationIssue::new(
            path,
            ValidationCode::UnsupportedSize,
            if require_equal {
                "dashboard widgets must use tile size"
            } else {
                "single-widget screens cannot use tile size"
            },
        ));
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
