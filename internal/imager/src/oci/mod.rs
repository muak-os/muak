use crate::error::{ImagerError, Result};

const OCI_MANIFEST_ACCEPT_HEADERS: &[&str] = &[
    "application/vnd.oci.image.manifest.v1+json",
    "application/vnd.docker.distribution.manifest.v2+json",
    "application/vnd.oci.image.index.v1+json",
    "application/vnd.docker.distribution.manifest.list.v2+json",
];

const USER_AGENT: &str = "muak-imager/0.1";

mod auth;
mod http;
mod layer;
mod manifest;
pub(crate) mod sign;
pub(crate) mod verify;

pub mod local;
pub mod remote;

/// Create a temporary directory.
pub(crate) fn create_temp_dir(prefix: &str) -> Result<tempfile::TempDir> {
    let locations = ["/run", "/tmp"];
    for &dir in &locations {
        if let Ok(temp) = tempfile::Builder::new().prefix(prefix).tempdir_in(dir) {
            return Ok(temp);
        }
    }
    Err(ImagerError::TempDirError(format!(
        "Failed to create temp dir in any of: {:?}",
        locations
    )))
}
