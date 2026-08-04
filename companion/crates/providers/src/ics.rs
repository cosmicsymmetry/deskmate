use std::collections::HashMap;
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

use crate::{Provider, ProviderError, ProviderSnapshot, RefreshPolicy};

pub const MAX_ICS_BYTES: usize = 1_048_576;
pub const MAX_ICS_EVENTS: usize = 4_096;
pub const MAX_UNFOLDED_LINE_BYTES: usize = 8_192;
pub const MAX_CALENDAR_ROWS: usize = 5;
const MAX_RECURRENCE_DAYS: i64 = 100_000;
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

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
                    return Err(ProviderError::FeedTooLarge);
                }
                String::from_utf8(bytes)
                    .map_err(|_| ProviderError::MalformedFeed("feed is not UTF-8".into()))
            }
            IcsSource::Url(url) => {
                let agent: ureq::Agent = ureq::Agent::config_builder()
                    .timeout_global(Some(HTTP_TIMEOUT))
                    .build()
                    .into();
                let mut response = agent
                    .get(url)
                    .call()
                    .map_err(|error| ProviderError::Http(error.to_string()))?;
                let text = response
                    .body_mut()
                    .with_config()
                    .limit((MAX_ICS_BYTES + 1) as u64)
                    .lossy_utf8(false)
                    .read_to_string()
                    .map_err(|error| match error {
                        ureq::Error::BodyExceedsLimit(_) => ProviderError::FeedTooLarge,
                        other => ProviderError::Http(other.to_string()),
                    })?;
                if text.len() > MAX_ICS_BYTES {
                    return Err(ProviderError::FeedTooLarge);
                }
                Ok(text)
            }
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
        let mut fields = Vec::with_capacity(13);
        fields.push(text_field("title", truncate_utf8(title, 64)));
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
    last_good: Option<IcsCalendar>,
    last_success: Option<DateTime<Utc>>,
}

impl<L: IcsLoader> IcsProvider<L> {
    pub fn new(source: IcsSource, loader: L, options: CalendarOptions) -> Self {
        Self {
            source,
            loader,
            options,
            last_good: None,
            last_success: None,
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
        match result {
            Ok(calendar) => {
                self.last_good = Some(calendar.clone());
                self.last_success = Some(now);
                ProviderSnapshot {
                    value: calendar,
                    refreshed_at: Some(now),
                    age: Some(Duration::ZERO),
                    stale: false,
                    error: None,
                }
            }
            Err(error) => {
                let age = self
                    .last_success
                    .and_then(|success| now.signed_duration_since(success).to_std().ok());
                ProviderSnapshot {
                    value: self.last_good.clone().unwrap_or_default(),
                    refreshed_at: self.last_success,
                    age,
                    stale: true,
                    error: Some(truncate_utf8(&error.to_string(), 96)),
                }
            }
        }
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
}

#[derive(Debug, Clone)]
struct RecurrenceRule {
    frequency: Frequency,
    interval: u32,
    count: Option<u32>,
    until: Option<DateTime<Utc>>,
    weekdays: Vec<Weekday>,
}

#[derive(Debug, Clone)]
struct ParsedEvent {
    uid: String,
    title: String,
    start: EventTime,
    duration: ChronoDuration,
    recurrence: Option<RecurrenceRule>,
    exclusions: Vec<DateTime<Utc>>,
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
    for component in selected {
        if component
            .property("STATUS")
            .is_some_and(|property| property.value.eq_ignore_ascii_case("CANCELLED"))
        {
            counters.cancelled_events = counters.cancelled_events.saturating_add(1);
            continue;
        }
        match parse_event(&component, options.default_timezone) {
            Ok(event) => expand_event(&event, now, horizon, &mut events, &mut counters),
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
        return Err(ProviderError::FeedTooLarge);
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
        if let Some(index) = uid_indexes.get(&uid).copied() {
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
            uid_indexes.insert(uid, selected.len());
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
    if component.property("RECURRENCE-ID").is_some() {
        return Err(EventParseError::UnsupportedRecurrence);
    }
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
        .map(|property| parse_recurrence(&property.value, default_timezone))
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
    Ok(ParsedEvent {
        uid,
        title,
        start,
        duration,
        recurrence,
        exclusions,
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

fn parse_recurrence(raw: &str, default_timezone: Tz) -> Result<RecurrenceRule, EventParseError> {
    let mut frequency = None;
    let mut interval = 1_u32;
    let mut count = None;
    let mut until = None;
    let mut weekdays = Vec::new();
    for part in raw.split(';') {
        let (key, value) = part
            .split_once('=')
            .ok_or(EventParseError::UnsupportedRecurrence)?;
        match key {
            "FREQ" => {
                frequency = Some(match value {
                    "DAILY" => Frequency::Daily,
                    "WEEKLY" => Frequency::Weekly,
                    "MONTHLY" => Frequency::Monthly,
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
                    if day.len() != 2 {
                        return Err(EventParseError::UnsupportedRecurrence);
                    }
                    weekdays.push(match day {
                        "MO" => Weekday::Mon,
                        "TU" => Weekday::Tue,
                        "WE" => Weekday::Wed,
                        "TH" => Weekday::Thu,
                        "FR" => Weekday::Fri,
                        "SA" => Weekday::Sat,
                        "SU" => Weekday::Sun,
                        _ => return Err(EventParseError::UnsupportedRecurrence),
                    });
                }
                weekdays.sort_unstable_by_key(Weekday::num_days_from_monday);
                weekdays.dedup();
            }
            _ => return Err(EventParseError::UnsupportedRecurrence),
        }
    }
    if count.is_some() && until.is_some() {
        return Err(EventParseError::Malformed);
    }
    Ok(RecurrenceRule {
        frequency: frequency.ok_or(EventParseError::Malformed)?,
        interval,
        count,
        until,
        weekdays,
    })
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
}

fn recurrence_matches(rule: &RecurrenceRule, start: NaiveDate, candidate: NaiveDate) -> bool {
    if candidate == start {
        return true;
    }
    let days = (candidate - start).num_days();
    if days < 0 {
        return false;
    }
    match rule.frequency {
        Frequency::Daily => {
            days % i64::from(rule.interval) == 0
                && (rule.weekdays.is_empty() || rule.weekdays.contains(&candidate.weekday()))
        }
        Frequency::Weekly => {
            let start_week =
                start - ChronoDuration::days(i64::from(start.weekday().num_days_from_monday()));
            let candidate_week = candidate
                - ChronoDuration::days(i64::from(candidate.weekday().num_days_from_monday()));
            let weeks = (candidate_week - start_week).num_days() / 7;
            weeks % i64::from(rule.interval) == 0
                && if rule.weekdays.is_empty() {
                    candidate.weekday() == start.weekday()
                } else {
                    rule.weekdays.contains(&candidate.weekday())
                }
        }
        Frequency::Monthly => {
            let months = (candidate.year() - start.year()) * 12
                + i32::try_from(candidate.month()).unwrap_or(0)
                - i32::try_from(start.month()).unwrap_or(0);
            months >= 0
                && months % i32::try_from(rule.interval).unwrap_or(i32::MAX) == 0
                && if rule.weekdays.is_empty() {
                    candidate.day() == start.day()
                } else {
                    rule.weekdays.contains(&candidate.weekday())
                }
        }
    }
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
    fn skips_and_counts_every_out_of_scope_recurrence_form() {
        let calendar =
            parse_ics(RECURRENCE_UNSUPPORTED, utc(2026, 8, 1, 0, 0), &options()).unwrap();
        assert!(calendar.events.is_empty());
        assert_eq!(calendar.counters.unsupported_recurrences, 5);
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
            .push_back(Err(ProviderError::Http("offline".into())));
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
        assert_eq!(fields.len(), 13);
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
