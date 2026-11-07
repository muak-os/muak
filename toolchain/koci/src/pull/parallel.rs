//! Bounded parallel layer download scheduling.

use core::sync::atomic::{AtomicUsize, Ordering};

use oci::model::Descriptor;
use oci_client::auth::Access;
use oci_client::client::Client;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::task::JoinSet;

use super::Strategy;
use super::content::Content;
use super::download::ByteProgress;
use super::fetch::fetch_one;
use crate::error::{KociError, Result};
use crate::progress::{ByteSample, Event, Phase, Reporter, Unit};

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
    content: &Content,
    layers: &[Descriptor],
    reporter: &dyn Reporter,
    strategy: Strategy,
) -> Result<Vec<Option<Vec<u8>>>> {
    let client = if strategy == Strategy::CacheOnly || !needs_network(content, layers) {
        client.clone()
    } else {
        client.authenticate(Access::Pull).await?
    };
    let total = layers.len();
    let workers = parallel_downloads().min(total);
    let total_bytes = layers
        .iter()
        .fold(0_u64, |sum, layer| sum.saturating_add(layer.size));

    for (layer_idx, layer) in layers.iter().enumerate() {
        reporter.report(Event::ItemQueued {
            phase: Phase::Download,
            index: layer_idx.saturating_add(1),
            total,
            digest: &layer.digest,
            size: layer.size,
        });
    }
    reporter.report(Event::PhaseStarted {
        phase: Phase::Download,
        label: "Downloading layers",
        total: total_bytes,
        unit: Unit::Bytes,
    });

    let (sender, receiver) = unbounded_channel();
    let mut downloads = JoinSet::new();
    for worker_idx in 0..workers {
        let content = content.clone();
        let worker_client = client.clone();
        let assigned = assigned_layers(worker_idx, workers, layers);
        downloads.spawn(download_batch(
            content,
            worker_client,
            assigned,
            sender.clone(),
            strategy,
        ));
    }
    drop(sender);

    let bytes = collect_downloads(downloads, layers, reporter, receiver).await?;
    if total > 0 {
        reporter.report(Event::PhaseCompleted {
            phase: Phase::Download,
            label: &format!("Downloaded {total}/{total} layers"),
        });
    }

    Ok(bytes)
}

fn needs_network(content: &Content, layers: &[Descriptor]) -> bool {
    layers
        .iter()
        .any(|layer| content.has_blob(&layer.digest).is_none())
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
    content: Content,
    client: Client,
    assigned: Vec<(usize, Descriptor)>,
    sender: UnboundedSender<ByteSample>,
    strategy: Strategy,
) -> FetchBatch {
    let mut results = Vec::with_capacity(assigned.len());
    for (layer_idx, layer) in assigned {
        let index = layer_idx.saturating_add(1);
        let progress = ByteProgress::new(sender.clone(), index, layer.size);
        let blob = fetch_one(&content, &client, &layer.digest, Some(progress), strategy).await;
        results.push((layer_idx, blob));
    }

    results
}

async fn collect_downloads(
    mut downloads: JoinSet<FetchBatch>,
    layers: &[Descriptor],
    reporter: &dyn Reporter,
    mut receiver: UnboundedReceiver<ByteSample>,
) -> Result<Vec<Option<Vec<u8>>>> {
    let mut bytes: Vec<Option<Vec<u8>>> = vec![None; layers.len()];
    loop {
        tokio::select! {
            Some(batch) = downloads.join_next() => {
                let batch = batch.map_err(|error| {
                    KociError::Pull(format!("layer download worker failed: {error}"))
                })?;
                place_batch(batch, layers, reporter, &mut bytes)?;
            }
            Some(sample) = receiver.recv() => {
                reporter.bytes(sample);
            }
            else => break,
        }
    }

    Ok(bytes)
}

fn place_batch(
    batch: FetchBatch,
    layers: &[Descriptor],
    reporter: &dyn Reporter,
    bytes: &mut [Option<Vec<u8>>],
) -> Result<()> {
    for (layer_idx, blob) in batch {
        let layer = layers.get(layer_idx).ok_or_else(|| {
            KociError::Pull(format!("missing download slot for layer {layer_idx}"))
        })?;
        reporter.report(Event::ItemDone {
            phase: Phase::Download,
            index: layer_idx.saturating_add(1),
            size: layer.size,
            digest: &layer.digest,
        });
        let slot = bytes.get_mut(layer_idx).ok_or_else(|| {
            KociError::Pull(format!("missing download slot for layer {layer_idx}"))
        })?;
        *slot = blob?;
    }

    Ok(())
}
