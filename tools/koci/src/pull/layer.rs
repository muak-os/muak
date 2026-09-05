//! OCI layer downloading, decompression, and tar entry iteration.

use alloc::collections::BTreeMap;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tar::Archive;
use tokio::task::JoinSet;

use super::entries::FileEntry;
use super::{download, resolve, scan};
use crate::arch::Arch;
use crate::error::{KociError, Result};
use crate::image::OciDescriptor;
use crate::registry::auth::Access;
use crate::registry::session::Session;

/// Stream every live file entry of the image's platform layers.
///
/// # Errors
///
/// Returns an error if the image cannot be fetched, signature verification
/// fails, a layer cannot be decompressed, or the handler returns an error.
pub(crate) async fn files<F>(
    reference: &str,
    arch: &Arch,
    pubkey_pem: Option<&str>,
    mut handler: F,
) -> Result<()>
where
    F: FnMut(FileEntry<'_>) -> Result<()>,
{
    let session = Session::new(reference, Access::Pull, None).await?;
    eprintln!("Pulling {reference} for {}", arch.as_str());
    let layers = resolve::layers(&session, arch, pubkey_pem).await?;
    eprintln!("Resolved {} layer(s)", layers.len());

    walk(&session, &layers, |_layer_idx, entry, info| {
        scan::handle_file_entry(entry, info, &mut handler)
    })
    .await
}

/// Collect the byte size of every live file entry, keyed by normalized path.
///
/// # Errors
///
/// Returns an error if a layer cannot be downloaded or decompressed.
pub(crate) async fn entry_sizes(
    session: &Session,
    layers: &[OciDescriptor],
    exclude: &[String],
) -> Result<BTreeMap<String, u64>> {
    let mut sizes = BTreeMap::new();

    walk(session, layers, |_layer_idx, _entry, info| {
        if let scan::EntryInfo::File(path, size, _) = info
            && !excluded(&path, exclude)
        {
            sizes.insert(path.to_string_lossy().to_string(), size);
        }

        Ok(())
    })
    .await?;

    Ok(sizes)
}

/// Download all layers, then iterate every archive entry not blocked by a whiteout.
async fn walk<F>(session: &Session, layers: &[OciDescriptor], mut on_entry: F) -> Result<()>
where
    F: for<'a, 'b> FnMut(
        usize,
        tar::Entry<&'b mut download::LayerReader<'a>>,
        scan::EntryInfo,
    ) -> Result<()>,
{
    let (blobs, whiteouts) = download_all(session, layers).await?;
    let n = layers.len();

    for (layer_idx, layer) in layers.iter().enumerate() {
        let data = blobs.get(layer_idx).ok_or_else(|| {
            KociError::DownloadError(format!("missing layer bytes for layer {layer_idx}"))
        })?;
        eprintln!(
            "Extracting layer {}/{}: {}",
            layer_idx.saturating_add(1),
            n,
            short_digest(&layer.digest)
        );
        let mut reader = download::decompress(data, layer.media_type.as_deref())?;
        extract_layer(&mut reader, layer_idx, &whiteouts, &mut on_entry)?;
    }

    Ok(())
}

/// Iterate one layer's archive, skipping entries blocked by whiteouts.
fn extract_layer<'a, F>(
    reader: &mut download::LayerReader<'a>,
    layer_idx: usize,
    whiteouts: &HashMap<PathBuf, usize>,
    on_entry: &mut F,
) -> Result<()>
where
    F: FnMut(usize, tar::Entry<&mut download::LayerReader<'a>>, scan::EntryInfo) -> Result<()>,
{
    let mut archive = Archive::new(reader);
    let entries = archive.entries()?;
    for entry_result in entries {
        let entry = entry_result?;
        let info = scan::classify_tar_entry(&entry)?;
        if blocked_by_whiteout(&info, layer_idx, whiteouts) {
            continue;
        }
        on_entry(layer_idx, entry, info)?;
    }

    Ok(())
}

/// Download every layer blob concurrently, then map whiteout targets to the first layer that must be hidden by them.
async fn download_all(
    session: &Session,
    layers: &[OciDescriptor],
) -> Result<(Vec<Vec<u8>>, HashMap<PathBuf, usize>)> {
    let n = layers.len();

    let mut downloads = JoinSet::new();
    for (layer_idx, layer) in layers.iter().enumerate() {
        let cache = session.cache.clone();
        let client = session.client.clone();
        let image = session.image.clone();
        let authorization = session.authorization().map(str::to_owned);
        let digest = layer.digest.clone();
        downloads.spawn(async move {
            let layer_number = layer_idx.saturating_add(1);
            let short = short_digest(&digest);
            eprintln!("Downloading layer {layer_number}/{n}: {short}");
            (
                layer_idx,
                download::cached(&cache, &client, &image, &digest, authorization.as_deref()).await,
            )
        });
    }

    let mut blobs: Vec<Option<Result<Vec<u8>>>> = std::iter::repeat_with(|| None).take(n).collect();
    while let Some(joined) = downloads.join_next().await {
        let (layer_idx, blob) = joined.map_err(|error| {
            KociError::NetworkError(format!("layer download task failed: {error}"))
        })?;
        *blobs.get_mut(layer_idx).ok_or_else(|| {
            KociError::DownloadError(format!("missing download slot for layer {layer_idx}"))
        })? = Some(blob);
    }

    let mut bytes = Vec::with_capacity(n);
    let mut whiteouts: HashMap<PathBuf, usize> = HashMap::new();
    for (layer_idx, layer) in layers.iter().enumerate() {
        let blob = blobs
            .get_mut(layer_idx)
            .and_then(Option::take)
            .ok_or_else(|| {
                KociError::DownloadError(format!("missing download for layer {layer_idx}"))
            })??;
        let reader = download::decompress(&blob, layer.media_type.as_deref())?;
        for whiteout in scan::scan_whiteouts(reader)? {
            whiteouts.entry(whiteout).or_insert(layer_idx);
        }
        bytes.push(blob);
    }

    Ok((bytes, whiteouts))
}

fn short_digest(digest: &str) -> &str {
    if let Some(hash) = digest.strip_prefix("sha256:") {
        hash.get(..12).unwrap_or(hash)
    } else {
        digest
    }
}

/// Whether a file entry is deleted by a whiteout recorded in a later layer.
fn blocked_by_whiteout(
    info: &scan::EntryInfo,
    layer_idx: usize,
    whiteouts: &HashMap<PathBuf, usize>,
) -> bool {
    matches!(
        info,
        scan::EntryInfo::File(path, ..)
            if whiteouts.get(path).is_some_and(|&blocking| blocking > layer_idx)
    )
}

/// Whether a normalized entry path matches an exclusion prefix at a path segment boundary.
fn excluded(path: &Path, exclude: &[String]) -> bool {
    let text = path.to_string_lossy();

    exclude.iter().any(|prefix| {
        text == prefix.as_str()
            || text
                .strip_prefix(prefix.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excluded_matches_exact_path_and_directory_prefixes() {
        // ARRANGE
        let exclude = ["lib/modules".to_owned(), "etc/motd".to_owned()];

        // ACT / ASSERT
        assert!(excluded(Path::new("lib/modules"), &exclude));
        assert!(excluded(
            Path::new("lib/modules/7.2.0/kernel/x.ko"),
            &exclude
        ));
        assert!(excluded(Path::new("etc/motd"), &exclude));
    }

    #[test]
    fn excluded_requires_segment_boundary() {
        // ARRANGE
        let exclude = ["lib/modules".to_owned()];

        // ACT / ASSERT
        assert!(!excluded(Path::new("lib/modules.builtin"), &exclude));
        assert!(!excluded(Path::new("vmlinuz"), &exclude));
    }
}
