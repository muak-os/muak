//! Imager library for OCI image pulling and manifest signing.

mod image;
mod oci;

pub mod error;

use std::path::Path;

pub use error::{KociError, Result};
use oci::remote::pull_to_dir;

/// Pull an OCI image and extract it to a directory.
pub async fn pull(
    reference: &str,
    arch: &str,
    output: &Path,
    pubkey_pem: Option<&str>,
) -> Result<()> {
    tokio::fs::create_dir_all(output).await?;
    pull_to_dir(reference, arch, output, pubkey_pem).await
}

/// Sign an OCI image manifest in the registry.
pub async fn sign(reference: &str, privkey_pem: &str) -> Result<()> {
    oci::sign::sign_manifest(reference, privkey_pem).await
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[tokio::test]
    async fn pull_creates_output_directory_before_pull_attempt() {
        // ARRANGE
        let workspace = TempDir::new().expect("create temp dir");
        let output = workspace.path().join("nested/output");

        // ACT
        let error = pull("http://127.0.0.1:9/repo:test", "amd64", &output, None)
            .await
            .expect_err("pull should fail");

        // ASSERT
        assert!(output.is_dir());
        assert!(matches!(error, KociError::NetworkError(_)));
    }
}
