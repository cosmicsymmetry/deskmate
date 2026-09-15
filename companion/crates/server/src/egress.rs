//! SSRF egress guard for the server's one outbound call: the OAuth token
//! endpoint.
//!
//! This module was written for plugin data fetches -- owner-supplied URLs the
//! server fetched on a plugin's behalf -- and schema v9 removed those. What
//! survives is the credential path, and it is now bounded twice over: a
//! positive allowlist of exactly one host ([`IDENTITY_HOST`]), and, behind it,
//! the original address-level deny of RFC1918, loopback, link-local and
//! `169.254.169.254` with resolve-then-pin against DNS rebinding.
//!
//! Both layers earn their place. The allowlist states the policy -- the server
//! is a credential custodian and fetches no card content (revision spec §1) --
//! while the address checks stop a DNS answer for that one permitted host from
//! pointing the pinned connection at a homelab neighbour or, on a cloud VM, at
//! the metadata endpoint. This module is the only place allowed to decide
//! "yes, fetch that" and the only place that performs the fetch.
//!
//! # Why hostname validation alone is not enough
//!
//! Checking a URL's hostname and then handing the URL to an HTTP client is
//! not sufficient: the client performs its own DNS lookup at connect time,
//! and the answer can differ from whatever the guard saw a moment earlier
//! (DNS rebinding). [`fetch_post_form`] instead resolves the host exactly once,
//! validates every address that resolution returned, and then pins the
//! HTTP client to that *specific validated address* via
//! `reqwest::ClientBuilder::resolve`, which overrides reqwest's own
//! resolver for that request. reqwest never re-resolves the host itself.
//!
//! # What this does not guarantee
//!
//! `reqwest::ClientBuilder::resolve` is a documented, first-class reqwest
//! API for exactly this purpose, and is trusted here to do what it says:
//! connect to the pinned `SocketAddr` rather than re-resolving (this
//! assumption itself is pinned by a characterization test in this module's
//! tests: a loopback server, a client built with `.resolve()` alone -- no
//! `egress_guard` involved -- and an assertion that the `Host` header the
//! server observed is the pinned name, not the literal address). This
//! module does not independently verify reqwest's TCP connect behaviour
//! beyond that, so a defect inside reqwest's connector itself is outside
//! what this guard can catch. Every redirect hop is re-validated and
//! re-pinned from scratch (a permitted host can redirect to
//! `169.254.169.254`), but a single connection is not continuously
//! re-checked against TOCTOU races faster than one DNS resolution.
//!
//! Every outbound client also disables reqwest's proxy support
//! (`.no_proxy()`): `auto_sys_proxy` defaults to true, and the underlying
//! connector reads `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY` from the
//! environment unconditionally. Left enabled, any of those variables being
//! set would route the connection through a proxy instead of the pinned
//! address, silently defeating resolve-then-pin.
//!
//! # Allowlist, not a remembered deny list
//!
//! [`deny_reason_v4`]/[`deny_reason_v6`] implement "permit only
//! globally-routable unicast, deny everything else" (spec §5 calls this an
//! *allowlist*): permission is what is left over once every named
//! exclusion has been checked, not a list of ranges someone remembered to
//! write down. This module's first version was a genuine deny list and
//! missed `100.64.0.0/10` (RFC 6598, carrier-grade NAT) -- which is
//! Tailscale's entire address range, and this deployment's own render host
//! reaches its neighbours over Tailscale.
//!
//! **The two families are not equally exhaustive, and that asymmetry is
//! deliberate rather than an oversight left unstated.** `deny_reason_v4`
//! was independently audited against nightly `Ipv4Addr::is_global()` and
//! found to deny a strict superset of what it denies -- i.e. it is checked
//! exhaustive against the IANA IPv4 Special-Purpose Address Registry.
//! `deny_reason_v6` checks a substantially wider set than its first version
//! (loopback, unspecified, link-local, unique-local, the deprecated
//! site-local range, multicast, the documentation range, the well-known and
//! local-use NAT64 prefixes, the `2001::/23` IETF protocol assignment block
//! -- Teredo, benchmarking, `ORCHIDv2` -- 6to4, the discard-only range, the
//! `SRv6` SID space, and both legacy IPv4-in-IPv6 encodings), but has **not**
//! been independently audited against the full IANA IPv6 Special-Purpose
//! Address Registry the way the IPv4 side has, so it should not be trusted
//! as proven exhaustive. Everything in it embeds or is adjacent to an
//! attacker-influenced address (most concretely: `2002:a9fe:a9fe::1` is the
//! 6to4 encoding of the cloud metadata address `169.254.169.254`, and is
//! denied by name in this module's tests) or is otherwise non-globally-
//! routable, but a still-undiscovered IPv6 special-purpose range is a live
//! possibility in a way an undiscovered IPv4 one is not.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use reqwest::Url;

/// The well-known cloud instance-metadata address (AWS/GCP/Azure/etc.),
/// called out by the spec separately from the RFC 3927 link-local block it
/// technically already falls inside -- deny it by name so a denial reads as
/// "this is the metadata endpoint," not merely "this is link-local".
const CLOUD_METADATA_ADDR: Ipv4Addr = Ipv4Addr::new(169, 254, 169, 254);

/// How this server identifies itself to the hosts it fetches from.
///
/// One name for every outbound request, so an operator reading their access
/// log sees one client rather than a blank line.
pub const USER_AGENT: &str = concat!("deskmate-server/", env!("CARGO_PKG_VERSION"));

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

/// Maximum response body size accepted from the token endpoint.
pub const MAX_RESPONSE_BODY_BYTES: u64 = 2 * 1024 * 1024;

/// Why a fetch was refused. Every variant is meant to be safe to log and to
/// surface to an operator; none carry response bodies or secrets.
#[derive(Debug, thiserror::Error)]
pub enum EgressError {
    /// The URL could not be parsed at all.
    #[error("invalid url: {0}")]
    InvalidUrl(String),
    /// The URL's scheme was not `http` or `https`.
    #[error("unsupported scheme: {0}")]
    UnsupportedScheme(String),
    /// The URL had no host component.
    #[error("url has no host")]
    MissingHost,
    /// `host` (or one of the addresses it resolved to) is on the deny list.
    #[error("egress denied to {host}: {reason}")]
    Denied { host: String, reason: DenyReason },
    /// DNS resolution for `host` failed or timed out.
    #[error("dns resolution failed for {host}: {detail}")]
    ResolutionFailed { host: String, detail: String },
    /// The response body exceeded [`MAX_RESPONSE_BODY_BYTES`].
    #[error("response exceeded {limit}-byte cap")]
    ResponseTooLarge { limit: u64 },
    /// The fetch did not complete inside [`TOTAL_FETCH_BUDGET`].
    #[error("fetch exceeded {:?} time budget", TOTAL_FETCH_BUDGET)]
    Timeout,
    /// The underlying HTTP client reported an error (connect failure,
    /// protocol error, etc.).
    #[error("request failed: {0}")]
    Request(String),
    /// Too many redirect hops.
    #[error("too many redirects (limit {})", MAX_REDIRECTS)]
    TooManyRedirects,
    /// A redirect response had a missing or unusable `Location` header.
    #[error("bad redirect: {0}")]
    BadRedirect(String),
    /// The host is not [`IDENTITY_HOST`]. The server is a credential custodian,
    /// not a content fetcher; see [`IDENTITY_HOST`].
    #[error("host {host} is not the permitted identity host")]
    HostNotPermitted { host: String },
}

/// Why an address is not globally-routable unicast, and therefore denied.
/// Every variant names a distinct IANA special-purpose range so a denial is
/// always informative, even though the *decision* to deny is a positive
/// allowlist (see [`deny_reason_v4`]/[`deny_reason_v6`]), not a match against
/// this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyReason {
    Loopback,
    LinkLocal,
    CloudMetadata,
    Rfc1918Private,
    /// `100.64.0.0/10` (RFC 6598) -- shared address space for carrier-grade
    /// NAT. This is Tailscale's entire range: the render host's own
    /// tailnet, and every other tailnet reachable from it, lives here.
    CarrierGradeNat,
    UniqueLocalV6,
    /// `fec0::/10` -- the deprecated IPv6 site-local range (RFC 3879).
    SiteLocalV6,
    Unspecified,
    Multicast,
    Broadcast,
    /// `192.0.0.0/24` (RFC 6890) -- IETF protocol assignments.
    IetfProtocolAssignment,
    /// The IPv4 TEST-NET ranges (`192.0.2.0/24`, `198.51.100.0/24`,
    /// `203.0.113.0/24`, RFC 5737) or the IPv6 documentation range
    /// (`2001:db8::/32`, RFC 3849).
    Documentation,
    /// `198.18.0.0/15` (RFC 2544) -- reserved for network benchmarking.
    Benchmarking,
    /// `64:ff9b::/96` (RFC 6052) -- the well-known NAT64 prefix. Its low 32
    /// bits carry an attacker-chosen embedded address, so it is denied
    /// outright rather than trusted.
    Nat64V6,
    /// `::a.b.c.d` (RFC 4291 §2.5.5.1) -- the deprecated IPv4-compatible
    /// IPv6 form: top 64 bits AND bits 64-95 all zero. Distinct from the
    /// IPv4-*mapped* form (`::ffff:a.b.c.d`, handled separately by
    /// unwrapping to the embedded address before this classifier runs).
    Ipv4CompatibleV6,
    /// Top 64 bits zero, but bits 64-95 are *not* also zero (so this is not
    /// [`Ipv4CompatibleV6`]) -- e.g. the RFC 2765 IPv4-*translated* form
    /// `::ffff:0:a.b.c.d` (bits 64-79 == `0xffff`, a different bit position
    /// than the IPv4-mapped form's bits 80-95). Every such address embeds
    /// an attacker-chosen low-32-bit payload, so the whole `::/64` prefix
    /// is denied regardless of what occupies bits 64-95.
    Ipv4TranslatedV6,
    /// `2001::/23` (RFC 6890, "IETF Protocol Assignments") -- covers
    /// Teredo (`2001::/32`), benchmarking (`2001:2::/48`), and `ORCHIDv2`
    /// (`2001:20::/28`) as sub-blocks; denied as one range rather than
    /// three, since none of it is globally-routable unicast.
    Ipv6ProtocolAssignment,
    /// `2002::/16` (RFC 3056) -- 6to4. Every address in this block encodes
    /// an IPv4 address in its next 32 bits (`2002:AABB:CCDD::/48` <->
    /// `AA.BB.CC.DD`), attacker-chosen and unchecked by this range alone --
    /// e.g. `2002:a9fe:a9fe::1` is the 6to4 encoding of the cloud metadata
    /// address `169.254.169.254`.
    SixToFourV6,
    /// `64:ff9b:1::/48` (RFC 8215) -- the *local-use* NAT64 prefix,
    /// distinct from the well-known prefix ([`Nat64V6`]). Same reasoning:
    /// its low bits carry an attacker-chosen embedded address.
    Nat64LocalUseV6,
    /// `100::/64` (RFC 6666) -- "discard-only" address space.
    DiscardOnlyV6,
    /// `5f00::/16` -- the `SRv6` SID space (IANA-registered, not globally
    /// routable unicast).
    Srv6V6,
    /// Any other IANA-reserved, non-globally-routable IPv4 block (e.g.
    /// `0.0.0.0/8` beyond the single unspecified address, the deprecated
    /// 6to4 relay anycast range `192.88.99.0/24`, or `240.0.0.0/4`).
    Reserved,
}

impl fmt::Display for DenyReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            DenyReason::Loopback => "loopback",
            DenyReason::LinkLocal => "link-local",
            DenyReason::CloudMetadata => "cloud metadata endpoint",
            DenyReason::Rfc1918Private => "RFC1918 private",
            DenyReason::CarrierGradeNat => "carrier-grade NAT (100.64.0.0/10)",
            DenyReason::UniqueLocalV6 => "unique-local IPv6 (fc00::/7)",
            DenyReason::SiteLocalV6 => "deprecated site-local IPv6 (fec0::/10)",
            DenyReason::Unspecified => "unspecified address",
            DenyReason::Multicast => "multicast",
            DenyReason::Broadcast => "broadcast",
            DenyReason::IetfProtocolAssignment => "IETF protocol assignment (192.0.0.0/24)",
            DenyReason::Documentation => "documentation/test-net range",
            DenyReason::Benchmarking => "benchmarking range (198.18.0.0/15)",
            DenyReason::Nat64V6 => "NAT64 well-known prefix (64:ff9b::/96)",
            DenyReason::Ipv4CompatibleV6 => "deprecated IPv4-compatible IPv6",
            DenyReason::Ipv4TranslatedV6 => "IPv4-translated IPv6 (::ffff:0:0/96)",
            DenyReason::Ipv6ProtocolAssignment => "IETF protocol assignment (2001::/23)",
            DenyReason::SixToFourV6 => "6to4 (2002::/16)",
            DenyReason::Nat64LocalUseV6 => "local-use NAT64 prefix (64:ff9b:1::/48)",
            DenyReason::DiscardOnlyV6 => "discard-only range (100::/64)",
            DenyReason::Srv6V6 => "SRv6 SID space (5f00::/16)",
            DenyReason::Reserved => "reserved, not globally routable",
        };
        f.write_str(label)
    }
}

/// Classifies `ip` against the allowlist. `None` means `ip` is a globally
/// routable unicast address and is permitted; every `Some` is a positive
/// finding that `ip` falls inside a specific IANA special-purpose range.
///
/// The predicate this implements is **"permit only globally-routable
/// unicast, deny everything else"** (spec §5 names it an *allowlist*, and a
/// deny list is a list of what someone remembered -- CGNAT was the range
/// this module's first version forgot); permission is what is left over
/// once every named exclusion has been checked, not a remembered blocklist
/// of "the bad ones". `Ipv4Addr::is_global()` and friends would express the
/// same predicate, but they are unstable (nightly-only); this reimplements
/// the stable subset directly (loopback, link-local, private, broadcast,
/// multicast) and the rest (CGNAT, IETF protocol assignment,
/// documentation/TEST-NET, benchmarking, the broader reserved ranges, and
/// their IPv6 counterparts) by explicit CIDR arithmetic below, rather than
/// pulling in a crate for a security boundary that most needs to stay
/// readable in this file. **`deny_reason_v4` is checked exhaustive against
/// the IANA IPv4 registry (see the module doc's "Allowlist" section);
/// `deny_reason_v6` covers a wide, deliberately-widened set of IPv6
/// special-purpose ranges but is not independently proven exhaustive the
/// same way** -- do not read the two functions as equally complete.
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

/// `true` only for an address this module has positively confirmed is
/// globally-routable unicast -- i.e. `deny_reason_for_ip` found no
/// applicable exclusion. Exposed mainly for tests/documentation: `fetch`
/// and `egress_guard` call `deny_reason_for_ip`/`deny_reason_v4` directly
/// so the informative `DenyReason` is available on the deny path.
#[must_use]
pub fn is_globally_routable(ip: IpAddr) -> bool {
    deny_reason_for_ip(ip).is_none()
}

/// The part of the IANA IPv4 special-purpose registry std has no stable
/// classifier for. Each entry is `(base, prefix_len, reason)`; the first
/// matching CIDR wins. Checked by [`deny_reason_v4`] only after every
/// stable `std::net::Ipv4Addr` classifier (loopback, link-local, private,
/// broadcast, multicast) has already come back negative.
const EXTRA_SPECIAL_USE_V4: &[(Ipv4Addr, u32, DenyReason)] = &[
    // 0.0.0.0/8 beyond 0.0.0.0 itself ("this network", RFC 1122 §3.2.1.3).
    (Ipv4Addr::UNSPECIFIED, 8, DenyReason::Reserved),
    // Shared address space for carrier-grade NAT (RFC 6598) -- Tailscale's
    // entire range, and the docker-vm render host's own Tailscale address,
    // live here.
    (
        Ipv4Addr::new(100, 64, 0, 0),
        10,
        DenyReason::CarrierGradeNat,
    ),
    (
        Ipv4Addr::new(192, 0, 0, 0),
        24,
        DenyReason::IetfProtocolAssignment,
    ),
    (Ipv4Addr::new(192, 0, 2, 0), 24, DenyReason::Documentation), // TEST-NET-1
    // Deprecated 6to4 relay anycast (RFC 7526).
    (Ipv4Addr::new(192, 88, 99, 0), 24, DenyReason::Reserved),
    (Ipv4Addr::new(198, 18, 0, 0), 15, DenyReason::Benchmarking),
    (
        Ipv4Addr::new(198, 51, 100, 0),
        24,
        DenyReason::Documentation,
    ), // TEST-NET-2
    (Ipv4Addr::new(203, 0, 113, 0), 24, DenyReason::Documentation), // TEST-NET-3
    // Class E / "reserved for future use" (includes 255.255.255.255,
    // already caught above by `is_broadcast`).
    (Ipv4Addr::new(240, 0, 0, 0), 4, DenyReason::Reserved),
];

fn deny_reason_v4(ip: Ipv4Addr) -> Option<DenyReason> {
    // The stable, well-tested std classifiers first.
    if ip == CLOUD_METADATA_ADDR {
        return Some(DenyReason::CloudMetadata);
    }
    if ip.is_unspecified() {
        return Some(DenyReason::Unspecified);
    }
    if ip.is_loopback() {
        return Some(DenyReason::Loopback);
    }
    if ip.is_link_local() {
        return Some(DenyReason::LinkLocal);
    }
    if ip.is_private() {
        return Some(DenyReason::Rfc1918Private);
    }
    if ip.is_broadcast() {
        return Some(DenyReason::Broadcast);
    }
    if ip.is_multicast() {
        return Some(DenyReason::Multicast);
    }

    // The rest of the IANA IPv4 special-purpose registry.
    EXTRA_SPECIAL_USE_V4
        .iter()
        .find(|(base, prefix, _)| v4_in_cidr(ip, *base, *prefix))
        .map(|(_, _, reason)| *reason)
}

/// `true` if `ip` falls inside `base/prefix_len`. `prefix_len` is always a
/// small compile-time constant from the tables above, never
/// attacker-controlled.
fn v4_in_cidr(ip: Ipv4Addr, base: Ipv4Addr, prefix_len: u32) -> bool {
    let mask: u32 = if prefix_len == 0 {
        0
    } else {
        u32::MAX << (32 - prefix_len)
    };
    (u32::from(ip) & mask) == (u32::from(base) & mask)
}

fn deny_reason_v6(ip: Ipv6Addr) -> Option<DenyReason> {
    const SITE_LOCAL_MASK: u16 = 0xffc0; // fec0::/10, deprecated (RFC 3879)
    const SITE_LOCAL_PREFIX: u16 = 0xfec0;

    if ip.is_loopback() {
        return Some(DenyReason::Loopback);
    }
    if ip.is_unspecified() {
        return Some(DenyReason::Unspecified);
    }

    let segments = ip.segments();
    let leading = segments[0];
    if ip.is_unicast_link_local() {
        return Some(DenyReason::LinkLocal);
    }
    if ip.is_unique_local() {
        return Some(DenyReason::UniqueLocalV6);
    }
    if leading & SITE_LOCAL_MASK == SITE_LOCAL_PREFIX {
        return Some(DenyReason::SiteLocalV6);
    }
    if ip.is_multicast() {
        return Some(DenyReason::Multicast);
    }
    // 2001:db8::/32 -- documentation (RFC 3849).
    if segments[0] == 0x2001 && segments[1] == 0x0db8 {
        return Some(DenyReason::Documentation);
    }
    // 64:ff9b::/96 -- the well-known NAT64 prefix (RFC 6052): its low 32
    // bits carry an embedded address a resolver can be tricked into
    // synthesizing, so it is denied outright rather than trusted.
    if segments[0] == 0x0064
        && segments[1] == 0xff9b
        && segments[2] == 0
        && segments[3] == 0
        && segments[4] == 0
        && segments[5] == 0
    {
        return Some(DenyReason::Nat64V6);
    }
    // 2001::/23 (RFC 6890, "IETF Protocol Assignments"): Teredo
    // (2001::/32), benchmarking (2001:2::/48), and ORCHIDv2 (2001:20::/28)
    // all live in this block. Distinct from -- and does not overlap --
    // the 2001:db8::/32 documentation check above (segments[1] there is
    // 0x0db8, far outside this /23's segments[1] range of 0x0000-0x01ff).
    if segments[0] == 0x2001 && (segments[1] & 0xfe00) == 0x0000 {
        return Some(DenyReason::Ipv6ProtocolAssignment);
    }
    // 2002::/16 -- 6to4 (RFC 3056). Every address here encodes an IPv4
    // address in the next 32 bits, attacker-chosen and unchecked by the
    // range alone: 2002:a9fe:a9fe::1 is the 6to4 encoding of the cloud
    // metadata address 169.254.169.254.
    if segments[0] == 0x2002 {
        return Some(DenyReason::SixToFourV6);
    }
    // 64:ff9b:1::/48 -- the *local-use* NAT64 prefix (RFC 8215), distinct
    // from the well-known 64:ff9b::/96 prefix checked above.
    if segments[0] == 0x0064 && segments[1] == 0xff9b && segments[2] == 0x0001 {
        return Some(DenyReason::Nat64LocalUseV6);
    }
    // 100::/64 -- discard-only address space (RFC 6666).
    if segments[0] == 0x0100 && segments[1] == 0 && segments[2] == 0 && segments[3] == 0 {
        return Some(DenyReason::DiscardOnlyV6);
    }
    // 5f00::/16 -- the SRv6 SID space.
    if segments[0] == 0x5f00 {
        return Some(DenyReason::Srv6V6);
    }
    // A zero `::/64` prefix, regardless of what sits in bits 64-95: this is
    // deliberately broader than "top 96 bits zero" so it also catches the
    // RFC 2765 IPv4-*translated* form `::ffff:0:a.b.c.d` (0xffff sits at
    // bits 64-79 there, a different position than the IPv4-*mapped* form's
    // bits 80-95, which `deny_reason_for_ip` has already unwrapped upstream
    // of this function). `::` and `::1` are already excluded above by
    // is_unspecified/is_loopback, so every address reaching this check with
    // a zero `::/64` prefix carries an attacker-chosen low-bits payload one
    // way or another.
    if segments[0] == 0 && segments[1] == 0 && segments[2] == 0 && segments[3] == 0 {
        if segments[4] == 0 && segments[5] == 0 {
            return Some(DenyReason::Ipv4CompatibleV6);
        }
        return Some(DenyReason::Ipv4TranslatedV6);
    }
    None
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

/// What [`post_form_inner`] needs from DNS resolution: given the
/// current URL, produce the `(host, SocketAddr)` to pin the connection to.
///
/// Production always uses [`RealResolver`] (real DNS via [`resolve_and_pin`],
/// hence real validation of every resolved address). Tests inject
/// [`FixedAddrResolver`], which points the *connection* at a loopback
/// wiremock while leaving `egress_guard` -- which still runs, unmodified, on
/// the POST path -- to validate the URL exactly as
/// production does. This is what makes the redirect chain, the body cap and
/// the total time budget testable end-to-end against a real HTTP server:
/// the seam is DNS resolution, never the deny-list, so a loopback address is
/// still correctly refused everywhere the guard itself runs.
trait HopResolver {
    async fn resolve(&self, url: &Url) -> Result<(String, SocketAddr), EgressError>;
}

struct RealResolver;

impl HopResolver for RealResolver {
    async fn resolve(&self, url: &Url) -> Result<(String, SocketAddr), EgressError> {
        resolve_and_pin(url).await
    }
}

/// The result of a fetch that ran to completion under the egress guard:
/// the HTTP status the final (non-redirect) response returned, alongside
/// its capped body. Added on top of the guard itself (fix round 1 of Task
/// 7b): [`fetch`] previously returned only `Vec<u8>`, discarding status
/// entirely, which made a 503 and a 200 indistinguishable to every caller
/// -- exactly what let a transient server error masquerade as either a
/// permanent parse failure or, worse, a successful response. A redirect
/// status now arrives here as itself: the POST client disables redirect
/// following, and the token endpoint has no reason to issue one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// `reqwest::Error`'s `Display` is usually just the outermost frame (e.g.
/// "error sending request for url (...)"); the actually useful cause lives
/// in its `source()` chain. Walk it so `EgressError::Request` messages are
/// diagnosable rather than generic.
fn describe_reqwest_error(error: &reqwest::Error) -> String {
    let mut message = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(inner) = source {
        message.push_str(": ");
        message.push_str(&inner.to_string());
        source = inner.source();
    }
    message
}

/// Builds the reqwest client used for one pinned hop, shared by the GET
/// ([`fetch`]) and POST ([`fetch_post_form`]) paths so both get identical
/// resolve-then-pin, no-redirect, timeout and no-proxy behaviour.
///
/// DO NOT REMOVE `.no_proxy()`: reqwest's `auto_sys_proxy` defaults to true and
/// the underlying hyper-util connector reads `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY`
/// from the environment unconditionally. If any were set, the connection would
/// go to the proxy instead of `pinned_addr`, silently defeating resolve-then-pin
/// -- the `.resolve()` override would never be consulted. `.no_proxy()` clears
/// any configured proxy and disables that environment lookup.
fn build_pinned_client(
    host: &str,
    pinned_addr: SocketAddr,
) -> Result<reqwest::Client, EgressError> {
    reqwest::Client::builder()
        .resolve(host, pinned_addr)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(REQUEST_TIMEOUT)
        .no_proxy()
        .build()
        .map_err(|error| EgressError::Request(describe_reqwest_error(&error)))
}

/// Fetches `url` under the egress guard: scheme/deny-list checks, DNS
/// resolve-then-pin, a capped redirect chain (every hop re-validated and
/// re-pinned from scratch), a capped response body, and an overall wall-clock
/// budget. This is the only function in this module that touches the
/// network.
pub async fn fetch(url: &str) -> Result<FetchResponse, EgressError> {
    fetch_with_resolver(url, &RealResolver).await
}

/// The whole of [`fetch`]'s behaviour -- including the total-budget
/// wrapping -- parameterized over the DNS step *and* the budget itself, so
/// tests exercise the exact same composition production uses rather than a
/// parallel reimplementation of it. Production always calls this through
/// [`fetch_with_resolver`], which fixes `total_budget` at
/// [`TOTAL_FETCH_BUDGET`]; tests pass a millisecond-scale budget instead so
/// the over-budget case is proven with a real (not virtual/paused) clock in
/// milliseconds rather than tens of real seconds. [`REQUEST_TIMEOUT`] is
/// unaffected either way -- it stays the real per-hop constant, comfortably
/// larger than any test delay used against it.
async fn fetch_with_budget(
    url: &str,
    resolver: &impl HopResolver,
    total_budget: Duration,
) -> Result<FetchResponse, EgressError> {
    match tokio::time::timeout(total_budget, fetch_inner(url, resolver)).await {
        Ok(result) => result,
        Err(_elapsed) => Err(EgressError::Timeout),
    }
}

async fn fetch_with_resolver(
    url: &str,
    resolver: &impl HopResolver,
) -> Result<FetchResponse, EgressError> {
    fetch_with_budget(url, resolver, TOTAL_FETCH_BUDGET).await
}

async fn fetch_inner(url: &str, resolver: &impl HopResolver) -> Result<FetchResponse, EgressError> {
    let mut current = egress_guard(url)?;
    let mut redirects = RedirectBudget::new(MAX_REDIRECTS);

    loop {
        let (host, pinned_addr) = resolver.resolve(&current).await?;

        let client = reqwest::Client::builder()
            .resolve(&host, pinned_addr)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(REQUEST_TIMEOUT)
            // reqwest sends no User-Agent at all unless one is set, and a
            // Cloudflare-fronted API answers that with 403 before it ever
            // looks at the path -- which is how the CoinGecko fetch failed
            // with "provider returned HTTP 403" while curl to the same URL
            // worked. `tools/picture-producers/claude_limits_png.py` carries
            // the same note for urllib. This is identifying ourselves, not
            // working around a control.
            .user_agent(USER_AGENT)
            // DO NOT REMOVE: reqwest's `auto_sys_proxy` defaults to true,
            // and the underlying hyper-util connector reads
            // HTTP_PROXY/HTTPS_PROXY/ALL_PROXY from the environment
            // unconditionally. If any of those were set, the connection
            // would go to the proxy instead of `pinned_addr`, silently
            // defeating resolve-then-pin -- the DNS override above would
            // simply never be consulted. `.no_proxy()` clears any
            // configured proxy and disables that environment lookup.
            .no_proxy()
            .build()
            .map_err(|error| EgressError::Request(describe_reqwest_error(&error)))?;

        let response = client
            .get(current.clone())
            .send()
            .await
            .map_err(|error| EgressError::Request(describe_reqwest_error(&error)))?;

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
            deny_scheme_downgrade(&current, &next)?;
            // Re-run the full guard (scheme + literal-IP deny check) on the
            // redirect target: a permitted host can redirect to
            // 169.254.169.254, and this is what catches that.
            current = egress_guard(next.as_str())?;
            continue;
        }

        let status = response.status().as_u16();
        let body = read_capped_body(response).await?;
        return Ok(FetchResponse { status, body });
    }
}

/// Refuses a redirect that walks an `https` fetch down onto `http`.
///
/// `egress_guard` deliberately accepts both schemes, because a plugin may name an
/// `http` feed and that is its own choice. What it cannot express is that a fetch
/// which *started* encrypted must stay encrypted: the manifest parser pins a
/// plugin's declared source to `https` (`plugin::manifest::ALLOWED_URL_SCHEME`),
/// and without this check a single `Location: http://...` silently undoes that pin
/// for every remaining hop. One curated feed carries a capability token in its URL
/// path, so a downgraded hop puts a credential on the wire in the clear.
///
/// Upgrades (`http` -> `https`) and same-scheme hops are unaffected.
fn deny_scheme_downgrade(current: &Url, next: &Url) -> Result<(), EgressError> {
    if current.scheme() == "https" && next.scheme() != "https" {
        return Err(EgressError::BadRedirect(format!(
            "refusing to downgrade an https fetch to {}",
            next.scheme()
        )));
    }
    Ok(())
}

/// The server's only permitted outbound host.
///
/// `accounts.google.com` is deliberately absent: consent is a `302` the
/// *browser* follows, not a request the server issues. `www.googleapis.com` is
/// absent because the Calendar API is the *producer's* egress. Widening this
/// constant means the server has started fetching something, which the
/// revision spec §1 forbids -- three tests fail if it does.
pub const IDENTITY_HOST: &str = "oauth2.googleapis.com";

/// POSTs `form` as `application/x-www-form-urlencoded` to `url` under the full
/// egress guard. Used for OAuth token exchange, refresh, and revoke (spec §6:
/// credential-bearing calls keep resolve-then-pin). Single-hop by design: a
/// token endpoint answering a POST with a redirect is not a flow to follow, so a
/// 3xx is returned to the caller as-is (and treated as an error there) rather
/// than re-issued as a POST or silently downgraded to GET.
///
/// The production entry point, and therefore where the allowlist lives.
///
/// The check sits here rather than in [`post_form_inner`] because
/// `post_form_inner` is shared with [`post_form_with_resolver`], which the
/// resolver-injected tests drive against a loopback listener under a fake
/// hostname. That path is private to this module and unreachable from
/// production: [`crate::oauth::transport::EgressTransport`] is the only caller
/// that reaches the network, and it comes through here.
///
/// [`egress_guard`] runs first so that a bad scheme or a literal denied
/// address keeps reporting its own specific error rather than being masked by
/// the host check.
pub async fn fetch_post_form(
    url: &str,
    form: &[(&str, &str)],
) -> Result<FetchResponse, EgressError> {
    let parsed = egress_guard(url)?;
    let host = parsed.host_str().unwrap_or_default();
    if host != IDENTITY_HOST {
        return Err(EgressError::HostNotPermitted {
            host: host.to_owned(),
        });
    }
    post_form_with_resolver(url, &RealResolver, form).await
}

async fn post_form_with_resolver(
    url: &str,
    resolver: &impl HopResolver,
    form: &[(&str, &str)],
) -> Result<FetchResponse, EgressError> {
    match tokio::time::timeout(TOTAL_FETCH_BUDGET, post_form_inner(url, resolver, form)).await {
        Ok(result) => result,
        Err(_elapsed) => Err(EgressError::Timeout),
    }
}

async fn post_form_inner(
    url: &str,
    resolver: &impl HopResolver,
    form: &[(&str, &str)],
) -> Result<FetchResponse, EgressError> {
    let current = egress_guard(url)?;
    let (host, pinned_addr) = resolver.resolve(&current).await?;
    let client = build_pinned_client(&host, pinned_addr)?;
    let response = client
        .post(current.clone())
        .form(form)
        .send()
        .await
        .map_err(|error| EgressError::Request(describe_reqwest_error(&error)))?;
    let status = response.status().as_u16();
    let body = read_capped_body(response).await?;
    Ok(FetchResponse { status, body })
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
        .map_err(|error| EgressError::Request(describe_reqwest_error(&error)))?
    {
        limiter.push(chunk.len())?;
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
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
    async fn the_calendar_api_host_is_not_reachable_from_the_server() {
        // www.googleapis.com is the PRODUCER's egress, never the server's. If
        // this ever stops failing, the server has become a content fetcher
        // again, which the revision spec §1 forbids outright.
        let error = super::fetch_post_form(
            "https://www.googleapis.com/calendar/v3/calendars/primary/events",
            &[],
        )
        .await
        .expect_err("the Calendar API must not be reachable from the server");
        assert!(matches!(error, EgressError::HostNotPermitted { .. }));
    }

    #[tokio::test]
    async fn the_consent_host_is_not_reachable_from_the_server() {
        // accounts.google.com is a 302 the BROWSER follows, not a fetch the
        // server makes, so it has no business on the allowlist.
        let error = super::fetch_post_form("https://accounts.google.com/o/oauth2/v2/auth", &[])
            .await
            .expect_err("the consent host must not be reachable from the server");
        assert!(matches!(error, EgressError::HostNotPermitted { .. }));
    }

    #[tokio::test]
    async fn an_arbitrary_host_is_not_reachable_from_the_server() {
        let error = super::fetch_post_form("https://example.invalid/token", &[])
            .await
            .expect_err("only the identity host is permitted");
        assert!(matches!(error, EgressError::HostNotPermitted { .. }));
    }

    #[tokio::test]
    async fn the_identity_host_passes_the_allowlist() {
        // Asserts only that the ALLOWLIST admits it. Resolution and transport
        // may fail offline; anything other than HostNotPermitted means the host
        // cleared the check, which is what is under test.
        let outcome = super::fetch_post_form("https://oauth2.googleapis.com/token", &[]).await;
        assert!(!matches!(
            outcome,
            Err(EgressError::HostNotPermitted { .. })
        ));
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

    /// The credential-bearing POST path had no test that reached
    /// `build_pinned_client`: both existing POST tests fail inside
    /// `egress_guard` first, and `post_form_with_resolver` -- the seam that
    /// exists so this path can be driven at all -- had no caller.
    ///
    /// This drives it end to end against a real loopback server. Replacing
    /// `build_pinned_client(&host, pinned_addr)?` in `post_form_inner` with a
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
    async fn post_form_resolves_then_pins_and_sends_the_form() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a loopback listener");
        let addr = listener.local_addr().expect("listener has a local addr");
        let router = axum::Router::new().route(
            "/token",
            axum::routing::post(|body: String| async move { body }),
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
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
        // Fix round 2, item 1: the status capture at the end of
        // `post_form_inner` (`let status = response.status().as_u16();`) had
        // no test at THIS level -- `plugin_provider.rs` uses deterministic
        // fake response sequences to prove status-before-parse and last-good
        // behavior. This drives the real guarded path (`fetch_with_resolver`,
        // the same production
        // composition the redirect-chain test above uses) against a server
        // that answers 503, and asserts the returned status is 503, not
        // silently 200.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a loopback listener");
        let addr = listener.local_addr().expect("listener has a local addr");
        let router = axum::Router::new().route(
            "/unavailable",
            axum::routing::post(|| async {
                (
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "service unavailable",
                )
            }),
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
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
        let cap = usize::try_from(MAX_RESPONSE_BODY_BYTES).expect("cap fits in usize");
        let oversized = vec![0u8; cap + 1];
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a loopback listener");
        let addr = listener.local_addr().expect("listener has a local addr");
        let router = axum::Router::new().route(
            "/oversized",
            axum::routing::post(move || async move { oversized }),
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
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
        // Fix round 2, item 2: the case above alone left the streaming
        // `limiter.push` check unproven -- a mutation deleting it, alone,
        // still passed, because the `content_length()` precheck caught
        // that response before streaming ever started. A hostile server
        // does not have to be honest: it can simply never send
        // Content-Length at all (chunked transfer-encoding), which is
        // exactly what `Body::from_stream` produces here, since axum only
        // emits Content-Length when it knows the full size upfront. This
        // is the case that can only be caught by the per-chunk
        // `limiter.push` call as bytes actually arrive.
        let cap = usize::try_from(MAX_RESPONSE_BODY_BYTES).expect("cap fits in usize");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a loopback listener");
        let addr = listener.local_addr().expect("listener has a local addr");
        let router = axum::Router::new().route(
            "/chunked-oversized",
            axum::routing::post(move || async move {
                let chunks: Vec<Result<Vec<u8>, std::io::Error>> =
                    vec![Ok(vec![0u8; cap]), Ok(vec![0u8; 1])];
                axum::body::Body::from_stream(futures_util::stream::iter(chunks))
            }),
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
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
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a loopback listener");
        let addr = listener.local_addr().expect("listener has a local addr");
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
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });

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

    /// A redirect must not walk an encrypted fetch down onto cleartext. Every
    /// plugin's declared source is pinned to `https` at parse time, and this is what
    /// keeps that pin true for the rest of the chain -- `egress_guard` itself
    /// accepts `http`, so it will never catch a downgrade on its own.
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
        // Fix round 2, item 1: a mutation that turned `redirects.consume()?`
        // into `let _ = redirects.consume();` passed every prior test,
        // because the longest redirect chain any test drove was 3 hops --
        // well under MAX_REDIRECTS (5). A handler that redirects to itself
        // forever is what actually exercises the cap: `fetch_inner` must
        // give up after MAX_REDIRECTS hops, not loop indefinitely.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a loopback listener");
        let addr = listener.local_addr().expect("listener has a local addr");
        let router = axum::Router::new().route(
            "/loop",
            axum::routing::get(|| async { axum::response::Redirect::to("/loop") }),
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
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
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a loopback listener");
        let addr = listener.local_addr().expect("listener has a local addr");
        let router = axum::Router::new().route(
            "/start",
            axum::routing::get(|| async {
                axum::response::Redirect::to("http://169.254.169.254/secret")
            }),
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
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
        // Fix round 2, item 2: the case above alone left the streaming
        // `limiter.push` check unproven -- a mutation deleting it, alone,
        // still passed, because the `content_length()` precheck caught
        // that response before streaming ever started. A hostile server
        // does not have to be honest: it can simply never send
        // Content-Length at all (chunked transfer-encoding), which is
        // exactly what `Body::from_stream` produces here, since axum only
        // emits Content-Length when it knows the full size upfront. This
        // is the case that can only be caught by the per-chunk
        // `limiter.push` call as bytes actually arrive.
        let cap = usize::try_from(MAX_RESPONSE_BODY_BYTES).expect("cap fits in usize");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a loopback listener");
        let addr = listener.local_addr().expect("listener has a local addr");
        let router = axum::Router::new().route(
            "/chunked-oversized",
            axum::routing::get(move || async move {
                let chunks: Vec<Result<Vec<u8>, std::io::Error>> =
                    vec![Ok(vec![0u8; cap]), Ok(vec![0u8; 1])];
                axum::body::Body::from_stream(futures_util::stream::iter(chunks))
            }),
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
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
        let cap = usize::try_from(MAX_RESPONSE_BODY_BYTES).expect("cap fits in usize");
        let oversized = vec![0u8; cap + 1];
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a loopback listener");
        let addr = listener.local_addr().expect("listener has a local addr");
        let router = axum::Router::new().route(
            "/oversized",
            axum::routing::get(move || async move { oversized }),
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
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
        // Fix round 2, item 1: the status capture at the end of
        // `fetch_inner` (`let status = response.status().as_u16();`) had
        // no test at THIS level -- `plugin_provider.rs` uses deterministic
        // fake response sequences to prove status-before-parse and last-good
        // behavior. This drives the real guarded path (`fetch_with_resolver`,
        // the same production
        // composition the redirect-chain test above uses) against a server
        // that answers 503, and asserts the returned status is 503, not
        // silently 200.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a loopback listener");
        let addr = listener.local_addr().expect("listener has a local addr");
        let router = axum::Router::new().route(
            "/unavailable",
            axum::routing::get(|| async {
                (
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "service unavailable",
                )
            }),
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
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
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a loopback listener");
        let addr = listener.local_addr().expect("listener has a local addr");
        let router = axum::Router::new()
            .route(
                "/hop1",
                axum::routing::get(move || async move {
                    tokio::time::sleep(hop_delay).await;
                    axum::response::Redirect::to("/hop2")
                }),
            )
            .route(
                "/hop2",
                axum::routing::get(move || async move {
                    tokio::time::sleep(hop_delay).await;
                    axum::response::Redirect::to("/hop3")
                }),
            )
            .route(
                "/hop3",
                axum::routing::get(move || async move {
                    tokio::time::sleep(hop_delay).await;
                    "done"
                }),
            );
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        addr
    }
}
