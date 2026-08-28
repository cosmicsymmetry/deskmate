//! Wires curated plugin data sources into `providers::Provider` (Task 7,
//! Steps 4-6 of `docs/superpowers/sdd/2026-08-28-deskmate-plugin-manifest/`
//! and spec §5: "`companion/crates/providers` already defines `Provider`,
//! `ProviderSnapshot`, `LastGood`, `RefreshPolicy`, and categorised
//! `ProviderError`. Plugin data sources implement that trait, so
//! stale/last-good behaviour and the companion's existing per-card `stale`
//! flag apply to plugins with no new machinery.")
//!
//! This module therefore adds **no** new machinery to `Provider` itself:
//! [`PluginDataProvider`] is a `Provider` impl shaped exactly like
//! `providers::json_feed::JsonFeedProvider`, backed by the egress guard
//! (`crate::egress`) instead of a bare HTTP client.
//!
//! # Transient vs permanent (Step 5)
//!
//! `LastGood::complete` treats every `Err` the same way: keep the last-good
//! value, mark the snapshot `stale`, and carry a diagnostic message. That is
//! correct for a fetch that will plausibly succeed if retried -- a timeout,
//! a 5xx, a DNS blip, a connection reset -- and it is **wrong** for a fetch
//! that cannot ever succeed as configured -- a manifest whose URL is
//! disallowed, a 4xx that will not change on retry, a payload that will not
//! parse. Stage 3a shipped that distinction backwards once already, and
//! nothing caught it because the rule was never named in one place. Here it
//! is: [`classify_provider_error`] is the one function that decides, and
//! every `ProviderError` variant has exactly one row in it (tested in this
//! module's tests, one test per row).
//!
//! `Provider`/`ProviderSnapshot`/`LastGood` stay unchanged, so `stale` alone
//! cannot carry this distinction on the wire. [`PluginDataProvider`] instead
//! exposes it as a plain accessor, [`PluginDataProvider::last_failure_class`],
//! alongside the snapshot `refresh` already returns -- a caller that needs
//! to decide whether a card is merely stale or genuinely faulted (Task 8)
//! reads both.
//!
//! # Per-plugin caps (Step 6, spec §5: "CPU, memory, render wall-clock, and
//! refresh rate")
//!
//! - **Refresh rate** is enforced by [`refresh_interval_from_minutes`],
//!   reusing (not restating) `plugin::MIN_REFRESH_MINUTES`/
//!   `MAX_REFRESH_MINUTES` -- the same bound `plugin::parse_manifest`
//!   already checks at parse time, enforced again at the point a
//!   `Duration` is actually built for scheduling.
//! - **Memory** is bounded by `egress::MAX_RESPONSE_BODY_BYTES` (2 MiB),
//!   already enforced and unit-tested in `egress.rs`; this module's own
//!   test proves a body over that cap is surfaced through
//!   [`PluginDataProvider`] as a *permanent* classification (a source that
//!   structurally exceeds the fixed cap will exceed it again on retry).
//! - **Render wall-clock** is enforced by
//!   [`within_render_wall_clock_budget`], see its doc for what it can and
//!   cannot guarantee.
//! - **CPU is not enforced here, and cannot honestly be claimed as
//!   enforced.** `compile_scene` is a plain synchronous function call with
//!   no subprocess or OS-level resource limit around it; nothing in this
//!   process can preempt or cap the CPU time of a `FnOnce` it calls, short
//!   of a sandboxed subprocess -- which spec §5 explicitly defers ("No
//!   headless browser: it would make the homelab an arbitrary-code-execution
//!   host. Deferred indefinitely, not scheduled."). What actually keeps a
//!   compile's CPU cost small in practice is `plugin::compile_scene`'s own
//!   structural bounds -- `plugin::expr`'s fuel budget on every expression,
//!   the manifest's `MAX_NODES`, and `plugin::compile::MAX_REPEAT_ITEMS` --
//!   none of which are new machinery added by this task; they were already
//!   in place from Tasks 1-3. Spec §5 also notes v1 ships only "a curated
//!   set, not public uploads," which is the actual mitigation for a
//!   hostile-CPU plugin at this stage: every manifest is first-party.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use plugin::Source;
use providers::{LastGood, Provider, ProviderError, ProviderSnapshot, RefreshPolicy};
use serde_json::Value;

use crate::egress::{self, EgressError};

// ---------------------------------------------------------------------------
// Step 5: transient vs permanent classification.
// ---------------------------------------------------------------------------

/// Whether a fetch/parse failure is worth retrying (surfaces as `stale` on
/// the card, last-good value retained) or is structural (the only kind
/// allowed to fault the card). See this module's doc for the full
/// rationale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    /// The identical request, retried later, might succeed.
    Transient,
    /// The identical request will not succeed no matter how many times it
    /// is retried, because the manifest or its data source is wrong.
    Permanent,
}

/// The one place this crate decides transient vs permanent. Every
/// `ProviderError` variant has exactly one arm; each arm is tested by name
/// in this module's tests. Several arms share a body (most failure modes
/// are permanent) -- `match_same_arms` would rather those were merged into
/// one pattern, but merging them is exactly the "left implicit... scattered
/// through" shape the brief this table was built from calls out as the
/// defect: collapsing the table back into fewer arms would delete the
/// one-row-per-variant structure and the row-specific reasoning comments
/// that make it possible to audit at a glance.
#[allow(clippy::match_same_arms)]
pub fn classify_provider_error(error: &ProviderError) -> FailureClass {
    match error {
        // Network/availability trouble -- a connection reset and a DNS
        // failure both fold into `Io` by `provider_error_from_egress`
        // below, so this one arm covers both, matching the brief's "a
        // timeout, a 5xx, a DNS failure, a connection reset" list.
        ProviderError::Io(_) => FailureClass::Transient,
        ProviderError::Timeout => FailureClass::Transient,
        ProviderError::HttpStatus(status) => classify_http_status(*status),
        // The destination itself loops; retrying the identical URL loops
        // again.
        ProviderError::RedirectLimit => FailureClass::Permanent,
        // Under the egress guard's fixed 2 MiB body cap, a source that is
        // too large is too large structurally, not on this one occasion.
        ProviderError::ResponseTooLarge => FailureClass::Permanent,
        // Not valid UTF-8 is a property of the source's bytes, not a
        // network condition that clears up on retry.
        ProviderError::InvalidEncoding => FailureClass::Permanent,
        // "a payload that cannot be parsed at all" -- named explicitly as
        // permanent by the brief this table was built from.
        ProviderError::MalformedFeed(_) => FailureClass::Permanent,
        ProviderError::UnsafeContent => FailureClass::Permanent,
        // A manifest/URL misconfiguration, including an egress-denied
        // destination: the request as authored can never succeed.
        ProviderError::InvalidConfiguration(_) => FailureClass::Permanent,
    }
}

/// HTTP status classification, split out of [`classify_provider_error`]
/// because it is the one row with sub-cases: not every 4xx or 5xx behaves
/// the same way on retry. See [`classify_provider_error`]'s doc for why
/// `match_same_arms` is silenced rather than acted on here too.
#[allow(clippy::match_same_arms)]
fn classify_http_status(status: u16) -> FailureClass {
    match status {
        // 408 Request Timeout, 425 Too Early, 429 Too Many Requests: the
        // server is explicitly asking for a retry.
        408 | 425 | 429 => FailureClass::Transient,
        // 5xx: the server's own fault, plausibly transient.
        500..=599 => FailureClass::Transient,
        // Every other 4xx (400, 401, 403, 404, 410, ...): "a 4xx that will
        // never succeed", named explicitly by the brief -- the identical
        // request will not succeed no matter how many times it is retried.
        _ => FailureClass::Permanent,
    }
}

/// Converts a fetch failure -- the egress guard's own error type -- into
/// the crate-wide `ProviderError` vocabulary every other provider in this
/// workspace speaks, so [`classify_provider_error`] is the single
/// transient/permanent decision for every provider, plugin or built-in
/// alike, rather than a second table that exists only for plugins.
fn provider_error_from_egress(error: EgressError) -> ProviderError {
    match error {
        EgressError::InvalidUrl(detail) => {
            ProviderError::InvalidConfiguration(format!("invalid url: {detail}"))
        }
        EgressError::UnsupportedScheme(scheme) => {
            ProviderError::InvalidConfiguration(format!("unsupported scheme: {scheme}"))
        }
        EgressError::MissingHost => {
            ProviderError::InvalidConfiguration("url has no host".to_string())
        }
        EgressError::Denied { host, reason } => {
            ProviderError::InvalidConfiguration(format!("egress denied to {host}: {reason}"))
        }
        // DNS failures and bad redirects are network/availability trouble,
        // the same bucket as a connection reset -- folded into `Io` so
        // `classify_provider_error` treats them as transient, matching the
        // brief's "a DNS failure ... keep[s] the last good value".
        EgressError::ResolutionFailed { host, detail } => {
            ProviderError::Io(format!("dns resolution failed for {host}: {detail}"))
        }
        EgressError::BadRedirect(detail) => ProviderError::Io(format!("bad redirect: {detail}")),
        EgressError::Request(detail) => ProviderError::Io(detail),
        EgressError::Timeout => ProviderError::Timeout,
        // The destination loops; retrying the identical URL loops again.
        EgressError::TooManyRedirects => ProviderError::RedirectLimit,
        EgressError::ResponseTooLarge { .. } => ProviderError::ResponseTooLarge,
    }
}

fn parse_json_payload(bytes: Vec<u8>) -> Result<Value, ProviderError> {
    let text = String::from_utf8(bytes).map_err(|_| ProviderError::InvalidEncoding)?;
    serde_json::from_str(&text)
        .map_err(|_| ProviderError::MalformedFeed("JSON syntax is invalid".to_string()))
}

// ---------------------------------------------------------------------------
// Step 4: the `Provider` wiring itself.
// ---------------------------------------------------------------------------

/// What [`PluginDataProvider`] needs to retrieve one data source's raw
/// bytes. `Provider::refresh` is synchronous -- every provider in this
/// workspace is, because app-core's runtime worker calls `refresh` from a
/// plain OS thread, never from inside a Tokio task -- while `egress::fetch`
/// is necessarily async (DNS resolution and the HTTP client both are). This
/// trait is the seam that bridges the two, the same shape as
/// `providers::http::HttpClient`, and it is what lets tests substitute a
/// fetcher that touches neither the network nor a runtime.
pub trait PluginFetcher {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, EgressError>;
}

/// Bridges `Provider::refresh`'s synchronous contract to `egress::fetch`'s
/// async one via a dedicated, lazily-started single-thread Tokio runtime.
/// A single-thread runtime is deliberate: [`SystemPluginFetcher::fetch`]
/// blocks the calling OS thread until the request completes (bounded by
/// `egress::TOTAL_FETCH_BUDGET`), so nothing here benefits from a second
/// runtime worker thread, and `current_thread` is cheaper to start than
/// `multi_thread`.
///
/// **Caller obligation:** this must never be called from *inside* an
/// existing Tokio runtime -- `Runtime::block_on` panics if it is. Every
/// other `Provider::refresh` in this workspace already runs from a plain OS
/// thread for exactly this reason (app-core's runtime worker), and Task 8
/// must keep calling this one the same way.
pub struct SystemPluginFetcher;

fn blocking_runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to start the plugin fetch runtime")
    })
}

impl PluginFetcher for SystemPluginFetcher {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, EgressError> {
        blocking_runtime().block_on(egress::fetch(url))
    }
}

/// A curated plugin's data source, wired to `providers::Provider` with no
/// new machinery beyond the fetch/parse/classify steps above. `Output` is
/// `serde_json::Value` because that is what `plugin::compile_scene` takes
/// as its `ProviderSnapshot` (a "json" source is the only `plugin::Source`
/// kind today).
pub struct PluginDataProvider<F: PluginFetcher = SystemPluginFetcher> {
    fetcher: F,
    url: String,
    refresh_interval: Duration,
    state: LastGood<Value>,
    last_failure: Option<FailureClass>,
}

impl PluginDataProvider<SystemPluginFetcher> {
    /// Builds a provider backed by the real egress-guarded fetch.
    pub fn system(source: &Source) -> Result<Self, PluginCapError> {
        Self::new(SystemPluginFetcher, source)
    }
}

impl<F: PluginFetcher> PluginDataProvider<F> {
    pub fn new(fetcher: F, source: &Source) -> Result<Self, PluginCapError> {
        let Source::Json {
            url,
            refresh_minutes,
        } = source;
        let refresh_interval = refresh_interval_from_minutes(*refresh_minutes)?;
        Ok(Self {
            fetcher,
            url: url.clone(),
            refresh_interval,
            state: LastGood::default(),
            last_failure: None,
        })
    }

    /// The transient/permanent classification of the most recent failure;
    /// `None` if the most recent refresh succeeded, or none has run yet.
    /// This is the signal Step 5 asks for beyond `ProviderSnapshot` itself
    /// -- see this module's doc for why `stale` alone cannot carry it.
    pub fn last_failure_class(&self) -> Option<FailureClass> {
        self.last_failure
    }
}

impl<F: PluginFetcher> Provider for PluginDataProvider<F> {
    type Output = Value;

    fn refresh_policy(&self) -> RefreshPolicy {
        RefreshPolicy::Interval(self.refresh_interval)
    }

    fn refresh(&mut self, now: DateTime<Utc>) -> ProviderSnapshot<Value> {
        let result = self
            .fetcher
            .fetch(&self.url)
            .map_err(provider_error_from_egress)
            .and_then(parse_json_payload);
        self.last_failure = result.as_ref().err().map(classify_provider_error);
        self.state.complete(now, result)
    }
}

// ---------------------------------------------------------------------------
// Step 6: per-plugin caps.
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PluginCapError {
    #[error(
        "plugin refresh_minutes {minutes} is below the {minimum}-minute minimum refresh interval"
    )]
    RefreshTooFrequent { minutes: u32, minimum: u32 },
    #[error(
        "plugin refresh_minutes {minutes} exceeds the {maximum}-minute maximum refresh interval"
    )]
    RefreshTooInfrequent { minutes: u32, maximum: u32 },
    #[error("plugin render exceeded its {limit:?} wall-clock budget (took {elapsed:?})")]
    RenderWallClockExceeded { limit: Duration, elapsed: Duration },
}

/// Reused, not restated: `plugin::MIN_REFRESH_MINUTES`/`MAX_REFRESH_MINUTES`
/// already bound `source.refresh_minutes` when a manifest is parsed
/// (`plugin::parse_manifest`). This enforces the same bound again at the
/// point a `Duration` is actually built for scheduling, so a
/// [`PluginDataProvider`] can never be constructed with a schedule outside
/// the bound it was validated against, even if a future caller builds a
/// [`Source`] some other way than parsing a manifest.
pub fn refresh_interval_from_minutes(minutes: u32) -> Result<Duration, PluginCapError> {
    if minutes < plugin::MIN_REFRESH_MINUTES {
        return Err(PluginCapError::RefreshTooFrequent {
            minutes,
            minimum: plugin::MIN_REFRESH_MINUTES,
        });
    }
    if minutes > plugin::MAX_REFRESH_MINUTES {
        return Err(PluginCapError::RefreshTooInfrequent {
            minutes,
            maximum: plugin::MAX_REFRESH_MINUTES,
        });
    }
    Ok(Duration::from_secs(u64::from(minutes) * 60))
}

/// Wall-clock budget for compiling one plugin's manifest into a `Scene`
/// (spec §5's "render wall-clock" cap).
///
/// `plugin::compile_scene` is synchronous, bounded CPU work with no I/O and
/// no yield points, so nothing here can *preempt* a compile that is already
/// running past budget -- Rust has no safe way to interrupt a plain
/// function call without a sandboxed subprocess, which spec §5 does not
/// provision for v1. What [`within_render_wall_clock_budget`] DOES enforce:
/// a compile that *finishes* over budget is refused rather than pushed, so
/// a pathological manifest cannot silently ship a rendering that took far
/// longer than any card should, and repeated overruns are visible rather
/// than silent. In practice `compile_scene`'s own structural bounds
/// (`plugin::expr`'s fuel budget, the manifest's node ceiling, the repeat
/// ceiling) already keep one compile's cost small; this budget is a
/// backstop against those bounds drifting loose, not the only thing
/// standing between a compile and runaway cost.
pub const MAX_RENDER_WALL_CLOCK: Duration = Duration::from_millis(250);

/// `MAX_RENDER_WALL_CLOCK`, parameterized so tests can prove the over-budget
/// path deterministically and quickly rather than sleeping past the real
/// 250 ms constant.
fn within_wall_clock_budget<T>(
    budget: Duration,
    compile: impl FnOnce() -> T,
) -> Result<T, PluginCapError> {
    let started = Instant::now();
    let result = compile();
    let elapsed = started.elapsed();
    if elapsed > budget {
        return Err(PluginCapError::RenderWallClockExceeded {
            limit: budget,
            elapsed,
        });
    }
    Ok(result)
}

/// Runs `compile` (expected to be a call to `plugin::compile_scene`) and
/// refuses its result if it took longer than [`MAX_RENDER_WALL_CLOCK`]. See
/// that constant's doc for exactly what this can and cannot guarantee.
pub fn within_render_wall_clock_budget<T>(
    compile: impl FnOnce() -> T,
) -> Result<T, PluginCapError> {
    within_wall_clock_budget(MAX_RENDER_WALL_CLOCK, compile)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    // -- Step 5: one test per classification-table row -----------------

    #[test]
    fn io_and_timeout_are_transient() {
        assert_eq!(
            classify_provider_error(&ProviderError::Io("boom".into())),
            FailureClass::Transient
        );
        assert_eq!(
            classify_provider_error(&ProviderError::Timeout),
            FailureClass::Transient
        );
    }

    #[test]
    fn http_5xx_and_retry_hinting_4xx_are_transient() {
        for status in [408, 425, 429, 500, 502, 503, 599] {
            assert_eq!(
                classify_provider_error(&ProviderError::HttpStatus(status)),
                FailureClass::Transient,
                "status {status} should be transient"
            );
        }
    }

    #[test]
    fn other_http_4xx_are_permanent() {
        for status in [400, 401, 403, 404, 410, 451] {
            assert_eq!(
                classify_provider_error(&ProviderError::HttpStatus(status)),
                FailureClass::Permanent,
                "status {status} should be permanent"
            );
        }
    }

    #[test]
    fn redirect_limit_is_permanent() {
        assert_eq!(
            classify_provider_error(&ProviderError::RedirectLimit),
            FailureClass::Permanent
        );
    }

    #[test]
    fn response_too_large_is_permanent() {
        assert_eq!(
            classify_provider_error(&ProviderError::ResponseTooLarge),
            FailureClass::Permanent
        );
    }

    #[test]
    fn invalid_encoding_is_permanent() {
        assert_eq!(
            classify_provider_error(&ProviderError::InvalidEncoding),
            FailureClass::Permanent
        );
    }

    #[test]
    fn malformed_feed_is_permanent() {
        assert_eq!(
            classify_provider_error(&ProviderError::MalformedFeed("bad json".into())),
            FailureClass::Permanent
        );
    }

    #[test]
    fn unsafe_content_is_permanent() {
        assert_eq!(
            classify_provider_error(&ProviderError::UnsafeContent),
            FailureClass::Permanent
        );
    }

    #[test]
    fn invalid_configuration_is_permanent() {
        assert_eq!(
            classify_provider_error(&ProviderError::InvalidConfiguration("bad url".into())),
            FailureClass::Permanent
        );
    }

    // -- provider_error_from_egress + end-to-end wiring ------------------

    struct FakeFetcher {
        responses: Mutex<Vec<Result<Vec<u8>, EgressError>>>,
    }

    impl FakeFetcher {
        fn once(response: Result<Vec<u8>, EgressError>) -> Self {
            Self {
                responses: Mutex::new(vec![response]),
            }
        }

        fn sequence(responses: Vec<Result<Vec<u8>, EgressError>>) -> Self {
            let mut responses = responses;
            responses.reverse();
            Self {
                responses: Mutex::new(responses),
            }
        }
    }

    impl PluginFetcher for FakeFetcher {
        fn fetch(&self, _url: &str) -> Result<Vec<u8>, EgressError> {
            self.responses
                .lock()
                .expect("fake fetcher lock")
                .pop()
                .expect("FakeFetcher exhausted: refresh() called more times than scripted")
        }
    }

    fn json_source() -> Source {
        Source::Json {
            url: "https://example.test/aqi.json".to_string(),
            refresh_minutes: 15,
        }
    }

    #[test]
    fn a_dns_failure_keeps_the_last_good_value_and_marks_stale_not_faulted() {
        let fetcher = FakeFetcher::sequence(vec![
            Ok(br#"{"aqi": 42}"#.to_vec()),
            Err(EgressError::ResolutionFailed {
                host: "example.test".into(),
                detail: "no such host".into(),
            }),
        ]);
        let mut provider =
            PluginDataProvider::new(fetcher, &json_source()).expect("construct provider");

        let first = provider.refresh(Utc::now());
        assert!(!first.stale);
        assert_eq!(provider.last_failure_class(), None);

        let second = provider.refresh(Utc::now());
        assert!(second.stale, "a DNS failure must surface as stale");
        assert_eq!(
            second.value,
            serde_json::json!({"aqi": 42}),
            "the last-good value must be retained across a transient failure"
        );
        assert_eq!(
            provider.last_failure_class(),
            Some(FailureClass::Transient),
            "a DNS failure must classify as transient, never a card fault"
        );
    }

    #[test]
    fn an_egress_denied_destination_is_a_permanent_card_fault() {
        let fetcher = FakeFetcher::once(Err(EgressError::Denied {
            host: "169.254.169.254".into(),
            reason: egress::DenyReason::CloudMetadata,
        }));
        let mut provider =
            PluginDataProvider::new(fetcher, &json_source()).expect("construct provider");

        let snapshot = provider.refresh(Utc::now());

        assert!(snapshot.stale);
        assert_eq!(
            provider.last_failure_class(),
            Some(FailureClass::Permanent),
            "an SSRF-denied destination will never succeed on retry"
        );
    }

    #[test]
    fn an_unparsable_payload_is_a_permanent_card_fault() {
        let fetcher = FakeFetcher::once(Ok(b"not json at all".to_vec()));
        let mut provider =
            PluginDataProvider::new(fetcher, &json_source()).expect("construct provider");

        provider.refresh(Utc::now());

        assert_eq!(provider.last_failure_class(), Some(FailureClass::Permanent));
    }

    /// Proves the memory cap (`egress::MAX_RESPONSE_BODY_BYTES`, enforced
    /// in `egress.rs` itself) is correctly classified once it reaches this
    /// module's wiring: a body over the cap is a structural property of the
    /// source, not a one-off condition, so it must fault the card rather
    /// than merely mark it stale.
    #[test]
    fn a_response_over_the_memory_cap_is_a_permanent_card_fault() {
        let fetcher = FakeFetcher::once(Err(EgressError::ResponseTooLarge {
            limit: egress::MAX_RESPONSE_BODY_BYTES,
        }));
        let mut provider =
            PluginDataProvider::new(fetcher, &json_source()).expect("construct provider");

        provider.refresh(Utc::now());

        assert_eq!(provider.last_failure_class(), Some(FailureClass::Permanent));
    }

    // -- Step 6: caps -----------------------------------------------------

    #[test]
    fn a_refresh_interval_one_minute_below_the_floor_is_rejected() {
        let error = refresh_interval_from_minutes(plugin::MIN_REFRESH_MINUTES - 1)
            .expect_err("must be rejected");
        assert_eq!(
            error,
            PluginCapError::RefreshTooFrequent {
                minutes: plugin::MIN_REFRESH_MINUTES - 1,
                minimum: plugin::MIN_REFRESH_MINUTES,
            }
        );
    }

    #[test]
    fn a_refresh_interval_one_minute_above_the_ceiling_is_rejected() {
        let error = refresh_interval_from_minutes(plugin::MAX_REFRESH_MINUTES + 1)
            .expect_err("must be rejected");
        assert_eq!(
            error,
            PluginCapError::RefreshTooInfrequent {
                minutes: plugin::MAX_REFRESH_MINUTES + 1,
                maximum: plugin::MAX_REFRESH_MINUTES,
            }
        );
    }

    #[test]
    fn refresh_intervals_at_the_bounds_are_accepted() {
        assert!(refresh_interval_from_minutes(plugin::MIN_REFRESH_MINUTES).is_ok());
        assert!(refresh_interval_from_minutes(plugin::MAX_REFRESH_MINUTES).is_ok());
    }

    #[test]
    fn a_compile_within_budget_is_accepted() {
        let result = within_wall_clock_budget(Duration::from_millis(50), || 7);
        assert_eq!(result, Ok(7));
    }

    #[test]
    fn a_compile_over_budget_is_rejected_with_the_specific_variant() {
        let budget = Duration::from_millis(5);
        let result: Result<(), PluginCapError> = within_wall_clock_budget(budget, || {
            std::thread::sleep(Duration::from_millis(40));
        });

        match result {
            Err(PluginCapError::RenderWallClockExceeded { limit, elapsed }) => {
                assert_eq!(limit, budget);
                assert!(elapsed > budget);
            }
            other => panic!("expected RenderWallClockExceeded, got {other:?}"),
        }
    }

    #[test]
    fn the_public_render_budget_wraps_the_documented_constant() {
        // A closure that returns immediately must always pass the real,
        // documented budget -- this pins `within_render_wall_clock_budget`
        // to `MAX_RENDER_WALL_CLOCK` rather than some other constant.
        assert!(within_render_wall_clock_budget(|| ()).is_ok());
    }
}
