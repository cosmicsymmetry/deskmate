use super::*;

enum AddressCase {
    Url(&'static str),
    Ip(&'static str),
}

#[test]
#[allow(clippy::too_many_lines)] // one audit-friendly labelled row per denied range
fn every_special_purpose_range_is_classified_by_name() {
    use AddressCase::{Ip, Url};

    macro_rules! denied {
        ($label:literal, $input:expr, $reason:ident) => {
            ($label, $input, Some(DenyReason::$reason))
        };
    }
    let cases = [
        denied!("RFC1918 10/8", Url("http://10.1.2.3/"), Rfc1918Private),
        denied!(
            "RFC1918 172 start",
            Url("http://172.16.0.1/"),
            Rfc1918Private
        ),
        denied!(
            "RFC1918 172 end",
            Url("http://172.31.255.255/"),
            Rfc1918Private
        ),
        ("outside RFC1918 172", Url("http://172.32.0.1/"), None),
        denied!("RFC1918 192", Url("http://192.168.0.1/"), Rfc1918Private),
        denied!("IPv4 loopback", Url("http://127.0.0.5/"), Loopback),
        denied!("IPv6 loopback", Url("http://[::1]/"), Loopback),
        denied!("IPv4 link-local", Url("http://169.254.1.1/"), LinkLocal),
        denied!("IPv6 link-local", Url("http://[fe80::1]/"), LinkLocal),
        denied!(
            "cloud metadata",
            Url("http://169.254.169.254/latest/meta-data/"),
            CloudMetadata
        ),
        denied!("IPv6 ULA start", Url("http://[fc00::1]/"), UniqueLocalV6),
        denied!(
            "IPv6 ULA fd",
            Url("http://[fd12:3456:789a::1]/"),
            UniqueLocalV6
        ),
        denied!(
            "mapped loopback",
            Url("http://[::ffff:127.0.0.1]/"),
            Loopback
        ),
        denied!(
            "mapped private",
            Url("http://[::ffff:10.0.0.1]/"),
            Rfc1918Private
        ),
        denied!(
            "mapped metadata",
            Url("http://[::ffff:169.254.169.254]/"),
            CloudMetadata
        ),
        denied!("IPv4 unspecified", Url("http://0.0.0.0/"), Unspecified),
        denied!("IPv4 broadcast", Url("http://255.255.255.255/"), Broadcast),
        denied!("IPv4 multicast", Url("http://224.0.0.1/"), Multicast),
        // docker-vm's real Tailscale address: the threat model's
        // concrete "reaches unrelated neighbours" case.
        denied!(
            "Tailscale CGNAT",
            Url("http://100.93.166.123/"),
            CarrierGradeNat
        ),
        denied!("CGNAT start", Url("http://100.64.0.0/"), CarrierGradeNat),
        denied!("CGNAT first", Url("http://100.64.0.1/"), CarrierGradeNat),
        denied!("CGNAT end", Url("http://100.127.255.255/"), CarrierGradeNat),
        ("before CGNAT", Url("http://100.63.255.255/"), None),
        ("after CGNAT", Url("http://100.128.0.0/"), None),
        denied!(
            "IETF assignment",
            Url("http://192.0.0.8/"),
            IetfProtocolAssignment
        ),
        denied!("TEST-NET-1", Url("http://192.0.2.1/"), Documentation),
        denied!("TEST-NET-2", Url("http://198.51.100.1/"), Documentation),
        denied!("TEST-NET-3", Url("http://203.0.113.1/"), Documentation),
        denied!("benchmark start", Url("http://198.18.0.1/"), Benchmarking),
        denied!("benchmark end", Url("http://198.19.255.255/"), Benchmarking),
        denied!("class E", Url("http://240.0.0.1/"), Reserved),
        denied!("class E high", Url("http://250.1.2.3/"), Reserved),
        denied!("6to4 relay", Url("http://192.88.99.1/"), Reserved),
        denied!("0/8 reserved", Url("http://0.1.2.3/"), Reserved),
        denied!("IPv6 site-local", Ip("fec0::1"), SiteLocalV6),
        denied!("IPv6 documentation", Ip("2001:db8::1"), Documentation),
        denied!("IPv6 documentation 3fff start", Ip("3fff::"), Documentation),
        denied!(
            "IPv6 documentation 3fff first",
            Url("http://[3fff::1]/"),
            Documentation
        ),
        denied!(
            "IPv6 documentation 3fff interior",
            Ip("3fff:abc:1234::1"),
            Documentation
        ),
        denied!(
            "IPv6 documentation 3fff end",
            Ip("3fff:fff:ffff:ffff:ffff:ffff:ffff:ffff"),
            Documentation
        ),
        (
            "before IPv6 documentation 3fff",
            Ip("3ffe:ffff:ffff:ffff:ffff:ffff:ffff:ffff"),
            None,
        ),
        ("after IPv6 documentation 3fff", Ip("3fff:1000::"), None),
        denied!("NAT64 well-known", Ip("64:ff9b::102:304"), Nat64V6),
        denied!("IPv4-compatible IPv6", Ip("::0.1.2.3"), Ipv4CompatibleV6),
        denied!("NAT64 local-use", Ip("64:ff9b:1::102:304"), Nat64LocalUseV6),
        denied!("Teredo assignment", Ip("2001::1"), Ipv6ProtocolAssignment),
        denied!(
            "IPv6 benchmark assignment",
            Ip("2001:2::1"),
            Ipv6ProtocolAssignment
        ),
        denied!(
            "ORCHIDv2 assignment",
            Ip("2001:20::1"),
            Ipv6ProtocolAssignment
        ),
        // 6to4 encoding of 169.254.169.254, the cloud metadata address.
        denied!(
            "6to4 metadata encoding",
            Ip("2002:a9fe:a9fe::1"),
            SixToFourV6
        ),
        denied!("IPv6 discard-only", Ip("100::1"), DiscardOnlyV6),
        denied!("SRv6 SID space", Ip("5f00::1"), Srv6V6),
        denied!(
            "IPv4-translated IPv6",
            Ip("::ffff:0:7f00:1"),
            Ipv4TranslatedV6
        ),
        ("globally routable IPv4", Ip("93.184.216.34"), None),
    ];

    for (label, input, expected) in cases {
        let (raw, actual) = match input {
            Url(url) => {
                let reason = match egress_guard(url) {
                    Ok(_) => None,
                    Err(EgressError::Denied { reason, .. }) => Some(reason),
                    Err(error) => panic!("{label}: unexpected error for {url}: {error}"),
                };
                (url, reason)
            }
            Ip(raw) => {
                let ip: IpAddr = raw.parse().expect("table IP must parse");
                assert_eq!(
                    is_globally_routable(ip),
                    expected.is_none(),
                    "{label}: {raw}"
                );
                (raw, deny_reason_for_ip(ip))
            }
        };
        assert_eq!(actual, expected, "{label}: {raw}");
    }
}

#[test]
fn rejects_non_http_schemes() {
    for url in ["ftp://example.com/", "file:///etc/passwd", "gopher://x/"] {
        assert!(
            matches!(egress_guard(url), Err(EgressError::UnsupportedScheme(_))),
            "{url}"
        );
    }
}

#[tokio::test]
async fn post_form_refuses_the_cloud_metadata_address() {
    // Literal-IP deny check runs before any network I/O, so this is offline.
    let error = super::fetch_post_form("http://169.254.169.254/token", &[("a", "b")])
        .await
        .expect_err("metadata address must be denied");
    assert!(matches!(error, EgressError::Denied { .. }));
}

#[tokio::test]
async fn the_calendar_api_host_is_not_reachable_from_the_credential_bearing_post_path() {
    // Calendar is the producer's egress, so credential-bearing POSTs must
    // not reach it. Provider GET content fetching uses the address guard
    // without this identity-host allowlist.
    let error = super::fetch_post_form(
        "https://www.googleapis.com/calendar/v3/calendars/primary/events",
        &[],
    )
    .await
    .expect_err("the Calendar API must not be reachable from the credential-bearing POST path");
    assert!(matches!(error, EgressError::HostNotPermitted { .. }));
}

#[tokio::test]
async fn the_consent_host_is_not_reachable_from_the_credential_bearing_post_path() {
    // accounts.google.com is a 302 the BROWSER follows, not a fetch the
    // server makes, so it has no business on the allowlist.
    let error = super::fetch_post_form("https://accounts.google.com/o/oauth2/v2/auth", &[])
        .await
        .expect_err("the consent host must not be reachable from the credential-bearing POST path");
    assert!(matches!(error, EgressError::HostNotPermitted { .. }));
}

#[tokio::test]
async fn an_arbitrary_host_is_not_reachable_from_the_credential_bearing_post_path() {
    let error = super::fetch_post_form("https://example.invalid/token", &[])
        .await
        .expect_err("only the identity host is permitted");
    assert!(matches!(error, EgressError::HostNotPermitted { .. }));
}

#[test]
fn the_identity_host_passes_the_allowlist() {
    assert!(guard_identity_url("https://oauth2.googleapis.com/token").is_ok());
}

#[tokio::test]
async fn post_form_rejects_a_non_http_scheme() {
    let error = super::fetch_post_form("ftp://example.com/token", &[])
        .await
        .expect_err("non-http scheme must be rejected");
    assert!(matches!(error, EgressError::UnsupportedScheme(_)));
}

#[test]
fn rejects_unparseable_urls() {
    assert!(matches!(
        egress_guard("not a url"),
        Err(EgressError::InvalidUrl(_))
    ));
}

#[test]
fn permits_an_ordinary_public_host() {
    // A DNS-name host is never denied by the synchronous guard alone --
    // that is exactly what resolve-then-pin (resolve_and_pin,
    // select_pinned_address) exists to check once resolution has run.
    assert!(egress_guard("https://example.com/data.json").is_ok());
}

// --- select_pinned_address: the core of resolve-then-pin. Proves the
// policy is fail-closed and that exactly one candidate list decides the
// outcome (no second resolution ever happens in this function). ---

#[test]
fn select_pinned_address_picks_the_only_permitted_candidate() {
    let ip: IpAddr = "93.184.216.34".parse().unwrap();
    assert_eq!(select_pinned_address("example.com", &[ip]).unwrap(), ip);
}

#[test]
fn select_pinned_address_denies_if_any_candidate_is_denied() {
    // Simulates a resolver answer that mixes a public address with a
    // rebound / internal one: fail closed rather than picking "the good
    // one", because an attacker choosing which answer the HTTP client's
    // own connect step prefers is exactly the rebinding attack.
    let good: IpAddr = "93.184.216.34".parse().unwrap();
    let bad: IpAddr = "127.0.0.1".parse().unwrap();
    assert!(matches!(
        select_pinned_address("evil.example", &[good, bad]),
        Err(EgressError::Denied {
            reason: DenyReason::Loopback,
            ..
        })
    ));
    // Order must not matter.
    assert!(matches!(
        select_pinned_address("evil.example", &[bad, good]),
        Err(EgressError::Denied {
            reason: DenyReason::Loopback,
            ..
        })
    ));
}

#[test]
fn select_pinned_address_denies_3fff_documentation_in_either_order() {
    let public: IpAddr = "2606:4700:4700::1111".parse().unwrap();
    let documentation: IpAddr = "3fff::1".parse().unwrap();
    for candidates in [[public, documentation], [documentation, public]] {
        assert!(matches!(
            select_pinned_address("docs.example", &candidates),
            Err(EgressError::Denied {
                reason: DenyReason::Documentation,
                ..
            })
        ));
    }
}

#[test]
fn select_pinned_address_denies_empty_candidate_list() {
    assert!(matches!(
        select_pinned_address("nowhere.example", &[]),
        Err(EgressError::ResolutionFailed { .. })
    ));
}

// --- Caps: each named constant gets a test proving one past it is
// rejected with the specific error variant. ---

#[test]
fn body_limiter_accepts_exactly_the_cap() {
    let mut limiter = BodyLimiter::new(10);
    assert!(limiter.push(6).is_ok());
    assert!(limiter.push(4).is_ok());
}

#[test]
fn body_limiter_rejects_one_byte_past_the_cap() {
    let mut limiter = BodyLimiter::new(10);
    assert!(limiter.push(10).is_ok());
    assert!(matches!(
        limiter.push(1),
        Err(EgressError::ResponseTooLarge { limit: 10 })
    ));
}

#[test]
fn max_response_body_bytes_cap_is_enforced_by_the_same_limiter_fetch_uses() {
    let mut limiter = BodyLimiter::new(MAX_RESPONSE_BODY_BYTES);
    let cap =
        usize::try_from(MAX_RESPONSE_BODY_BYTES).expect("cap fits in usize on any real target");
    assert!(limiter.push(cap).is_ok());
    assert!(matches!(
        limiter.push(1),
        Err(EgressError::ResponseTooLarge {
            limit: MAX_RESPONSE_BODY_BYTES
        })
    ));
}

/// Drives the credential-bearing POST path against a real loopback server.
/// Replacing `build_pinned_client(&host, pinned_addr)?` in `post_form_inner` with a
/// plain `reqwest::Client::new()` fails here, because nothing would then
/// map `token.invalid` onto the listener. That single substitution would
/// also drop `.no_proxy()` and the redirect policy from the one request
/// that carries the client secret and the refresh token.
/// Test-only resolver that swaps out DNS alone: the URL keeps its fake
/// hostname (so `Host:` and TLS naming stay honest) while the connection
/// is pointed at a loopback listener.
struct FixedAddrResolver(SocketAddr);

impl HopResolver for FixedAddrResolver {
    // Test-only stand-in for real DNS resolution: it never actually
    // awaits anything, unlike `RealResolver`, which is why clippy flags
    // the `async` here as needless on its own.
    #[allow(clippy::unused_async_trait_impl)]
    async fn resolve(&self, url: &Url) -> Result<(String, SocketAddr), EgressError> {
        let host = url.host_str().ok_or(EgressError::MissingHost)?.to_string();
        Ok((host, self.0))
    }
}

#[tokio::test]
async fn fetch_inner_strips_path_and_query_secrets_from_request_errors() {
    let addr = SocketAddr::from(([127, 0, 0, 1], 0));

    let url = "http://failure.invalid:0/path-secret?query-secret-key=query-secret-value";
    let error = fetch_inner(url, &FixedAddrResolver(addr))
        .await
        .expect_err("the closed listener must refuse the connection");
    let EgressError::Request(detail) = error else {
        panic!("expected a request error, got {error:?}");
    };

    for secret in ["path-secret", "query-secret-key", "query-secret-value"] {
        assert!(
            !detail.contains(secret),
            "request error leaked {secret}: {detail}"
        );
    }
    assert!(
        detail.contains("client error (Connect)") && detail.contains("tcp connect error"),
        "the underlying connection diagnostic was lost: {detail}"
    );
}

#[tokio::test]
async fn post_form_resolves_then_pins_and_sends_the_form() {
    let router = axum::Router::new().route(
        "/token",
        axum::routing::post(|headers: axum::http::HeaderMap, body: String| async move {
            assert!(headers.get(axum::http::header::USER_AGENT).is_none());
            body
        }),
    );
    let addr = spawn_server(router).await;
    let url = format!("http://token.invalid:{}/token", addr.port());

    let response = post_form_with_resolver(
        &url,
        &FixedAddrResolver(addr),
        &[("grant_type", "refresh_token"), ("refresh_token", "rt")],
    )
    .await
    .expect("the pinned POST reaches the listener");

    assert_eq!(response.status, 200);
    assert_eq!(
        String::from_utf8(response.body).expect("utf8 body"),
        "grant_type=refresh_token&refresh_token=rt",
        "the form must arrive urlencoded in the request body"
    );
}

#[tokio::test]
async fn post_form_returns_the_response_status_for_a_non_2xx_response() {
    // Drive the real pinned POST path against a server that answers 503 so
    // the transport cannot silently replace a non-success status with 200.
    let router = axum::Router::new().route(
        "/unavailable",
        axum::routing::post(|| async {
            (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "service unavailable",
            )
        }),
    );
    let addr = spawn_server(router).await;
    let url = format!("http://unavailable.invalid:{}/unavailable", addr.port());

    let outcome = post_form_with_resolver(&url, &FixedAddrResolver(addr), &[]).await;

    let response = outcome.expect("a non-2xx response must still be a successful fetch");
    assert_eq!(response.status, 503);
    assert_eq!(response.body, b"service unavailable".to_vec());
}

#[tokio::test]
async fn post_form_enforces_the_response_body_cap_over_a_real_connection() {
    // Exercises specifically the `content_length()` precheck in
    // `read_capped_body`: axum sets a real, honest Content-Length for a
    // fully-materialized `Vec<u8>` body, and that header alone is
    // enough to reject this response before a single byte streams in.
    let router = axum::Router::new().route(
        "/oversized",
        axum::routing::post(|| async { oversized_body() }),
    );
    let addr = spawn_server(router).await;
    let url = format!("http://oversized.invalid:{}/oversized", addr.port());

    let outcome = post_form_inner(&url, &FixedAddrResolver(addr), &[]).await;

    assert!(
        matches!(
            outcome,
            Err(EgressError::ResponseTooLarge {
                limit: MAX_RESPONSE_BODY_BYTES
            })
        ),
        "expected ResponseTooLarge, got {outcome:?}"
    );
}

#[tokio::test]
async fn post_form_enforces_the_body_cap_against_a_server_that_never_declares_a_length() {
    // A declared-length case alone does not exercise the streaming
    // `limiter.push` check because the `content_length()` precheck rejects
    // it before streaming starts. A hostile server can simply never send
    // Content-Length at all (chunked transfer-encoding), which is
    // exactly what `Body::from_stream` produces here, since axum only
    // emits Content-Length when it knows the full size upfront. This
    // is the case that can only be caught by the per-chunk
    // `limiter.push` call as bytes actually arrive.
    let router = axum::Router::new().route(
        "/chunked-oversized",
        axum::routing::post(|| async { chunked_oversized_body() }),
    );
    let addr = spawn_server(router).await;
    let url = format!(
        "http://chunked-oversized.invalid:{}/chunked-oversized",
        addr.port()
    );

    let outcome = post_form_inner(&url, &FixedAddrResolver(addr), &[]).await;

    assert!(
        matches!(
            outcome,
            Err(EgressError::ResponseTooLarge {
                limit: MAX_RESPONSE_BODY_BYTES
            })
        ),
        "expected ResponseTooLarge, got {outcome:?}"
    );
}

// --- The dependency assumption the module doc flags as unproven:
// reqwest's `.resolve()` override both connects to the pinned address
// AND preserves the original hostname in the outgoing `Host` header.
// This talks to `reqwest::Client` directly -- no `egress_guard`, no
// `post_form_inner`, nothing from this module's own deny logic -- so it
// pins reqwest's behaviour in isolation from everything this module
// built on top of it. ---

#[tokio::test]
async fn reqwest_resolve_override_connects_to_the_pin_and_preserves_the_host_header() {
    let router = axum::Router::new().route(
        "/",
        axum::routing::get(|headers: axum::http::HeaderMap| async move {
            headers
                .get(axum::http::header::HOST)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_string()
        }),
    );
    let addr = spawn_server(router).await;

    // example.invalid is IANA-reserved (RFC 2606) and will not resolve
    // via real DNS -- if `.resolve()` were not actually honoured, this
    // request would fail outright rather than quietly hitting the
    // wrong address.
    let client = reqwest::Client::builder()
        .resolve("example.invalid", addr)
        .no_proxy()
        .build()
        .expect("build a plain reqwest client");
    let url = format!("http://example.invalid:{}/", addr.port());

    let response = client
        .get(&url)
        .send()
        .await
        .expect("the pinned connection should succeed despite the unresolvable name");
    let observed_host = response.text().await.expect("read response body");

    assert_eq!(observed_host, format!("example.invalid:{}", addr.port()));
}

/// A redirect must not walk an encrypted fetch down onto cleartext.
/// `egress_guard` itself accepts `http`, so it cannot catch a downgrade
/// without the previous hop's scheme.
#[test]
fn a_redirect_may_not_downgrade_https_to_http() {
    let secure = Url::parse("https://feeds.example/data.json").expect("url");
    let cleartext = Url::parse("http://feeds.example/data.json").expect("url");

    let error =
        deny_scheme_downgrade(&secure, &cleartext).expect_err("https -> http must be refused");
    assert!(
        matches!(&error, EgressError::BadRedirect(detail) if detail.contains("downgrade")),
        "unexpected error: {error}"
    );

    // The three hops that are not downgrades all stay allowed.
    deny_scheme_downgrade(&secure, &secure).expect("https -> https is fine");
    deny_scheme_downgrade(&cleartext, &secure).expect("http -> https is an upgrade");
    deny_scheme_downgrade(&cleartext, &cleartext).expect("http -> http is the caller's choice");
}

#[test]
fn a_relative_redirect_target_resolves_against_its_base() {
    let base = Url::parse("https://example.com/a/start").unwrap();
    let next = base.join("../b/next").unwrap();
    assert_eq!(next.as_str(), "https://example.com/b/next");
    assert!(egress_guard(next.as_str()).is_ok());
}

#[tokio::test]
async fn fetch_inner_denies_a_redirect_chain_that_exceeds_max_redirects() {
    // A handler that redirects to itself forever exercises the cap directly:
    // `fetch_inner` must give up after MAX_REDIRECTS hops, not loop
    // indefinitely.
    let router = axum::Router::new().route(
        "/loop",
        axum::routing::get(|| async { axum::response::Redirect::to("/loop") }),
    );
    let addr = spawn_server(router).await;
    let url = format!("http://self-redirect.invalid:{}/loop", addr.port());

    let outcome = fetch_inner(&url, &FixedAddrResolver(addr)).await;

    assert!(
        matches!(outcome, Err(EgressError::TooManyRedirects)),
        "expected TooManyRedirects, got {outcome:?}"
    );
}

#[tokio::test]
async fn fetch_inner_denies_a_redirect_target_that_resolves_to_a_denied_address() {
    // Distinct from the URL-space-only check above: this drives the
    // REAL fetch_inner loop end to end -- resolver.resolve, the HTTP
    // GET, RedirectBudget::consume, Location extraction, Url::join,
    // and finally egress_guard running again on the joined URL -- via a
    // server that actually issues the redirect, rather than
    // hand-constructing the joined URL and calling egress_guard on it
    // directly. A `fetch_inner` that dropped the re-validation call
    // would return `Ok` here instead.
    let router = axum::Router::new().route(
        "/start",
        axum::routing::get(|| async {
            axum::response::Redirect::to("http://169.254.169.254/secret")
        }),
    );
    let addr = spawn_server(router).await;
    let url = format!("http://redirect-test.invalid:{}/start", addr.port());

    let outcome = fetch_inner(&url, &FixedAddrResolver(addr)).await;

    assert!(
        matches!(
            outcome,
            Err(EgressError::Denied {
                reason: DenyReason::CloudMetadata,
                ..
            })
        ),
        "expected the metadata redirect target to be denied, got {outcome:?}"
    );
}

#[tokio::test]
async fn fetch_inner_enforces_the_body_cap_against_a_server_that_never_declares_a_length() {
    // A declared-length case alone does not exercise the streaming
    // `limiter.push` check because the `content_length()` precheck rejects
    // it before streaming starts. A hostile server can simply never send
    // Content-Length at all (chunked transfer-encoding), which is
    // exactly what `Body::from_stream` produces here, since axum only
    // emits Content-Length when it knows the full size upfront. This
    // is the case that can only be caught by the per-chunk
    // `limiter.push` call as bytes actually arrive.
    let router = axum::Router::new().route(
        "/chunked-oversized",
        axum::routing::get(|| async { chunked_oversized_body() }),
    );
    let addr = spawn_server(router).await;
    let url = format!(
        "http://chunked-oversized.invalid:{}/chunked-oversized",
        addr.port()
    );

    let outcome = fetch_inner(&url, &FixedAddrResolver(addr)).await;

    assert!(
        matches!(
            outcome,
            Err(EgressError::ResponseTooLarge {
                limit: MAX_RESPONSE_BODY_BYTES
            })
        ),
        "expected ResponseTooLarge, got {outcome:?}"
    );
}

#[tokio::test]
async fn fetch_inner_enforces_the_response_body_cap_over_a_real_connection() {
    // Exercises specifically the `content_length()` precheck in
    // `read_capped_body`: axum sets a real, honest Content-Length for a
    // fully-materialized `Vec<u8>` body, and that header alone is
    // enough to reject this response before a single byte streams in.
    let router = axum::Router::new().route(
        "/oversized",
        axum::routing::get(|| async { oversized_body() }),
    );
    let addr = spawn_server(router).await;
    let url = format!("http://oversized.invalid:{}/oversized", addr.port());

    let outcome = fetch_inner(&url, &FixedAddrResolver(addr)).await;

    assert!(
        matches!(
            outcome,
            Err(EgressError::ResponseTooLarge {
                limit: MAX_RESPONSE_BODY_BYTES
            })
        ),
        "expected ResponseTooLarge, got {outcome:?}"
    );
}

#[tokio::test]
async fn fetch_returns_the_response_status_for_a_non_2xx_response() {
    // Drive the real guarded GET path against a server that answers 503 so
    // provider error handling receives the upstream status unchanged.
    let router = axum::Router::new().route(
        "/unavailable",
        axum::routing::get(|| async {
            (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "service unavailable",
            )
        }),
    );
    let addr = spawn_server(router).await;
    let url = format!("http://unavailable.invalid:{}/unavailable", addr.port());

    let outcome = fetch_with_resolver(&url, &FixedAddrResolver(addr)).await;

    let response = outcome.expect("a non-2xx response must still be a successful fetch");
    assert_eq!(response.status, 503);
    assert_eq!(response.body, b"service unavailable".to_vec());
}

#[tokio::test]
async fn fetch_succeeds_through_a_redirect_chain_that_stays_under_the_total_budget() {
    // 3 hops * 5ms is trivially under the real TOTAL_FETCH_BUDGET
    // (20s), so this drives the exact production composition
    // (`fetch_with_resolver`, hence `fetch`'s real constant) rather
    // than a shrunk test-only budget, with no risk of running slow.
    let hop_delay = Duration::from_millis(5);
    let addr = spawn_delayed_redirect_chain(hop_delay).await;
    let url = format!("http://fast-chain.invalid:{}/hop1", addr.port());

    let outcome = fetch_with_resolver(&url, &FixedAddrResolver(addr)).await;

    let response = outcome.expect("fetch should succeed");
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"done".to_vec());
}

#[tokio::test]
async fn fetch_times_out_when_cumulative_hop_delay_exceeds_the_total_budget() {
    // Real (not paused/virtual) time throughout: mixing tokio's paused
    // clock with genuine TCP I/O across two concurrently-running tasks
    // (this test's client, the spawned axum server) turned out to be
    // exactly as unreliable as its reputation -- an earlier version of
    // this test used `#[tokio::test(start_paused = true)]` with 9-second
    // hop delays and failed non-deterministically with the client's own
    // per-hop request timeout firing before the paused clock had
    // advanced the server's sleep, even though the sleep's deadline was
    // provably sooner. Real, small (millisecond) delays sidestep the
    // whole class of problem and are just as deterministic.
    //
    // `fetch_with_budget` (not `fetch_with_resolver`) lets this pass a
    // millisecond-scale total budget rather than waiting out the real
    // TOTAL_FETCH_BUDGET (20s); REQUEST_TIMEOUT stays the real 10s
    // constant, unaffected, and is not at risk of firing here.
    let hop_delay = Duration::from_millis(60);
    let total_budget = Duration::from_millis(100);
    assert!(hop_delay < REQUEST_TIMEOUT);
    assert!(hop_delay * 3 > total_budget);
    let addr = spawn_delayed_redirect_chain(hop_delay).await;
    let url = format!("http://slow-chain.invalid:{}/hop1", addr.port());

    let outcome = fetch_with_budget(&url, &FixedAddrResolver(addr), total_budget).await;

    assert!(
        matches!(outcome, Err(EgressError::Timeout)),
        "expected Timeout, got {outcome:?}"
    );
}

#[test]
fn redirect_budget_allows_exactly_max_redirects_hops() {
    let mut budget = RedirectBudget::new(MAX_REDIRECTS);
    for hop in 0..MAX_REDIRECTS {
        assert!(budget.consume().is_ok(), "hop {hop} should be permitted");
    }
}

#[test]
fn redirect_budget_rejects_one_hop_past_max_redirects() {
    let mut budget = RedirectBudget::new(MAX_REDIRECTS);
    for _ in 0..MAX_REDIRECTS {
        budget.consume().expect("hops up to the cap are permitted");
    }
    assert!(matches!(
        budget.consume(),
        Err(EgressError::TooManyRedirects)
    ));
}

/// Spawns a loopback server whose `/hop1` -> `/hop2` -> `/hop3` chain
/// each sleeps `hop_delay` before redirecting to the next, and `/hop3`
/// finally responds 200. Each hop's sleep is kept under
/// [`REQUEST_TIMEOUT`] individually so no single hop times out; the
/// point is their *sum*, which a caller picks to land on either side of
/// [`TOTAL_FETCH_BUDGET`].
async fn spawn_delayed_redirect_chain(hop_delay: Duration) -> SocketAddr {
    let router = axum::Router::new()
        .route(
            "/hop1",
            axum::routing::get(move |headers: axum::http::HeaderMap| async move {
                assert_eq!(headers[axum::http::header::USER_AGENT], USER_AGENT);
                tokio::time::sleep(hop_delay).await;
                axum::response::Redirect::to("/hop2")
            }),
        )
        .route(
            "/hop2",
            axum::routing::get(move |headers: axum::http::HeaderMap| async move {
                assert_eq!(headers[axum::http::header::USER_AGENT], USER_AGENT);
                tokio::time::sleep(hop_delay).await;
                axum::response::Redirect::to("/hop3")
            }),
        )
        .route(
            "/hop3",
            axum::routing::get(move |headers: axum::http::HeaderMap| async move {
                assert_eq!(headers[axum::http::header::USER_AGENT], USER_AGENT);
                tokio::time::sleep(hop_delay).await;
                "done"
            }),
        );
    spawn_server(router).await
}

async fn spawn_server(router: axum::Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a loopback listener");
    let addr = listener.local_addr().expect("listener has a local addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    addr
}

fn oversized_body() -> Vec<u8> {
    let cap = usize::try_from(MAX_RESPONSE_BODY_BYTES).expect("cap fits in usize");
    vec![0u8; cap + 1]
}

fn chunked_oversized_body() -> axum::body::Body {
    let cap = usize::try_from(MAX_RESPONSE_BODY_BYTES).expect("cap fits in usize");
    let chunks: Vec<Result<Vec<u8>, std::io::Error>> = vec![Ok(vec![0u8; cap]), Ok(vec![0u8; 1])];
    axum::body::Body::from_stream(futures_util::stream::iter(chunks))
}
