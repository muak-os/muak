//! Scanning and processing OCI layer tar archives.

use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;

use super::entries::{FileEntry, MetadataEntry};
use crate::error::{KociError, Result};
use crate::pull::layer::{normalize_entry_path, whiteout_target};

/// Outcome of classifying a tar entry.
pub(crate) enum EntryInfo {
    Skip,
    Whiteout(PathBuf),
    File(PathBuf, u64, u32),
}

/// Classify a tar entry and validate it.
pub(crate) fn classify_tar_entry(entry: &tar::Entry<impl Read>) -> Result<EntryInfo> {
    let header = entry.header();
    let entry_type = header.entry_type();
    let relative_path = normalize_entry_path(entry.path()?.as_ref())?;

    let Some(ref relative_path) = relative_path else {
        return Ok(EntryInfo::Skip);
    };

    if let Some(whiteout) = whiteout_target(relative_path) {
        return Ok(EntryInfo::Whiteout(whiteout));
    }

    if entry_type.is_dir() {
        return Ok(EntryInfo::Skip);
    }

    if entry_type.is_symlink() {
        return Err(KociError::LayerExtractionError(format!(
            "Unsupported symlink entry in OCI layer: {}",
            relative_path.display()
        )));
    }

    if entry_type.is_hard_link() {
        return Err(KociError::LayerExtractionError(format!(
            "Unsupported hard link entry in OCI layer: {}",
            relative_path.display()
        )));
    }

    if !entry_type.is_file() {
        return Err(KociError::LayerExtractionError(format!(
            "Unsupported OCI layer entry type for {}",
            relative_path.display()
        )));
    }

    Ok(EntryInfo::File(
        relative_path.clone(),
        header.size().unwrap_or(0),
        header.mode().unwrap_or(0o644),
    ))
}

/// Scan a single layer and collect its whiteout targets.
pub(crate) fn scan_whiteouts(data: &[u8]) -> Result<Vec<PathBuf>> {
    let mut whiteouts: Vec<PathBuf> = Vec::new();

    let mut archive = tar::Archive::new(data);
    let entries = archive.entries()?;
    for entry_result in entries {
        let entry = entry_result?;
        let info = classify_tar_entry(&entry)?;
        if let EntryInfo::Whiteout(path) = info {
            whiteouts.push(path);
        }
    }

    Ok(whiteouts)
}

/// Process a single tar entry for metadata extraction.
pub(crate) fn handle_metadata_entry(
    info: EntryInfo,
    layer_idx: usize,
    whiteout_layers: &HashMap<PathBuf, usize>,
    handler: &mut impl FnMut(MetadataEntry) -> Result<()>,
) -> Result<()> {
    if let EntryInfo::File(path, size, mode) = info {
        let blocked = whiteout_layers.get(&path).is_some_and(|&wl| wl > layer_idx);
        if !blocked {
            handler(MetadataEntry {
                path: path.to_string_lossy().to_string(),
                size,
                mode,
            })?;
        }
    }

    Ok(())
}

/// Process a single tar entry for file streaming.
pub(crate) fn handle_file_entry<R: Read>(
    mut entry: tar::Entry<R>,
    info: EntryInfo,
    layer_idx: usize,
    whiteout_layers: &HashMap<PathBuf, usize>,
    handler: &mut impl FnMut(FileEntry) -> Result<()>,
) -> Result<()> {
    if let EntryInfo::File(path, size, mode) = info {
        let blocked = whiteout_layers.get(&path).is_some_and(|&wl| wl > layer_idx);
        if !blocked {
            handler(FileEntry {
                path: path.to_string_lossy().to_string(),
                size,
                mode,
                reader: &mut entry,
            })?;
        }
    }

    Ok(())
}
