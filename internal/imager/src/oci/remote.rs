use std::path::{Path, PathBuf};

use reqwest::Client;

use crate::error::{ImagerError, Result};
use crate::image::{ImageReference, OciDescriptor};
use crate::oci::auth::fetch_auth_token;
use crate::oci::layer;
use crate::oci::manifest;
use crate::oci::verify;
use crate::oci::{self, USER_AGENT};

/// Maximum number of concurrent layer downloads.
const MAX_CONCURRENT_DOWNLOADS: usize = 8;

pub(crate) async fn pull_to_temp(reference: &str, pubkey_pem: Option<&str>) -> Result<PathBuf> {
    let temp = oci::create_temp_dir("oci-")?;
    pull_to_dir(reference, temp.path(), pubkey_pem).await?;
    Ok(temp.keep())
}

pub(crate) async fn pull_to_dir(
    reference: &str,
    dest: &Path,
    cosign_key: Option<&str>,
) -> Result<()> {
    let image_ref = ImageReference::parse(reference);
    let client = Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| ImagerError::NetworkError(format!("Failed to create HTTP client: {}", e)))?;

    let token = fetch_auth_token(&client, &image_ref.registry, &image_ref.name).await?;
    let manifest_url = manifest::build_url(&image_ref, &image_ref.tag);
    let manifest_json = manifest::fetch(&client, &manifest_url, token.as_deref()).await?;
    let manifest = manifest::parse(&manifest_json)?;

    let layers = if !manifest.manifests.is_empty() {
        verify::check_cosign(
            &client,
            &image_ref,
            &manifest_json,
            token.as_deref(),
            cosign_key,
        )
        .await?;

        let selected_manifest = manifest::select_platform(&manifest.manifests)?;
        let platform_url = manifest::build_url(&image_ref, &selected_manifest.digest);
        let platform_json = manifest::fetch(&client, &platform_url, token.as_deref()).await?;
        let platform_manifest = manifest::parse(&platform_json)?;
        platform_manifest.layers
    } else {
        verify::check_cosign(
            &client,
            &image_ref,
            &manifest_json,
            token.as_deref(),
            cosign_key,
        )
        .await?;
        manifest.layers
    };

    let token_owned = token.as_deref().map(String::from);
    download_and_extract_layers(&client, &image_ref, &layers, token_owned.as_deref(), dest).await
}

/// Download and extract all layers concurrently with bounded parallelism.
async fn download_and_extract_layers(
    client: &Client,
    image_ref: &ImageReference,
    layers: &[OciDescriptor],
    token: Option<&str>,
    dest: &Path,
) -> Result<()> {
    let mut join_set = tokio::task::JoinSet::new();
    let token = token.map(String::from);
    let mut iter = layers.iter();

    spawn_layer_batch(&mut join_set, &mut iter, client, image_ref, &token, dest);

    while let Some(result) = join_set.join_next().await {
        result.map_err(|e| ImagerError::LayerExtractionError(e.to_string()))??;
        spawn_layer_batch(&mut join_set, &mut iter, client, image_ref, &token, dest);
    }

    Ok(())
}

/// Spawn layer download tasks until the concurrency limit is reached.
fn spawn_layer_batch<'a>(
    join_set: &mut tokio::task::JoinSet<Result<()>>,
    iter: &mut impl Iterator<Item = &'a OciDescriptor>,
    client: &Client,
    image_ref: &ImageReference,
    token: &Option<String>,
    dest: &Path,
) {
    while join_set.len() < MAX_CONCURRENT_DOWNLOADS {
        let Some(layer_desc) = iter.next() else {
            return;
        };
        let task = download_and_extract_layer(
            client.clone(),
            image_ref.clone(),
            layer_desc.digest.clone(),
            token.clone(),
            dest.to_path_buf(),
        );
        join_set.spawn(task);
    }
}

async fn download_and_extract_layer(
    client: Client,
    image_ref: ImageReference,
    digest: String,
    token: Option<String>,
    dest: PathBuf,
) -> Result<()> {
    let bytes = layer::download_blob(&client, &image_ref, &digest, token.as_deref()).await?;
    tokio::task::spawn_blocking(move || layer::extract_archive(&bytes, &dest))
        .await
        .map_err(|e| ImagerError::LayerExtractionError(e.to_string()))?
}
