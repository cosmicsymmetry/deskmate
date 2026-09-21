//! SSRF egress guard for the server's one outbound call: a form POST to the
//! OAuth token endpoint.
//!
//! The server issues no GET of its own. It did twice -- plugin data feeds until
//! schema v9, then the weather, RSS and token providers -- and both times the
//! GET path was deleted with its consumer rather than left behind: a
//! `fetch(url)` with no allowlist in front of it leaves the invariant open to
//! the next caller. The faces that need the network now run in a separate
//! subprocess (`companion/faces`), which does its own fetching behind its own,
//! lighter guard (`companion/faces/src/kit/http.ts`). None of that traffic
//! passes through this module, and this module does not vouch for it.
//!
//! What remains is the credential path, bounded twice over: a positive
//! allowlist of exactly one host ([`IDENTITY_HOST`]), and, behind it,
//! address-level exclusions for RFC1918, loopback, link-local,
//! `169.254.169.254` and the other special-purpose ranges below, with
//! resolve-then-pin protection against DNS rebinding.
//!
//! Both layers earn their place. The allowlist states the policy -- credentials
//! are posted to the identity host and nowhere else -- while the address checks
//! stop a DNS answer for that one permitted host from pointing the pinned
//! connection at a homelab neighbour or, on a cloud VM, at the metadata
//! endpoint. This module is the only place allowed to decide "yes, fetch that"
//! and the only place that performs the fetch.
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
//! what this guard can catch. No redirect is followed at all -- a permitted
//! host can redirect to `169.254.169.254`, so a 3xx is handed back to the
//! caller as itself -- but a single connection is not continuously
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
//! globally-routable unicast, deny everything else" (an
//! *allowlist*): permission is what is left over once every named
//! exclusion has been checked, not a list of ranges someone remembered to
//! write down. A deny list can easily omit `100.64.0.0/10` (RFC 6598,
//! carrier-grade NAT), which is Tailscale's entire address range and how this
//! deployment's render host reaches its neighbours.
//!
//! **The two families are not equally exhaustive, and that asymmetry is
//! deliberate rather than an oversight left unstated.** `deny_reason_v4`
//! was independently audited against nightly `Ipv4Addr::is_global()` and
//! found to deny a strict superset of what it denies -- i.e. it is checked
//! exhaustive against the IANA IPv4 Special-Purpose Address Registry.
//! `deny_reason_v6` checks a substantially wider set than its first version
//! (loopback, unspecified, link-local, unique-local, the deprecated
//! site-local range, multicast, the documentation ranges (`2001:db8::/32` and
//! `3fff::/20`), the well-known and local-use NAT64 prefixes, the `2001::/23`
//! IETF protocol assignment block -- Teredo, benchmarking, `ORCHIDv2` -- 6to4,
//! the discard-only range, the `SRv6` SID space, and both legacy IPv4-in-IPv6
//! encodings), but has **not**
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

/// Wall-clock budget for a single DNS resolution.
const RESOLUTION_TIMEOUT: Duration = Duration::from_secs(5);

/// Wall-clock budget for the single HTTP request/response.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Wall-clock budget for the *entire* fetch: the DNS resolution and the
/// request together. Each already has its own timeout
/// ([`RESOLUTION_TIMEOUT`], [`REQUEST_TIMEOUT`]); this is the backstop over
/// their sum, so the caller's wait is bounded by one number.
const TOTAL_FETCH_BUDGET: Duration = Duration::from_secs(20);

/// Maximum response body size accepted from the token endpoint.
pub(crate) const MAX_RESPONSE_BODY_BYTES: u64 = 2 * 1024 * 1024;

/// Why a fetch was refused. Every variant is meant to be safe to log and to
/// surface to an operator; none carry response bodies or secrets.
#[derive(Debug, thiserror::Error)]
pub(crate) enum EgressError {
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
    /// The host is not [`IDENTITY_HOST`], the one host this server posts
    /// credentials to.
    #[error("host {host} is not the permitted identity host")]
    HostNotPermitted { host: String },
}

/// Why an address is not globally-routable unicast, and therefore denied.
/// Every variant names a distinct IANA special-purpose range so a denial is
/// always informative, even though the *decision* to deny is a positive
/// allowlist (see [`deny_reason_v4`]/[`deny_reason_v6`]), not a match against
/// this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DenyReason {
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
    /// `203.0.113.0/24`, RFC 5737) or the IPv6 documentation ranges
    /// (`2001:db8::/32`, RFC 3849; `3fff::/20`, RFC 9637).
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
/// unicast, deny everything else"** (an *allowlist*, and a
/// deny list is a list of what someone remembered and can omit CGNAT);
/// permission is what is left over
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
fn deny_reason_for_ip(ip: IpAddr) -> Option<DenyReason> {
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
/// applicable exclusion. Production uses the richer `DenyReason`; this boolean
/// predicate keeps the registry audit table readable.
#[cfg(test)]
#[must_use]
fn is_globally_routable(ip: IpAddr) -> bool {
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
    // 3fff::/20 -- documentation (RFC 9637).
    if segments[0] == 0x3fff && (segments[1] & 0xf000) == 0 {
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
/// [`fetch_post_form`] runs before anything touches the network. It does
/// **not** perform DNS resolution, so a DNS-name host that resolves to a
/// denied address is only caught by the resolve-then-pin step
/// ([`resolve_and_pin`]), which runs the real resolution exactly once.
fn egress_guard(url: &str) -> Result<Url, EgressError> {
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
/// the resolve-then-pin step: [`fetch_post_form`] never asks anything
/// (resolver or HTTP client) to resolve the same host a second time.
async fn resolve_and_pin(url: &Url) -> Result<(String, SocketAddr), EgressError> {
    let host = url.host_str().ok_or(EgressError::MissingHost)?.to_string();
    let port = url
        .port_or_known_default()
        .ok_or_else(|| EgressError::InvalidUrl("url has no resolvable port".to_string()))?;
    let candidates = resolve_once(&host, port).await?;
    let ip = select_pinned_address(&host, &candidates)?;
    Ok((host, SocketAddr::new(ip, port)))
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
/// production does. This is what makes the pinned client and the body cap
/// testable end-to-end against a real HTTP server:
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

/// The result of a request that ran to completion under the egress guard: the
/// HTTP status alongside its capped body. A redirect status arrives here as
/// itself: the client disables redirect following, and the token endpoint has
/// no reason to issue one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// `reqwest::Error`'s `Display` is usually just the outermost frame (e.g.
/// "error sending request for url (...)"); the actually useful cause lives
/// in its `source()` chain. Walk it so `EgressError::Request` messages are
/// diagnosable rather than generic.
fn describe_reqwest_error(error: reqwest::Error) -> String {
    let error = error.without_url();
    let mut message = error.to_string();
    let mut source = std::error::Error::source(&error);
    while let Some(inner) = source {
        message.push_str(": ");
        message.push_str(&inner.to_string());
        source = inner.source();
    }
    message
}

/// Builds the reqwest client for the one pinned request [`fetch_post_form`]
/// makes: resolve-then-pin, no redirect following, a request timeout and no
/// proxy.
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
        .map_err(|error| EgressError::Request(describe_reqwest_error(error)))
}

/// The server's only permitted outbound host.
///
/// `accounts.google.com` is deliberately absent: consent is a `302` the
/// *browser* follows, not a request the server issues. `www.googleapis.com` is
/// absent because the Calendar API is the *producer's* egress. Widening this
/// constant means the server has started fetching something again, and each of
/// those refusals has a test that says so.
const IDENTITY_HOST: &str = "oauth2.googleapis.com";

/// POSTs `form` as `application/x-www-form-urlencoded` to `url` under the full
/// egress guard. OAuth token exchange, refresh, and revoke keep
/// credential-bearing calls resolve-then-pin. Single-hop by design: a
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
/// of this form-post path, and it comes through here.
///
/// [`egress_guard`] runs first so that a bad scheme or a literal denied
/// address keeps reporting its own specific error rather than being masked by
/// the host check.
pub(crate) async fn fetch_post_form(
    url: &str,
    form: &[(&str, &str)],
) -> Result<FetchResponse, EgressError> {
    guard_identity_url(url)?;
    post_form_with_resolver(url, &RealResolver, form).await
}

fn guard_identity_url(url: &str) -> Result<Url, EgressError> {
    let parsed = egress_guard(url)?;
    let host = parsed.host_str().unwrap_or_default();
    if host != IDENTITY_HOST {
        return Err(EgressError::HostNotPermitted {
            host: host.to_owned(),
        });
    }
    Ok(parsed)
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
        .map_err(|error| EgressError::Request(describe_reqwest_error(error)))?;
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
        .map_err(|error| EgressError::Request(describe_reqwest_error(error)))?
    {
        limiter.push(chunk.len())?;
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests;
