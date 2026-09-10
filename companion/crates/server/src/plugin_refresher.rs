//! Provider refresher that adds curated plugin data to app-core's system providers.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use app_core::{ProviderRefreshRequest, ProviderRefreshResult, ProviderRefresher, ProviderRequest};
use protocol::{Field, FieldValue};
use providers::Provider as _;

use crate::plugin_provider::{PluginDataProvider, PluginFetcher, SystemPluginFetcher};
use crate::plugin_registry::PluginRegistry;

/// Serves plugin providers from the curated registry.
pub struct ServerProviderRefresher<F: PluginFetcher = SystemPluginFetcher> {
    registry: Arc<PluginRegistry>,
    plugins: HashMap<String, PluginProviderEntry<F>>,
    fetcher_factory: Box<dyn FnMut() -> F + Send>,
    /// Plugin ids whose `summary` has already been reported as failing. A
    /// broken expression fails every cadence, forever; one line is a defect
    /// report, one line every ten minutes is a log flood.
    summary_failures: HashSet<String>,
}

struct PluginProviderEntry<F: PluginFetcher> {
    plugin_id: String,
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
            registry,
            plugins: HashMap::new(),
            fetcher_factory: Box::new(fetcher_factory),
            summary_failures: HashSet::new(),
        }
    }

    /// The manifest's `summary`, evaluated against what was just fetched.
    /// A failure costs the tile its headline and nothing else: the fetched
    /// data, the freshness state and the title all still reach the card.
    fn evaluate_summary(
        &mut self,
        plugin_id: &str,
        snapshot: &providers::ProviderSnapshot<serde_json::Value>,
    ) -> Option<String> {
        let registry = Arc::clone(&self.registry);
        let plugin = registry.get(plugin_id)?;
        match plugin::evaluate_summary(&plugin.manifest, snapshot) {
            Ok(summary) => summary,
            Err(error) => {
                if self.summary_failures.insert(plugin_id.to_owned()) {
                    tracing::warn!(
                        plugin_id,
                        %error,
                        "plugin summary could not be evaluated; the card tile will show no headline"
                    );
                }
                None
            }
        }
    }
}

impl<F> ProviderRefresher for ServerProviderRefresher<F>
where
    F: PluginFetcher + Send + 'static,
{
    fn refresh(&mut self, request: ProviderRefreshRequest) -> ProviderRefreshResult {
        let ProviderRequest::Plugin { plugin_id } = &request.provider;
        let plugin_id = plugin_id.clone();

        self.plugins
            .retain(|widget_id, _| request.active_provider_ids.contains(widget_id));

        let needs_replacement = self
            .plugins
            .get(&request.widget_id)
            .is_none_or(|entry| entry.plugin_id != plugin_id);
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
                    // Cloned, not moved: `plugin_id` is still needed below to
                    // evaluate the summary, and this arm is conditional.
                    plugin_id: plugin_id.clone(),
                    provider,
                },
            );
        }

        // Scoped so the `&mut self.plugins` borrow ends before
        // `evaluate_summary` takes `&mut self`.
        let snapshot = {
            let entry = self
                .plugins
                .get_mut(&request.widget_id)
                .expect("plugin provider entry was inserted above");
            entry.provider.refresh(request.now)
        };
        let hero = self.evaluate_summary(&plugin_id, &snapshot);
        let mut fields = vec![title_field(request.title)];
        fields.extend(hero.map(hero_field));
        ProviderRefreshResult {
            generation: request.generation,
            widget_id: request.widget_id,
            fields,
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

/// The tile's live value. This rides the wire in the card's `PushData` and
/// the device drops it: a plugin card's `WidgetConfig.template` is always
/// `TemplateKind::DigitalClock` (`app-core/src/config.rs`'s `wire_config`),
/// whose registry declares only `title`/`show_seconds`/`stale`/`error`, and
/// `firmware/main/core/template_fields.h:59` states that unknown fields are
/// ignored -- so it is safe to send. The field exists for the Mac's tile,
/// which reads it out of `card_data`.
fn hero_field(summary: String) -> Field {
    Field {
        key: "hero".into(),
        value: FieldValue::Text(summary),
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
    use chrono::{TimeZone as _, Utc};
    use std::collections::VecDeque;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

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
            },
            active_provider_ids: active_provider_ids
                .iter()
                .map(|id| (*id).to_string())
                .collect(),
            now: Utc.with_ymd_and_hms(2026, 8, 29, 12, 0, 0).unwrap(),
        }
    }

    fn refresher_with_registry(
        registry: Arc<PluginRegistry>,
        responses: Vec<Result<FetchResponse, EgressError>>,
    ) -> ServerProviderRefresher<FakeFetcher> {
        let fetcher = FakeFetcher {
            responses: Arc::new(Mutex::new(responses.into())),
        };
        ServerProviderRefresher::with_fetcher_factory(registry, move || fetcher.clone())
    }

    fn refresher_with(
        responses: Vec<Result<FetchResponse, EgressError>>,
    ) -> ServerProviderRefresher<FakeFetcher> {
        refresher_with_registry(registry(), responses)
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
    fn title_or_card_cadence_edits_keep_last_good_until_the_plugin_id_changes() {
        let fetcher = FakeFetcher {
            responses: Arc::new(Mutex::new(
                vec![
                    Ok(ok_response(br#"{"current":{"aqi":42}}"#)),
                    Err(EgressError::Timeout),
                    Err(EgressError::Timeout),
                    Err(EgressError::Timeout),
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
        let good = refresher.refresh(first.clone());

        let mut renamed = first.clone();
        renamed.title = "Outside air".into();
        let after_title_edit = refresher.refresh(renamed.clone());
        assert_eq!(after_title_edit.value, good.value);
        // The cached payload is still `{"current":{"aqi":42}}` and the curated
        // `aqi` manifest's summary reads exactly that, so a last-good refresh
        // publishes a last-good headline too.
        assert_eq!(
            after_title_edit.fields,
            vec![title_field("Outside air".into()), hero_field("42".into())]
        );
        assert!(after_title_edit.stale);

        // Card cadence belongs to app-core's scheduler and is no longer part
        // of ProviderRequest::Plugin, so a cadence-only edit reaches this
        // cache with the same provider identity.
        let mut after_cadence_edit = renamed.clone();
        after_cadence_edit.now += chrono::Duration::minutes(15);
        let after_cadence_edit = refresher.refresh(after_cadence_edit);
        assert_eq!(after_cadence_edit.value, good.value);
        assert!(after_cadence_edit.stale);

        let mut changed_plugin = renamed;
        changed_plugin.provider = ProviderRequest::Plugin {
            plugin_id: "svg-aqi".into(),
        };
        let after_plugin_edit = refresher.refresh(changed_plugin);
        assert_eq!(after_plugin_edit.value, Some(serde_json::Value::Null));
        assert_eq!(
            constructions.load(Ordering::Relaxed),
            2,
            "only a plugin-id change may replace the cached provider"
        );
    }

    fn fixture_request(widget_id: &str) -> ProviderRefreshRequest {
        ProviderRefreshRequest {
            generation: 7,
            widget_id: widget_id.into(),
            title: "Air quality".into(),
            provider: ProviderRequest::Plugin {
                plugin_id: "fixture".into(),
            },
            active_provider_ids: vec![widget_id.into()],
            now: Utc.with_ymd_and_hms(2026, 8, 29, 12, 0, 0).unwrap(),
        }
    }

    #[test]
    fn a_declared_summary_is_published_as_the_hero_field_after_the_title() {
        let (registry, _base) =
            crate::test_plugins::fixture_registry(crate::test_plugins::V2_NAMED_MANIFEST);
        let mut refresher = refresher_with_registry(
            registry,
            vec![Ok(ok_response(br#"{"current":{"aqi":42}}"#))],
        );

        let result = refresher.refresh(fixture_request("fixture-card"));

        // Order matters only in that `title` keeps its existing position:
        // `hero` is additive, and a reader that only knows `title` is unharmed.
        assert_eq!(
            result.fields,
            vec![
                Field {
                    key: "title".into(),
                    value: FieldValue::Text("Air quality".into()),
                },
                Field {
                    key: "hero".into(),
                    value: FieldValue::Text("42".into()),
                },
            ]
        );
    }

    #[test]
    fn a_manifest_without_a_summary_publishes_the_title_alone() {
        // The tile then shows "—": an absent optional summary is not a fault.
        let (registry, _base) =
            crate::test_plugins::fixture_registry(crate::test_plugins::V2_UNNAMED_MANIFEST);
        let mut refresher = refresher_with_registry(
            registry,
            vec![Ok(ok_response(br#"{"current":{"aqi":42}}"#))],
        );

        let result = refresher.refresh(fixture_request("fixture-card"));

        assert_eq!(result.fields, vec![title_field("Air quality".into())]);
    }

    #[test]
    fn a_summary_that_fails_to_evaluate_drops_the_hero_rather_than_the_refresh() {
        let (registry, _base) =
            crate::test_plugins::fixture_registry(crate::test_plugins::V2_NAMED_MANIFEST);
        // One byte past `expr::MAX_OUTPUT_LEN` is `ExprError::OutputTooLong`,
        // which the evaluator rejects rather than truncating.
        let oversized = format!(
            r#"{{"current":{{"aqi":"{}"}}}}"#,
            "9".repeat(plugin::MAX_OUTPUT_LEN + 1)
        );
        let mut refresher = refresher_with_registry(
            registry,
            vec![
                Ok(ok_response(oversized.as_bytes())),
                Ok(ok_response(oversized.as_bytes())),
            ],
        );

        let result = refresher.refresh(fixture_request("fixture-card"));

        assert_eq!(result.fields, vec![title_field("Air quality".into())]);
        assert!(!result.stale, "a summary failure poisoned the fetched data");
        assert!(
            result.value.is_some(),
            "a summary failure discarded the payload"
        );

        // Once per plugin id, not once per refresh: this fires every cadence.
        refresher.refresh(fixture_request("fixture-card"));
        assert_eq!(refresher.summary_failures.len(), 1);
        assert!(refresher.summary_failures.contains("fixture"));
    }
}
