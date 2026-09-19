//! Token quotes from `CoinGecko`'s public API.
//!
//! # Two requests, one of them optional
//!
//! `/coins/markets` carries everything the face needs to be correct: the
//! price, the 24-hour change, the window's high and low, and the token's own
//! symbol and name. `/coins/{id}/market_chart` carries only the sparkline.
//!
//! So the second request is allowed to fail on its own. A rate-limited or
//! slow chart endpoint leaves a face with a price and no sparkline, which is
//! a worse face but a true one; refusing the whole refresh would replace a
//! correct price with a stale badge because a decoration was unavailable.

use protocol::truncate_utf8_to_bytes;
use serde_json::Value;

use crate::http::HttpClient;
use crate::{LastGood, ProviderError, ProviderSnapshot};

const MARKETS_ENDPOINT: &str = "https://api.coingecko.com/api/v3/coins/markets";
const CHART_ENDPOINT_PREFIX: &str = "https://api.coingecko.com/api/v3/coins/";
/// The sparkline is drawn across 400 pixels, so more than this many samples
/// buys sub-pixel detail at the cost of path bytes in every push.
const MAX_SERIES_SAMPLES: usize = 96;
/// A price outside this window is a decoding mistake, not a market.
const MAX_PRICE: f64 = 1e12;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenOptions {
    /// `CoinGecko`'s own coin id, e.g. `solana`. Not the ticker: several tokens
    /// share a ticker and exactly one owns an id.
    pub coin_id: String,
    /// Quote currency code, e.g. `usd`.
    pub currency: String,
    /// An optional `CoinGecko` demo key, passed as a query parameter.
    ///
    /// A query parameter rather than the documented header because the shared
    /// [`HttpClient`] trait sends no custom headers, and widening it for one
    /// provider's optional credential would put a header map in front of every
    /// other caller. `CoinGecko` accepts either carrier.
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TokenQuote {
    /// The ticker as the API reports it, e.g. `sol`.
    pub symbol: String,
    pub name: String,
    /// The quote currency, echoed so the face can label itself.
    pub currency: String,
    pub price: f64,
    pub change_percent_24h: f64,
    pub high_24h: f64,
    pub low_24h: f64,
    /// Oldest to newest, already downsampled. Empty when the chart request
    /// failed or returned nothing usable.
    pub series: Vec<f64>,
}

pub struct TokenProvider<C> {
    client: C,
    options: TokenOptions,
    state: LastGood<TokenQuote>,
}

impl<C: HttpClient> TokenProvider<C> {
    pub fn new(client: C, options: TokenOptions) -> Self {
        Self {
            client,
            options,
            state: LastGood::default(),
        }
    }

    pub fn refresh(&mut self) -> ProviderSnapshot<TokenQuote> {
        let result = fetch_quote(&mut self.client, &self.options);
        self.state.complete(result)
    }
}

fn fetch_quote(
    client: &mut impl HttpClient,
    options: &TokenOptions,
) -> Result<TokenQuote, ProviderError> {
    let coin_id = options.coin_id.trim();
    let currency = options.currency.trim().to_lowercase();
    if coin_id.is_empty() {
        return Err(ProviderError::InvalidConfiguration(
            "the token's coin id is empty".into(),
        ));
    }
    if currency.is_empty() {
        return Err(ProviderError::InvalidConfiguration(
            "the token's quote currency is empty".into(),
        ));
    }
    // The id reaches a URL path, so it is restricted rather than escaped: a
    // CoinGecko id is always lowercase alphanumerics and hyphens, and
    // anything else is a configuration mistake worth naming.
    if !coin_id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(ProviderError::InvalidConfiguration(
            "the token's coin id may use only lowercase letters, digits and hyphens".into(),
        ));
    }

    let markets_url = endpoint_with_query(
        MARKETS_ENDPOINT,
        &[
            ("vs_currency", currency.as_str()),
            ("ids", coin_id),
            ("price_change_percentage", "24h"),
        ],
        options.api_key.as_deref(),
    )?;
    let markets = client.get_text(&markets_url)?;
    let mut quote = parse_markets(&markets, &currency)?;

    // Deliberately not `?`: see the module header. A missing sparkline is a
    // plainer face, not a failed refresh.
    let chart_url = endpoint_with_query(
        &format!("{CHART_ENDPOINT_PREFIX}{coin_id}/market_chart"),
        &[("vs_currency", currency.as_str()), ("days", "1")],
        options.api_key.as_deref(),
    )?;
    quote.series = client
        .get_text(&chart_url)
        .ok()
        .and_then(|body| parse_chart(&body).ok())
        .unwrap_or_default();

    Ok(quote)
}

fn parse_markets(body: &str, currency: &str) -> Result<TokenQuote, ProviderError> {
    let document: Value = serde_json::from_str(body)
        .map_err(|_| ProviderError::MalformedFeed("token market JSON is invalid".into()))?;
    let entry = document
        .as_array()
        .and_then(|entries| entries.first())
        .ok_or_else(|| ProviderError::MalformedFeed("the token was not found".into()))?;

    let price = bounded_price(finite(entry, "current_price")?)?;
    // The high and low are optional on a freshly listed token, and the face
    // handles a zero-width range. Falling back to the price keeps the range
    // marker honest rather than pinning it to an invented endpoint.
    let high = entry
        .get("high_24h")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .and_then(|value| bounded_price(value).ok())
        .unwrap_or(price);
    let low = entry
        .get("low_24h")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .and_then(|value| bounded_price(value).ok())
        .unwrap_or(price);
    let change = entry
        .get("price_change_percentage_24h_in_currency")
        .or_else(|| entry.get("price_change_percentage_24h"))
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .unwrap_or(0.0);

    Ok(TokenQuote {
        symbol: truncate_utf8_to_bytes(
            entry.get("symbol").and_then(Value::as_str).unwrap_or(""),
            12,
        )
        .to_owned(),
        name: truncate_utf8_to_bytes(entry.get("name").and_then(Value::as_str).unwrap_or(""), 32)
            .to_owned(),
        currency: currency.to_uppercase(),
        price,
        change_percent_24h: change.clamp(-100_000.0, 100_000.0),
        high_24h: high.max(low),
        low_24h: high.min(low),
        series: Vec::new(),
    })
}

/// Reads `prices: [[millis, price], ...]` and downsamples it.
fn parse_chart(body: &str) -> Result<Vec<f64>, ProviderError> {
    let document: Value = serde_json::from_str(body)
        .map_err(|_| ProviderError::MalformedFeed("token chart JSON is invalid".into()))?;
    let points = document
        .get("prices")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::MalformedFeed("the token chart has no prices".into()))?;

    let prices: Vec<f64> = points
        .iter()
        .filter_map(|point| point.as_array()?.get(1)?.as_f64())
        .filter(|price| price.is_finite() && *price >= 0.0 && *price <= MAX_PRICE)
        .collect();
    Ok(downsample(&prices, MAX_SERIES_SAMPLES))
}

/// Picks evenly spaced samples, always keeping the first and last.
///
/// The last point matters more than the rest: the face marks it as "now", and
/// a naive chunk-average would smooth the current price away from the price
/// printed above it.
fn downsample(prices: &[f64], target: usize) -> Vec<f64> {
    if prices.len() <= target || target < 2 {
        return prices.to_vec();
    }
    let last = prices.len() - 1;
    (0..target)
        .map(|index| {
            let position = index * last / (target - 1);
            prices[position]
        })
        .collect()
}

fn bounded_price(value: f64) -> Result<f64, ProviderError> {
    if value.is_finite() && (0.0..=MAX_PRICE).contains(&value) {
        Ok(value)
    } else {
        Err(ProviderError::MalformedFeed(
            "the token price is outside supported bounds".into(),
        ))
    }
}

fn finite(object: &Value, key: &str) -> Result<f64, ProviderError> {
    object
        .get(key)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .ok_or_else(|| ProviderError::MalformedFeed("a token numeric field is invalid".into()))
}

fn endpoint_with_query(
    endpoint: &str,
    values: &[(&str, &str)],
    api_key: Option<&str>,
) -> Result<String, ProviderError> {
    let mut url = url::Url::parse(endpoint)
        .map_err(|_| ProviderError::InvalidConfiguration("the token endpoint is invalid".into()))?;
    url.query_pairs_mut().extend_pairs(values.iter().copied());
    if let Some(key) = api_key.map(str::trim).filter(|key| !key.is_empty()) {
        url.query_pairs_mut().append_pair("x_cg_demo_api_key", key);
    }
    Ok(url.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MARKETS: &str = r#"[{
        "id": "solana",
        "symbol": "sol",
        "name": "Solana",
        "current_price": 142.37,
        "high_24h": 144.12,
        "low_24h": 136.9,
        "price_change_percentage_24h": 2.41
    }]"#;

    const CHART: &str = r#"{"prices": [[1,100.0],[2,101.0],[3,102.0],[4,103.5]]}"#;

    struct FakeClient {
        responses: Vec<Result<String, ProviderError>>,
        requested: Vec<String>,
    }

    impl FakeClient {
        fn new(responses: Vec<Result<String, ProviderError>>) -> Self {
            Self {
                responses,
                requested: Vec::new(),
            }
        }
    }

    impl HttpClient for FakeClient {
        fn get_text(&mut self, url: &str) -> Result<String, ProviderError> {
            self.requested.push(url.to_owned());
            if self.responses.is_empty() {
                return Err(ProviderError::Io("no response queued".into()));
            }
            self.responses.remove(0)
        }
    }

    fn options() -> TokenOptions {
        TokenOptions {
            coin_id: "solana".to_owned(),
            currency: "usd".to_owned(),
            api_key: None,
        }
    }

    #[test]
    fn a_quote_and_its_chart_are_read_into_one_reading() {
        let mut client = FakeClient::new(vec![Ok(MARKETS.into()), Ok(CHART.into())]);
        let quote = fetch_quote(&mut client, &options()).expect("the quote is read");
        assert_eq!(quote.symbol, "sol");
        assert_eq!(quote.name, "Solana");
        assert_eq!(quote.currency, "USD");
        assert!((quote.price - 142.37).abs() < f64::EPSILON);
        assert!((quote.change_percent_24h - 2.41).abs() < f64::EPSILON);
        assert_eq!(quote.series, vec![100.0, 101.0, 102.0, 103.5]);
    }

    #[test]
    fn a_failed_chart_leaves_a_priced_face_rather_than_failing_the_refresh() {
        // The whole point of the two-request split.
        let mut client = FakeClient::new(vec![Ok(MARKETS.into()), Err(ProviderError::Timeout)]);
        let quote = fetch_quote(&mut client, &options()).expect("the price still arrives");
        assert!((quote.price - 142.37).abs() < f64::EPSILON);
        assert!(quote.series.is_empty(), "only the sparkline is lost");
    }

    #[test]
    fn a_failed_market_request_fails_the_refresh() {
        let mut client = FakeClient::new(vec![Err(ProviderError::HttpStatus(429))]);
        let error = fetch_quote(&mut client, &options()).expect_err("the price is mandatory");
        assert_eq!(error, ProviderError::HttpStatus(429));
    }

    #[test]
    fn an_unknown_token_is_a_typed_refusal() {
        let mut client = FakeClient::new(vec![Ok("[]".into())]);
        let error = fetch_quote(&mut client, &options()).expect_err("an empty array is refused");
        assert!(matches!(&error, ProviderError::MalformedFeed(_)));
    }

    #[test]
    fn only_a_well_formed_coin_id_is_accepted() {
        // Uppercase is rejected too: CoinGecko ids are lowercase, and
        // accepting "Solana" would send a request that always 404s.
        for hostile in [
            "../../etc/passwd",
            "solana/../bitcoin",
            "Solana",
            "sol ana",
            "solana?x=1",
            "",
        ] {
            let mut client = FakeClient::new(vec![Ok(MARKETS.into())]);
            let mut bad = options();
            bad.coin_id = hostile.to_owned();
            let error =
                fetch_quote(&mut client, &bad).expect_err(&format!("{hostile:?} must be refused"));
            assert!(
                matches!(&error, ProviderError::InvalidConfiguration(_)),
                "{hostile:?} was refused for the wrong reason"
            );
        }
    }

    #[test]
    fn a_hostile_coin_id_never_reaches_the_network() {
        let mut client = FakeClient::new(vec![Ok(MARKETS.into())]);
        let mut bad = options();
        bad.coin_id = "solana/../../bitcoin".to_owned();
        let error = fetch_quote(&mut client, &bad).expect_err("a path-bearing id is refused");
        assert!(matches!(&error, ProviderError::InvalidConfiguration(_)));
        assert!(
            client.requested.is_empty(),
            "the refusal happens before the request, not after"
        );
    }

    #[test]
    fn an_api_key_rides_in_the_query() {
        let mut client = FakeClient::new(vec![Ok(MARKETS.into()), Ok(CHART.into())]);
        let mut keyed = options();
        keyed.api_key = Some("secret-key".to_owned());
        fetch_quote(&mut client, &keyed).expect("the quote is read");
        assert!(
            client.requested[0].contains("x_cg_demo_api_key=secret-key"),
            "the key is attached: {}",
            client.requested[0]
        );
    }

    #[test]
    fn a_blank_api_key_is_omitted_rather_than_sent_empty() {
        let mut client = FakeClient::new(vec![Ok(MARKETS.into()), Ok(CHART.into())]);
        let mut blank = options();
        blank.api_key = Some("   ".to_owned());
        fetch_quote(&mut client, &blank).expect("the quote is read");
        assert!(!client.requested[0].contains("x_cg_demo_api_key"));
    }

    #[test]
    fn a_missing_high_and_low_fall_back_to_the_price() {
        let body = r#"[{"symbol":"new","name":"New","current_price":5.0}]"#;
        let quote = parse_markets(body, "usd").expect("a bare quote is read");
        assert!((quote.high_24h - 5.0).abs() < f64::EPSILON);
        assert!((quote.low_24h - 5.0).abs() < f64::EPSILON);
        assert!(quote.change_percent_24h.abs() < f64::EPSILON);
    }

    #[test]
    fn a_swapped_high_and_low_are_put_back_in_order() {
        let body =
            r#"[{"symbol":"x","name":"X","current_price":5.0,"high_24h":1.0,"low_24h":9.0}]"#;
        let quote = parse_markets(body, "usd").expect("the quote is read");
        assert!(quote.high_24h >= quote.low_24h);
    }

    #[test]
    fn a_non_finite_or_negative_price_is_refused() {
        for body in [
            r#"[{"symbol":"x","name":"X","current_price":-1.0}]"#,
            r#"[{"symbol":"x","name":"X","current_price":null}]"#,
            r#"[{"symbol":"x","name":"X"}]"#,
        ] {
            assert!(
                parse_markets(body, "usd").is_err(),
                "{body} must be refused"
            );
        }
    }

    #[test]
    fn downsampling_keeps_the_endpoints_and_hits_the_target_length() {
        let prices: Vec<f64> = (0..500).map(f64::from).collect();
        let sampled = downsample(&prices, MAX_SERIES_SAMPLES);
        assert_eq!(sampled.len(), MAX_SERIES_SAMPLES);
        assert!((sampled[0] - 0.0).abs() < f64::EPSILON);
        assert!(
            (sampled[MAX_SERIES_SAMPLES - 1] - 499.0).abs() < f64::EPSILON,
            "the newest price survives, because the face marks it as now"
        );
    }

    #[test]
    fn a_short_series_is_left_alone() {
        let prices = vec![1.0, 2.0, 3.0];
        assert_eq!(downsample(&prices, MAX_SERIES_SAMPLES), prices);
    }

    #[test]
    fn a_chart_of_malformed_points_yields_an_empty_series_not_a_panic() {
        let body = r#"{"prices": [[1],[2,null],["x","y"],[3,"1e999"]]}"#;
        assert!(parse_chart(body).expect("the body parses").is_empty());
    }

    #[test]
    fn a_failed_refresh_keeps_the_last_good_price_and_reports_the_error() {
        let mut provider = TokenProvider::new(
            FakeClient::new(vec![
                Ok(MARKETS.into()),
                Ok(CHART.into()),
                Err(ProviderError::Timeout),
            ]),
            options(),
        );
        let first = provider.refresh();
        assert!(first.value.is_some());
        assert!(first.error.is_none());
        let second = provider.refresh();
        assert_eq!(
            second.value, first.value,
            "the complete last reading survives"
        );
        assert!(
            (second.value.as_ref().unwrap().price - 142.37).abs() < f64::EPSILON,
            "the panel keeps showing the last price it knew"
        );
        assert!(second.error.is_some());
    }
}
