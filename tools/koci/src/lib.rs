//! OCI image pulling and manifest signing.

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

/// Return the OCI architecture string for the current host.
#[must_use]
pub fn host_oci_arch() -> &'static str {
    normalize_host_arch(std::env::consts::ARCH)
}

fn normalize_host_arch(arch: &str) -> &str {
    match arch {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        other => other,
    }
}

/// Pull an OCI image and extract it to a directory.
///
/// # Errors
///
/// Returns an error if the image cannot be fetched, verified, or extracted.
pub async fn pull(reference: &str, output: &Path, pubkey_pem: Option<&str>) -> error::Result<()> {
    pull_arch(reference, host_oci_arch(), output, pubkey_pem).await
}

/// Pull an OCI image for a specific architecture and extract it to a directory.
///
/// # Errors
///
/// Returns an error if the output directory cannot be prepared or the image pull fails.
pub async fn pull_arch(
    reference: &str,
    arch: &str,
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
        let error = pull_arch("http://127.0.0.1:9/repo:test", "amd64", &output, None)
            .await
            .expect_err("pull should fail");

        // ASSERT
        assert!(output.is_dir());
        assert!(matches!(error, KociError::NetworkError(_)));
    }

    #[test]
    fn normalize_host_arch_maps_known_architectures() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(normalize_host_arch("x86_64"), "amd64");
        assert_eq!(normalize_host_arch("aarch64"), "arm64");
    }

    #[test]
    fn normalize_host_arch_preserves_unknown_architectures() {
        // ARRANGE
        let arch = "riscv64";

        // ACT
        let normalized = normalize_host_arch(arch);

        // ASSERT
        assert_eq!(normalized, arch);
    }
}
