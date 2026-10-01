//! Account sources share one frame. Config v10 stores zones per device, so prefer
//! the first active device displaying this source (stable device-id order). Before
//! a new source is attached to a card, use the first active saved config, then UTC.
//! Reading on each render/poll means a saved timezone change needs no restart.
use std::sync::Arc;

use app_core::{CardSettings, ConfigOrigin};
use chrono::{DateTime, NaiveDate, Utc};

use crate::ServerState;
use crate::accounts::AccountSpace;
use crate::identity::DeviceState;

pub(super) fn source_timezone(state: &ServerState, space: &AccountSpace, source: &str) -> String {
    let mut fallback = None;
    for device in state
        .identity()
        .devices_for(&space.account_id)
        .unwrap_or_default()
    {
        if device.state != DeviceState::Active {
            continue;
        }
        let loaded = space.configs.for_device(&device.device_id).store.load();
        // Defaults may be seeded from the SERVER's zone. Only saved or last-good
        // owner settings count here; never silently make that seed authoritative.
        if loaded.origin() == ConfigOrigin::Defaults {
            continue;
        }
        let config = loaded.config();
        let zone = &config.preferences.timezone;
        fallback.get_or_insert_with(|| zone.clone());
        if config.cards.iter().any(
            |card| matches!(card, CardSettings::Picture { source_id, .. } if source_id == source),
        ) {
            return zone.clone();
        }
    }
    fallback.unwrap_or_else(|| "UTC".into())
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Calendar {
    timezone: String,
    date: NaiveDate,
}

impl Calendar {
    fn at(timezone: String, now: DateTime<Utc>) -> Self {
        let offset = app_core::utc_offset_minutes(&timezone, now).unwrap_or(0);
        Self {
            timezone,
            date: (now + chrono::Duration::minutes(i64::from(offset))).date_naive(),
        }
    }
}

/// Failed renders keep their retry policy; successful ones may refresh at the
/// next date/zone change, without violating the scheduler's one-minute floor.
pub(super) fn should_refresh(
    drawn: bool,
    elapsed: std::time::Duration,
    before: &Calendar,
    after: &Calendar,
) -> bool {
    drawn && elapsed >= super::worker::MIN_REFRESH && before != after
}

pub(super) async fn sample(
    state: &ServerState,
    space: &Arc<AccountSpace>,
    source: &str,
    now: &(dyn Fn() -> DateTime<Utc> + Send + Sync),
) -> Calendar {
    let state = state.clone();
    let space = Arc::clone(space);
    let source = source.to_owned();
    let zone = tokio::task::spawn_blocking(move || source_timezone(&state, &space, &source))
        .await
        .unwrap_or_else(|_| "UTC".into());
    Calendar::at(zone, now())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    fn calendar(zone: &str, instant: &str) -> Calendar {
        Calendar::at(zone.into(), instant.parse().unwrap())
    }

    #[test]
    fn local_dates_change_on_both_sides_of_utc_and_across_leap_years() {
        for (zone, before, after) in [
            ("Asia/Tokyo", "2027-12-31T14:59:59Z", "2027-12-31T15:00:00Z"),
            (
                "America/Los_Angeles",
                "2028-01-01T07:59:59Z",
                "2028-01-01T08:00:00Z",
            ),
            ("Asia/Tokyo", "2028-02-28T14:59:59Z", "2028-02-28T15:00:00Z"),
            // Sao Paulo skipped midnight when DST began: the first new date was 01:00.
            (
                "America/Sao_Paulo",
                "2018-11-04T02:59:59Z",
                "2018-11-04T03:00:00Z",
            ),
            // Samoa skipped a whole calendar date.
            (
                "Pacific/Apia",
                "2011-12-30T09:59:59Z",
                "2011-12-30T10:00:00Z",
            ),
        ] {
            assert_ne!(calendar(zone, before), calendar(zone, after), "{zone}");
        }
        assert_eq!(
            calendar("America/Los_Angeles", "2026-03-08T09:59:59Z"),
            calendar("America/Los_Angeles", "2026-03-08T10:00:00Z"),
            "an ordinary DST clock jump is not a date change"
        );
        assert_eq!(
            calendar("Asia/Tokyo", "2027-12-31T23:59:59Z"),
            calendar("Asia/Tokyo", "2028-01-01T00:00:00Z"),
            "UTC midnight is irrelevant"
        );
        assert_ne!(
            calendar("UTC", "2026-01-01T12:00:00Z"),
            calendar("Asia/Tokyo", "2026-01-01T12:00:00Z")
        );
    }

    #[test]
    fn midnight_wakeup_preserves_the_floor_and_failure_backoff() {
        let before = calendar("Asia/Tokyo", "2027-12-31T14:59:59Z");
        let after = calendar("Asia/Tokyo", "2027-12-31T15:00:00Z");
        let minute = std::time::Duration::from_secs(60);
        assert!(should_refresh(true, minute, &before, &after));
        assert!(!should_refresh(false, minute, &before, &after));
        assert!(!should_refresh(
            true,
            std::time::Duration::from_secs(59),
            &before,
            &after
        ));
        assert!(!should_refresh(true, minute, &after, &after));
    }

    pub(crate) fn save_device(
        state: &ServerState,
        space: &AccountSpace,
        id: &str,
        zone: &str,
        source: Option<&str>,
    ) {
        if state.identity().device_owner(id).unwrap().is_none() {
            state
                .identity()
                .assign_device(id, &space.account_id, DeviceState::Active, Utc::now())
                .unwrap();
        }
        let mut config = app_core::AppConfig::default();
        config.preferences.timezone = zone.into();
        if let Some(source) = source {
            config.image_sources =
                serde_json::from_value(serde_json::json!([{"id": source, "name": "Test"}]))
                    .unwrap();
            config.cards = vec![
                serde_json::from_value(serde_json::json!({
                    "kind": "picture", "id": "test", "title": "Test", "source_id": source,
                    "tap_action": { "kind": "none" }, "refresh": { "kind": "manual" },
                    "alert": { "kind": "none" }, "dwell_seconds": null
                }))
                .unwrap(),
            ];
        }
        space.configs.for_device(id).store.save(&config).unwrap();
    }

    #[tokio::test]
    async fn pending_and_unsaved_devices_cannot_supply_a_timezone() {
        let state = ServerState::in_memory();
        let account = state
            .identity()
            .create_account("a@example.com", true, true, Utc::now())
            .unwrap();
        let space = state.account_space(&account.id);
        state
            .identity()
            .assign_device("dev-0000", &account.id, DeviceState::Active, Utc::now())
            .unwrap();
        let configured = if app_core::AppConfig::default().preferences.timezone == "Asia/Tokyo" {
            "UTC"
        } else {
            "Asia/Tokyo"
        };
        save_device(&state, &space, "dev-0002", configured, None);
        assert_eq!(
            source_timezone(&state, &space, "unattached"),
            configured,
            "unsaved defaults do not win over owner settings"
        );
        state
            .identity()
            .assign_device("dev-0001", &account.id, DeviceState::Pending, Utc::now())
            .unwrap();
        save_device(
            &state,
            &space,
            "dev-0001",
            "America/Los_Angeles",
            Some("source"),
        );
        assert_eq!(
            source_timezone(&state, &space, "source"),
            configured,
            "a pending device is not a consumer"
        );
        state.shutdown();
    }

    #[tokio::test]
    async fn source_clock_follows_saved_owner_settings_and_never_another_account() {
        let state = ServerState::in_memory();
        let account = state
            .identity()
            .create_account("a@example.com", true, true, Utc::now())
            .unwrap();
        let space = state.account_space(&account.id);
        assert_eq!(source_timezone(&state, &space, "source"), "UTC");
        save_device(&state, &space, "dev-0001", "Asia/Tokyo", None);
        assert_eq!(source_timezone(&state, &space, "source"), "Asia/Tokyo");
        save_device(
            &state,
            &space,
            "dev-0002",
            "America/Los_Angeles",
            Some("source"),
        );
        assert_eq!(
            source_timezone(&state, &space, "source"),
            "America/Los_Angeles"
        );
        save_device(&state, &space, "dev-0002", "Europe/London", Some("source"));
        assert_eq!(source_timezone(&state, &space, "source"), "Europe/London");
        save_device(&state, &space, "dev-0001", "Asia/Tokyo", Some("source"));
        assert_eq!(
            source_timezone(&state, &space, "source"),
            "Asia/Tokyo",
            "shared source has a stable first consumer"
        );
        let other = state
            .identity()
            .create_account("b@example.com", true, true, Utc::now())
            .unwrap();
        let other_space = state.account_space(&other.id);
        save_device(
            &state,
            &other_space,
            "dev-0000",
            "Pacific/Auckland",
            Some("source"),
        );
        assert_eq!(source_timezone(&state, &space, "source"), "Asia/Tokyo");
        assert_eq!(
            source_timezone(&state, &other_space, "source"),
            "Pacific/Auckland"
        );
        state.shutdown();
    }
}
