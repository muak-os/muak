//! Bounded parallel layer download scheduling.

use core::sync::atomic::{AtomicUsize, Ordering};

use oci::model::Descriptor;
use oci::reference::Image;
use oci_client::client::Client;
use oci_client::transport::Transport;
use tokio::task::JoinSet;

use super::cache::Store;
use super::content::Content;
use super::fetch::fetch_one;
use crate::error::{KociError, Result};
use crate::progress::Progress;
use crate::pull::layer::short_digest;

/// Default maximum number of layers downloading concurrently.
const DEFAULT_PARALLEL_DOWNLOADS: usize = 4;

/// Environment variable overriding the parallel download limit.
const PARALLEL_DOWNLOADS_ENV: &str = "KOCI_PARALLEL_DOWNLOADS";

/// Sentinel stored in [`PARALLEL_DOWNLOADS`] when no limit is configured.
const UNSET_PARALLEL_DOWNLOADS: usize = 0;

/// Process-wide parallel download limit, when set programmatically.
static PARALLEL_DOWNLOADS: AtomicUsize = AtomicUsize::new(UNSET_PARALLEL_DOWNLOADS);

/// Result of one worker batch: layer positions paired with their fetched bytes.
type FetchBatch = Vec<(usize, Result<Option<Vec<u8>>>)>;

/// Configure the maximum number of layers that download concurrently.
pub fn set_parallel_downloads(limit: usize) {
    if limit == UNSET_PARALLEL_DOWNLOADS {
        return;
    }

    PARALLEL_DOWNLOADS.store(limit, Ordering::Relaxed);
}

/// Download every layer blob with a bounded worker pool, into the store or memory.
pub(crate) async fn download_all(
    client: &Client,
    cache: &Store,
    content: &Content,
    layers: &[Descriptor],
    progress: &dyn Progress,
) -> Result<Vec<Option<Vec<u8>>>> {
    let total = layers.len();
    let workers = parallel_downloads().min(total);

    for (layer_idx, layer) in layers.iter().enumerate() {
        progress.layer_download_queued(
            layer_idx.saturating_add(1),
            total,
            short_digest(&layer.digest),
        );
    }

    let mut downloads = JoinSet::new();
    for worker_idx in 0..workers {
        let cache = cache.clone();
        let content = content.clone();
        let http = client.http().clone();
        let image = client.image().clone();
        let authorization = client.authorization().map(str::to_owned);
        let assigned = assigned_layers(worker_idx, workers, layers);
        downloads.spawn(download_batch(
            cache,
            content,
            http,
            image,
            authorization,
            assigned,
        ));
    }

    collect_downloads(downloads, layers, progress).await
}

fn parallel_downloads() -> usize {
    std::env::var(PARALLEL_DOWNLOADS_ENV)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|limit| *limit > 0)
        .unwrap_or_else(parallel_downloads_configured)
}

fn parallel_downloads_configured() -> usize {
    let configured = PARALLEL_DOWNLOADS.load(Ordering::Relaxed);
    if configured == UNSET_PARALLEL_DOWNLOADS {
        return DEFAULT_PARALLEL_DOWNLOADS;
    }

    configured
}

fn assigned_layers(
    worker_idx: usize,
    workers: usize,
    layers: &[Descriptor],
) -> Vec<(usize, Descriptor)> {
    (worker_idx..layers.len())
        .step_by(workers)
        .filter_map(|layer_idx| {
            layers
                .get(layer_idx)
                .map(|layer| (layer_idx, layer.clone()))
        })
        .collect()
}

async fn download_batch(
    cache: Store,
    content: Content,
    http: Transport,
    image: Image,
    authorization: Option<String>,
    assigned: Vec<(usize, Descriptor)>,
) -> FetchBatch {
    let mut results = Vec::with_capacity(assigned.len());
    for (layer_idx, layer) in assigned {
        let blob = fetch_one(
            &cache,
            &content,
            &http,
            &image,
            &layer.digest,
            authorization.as_deref(),
        )
        .await;
        results.push((layer_idx, blob));
    }

    results
}

async fn collect_downloads(
    mut downloads: JoinSet<FetchBatch>,
    layers: &[Descriptor],
    progress: &dyn Progress,
) -> Result<Vec<Option<Vec<u8>>>> {
    let mut bytes: Vec<Option<Vec<u8>>> = vec![None; layers.len()];
    while let Some(batch) = downloads.join_next().await {
        let batch = batch
            .map_err(|error| KociError::Pull(format!("layer download worker failed: {error}")))?;
        place_batch(batch, layers, progress, &mut bytes)?;
    }

    Ok(bytes)
}

fn place_batch(
    batch: FetchBatch,
    layers: &[Descriptor],
    progress: &dyn Progress,
    bytes: &mut [Option<Vec<u8>>],
) -> Result<()> {
    let total = layers.len();
    for (layer_idx, blob) in batch {
        let layer = layers.get(layer_idx).ok_or_else(|| {
            KociError::Pull(format!("missing download slot for layer {layer_idx}"))
        })?;
        progress.layer_downloaded(
            layer_idx.saturating_add(1),
            total,
            short_digest(&layer.digest),
        );
        let slot = bytes.get_mut(layer_idx).ok_or_else(|| {
            KociError::Pull(format!("missing download slot for layer {layer_idx}"))
        })?;
        *slot = blob?;
    }

    Ok(())
}
