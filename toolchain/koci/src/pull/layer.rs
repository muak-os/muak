//! OCI layer downloading, decompression, and tar entry iteration.

#[cfg(feature = "annotate")]
use alloc::collections::BTreeMap;
use std::collections::HashMap;
#[cfg(feature = "annotate")]
use std::path::Path;
use std::path::PathBuf;

use oci::model::Descriptor;
use oci_client::client::Client;
use tar::Archive;

use super::cache::Store;
use super::content::Content;
use super::download;
use super::fetch;
use super::scan;
use crate::error::Result;
use crate::progress::Progress;

/// Collect the byte size of every live file entry, keyed by normalized path.
///
/// # Errors
///
/// Returns an error if a layer cannot be downloaded or decompressed.
#[cfg(feature = "annotate")]
pub(crate) async fn entry_sizes(
    client: &Client,
    layers: &[Descriptor],
    exclude: &[String],
    progress: &dyn Progress,
) -> Result<BTreeMap<String, u64>> {
    let mut sizes = BTreeMap::new();

    walk(
        client,
        &Store::new(),
        &Content::new(),
        layers,
        progress,
        |_layer_idx, _entry, info| {
            if let scan::EntryInfo::File(path, size, _) = info
                && !excluded(path, exclude)
            {
                sizes.insert(scan::path_string(path), size);
            }

            Ok(())
        },
    )
    .await?;

    Ok(sizes)
}

/// Download all layers, then iterate every archive entry not blocked by a whiteout.
pub(crate) async fn walk<F>(
    client: &Client,
    cache: &Store,
    content: &Content,
    layers: &[Descriptor],
    progress: &dyn Progress,
    mut on_entry: F,
) -> Result<()>
where
    F: for<'a, 'b> FnMut(
        usize,
        tar::Entry<&'a mut download::LayerReader>,
        scan::EntryInfo<'b>,
    ) -> Result<()>,
{
    let bytes = fetch::download_all(client, cache, content, layers, progress).await?;
    let whiteouts = fetch::whiteout_map(content, layers, &bytes, progress)?;
    let n = layers.len();

    for (layer_idx, layer) in layers.iter().enumerate() {
        progress.layer_extracting(layer_idx.saturating_add(1), n, short_digest(&layer.digest));
        let cached = fetch::cached_bytes(&bytes, layer_idx);
        let mut reader = fetch::layer_reader(content, cached.as_deref(), layer)?;
        extract_layer(&mut reader, layer_idx, &whiteouts, &mut on_entry)?;
    }

    Ok(())
}

/// The short digest prefix used for progress lines.
pub(crate) fn short_digest(digest: &str) -> &str {
    if let Some(hash) = digest.strip_prefix("sha256:") {
        hash.get(..12).unwrap_or(hash)
    } else {
        digest
    }
}

/// Iterate one layer's archive, skipping entries blocked by whiteouts.
fn extract_layer<F>(
    reader: &mut download::LayerReader,
    layer_idx: usize,
    whiteouts: &HashMap<PathBuf, usize>,
    on_entry: &mut F,
) -> Result<()>
where
    F: for<'a, 'b> FnMut(
        usize,
        tar::Entry<&'a mut download::LayerReader>,
        scan::EntryInfo<'b>,
    ) -> Result<()>,
{
    let mut archive = Archive::new(reader);
    let entries = archive.entries()?;
    let mut scratch = PathBuf::new();
    for entry_result in entries {
        let entry = entry_result?;
        let info = scan::classify_tar_entry(&entry, &mut scratch)?;
        if blocked_by_whiteout(&info, layer_idx, whiteouts) {
            continue;
        }
        on_entry(layer_idx, entry, info)?;
    }

    Ok(())
}

/// Whether a file entry is deleted by a whiteout recorded in a later layer.
fn blocked_by_whiteout(
    info: &scan::EntryInfo<'_>,
    layer_idx: usize,
    whiteouts: &HashMap<PathBuf, usize>,
) -> bool {
    matches!(
        info,
        scan::EntryInfo::File(path, ..)
            if whiteouts.get(*path).is_some_and(|&blocking| blocking > layer_idx)
    )
}

/// Whether a normalized entry path matches an exclusion prefix at a path segment boundary.
#[cfg(feature = "annotate")]
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
