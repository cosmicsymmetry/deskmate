//! The firmware endpoints: a bearer-authenticated update check and the image
//! download it points to. This is the *only* pair a local-tier device ever
//! touches -- it never sees `/v1/device/link`.
//!
//! Both endpoints sit behind a public Cloudflare tunnel in production, so both
//! are written as if an anonymous internet client is hostile by default: the
//! download streams instead of buffering, and every
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
/// The catalog exposes exactly one live image at a time.
pub struct FirmwareCatalog {
    directory: PathBuf,
    current_version: String,
    /// Keeps an `in_memory()` catalog's private temp directory alive for as
    /// long as the catalog is; `None` for a catalog built with [`Self::new`].
    _temp_dir: Option<tempfile::TempDir>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("firmware version cannot be served as a firmware image path")]
pub struct InvalidFirmwareVersion;

/// The result of comparing a device's reported version against the catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FirmwareCheck {
    UpToDate,
    UpdateAvailable { version: String, url: String },
}

impl FirmwareCatalog {
    pub fn new(
        directory: PathBuf,
        current_version: String,
    ) -> Result<Self, InvalidFirmwareVersion> {
        if !valid_firmware_version(&current_version) {
            return Err(InvalidFirmwareVersion);
        }
        Ok(Self {
            directory,
            current_version,
            _temp_dir: None,
        })
    }

    /// A catalog suitable for tests: the current version is the fixed
    /// `1.0.0` the test fixtures assume, and images are read from a
    /// freshly created, process-unique temp directory that is deleted when
    /// the catalog (and the [`ServerState`] holding it) is dropped.
    ///
    /// This is deliberately *not* the shared `std::env::temp_dir()` --
    /// that's world-writable on a multi-user host, which would let another
    /// local user plant a `1.0.0.bin` for a test to unwittingly serve.
    ///
    /// `directory` is a *subdirectory* of the owned temp dir, not the temp
    /// dir's own root: that's what keeps a traversal test's planted
    /// "outside the catalog" file (`directory().parent()`) contained
    /// inside our own private, uniquely-named, auto-cleaned tree, rather
    /// than landing it in the shared system temp base one level up --
    /// exactly the exposure this doc comment just finished warning about.
    #[must_use]
    pub fn in_memory() -> Self {
        let temp_dir =
            tempfile::tempdir().expect("failed to create a temp dir for an in-memory catalog");
        let directory = temp_dir.path().join("firmware");
        std::fs::create_dir_all(&directory)
            .expect("failed to create the in-memory catalog's firmware subdirectory");
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

    /// Compares `requested` against the newest available version.
    #[must_use]
    fn check(&self, requested: &str) -> FirmwareCheck {
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
    async fn open_image(&self, version: &str) -> std::io::Result<(tokio::fs::File, u64)> {
        let path = self.image_path(version).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid firmware version")
        })?;
        let file = tokio::fs::File::open(path).await?;
        let length = file.metadata().await?.len();
        Ok((file, length))
    }

    fn image_path(&self, version: &str) -> Option<PathBuf> {
        if !valid_firmware_version(version) {
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

fn valid_firmware_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= protocol::MAX_FIRMWARE_VERSION_LEN
        && !version.contains("..")
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
}

#[derive(Debug, Deserialize)]
pub(crate) struct FirmwareQuery {
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
pub(crate) async fn check(
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
pub(crate) async fn download(
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
    fn constructor_accepts_every_supported_firmware_version_boundary() {
        for version in [
            "v2.1.0-proto2".to_string(),
            "release_1-rc.2".to_string(),
            "a".repeat(protocol::MAX_FIRMWARE_VERSION_LEN),
        ] {
            let catalog = FirmwareCatalog::new(PathBuf::from("/firmware"), version.clone())
                .expect("supported firmware version");
            assert!(
                catalog.image_path(&version).is_some(),
                "constructor rejected supported version {version:?}"
            );
        }
    }

    #[test]
    fn constructor_rejects_unservable_firmware_versions() {
        for version in [
            String::new(),
            "a".repeat(protocol::MAX_FIRMWARE_VERSION_LEN + 1),
            "bad/version".to_string(),
            r"bad\version".to_string(),
            "bad..version".to_string(),
            "vé2".to_string(),
            "bad\nversion".to_string(),
        ] {
            assert!(
                matches!(
                    FirmwareCatalog::new(PathBuf::from("/firmware"), version.clone()),
                    Err(InvalidFirmwareVersion)
                ),
                "constructor accepted unservable version {version:?}"
            );
        }
    }

    #[test]
    fn check_reports_up_to_date_for_the_current_version() {
        let catalog = FirmwareCatalog::new(PathBuf::from("/tmp"), "1.0.0".to_string())
            .expect("valid version");
        assert_eq!(catalog.check("1.0.0"), FirmwareCheck::UpToDate);
    }

    #[test]
    fn check_reports_an_update_for_an_older_version() {
        let catalog = FirmwareCatalog::new(PathBuf::from("/tmp"), "1.1.0".to_string())
            .expect("valid version");
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
        let catalog = FirmwareCatalog::new(PathBuf::from("/firmware"), "1.0.0".to_string())
            .expect("valid version");
        assert!(catalog.image_path("../etc/passwd").is_none());
        assert!(catalog.image_path("..").is_none());
        assert!(catalog.image_path("1.0.0").is_some());
    }

    #[test]
    fn image_path_rejects_an_absolute_override() {
        let catalog = FirmwareCatalog::new(PathBuf::from("/firmware"), "1.0.0".to_string())
            .expect("valid version");
        // A naive `directory.join(version)` silently replaces the base when
        // `version` is itself absolute; the allowlist must reject this
        // before it ever reaches `.join()`.
        assert!(catalog.image_path("/etc/passwd").is_none());
    }

    #[test]
    fn loggable_truncates_to_exactly_sixty_four_characters() {
        let attack = "a".repeat(200);
        assert_eq!(loggable(&attack), "a".repeat(64));
    }

    #[test]
    fn loggable_replaces_every_control_character() {
        assert_eq!(
            loggable("release\nnext\rcolumn\tdelete\u{7f}done"),
            "release�next�column�delete�done"
        );
    }
}
