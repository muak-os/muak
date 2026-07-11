//! OCI layer processing and tar path utilities.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use tar::Archive;

use super::{cache, download, resolve, scan};
use crate::arch::Arch;
use crate::error::{KociError, Result};
use crate::image::ImageReference;
use crate::pull::download::LayerReader;
use crate::registry::auth::fetch_auth_token;
use crate::registry::http::build_client;

/// Process all layers in an image, calling `on_entry` for each file entry.
pub(crate) async fn process<F>(
    reference: &str,
    arch: &Arch,
    pubkey_pem: Option<&str>,
    mut on_entry: F,
) -> Result<()>
where
    F: FnMut(
        usize,
        tar::Entry<&mut LayerReader>,
        scan::EntryInfo,
        &HashMap<PathBuf, usize>,
    ) -> Result<()>,
{
    let cache = cache::Store::new();
    let image_ref = ImageReference::parse(reference);
    let client = build_client();
    let token = fetch_auth_token(&client, &image_ref.registry, &image_ref.name).await?;

    let layers = resolve::layers(
        &cache,
        &client,
        &image_ref,
        token.as_deref(),
        arch,
        pubkey_pem,
    )
    .await?;

    let mut whiteout_layers: HashMap<PathBuf, usize> = HashMap::new();
    for (layer_idx, layer) in layers.iter().enumerate() {
        let file =
            download::cached(&cache, &client, &image_ref, &layer.digest, token.as_deref()).await?;
        let reader = download::decompress(file, layer.media_type.as_deref())?;
        let whiteouts = scan::scan_whiteouts(reader)?;
        for w in whiteouts {
            whiteout_layers.entry(w).or_insert(layer_idx);
        }
    }

    for (layer_idx, layer) in layers.iter().enumerate() {
        let file =
            download::cached(&cache, &client, &image_ref, &layer.digest, token.as_deref()).await?;
        let mut reader = download::decompress(file, layer.media_type.as_deref())?;

        let mut archive = Archive::new(&mut reader);
        let entries = archive.entries()?;
        for entry_result in entries {
            let entry = entry_result?;
            let info = scan::classify_tar_entry(&entry)?;
            on_entry(layer_idx, entry, info, &whiteout_layers)?;
        }
    }

    Ok(())
}

/// Normalize a tar entry path, rejecting parent traversal and skipping `.` / root entries.
pub(crate) fn normalize_entry_path(path: &Path) -> Result<Option<PathBuf>> {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir | Component::RootDir => {}
            Component::ParentDir => {
                return Err(KociError::LayerExtractionError(format!(
                    "OCI layer entry escapes extraction root: {}",
                    path.display()
                )));
            }
            Component::Prefix(prefix) => {
                #[cfg(windows)]
                {
                    let _ = prefix;
                    return Err(KociError::LayerExtractionError(format!(
                        "OCI layer entry uses unsupported path prefix: {}",
                        path.display()
                    )));
                }

                #[cfg(not(windows))]
                normalized.push(prefix.as_os_str());
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        Ok(None)
    } else {
        Ok(Some(normalized))
    }
}

/// If `path` is a whiteout entry, return the target path that should be removed.
pub(crate) fn whiteout_target(path: &Path) -> Option<PathBuf> {
    let file_name = path.file_name().and_then(|name| name.to_str())?;

    if file_name == ".wh..wh..opq" {
        return Some(path.parent().unwrap_or_else(|| Path::new("")).to_path_buf());
    }

    let stripped = file_name.strip_prefix(".wh.")?;
    let parent = path.parent().unwrap_or_else(|| Path::new(""));

    Some(parent.join(stripped))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_entry_path_returns_none_for_current_directory() {
        // ARRANGE
        let path = Path::new("./");

        // ACT
        let normalized = normalize_entry_path(path).expect("normalize path");

        // ASSERT
        assert!(normalized.is_none());
    }

    #[test]
    fn normalize_entry_path_rejects_parent_traversal() {
        // ACT
        let error =
            normalize_entry_path(Path::new("../escape")).expect_err("normalize should fail");

        // ASSERT
        assert!(matches!(error, KociError::LayerExtractionError(_)));
    }

    #[test]
    fn whiteout_target_returns_none_for_non_whiteout_path() {
        // ACT
        let target = whiteout_target(Path::new("etc/file"));

        // ASSERT
        assert!(target.is_none());
    }

    #[test]
    fn whiteout_target_returns_file_target() {
        // ACT
        let target = whiteout_target(Path::new("etc/.wh.obsolete"));

        // ASSERT
        assert_eq!(target, Some(PathBuf::from("etc/obsolete")));
    }

    #[test]
    fn whiteout_target_returns_opaque_directory_target() {
        // ACT
        let target = whiteout_target(Path::new("etc/.wh..wh..opq"));

        // ASSERT
        assert_eq!(target, Some(PathBuf::from("etc")));
    }
}
