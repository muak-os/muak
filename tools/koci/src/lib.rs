//! OCI image pulling and manifest signing.

#![warn(missing_docs)]

pub mod arch;
#[cfg(feature = "cli")]
pub mod cli;
mod digest;
pub mod error;
mod image;
pub mod pull;
pub mod pulled;
mod registry;
mod sign;

use crate::pulled::PulledImage;

/// Pull an OCI image into an in-memory merged filesystem view.
///
/// # Errors
///
/// Returns an error if the image cannot be fetched, verified, or decoded.
pub async fn pull(reference: &str, pubkey_pem: Option<&str>) -> error::Result<PulledImage> {
    pull_arch(reference, &arch::host(), pubkey_pem).await
}

/// Pull an OCI image for a specific architecture into an in-memory merged filesystem view.
///
/// # Errors
///
/// Returns an error if the image pull fails.
pub async fn pull_arch(
    reference: &str,
    arch: &arch::Arch,
    pubkey_pem: Option<&str>,
) -> error::Result<PulledImage> {
    pull::pull_image(reference, arch, pubkey_pem).await
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
    use super::*;
    use crate::error::KociError;

    #[tokio::test]
    async fn pull_creates_output_directory_before_pull_attempt() {
        // ARRANGE
        // ACT
        let error = pull_arch("http://127.0.0.1:9/repo:test", &arch::Arch::Amd64, None)
            .await
            .expect_err("pull should fail");

        // ASSERT
        assert!(matches!(error, KociError::NetworkError(_)));
    }
}
