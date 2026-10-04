//! Resolving an image reference to platform manifests and layer descriptors.

use oci::arch::Arch;
use oci::digest::sha256_hex;
use oci::model::Descriptor;
use oci_client::client::Client;
use oci_client::manifest;

use super::cache::Store;
use crate::error::Result;
use crate::signature;
use crate::signature::Verification;

/// Resolve an image reference to the ordered list of layers for the target platform.
pub(crate) async fn layers(
    client: &Client,
    cache: &Store,
    arch: &Arch,
    verification: Option<&Verification<'_>>,
) -> Result<Vec<Descriptor>> {
    if verification.is_none()
        && let Some(layers) = cached_layers(client, cache)
    {
        return Ok(layers);
    }

    let manifest_json = platform_manifest_json(client, cache, arch, verification).await?;
    let manifest = oci::manifest::parse(&manifest_json)?;
    cache.put_manifest_layers(
        client.image(),
        &content_digest(&manifest_json),
        &manifest.layers,
    );

    Ok(manifest.layers)
}

/// Fetch the platform manifest JSON for the target architecture.
pub(crate) async fn platform_manifest_json(
    client: &Client,
    cache: &Store,
    arch: &Arch,
    verification: Option<&Verification<'_>>,
) -> Result<String> {
    let manifest_json = fetch_cached_manifest(client, cache, &client.image().manifest_ref).await?;
    let manifest = oci::manifest::parse(&manifest_json)?;
    signature::check_signature(&manifest_json, verification)?;

    if manifest.manifests.is_empty() {
        return Ok(manifest_json);
    }

    let selected = oci::manifest::select_platform(&manifest.manifests, arch.as_str())?;
    let platform_json = fetch_cached_manifest(client, cache, &selected.digest).await?;
    signature::check_signature(&platform_json, verification)?;

    Ok(platform_json)
}

/// Fetch a manifest, checking the local cache before hitting the network.
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

fn cached_layers(client: &Client, cache: &Store) -> Option<Vec<Descriptor>> {
    let manifest_json = cache.get_manifest(client.image(), &client.image().manifest_ref)?;

    cache.manifest_layers(client.image(), &content_digest(&manifest_json))
}

fn content_digest(manifest_json: &str) -> String {
    format!("sha256:{}", sha256_hex(manifest_json.as_bytes()))
}
