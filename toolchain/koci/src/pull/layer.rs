//! OCI layer downloading, decompression, and tar entry iteration.

use std::collections::HashMap;
use std::path::PathBuf;

use oci::model::Descriptor;
use oci_client::client::Client;
use tar::Archive;

use super::cache::Store;
use super::content::Content;
use super::{download, fetch, parallel, scan};
use crate::error::Result;
use crate::progress::{Event, Phase, Reporter, Unit};

/// Download all layers, then iterate every archive entry not blocked by a whiteout.
pub(crate) async fn walk<F>(
    client: &Client,
    cache: &Store,
    content: &Content,
    layers: &[Descriptor],
    reporter: &dyn Reporter,
    mut on_entry: F,
) -> Result<()>
where
    F: for<'a, 'b> FnMut(
        usize,
        tar::Entry<&'a mut download::LayerReader>,
        scan::EntryInfo<'b>,
    ) -> Result<()>,
{
    let bytes = parallel::download_all(client, cache, content, layers, reporter).await?;
    let n = layers.len();
    if n > 1 {
        let whiteouts = fetch::whiteout_map(content, layers, &bytes, reporter)?;
        reporter.report(Event::PhaseCompleted {
            phase: Phase::Scan,
            label: &format!("Scanned {n}/{n} layers"),
        });
        extract_all(content, layers, &bytes, &whiteouts, reporter, &mut on_entry)?;
    } else {
        let whiteouts = HashMap::new();
        extract_all(content, layers, &bytes, &whiteouts, reporter, &mut on_entry)?;
    }

    if n > 0 {
        reporter.report(Event::PhaseCompleted {
            phase: Phase::Extract,
            label: &format!("Extracted {n}/{n} layers"),
        });
    }

    Ok(())
}

fn extract_all<F>(
    content: &Content,
    layers: &[Descriptor],
    bytes: &[Option<Vec<u8>>],
    whiteouts: &HashMap<PathBuf, usize>,
    reporter: &dyn Reporter,
    on_entry: &mut F,
) -> Result<()>
where
    F: for<'a, 'b> FnMut(
        usize,
        tar::Entry<&'a mut download::LayerReader>,
        scan::EntryInfo<'b>,
    ) -> Result<()>,
{
    let n = layers.len();
    let total = u64::try_from(n).unwrap_or(u64::MAX);
    reporter.report(Event::PhaseStarted {
        phase: Phase::Extract,
        label: "Extracting layers",
        total,
        unit: Unit::Items,
    });
    for (layer_idx, layer) in layers.iter().enumerate() {
        let current = u64::try_from(layer_idx.saturating_add(1)).unwrap_or(u64::MAX);
        reporter.report(Event::Progress {
            phase: Phase::Extract,
            current,
            total,
        });
        let cached = fetch::cached_bytes(bytes, layer_idx);
        let mut reader = fetch::layer_reader(content, cached.as_deref(), layer)?;
        extract_layer(&mut reader, layer_idx, whiteouts, on_entry)?;
    }

    Ok(())
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
