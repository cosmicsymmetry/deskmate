//! Shared construction for Deskmate server URLs.

use crate::commands::{IpcError, validate_server_url};

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

#[cfg(test)]
mod tests {
    use super::server_url;

    #[test]
    fn server_url_appends_admin_segments_and_drops_query_and_fragment() {
        assert_eq!(
            server_url("https://desk.example/base/?a=1#f", &["v1", "images"])
                .unwrap()
                .as_str(),
            "https://desk.example/base/v1/images"
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
}
