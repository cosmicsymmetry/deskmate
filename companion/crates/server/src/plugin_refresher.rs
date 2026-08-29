//! Provider refresher that adds curated plugin data to app-core's system providers.

use std::collections::HashMap;
use std::sync::Arc;

use app_core::{
    ProviderRefreshRequest, ProviderRefreshResult, ProviderRefresher, ProviderRequest,
    SystemProviderRefresher,
};
use protocol::{Field, FieldValue};
use providers::Provider as _;

use crate::plugin_provider::{PluginDataProvider, PluginFetcher, SystemPluginFetcher};
use crate::plugin_registry::PluginRegistry;

/// Delegates built-in providers unchanged and serves plugin providers from a registry.
pub struct ServerProviderRefresher<F: PluginFetcher = SystemPluginFetcher> {
    system: SystemProviderRefresher,
    registry: Arc<PluginRegistry>,
    plugins: HashMap<String, PluginProviderEntry<F>>,
    fetcher_factory: Box<dyn FnMut() -> F + Send>,
}

struct PluginProviderEntry<F: PluginFetcher> {
    title: String,
    request: ProviderRequest,
    provider: PluginDataProvider<F>,
}

impl ServerProviderRefresher<SystemPluginFetcher> {
    #[must_use]
    pub fn new(registry: Arc<PluginRegistry>) -> Self {
        Self::with_fetcher_factory(registry, || SystemPluginFetcher)
    }
}

impl<F> ServerProviderRefresher<F>
where
    F: PluginFetcher + Send + 'static,
{
    fn with_fetcher_factory(
        registry: Arc<PluginRegistry>,
        fetcher_factory: impl FnMut() -> F + Send + 'static,
    ) -> Self {
        Self {
            system: SystemProviderRefresher::default(),
            registry,
            plugins: HashMap::new(),
            fetcher_factory: Box::new(fetcher_factory),
        }
    }
}

impl<F> ProviderRefresher for ServerProviderRefresher<F>
where
    F: PluginFetcher + Send + 'static,
{
    fn refresh(&mut self, request: ProviderRefreshRequest) -> ProviderRefreshResult {
        let plugin_id = match &request.provider {
            ProviderRequest::Plugin { plugin_id, .. } => plugin_id.clone(),
            _ => return self.system.refresh(request),
        };

        self.plugins
            .retain(|widget_id, _| request.active_provider_ids.contains(widget_id));

        let needs_replacement = self
            .plugins
            .get(&request.widget_id)
            .is_none_or(|entry| entry.title != request.title || entry.request != request.provider);
        if needs_replacement {
            let Some(plugin) = self.registry.get(&plugin_id) else {
                return plugin_error_result(request, format!("plugin {plugin_id:?} is not loaded"));
            };
            let provider =
                match PluginDataProvider::new((self.fetcher_factory)(), &plugin.manifest.source) {
                    Ok(provider) => provider,
                    Err(error) => {
                        return plugin_error_result(
                            request,
                            format!("plugin {plugin_id:?} provider cannot start: {error}"),
                        );
                    }
                };
            self.plugins.insert(
                request.widget_id.clone(),
                PluginProviderEntry {
                    title: request.title.clone(),
                    request: request.provider.clone(),
                    provider,
                },
            );
        }

        let entry = self
            .plugins
            .get_mut(&request.widget_id)
            .expect("plugin provider entry was inserted above");
        let snapshot = entry.provider.refresh(request.now);
        ProviderRefreshResult {
            generation: request.generation,
            widget_id: request.widget_id,
            fields: vec![title_field(request.title)],
            value: Some(snapshot.value),
            refreshed_at: snapshot.refreshed_at,
            age: snapshot.age,
            stale: snapshot.stale,
            error: snapshot.error,
        }
    }
}

fn title_field(title: String) -> Field {
    Field {
        key: "title".into(),
        value: FieldValue::Text(title),
    }
}

fn plugin_error_result(request: ProviderRefreshRequest, error: String) -> ProviderRefreshResult {
    ProviderRefreshResult {
        generation: request.generation,
        widget_id: request.widget_id,
        fields: vec![title_field(request.title)],
        value: None,
        refreshed_at: None,
        age: None,
        stale: true,
        error: Some(error),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use app_core::CalendarSource;
    use chrono::{TimeZone as _, Utc};

    use super::*;
    use crate::egress::{EgressError, FetchResponse};

    const AQI_FIXTURE: &[u8] = include_bytes!("../../plugin/tests/fixtures/aqi_response.json");

    #[derive(Clone)]
    struct FakeFetcher {
        responses: Arc<Mutex<VecDeque<Result<FetchResponse, EgressError>>>>,
    }

    impl PluginFetcher for FakeFetcher {
        fn fetch(&self, _url: &str) -> Result<FetchResponse, EgressError> {
            self.responses
                .lock()
                .expect("fake response lock")
                .pop_front()
                .expect("fake fetcher exhausted")
        }
    }

    fn ok_response(body: &[u8]) -> FetchResponse {
        FetchResponse {
            status: 200,
            body: body.to_vec(),
        }
    }

    fn curated_plugins_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins")
    }

    fn registry() -> Arc<PluginRegistry> {
        let (registry, failures) =
            PluginRegistry::load(&curated_plugins_dir()).expect("load curated registry");
        assert!(failures.is_empty());
        Arc::new(registry)
    }

    fn plugin_request(widget_id: &str, active_provider_ids: &[&str]) -> ProviderRefreshRequest {
        ProviderRefreshRequest {
            generation: 7,
            widget_id: widget_id.into(),
            title: "Air quality".into(),
            provider: ProviderRequest::Plugin {
                plugin_id: "aqi".into(),
                refresh_interval: Duration::from_mins(15),
            },
            active_provider_ids: active_provider_ids
                .iter()
                .map(|id| (*id).to_string())
                .collect(),
            now: Utc.with_ymd_and_hms(2026, 8, 29, 12, 0, 0).unwrap(),
        }
    }

    fn refresher_with(
        responses: Vec<Result<FetchResponse, EgressError>>,
    ) -> ServerProviderRefresher<FakeFetcher> {
        let fetcher = FakeFetcher {
            responses: Arc::new(Mutex::new(responses.into())),
        };
        ServerProviderRefresher::with_fetcher_factory(registry(), move || fetcher.clone())
    }

    #[test]
    fn successful_fetch_returns_raw_value_and_title_field() {
        let mut refresher = refresher_with(vec![Ok(ok_response(AQI_FIXTURE))]);

        let result = refresher.refresh(plugin_request("aqi-card", &["aqi-card"]));

        assert!(!result.stale, "successful fetch was marked stale");
        assert_eq!(
            result.fields,
            vec![Field {
                key: "title".into(),
                value: FieldValue::Text("Air quality".into()),
            }]
        );
        let value = result.value.expect("plugin value");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["payload"]["current"]["aqi"], 42);
        assert!(
            value.get("current").is_none(),
            "production must keep the raw HTTP body; envelope unwrapping was added silently"
        );
    }

    #[test]
    fn transient_failure_keeps_cache_and_next_success_clears_stale_state() {
        let mut refresher = refresher_with(vec![
            Ok(ok_response(br#"{"current":{"aqi":42}}"#)),
            Err(EgressError::Timeout),
            Ok(ok_response(br#"{"current":{"aqi":17}}"#)),
        ]);
        let request = plugin_request("aqi-card", &["aqi-card"]);

        let first = refresher.refresh(request.clone());
        assert!(!first.stale);
        let stale = refresher.refresh(request.clone());
        assert!(stale.stale);
        assert!(
            stale
                .error
                .as_deref()
                .is_some_and(|error| !error.is_empty())
        );
        assert_eq!(
            stale.value, first.value,
            "transient error lost last-good data"
        );

        let recovered = refresher.refresh(request);
        assert!(!recovered.stale);
        assert!(recovered.error.is_none());
        assert_eq!(recovered.value.unwrap()["current"]["aqi"], 17);
        assert_eq!(refresher.plugins.len(), 1, "provider cache was poisoned");
    }

    #[test]
    fn non_plugin_request_is_delegated_unchanged() {
        let request = ProviderRefreshRequest {
            generation: 11,
            widget_id: "calendar".into(),
            title: "Agenda".into(),
            provider: ProviderRequest::Calendar {
                source: CalendarSource::File("/definitely/not/a/real/deskmate-calendar.ics".into()),
                timezone: "UTC".parse().expect("UTC timezone"),
                refresh_interval: Duration::from_mins(5),
            },
            active_provider_ids: vec!["calendar".into()],
            now: Utc.with_ymd_and_hms(2026, 8, 29, 12, 0, 0).unwrap(),
        };
        let mut direct = SystemProviderRefresher::default();
        let expected = direct.refresh(request.clone());
        let mut wrapped = refresher_with(Vec::new());

        let actual = wrapped.refresh(request);

        assert_eq!(actual.generation, expected.generation);
        assert_eq!(actual.widget_id, expected.widget_id);
        assert_eq!(actual.fields, expected.fields);
        assert_eq!(actual.value, expected.value);
        assert_eq!(actual.refreshed_at, expected.refreshed_at);
        assert_eq!(actual.age, expected.age);
        assert_eq!(actual.stale, expected.stale);
        assert_eq!(actual.error, expected.error);
    }

    #[test]
    fn plugin_provider_entries_are_pruned_by_active_widget_ids() {
        let mut refresher = refresher_with(vec![
            Ok(ok_response(br#"{"current":{"aqi":42}}"#)),
            Ok(ok_response(br#"{"current":{"aqi":17}}"#)),
        ]);
        refresher.refresh(plugin_request("first", &["first"]));
        assert!(refresher.plugins.contains_key("first"));

        refresher.refresh(plugin_request("second", &["second"]));

        assert_eq!(refresher.plugins.len(), 1);
        assert!(refresher.plugins.contains_key("second"));
        assert!(!refresher.plugins.contains_key("first"));
    }

    #[test]
    fn title_or_provider_request_changes_replace_the_cached_provider() {
        let fetcher = FakeFetcher {
            responses: Arc::new(Mutex::new(
                vec![
                    Ok(ok_response(br#"{"current":{"aqi":42}}"#)),
                    Ok(ok_response(br#"{"current":{"aqi":43}}"#)),
                    Ok(ok_response(br#"{"current":{"aqi":44}}"#)),
                ]
                .into(),
            )),
        };
        let constructions = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&constructions);
        let mut refresher = ServerProviderRefresher::with_fetcher_factory(registry(), move || {
            counted.fetch_add(1, Ordering::Relaxed);
            fetcher.clone()
        });
        let first = plugin_request("aqi-card", &["aqi-card"]);
        refresher.refresh(first.clone());

        let mut renamed = first.clone();
        renamed.title = "Outside air".into();
        refresher.refresh(renamed.clone());

        let mut rescheduled = renamed;
        rescheduled.provider = ProviderRequest::Plugin {
            plugin_id: "aqi".into(),
            refresh_interval: Duration::from_mins(30),
        };
        refresher.refresh(rescheduled);

        assert_eq!(
            constructions.load(Ordering::Relaxed),
            3,
            "title/request changes reused a stale provider cache entry"
        );
    }
}
