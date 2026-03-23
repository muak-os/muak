//! Imager library for OCI image pulling and manifest signing.

mod image;
mod oci;

pub mod error;

use std::path::Path;

pub use error::{ImagerError, Result};
use oci::remote::pull_to_dir;

/// Pull an OCI image and extract it to a directory.
pub async fn pull(reference: &str, output: &Path, pubkey_pem: Option<&str>) -> Result<()> {
    tokio::fs::create_dir_all(output).await?;
    pull_to_dir(reference, output, pubkey_pem).await
}

/// Sign an OCI image manifest in the registry.
pub async fn sign(reference: &str, privkey_pem: &str) -> Result<()> {
    oci::sign::sign_manifest(reference, privkey_pem).await
}
