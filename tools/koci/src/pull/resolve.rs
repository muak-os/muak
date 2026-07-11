//! Resolving an image reference to its ordered list of OCI layer descriptors.

use super::cache::Store;
use crate::arch::Arch;
use crate::error::Result;
use crate::image::manifest;
use crate::image::{ImageReference, OciDescriptor};
use crate::registry::http::HttpClient;
use crate::sign::verify;

/// Resolve an image reference to the ordered list of layers for the target platform.
pub(crate) async fn layers(
    cache: &Store,
    client: &HttpClient,
    image_ref: &ImageReference,
    token: Option<&str>,
    arch: &Arch,
    pubkey_pem: Option<&str>,
) -> Result<Vec<OciDescriptor>> {
    let target_arch = arch.as_str().to_owned();
    let manifest_json =
        fetch_cached_manifest(cache, client, image_ref, &image_ref.manifest_ref, token).await?;
    let manifest = manifest::parse(&manifest_json)?;

    if manifest.manifests.is_empty() {
        verify::check_signature(&manifest_json, pubkey_pem)?;

        Ok(manifest.layers)
    } else {
        verify::check_signature(&manifest_json, pubkey_pem)?;
        let selected = manifest::select_platform(&manifest.manifests, &target_arch)?;
        let platform_json =
            fetch_cached_manifest(cache, client, image_ref, &selected.digest, token).await?;
        verify::check_signature(&platform_json, pubkey_pem)?;
        let platform_manifest = manifest::parse(&platform_json)?;

        Ok(platform_manifest.layers)
    }
}

/// Fetch a manifest, checking the local cache before hitting the network.
async fn fetch_cached_manifest(
    cache: &Store,
    client: &HttpClient,
    image_ref: &ImageReference,
    manifest_ref: &str,
    token: Option<&str>,
) -> Result<String> {
    let is_digest = manifest_ref.starts_with("sha256:");

    if is_digest {
        let cached = cache
            .blob_path(manifest_ref)
            .and_then(|path| std::fs::read_to_string(&path).ok());
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let url = manifest::build_url(image_ref, manifest_ref);
        let json = manifest::fetch(client, &url, token).await?;
        if let Some(path) = cache.blob_path(manifest_ref) {
            drop(std::fs::write(&path, json.as_bytes()));
        }

        Ok(json)
    } else {
        if let Some(cached) = cache.get_ref(&image_ref.registry, &image_ref.name, manifest_ref) {
            return Ok(cached);
        }
        let url = manifest::build_url(image_ref, manifest_ref);
        let json = manifest::fetch(client, &url, token).await?;
        cache.put_ref(&image_ref.registry, &image_ref.name, manifest_ref, &json);

        Ok(json)
    }
}
