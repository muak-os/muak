//! Remote OCI registry pull orchestration.

use std::path::Path;

use crate::error::Result;
use crate::image::ImageReference;
use crate::image::manifest;
use crate::registry::auth::fetch_auth_token;
use crate::registry::http::build_client;
use crate::sign::verify;

mod download;
pub(crate) mod layer;

/// Pull an OCI image and extract all layers to `dest`.
pub(crate) async fn pull_to_dir(
    reference: &str,
    arch: &str,
    dest: &Path,
    signature_key: Option<&str>,
) -> Result<()> {
    let image_ref = ImageReference::parse(reference);
    let client = build_client();
    let target_arch = arch.to_owned();

    let token = fetch_auth_token(&client, &image_ref.registry, &image_ref.name).await?;
    let manifest_url = manifest::build_url(&image_ref, &image_ref.manifest_ref);
    let manifest_json = manifest::fetch(&client, &manifest_url, token.as_deref()).await?;
    let manifest = manifest::parse(&manifest_json)?;

    let layers = if manifest.manifests.is_empty() {
        verify::check_signature(&manifest_json, signature_key)?;
        manifest.layers
    } else {
        verify::check_signature(&manifest_json, signature_key)?;

        let selected = manifest::select_platform(&manifest.manifests, &target_arch)?;
        let platform_url = manifest::build_url(&image_ref, &selected.digest);
        let platform_json = manifest::fetch(&client, &platform_url, token.as_deref()).await?;

        verify::check_signature(&platform_json, signature_key)?;

        manifest::parse(&platform_json)?.layers
    };

    download::extract_layers(&client, &image_ref, &layers, token.as_deref(), dest).await
}
