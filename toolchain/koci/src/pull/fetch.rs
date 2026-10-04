//! Layer blob fetch orchestration for one pull session.

use std::collections::HashMap;
use std::path::PathBuf;

use oci::model::Descriptor;
use oci::reference::Image;
use oci_client::client::Client;
use oci_client::transport::Transport;
use tokio::task::JoinSet;

use super::content::Content;
use super::{download, scan};
use crate::error::{KociError, Result};
use crate::progress::Progress;
use crate::pull::layer::short_digest;

/// The fetched bytes of a layer, held in memory only when no store is configured.
pub(crate) fn cached_bytes(bytes: &[Option<Vec<u8>>], layer_idx: usize) -> Option<Vec<u8>> {
    bytes.get(layer_idx).and_then(Option::as_ref).cloned()
}

/// Open a decompressed streaming reader for one layer blob.
pub(crate) fn layer_reader<'bytes>(
    content: &Content,
    bytes: Option<&'bytes [u8]>,
    layer: &Descriptor,
) -> Result<download::LayerReader<'bytes>> {
    if let Some(bytes) = bytes {
        download::decompress(bytes, layer.media_type.as_deref())
    } else {
        let file = content
            .open_blob(&layer.digest)
            .map_err(KociError::IoError)?;
        download::decompress_file(file, layer.media_type.as_deref())
    }
}

/// Download every layer blob concurrently, then map whiteout targets to the first layer that must be hidden by them.
pub(crate) async fn download_all(
    client: &Client,
    content: &Content,
    layers: &[Descriptor],
    progress: &dyn Progress,
) -> Result<(Vec<Option<Vec<u8>>>, HashMap<PathBuf, usize>)> {
    let n = layers.len();

    let mut downloads = JoinSet::new();
    for (layer_idx, layer) in layers.iter().enumerate() {
        progress.layer_download_queued(layer_idx.saturating_add(1), n, short_digest(&layer.digest));
        let blob = fetch_blob(
            content.clone(),
            client.http().clone(),
            client.image().clone(),
            layer.digest.clone(),
            client.authorization().map(str::to_owned),
        );
        downloads.spawn(async move { (layer_idx, blob.await) });
    }

    let mut bytes: Vec<Option<Vec<u8>>> = std::iter::repeat_with(|| None).take(n).collect();
    while let Some(joined) = downloads.join_next().await {
        let (layer_idx, blob) = joined
            .map_err(|error| KociError::Pull(format!("layer download task failed: {error}")))?;
        let layer = layers.get(layer_idx).ok_or_else(|| {
            KociError::Pull(format!("missing download slot for layer {layer_idx}"))
        })?;
        progress.layer_downloaded(layer_idx.saturating_add(1), n, short_digest(&layer.digest));
        *bytes.get_mut(layer_idx).ok_or_else(|| {
            KociError::Pull(format!("missing download slot for layer {layer_idx}"))
        })? = blob?;
    }

    let whiteouts = scan_whiteouts(content, layers, &bytes)?;

    Ok((bytes, whiteouts))
}

fn scan_whiteouts(
    content: &Content,
    layers: &[Descriptor],
    bytes: &[Option<Vec<u8>>],
) -> Result<HashMap<PathBuf, usize>> {
    let mut whiteouts: HashMap<PathBuf, usize> = HashMap::new();
    for (layer_idx, layer) in layers.iter().enumerate() {
        let cached = cached_bytes(bytes, layer_idx);
        let mut reader = layer_reader(content, cached.as_deref(), layer)?;
        for whiteout in scan::scan_whiteouts(&mut reader)? {
            whiteouts.entry(whiteout).or_insert(layer_idx);
        }
    }

    Ok(whiteouts)
}

async fn fetch_blob(
    content: Content,
    http: Transport,
    image: Image,
    digest: String,
    authorization: Option<String>,
) -> Result<Option<Vec<u8>>> {
    if content.disk().is_some() {
        download::fetch_into_store(&content, &http, &image, &digest, authorization.as_deref())
            .await
            .map(|()| None)
    } else {
        download::blob(&http, &image, &digest, authorization.as_deref())
            .await
            .map(Some)
    }
}
