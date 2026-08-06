use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use chrono::{
    DateTime, Datelike, Duration as ChronoDuration, LocalResult, NaiveDate, NaiveDateTime,
    TimeZone, Utc, Weekday,
};
use chrono_tz::Tz;
use protocol::{Field, FieldValue};

use crate::http::{HttpClient, SystemHttpClient};
use crate::{LastGood, Provider, ProviderError, ProviderSnapshot, RefreshPolicy};

pub const MAX_ICS_BYTES: usize = 1_048_576;
pub const MAX_ICS_EVENTS: usize = 4_096;
pub const MAX_UNFOLDED_LINE_BYTES: usize = 8_192;
pub const MAX_CALENDAR_ROWS: usize = 5;
const MAX_RECURRENCE_DAYS: i64 = 100_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IcsSource {
    File(PathBuf),
    Url(String),
}

pub trait IcsLoader {
    fn load(&mut self, source: &IcsSource) -> Result<String, ProviderError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemIcsLoader;

impl IcsLoader for SystemIcsLoader {
    fn load(&mut self, source: &IcsSource) -> Result<String, ProviderError> {
        match source {
            IcsSource::File(path) => {
                let file =
                    File::open(path).map_err(|error| ProviderError::Io(error.to_string()))?;
                let mut bytes = Vec::new();
                file.take((MAX_ICS_BYTES + 1) as u64)
                    .read_to_end(&mut bytes)
                    .map_err(|error| ProviderError::Io(error.to_string()))?;
                if bytes.len() > MAX_ICS_BYTES {
                    return Err(ProviderError::ResponseTooLarge);
                }
                String::from_utf8(bytes).map_err(|_| ProviderError::InvalidEncoding)
            }
            IcsSource::Url(url) => SystemHttpClient::default().get_text(url),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarOptions {
    pub default_timezone: Tz,
    pub display_timezone: Tz,
    pub horizon: Duration,
    pub maximum_events: usize,
    pub refresh_interval: Duration,
    pub title: String,
}

impl Default for CalendarOptions {
    fn default() -> Self {
        Self {
            default_timezone: chrono_tz::UTC,
            display_timezone: chrono_tz::UTC,
            horizon: Duration::from_hours(366 * 24),
            maximum_events: MAX_CALENDAR_ROWS,
            refresh_interval: Duration::from_mins(15),
            title: "Calendar".into(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IcsCounters {
    pub malformed_events: u32,
    pub duplicate_uids: u32,
    pub cancelled_events: u32,
    pub unsupported_recurrences: u32,
    pub recurrence_scan_limit_hits: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarEvent {
    pub uid: String,
    pub title: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub all_day: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IcsCalendar {
    pub events: Vec<CalendarEvent>,
    pub counters: IcsCounters,
}

impl IcsCalendar {
    pub fn fields(
        &self,
        title: &str,
        display_timezone: Tz,
        now: DateTime<Utc>,
        stale: bool,
        error: Option<&str>,
    ) -> Vec<Field> {
        let mut fields = Vec::with_capacity(14);
        fields.push(text_field("title", truncate_utf8(title, 64)));
        // Machine-readable next-event start, in contrast to the localised,
        // date-dropping `row0_time` display text below. Callers that need to act
        // on the event start (e.g. a "before event" alert trigger) must read this
        // field, not parse the display text. `0` means "no upcoming event";
        // events are sorted ascending, so `events.first()` is always the next
        // occurrence, not the recurrence series start. Derived from `event.start`
        // (a `DateTime<Utc>`) via `timestamp_millis()`, so — unlike `row0_time`
        // above — the value does not depend on `display_timezone` at all.
        //
        // KNOWN COUNTER DRIFT: this key is not (yet) in the `RowList` field
        // registry (`docs/protocol/v1.md`'s template field table only lists
        // `title`, `row{N}_title`, `row{N}_time`, `stale`, `error` for
        // `RowList`), so firmware's `widget_model_push_widget_fields`
        // (`firmware/main/core/widget_model.c`) counts it as an unknown field
        // on every calendar push, and `unknown_field_count` climbs
        // (saturating, never wrapping) for the life of the device. This is
        // harmless — the 14 fields pushed here stay well under
        // `protocol::MAX_FIELD_COUNT` (16) — but is a deliberate, known
        // consequence of adding a host-only field without a matching firmware
        // registry update (out of scope here; wire protocol is unchanged).
        // Documented so a future reader debugging a climbing
        // `unknown_field_count` on a calendar widget doesn't chase it as a
        // fault.
        fields.push(Field {
            key: "next_start_unix_ms".into(),
            value: FieldValue::Integer(
                self.events
                    .first()
                    .map_or(0, |event| event.start.timestamp_millis()),
            ),
        });
        let local_today = now.with_timezone(&display_timezone).date_naive();
        for row in 0..MAX_CALENDAR_ROWS {
            let (row_title, row_time) = self.events.get(row).map_or_else(
                || (String::new(), String::new()),
                |event| {
                    let local_start = event.start.with_timezone(&display_timezone);
                    let time = if event.all_day {
                        if local_start.date_naive() == local_today {
                            "Today · All day".into()
                        } else {
                            format!("{} · All day", local_start.format("%a %b %-d"))
                        }
                    } else if local_start.date_naive() == local_today {
                        local_start.format("%H:%M").to_string()
                    } else {
                        local_start.format("%a %H:%M").to_string()
                    };
                    (truncate_utf8(&event.title, 96), truncate_utf8(&time, 32))
                },
            );
            fields.push(text_field(format!("row{row}_title"), row_title));
            fields.push(text_field(format!("row{row}_time"), row_time));
        }
        fields.push(Field {
            key: "stale".into(),
            value: FieldValue::Boolean(stale),
        });
        fields.push(text_field(
            "error",
            error.map_or_else(String::new, |message| truncate_utf8(message, 96)),
        ));
        fields
    }
}

pub struct IcsProvider<L> {
    source: IcsSource,
    loader: L,
    options: CalendarOptions,
    state: LastGood<IcsCalendar>,
}

impl<L: IcsLoader> IcsProvider<L> {
    pub fn new(source: IcsSource, loader: L, options: CalendarOptions) -> Self {
        Self {
            source,
            loader,
            options,
            state: LastGood::default(),
        }
    }

    pub fn fields(
        &self,
        snapshot: &ProviderSnapshot<IcsCalendar>,
        now: DateTime<Utc>,
    ) -> Vec<Field> {
        snapshot.value.fields(
            &self.options.title,
            self.options.display_timezone,
            now,
            snapshot.stale,
            snapshot.error.as_deref(),
        )
    }

    pub fn options(&self) -> &CalendarOptions {
        &self.options
    }
}

impl IcsProvider<SystemIcsLoader> {
    pub fn system(source: IcsSource, options: CalendarOptions) -> Self {
        Self::new(source, SystemIcsLoader, options)
    }
}

impl<L: IcsLoader> Provider for IcsProvider<L> {
    type Output = IcsCalendar;

    fn refresh_policy(&self) -> RefreshPolicy {
        RefreshPolicy::Interval(self.options.refresh_interval)
    }

    fn refresh(&mut self, now: DateTime<Utc>) -> ProviderSnapshot<Self::Output> {
        let result = self
            .loader
            .load(&self.source)
            .and_then(|feed| parse_ics(&feed, now, &self.options));
        self.state.complete(now, result)
    }
}

#[derive(Debug, Clone)]
struct Property {
    name: String,
    parameters: Vec<(String, String)>,
    value: String,
}

impl Property {
    fn parameter(&self, name: &str) -> Option<&str> {
        self.parameters
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Debug, Clone)]
struct EventComponent {
    properties: Vec<Property>,
    source_index: usize,
}

impl EventComponent {
    fn property(&self, name: &str) -> Option<&Property> {
        self.properties
            .iter()
            .find(|property| property.name == name)
    }

    fn properties(&self, name: &str) -> impl Iterator<Item = &Property> {
        self.properties
            .iter()
            .filter(move |property| property.name == name)
    }
}

#[derive(Debug, Clone, Copy)]
struct EventTime {
    local: NaiveDateTime,
    timezone: Tz,
    all_day: bool,
}

impl EventTime {
    fn to_utc(self) -> Option<DateTime<Utc>> {
        resolve_local(self.timezone, self.local).map(|time| time.with_timezone(&Utc))
    }

    fn on_date(self, date: NaiveDate) -> Self {
        Self {
            local: date.and_time(self.local.time()),
            ..self
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Frequency {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WeekdaySpec {
    ordinal: Option<i8>,
    weekday: Weekday,
}

#[derive(Debug, Clone)]
struct RecurrenceRule {
    frequency: Frequency,
    interval: u32,
    count: Option<u32>,
    until: Option<DateTime<Utc>>,
    weekdays: Vec<WeekdaySpec>,
    month_days: Vec<i8>,
    months: Vec<u8>,
    set_positions: Vec<i16>,
    week_start: Weekday,
}

#[derive(Debug, Clone)]
struct ParsedEvent {
    uid: String,
    title: String,
    start: EventTime,
    duration: ChronoDuration,
    recurrence: Option<RecurrenceRule>,
    exclusions: Vec<DateTime<Utc>>,
    additional_dates: Vec<DateTime<Utc>>,
}

enum EventParseError {
    Malformed,
    UnsupportedRecurrence,
}

pub fn parse_ics(
    feed: &str,
    now: DateTime<Utc>,
    options: &CalendarOptions,
) -> Result<IcsCalendar, ProviderError> {
    let components = parse_components(feed)?;
    let mut counters = IcsCounters::default();
    let selected = select_uid_versions(components, &mut counters);
    let horizon = now
        + ChronoDuration::from_std(options.horizon)
            .map_err(|_| ProviderError::MalformedFeed("calendar horizon is too large".into()))?;
    let mut events = Vec::new();
    let mut groups = BTreeMap::<String, Vec<EventComponent>>::new();
    for component in selected {
        let Some(uid) = component
            .property("UID")
            .map(|property| property.value.clone())
        else {
            counters.malformed_events = counters.malformed_events.saturating_add(1);
            continue;
        };
        groups.entry(uid).or_default().push(component);
    }
    for (_, components) in groups {
        let Some(master) = components
            .iter()
            .find(|component| component.property("RECURRENCE-ID").is_none())
        else {
            counters.malformed_events = counters.malformed_events.saturating_add(1);
            continue;
        };
        if master
            .property("STATUS")
            .is_some_and(|property| property.value.eq_ignore_ascii_case("CANCELLED"))
        {
            counters.cancelled_events = counters.cancelled_events.saturating_add(1);
            continue;
        }
        match parse_event(master, options.default_timezone) {
            Ok(mut event) => {
                let mut overrides = Vec::new();
                for exception in components
                    .iter()
                    .filter(|component| component.property("RECURRENCE-ID").is_some())
                {
                    let recurrence_id = exception
                        .property("RECURRENCE-ID")
                        .filter(|property| property.parameter("RANGE").is_none())
                        .and_then(|property| parse_event_time(property, event.start.timezone))
                        .and_then(EventTime::to_utc);
                    let Some(recurrence_id) = recurrence_id else {
                        counters.unsupported_recurrences =
                            counters.unsupported_recurrences.saturating_add(1);
                        continue;
                    };
                    event.exclusions.push(recurrence_id);
                    if exception
                        .property("STATUS")
                        .is_some_and(|property| property.value.eq_ignore_ascii_case("CANCELLED"))
                    {
                        counters.cancelled_events = counters.cancelled_events.saturating_add(1);
                        continue;
                    }
                    match parse_event(exception, options.default_timezone) {
                        Ok(mut override_event) if override_event.recurrence.is_none() => {
                            override_event.additional_dates.clear();
                            overrides.push(override_event);
                        }
                        Ok(_) | Err(EventParseError::UnsupportedRecurrence) => {
                            counters.unsupported_recurrences =
                                counters.unsupported_recurrences.saturating_add(1);
                        }
                        Err(EventParseError::Malformed) => {
                            counters.malformed_events = counters.malformed_events.saturating_add(1);
                        }
                    }
                }
                expand_event(&event, now, horizon, &mut events, &mut counters);
                for override_event in overrides {
                    expand_event(&override_event, now, horizon, &mut events, &mut counters);
                }
            }
            Err(EventParseError::Malformed) => {
                counters.malformed_events = counters.malformed_events.saturating_add(1);
            }
            Err(EventParseError::UnsupportedRecurrence) => {
                counters.unsupported_recurrences =
                    counters.unsupported_recurrences.saturating_add(1);
            }
        }
    }
    events.sort_by(|left, right| {
        left.start
            .cmp(&right.start)
            .then_with(|| right.all_day.cmp(&left.all_day))
            .then_with(|| left.title.cmp(&right.title))
            .then_with(|| left.uid.cmp(&right.uid))
    });
    events.truncate(options.maximum_events.min(MAX_CALENDAR_ROWS));
    Ok(IcsCalendar { events, counters })
}

fn parse_components(feed: &str) -> Result<Vec<EventComponent>, ProviderError> {
    if feed.len() > MAX_ICS_BYTES {
        return Err(ProviderError::ResponseTooLarge);
    }
    let lines = unfold_lines(feed)?;
    let mut saw_calendar_begin = false;
    let mut saw_calendar_end = false;
    let mut current = None::<Vec<Property>>;
    let mut components = Vec::new();
    for line in lines {
        let property = parse_content_line(&line)?;
        if property.name == "BEGIN" && property.value.eq_ignore_ascii_case("VCALENDAR") {
            saw_calendar_begin = true;
        } else if property.name == "END" && property.value.eq_ignore_ascii_case("VCALENDAR") {
            if current.is_some() {
                return Err(ProviderError::MalformedFeed(
                    "calendar ended inside VEVENT".into(),
                ));
            }
            saw_calendar_end = true;
        } else if property.name == "BEGIN" && property.value.eq_ignore_ascii_case("VEVENT") {
            if current.is_some() {
                return Err(ProviderError::MalformedFeed("nested VEVENT".into()));
            }
            if components.len() >= MAX_ICS_EVENTS {
                return Err(ProviderError::MalformedFeed("too many VEVENTs".into()));
            }
            current = Some(Vec::new());
        } else if property.name == "END" && property.value.eq_ignore_ascii_case("VEVENT") {
            let properties = current
                .take()
                .ok_or_else(|| ProviderError::MalformedFeed("END:VEVENT without BEGIN".into()))?;
            let source_index = components.len();
            components.push(EventComponent {
                properties,
                source_index,
            });
        } else if let Some(properties) = &mut current {
            properties.push(property);
        }
    }
    if current.is_some() {
        return Err(ProviderError::MalformedFeed("unterminated VEVENT".into()));
    }
    if !saw_calendar_begin || !saw_calendar_end {
        return Err(ProviderError::MalformedFeed(
            "missing VCALENDAR boundary".into(),
        ));
    }
    Ok(components)
}

fn unfold_lines(feed: &str) -> Result<Vec<String>, ProviderError> {
    let mut unfolded = Vec::<String>::new();
    for raw_line in feed.lines() {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.starts_with([' ', '\t']) {
            let previous = unfolded
                .last_mut()
                .ok_or_else(|| ProviderError::MalformedFeed("folded first content line".into()))?;
            previous.push_str(&line[1..]);
            if previous.len() > MAX_UNFOLDED_LINE_BYTES {
                return Err(ProviderError::MalformedFeed(
                    "unfolded content line is too long".into(),
                ));
            }
        } else {
            if line.len() > MAX_UNFOLDED_LINE_BYTES {
                return Err(ProviderError::MalformedFeed(
                    "content line is too long".into(),
                ));
            }
            if !line.is_empty() {
                unfolded.push(line.to_owned());
            }
        }
    }
    Ok(unfolded)
}

fn parse_content_line(line: &str) -> Result<Property, ProviderError> {
    let (head, value) = line
        .split_once(':')
        .ok_or_else(|| ProviderError::MalformedFeed("content line has no value".into()))?;
    let mut parts = head.split(';');
    let name = parts
        .next()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| ProviderError::MalformedFeed("content line has no name".into()))?
        .to_ascii_uppercase();
    let mut parameters = Vec::new();
    for parameter in parts {
        let (key, value) = parameter
            .split_once('=')
            .ok_or_else(|| ProviderError::MalformedFeed("invalid property parameter".into()))?;
        parameters.push((key.to_ascii_uppercase(), value.trim_matches('"').to_owned()));
    }
    Ok(Property {
        name,
        parameters,
        value: value.to_owned(),
    })
}

fn select_uid_versions(
    components: Vec<EventComponent>,
    counters: &mut IcsCounters,
) -> Vec<EventComponent> {
    let mut selected = Vec::<EventComponent>::new();
    let mut uid_indexes = HashMap::<String, usize>::new();
    for component in components {
        let Some(uid) = component
            .property("UID")
            .map(|property| property.value.clone())
        else {
            counters.malformed_events = counters.malformed_events.saturating_add(1);
            continue;
        };
        if uid.is_empty() {
            counters.malformed_events = counters.malformed_events.saturating_add(1);
            continue;
        }
        let instance_key = component.property("RECURRENCE-ID").map_or_else(
            || uid.clone(),
            |property| {
                format!(
                    "{uid}\u{1f}{}\u{1f}{}",
                    property.parameter("TZID").unwrap_or_default(),
                    property.value
                )
            },
        );
        if let Some(index) = uid_indexes.get(&instance_key).copied() {
            counters.duplicate_uids = counters.duplicate_uids.saturating_add(1);
            let old_sequence = sequence(&selected[index]);
            let new_sequence = sequence(&component);
            if new_sequence > old_sequence
                || (new_sequence == old_sequence
                    && component.source_index > selected[index].source_index)
            {
                selected[index] = component;
            }
        } else {
            uid_indexes.insert(instance_key, selected.len());
            selected.push(component);
        }
    }
    selected
}

fn sequence(component: &EventComponent) -> u32 {
    component
        .property("SEQUENCE")
        .and_then(|property| property.value.parse().ok())
        .unwrap_or(0)
}

fn parse_event(
    component: &EventComponent,
    default_timezone: Tz,
) -> Result<ParsedEvent, EventParseError> {
    let uid = component
        .property("UID")
        .map(|property| property.value.clone())
        .ok_or(EventParseError::Malformed)?;
    let title = component.property("SUMMARY").map_or_else(
        || "(Untitled)".into(),
        |property| unescape_text(&property.value),
    );
    let start_property = component
        .property("DTSTART")
        .ok_or(EventParseError::Malformed)?;
    let start =
        parse_event_time(start_property, default_timezone).ok_or(EventParseError::Malformed)?;
    let start_utc = start.to_utc().ok_or(EventParseError::Malformed)?;
    let duration = if let Some(end_property) = component.property("DTEND") {
        let end =
            parse_event_time(end_property, default_timezone).ok_or(EventParseError::Malformed)?;
        let duration = end.to_utc().ok_or(EventParseError::Malformed)? - start_utc;
        if duration <= ChronoDuration::zero() {
            return Err(EventParseError::Malformed);
        }
        duration
    } else if start.all_day {
        ChronoDuration::days(1)
    } else {
        ChronoDuration::zero()
    };
    let recurrence = component
        .property("RRULE")
        .map(|property| parse_recurrence(&property.value, start.timezone))
        .transpose()?;
    let mut exclusions = Vec::new();
    for property in component.properties("EXDATE") {
        for raw in property.value.split(',') {
            let mut single = property.clone();
            raw.clone_into(&mut single.value);
            let exclusion = parse_event_time(&single, default_timezone)
                .and_then(EventTime::to_utc)
                .ok_or(EventParseError::Malformed)?;
            exclusions.push(exclusion);
        }
    }
    let mut additional_dates = Vec::new();
    for property in component.properties("RDATE") {
        if property.parameter("VALUE") == Some("PERIOD") {
            return Err(EventParseError::UnsupportedRecurrence);
        }
        for raw in property.value.split(',') {
            let mut single = property.clone();
            raw.clone_into(&mut single.value);
            let date = parse_event_time(&single, default_timezone)
                .and_then(EventTime::to_utc)
                .ok_or(EventParseError::Malformed)?;
            additional_dates.push(date);
        }
    }
    additional_dates.sort_unstable();
    additional_dates.dedup();
    Ok(ParsedEvent {
        uid,
        title,
        start,
        duration,
        recurrence,
        exclusions,
        additional_dates,
    })
}

fn parse_event_time(property: &Property, default_timezone: Tz) -> Option<EventTime> {
    let is_date = property.parameter("VALUE") == Some("DATE") || property.value.len() == 8;
    if is_date {
        let date = NaiveDate::parse_from_str(&property.value, "%Y%m%d").ok()?;
        return Some(EventTime {
            local: date.and_hms_opt(0, 0, 0)?,
            timezone: property
                .parameter("TZID")
                .and_then(|name| name.parse().ok())
                .unwrap_or(default_timezone),
            all_day: true,
        });
    }
    let (raw, timezone) = if let Some(raw) = property.value.strip_suffix('Z') {
        (raw, chrono_tz::UTC)
    } else if let Some(name) = property.parameter("TZID") {
        (property.value.as_str(), name.parse().ok()?)
    } else {
        (property.value.as_str(), default_timezone)
    };
    let local = NaiveDateTime::parse_from_str(raw, "%Y%m%dT%H%M%S")
        .or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y%m%dT%H%M"))
        .ok()?;
    Some(EventTime {
        local,
        timezone,
        all_day: false,
    })
}

#[allow(clippy::too_many_lines)]
fn parse_recurrence(raw: &str, default_timezone: Tz) -> Result<RecurrenceRule, EventParseError> {
    let mut frequency = None;
    let mut interval = 1_u32;
    let mut count = None;
    let mut until = None;
    let mut weekdays = Vec::new();
    let mut month_days = Vec::new();
    let mut months = Vec::new();
    let mut set_positions = Vec::new();
    let mut week_start = Weekday::Mon;
    for part in raw.split(';') {
        let (key, value) = part
            .split_once('=')
            .ok_or(EventParseError::UnsupportedRecurrence)?;
        match key.to_ascii_uppercase().as_str() {
            "FREQ" => {
                frequency = Some(match value.to_ascii_uppercase().as_str() {
                    "DAILY" => Frequency::Daily,
                    "WEEKLY" => Frequency::Weekly,
                    "MONTHLY" => Frequency::Monthly,
                    "YEARLY" => Frequency::Yearly,
                    _ => return Err(EventParseError::UnsupportedRecurrence),
                });
            }
            "INTERVAL" => {
                interval = value
                    .parse()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or(EventParseError::Malformed)?;
            }
            "COUNT" => {
                count = Some(
                    value
                        .parse()
                        .ok()
                        .filter(|value| *value > 0)
                        .ok_or(EventParseError::Malformed)?,
                );
            }
            "UNTIL" => {
                let property = Property {
                    name: "UNTIL".into(),
                    parameters: Vec::new(),
                    value: value.into(),
                };
                until = Some(
                    parse_event_time(&property, default_timezone)
                        .and_then(EventTime::to_utc)
                        .ok_or(EventParseError::Malformed)?,
                );
            }
            "BYDAY" => {
                for day in value.split(',') {
                    weekdays.push(parse_weekday_spec(day)?);
                }
                weekdays.sort_unstable_by_key(|spec| {
                    (
                        spec.weekday.num_days_from_monday(),
                        spec.ordinal.unwrap_or_default(),
                    )
                });
                weekdays.dedup();
            }
            "BYMONTHDAY" => {
                for day in value.split(',') {
                    month_days.push(
                        day.parse::<i8>()
                            .ok()
                            .filter(|day| *day != 0 && (-31..=31).contains(day))
                            .ok_or(EventParseError::Malformed)?,
                    );
                }
                month_days.sort_unstable();
                month_days.dedup();
            }
            "BYMONTH" => {
                for month in value.split(',') {
                    months.push(
                        month
                            .parse::<u8>()
                            .ok()
                            .filter(|month| (1..=12).contains(month))
                            .ok_or(EventParseError::Malformed)?,
                    );
                }
                months.sort_unstable();
                months.dedup();
            }
            "BYSETPOS" => {
                for position in value.split(',') {
                    set_positions.push(
                        position
                            .parse::<i16>()
                            .ok()
                            .filter(|position| *position != 0 && (-366..=366).contains(position))
                            .ok_or(EventParseError::Malformed)?,
                    );
                }
                set_positions.sort_unstable();
                set_positions.dedup();
            }
            "WKST" => week_start = parse_weekday(value)?,
            _ => return Err(EventParseError::UnsupportedRecurrence),
        }
    }
    if count.is_some() && until.is_some() {
        return Err(EventParseError::Malformed);
    }
    let frequency = frequency.ok_or(EventParseError::Malformed)?;
    if matches!(frequency, Frequency::Daily | Frequency::Weekly)
        && weekdays.iter().any(|spec| spec.ordinal.is_some())
    {
        return Err(EventParseError::UnsupportedRecurrence);
    }
    Ok(RecurrenceRule {
        frequency,
        interval,
        count,
        until,
        weekdays,
        month_days,
        months,
        set_positions,
        week_start,
    })
}

fn parse_weekday_spec(raw: &str) -> Result<WeekdaySpec, EventParseError> {
    if raw.len() < 2 {
        return Err(EventParseError::UnsupportedRecurrence);
    }
    let (ordinal, weekday) = raw.split_at(raw.len() - 2);
    let ordinal = if ordinal.is_empty() {
        None
    } else {
        Some(
            ordinal
                .parse::<i8>()
                .ok()
                .filter(|value| *value != 0 && (-53..=53).contains(value))
                .ok_or(EventParseError::Malformed)?,
        )
    };
    Ok(WeekdaySpec {
        ordinal,
        weekday: parse_weekday(weekday)?,
    })
}

fn parse_weekday(raw: &str) -> Result<Weekday, EventParseError> {
    match raw.to_ascii_uppercase().as_str() {
        "MO" => Ok(Weekday::Mon),
        "TU" => Ok(Weekday::Tue),
        "WE" => Ok(Weekday::Wed),
        "TH" => Ok(Weekday::Thu),
        "FR" => Ok(Weekday::Fri),
        "SA" => Ok(Weekday::Sat),
        "SU" => Ok(Weekday::Sun),
        _ => Err(EventParseError::UnsupportedRecurrence),
    }
}

fn expand_event(
    event: &ParsedEvent,
    now: DateTime<Utc>,
    horizon: DateTime<Utc>,
    output: &mut Vec<CalendarEvent>,
    counters: &mut IcsCounters,
) {
    let start_date = event.start.local.date();
    let horizon_date = horizon
        .with_timezone(&event.start.timezone)
        .date_naive()
        .succ_opt()
        .unwrap_or_else(|| horizon.with_timezone(&event.start.timezone).date_naive());
    let maximum_days = (horizon_date - start_date)
        .num_days()
        .clamp(0, MAX_RECURRENCE_DAYS);
    let mut recurrence_count = 0_u32;
    let mut exhausted = false;
    let mut starts = BTreeSet::new();
    for day_offset in 0..=maximum_days {
        let Some(date) = start_date.checked_add_signed(ChronoDuration::days(day_offset)) else {
            break;
        };
        let matches = event.recurrence.as_ref().map_or(day_offset == 0, |rule| {
            recurrence_matches(rule, start_date, date)
        });
        if !matches {
            continue;
        }
        recurrence_count = recurrence_count.saturating_add(1);
        if event
            .recurrence
            .as_ref()
            .and_then(|rule| rule.count)
            .is_some_and(|count| recurrence_count > count)
        {
            break;
        }
        let candidate = event.start.on_date(date);
        let Some(start) = candidate.to_utc() else {
            continue;
        };
        if event
            .recurrence
            .as_ref()
            .and_then(|rule| rule.until)
            .is_some_and(|until| start > until)
        {
            break;
        }
        if event.exclusions.contains(&start) {
            continue;
        }
        starts.insert(start);
        if event.recurrence.is_none() {
            break;
        }
        if day_offset == MAX_RECURRENCE_DAYS {
            exhausted = true;
        }
    }
    if exhausted {
        counters.recurrence_scan_limit_hits = counters.recurrence_scan_limit_hits.saturating_add(1);
    }
    starts.extend(
        event
            .additional_dates
            .iter()
            .copied()
            .filter(|start| !event.exclusions.contains(start)),
    );
    for start in starts {
        let end = start + event.duration;
        let is_upcoming = if event.duration.is_zero() {
            start >= now
        } else {
            end > now
        };
        if is_upcoming && start <= horizon {
            output.push(CalendarEvent {
                uid: event.uid.clone(),
                title: truncate_utf8(&event.title, 96),
                start,
                end,
                all_day: event.start.all_day,
            });
        }
    }
}

fn recurrence_matches(rule: &RecurrenceRule, start: NaiveDate, candidate: NaiveDate) -> bool {
    if candidate == start {
        return true;
    }
    let days = (candidate - start).num_days();
    if days < 0 {
        return false;
    }
    let interval_matches = match rule.frequency {
        Frequency::Daily => days % i64::from(rule.interval) == 0,
        Frequency::Weekly => {
            let start_week = week_start(start, rule.week_start);
            let candidate_week = week_start(candidate, rule.week_start);
            let weeks = (candidate_week - start_week).num_days() / 7;
            weeks % i64::from(rule.interval) == 0
        }
        Frequency::Monthly => {
            let months = (candidate.year() - start.year()) * 12
                + i32::try_from(candidate.month()).unwrap_or(0)
                - i32::try_from(start.month()).unwrap_or(0);
            months >= 0 && months % i32::try_from(rule.interval).unwrap_or(i32::MAX) == 0
        }
        Frequency::Yearly => {
            let years = candidate.year() - start.year();
            years >= 0 && years % i32::try_from(rule.interval).unwrap_or(i32::MAX) == 0
        }
    };
    if !interval_matches || !recurrence_filters_match(rule, start, candidate) {
        return false;
    }
    if rule.set_positions.is_empty() {
        return true;
    }
    set_position_matches(rule, start, candidate)
}

fn recurrence_filters_match(rule: &RecurrenceRule, start: NaiveDate, candidate: NaiveDate) -> bool {
    if !rule.months.is_empty()
        && !rule
            .months
            .contains(&u8::try_from(candidate.month()).unwrap_or_default())
    {
        return false;
    }
    if !rule.month_days.is_empty()
        && !rule
            .month_days
            .iter()
            .any(|day| month_day_matches(candidate, *day))
    {
        return false;
    }
    if !rule.weekdays.is_empty()
        && !rule
            .weekdays
            .iter()
            .any(|spec| weekday_matches(rule, candidate, *spec))
    {
        return false;
    }
    match rule.frequency {
        Frequency::Daily => true,
        Frequency::Weekly => !rule.weekdays.is_empty() || candidate.weekday() == start.weekday(),
        Frequency::Monthly => {
            !rule.weekdays.is_empty()
                || !rule.month_days.is_empty()
                || candidate.day() == start.day()
        }
        Frequency::Yearly => {
            if rule.weekdays.is_empty() && rule.month_days.is_empty() {
                let month_matches = !rule.months.is_empty() || candidate.month() == start.month();
                month_matches && candidate.day() == start.day()
            } else {
                true
            }
        }
    }
}

fn set_position_matches(rule: &RecurrenceRule, start: NaiveDate, candidate: NaiveDate) -> bool {
    let (first, last) = period_bounds(rule, candidate);
    let mut dates = Vec::new();
    let mut date = first;
    loop {
        if date >= start
            && recurrence_interval_matches(rule, start, date)
            && recurrence_filters_match(rule, start, date)
        {
            dates.push(date);
        }
        if date >= last {
            break;
        }
        let Some(next) = date.succ_opt() else {
            break;
        };
        date = next;
    }
    rule.set_positions.iter().any(|position| {
        let index = if *position > 0 {
            usize::try_from(*position - 1).ok()
        } else {
            isize::try_from(dates.len())
                .ok()
                .and_then(|length| length.checked_add(isize::from(*position)))
                .and_then(|index| usize::try_from(index).ok())
        };
        index.and_then(|index| dates.get(index)).copied() == Some(candidate)
    })
}

fn recurrence_interval_matches(
    rule: &RecurrenceRule,
    start: NaiveDate,
    candidate: NaiveDate,
) -> bool {
    if candidate < start {
        return false;
    }
    match rule.frequency {
        Frequency::Daily => (candidate - start).num_days() % i64::from(rule.interval) == 0,
        Frequency::Weekly => {
            let weeks = (week_start(candidate, rule.week_start)
                - week_start(start, rule.week_start))
            .num_days()
                / 7;
            weeks % i64::from(rule.interval) == 0
        }
        Frequency::Monthly => {
            let months = (candidate.year() - start.year()) * 12
                + i32::try_from(candidate.month()).unwrap_or_default()
                - i32::try_from(start.month()).unwrap_or_default();
            months % i32::try_from(rule.interval).unwrap_or(i32::MAX) == 0
        }
        Frequency::Yearly => {
            (candidate.year() - start.year()) % i32::try_from(rule.interval).unwrap_or(i32::MAX)
                == 0
        }
    }
}

fn period_bounds(rule: &RecurrenceRule, candidate: NaiveDate) -> (NaiveDate, NaiveDate) {
    match rule.frequency {
        Frequency::Daily => (candidate, candidate),
        Frequency::Weekly => {
            let first = week_start(candidate, rule.week_start);
            (first, first + ChronoDuration::days(6))
        }
        Frequency::Monthly => {
            let first = candidate.with_day(1).unwrap_or(candidate);
            (
                first,
                first
                    .with_day(days_in_month(candidate))
                    .unwrap_or(candidate),
            )
        }
        Frequency::Yearly => {
            let first = NaiveDate::from_ymd_opt(candidate.year(), 1, 1).unwrap_or(candidate);
            let last = NaiveDate::from_ymd_opt(candidate.year(), 12, 31).unwrap_or(candidate);
            (first, last)
        }
    }
}

fn weekday_matches(rule: &RecurrenceRule, candidate: NaiveDate, spec: WeekdaySpec) -> bool {
    if candidate.weekday() != spec.weekday {
        return false;
    }
    let Some(ordinal) = spec.ordinal else {
        return true;
    };
    if rule.frequency == Frequency::Yearly && rule.months.is_empty() {
        ordinal_matches(
            ordinal,
            candidate,
            NaiveDate::from_ymd_opt(candidate.year(), 1, 1).unwrap_or(candidate),
            NaiveDate::from_ymd_opt(candidate.year(), 12, 31).unwrap_or(candidate),
        )
    } else {
        let first = candidate.with_day(1).unwrap_or(candidate);
        let last = first
            .with_day(days_in_month(candidate))
            .unwrap_or(candidate);
        ordinal_matches(ordinal, candidate, first, last)
    }
}

fn ordinal_matches(requested: i8, candidate: NaiveDate, first: NaiveDate, last: NaiveDate) -> bool {
    let first_offset = (candidate.weekday().num_days_from_monday() + 7
        - first.weekday().num_days_from_monday())
        % 7;
    let first_match = first + ChronoDuration::days(i64::from(first_offset));
    let positive = ((candidate - first_match).num_days() / 7) + 1;
    let last_offset = (last.weekday().num_days_from_monday() + 7
        - candidate.weekday().num_days_from_monday())
        % 7;
    let last_match = last - ChronoDuration::days(i64::from(last_offset));
    let negative = -(((last_match - candidate).num_days() / 7) + 1);
    i64::from(requested) == positive || i64::from(requested) == negative
}

fn month_day_matches(candidate: NaiveDate, requested: i8) -> bool {
    if requested > 0 {
        candidate.day() == u32::try_from(requested).unwrap_or_default()
    } else {
        let from_end = i32::try_from(days_in_month(candidate)).unwrap_or_default()
            - i32::try_from(candidate.day()).unwrap_or_default()
            + 1;
        from_end == -i32::from(requested)
    }
}

fn days_in_month(date: NaiveDate) -> u32 {
    let (year, month) = if date.month() == 12 {
        (date.year() + 1, 1)
    } else {
        (date.year(), date.month() + 1)
    };
    NaiveDate::from_ymd_opt(year, month, 1)
        .and_then(|first| first.pred_opt())
        .map_or(28, |last| last.day())
}

fn week_start(date: NaiveDate, start: Weekday) -> NaiveDate {
    let offset = (date.weekday().num_days_from_monday() + 7 - start.num_days_from_monday()) % 7;
    date - ChronoDuration::days(i64::from(offset))
}

fn resolve_local(timezone: Tz, local: NaiveDateTime) -> Option<DateTime<Tz>> {
    match timezone.from_local_datetime(&local) {
        LocalResult::Single(time) => Some(time),
        LocalResult::Ambiguous(first, second) => Some(first.min(second)),
        LocalResult::None => None,
    }
}

fn unescape_text(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            output.push(character);
            continue;
        }
        match characters.next() {
            Some('n' | 'N') => output.push('\n'),
            Some(escaped @ (',' | ';' | '\\')) => output.push(escaped),
            Some(other) => output.push(other),
            None => output.push('\\'),
        }
    }
    output
}

fn truncate_utf8(value: &str, maximum_bytes: usize) -> String {
    if value.len() <= maximum_bytes {
        return value.to_owned();
    }
    let mut end = maximum_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
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

    const BASIC: &str = include_str!("../tests/fixtures/basic.ics");
    const RECURRENCE_SUPPORTED: &str = include_str!("../tests/fixtures/recurrence-supported.ics");
    const RECURRENCE_UNSUPPORTED: &str =
        include_str!("../tests/fixtures/recurrence-unsupported.ics");
    const RECURRENCE_V1: &str = include_str!("../tests/fixtures/recurrence-v1.ics");
    const RECURRENCE_EXCEPTIONS: &str = include_str!("../tests/fixtures/recurrence-exceptions.ics");
    const MALFORMED: &str = include_str!("../tests/fixtures/malformed.ics");

    fn utc(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
            .single()
            .unwrap()
    }

    fn options() -> CalendarOptions {
        CalendarOptions {
            default_timezone: chrono_tz::America::New_York,
            display_timezone: chrono_tz::America::New_York,
            horizon: Duration::from_hours(120 * 24),
            ..CalendarOptions::default()
        }
    }

    #[test]
    fn parses_folded_timezone_utc_all_day_cancelled_and_duplicate_events() {
        let calendar = parse_ics(BASIC, utc(2026, 8, 4, 10, 0), &options()).unwrap();
        let titles: Vec<_> = calendar
            .events
            .iter()
            .map(|event| event.title.as_str())
            .collect();
        assert_eq!(
            titles,
            [
                "Team sync and planning",
                "Updated duplicate",
                "UTC review",
                "Launch day"
            ]
        );
        assert_eq!(calendar.events[0].start, utc(2026, 8, 4, 13, 0));
        assert!(calendar.events[3].all_day);
        assert_eq!(calendar.counters.cancelled_events, 1);
        assert_eq!(calendar.counters.duplicate_uids, 1);
        assert_eq!(calendar.counters.malformed_events, 0);
    }

    #[test]
    fn expands_the_supported_bounded_recurrence_subset_and_exdates() {
        let mut recurring_options = options();
        recurring_options.maximum_events = 5;
        let calendar = parse_ics(
            RECURRENCE_SUPPORTED,
            utc(2026, 8, 2, 0, 0),
            &recurring_options,
        )
        .unwrap();
        let starts: Vec<_> = calendar.events.iter().map(|event| event.start).collect();
        assert!(starts.contains(&utc(2026, 8, 2, 13, 0)));
        assert!(!starts.contains(&utc(2026, 8, 3, 13, 0)));
        assert!(starts.contains(&utc(2026, 8, 4, 13, 0)));
        assert_eq!(calendar.counters.unsupported_recurrences, 0);
        assert_eq!(calendar.counters.recurrence_scan_limit_hits, 0);
    }

    #[test]
    fn next_start_unix_ms_is_the_next_occurrence_not_the_recurring_series_start() {
        let now = utc(2026, 8, 2, 0, 0);
        let calendar = parse_ics(RECURRENCE_SUPPORTED, now, &options()).unwrap();
        // The "daily" series starts 2026-08-01 09:00 EDT (13:00 UTC), which is already
        // in the past relative to `now`. The next actual occurrence is the following
        // day, 2026-08-02 09:00 EDT (13:00 UTC): a different instant than the series
        // start, so this pins that the field reports the occurrence, not DTSTART.
        let series_start = utc(2026, 8, 1, 13, 0);
        let next_occurrence = utc(2026, 8, 2, 13, 0);
        assert_eq!(calendar.events[0].start, next_occurrence);
        assert_ne!(calendar.events[0].start, series_start);

        let fields = calendar.fields("Calendar", options().display_timezone, now, false, None);
        let next_start = fields
            .iter()
            .find(|field| field.key == "next_start_unix_ms")
            .expect("next_start_unix_ms field is present");
        assert_eq!(
            next_start.value,
            FieldValue::Integer(next_occurrence.timestamp_millis())
        );

        // Unlike `row0_time`, this field does not depend on `display_timezone`
        // at all: a wildly different zone must report the exact same integer.
        let fields_other_zone =
            calendar.fields("Calendar", chrono_tz::Asia::Tokyo, now, false, None);
        let next_start_other_zone = fields_other_zone
            .iter()
            .find(|field| field.key == "next_start_unix_ms")
            .expect("next_start_unix_ms field is present");
        assert_eq!(
            next_start_other_zone.value, next_start.value,
            "next_start_unix_ms must be UTC-derived and independent of display_timezone"
        );

        // No upcoming events: the field is `0`, never a stale or missing value.
        let empty_fields =
            IcsCalendar::default().fields("Calendar", options().display_timezone, now, false, None);
        let empty_next_start = empty_fields
            .iter()
            .find(|field| field.key == "next_start_unix_ms")
            .expect("next_start_unix_ms field is present even with no events");
        assert_eq!(empty_next_start.value, FieldValue::Integer(0));
    }

    #[test]
    fn skips_and_counts_every_out_of_scope_recurrence_form() {
        let calendar =
            parse_ics(RECURRENCE_UNSUPPORTED, utc(2026, 8, 1, 0, 0), &options()).unwrap();
        assert!(calendar.events.is_empty());
        assert_eq!(calendar.counters.unsupported_recurrences, 5);
    }

    #[test]
    fn expands_v1_month_year_ordinal_set_position_and_rdate_cases() {
        let calendar = parse_ics(RECURRENCE_V1, utc(2026, 8, 1, 0, 0), &options()).unwrap();
        let expected = [
            ("First Monday", utc(2026, 8, 3, 13, 0)),
            ("Extra date", utc(2026, 8, 4, 16, 0)),
            ("Anniversary", utc(2026, 8, 5, 15, 0)),
            ("Last weekday", utc(2026, 8, 31, 14, 0)),
            ("Month end", utc(2026, 8, 31, 13, 0)),
        ];
        for (title, start) in expected {
            assert!(
                calendar
                    .events
                    .iter()
                    .any(|event| { event.title == title && event.start == start }),
                "missing {title} at {start}"
            );
        }
        assert_eq!(calendar.counters.unsupported_recurrences, 0);
    }

    #[test]
    fn applies_timezone_aware_cancelled_and_moved_recurrence_exceptions() {
        let calendar =
            parse_ics(RECURRENCE_EXCEPTIONS, utc(2026, 10, 24, 0, 0), &options()).unwrap();
        let starts: Vec<_> = calendar.events.iter().map(|event| event.start).collect();
        assert_eq!(
            starts,
            [
                utc(2026, 10, 25, 13, 0),
                utc(2026, 11, 8, 16, 0),
                utc(2026, 11, 15, 14, 0),
            ]
        );
        assert_eq!(calendar.events[1].title, "Moved standup");
        assert_eq!(calendar.counters.cancelled_events, 1);
        assert_eq!(calendar.counters.unsupported_recurrences, 0);
        assert_eq!(calendar.counters.duplicate_uids, 0);
    }

    #[test]
    fn malformed_calendar_boundary_is_a_refresh_failure() {
        assert!(matches!(
            parse_ics(MALFORMED, utc(2026, 8, 1, 0, 0), &options()),
            Err(ProviderError::MalformedFeed(_))
        ));
    }

    #[test]
    fn system_loader_reads_file_sources_with_the_same_size_bound() {
        let path = std::env::temp_dir().join(format!(
            "deskmate-provider-fixture-{}.ics",
            std::process::id()
        ));
        std::fs::write(&path, BASIC).unwrap();
        let loaded = SystemIcsLoader
            .load(&IcsSource::File(path.clone()))
            .unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(loaded, BASIC);
    }

    #[derive(Default)]
    struct FakeLoader {
        results: VecDeque<Result<String, ProviderError>>,
    }

    impl IcsLoader for FakeLoader {
        fn load(&mut self, _source: &IcsSource) -> Result<String, ProviderError> {
            self.results
                .pop_front()
                .unwrap_or_else(|| Err(ProviderError::Io("no fixture".into())))
        }
    }

    #[test]
    fn refresh_failure_preserves_last_good_with_age_and_error_fields() {
        let mut loader = FakeLoader::default();
        loader.results.push_back(Ok(BASIC.into()));
        loader
            .results
            .push_back(Err(ProviderError::Io("offline".into())));
        let mut provider = IcsProvider::new(
            IcsSource::Url("https://example.test/a.ics".into()),
            loader,
            options(),
        );
        let first_time = utc(2026, 8, 4, 10, 0);
        let first = provider.refresh(first_time);
        assert!(!first.stale);
        let failed = provider.refresh(first_time + ChronoDuration::minutes(7));
        assert!(failed.stale);
        assert_eq!(failed.value.events, first.value.events);
        assert_eq!(failed.age, Some(Duration::from_mins(7)));
        let fields = provider.fields(&failed, first_time + ChronoDuration::minutes(7));
        assert_eq!(fields.len(), 14);
        assert!(
            fields
                .iter()
                .any(|field| { field.key == "stale" && field.value == FieldValue::Boolean(true) })
        );
        assert!(fields.iter().any(|field| {
            field.key == "error"
                && matches!(&field.value, FieldValue::Text(message) if message.contains("offline"))
        }));
    }
}
