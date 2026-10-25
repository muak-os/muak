//! Layer blob fetch orchestration for one pull session.

use std::collections::HashMap;
use std::path::PathBuf;

use oci::model::Descriptor;
use oci::reference::Image;
use oci_client::blob::build_url;
use oci_client::error::ClientError;
use oci_client::transport::Transport;

use super::cache::Store;
use super::content::Content;
use super::download;
use super::download::ByteProgress;
use super::{blobinfo, scan};
use crate::error::{KociError, Result};
use crate::progress::{Event, Phase, Reporter, Unit};

/// HTTP status code for a blob the registry does not serve.
const NOT_FOUND: u16 = 404;

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
        download::decompress(
            download::BlobSource::Borrowed(bytes),
            layer.media_type.as_deref(),
        )
    } else {
        let file = content
            .open_blob(&layer.digest)
            .map_err(KociError::IoError)?;
        download::decompress(
            download::BlobSource::File(file),
            layer.media_type.as_deref(),
        )
    }
}

/// Map whiteout targets to the first layer that must hide them.
pub(crate) fn whiteout_map(
    content: &Content,
    layers: &[Descriptor],
    bytes: &[Option<Vec<u8>>],
    reporter: &dyn Reporter,
) -> Result<HashMap<PathBuf, usize>> {
    if layers.len() < 2 {
        return Ok(HashMap::new());
    }

    let total = layers.len();
    let total_u64 = u64::try_from(total).unwrap_or(u64::MAX);
    reporter.report(Event::PhaseStarted {
        phase: Phase::Scan,
        label: "Scanning layers",
        total: total_u64,
        unit: Unit::Items,
    });
    let mut whiteouts: HashMap<PathBuf, usize> = HashMap::new();
    for (layer_idx, layer) in layers.iter().enumerate() {
        let current = u64::try_from(layer_idx.saturating_add(1)).unwrap_or(u64::MAX);
        reporter.report(Event::Progress {
            phase: Phase::Scan,
            current,
            total: total_u64,
        });
        let cached = cached_bytes(bytes, layer_idx);
        let mut reader = layer_reader(content, cached.as_deref(), layer)?;
        for whiteout in scan::scan_whiteouts(&mut reader)? {
            whiteouts.entry(whiteout).or_insert(layer_idx);
        }
    }

    Ok(whiteouts)
}

/// Fetch one blob for a session, consulting and updating the blob info cache.
pub(crate) async fn fetch_one(
    cache: &Store,
    content: &Content,
    http: &Transport,
    image: &Image,
    digest: &str,
    authorization: Option<&str>,
    progress: Option<ByteProgress>,
) -> Result<Option<Vec<u8>>> {
    if cache.layer_location(image, digest) == Some(blobinfo::LayerLocation::Missing) {
        return Err(ClientError::Status {
            status: NOT_FOUND,
            url: build_url(image, digest),
        }
        .into());
    }

    match fetch_bytes(cache, content, http, image, digest, authorization, progress).await {
        Ok(bytes) => Ok(bytes),
        Err(KociError::Client(
            error @ ClientError::Status {
                status: NOT_FOUND, ..
            },
        )) => {
            cache.put_blob_missing(image, digest);
            Err(KociError::Client(error))
        }
        Err(other) => Err(other),
    }
}

/// Download one blob, keeping it in the store when available, and record its presence.
async fn fetch_bytes(
    cache: &Store,
    content: &Content,
    http: &Transport,
    image: &Image,
    digest: &str,
    authorization: Option<&str>,
    progress: Option<ByteProgress>,
) -> Result<Option<Vec<u8>>> {
    let fetched = if content.disk().is_some() {
        download::fetch_into_store(content, http, image, digest, authorization, progress)
            .await
            .map(|size| (size, None))
    } else {
        download::blob(http, image, digest, authorization, progress)
            .await
            .map(|bytes| {
                let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
                (size, Some(bytes))
            })
    };

    let (size, bytes) = fetched?;
    cache.put_blob_present(image, digest, size);

    Ok(bytes)
}
