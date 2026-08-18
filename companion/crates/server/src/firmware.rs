//! The firmware endpoints: a bearer-authenticated update check and the image
//! download it points to. This is the *only* pair a local-tier device ever
//! touches -- it never sees `/v1/device/link`.
//!
//! Both endpoints sit behind a public Cloudflare tunnel from Task 8 onward,
//! so both are written as if an anonymous internet client is hostile by
//! default: the download streams instead of buffering, and every
//! attacker-controlled string (the requested version) is bounded before it
//! touches disk, a filesystem call, or a log line.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::extract::{Path as PathParam, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tokio_util::io::ReaderStream;

use crate::ServerState;
use crate::auth::AuthenticatedDevice;

/// What's on disk and what version it represents. Config, not a database:
/// V2 has exactly one image live at a time.
pub struct FirmwareCatalog {
    directory: PathBuf,
    current_version: String,
    /// Keeps an `in_memory()` catalog's private temp directory alive for as
    /// long as the catalog is; `None` for a catalog built with [`Self::new`].
    _temp_dir: Option<tempfile::TempDir>,
}

/// The result of comparing a device's reported version against the catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FirmwareCheck {
    UpToDate,
    UpdateAvailable { version: String, url: String },
}

impl FirmwareCatalog {
    #[must_use]
    pub fn new(directory: PathBuf, current_version: String) -> Self {
        Self {
            directory,
            current_version,
            _temp_dir: None,
        }
    }

    /// A catalog suitable for tests: the current version is the fixed
    /// `1.0.0` the test fixtures assume, and images are read from a
    /// freshly created, process-unique temp directory that is deleted when
    /// the catalog (and the [`ServerState`] holding it) is dropped.
    ///
    /// This is deliberately *not* the shared `std::env::temp_dir()` --
    /// that's world-writable on a multi-user host, which would let another
    /// local user plant a `1.0.0.bin` for a test to unwittingly serve.
    #[must_use]
    pub fn in_memory() -> Self {
        let temp_dir =
            tempfile::tempdir().expect("failed to create a temp dir for an in-memory catalog");
        let directory = temp_dir.path().to_path_buf();
        Self {
            directory,
            current_version: "1.0.0".to_string(),
            _temp_dir: Some(temp_dir),
        }
    }

    /// The directory images are served from. Exposed for tests that need to
    /// place a real image before exercising the download route end to end.
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    #[must_use]
    pub fn current_version(&self) -> &str {
        &self.current_version
    }

    /// Compares `requested` against the newest available version.
    #[must_use]
    pub fn check(&self, requested: &str) -> FirmwareCheck {
        if requested == self.current_version {
            return FirmwareCheck::UpToDate;
        }
        let version = self.current_version.clone();
        FirmwareCheck::UpdateAvailable {
            url: format!("/v1/firmware/{version}.bin"),
            version,
        }
    }

    /// Opens the image for `version` and reports its size, without reading
    /// it into memory -- the caller streams it.
    ///
    /// `version` comes from an untrusted URL path segment, so it is checked
    /// for path-traversal and other unsafe characters before being joined
    /// onto the directory; a rejected version reads as "not found" like any
    /// other bad request.
    pub async fn open_image(&self, version: &str) -> std::io::Result<(tokio::fs::File, u64)> {
        let path = self.image_path(version).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid firmware version")
        })?;
        let file = tokio::fs::File::open(path).await?;
        let length = file.metadata().await?.len();
        Ok((file, length))
    }

    fn image_path(&self, version: &str) -> Option<PathBuf> {
        if version.is_empty() || version.len() > 32 {
            return None;
        }
        let is_safe = version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_');
        if !is_safe || version.contains("..") {
            return None;
        }
        // Every character above is ASCII-alphanumeric, '.', '-' or '_', so
        // `file_name` can never contain a path separator, a drive prefix, or
        // resolve outside `directory` -- this is defense in depth, not the
        // primary guard.
        let file_name = format!("{version}.bin");
        if Path::new(&file_name).components().count() != 1 {
            return None;
        }
        Some(self.directory.join(file_name))
    }
}

#[derive(Debug, Deserialize)]
pub struct FirmwareQuery {
    current: String,
}

#[derive(Debug, Serialize)]
struct FirmwareUpdateResponse {
    version: String,
    url: String,
}

/// Bounds and sanitizes an attacker-controlled string before it reaches a
/// log line: truncated to a sane length (well past any real firmware
/// version) and with control characters (including newlines, which could
/// otherwise forge extra log lines) replaced.
fn loggable(value: &str) -> String {
    const MAX_LOGGED_LEN: usize = 64;
    let truncated: String = value.chars().take(MAX_LOGGED_LEN).collect();
    truncated
        .chars()
        .map(|c| if c.is_control() { '\u{fffd}' } else { c })
        .collect()
}

/// `GET /v1/device/firmware?current=<version>` -- `204` when the device is
/// already current, otherwise `200` with the newer version and its download
/// url.
pub async fn check(
    State(state): State<ServerState>,
    AuthenticatedDevice { device_id }: AuthenticatedDevice,
    Query(query): Query<FirmwareQuery>,
) -> impl IntoResponse {
    tracing::debug!(
        device_id = %device_id,
        current = %loggable(&query.current),
        "firmware check"
    );
    match state.firmware().check(&query.current) {
        FirmwareCheck::UpToDate => StatusCode::NO_CONTENT.into_response(),
        FirmwareCheck::UpdateAvailable { version, url } => {
            axum::Json(FirmwareUpdateResponse { version, url }).into_response()
        }
    }
}

/// `GET /v1/firmware/{version}.bin` -- the image itself, for `esp_https_ota`.
/// Deliberately unauthenticated: images are byte-identical across every
/// device and hold no per-device secret, integrity comes from the tunnel's
/// TLS, and version *discovery* (which already requires a bearer token via
/// [`check`]) is what would actually need protecting.
///
/// Axum's router matches whole path segments, so the route captures the
/// segment as `filename` and this handler splits the `.bin` suffix back off
/// to recover the version; a segment that isn't `<version>.bin` is a 404
/// like any other unknown resource.
///
/// The image is streamed from disk in bounded chunks rather than read into
/// a `Vec<u8>`, so an anonymous caller can never make this handler hold a
/// whole multi-megabyte image in memory -- load-bearing once this route
/// sits behind a public tunnel with arbitrarily many concurrent callers.
pub async fn download(
    State(state): State<ServerState>,
    PathParam(filename): PathParam<String>,
) -> Response {
    let Some(version) = filename.strip_suffix(".bin") else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok((file, length)) = state.firmware().open_image(version).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let body = Body::from_stream(ReaderStream::new(file));
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, length)
        .body(body)
        .expect("a response built from static headers and a streaming body is always valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_reports_up_to_date_for_the_current_version() {
        let catalog = FirmwareCatalog::new(PathBuf::from("/tmp"), "1.0.0".to_string());
        assert_eq!(catalog.check("1.0.0"), FirmwareCheck::UpToDate);
    }

    #[test]
    fn check_reports_an_update_for_an_older_version() {
        let catalog = FirmwareCatalog::new(PathBuf::from("/tmp"), "1.1.0".to_string());
        assert_eq!(
            catalog.check("1.0.0"),
            FirmwareCheck::UpdateAvailable {
                version: "1.1.0".to_string(),
                url: "/v1/firmware/1.1.0.bin".to_string(),
            }
        );
    }

    #[test]
    fn image_path_rejects_traversal() {
        let catalog = FirmwareCatalog::new(PathBuf::from("/firmware"), "1.0.0".to_string());
        assert!(catalog.image_path("../etc/passwd").is_none());
        assert!(catalog.image_path("..").is_none());
        assert!(catalog.image_path("1.0.0").is_some());
    }

    #[test]
    fn image_path_rejects_an_absolute_override() {
        let catalog = FirmwareCatalog::new(PathBuf::from("/firmware"), "1.0.0".to_string());
        // A naive `directory.join(version)` silently replaces the base when
        // `version` is itself absolute; the allowlist must reject this
        // before it ever reaches `.join()`.
        assert!(catalog.image_path("/etc/passwd").is_none());
    }

    #[test]
    fn loggable_truncates_and_strips_control_characters() {
        let attack = format!("{}\nSMUGGLED LOG LINE", "a".repeat(200));
        let logged = loggable(&attack);
        assert_eq!(logged.chars().count(), 64);
        assert!(!logged.contains('\n'));
    }
}
