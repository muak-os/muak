//! Resolving an image reference to platform manifests and layer descriptors.

use oci::arch::Arch;
use oci::digest::sha256_hex;
use oci::model::{Descriptor, Manifest, Platform};
use oci_client::client::Client;
use oci_client::manifest;

use super::cache::Store;
use super::download;
use crate::error::{KociError, Result};
use crate::signature;
use crate::signature::Verification;

/// Resolve an image reference to the ordered layers and digest of the target platform.
pub(crate) async fn layers(
    client: &Client,
    cache: &Store,
    arch: &Arch,
    verification: Option<&Verification<'_>>,
) -> Result<(Vec<Descriptor>, String)> {
    let (manifest_json, digest) = platform_manifest(client, cache, arch, verification).await?;
    let manifest = oci::manifest::parse(&manifest_json)?;

    Ok((manifest.layers, digest))
}

/// Fetch the platform manifest JSON for the target architecture.
pub(crate) async fn platform_manifest_json(
    client: &Client,
    cache: &Store,
    arch: &Arch,
    verification: Option<&Verification<'_>>,
) -> Result<String> {
    platform_manifest(client, cache, arch, verification)
        .await
        .map(|(json, _digest)| json)
}

/// Fetch the platform manifest JSON and its digest for the target architecture.
pub(crate) async fn platform_manifest(
    client: &Client,
    cache: &Store,
    arch: &Arch,
    verification: Option<&Verification<'_>>,
) -> Result<(String, String)> {
    let manifest_json = fetch_cached_manifest(client, cache, &client.image().manifest_ref).await?;
    let manifest = oci::manifest::parse(&manifest_json)?;
    signature::check_signature(&manifest_json, verification)?;

    if manifest.manifests.is_empty() {
        ensure_platform(client, &manifest, arch).await?;
        let digest = format!("sha256:{}", sha256_hex(manifest_json.as_bytes()));

        return Ok((manifest_json, digest));
    }

    let selected = oci::manifest::select_platform(&manifest.manifests, arch.as_str())?;
    let platform_json = fetch_cached_manifest(client, cache, &selected.digest).await?;
    signature::check_signature(&platform_json, verification)?;

    Ok((platform_json, selected.digest.clone()))
}

async fn ensure_platform(client: &Client, manifest: &Manifest, arch: &Arch) -> Result<()> {
    let reference = client.image().manifest_ref.as_str();
    let Some(config) = manifest.config.as_ref() else {
        return Err(KociError::Pull(format!(
            "{reference} carries no image config, so its platform cannot be verified"
        )));
    };
    let authorization = client.authorization().map(str::to_owned);
    let bytes = download::blob(
        client.http(),
        client.image(),
        &config.digest,
        authorization.as_deref(),
        None,
    )
    .await?;
    let json = core::str::from_utf8(&bytes)
        .map_err(|error| KociError::Pull(format!("{reference} config is not UTF-8: {error}")))?;
    let platform = oci::manifest::config_platform(json)?;

    check_linux_platform(reference, &platform, *arch)
}

fn check_linux_platform(reference: &str, platform: &Platform, arch: Arch) -> Result<()> {
    if oci::manifest::platform_matches(platform, arch.as_str()) {
        return Ok(());
    }

    Err(KociError::Pull(format!(
        "{reference} targets {}/{}, but linux/{} is required",
        platform.os.as_deref().unwrap_or("unknown"),
        platform.architecture.as_deref().unwrap_or("unknown"),
        arch.as_str(),
    )))
}

async fn fetch_cached_manifest(
    client: &Client,
    cache: &Store,
    manifest_ref: &str,
) -> Result<String> {
    if let Some(cached) = cache.get_manifest(client.image(), manifest_ref) {
        return Ok(cached);
    }

    let json = manifest::fetch(client, manifest_ref).await?;
    cache.put_manifest(client.image(), manifest_ref, &json);

    Ok(json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn platform(architecture: &str, os: &str) -> Platform {
        Platform {
            architecture: Some(architecture.to_owned()),
            os: Some(os.to_owned()),
        }
    }

    #[test]
    fn check_platform_accepts_matching_target() {
        // ARRANGE
        let platform = platform("arm64", "linux");

        // ACT
        let result = check_linux_platform("reg.io/img:tag", &platform, Arch::Arm64);

        // ASSERT
        result.expect("matching platform must be accepted");
    }

    #[test]
    fn check_platform_rejects_wrong_architecture() {
        // ARRANGE
        let platform = platform("amd64", "linux");

        // ACT
        let error = check_linux_platform("reg.io/img:tag", &platform, Arch::Arm64)
            .expect_err("wrong architecture must fail");

        // ASSERT
        let message = error.to_string();
        assert!(message.contains("linux/amd64"));
        assert!(message.contains("linux/arm64"));
    }

    #[test]
    fn check_platform_rejects_missing_platform() {
        // ARRANGE
        let platform = Platform::default();

        // ACT
        let error = check_linux_platform("reg.io/img:tag", &platform, Arch::Amd64)
            .expect_err("missing platform must fail");

        // ASSERT
        assert!(error.to_string().contains("unknown/unknown"));
    }
}
