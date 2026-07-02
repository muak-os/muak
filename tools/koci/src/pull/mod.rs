//! Remote OCI registry pull orchestration.

use crate::arch::Arch;
use crate::error::{KociError, Result};
use crate::image::ImageReference;
use crate::image::manifest;
use crate::pulled::PulledImage;
use crate::registry::auth::fetch_auth_token;
use crate::registry::http::{HttpClient, build_client};
use crate::sign::verify;

pub mod cache;
mod download;
pub(crate) mod layer;

/// Pull an OCI image and materialize all layers into a merged in-memory image.
pub(crate) async fn pull_image(
    reference: &str,
    arch: &Arch,
    signature_key: Option<&str>,
) -> Result<PulledImage> {
    let cache = cache::BlobCache::new();
    let image_ref = ImageReference::parse(reference);
    let client = build_client();
    let target_arch = arch.as_str().to_owned();

    let token = fetch_auth_token(&client, &image_ref.registry, &image_ref.name).await?;
    let manifest_json = fetch_cached_manifest(
        &cache,
        &client,
        &image_ref,
        &image_ref.manifest_ref,
        token.as_deref(),
    )
    .await?;
    let manifest = manifest::parse(&manifest_json)?;

    let layers = if manifest.manifests.is_empty() {
        verify::check_signature(&manifest_json, signature_key)?;
        manifest.layers
    } else {
        verify::check_signature(&manifest_json, signature_key)?;

        let selected = manifest::select_platform(&manifest.manifests, &target_arch)?;
        let platform_json = fetch_cached_manifest(
            &cache,
            &client,
            &image_ref,
            &selected.digest,
            token.as_deref(),
        )
        .await?;

        verify::check_signature(&platform_json, signature_key)?;

        manifest::parse(&platform_json)?.layers
    };

    download::pull_layers(&client, &image_ref, &layers, token.as_deref(), &cache).await
}

/// Fetch a manifest, checking the local cache before hitting the network.
async fn fetch_cached_manifest(
    cache: &cache::BlobCache,
    client: &HttpClient,
    image_ref: &ImageReference,
    manifest_ref: &str,
    token: Option<&str>,
) -> Result<String> {
    let is_digest = manifest_ref.starts_with("sha256:");

    if is_digest {
        if let Some(cached) = cache.get_blob(manifest_ref) {
            return utf8_bytes_to_string(cached);
        }
        let url = manifest::build_url(image_ref, manifest_ref);
        let json = manifest::fetch(client, &url, token).await?;
        cache.put_blob(manifest_ref, json.as_bytes());

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

fn utf8_bytes_to_string(bytes: Vec<u8>) -> Result<String> {
    let text = String::from_utf8(bytes).map_err(|err| {
        KociError::OciParseError(format!("cached manifest is not valid UTF-8: {err}"))
    })?;

    Ok(text)
}
