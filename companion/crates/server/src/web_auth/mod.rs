pub(crate) mod google;
mod rate_limit;
pub(crate) mod routes;
pub(crate) mod setup_code;

use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::header::{COOKIE, ORIGIN};
use axum::http::request::Parts;
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use chrono::Utc;

use crate::ServerState;
use crate::identity::Account;

pub(crate) use rate_limit::RateLimiter;

pub(crate) const SESSION_COOKIE: &str = "__Host-deskmate_session";
const SESSION_MAX_AGE_SECONDS: i64 = crate::identity::SESSION_TTL_DAYS * 24 * 60 * 60;

#[must_use]
pub(crate) fn set_session_cookie(plaintext_sid: &str) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{SESSION_COOKIE}={plaintext_sid}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age={SESSION_MAX_AGE_SECONDS}"
    ))
    .expect("a random session id is a valid cookie value")
}

#[must_use]
pub(crate) fn clear_session_cookie() -> HeaderValue {
    HeaderValue::from_static(
        "__Host-deskmate_session=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0",
    )
}

fn session_cookie_value(parts: &Parts) -> Option<String> {
    for header in parts.headers.get_all(COOKIE) {
        let Ok(header) = header.to_str() else {
            continue;
        };
        if let Some(value) = header.split(';').find_map(|pair| {
            let (name, value) = pair.trim().split_once('=')?;
            (name == SESSION_COOKIE).then(|| value.to_owned())
        }) {
            return Some(value);
        }
    }
    None
}

pub(crate) struct AccountSession {
    pub(crate) account: Account,
    pub(crate) session_plaintext: String,
}

impl FromRequestParts<ServerState> for AccountSession {
    type Rejection = StatusCode;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &ServerState,
    ) -> Result<Self, Self::Rejection> {
        let Some(session_plaintext) = session_cookie_value(parts) else {
            return Err(StatusCode::UNAUTHORIZED);
        };
        SameOrigin::from_request_parts(parts, state).await?;

        let lookup = state.clone();
        let candidate = session_plaintext.clone();
        let account = tokio::task::spawn_blocking(move || {
            lookup.identity().session_account(&candidate, Utc::now())
        })
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;

        Ok(Self {
            account,
            session_plaintext,
        })
    }
}

pub(crate) struct InstanceOwner(pub(crate) AccountSession);

impl FromRequestParts<ServerState> for InstanceOwner {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &ServerState,
    ) -> Result<Self, Self::Rejection> {
        let management_path = parts.uri.path().starts_with("/v1/manage");
        let session = AccountSession::from_request_parts(parts, state)
            .await
            .map_err(|status| {
                if management_path && status == StatusCode::UNAUTHORIZED {
                    Redirect::to("/").into_response()
                } else {
                    status.into_response()
                }
            })?;
        if !session.account.is_instance_owner {
            return Err(StatusCode::NOT_FOUND.into_response());
        }
        Ok(Self(session))
    }
}

pub(crate) struct SameOrigin;

impl FromRequestParts<ServerState> for SameOrigin {
    type Rejection = StatusCode;

    #[allow(clippy::unused_async_trait_impl)]
    async fn from_request_parts(
        parts: &mut Parts,
        state: &ServerState,
    ) -> Result<Self, Self::Rejection> {
        if matches!(parts.method, Method::GET | Method::HEAD | Method::OPTIONS) {
            return Ok(Self);
        }
        let expected = state.public_url().origin().ascii_serialization();
        let presented = parts
            .headers
            .get(ORIGIN)
            .and_then(|value| value.to_str().ok());
        if presented == Some(expected.as_str()) {
            Ok(Self)
        } else {
            Err(StatusCode::FORBIDDEN)
        }
    }
}

pub(crate) struct ClientIp(pub(crate) IpAddr);

impl FromRequestParts<ServerState> for ClientIp {
    type Rejection = std::convert::Infallible;

    #[allow(clippy::unused_async_trait_impl)]
    async fn from_request_parts(
        parts: &mut Parts,
        state: &ServerState,
    ) -> Result<Self, Self::Rejection> {
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|connect| connect.0);
        let forwarded = parts
            .headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok());
        Ok(Self(client_ip(peer, forwarded, state.trusted_proxies())))
    }
}

/// The peers whose `X-Forwarded-For` names the client: loopback always, plus the
/// networks in `DESKMATE_TRUSTED_PROXIES`. Every per-IP auth limit keys on the
/// address this yields, so a proxy that is not trusted here turns them all into
/// one instance-wide bucket -- and trusting a peer that does not overwrite the
/// header lets a visitor choose their own bucket. Name only the proxy itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustedProxies(Vec<(IpAddr, u8)>);

impl TrustedProxies {
    pub const NONE: Self = Self(Vec::new());

    /// Comma-separated addresses or CIDR networks, e.g. `172.26.0.0/16`.
    /// A zero-length prefix is refused: it would trust every peer.
    pub fn parse(value: &str) -> Result<Self, String> {
        value
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(|entry| {
                let (address, prefix) = match entry.split_once('/') {
                    Some((address, prefix)) => (address, Some(prefix)),
                    None => (entry, None),
                };
                let address: IpAddr = address
                    .parse()
                    .map_err(|_| format!("{entry:?} is not an IP address or CIDR network"))?;
                let width = if address.is_ipv4() { 32 } else { 128 };
                let prefix = match prefix {
                    None => width,
                    Some(prefix) => prefix
                        .parse::<u8>()
                        .ok()
                        .filter(|prefix| (1..=width).contains(prefix))
                        .ok_or_else(|| {
                            format!("{entry:?} needs a prefix length between 1 and {width}")
                        })?,
                };
                Ok((address, prefix))
            })
            .collect::<Result<_, _>>()
            .map(Self)
    }

    fn trusts(&self, peer: IpAddr) -> bool {
        let peer = peer.to_canonical();
        peer.is_loopback()
            || self
                .0
                .iter()
                .any(|&(network, prefix)| match (peer, network) {
                    (IpAddr::V4(peer), IpAddr::V4(network)) => {
                        let mask = u32::MAX << (32 - u32::from(prefix));
                        u32::from(peer) & mask == u32::from(network) & mask
                    }
                    (IpAddr::V6(peer), IpAddr::V6(network)) => {
                        let mask = u128::MAX << (128 - u32::from(prefix));
                        u128::from(peer) & mask == u128::from(network) & mask
                    }
                    _ => false,
                })
    }
}

fn client_ip(
    peer: Option<SocketAddr>,
    forwarded: Option<&str>,
    trusted: &TrustedProxies,
) -> IpAddr {
    match peer {
        Some(peer) if trusted.trusts(peer.ip()) => forwarded
            .and_then(|value| value.rsplit(',').next())
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or_else(|| peer.ip()),
        Some(peer) => peer.ip(),
        None => IpAddr::from([0, 0, 0, 0]),
    }
}

pub(crate) fn session_digest_key(plaintext_sid: &str) -> String {
    use std::fmt::Write as _;

    crate::credential::token_digest(plaintext_sid).iter().fold(
        String::with_capacity(64),
        |mut output, byte| {
            write!(output, "{byte:02x}").expect("writing to a string cannot fail");
            output
        },
    )
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use axum::body::Body;
    use axum::http::Request;

    use super::*;

    #[test]
    fn cookie_parser_takes_the_first_session_value_across_cookie_headers() {
        let (mut parts, _) = Request::builder()
            .header(COOKIE, "other=1; __Host-deskmate_session=first")
            .body(Body::empty())
            .unwrap()
            .into_parts();
        parts.headers.append(
            COOKIE,
            HeaderValue::from_static("__Host-deskmate_session=second"),
        );

        assert_eq!(session_cookie_value(&parts).as_deref(), Some("first"));
    }

    #[test]
    fn forwarded_for_is_trusted_only_from_loopback() {
        let none = &TrustedProxies::NONE;
        let from_proxy = client_ip(
            Some("127.0.0.1:5000".parse().unwrap()),
            Some("198.51.100.7, 203.0.113.9"),
            none,
        );
        assert_eq!(from_proxy, "203.0.113.9".parse::<IpAddr>().unwrap());

        let direct = client_ip(
            Some("192.0.2.1:5000".parse().unwrap()),
            Some("203.0.113.9"),
            none,
        );
        assert_eq!(direct, "192.0.2.1".parse::<IpAddr>().unwrap());
        assert_eq!(client_ip(None, None, none), IpAddr::from([0, 0, 0, 0]));
    }

    #[test]
    fn forwarded_for_is_trusted_from_a_configured_proxy_network_only() {
        // The hosted shape: Caddy in a Docker bridge reaching a LAN-bound listener.
        let trusted = TrustedProxies::parse("172.26.0.0/16, 2001:db8::1").unwrap();
        let ip = |text: &str| text.parse::<IpAddr>().unwrap();

        let via_caddy = client_ip(
            Some("172.26.0.2:41000".parse().unwrap()),
            Some("203.0.113.9"),
            &trusted,
        );
        assert_eq!(via_caddy, ip("203.0.113.9"));
        let mapped = client_ip(
            Some("[::ffff:172.26.0.2]:41000".parse().unwrap()),
            Some("203.0.113.9"),
            &trusted,
        );
        assert_eq!(
            mapped,
            ip("203.0.113.9"),
            "an IPv4-mapped peer is the same peer"
        );
        let v6 = client_ip(
            Some("[2001:db8::1]:41000".parse().unwrap()),
            Some("203.0.113.9"),
            &trusted,
        );
        assert_eq!(v6, ip("203.0.113.9"));

        // A LAN host outside the network cannot choose its bucket.
        let lan = client_ip(
            Some("192.168.8.30:41000".parse().unwrap()),
            Some("203.0.113.9"),
            &trusted,
        );
        assert_eq!(lan, ip("192.168.8.30"));
        let neighbour = client_ip(
            Some("172.27.0.2:41000".parse().unwrap()),
            Some("203.0.113.9"),
            &trusted,
        );
        assert_eq!(neighbour, ip("172.27.0.2"));
        // A trusted proxy that sent nothing usable is itself the bucket.
        let garbage = client_ip(
            Some("172.26.0.2:41000".parse().unwrap()),
            Some("not-an-ip"),
            &trusted,
        );
        assert_eq!(garbage, ip("172.26.0.2"));
    }

    #[test]
    fn trusted_proxies_parse_addresses_and_networks_and_refuse_nonsense() {
        assert_eq!(TrustedProxies::parse("").unwrap(), TrustedProxies::NONE);
        assert_eq!(TrustedProxies::parse(" , ").unwrap(), TrustedProxies::NONE);
        assert_eq!(
            TrustedProxies::parse("10.0.0.1, ::1/128").unwrap(),
            TrustedProxies(vec![
                ("10.0.0.1".parse().unwrap(), 32),
                ("::1".parse().unwrap(), 128)
            ])
        );
        for bad in [
            "0.0.0.0/0",
            "::/0",
            "10.0.0.0/33",
            "10.0.0.0/x",
            "caddy",
            "10.0.0/8",
        ] {
            assert!(TrustedProxies::parse(bad).is_err(), "{bad} must be refused");
        }
    }

    #[test]
    fn cookie_headers_have_the_required_security_attributes() {
        assert_eq!(
            set_session_cookie("session-value").to_str().unwrap(),
            "__Host-deskmate_session=session-value; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=2592000"
        );
        assert_eq!(
            clear_session_cookie().to_str().unwrap(),
            "__Host-deskmate_session=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0"
        );
    }

    #[test]
    fn digest_key_is_hex_and_does_not_contain_the_session() {
        let key = session_digest_key("plaintext-session");
        assert_eq!(key.len(), 64);
        assert!(key.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(!key.contains("plaintext-session"));
    }
}
