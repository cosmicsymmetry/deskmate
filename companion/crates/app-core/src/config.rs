use std::collections::{HashMap, HashSet};
use std::fmt;

use protocol::{
    ApplyConfig, Field, FieldValue, InterruptPolicy, MAX_CONFIG_SCREENS, MAX_CONFIG_WIDGETS,
    MAX_SCREEN_ID_LEN, MAX_WIDGET_ID_LEN, PushData, ScreenConfig, SizeClass, TapAction,
    TemplateKind, WidgetConfig,
};
use serde::{Deserialize, Serialize};

pub const CURRENT_SCHEMA_VERSION: u32 = 1;
pub const MAX_WIDGET_TITLE_LEN: usize = 64;
pub const MAX_ICS_SOURCE_LEN: usize = 2_048;
pub const MIN_POMODORO_SECONDS: u32 = 1;
pub const MAX_POMODORO_SECONDS: u32 = 86_400;
pub const MIN_CALENDAR_REFRESH_MINUTES: u16 = 1;
pub const MAX_CALENDAR_REFRESH_MINUTES: u16 = 1_440;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub schema_version: u32,
    pub preferences: AppPreferences,
    pub widgets: Vec<WidgetSettings>,
    pub screens: Vec<ScreenSettings>,
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
            }],
            screens: vec![ScreenSettings {
                id: "clock-screen".into(),
                widget_id: "clock".into(),
            }],
        }
    }
}

impl AppConfig {
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
        for (index, screen) in self.screens.iter().enumerate() {
            let path = format!("screens[{index}]");
            validate_identifier(
                &format!("{path}.id"),
                &screen.id,
                MAX_SCREEN_ID_LEN,
                &mut issues,
            );
            validate_identifier(
                &format!("{path}.widget_id"),
                &screen.widget_id,
                MAX_WIDGET_ID_LEN,
                &mut issues,
            );
            if !screen_ids.insert(screen.id.as_str()) {
                issues.push(ValidationIssue::new(
                    format!("{path}.id"),
                    ValidationCode::DuplicateId,
                    format!("screen ID {:?} is duplicated", screen.id),
                ));
            }
            if widget_ids.contains(screen.widget_id.as_str()) {
                *references.entry(&screen.widget_id).or_default() += 1;
            } else {
                issues.push(ValidationIssue::new(
                    format!("{path}.widget_id"),
                    ValidationCode::MissingReference,
                    format!("widget {:?} does not exist", screen.widget_id),
                ));
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

        let widgets = self
            .widgets
            .iter()
            .map(WidgetSettings::wire_config)
            .collect();
        let screens = self
            .screens
            .iter()
            .map(|screen| ScreenConfig {
                screen_id: screen.id.clone(),
                widget_id: screen.widget_id.clone(),
            })
            .collect();
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
                widgets,
                screens,
            },
            initial_pushes,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppPreferences {
    pub timezone: String,
    pub autostart: bool,
    pub paused: bool,
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            timezone: "UTC".into(),
            autostart: false,
            paused: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WidgetSize {
    Full,
    Standard,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum WidgetSettings {
    Clock {
        id: String,
        size: WidgetSize,
        title: String,
        show_seconds: bool,
    },
    Pomodoro {
        id: String,
        size: WidgetSize,
        label: String,
        duration_seconds: u32,
    },
    Calendar {
        id: String,
        size: WidgetSize,
        title: String,
        source: CalendarSource,
        refresh_minutes: u16,
    },
}

impl WidgetSettings {
    pub fn id(&self) -> &str {
        match self {
            Self::Clock { id, .. } | Self::Pomodoro { id, .. } | Self::Calendar { id, .. } => id,
        }
    }

    pub const fn size(&self) -> WidgetSize {
        match self {
            Self::Clock { size, .. }
            | Self::Pomodoro { size, .. }
            | Self::Calendar { size, .. } => *size,
        }
    }

    fn validate(&self, path: &str, issues: &mut Vec<ValidationIssue>) {
        match self {
            Self::Clock { size, title, .. } => {
                validate_size(path, *size, WidgetSize::Full, "clock", issues);
                validate_text(
                    &format!("{path}.title"),
                    title,
                    MAX_WIDGET_TITLE_LEN,
                    false,
                    issues,
                );
            }
            Self::Pomodoro {
                size,
                label,
                duration_seconds,
                ..
            } => {
                validate_size(path, *size, WidgetSize::Standard, "pomodoro", issues);
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
            }
            Self::Calendar {
                size,
                title,
                source,
                refresh_minutes,
                ..
            } => {
                validate_size(path, *size, WidgetSize::Standard, "calendar", issues);
                validate_text(
                    &format!("{path}.title"),
                    title,
                    MAX_WIDGET_TITLE_LEN,
                    false,
                    issues,
                );
                source.validate(&format!("{path}.source"), issues);
                if !(MIN_CALENDAR_REFRESH_MINUTES..=MAX_CALENDAR_REFRESH_MINUTES)
                    .contains(refresh_minutes)
                {
                    issues.push(ValidationIssue::new(
                        format!("{path}.refresh_minutes"),
                        ValidationCode::OutOfRange,
                        format!(
                            "refresh interval must be {MIN_CALENDAR_REFRESH_MINUTES}..={MAX_CALENDAR_REFRESH_MINUTES} minutes"
                        ),
                    ));
                }
            }
        }
    }

    fn wire_config(&self) -> WidgetConfig {
        match self {
            Self::Clock { id, size, .. } => WidgetConfig {
                widget_id: id.clone(),
                template: TemplateKind::DigitalClock,
                size_class: size.to_wire(),
                tap_action: TapAction::None,
                interrupt_policy: InterruptPolicy::Disabled,
            },
            Self::Pomodoro { id, size, .. } => WidgetConfig {
                widget_id: id.clone(),
                template: TemplateKind::ProgressRing,
                size_class: size.to_wire(),
                tap_action: TapAction::StartPause,
                interrupt_policy: InterruptPolicy::Enabled,
            },
            Self::Calendar { id, size, .. } => WidgetConfig {
                widget_id: id.clone(),
                template: TemplateKind::RowList,
                size_class: size.to_wire(),
                tap_action: TapAction::None,
                interrupt_policy: InterruptPolicy::Disabled,
            },
        }
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
        }
    }
}

impl WidgetSize {
    const fn to_wire(self) -> SizeClass {
        match self {
            Self::Full => SizeClass::Full,
            Self::Standard => SizeClass::Standard,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "kebab-case")]
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
        let value = self.value();
        validate_text(path, value, MAX_ICS_SOURCE_LEN, true, issues);
        if let Self::Url(url) = self {
            let valid_scheme = url.starts_with("https://") || url.starts_with("http://");
            if !url.is_empty() && !valid_scheme {
                issues.push(ValidationIssue::new(
                    path,
                    ValidationCode::InvalidSource,
                    "calendar URL must start with https:// or http://",
                ));
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScreenSettings {
    pub id: String,
    pub widget_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledAppConfig {
    pub layout: ApplyConfig,
    pub initial_pushes: Vec<PushData>,
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

fn validate_size(
    path: &str,
    actual: WidgetSize,
    expected: WidgetSize,
    widget_kind: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    if actual != expected {
        issues.push(ValidationIssue::new(
            format!("{path}.size"),
            ValidationCode::UnsupportedSize,
            format!("{widget_kind} widgets require {expected:?} size in M3"),
        ));
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
