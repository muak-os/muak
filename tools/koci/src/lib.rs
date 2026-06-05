//! OCI image pulling and manifest signing.

#![warn(missing_docs)]

pub mod arch;
#[cfg(feature = "cli")]
pub mod cli;
mod digest;
pub mod error;
mod image;
mod pull;
mod registry;
mod sign;

use std::path::Path;

use tokio::fs::create_dir_all;

/// Pull an OCI image and extract it to a directory.
///
/// # Errors
///
/// Returns an error if the image cannot be fetched, verified, or extracted.
pub async fn pull(reference: &str, output: &Path, pubkey_pem: Option<&str>) -> error::Result<()> {
    pull_arch(reference, &arch::host(), output, pubkey_pem).await
}

/// Pull an OCI image for a specific architecture and extract it to a directory.
///
/// # Errors
///
/// Returns an error if the output directory cannot be prepared or the image pull fails.
pub async fn pull_arch(
    reference: &str,
    arch: &arch::Arch,
    output: &Path,
    pubkey_pem: Option<&str>,
) -> error::Result<()> {
    create_dir_all(output).await?;
    pull::pull_to_dir(reference, arch, output, pubkey_pem).await
}

/// Sign an OCI image manifest in the registry.
///
/// # Errors
///
/// Returns an error if the manifest cannot be fetched, signed, or pushed.
pub async fn sign(reference: &str, privkey_pem: &str) -> error::Result<()> {
    sign::sign_manifest(reference, privkey_pem).await
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::error::KociError;

    #[tokio::test]
    async fn pull_creates_output_directory_before_pull_attempt() {
        // ARRANGE
        let workspace = TempDir::new().expect("create temp dir");
        let output = workspace.path().join("nested/output");

        // ACT
        let error = pull_arch(
            "http://127.0.0.1:9/repo:test",
            &arch::Arch::Amd64,
            &output,
            None,
        )
        .await
        .expect_err("pull should fail");

        // ASSERT
        assert!(output.is_dir());
        assert!(matches!(error, KociError::NetworkError(_)));
    }
}
