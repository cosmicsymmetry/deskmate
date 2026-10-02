//! Serves the browser companion's built assets.
//!
//! The directory comes from `DESKMATE_WEB_DIR` and is **not** embedded in the
//! binary. That is the point: a UI change then costs a `bun run build` and a file
//! copy, with no Rust compile and no service restart.
//!
//! When the variable is unset the routes are not mounted and the server remains
//! API-only.

use std::path::{Component, Path, PathBuf};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

/// Where the built SPA lives, as configured at startup.
#[derive(Clone)]
pub(crate) struct WebRoot(PathBuf);

impl WebRoot {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self(root)
    }
}

pub(crate) fn routes(root: WebRoot) -> Router {
    Router::new()
        .route("/", get(index))
        .fallback(get(asset))
        .with_state(root)
}

async fn index(State(root): State<WebRoot>) -> Response {
    serve_file(&root.0.join("index.html"), true)
}

/// Serves a built asset, or the SPA shell for anything that is not one.
///
/// A single-page app owns its own routing, so an unknown path is a *client*
/// route, not a 404 -- except under `/v1`, which is the API's namespace and must
/// keep answering 404 for a route that genuinely does not exist rather than
/// handing a JSON client an HTML page.
async fn asset(State(root): State<WebRoot>, uri: Uri) -> Response {
    let path = uri.path();
    if path.starts_with("/v1") {
        return StatusCode::NOT_FOUND.into_response();
    }
    match safe_join(&root.0, path) {
        Some(file) if file.is_file() => {
            let is_shell = file == root.0.join("index.html");
            serve_file(&file, is_shell)
        }
        _ => serve_file(&root.0.join("index.html"), true),
    }
}

/// Resolves a request path inside the web root, refusing anything that would
/// escape it.
///
/// The check is structural rather than a string scan: decoded parent components
/// are rejected when the path is rebuilt, while `http::Uri::path` preserves
/// percent encodings, so encoded separators remain literal path-component bytes.
fn safe_join(root: &Path, request_path: &str) -> Option<PathBuf> {
    let mut resolved = root.to_path_buf();
    for component in Path::new(request_path.trim_start_matches('/')).components() {
        match component {
            Component::Normal(part) => resolved.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(resolved)
}

fn serve_file(path: &Path, is_shell: bool) -> Response {
    let Ok(bytes) = std::fs::read(path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut response = Response::new(Body::from(bytes));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, content_type(path));
    // Vite fingerprints every built asset, so an asset URL names exactly one
    // body forever and may be cached hard. The shell must not be: it is the one
    // file whose URL stays the same while its content changes on every deploy,
    // and a cached shell would keep pointing at the previous build's assets.
    let cache = if is_shell {
        "no-cache"
    } else {
        "public, max-age=31536000, immutable"
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    if is_shell {
        // /signin carries a one-time credential. Do not forward it in Referer
        // when loading assets or following links, including same-origin URLs.
        response.headers_mut().insert(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        );
    }
    response
}

/// Minimal extension map. Only the types a Vite build actually emits; anything
/// else is served as bytes rather than guessed at.
fn content_type(path: &Path) -> HeaderValue {
    let value = match path.extension().and_then(|ext| ext.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    };
    HeaderValue::from_static(value)
}

#[cfg(test)]
mod tests {
    use super::safe_join;
    use std::path::Path;

    #[test]
    fn a_normal_asset_path_resolves_inside_the_root() {
        let joined = safe_join(Path::new("/srv/web"), "/assets/index-abc123.js");
        assert_eq!(
            joined.as_deref(),
            Some(Path::new("/srv/web/assets/index-abc123.js"))
        );
    }

    #[test]
    fn a_traversal_is_refused_rather_than_normalized() {
        // Refused, not clamped: returning the root for an escaping path would
        // make `/../../etc/passwd` serve the SPA shell and look like it worked.
        assert!(safe_join(Path::new("/srv/web"), "/../../etc/passwd").is_none());
        assert!(safe_join(Path::new("/srv/web"), "/assets/../../etc/passwd").is_none());
    }

    #[test]
    fn an_absolute_looking_segment_cannot_reset_the_root() {
        // Repeated literal separators are normalized by Path components; they
        // cannot turn a relative join into a filesystem-rooted path.
        assert!(safe_join(Path::new("/srv/web"), "//etc/passwd").is_some());
        assert_eq!(
            safe_join(Path::new("/srv/web"), "//etc/passwd").as_deref(),
            Some(Path::new("/srv/web/etc/passwd"))
        );
    }
}
