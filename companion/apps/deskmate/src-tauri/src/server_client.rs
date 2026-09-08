//! One place where a Deskmate server GET is built, read and bounded.
//!
//! Every admin call is bearer-authenticated and reads at most a stated number of
//! bytes before parsing, so an unreachable or hostile server can waste bytes but
//! never memory. The typed failure mapping is `commands::server_failure`, shared
//! with the config PUT: a 401 says the token was rejected in exactly one voice.

use serde::de::DeserializeOwned;

use crate::commands::{IpcError, server_failure, validate_server_url};

/// A 448x368 RGB565 frame encoded as PNG is far under this; the bound exists so a
/// server that answers a preview with a firehose is refused, not swallowed.
pub(crate) const MAX_SERVER_PREVIEW_BYTES: usize = 1_024 * 1_024;

/// Appends admin path segments to the operator's base URL. Query and fragment are
/// dropped for the same reason `device_link_url` drops them: an admin base is a
/// prefix, not a request. Segments are percent-encoded by `url` (`/` becomes
/// `%2F`), so a device or card id containing a slash cannot escape into another
/// route. `url` silently drops a `.` or `..` segment, which can only ever shorten
/// the path into a route that does not exist -- a 404, never another device.
pub(crate) fn server_url(base: &str, path_segments: &[&str]) -> Result<url::Url, IpcError> {
    let mut url = validate_server_url(base)?;
    url.set_query(None);
    url.set_fragment(None);
    let mut segments = url
        .path_segments_mut()
        .map_err(|()| IpcError::InvalidPayload {
            message: "server URL cannot be used as a base URL".into(),
        })?;
    segments.pop_if_empty().extend(path_segments);
    drop(segments);
    Ok(url)
}

/// Reads at most `max_bytes` of an admin GET body, then maps the status through the
/// same `server_failure` the config PUT uses. The body is read before the status is
/// judged, because a 422 carries its validation details in it.
///
/// The bound is enforced by `ureq`'s own limited reader rather than by measuring the
/// body afterwards: a limit of `max_bytes + 1` accepts a body of exactly `max_bytes`
/// and fails on the first byte past it, so an oversized response is never fully
/// allocated and never parsed. `BodyExceedsLimit` is the one read failure with its
/// own sentence, because it is the only one that says something about the server
/// rather than about the socket.
pub(crate) fn get_server_json<T: DeserializeOwned>(
    agent: &ureq::Agent,
    url: &str,
    admin_token: &str,
    max_bytes: usize,
) -> Result<T, IpcError> {
    let mut response = agent
        .get(url)
        .header("Authorization", format!("Bearer {admin_token}"))
        .call()
        .map_err(|_| IpcError::RuntimeUnavailable {
            message: "the configured server could not be reached".into(),
        })?;
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .with_config()
        .limit(max_bytes.saturating_add(1) as u64)
        .read_to_vec()
        .map_err(|error| match error {
            ureq::Error::BodyExceedsLimit(_) => IpcError::RuntimeUnavailable {
                message: "the server returned an oversized response".into(),
            },
            _ => IpcError::RuntimeUnavailable {
                message: "the server returned an unreadable response".into(),
            },
        })?;
    if !(200..300).contains(&status) {
        return Err(server_failure(status, &body));
    }
    serde_json::from_slice(&body).map_err(|_| IpcError::RuntimeUnavailable {
        message: "the server returned a response this app could not read".into(),
    })
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use crate::commands::IpcError;
    use crate::commands::tests::read_http_request;

    use super::{get_server_json, server_url};

    /// Serves one request, replies with `status` and `body`, and hands back the
    /// bytes the client actually sent so a test can assert the method, path and
    /// Authorization header rather than trusting the client's own report.
    fn serve_once(
        status: &'static str,
        body: Vec<u8>,
    ) -> (String, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = String::from_utf8_lossy(&read_http_request(&mut stream)).to_string();
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
            request
        });
        (format!("http://{address}"), handle)
    }

    #[test]
    fn server_url_appends_admin_segments_and_drops_query_and_fragment() {
        assert_eq!(
            server_url("https://desk.example/base/?a=1#f", &["v1", "plugins"])
                .unwrap()
                .as_str(),
            "https://desk.example/base/v1/plugins"
        );
        // A segment that tries to escape into another route is percent-encoded,
        // not obeyed: `url` encodes `/` as `%2F` inside a pushed segment.
        assert_eq!(
            server_url(
                "https://desk.example",
                &["v1", "devices", "a b/c", "config"]
            )
            .unwrap()
            .as_str(),
            "https://desk.example/v1/devices/a%20b%2Fc/config"
        );
        assert!(server_url("not-a-url", &["v1"]).is_err());
    }

    #[test]
    fn a_bearer_authenticated_get_returns_the_parsed_body() {
        let (base, server) = serve_once("200 OK", br#"{"value":"ok"}"#.to_vec());
        let url = server_url(&base, &["v1", "plugins"]).unwrap().to_string();

        let parsed: serde_json::Value = get_server_json(
            &crate::server_http_agent(),
            &url,
            "admin-secret",
            crate::commands::MAX_SERVER_ERROR_BYTES,
        )
        .unwrap();

        let request = server.join().unwrap();
        assert!(request.starts_with("GET /v1/plugins "));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bearer admin-secret")
        );
        assert_eq!(parsed["value"], "ok");
    }

    #[test]
    fn a_rejected_admin_token_keeps_the_one_shared_token_message() {
        let (base, server) = serve_once("401 Unauthorized", Vec::new());
        let url = server_url(&base, &["v1", "plugins"]).unwrap().to_string();

        let error = get_server_json::<serde_json::Value>(
            &crate::server_http_agent(),
            &url,
            "wrong",
            crate::commands::MAX_SERVER_ERROR_BYTES,
        )
        .unwrap_err();

        server.join().unwrap();
        assert!(matches!(
            error,
            IpcError::InvalidPayload { message } if message == "the server rejected the admin token"
        ));
    }

    /// The bound is enforced while reading, so an oversized body is never fully
    /// allocated and never reaches `serde_json`. 522 bytes against a 512-byte
    /// bound: one byte past the limit is enough to refuse the whole response.
    #[test]
    fn an_oversized_body_is_refused_before_it_is_parsed() {
        let bound = 512;
        let body = format!(r#"{{"pad":"{}"}}"#, "x".repeat(bound)).into_bytes();
        assert!(body.len() > bound);
        let (base, server) = serve_once("200 OK", body);
        let url = server_url(&base, &["v1", "plugins"]).unwrap().to_string();

        let error =
            get_server_json::<serde_json::Value>(&crate::server_http_agent(), &url, "t", bound)
                .unwrap_err();

        server.join().unwrap();
        assert!(matches!(
            error,
            IpcError::RuntimeUnavailable { message } if message.contains("oversized")
        ));
    }
}
