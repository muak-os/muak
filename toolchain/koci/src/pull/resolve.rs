//! Resolving an image reference to platform manifests and layer descriptors.

use std::io::Read as _;

use oci::arch::Arch;
use oci::digest::sha256_hex;
use oci::model::{Descriptor, Manifest, Platform};
use oci_client::auth::Access;
use oci_client::client::Client;
use oci_client::manifest;

use super::cache::Store;
use super::content::Content;
use super::{Strategy, download};
use crate::error::{KociError, Result};
use crate::signature;
use crate::signature::Verification;

/// Resolve an image reference to the ordered layers and digest of the target platform.
pub(crate) async fn layers(
    client: &Client,
    cache: &Store,
    content: &Content,
    arch: &Arch,
    verification: Option<&Verification<'_>>,
    strategy: Strategy,
) -> Result<(Vec<Descriptor>, String)> {
    let (manifest_json, digest) =
        platform_manifest(client, cache, content, arch, verification, strategy).await?;
    let manifest = oci::manifest::parse(&manifest_json)?;

    Ok((manifest.layers, digest))
}

/// Fetch the platform manifest JSON for the target architecture.
pub(crate) async fn platform_manifest_json(
    client: &Client,
    cache: &Store,
    content: &Content,
    arch: &Arch,
    verification: Option<&Verification<'_>>,
    strategy: Strategy,
) -> Result<String> {
    platform_manifest(client, cache, content, arch, verification, strategy)
        .await
        .map(|(json, _digest)| json)
}

/// Fetch the platform manifest JSON and its digest for the target architecture.
pub(crate) async fn platform_manifest(
    client: &Client,
    cache: &Store,
    content: &Content,
    arch: &Arch,
    verification: Option<&Verification<'_>>,
    strategy: Strategy,
) -> Result<(String, String)> {
    let manifest_json =
        fetch_cached_manifest(client, cache, &client.image().manifest_ref, strategy).await?;
    let manifest = oci::manifest::parse(&manifest_json)?;
    signature::check_signature(&manifest_json, verification)?;

    if manifest.manifests.is_empty() {
        ensure_platform(client, content, &manifest, arch, strategy).await?;
        let digest = format!("sha256:{}", sha256_hex(manifest_json.as_bytes()));

        return Ok((manifest_json, digest));
    }

    let selected = oci::manifest::select_platform(&manifest.manifests, arch.as_str())?;
    let platform_json = fetch_cached_manifest(client, cache, &selected.digest, strategy).await?;
    signature::check_signature(&platform_json, verification)?;

    Ok((platform_json, selected.digest.clone()))
}

async fn ensure_platform(
    client: &Client,
    content: &Content,
    manifest: &Manifest,
    arch: &Arch,
    strategy: Strategy,
) -> Result<()> {
    let reference = client.image().manifest_ref.as_str();
    let Some(config) = manifest.config.as_ref() else {
        return Err(KociError::Pull(format!(
            "{reference} carries no image config, so its platform cannot be verified"
        )));
    };
    let bytes = match read_config(content, &config.digest)? {
        Some(bytes) => bytes,
        None => fetch_config(client, content, &config.digest, strategy).await?,
    };
    let json = core::str::from_utf8(&bytes)
        .map_err(|error| KociError::Pull(format!("{reference} config is not UTF-8: {error}")))?;
    let platform = oci::manifest::config_platform(json)?;

    check_linux_platform(reference, &platform, *arch)
}

async fn fetch_config(
    client: &Client,
    content: &Content,
    digest: &str,
    strategy: Strategy,
) -> Result<Vec<u8>> {
    if strategy == Strategy::CacheOnly {
        return Err(KociError::Offline {
            missing: format!("config blob {digest}"),
        });
    }
    let client = client.authenticate(Access::Pull).await?;
    let authorization = client.authorization().map(str::to_owned);
    let bytes = download::blob(
        client.http(),
        client.image(),
        digest,
        authorization.as_deref(),
        None,
    )
    .await?;
    content.store(digest, &bytes);

    Ok(bytes)
}

fn read_config(content: &Content, digest: &str) -> Result<Option<Vec<u8>>> {
    if content.has_blob(digest).is_none() {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    content
        .open_blob(digest)
        .map_err(KociError::IoError)?
        .read_to_end(&mut bytes)
        .map_err(KociError::IoError)?;

    Ok(Some(bytes))
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
    strategy: Strategy,
) -> Result<String> {
    let is_tag = !manifest_ref.starts_with("sha256:");
    if strategy == Strategy::CacheOnly {
        return cache
            .get_manifest(client.image(), manifest_ref)
            .ok_or_else(|| KociError::Offline {
                missing: format!("manifest {manifest_ref}"),
            });
    }
    if (strategy == Strategy::Network || !is_tag)
        && let Some(cached) = cache.get_manifest(client.image(), manifest_ref)
    {
        return Ok(cached);
    }

    let client = client.authenticate(Access::Pull).await?;
    let json = manifest::fetch(&client, manifest_ref).await?;
    cache.put_manifest(client.image(), manifest_ref, &json);

    Ok(json)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

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

    #[test]
    fn cache_only_reports_a_cold_manifest_miss() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let cache = Store::at(Some(tmp.path().to_path_buf()));
        let client = Client::local("127.0.0.1:1/repo:tag");

        // ACT
        let error = crate::runtime::runtime()
            .expect("runtime")
            .block_on(fetch_cached_manifest(
                &client,
                &cache,
                "tag",
                Strategy::CacheOnly,
            ))
            .expect_err("a cold cache must fail offline");

        // ASSERT
        assert!(matches!(error, KociError::Offline { .. }), "{error:?}");
    }

    #[test]
    fn cache_only_serves_a_warm_manifest_without_network() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let cache = Store::at(Some(tmp.path().to_path_buf()));
        let client = Client::local("127.0.0.1:1/repo:tag");
        let body = r#"{"schemaVersion":2,"layers":[]}"#;
        cache.put_manifest(client.image(), "tag", body);

        // ACT
        let cached = crate::runtime::runtime()
            .expect("runtime")
            .block_on(fetch_cached_manifest(
                &client,
                &cache,
                "tag",
                Strategy::CacheOnly,
            ));

        // ASSERT
        assert_eq!(cached.expect("warm cache hit"), body);
    }
}
