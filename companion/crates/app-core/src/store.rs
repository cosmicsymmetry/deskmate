use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::config::{DEFAULT_PLAYLIST_ID, DEFAULT_PLAYLIST_NAME};
use crate::secure_file::{self, BoundedReadError, FileIoError};
use crate::{
    AlertHold, AppConfig, AppPreferences, AssetSettings, CURRENT_SCHEMA_VERSION, CardAlert,
    CardSettings, CarouselAdvance, DisplayTemplate, Playlist, PlaylistEntry, RefreshPolicy,
    UpdaterSettings, ValidationIssue, WidgetTapAction,
};

pub const MAX_CONFIG_FILE_BYTES: usize = 64 * 1_024;
pub const SAVED_SETTINGS_VALIDATION_FAILURE_MESSAGE: &str =
    "Your saved settings failed validation and were not applied";

pub struct ConfigStore {
    path: PathBuf,
    state: Mutex<StoreState>,
}

struct StoreState {
    last_good: Option<AppConfig>,
    generation: u64,
}

impl ConfigStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            state: Mutex::new(StoreState {
                last_good: None,
                generation: 0,
            }),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> LoadOutcome {
        let Ok(mut state) = self.state.lock() else {
            return LoadOutcome::Recovered {
                config: AppConfig::default(),
                origin: ConfigOrigin::Defaults,
                error: StoreError::LockPoisoned,
            };
        };

        let bytes = match read_bounded(&self.path) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => {
                let config = AppConfig::default();
                return LoadOutcome::Loaded {
                    config,
                    origin: ConfigOrigin::Defaults,
                };
            }
            Err(error) => return recovered(&state, error),
        };

        match decode_config(&bytes) {
            Ok((config, origin)) => {
                state.last_good = Some(config.clone());
                LoadOutcome::Loaded { config, origin }
            }
            Err(StoreError::Validation { issues }) => validation_failed(&state, issues),
            Err(error) => recovered(&state, error),
        }
    }

    pub fn save(&self, config: &AppConfig) -> Result<SaveReceipt, StoreError> {
        let mut state = self.state.lock().map_err(|_| StoreError::LockPoisoned)?;
        config.validate().map_err(|error| StoreError::Validation {
            issues: error.issues,
        })?;
        let bytes = serde_json::to_vec_pretty(config).map_err(|error| StoreError::InvalidJson {
            message: error.to_string(),
        })?;
        if bytes.len() > MAX_CONFIG_FILE_BYTES {
            return Err(StoreError::TooLarge {
                maximum: MAX_CONFIG_FILE_BYTES,
            });
        }

        let parent =
            secure_file::usable_parent(&self.path).ok_or_else(|| StoreError::InvalidPath {
                message: format!("{} has no parent directory", self.path.display()),
            })?;
        secure_file::create_directory(parent)
            .map_err(|error| secure_io_error("config directory", error))?;
        secure_file::write_and_replace(&self.path, &bytes)
            .map_err(|error| secure_io_error("config", error))?;

        state.generation = state.generation.saturating_add(1);
        state.last_good = Some(config.clone());
        let warning = secure_file::sync_parent(parent).err().map(|error| {
            let (operation, message) = error.into_strings("config directory");
            StoreWarning::Io { operation, message }
        });
        Ok(SaveReceipt {
            generation: state.generation,
            warning,
        })
    }

    pub fn last_good(&self) -> Result<AppConfig, StoreError> {
        self.state
            .lock()
            .map(|state| state.last_good.clone().unwrap_or_default())
            .map_err(|_| StoreError::LockPoisoned)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConfigOrigin {
    Defaults,
    Current,
    MigratedV0,
    MigratedV1,
    MigratedV2,
    MigratedV3,
    MigratedV4,
    MigratedV5,
    MigratedV6,
    MigratedV7,
    MigratedV8,
    LastGood,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum LoadOutcome {
    Loaded {
        config: AppConfig,
        origin: ConfigOrigin,
    },
    Recovered {
        config: AppConfig,
        origin: ConfigOrigin,
        error: StoreError,
    },
    ValidationFailed {
        config: AppConfig,
        origin: ConfigOrigin,
        issues: Vec<ValidationIssue>,
    },
}

impl LoadOutcome {
    pub fn config(&self) -> &AppConfig {
        match self {
            Self::Loaded { config, .. }
            | Self::Recovered { config, .. }
            | Self::ValidationFailed { config, .. } => config,
        }
    }

    pub const fn origin(&self) -> ConfigOrigin {
        match self {
            Self::Loaded { origin, .. }
            | Self::Recovered { origin, .. }
            | Self::ValidationFailed { origin, .. } => *origin,
        }
    }

    pub fn recovery(&self) -> Option<StoreError> {
        match self {
            Self::Loaded { .. } => None,
            Self::Recovered { error, .. } => Some(error.clone()),
            Self::ValidationFailed { issues, .. } => Some(StoreError::Validation {
                issues: issues.clone(),
            }),
        }
    }

    pub fn into_config(self) -> AppConfig {
        match self {
            Self::Loaded { config, .. }
            | Self::Recovered { config, .. }
            | Self::ValidationFailed { config, .. } => config,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveReceipt {
    pub generation: u64,
    pub warning: Option<StoreWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum StoreError {
    Io { operation: String, message: String },
    InvalidPath { message: String },
    TooLarge { maximum: usize },
    InvalidUtf8,
    InvalidJson { message: String },
    UnsupportedVersion { found: u32, supported: u32 },
    Validation { issues: Vec<ValidationIssue> },
    LockPoisoned,
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { operation, message } => write!(formatter, "{operation}: {message}"),
            Self::InvalidPath { message } => write!(formatter, "invalid config path: {message}"),
            Self::TooLarge { maximum } => {
                write!(formatter, "config exceeds the {maximum}-byte limit")
            }
            Self::InvalidUtf8 => formatter.write_str("config is not valid UTF-8"),
            Self::InvalidJson { message } => write!(formatter, "invalid config JSON: {message}"),
            Self::UnsupportedVersion { found, supported } => write!(
                formatter,
                "config schema version {found} is unsupported; expected {supported}"
            ),
            Self::Validation { .. } => {
                formatter.write_str(SAVED_SETTINGS_VALIDATION_FAILURE_MESSAGE)
            }
            Self::LockPoisoned => formatter.write_str("config store lock is poisoned"),
        }
    }
}

impl std::error::Error for StoreError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum StoreWarning {
    Io { operation: String, message: String },
}

#[derive(Deserialize)]
struct VersionHeader {
    schema_version: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyConfigV0 {
    #[serde(rename = "schema_version")]
    _schema_version: u32,
    timezone: String,
    widgets: Vec<LegacyWidgetSettings>,
    screens: Vec<LegacyScreenSettings>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyConfigV1 {
    #[serde(rename = "schema_version")]
    _schema_version: u32,
    preferences: AppPreferences,
    widgets: Vec<LegacyWidgetSettings>,
    screens: Vec<LegacyScreenSettings>,
}

/// Legacy (schema v0/v1) widget size class. The card model has no size
/// concept; this is parsed only so the legacy JSON round-trips and then
/// discarded during migration.
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum LegacyWidgetSize {
    Full,
    Standard,
    Tile,
}

// Retired payload fields remain intentionally unread: serde must consume the old
// shape before migration can decide that the whole card is no longer supported.
#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "kebab-case",
    deny_unknown_fields
)]
enum LegacyCalendarSource {
    File(String),
    Url(String),
}

#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum LegacyWeatherUnits {
    Metric,
    Imperial,
}

#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyJsonFieldMapping {
    field: String,
    path: String,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum LegacyCardAlert {
    None,
    OnTimerFinish {
        hold: AlertHold,
    },
    BeforeEvent {
        #[allow(dead_code)]
        lead_minutes: u16,
        #[allow(dead_code)]
        hold: AlertHold,
    },
}

#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum LegacyWidgetSettings {
    Clock {
        id: String,
        #[allow(dead_code)]
        size: LegacyWidgetSize,
        title: String,
        show_seconds: bool,
    },
    Pomodoro {
        id: String,
        #[allow(dead_code)]
        size: LegacyWidgetSize,
        label: String,
        duration_seconds: u32,
    },
    Calendar {
        id: String,
        #[allow(dead_code)]
        size: LegacyWidgetSize,
        title: String,
        source: LegacyCalendarSource,
        refresh_minutes: u16,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyScreenSettings {
    // The card model folds screen and widget identity into a single id, so
    // a legacy screen's own id is intentionally discarded on migration; only
    // its order and widget reference survive.
    #[allow(dead_code)]
    id: String,
    widget_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyConfigV2 {
    #[serde(rename = "schema_version")]
    _schema_version: u32,
    preferences: AppPreferences,
    widgets: Vec<LegacyWidgetV2>,
    screens: Vec<LegacyScreenV2>,
    #[serde(default)]
    assets: Vec<AssetSettings>,
    #[serde(default)]
    carousel: LegacyCarouselV2,
    #[serde(default)]
    updater: UpdaterSettings,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyConfigV3 {
    #[serde(rename = "schema_version")]
    _schema_version: u32,
    preferences: AppPreferences,
    cards: Vec<LegacyCardV3>,
    assets: Vec<AssetSettings>,
    carousel: LegacyCarouselV3,
    updater: UpdaterSettings,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyCarouselV3 {
    advance: CarouselAdvance,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum LegacyCardPresenceV3 {
    InRotation { dwell_seconds: Option<u16> },
    AlertOnly,
    Off,
}

/// Mirrors the v3 `CardSettings` shape exactly. `presence` is legacy-only and
/// is translated into playlist membership; all other fields carry directly
/// into the v4 card library.
#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum LegacyCardV3 {
    Clock {
        id: String,
        title: String,
        show_seconds: bool,
        template: DisplayTemplate,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
        presence: LegacyCardPresenceV3,
        alert: LegacyCardAlert,
    },
    Pomodoro {
        id: String,
        label: String,
        duration_seconds: u32,
        template: DisplayTemplate,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
        presence: LegacyCardPresenceV3,
        alert: LegacyCardAlert,
    },
    Calendar {
        id: String,
        title: String,
        source: LegacyCalendarSource,
        template: DisplayTemplate,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
        presence: LegacyCardPresenceV3,
        alert: LegacyCardAlert,
    },
    Weather {
        id: String,
        title: String,
        location: String,
        units: LegacyWeatherUnits,
        template: DisplayTemplate,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
        presence: LegacyCardPresenceV3,
        alert: LegacyCardAlert,
    },
    JsonFeed {
        id: String,
        title: String,
        url: String,
        mappings: Vec<LegacyJsonFieldMapping>,
        template: DisplayTemplate,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
        presence: LegacyCardPresenceV3,
        alert: LegacyCardAlert,
    },
    Rss {
        id: String,
        title: String,
        url: String,
        max_items: u8,
        template: DisplayTemplate,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
        presence: LegacyCardPresenceV3,
        alert: LegacyCardAlert,
    },
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyCarouselV2 {
    #[serde(default)]
    auto_advance_seconds: Option<u16>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyScreenV2 {
    // Mirrors `LegacyScreenSettings`: the screen's own id is dropped on
    // migration, only its order and widget reference survive.
    #[allow(dead_code)]
    id: String,
    layout: LegacyLayoutV2,
}

/// Deliberately `Single`-only. A dashboard layout could never appear in a
/// persisted v2 file: v2's `compile()` rejected `ScreenLayout::Dashboard` with
/// `requires-capability`, and `ConfigStore::save` validates/compiles before
/// writing to disk, so no dashboard-bearing document was ever written. A
/// dashboard document therefore fails to deserialize here (an unknown `kind`
/// variant), surfacing as a recoverable `StoreError::InvalidJson` rather than
/// silently inventing a lossy dashboard-to-card mapping that never needs to
/// exist.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum LegacyLayoutV2 {
    Single { widget_id: String },
}

/// Legacy (schema v2) interrupt policy. The card model replaces this with the
/// richer `CardAlert`; this is parsed only so the legacy JSON round-trips and
/// then discarded (mapped through `legacy_alert`) during migration.
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum LegacyInterruptPolicy {
    Disabled,
    Enabled,
}

/// Mirrors the v2 `WidgetSettings` shape exactly (six kinds, each carrying
/// `size`, its source fields, `template`, `tap_action`, `refresh`, and
/// `interrupt_policy`). `size` and `interrupt_policy` are legacy-only
/// concepts and are parsed here only to be dropped/translated on migration.
#[allow(dead_code)]
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum LegacyWidgetV2 {
    Clock {
        id: String,
        #[allow(dead_code)]
        size: LegacyWidgetSize,
        title: String,
        show_seconds: bool,
        template: DisplayTemplate,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
        interrupt_policy: LegacyInterruptPolicy,
    },
    Pomodoro {
        id: String,
        #[allow(dead_code)]
        size: LegacyWidgetSize,
        label: String,
        duration_seconds: u32,
        template: DisplayTemplate,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
        interrupt_policy: LegacyInterruptPolicy,
    },
    Calendar {
        id: String,
        #[allow(dead_code)]
        size: LegacyWidgetSize,
        title: String,
        source: LegacyCalendarSource,
        template: DisplayTemplate,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
        interrupt_policy: LegacyInterruptPolicy,
    },
    Weather {
        id: String,
        #[allow(dead_code)]
        size: LegacyWidgetSize,
        title: String,
        location: String,
        units: LegacyWeatherUnits,
        template: DisplayTemplate,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
        interrupt_policy: LegacyInterruptPolicy,
    },
    JsonFeed {
        id: String,
        #[allow(dead_code)]
        size: LegacyWidgetSize,
        title: String,
        url: String,
        mappings: Vec<LegacyJsonFieldMapping>,
        template: DisplayTemplate,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
        interrupt_policy: LegacyInterruptPolicy,
    },
    Rss {
        id: String,
        #[allow(dead_code)]
        size: LegacyWidgetSize,
        title: String,
        url: String,
        max_items: u8,
        template: DisplayTemplate,
        tap_action: WidgetTapAction,
        refresh: RefreshPolicy,
        interrupt_policy: LegacyInterruptPolicy,
    },
}

impl LegacyWidgetV2 {
    fn id(&self) -> &str {
        match self {
            Self::Clock { id, .. }
            | Self::Pomodoro { id, .. }
            | Self::Calendar { id, .. }
            | Self::Weather { id, .. }
            | Self::JsonFeed { id, .. }
            | Self::Rss { id, .. } => id,
        }
    }

    fn interrupts_enabled(&self) -> bool {
        let policy = match self {
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
            } => interrupt_policy,
        };
        matches!(policy, LegacyInterruptPolicy::Enabled)
    }
}

#[allow(clippy::too_many_lines)]
fn card_from_legacy_v2(widget: LegacyWidgetV2) -> Option<CardSettings> {
    let enabled = widget.interrupts_enabled();
    match widget {
        LegacyWidgetV2::Clock {
            id,
            size: _,
            title,
            show_seconds,
            template,
            tap_action,
            refresh,
            interrupt_policy: _,
        } => Some(CardSettings::Clock {
            id,
            title,
            show_seconds,
            template,
            tap_action,
            refresh,
            alert: legacy_alert(enabled, LegacyCardKind::Clock),
        }),
        LegacyWidgetV2::Pomodoro {
            id,
            size: _,
            label,
            duration_seconds,
            template,
            tap_action,
            refresh,
            interrupt_policy: _,
        } => Some(CardSettings::Pomodoro {
            id,
            label,
            duration_seconds,
            template,
            tap_action,
            refresh,
            alert: legacy_alert(enabled, LegacyCardKind::Pomodoro),
        }),
        LegacyWidgetV2::Calendar { .. }
        | LegacyWidgetV2::Weather { .. }
        | LegacyWidgetV2::JsonFeed { .. }
        | LegacyWidgetV2::Rss { .. } => None,
    }
}

fn migrate_v2(legacy: LegacyConfigV2) -> AppConfig {
    let mut by_id: HashMap<String, LegacyWidgetV2> = legacy
        .widgets
        .into_iter()
        .map(|widget| (widget.id().to_owned(), widget))
        .collect();

    // Card order follows screens[]: that is the carousel order the user authored.
    let mut cards = Vec::with_capacity(by_id.len());
    let mut entries = Vec::with_capacity(legacy.screens.len());
    for screen in legacy.screens {
        let LegacyLayoutV2::Single { widget_id } = screen.layout;
        if let Some(widget) = by_id.remove(&widget_id)
            && let Some(card) = card_from_legacy_v2(widget)
        {
            entries.push(PlaylistEntry {
                card_id: card.id().to_owned(),
                dwell_seconds: None,
            });
            cards.push(card);
        }
    }

    // Widgets with no referencing screen: v2 validation should prevent these, but the
    // protocol permits them. Preserve them (design spec §5) rather than silently
    // dropping configuration: a widget whose interrupts were enabled becomes
    // alert-only so its alert can still fire; otherwise it becomes off. Sort by ID so
    // migration is deterministic regardless of hash-map iteration order.
    let mut orphans: Vec<LegacyWidgetV2> = by_id.into_values().collect();
    orphans.sort_by(|left, right| left.id().cmp(right.id()));
    for widget in orphans {
        if let Some(card) = card_from_legacy_v2(widget) {
            cards.push(card);
        }
    }

    let advance = match legacy.carousel.auto_advance_seconds {
        Some(seconds) => CarouselAdvance::Timed {
            default_dwell_seconds: seconds,
        },
        None => CarouselAdvance::Manual,
    };
    let playlist = synthesize_playlist(&cards, advance, entries);
    AppConfig {
        schema_version: CURRENT_SCHEMA_VERSION,
        preferences: legacy.preferences,
        cards,
        image_sources: Vec::new(),
        assets: legacy.assets,
        playlists: vec![playlist],
        active_playlist_id: DEFAULT_PLAYLIST_ID.into(),
        updater: legacy.updater,
    }
}

#[allow(clippy::too_many_lines)]
fn card_from_legacy_v3(legacy: LegacyCardV3) -> Option<(CardSettings, LegacyCardPresenceV3)> {
    match legacy {
        LegacyCardV3::Clock {
            id,
            title,
            show_seconds,
            template,
            tap_action,
            refresh,
            presence,
            alert,
        } => Some((
            CardSettings::Clock {
                id,
                title,
                show_seconds,
                template,
                tap_action,
                refresh,
                alert: migrate_v3_alert(presence, alert),
            },
            presence,
        )),
        LegacyCardV3::Pomodoro {
            id,
            label,
            duration_seconds,
            template,
            tap_action,
            refresh,
            presence,
            alert,
        } => Some((
            CardSettings::Pomodoro {
                id,
                label,
                duration_seconds,
                template,
                tap_action,
                refresh,
                alert: migrate_v3_alert(presence, alert),
            },
            presence,
        )),
        LegacyCardV3::Calendar { .. }
        | LegacyCardV3::Weather { .. }
        | LegacyCardV3::JsonFeed { .. }
        | LegacyCardV3::Rss { .. } => None,
    }
}

fn migrate_v3_alert(presence: LegacyCardPresenceV3, alert: LegacyCardAlert) -> CardAlert {
    if matches!(presence, LegacyCardPresenceV3::Off) {
        CardAlert::None
    } else {
        match alert {
            LegacyCardAlert::None | LegacyCardAlert::BeforeEvent { .. } => CardAlert::None,
            LegacyCardAlert::OnTimerFinish { hold } => CardAlert::OnTimerFinish { hold },
        }
    }
}

fn migrate_v3(legacy: LegacyConfigV3) -> AppConfig {
    let mut cards = Vec::with_capacity(legacy.cards.len());
    let mut entries = Vec::with_capacity(legacy.cards.len());
    for legacy_card in legacy.cards {
        let Some((card, presence)) = card_from_legacy_v3(legacy_card) else {
            continue;
        };
        if let LegacyCardPresenceV3::InRotation { dwell_seconds } = presence {
            entries.push(PlaylistEntry {
                card_id: card.id().to_owned(),
                dwell_seconds,
            });
        }
        cards.push(card);
    }

    AppConfig {
        schema_version: CURRENT_SCHEMA_VERSION,
        preferences: legacy.preferences,
        playlists: vec![synthesize_playlist(
            &cards,
            legacy.carousel.advance,
            entries,
        )],
        cards,
        image_sources: Vec::new(),
        assets: legacy.assets,
        active_playlist_id: DEFAULT_PLAYLIST_ID.into(),
        updater: legacy.updater,
    }
}

fn synthesize_playlist(
    cards: &[CardSettings],
    advance: CarouselAdvance,
    mut entries: Vec<PlaylistEntry>,
) -> Playlist {
    // With no InRotation cards, preserve authored array order rather than sorting by card ID.
    if entries.is_empty()
        && let Some(card) = cards.first()
    {
        entries.push(PlaylistEntry {
            card_id: card.id().to_owned(),
            dwell_seconds: None,
        });
    }
    Playlist {
        id: DEFAULT_PLAYLIST_ID.into(),
        name: DEFAULT_PLAYLIST_NAME.into(),
        advance,
        entries,
    }
}

fn finish_migration(mut config: AppConfig) -> AppConfig {
    config.playlists.retain(|playlist| {
        playlist.id == config.active_playlist_id || !playlist.entries.is_empty()
    });
    let Some(active_index) = config
        .playlists
        .iter()
        .position(|playlist| playlist.id == config.active_playlist_id)
    else {
        return config;
    };
    if config.playlists[active_index].entries.is_empty() {
        // Retiring a card kind is the first subtractive migration in this
        // product. An empty active playlist is not merely sparse: it makes the
        // document invalid and leaves the panel with no face. Reuse an existing
        // `clock` identity when one survived elsewhere in the library; otherwise
        // add the exact fallback card `AppConfig::default()` ships.
        if !config.cards.iter().any(|card| card.id() == "clock") {
            let fallback = AppConfig::default()
                .cards
                .into_iter()
                .next()
                .expect("the default configuration has one clock card");
            config.cards.push(fallback);
        }
        config.playlists[active_index].entries.push(PlaylistEntry {
            card_id: "clock".into(),
            dwell_seconds: None,
        });
    }
    config
}

fn drop_retired_cards_from_json(text: &str) -> Result<AppConfig, StoreError> {
    let mut value: serde_json::Value = parse_json(text)?;
    let mut retired_ids = std::collections::HashSet::new();
    if let Some(cards) = value
        .get_mut("cards")
        .and_then(serde_json::Value::as_array_mut)
    {
        cards.retain(|card| {
            let retired = card
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|kind| {
                    matches!(
                        kind,
                        "calendar" | "weather" | "json-feed" | "rss" | "plugin"
                    )
                });
            if retired && let Some(id) = card.get("id").and_then(serde_json::Value::as_str) {
                retired_ids.insert(id.to_owned());
            }
            !retired
        });
    }
    if let Some(playlists) = value
        .get_mut("playlists")
        .and_then(serde_json::Value::as_array_mut)
    {
        for playlist in playlists {
            if let Some(entries) = playlist
                .get_mut("entries")
                .and_then(serde_json::Value::as_array_mut)
            {
                // Entries name cards by identity, not by kind. Remove every
                // reference in the same pass as its retired card or a formerly
                // valid document would fail later as a dangling reference.
                entries.retain(|entry| {
                    entry
                        .get("card_id")
                        .and_then(serde_json::Value::as_str)
                        .is_none_or(|id| !retired_ids.contains(id))
                });
            }
        }
    }
    serde_json::from_value(value).map_err(|error| StoreError::InvalidJson {
        message: error.to_string(),
    })
}

fn parse_json<T: DeserializeOwned>(text: &str) -> Result<T, StoreError> {
    serde_json::from_str(text).map_err(|error| StoreError::InvalidJson {
        message: error.to_string(),
    })
}

#[allow(clippy::too_many_lines)]
fn decode_config(bytes: &[u8]) -> Result<(AppConfig, ConfigOrigin), StoreError> {
    let text = std::str::from_utf8(bytes).map_err(|_| StoreError::InvalidUtf8)?;
    let header: VersionHeader = parse_json(text)?;
    let (config, origin) = match header.schema_version {
        CURRENT_SCHEMA_VERSION => (parse_json(text)?, ConfigOrigin::Current),
        version @ (4..=8) => {
            // v4's asset variants (`icon { width, height }`, `font { pixel_size,
            // glyph_ranges }`) encoded the pre-tiny_ttf design where glyphs were
            // baked at a fixed size. `config.rs`'s compile step has always
            // rejected a non-empty `assets` array, so no saved v4 config has ever
            // contained one, which makes this migration a version bump with no
            // data transformation: the current `AppConfig` shape parses a v4
            // document unchanged because `assets` is always empty, and the v5->v6
            // change (a new card kind no v4 document could contain either) adds
            // nothing that shape lacks. v4 therefore migrates directly to the
            // current schema in one step, not chained through v5. v5 likewise differs
            // only by adding the plugin card kind, and v6->v7 adds `image_sources`,
            // which `#[serde(default)]` supplies for older documents. v8 and v9 are
            // subtractive bumps: remove retired card objects and their playlist
            // references while the JSON still has enough information to recognize
            // them, then deserialize the surviving current shape strictly.
            let legacy = drop_retired_cards_from_json(text)?;
            let origin = match version {
                4 => ConfigOrigin::MigratedV4,
                5 => ConfigOrigin::MigratedV5,
                6 => ConfigOrigin::MigratedV6,
                7 => ConfigOrigin::MigratedV7,
                8 => ConfigOrigin::MigratedV8,
                _ => unreachable!(),
            };
            (
                finish_migration(AppConfig {
                    schema_version: CURRENT_SCHEMA_VERSION,
                    ..legacy
                }),
                origin,
            )
        }
        3 => {
            let legacy: LegacyConfigV3 = parse_json(text)?;
            (
                finish_migration(migrate_v3(legacy)),
                ConfigOrigin::MigratedV3,
            )
        }
        2 => {
            let legacy: LegacyConfigV2 = parse_json(text)?;
            (
                finish_migration(migrate_v2(legacy)),
                ConfigOrigin::MigratedV2,
            )
        }
        1 => {
            let legacy: LegacyConfigV1 = parse_json(text)?;
            (
                finish_migration(migrate_legacy(
                    legacy.preferences,
                    legacy.widgets,
                    legacy.screens,
                )),
                ConfigOrigin::MigratedV1,
            )
        }
        0 => {
            let legacy: LegacyConfigV0 = parse_json(text)?;
            (
                finish_migration(migrate_legacy(
                    AppPreferences {
                        timezone: legacy.timezone,
                        ..AppPreferences::default()
                    },
                    legacy.widgets,
                    legacy.screens,
                )),
                ConfigOrigin::MigratedV0,
            )
        }
        found => {
            return Err(StoreError::UnsupportedVersion {
                found,
                supported: CURRENT_SCHEMA_VERSION,
            });
        }
    };
    config.validate().map_err(|error| StoreError::Validation {
        issues: error.issues,
    })?;
    Ok((config, origin))
}

fn card_from_legacy_widget(widget: LegacyWidgetSettings) -> Option<CardSettings> {
    match widget {
        LegacyWidgetSettings::Clock {
            id,
            size: _,
            title,
            show_seconds,
        } => Some(CardSettings::Clock {
            id,
            title,
            show_seconds,
            template: DisplayTemplate::DigitalClock,
            tap_action: WidgetTapAction::None,
            refresh: RefreshPolicy::DeviceLocal,
            alert: legacy_alert(false, LegacyCardKind::Clock),
        }),
        LegacyWidgetSettings::Pomodoro {
            id,
            size: _,
            label,
            duration_seconds,
        } => Some(CardSettings::Pomodoro {
            id,
            label,
            duration_seconds,
            template: DisplayTemplate::ProgressRing,
            tap_action: WidgetTapAction::StartPause,
            refresh: RefreshPolicy::DeviceLocal,
            alert: legacy_alert(true, LegacyCardKind::Pomodoro),
        }),
        LegacyWidgetSettings::Calendar { .. } => None,
    }
}

fn migrate_legacy(
    preferences: AppPreferences,
    widgets: Vec<LegacyWidgetSettings>,
    screens: Vec<LegacyScreenSettings>,
) -> AppConfig {
    // Legacy schemas required every widget to be assigned to exactly one
    // screen, so every migrated card is in rotation with the device's
    // default dwell. The on-device rotation order was the screen order
    // (dashboards did not exist in v0/v1, so each screen named exactly one
    // widget), so cards are rebuilt by walking `screens` in order and
    // looking up the widget each one names.
    let mut widgets_by_id: HashMap<String, LegacyWidgetSettings> = widgets
        .into_iter()
        .map(|widget| (legacy_widget_id(&widget).to_owned(), widget))
        .collect();
    let mut cards = Vec::with_capacity(widgets_by_id.len());
    let mut entries = Vec::with_capacity(screens.len());
    for screen in screens {
        if let Some(widget) = widgets_by_id.remove(&screen.widget_id)
            && let Some(card) = card_from_legacy_widget(widget)
        {
            entries.push(PlaylistEntry {
                card_id: card.id().to_owned(),
                dwell_seconds: None,
            });
            cards.push(card);
        }
    }

    // Widgets with no referencing screen: legacy validation should prevent these, but
    // the on-disk format permits them. Preserve them (design spec §5) rather than
    // silently dropping configuration: a widget whose historical interrupt policy was
    // enabled becomes alert-only so its alert can still fire; otherwise it becomes off.
    // Sort by ID so migration is deterministic regardless of hash-map iteration order.
    let mut orphans: Vec<LegacyWidgetSettings> = widgets_by_id.into_values().collect();
    orphans.sort_by(|left, right| legacy_widget_id(left).cmp(legacy_widget_id(right)));
    for widget in orphans {
        if let Some(card) = card_from_legacy_widget(widget) {
            cards.push(card);
        }
    }

    let playlist = synthesize_playlist(&cards, CarouselAdvance::Manual, entries);
    AppConfig {
        schema_version: CURRENT_SCHEMA_VERSION,
        preferences,
        cards,
        image_sources: Vec::new(),
        assets: Vec::new(),
        playlists: vec![playlist],
        active_playlist_id: DEFAULT_PLAYLIST_ID.into(),
        updater: UpdaterSettings::default(),
    }
}

#[derive(Clone, Copy)]
enum LegacyCardKind {
    Clock,
    Pomodoro,
}

/// The v2→v3 alert-migration rule (design spec §5, "Migration"): an old
/// `interrupt_policy: "enabled"` becomes the kind-appropriate alert with sensible
/// defaults — pomodoro to `on-timer-finish`/`until-dismissed`. Every other combination, INCLUDING every
/// `"disabled"` widget regardless of kind, becomes `alert: none`. The alert is derived
/// from the (historical) interrupt policy, never from the card kind alone.
fn legacy_alert(interrupt_policy_enabled: bool, kind: LegacyCardKind) -> CardAlert {
    if !interrupt_policy_enabled {
        return CardAlert::None;
    }
    match kind {
        LegacyCardKind::Pomodoro => CardAlert::OnTimerFinish {
            hold: AlertHold::UntilDismissed,
        },
        LegacyCardKind::Clock => CardAlert::None,
    }
}

fn legacy_widget_id(widget: &LegacyWidgetSettings) -> &str {
    match widget {
        LegacyWidgetSettings::Clock { id, .. }
        | LegacyWidgetSettings::Pomodoro { id, .. }
        | LegacyWidgetSettings::Calendar { id, .. } => id,
    }
}

fn read_bounded(path: &Path) -> Result<Option<Vec<u8>>, StoreError> {
    secure_file::read_bounded(path, MAX_CONFIG_FILE_BYTES).map_err(|error| match error {
        BoundedReadError::Io(error) => secure_io_error("config", error),
        BoundedReadError::TooLarge { maximum } => StoreError::TooLarge { maximum },
    })
}

fn recovered(state: &StoreState, error: StoreError) -> LoadOutcome {
    let (config, origin) = fallback(state);
    LoadOutcome::Recovered {
        config,
        origin,
        error,
    }
}

fn validation_failed(state: &StoreState, issues: Vec<ValidationIssue>) -> LoadOutcome {
    let (config, origin) = fallback(state);
    LoadOutcome::ValidationFailed {
        config,
        origin,
        issues,
    }
}

fn fallback(state: &StoreState) -> (AppConfig, ConfigOrigin) {
    match &state.last_good {
        Some(config) => (config.clone(), ConfigOrigin::LastGood),
        None => (AppConfig::default(), ConfigOrigin::Defaults),
    }
}

fn secure_io_error(subject: &str, error: FileIoError) -> StoreError {
    let (operation, message) = error.into_strings(subject);
    StoreError::Io { operation, message }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    use std::{fs, io};

    #[test]
    fn failed_mode_repair_does_not_discard_valid_config() {
        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "deskmate-mode-repair-failure-{}-{serial}.json",
            std::process::id()
        ));
        fs::write(
            &path,
            serde_json::to_vec_pretty(&AppConfig::default()).unwrap(),
        )
        .unwrap();

        let bytes =
            secure_file::read_bounded_with_mode_repair(&path, MAX_CONFIG_FILE_BYTES, |_| {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected chmod failure",
                ))
            })
            .unwrap()
            .unwrap();
        let (config, origin) = decode_config(&bytes).unwrap();

        assert_eq!(origin, ConfigOrigin::Current);
        assert_eq!(config, AppConfig::default());
        fs::remove_file(path).unwrap();
    }
}
