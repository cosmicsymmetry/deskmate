//! SSRF egress guard for outbound plugin data fetches (spec `docs/plugin
//! manifest` §5: "Deny RFC1918, loopback, link-local, and
//! `169.254.169.254`; resolve-then-pin against DNS rebinding; cap size,
//! time, and redirects.").
//!
//! The render host is the owner's homelab, alongside unrelated services
//! behind the same private network. A plugin data source is an
//! owner-supplied URL that this process fetches on the plugin's behalf; an
//! SSRF here would let a malicious or compromised plugin reach those
//! neighbours (or the cloud metadata endpoint, if this ever runs on a
//! cloud VM). This module is the only place that is allowed to decide
//! "yes, fetch that" and the only place that performs the fetch.
//!
//! # Why hostname validation alone is not enough
//!
//! Checking a URL's hostname and then handing the URL to an HTTP client is
//! not sufficient: the client performs its own DNS lookup at connect time,
//! and the answer can differ from whatever the guard saw a moment earlier
//! (DNS rebinding). [`fetch`] instead resolves each host exactly once,
//! validates every address that resolution returned, and then pins the
//! HTTP client to that *specific validated address* via
//! `reqwest::ClientBuilder::resolve`, which overrides reqwest's own
//! resolver for that request. reqwest never re-resolves the host itself.
//!
//! # What this does not guarantee
//!
//! `reqwest::ClientBuilder::resolve` is a documented, first-class reqwest
//! API for exactly this purpose, and is trusted here to do what it says:
//! connect to the pinned `SocketAddr` rather than re-resolving. This
//! module does not independently verify reqwest's TCP connect behaviour
//! (e.g. by intercepting the socket), so a defect inside reqwest's
//! connector itself is outside what this guard can catch. Every redirect
//! hop is re-validated and re-pinned from scratch (a permitted host can
//! redirect to `169.254.169.254`), but a single connection is not
//! continuously re-checked against TOCTOU races faster than one DNS
//! resolution.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use reqwest::Url;

/// The well-known cloud instance-metadata address (AWS/GCP/Azure/etc.),
/// called out by the spec separately from the RFC 3927 link-local block it
/// technically already falls inside -- deny it by name so a denial reads as
/// "this is the metadata endpoint," not merely "this is link-local".
const CLOUD_METADATA_ADDR: Ipv4Addr = Ipv4Addr::new(169, 254, 169, 254);

/// Wall-clock budget for a single DNS resolution.
pub const RESOLUTION_TIMEOUT: Duration = Duration::from_secs(5);

/// Wall-clock budget for a single HTTP request/response (one redirect hop).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Wall-clock budget for the *entire* fetch, including every redirect hop
/// and DNS resolution. Bounds total time even though each hop also has its
/// own [`REQUEST_TIMEOUT`], because a chain of hops that are each fast
/// individually could otherwise still run unboundedly long.
pub const TOTAL_FETCH_BUDGET: Duration = Duration::from_secs(20);

/// Maximum number of redirect hops followed before giving up. `0` means the
/// first response must not be a redirect.
pub const MAX_REDIRECTS: u8 = 5;

/// Maximum response body size accepted from a plugin data source.
pub const MAX_RESPONSE_BODY_BYTES: u64 = 2 * 1024 * 1024;

/// Why a fetch was refused. Every variant is meant to be safe to log and to
/// surface to an operator; none carry response bodies or secrets.
#[derive(Debug)]
pub enum EgressError {
    /// The URL could not be parsed at all.
    InvalidUrl(String),
    /// The URL's scheme was not `http` or `https`.
    UnsupportedScheme(String),
    /// The URL had no host component.
    MissingHost,
    /// `host` (or one of the addresses it resolved to) is on the deny list.
    Denied { host: String, reason: DenyReason },
    /// DNS resolution for `host` failed or timed out.
    ResolutionFailed { host: String, detail: String },
    /// Too many redirect hops.
    TooManyRedirects,
    /// The response body exceeded [`MAX_RESPONSE_BODY_BYTES`].
    ResponseTooLarge { limit: u64 },
    /// The fetch did not complete inside [`TOTAL_FETCH_BUDGET`].
    Timeout,
    /// A redirect response had a missing or unusable `Location` header.
    BadRedirect(String),
    /// The underlying HTTP client reported an error (connect failure,
    /// protocol error, etc.).
    Request(String),
}

impl fmt::Display for EgressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EgressError::InvalidUrl(detail) => write!(f, "invalid url: {detail}"),
            EgressError::UnsupportedScheme(scheme) => {
                write!(f, "unsupported scheme: {scheme}")
            }
            EgressError::MissingHost => write!(f, "url has no host"),
            EgressError::Denied { host, reason } => {
                write!(f, "egress denied to {host}: {reason}")
            }
            EgressError::ResolutionFailed { host, detail } => {
                write!(f, "dns resolution failed for {host}: {detail}")
            }
            EgressError::TooManyRedirects => {
                write!(f, "too many redirects (limit {MAX_REDIRECTS})")
            }
            EgressError::ResponseTooLarge { limit } => {
                write!(f, "response exceeded {limit}-byte cap")
            }
            EgressError::Timeout => {
                write!(f, "fetch exceeded {TOTAL_FETCH_BUDGET:?} time budget")
            }
            EgressError::BadRedirect(detail) => write!(f, "bad redirect: {detail}"),
            EgressError::Request(detail) => write!(f, "request failed: {detail}"),
        }
    }
}

impl std::error::Error for EgressError {}

/// Why an address is on the deny list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyReason {
    Loopback,
    LinkLocal,
    CloudMetadata,
    Rfc1918Private,
    UniqueLocalV6,
    Unspecified,
    Multicast,
    Broadcast,
}

impl fmt::Display for DenyReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            DenyReason::Loopback => "loopback",
            DenyReason::LinkLocal => "link-local",
            DenyReason::CloudMetadata => "cloud metadata endpoint",
            DenyReason::Rfc1918Private => "RFC1918 private",
            DenyReason::UniqueLocalV6 => "unique-local IPv6 (fc00::/7)",
            DenyReason::Unspecified => "unspecified address",
            DenyReason::Multicast => "multicast",
            DenyReason::Broadcast => "broadcast",
        };
        f.write_str(label)
    }
}

/// Classifies `ip` against the deny list. `None` means `ip` is permitted.
///
/// IPv4-mapped IPv6 addresses (`::ffff:a.b.c.d`) are unwrapped to their
/// embedded IPv4 form first, so e.g. `::ffff:127.0.0.1` is denied as
/// loopback rather than sailing through an IPv6-only check.
#[must_use]
pub fn deny_reason_for_ip(ip: IpAddr) -> Option<DenyReason> {
    match ip {
        IpAddr::V4(v4) => deny_reason_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(mapped) => deny_reason_v4(mapped),
            None => deny_reason_v6(v6),
        },
    }
}

fn deny_reason_v4(ip: Ipv4Addr) -> Option<DenyReason> {
    if ip == CLOUD_METADATA_ADDR {
        Some(DenyReason::CloudMetadata)
    } else if ip.is_loopback() {
        Some(DenyReason::Loopback)
    } else if ip.is_link_local() {
        Some(DenyReason::LinkLocal)
    } else if ip.is_private() {
        Some(DenyReason::Rfc1918Private)
    } else if ip.is_unspecified() {
        Some(DenyReason::Unspecified)
    } else if ip.is_broadcast() {
        Some(DenyReason::Broadcast)
    } else if ip.is_multicast() {
        Some(DenyReason::Multicast)
    } else {
        None
    }
}

fn deny_reason_v6(ip: Ipv6Addr) -> Option<DenyReason> {
    const LINK_LOCAL_MASK: u16 = 0xffc0; // fe80::/10
    const LINK_LOCAL_PREFIX: u16 = 0xfe80;
    const UNIQUE_LOCAL_MASK: u16 = 0xfe00; // fc00::/7
    const UNIQUE_LOCAL_PREFIX: u16 = 0xfc00;

    let leading = ip.segments()[0];
    if ip.is_loopback() {
        Some(DenyReason::Loopback)
    } else if ip.is_unspecified() {
        Some(DenyReason::Unspecified)
    } else if leading & LINK_LOCAL_MASK == LINK_LOCAL_PREFIX {
        Some(DenyReason::LinkLocal)
    } else if leading & UNIQUE_LOCAL_MASK == UNIQUE_LOCAL_PREFIX {
        Some(DenyReason::UniqueLocalV6)
    } else if ip.is_multicast() {
        Some(DenyReason::Multicast)
    } else {
        None
    }
}

/// Synchronous pre-check: parses `url`, rejects any scheme other than
/// `http`/`https`, and -- when the host is a literal IP address rather
/// than a DNS name -- rejects it immediately if it is on the deny list.
///
/// A literal-IP host needs no DNS resolution to validate, so this check is
/// synchronous and cheap; it exists as a fast, obvious first gate that
/// [`fetch`] also runs on every redirect target. It does **not** perform
/// DNS resolution, so a DNS-name host that resolves to a denied address is
/// only caught by [`fetch`]'s resolve-then-pin step, which runs the real
/// resolution exactly once per hop.
pub fn egress_guard(url: &str) -> Result<Url, EgressError> {
    let parsed = Url::parse(url).map_err(|error| EgressError::InvalidUrl(error.to_string()))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => return Err(EgressError::UnsupportedScheme(other.to_string())),
    }
    match parsed.host() {
        Some(url::Host::Ipv4(ip)) => deny_literal(&parsed, IpAddr::V4(ip))?,
        Some(url::Host::Ipv6(ip)) => deny_literal(&parsed, IpAddr::V6(ip))?,
        Some(url::Host::Domain(_)) => {}
        None => return Err(EgressError::MissingHost),
    }
    Ok(parsed)
}

fn deny_literal(url: &Url, ip: IpAddr) -> Result<(), EgressError> {
    if let Some(reason) = deny_reason_for_ip(ip) {
        return Err(EgressError::Denied {
            host: url.host_str().unwrap_or_default().to_string(),
            reason,
        });
    }
    Ok(())
}

/// Chooses the address a fetch will connect to, from every address one DNS
/// resolution returned for `host`.
///
/// Fails closed: if **any** returned address is on the deny list, the
/// whole resolution is denied rather than silently picking one of the
/// remaining permitted addresses. A resolver returning a mix of public and
/// internal answers for one name is itself a signal worth refusing outright
/// -- accepting "the good one" would depend on this function and the HTTP
/// client always agreeing on which answer that is, which is exactly the
/// kind of assumption DNS rebinding defeats.
fn select_pinned_address(host: &str, candidates: &[IpAddr]) -> Result<IpAddr, EgressError> {
    if candidates.is_empty() {
        return Err(EgressError::ResolutionFailed {
            host: host.to_string(),
            detail: "resolution returned no addresses".to_string(),
        });
    }
    for ip in candidates {
        if let Some(reason) = deny_reason_for_ip(*ip) {
            return Err(EgressError::Denied {
                host: host.to_string(),
                reason,
            });
        }
    }
    Ok(candidates[0])
}

/// Performs the one-and-only DNS resolution for `host`, honouring
/// [`RESOLUTION_TIMEOUT`]. A literal IP-address host is returned as-is
/// without touching the network or the resolver.
async fn resolve_once(host: &str, port: u16) -> Result<Vec<IpAddr>, EgressError> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![ip]);
    }
    let lookup = format!("{host}:{port}");
    let outcome = tokio::time::timeout(RESOLUTION_TIMEOUT, tokio::net::lookup_host(lookup)).await;
    match outcome {
        Ok(Ok(addrs)) => {
            let ips: Vec<IpAddr> = addrs.map(|addr| addr.ip()).collect();
            if ips.is_empty() {
                Err(EgressError::ResolutionFailed {
                    host: host.to_string(),
                    detail: "resolution returned no addresses".to_string(),
                })
            } else {
                Ok(ips)
            }
        }
        Ok(Err(error)) => Err(EgressError::ResolutionFailed {
            host: host.to_string(),
            detail: error.to_string(),
        }),
        Err(_elapsed) => Err(EgressError::ResolutionFailed {
            host: host.to_string(),
            detail: format!("resolution exceeded {RESOLUTION_TIMEOUT:?}"),
        }),
    }
}

/// Resolves `url`'s host exactly once, validates every address that
/// resolution returned, and returns the single validated
/// `(host, SocketAddr)` pair the caller must pin the connection to. This is
/// the resolve-then-pin step: [`fetch`] never asks anything (resolver or
/// HTTP client) to resolve the same host a second time for the same hop.
async fn resolve_and_pin(url: &Url) -> Result<(String, SocketAddr), EgressError> {
    let host = url.host_str().ok_or(EgressError::MissingHost)?.to_string();
    let port = url
        .port_or_known_default()
        .ok_or_else(|| EgressError::InvalidUrl("url has no resolvable port".to_string()))?;
    let candidates = resolve_once(&host, port).await?;
    let ip = select_pinned_address(&host, &candidates)?;
    Ok((host, SocketAddr::new(ip, port)))
}

/// Counts redirect hops and fails the moment more than `max` have been
/// followed. Used by [`fetch_inner`] instead of an inline loop counter so
/// the cap is directly unit-testable, the same way [`BodyLimiter`] makes
/// the size cap directly testable.
struct RedirectBudget {
    max: u8,
    followed: u8,
}

impl RedirectBudget {
    fn new(max: u8) -> Self {
        Self { max, followed: 0 }
    }

    /// Call once per redirect hop actually followed. Errs on the hop that
    /// would take the count past `max`.
    fn consume(&mut self) -> Result<(), EgressError> {
        if self.followed >= self.max {
            return Err(EgressError::TooManyRedirects);
        }
        self.followed += 1;
        Ok(())
    }
}

/// A byte counter that fails the moment it would exceed `limit`, used to
/// cap a response body while it is still streaming in rather than after
/// the fact.
struct BodyLimiter {
    limit: u64,
    seen: u64,
}

impl BodyLimiter {
    fn new(limit: u64) -> Self {
        Self { limit, seen: 0 }
    }

    fn push(&mut self, additional: usize) -> Result<(), EgressError> {
        self.seen = self.seen.saturating_add(additional as u64);
        if self.seen > self.limit {
            Err(EgressError::ResponseTooLarge { limit: self.limit })
        } else {
            Ok(())
        }
    }
}

/// Fetches `url` under the egress guard: scheme/deny-list checks, DNS
/// resolve-then-pin, a capped redirect chain (every hop re-validated and
/// re-pinned from scratch), a capped response body, and an overall wall-clock
/// budget. This is the only function in this module that touches the
/// network.
pub async fn fetch(url: &str) -> Result<Vec<u8>, EgressError> {
    match tokio::time::timeout(TOTAL_FETCH_BUDGET, fetch_inner(url)).await {
        Ok(result) => result,
        Err(_elapsed) => Err(EgressError::Timeout),
    }
}

async fn fetch_inner(url: &str) -> Result<Vec<u8>, EgressError> {
    let mut current = egress_guard(url)?;
    let mut redirects = RedirectBudget::new(MAX_REDIRECTS);

    loop {
        let (host, pinned_addr) = resolve_and_pin(&current).await?;

        let client = reqwest::Client::builder()
            .resolve(&host, pinned_addr)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|error| EgressError::Request(error.to_string()))?;

        let response = client
            .get(current.clone())
            .send()
            .await
            .map_err(|error| EgressError::Request(error.to_string()))?;

        if response.status().is_redirection() {
            redirects.consume()?;
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .ok_or_else(|| EgressError::BadRedirect("missing Location header".to_string()))?
                .to_str()
                .map_err(|_| {
                    EgressError::BadRedirect("Location header is not valid UTF-8".to_string())
                })?;
            let next = current
                .join(location)
                .map_err(|error| EgressError::BadRedirect(error.to_string()))?;
            // Re-run the full guard (scheme + literal-IP deny check) on the
            // redirect target: a permitted host can redirect to
            // 169.254.169.254, and this is what catches that.
            current = egress_guard(next.as_str())?;
            continue;
        }

        return read_capped_body(response).await;
    }
}

async fn read_capped_body(mut response: reqwest::Response) -> Result<Vec<u8>, EgressError> {
    if let Some(len) = response.content_length()
        && len > MAX_RESPONSE_BODY_BYTES
    {
        return Err(EgressError::ResponseTooLarge {
            limit: MAX_RESPONSE_BODY_BYTES,
        });
    }

    let mut limiter = BodyLimiter::new(MAX_RESPONSE_BODY_BYTES);
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| EgressError::Request(error.to_string()))?
    {
        limiter.push(chunk.len())?;
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Step 1: the literal test from the task, verbatim. ---

    #[test]
    fn the_egress_guard_denies_private_and_metadata_destinations() {
        for url in [
            "http://127.0.0.1/",
            "http://10.0.0.1/",
            "http://169.254.169.254/",
            "http://[::1]/",
            "http://192.168.8.20/",
        ] {
            assert!(
                matches!(egress_guard(url), Err(EgressError::Denied { .. })),
                "{url}"
            );
        }
    }

    // --- Broadened: one assertion per denied class, each naming its own
    // reason so a future edit that silently swaps in the wrong reason (or
    // stops denying a class while still denying a sibling) fails loudly. ---

    #[test]
    fn denies_rfc1918_10_block() {
        assert!(matches!(
            egress_guard("http://10.1.2.3/"),
            Err(EgressError::Denied {
                reason: DenyReason::Rfc1918Private,
                ..
            })
        ));
    }

    #[test]
    fn denies_rfc1918_172_16_block() {
        for host in ["172.16.0.1", "172.31.255.255"] {
            let url = format!("http://{host}/");
            assert!(
                matches!(
                    egress_guard(&url),
                    Err(EgressError::Denied {
                        reason: DenyReason::Rfc1918Private,
                        ..
                    })
                ),
                "{url}"
            );
        }
        // 172.32.0.0 is outside 172.16/12 and must NOT be denied as private.
        assert!(!matches!(
            egress_guard("http://172.32.0.1/"),
            Err(EgressError::Denied {
                reason: DenyReason::Rfc1918Private,
                ..
            })
        ));
    }

    #[test]
    fn denies_rfc1918_192_168_block() {
        assert!(matches!(
            egress_guard("http://192.168.0.1/"),
            Err(EgressError::Denied {
                reason: DenyReason::Rfc1918Private,
                ..
            })
        ));
    }

    #[test]
    fn denies_ipv4_loopback() {
        assert!(matches!(
            egress_guard("http://127.0.0.5/"),
            Err(EgressError::Denied {
                reason: DenyReason::Loopback,
                ..
            })
        ));
    }

    #[test]
    fn denies_ipv6_loopback() {
        assert!(matches!(
            egress_guard("http://[::1]/"),
            Err(EgressError::Denied {
                reason: DenyReason::Loopback,
                ..
            })
        ));
    }

    #[test]
    fn denies_ipv4_link_local() {
        assert!(matches!(
            egress_guard("http://169.254.1.1/"),
            Err(EgressError::Denied {
                reason: DenyReason::LinkLocal,
                ..
            })
        ));
    }

    #[test]
    fn denies_ipv6_link_local() {
        assert!(matches!(
            egress_guard("http://[fe80::1]/"),
            Err(EgressError::Denied {
                reason: DenyReason::LinkLocal,
                ..
            })
        ));
    }

    #[test]
    fn denies_cloud_metadata_address_by_name() {
        assert!(matches!(
            egress_guard("http://169.254.169.254/latest/meta-data/"),
            Err(EgressError::Denied {
                reason: DenyReason::CloudMetadata,
                ..
            })
        ));
    }

    #[test]
    fn denies_ipv6_unique_local() {
        assert!(matches!(
            egress_guard("http://[fc00::1]/"),
            Err(EgressError::Denied {
                reason: DenyReason::UniqueLocalV6,
                ..
            })
        ));
        assert!(matches!(
            egress_guard("http://[fd12:3456:789a::1]/"),
            Err(EgressError::Denied {
                reason: DenyReason::UniqueLocalV6,
                ..
            })
        ));
    }

    #[test]
    fn denies_ipv4_mapped_ipv6_forms() {
        // ::ffff:127.0.0.1 -> loopback once unwrapped.
        assert!(matches!(
            egress_guard("http://[::ffff:127.0.0.1]/"),
            Err(EgressError::Denied {
                reason: DenyReason::Loopback,
                ..
            })
        ));
        // ::ffff:10.0.0.1 -> RFC1918 once unwrapped.
        assert!(matches!(
            egress_guard("http://[::ffff:10.0.0.1]/"),
            Err(EgressError::Denied {
                reason: DenyReason::Rfc1918Private,
                ..
            })
        ));
        // ::ffff:169.254.169.254 -> cloud metadata once unwrapped.
        assert!(matches!(
            egress_guard("http://[::ffff:169.254.169.254]/"),
            Err(EgressError::Denied {
                reason: DenyReason::CloudMetadata,
                ..
            })
        ));
    }

    #[test]
    fn denies_unspecified_and_broadcast_and_multicast() {
        assert!(matches!(
            egress_guard("http://0.0.0.0/"),
            Err(EgressError::Denied {
                reason: DenyReason::Unspecified,
                ..
            })
        ));
        assert!(matches!(
            egress_guard("http://255.255.255.255/"),
            Err(EgressError::Denied {
                reason: DenyReason::Broadcast,
                ..
            })
        ));
        assert!(matches!(
            egress_guard("http://224.0.0.1/"),
            Err(EgressError::Denied {
                reason: DenyReason::Multicast,
                ..
            })
        ));
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

    #[test]
    fn permits_a_globally_routable_literal_ip() {
        assert!(egress_guard("http://93.184.216.34/").is_ok());
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

    #[tokio::test(start_paused = true)]
    async fn total_fetch_budget_rejects_work_that_runs_one_moment_past_it() {
        let over_budget = tokio::time::sleep(TOTAL_FETCH_BUDGET + Duration::from_millis(1));
        let outcome = tokio::time::timeout(TOTAL_FETCH_BUDGET, over_budget).await;
        assert!(outcome.is_err(), "expected the budget to be exceeded");
        // fetch() maps exactly this Elapsed into EgressError::Timeout.
        let mapped: Result<(), EgressError> = outcome.map_err(|_elapsed| EgressError::Timeout);
        assert!(matches!(mapped, Err(EgressError::Timeout)));
    }

    #[tokio::test(start_paused = true)]
    async fn total_fetch_budget_permits_work_that_finishes_inside_it() {
        let under_budget = tokio::time::sleep(
            TOTAL_FETCH_BUDGET
                .checked_sub(Duration::from_millis(1))
                .expect("TOTAL_FETCH_BUDGET is well over 1ms"),
        );
        let outcome = tokio::time::timeout(TOTAL_FETCH_BUDGET, under_budget).await;
        assert!(outcome.is_ok());
    }

    // --- Redirect-target re-validation: proves a redirect Location is run
    // back through egress_guard, not merely joined and trusted. ---

    #[test]
    fn a_redirect_target_that_points_at_a_denied_address_is_denied() {
        let base = Url::parse("https://example.com/start").unwrap();
        let next = base.join("http://169.254.169.254/secret").unwrap();
        assert!(matches!(
            egress_guard(next.as_str()),
            Err(EgressError::Denied {
                reason: DenyReason::CloudMetadata,
                ..
            })
        ));
    }

    #[test]
    fn a_relative_redirect_target_resolves_against_its_base() {
        let base = Url::parse("https://example.com/a/start").unwrap();
        let next = base.join("../b/next").unwrap();
        assert_eq!(next.as_str(), "https://example.com/b/next");
        assert!(egress_guard(next.as_str()).is_ok());
    }
}
