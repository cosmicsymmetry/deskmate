//! The firmware endpoints: a bearer-authenticated update check and the image
//! download it points to. This is the *only* pair a local-tier device ever
//! touches -- it never sees `/v1/device/link`.

use std::path::{Path, PathBuf};

use axum::extract::{Path as PathParam, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::ServerState;
use crate::auth::AuthenticatedDevice;

/// What's on disk and what version it represents. Config, not a database:
/// V2 has exactly one image live at a time.
#[derive(Debug, Clone)]
pub struct FirmwareCatalog {
    directory: PathBuf,
    current_version: String,
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
        }
    }

    /// A catalog suitable for tests: the current version is the fixed
    /// `1.0.0` the test fixtures assume, and images are read from the
    /// process's temp directory (which these tests never populate, since
    /// none of them exercise the download route).
    #[must_use]
    pub fn in_memory() -> Self {
        Self::new(std::env::temp_dir(), "1.0.0".to_string())
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

    /// Reads the image for `version` from the configured directory.
    ///
    /// `version` comes from an untrusted URL path segment, so it is checked
    /// for path-traversal components before being joined onto the directory;
    /// a rejected version reads as "not found" like any other bad request.
    pub async fn read_image(&self, version: &str) -> std::io::Result<Vec<u8>> {
        let path = self.image_path(version).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid firmware version")
        })?;
        tokio::fs::read(path).await
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
        let file_name = format!("{version}.bin");
        let path = self.directory.join(&file_name);
        // The joined path must still resolve to a direct child of the
        // configured directory -- belt and braces alongside the character
        // allowlist above.
        if Path::new(&file_name).components().count() != 1 {
            return None;
        }
        Some(path)
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

/// `GET /v1/device/firmware?current=<version>` -- `204` when the device is
/// already current, otherwise `200` with the newer version and its download
/// url.
pub async fn check(
    State(state): State<ServerState>,
    AuthenticatedDevice { device_id }: AuthenticatedDevice,
    Query(query): Query<FirmwareQuery>,
) -> impl IntoResponse {
    tracing::debug!(device_id = %device_id, current = %query.current, "firmware check");
    match state.firmware().check(&query.current) {
        FirmwareCheck::UpToDate => StatusCode::NO_CONTENT.into_response(),
        FirmwareCheck::UpdateAvailable { version, url } => {
            axum::Json(FirmwareUpdateResponse { version, url }).into_response()
        }
    }
}

/// `GET /v1/firmware/{version}.bin` -- the image itself, for `esp_https_ota`.
///
/// Axum's router matches whole path segments, so the route captures the
/// segment as `filename` and this handler splits the `.bin` suffix back off
/// to recover the version; a segment that isn't `<version>.bin` is a 404
/// like any other unknown resource.
pub async fn download(
    State(state): State<ServerState>,
    PathParam(filename): PathParam<String>,
) -> Response {
    let Some(version) = filename.strip_suffix(".bin") else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match state.firmware().read_image(version).await {
        Ok(bytes) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/octet-stream")],
            bytes,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
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
}
