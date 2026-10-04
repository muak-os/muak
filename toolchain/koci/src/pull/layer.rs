//! OCI layer downloading, decompression, and tar entry iteration.

#[cfg(feature = "annotate")]
use alloc::collections::BTreeMap;
use std::collections::HashMap;
#[cfg(feature = "annotate")]
use std::path::Path;
use std::path::PathBuf;

use oci::arch::Arch;
use oci::model::Descriptor;
use oci_client::auth::Access;
use oci_client::client::Client;
use tar::Archive;

use super::cache::Store;
use super::content::Content;
use super::entries::FileEntry;
use super::fetch;
use super::{download, resolve, scan};
use crate::error::Result;
use crate::progress::Progress;
use crate::registry;
use crate::signature::Verification;

/// Stream every live file entry of the image's platform layers.
///
/// # Errors
///
/// Returns an error if the image cannot be fetched, signature verification
/// fails, a layer cannot be decompressed, or the handler returns an error.
pub(crate) async fn files<F>(
    reference: &str,
    arch: &Arch,
    verification: Option<&Verification<'_>>,
    progress: &dyn Progress,
    mut handler: F,
) -> Result<()>
where
    F: FnMut(FileEntry<'_>) -> Result<()>,
{
    let client = registry::connect(reference, Access::Pull).await?;
    let cache = Store::new();
    progress.pulling(reference, arch.as_str());
    let layers = resolve::layers(&client, &cache, arch, verification).await?;
    progress.resolved(layers.len());

    walk(
        &client,
        &cache,
        &Content::new(),
        &layers,
        progress,
        |_layer_idx, entry, info| scan::handle_file_entry(entry, info, &mut handler),
    )
    .await
}

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
                && !excluded(&path, exclude)
            {
                sizes.insert(path.to_string_lossy().to_string(), size);
            }

            Ok(())
        },
    )
    .await?;

    Ok(sizes)
}

/// Download all layers, then iterate every archive entry not blocked by a whiteout.
async fn walk<F>(
    client: &Client,
    cache: &Store,
    content: &Content,
    layers: &[Descriptor],
    progress: &dyn Progress,
    mut on_entry: F,
) -> Result<()>
where
    F: for<'a> FnMut(
        usize,
        tar::Entry<&'a mut download::LayerReader>,
        scan::EntryInfo,
    ) -> Result<()>,
{
    let (bytes, whiteouts) = fetch::download_all(client, cache, content, layers, progress).await?;
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
    F: FnMut(usize, tar::Entry<&mut download::LayerReader>, scan::EntryInfo) -> Result<()>,
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
