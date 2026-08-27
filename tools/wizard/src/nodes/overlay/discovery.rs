//! Overlay asset placement classification and discovery.

use std::collections::HashSet;

use koci::pull;

use super::guid;
use crate::domain::resolution::Overlay;
use crate::error::{Result, WizardError};

/// Where an overlay asset is placed on the boot device.
pub enum OverlayAsset {
    /// A file written into the EFI System Partition FAT image.
    EspFile {
        /// Stripped path of the file inside the ESP (e.g. `EFI/BOOT/BOOTAA64.EFI`).
        path: String,
        /// Size of the file payload in bytes.
        size: u64,
    },
    /// A raw blob written at a fixed byte offset before the partition table.
    RawBlob {
        /// File name of the blob as stored under `blob/<offset>/`.
        source: String,
        /// Size of the blob payload in bytes.
        size: u64,
        /// Byte offset on the boot device where the blob is written.
        offset: u64,
    },
}

impl OverlayAsset {
    /// Returns the canonical output stream name for this asset.
    pub(crate) fn name(&self) -> &str {
        match *self {
            OverlayAsset::EspFile { ref path, .. } => path,
            OverlayAsset::RawBlob { ref source, .. } => source,
        }
    }

    /// Returns the payload size of this asset in bytes.
    pub(crate) fn size(&self) -> u64 {
        match *self {
            OverlayAsset::EspFile { size, .. } | OverlayAsset::RawBlob { size, .. } => size,
        }
    }
}

/// Where an overlay entry is placed, once its OCI path is parsed and its
/// partition GUID or blob offset validated.
#[derive(Debug)]
enum Placement {
    /// An ESP file at its stripped path.
    Esp { file: String },
    /// A raw blob at a fixed byte offset.
    Blob { offset: u64, file: String },
}

/// Discovers every overlay asset from the OCI metadata listing, classifying
/// each into an ESP file or a raw blob and rejecting stray or malformed entries.
///
/// # Errors
///
/// Returns an error when the OCI metadata listing fails, an entry references a
/// non-EFI partition type, or a blob directory contains multiple files.
pub fn assets(overlay: &Overlay) -> Result<Vec<OverlayAsset>> {
    let mut found: Vec<OverlayAsset> = Vec::new();
    let mut blob_dirs: HashSet<u64> = HashSet::new();
    let mut failure: Option<WizardError> = None;

    pull::metadata(&overlay.source, &overlay.arch, None, |entry| {
        if failure.is_none()
            && let Err(error) = classify_entry(overlay, &entry, &mut found, &mut blob_dirs)
        {
            failure = Some(error);
        }
        Ok(())
    })
    .map_err(|e| WizardError::BuildError(format!("list overlay files: {e}")))?;

    if let Some(error) = failure {
        return Err(error);
    }
    found.sort_by(|left, right| left.name().cmp(right.name()));

    Ok(found)
}

/// Returns the canonical stream name for an OCI entry, or `None` when the entry
/// is not an overlay asset (e.g. outside the overlay name prefix).
pub(crate) fn entry_name(overlay: &Overlay, path: &str) -> Option<String> {
    match placement(overlay, path) {
        Ok(Some(Placement::Esp { file } | Placement::Blob { file, .. })) => Some(file),
        _ => None,
    }
}

/// Parses an OCI entry path into its placement, relative to the overlay name.
fn placement(overlay: &Overlay, path: &str) -> Result<Option<Placement>> {
    let prefix = format!("{}/", overlay.name);
    let Some(rel) = path.strip_prefix(&prefix) else {
        return Ok(None);
    };
    if rel.is_empty() {
        return Ok(None);
    }
    if let Some(rest) = rel.strip_prefix("partitions/") {
        let Some((type_guid, file)) = rest.split_once('/') else {
            return Err(WizardError::BuildError(format!(
                "partition entry missing file path: {path}"
            )));
        };
        guid::assert_esp(type_guid)?;
        return Ok(Some(Placement::Esp {
            file: file.to_owned(),
        }));
    }
    if let Some(rest) = rel.strip_prefix("blob/") {
        let Some((offset_str, file)) = rest.split_once('/') else {
            return Err(WizardError::BuildError(format!(
                "blob entry missing file path: {path}"
            )));
        };
        let offset = offset_str.parse::<u64>().map_err(|err| {
            WizardError::BuildError(format!("malformed blob offset: {offset_str} ({err})"))
        })?;
        return Ok(Some(Placement::Blob {
            offset,
            file: file.to_owned(),
        }));
    }

    Err(WizardError::BuildError(format!(
        "overlay asset outside partitions/ or blob/: {rel}"
    )))
}

fn classify_entry(
    overlay: &Overlay,
    entry: &pull::entries::MetadataEntry,
    found: &mut Vec<OverlayAsset>,
    blob_dirs: &mut HashSet<u64>,
) -> Result<()> {
    let Some(placement) = placement(overlay, &entry.path)? else {
        return Ok(());
    };
    match placement {
        Placement::Esp { file } => found.push(OverlayAsset::EspFile {
            path: file,
            size: entry.size,
        }),
        Placement::Blob { offset, file } => {
            if !blob_dirs.insert(offset) {
                return Err(WizardError::BuildError(format!(
                    "blob directory {offset} holds more than one file"
                )));
            }
            found.push(OverlayAsset::RawBlob {
                source: file,
                size: entry.size,
                offset,
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use koci::arch::Arch;
    use koci::pull::entries::MetadataEntry;

    use super::*;
    use crate::domain::resolution::Overlay;

    fn overlay() -> Overlay {
        Overlay::new(
            "board".to_owned(),
            "board".to_owned(),
            "ghcr.io/example/board:latest".to_owned(),
            Arch::Arm64,
        )
    }

    fn asset(path: &str, size: u64) -> MetadataEntry {
        MetadataEntry {
            path: path.to_owned(),
            size,
            mode: 0,
        }
    }

    fn assets_sim(ov: &Overlay, entries: Vec<MetadataEntry>) -> Result<Vec<OverlayAsset>> {
        let mut found: Vec<OverlayAsset> = Vec::new();
        let mut blob_dirs: HashSet<u64> = HashSet::new();
        for entry in entries {
            classify_entry(ov, &entry, &mut found, &mut blob_dirs)?;
        }
        found.sort_by(|left, right| left.name().cmp(right.name()));

        Ok(found)
    }

    #[test]
    fn parses_esp_file_under_partitions_guid() {
        // ARRANGE
        let ov = overlay();

        // ACT
        let placement = placement(
            &ov,
            "board/partitions/C12A7328-F81F-11D2-BA4B-00A0C93EC93B/EFI/BOOT/BOOTAA64.EFI",
        )
        .expect("parse must succeed");

        // ASSERT
        assert!(matches!(
            placement,
            Some(Placement::Esp { ref file }) if file == "EFI/BOOT/BOOTAA64.EFI"
        ));
    }

    #[test]
    fn parses_raw_blob_under_blob_offset() {
        // ARRANGE
        let ov = overlay();

        // ACT
        let placement = placement(&ov, "board/blob/32768/u-boot.bin").expect("parse must succeed");

        // ASSERT
        assert!(matches!(
            placement,
            Some(Placement::Blob { offset, ref file }) if offset == 32768 && file == "u-boot.bin"
        ));
    }

    #[test]
    fn rejects_asset_outside_partitions_and_blob() {
        // ARRANGE
        let ov = overlay();

        // ACT
        let result = placement(&ov, "board/random.txt");

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn rejects_non_esp_partition_guid() {
        // ARRANGE
        let ov = overlay();

        // ACT
        let result = placement(
            &ov,
            "board/partitions/11111111-1111-1111-1111-111111111111/boot.bin",
        );

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn rejects_malformed_blob_offset() {
        // ARRANGE
        let ov = overlay();

        // ACT
        let result = placement(&ov, "board/blob/notanumber/u-boot.bin");

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn assets_rejects_stray_files() {
        // ARRANGE
        let ov = overlay();

        // ACT
        let result = assets_sim(&ov, vec![asset("board/stray.txt", 10)]);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn assets_rejects_multi_file_blob() {
        // ARRANGE
        let ov = overlay();

        // ACT
        let result = assets_sim(
            &ov,
            vec![
                asset("board/blob/8192/u-boot-sunxi-with-spl.bin", 10),
                asset("board/blob/8192/second.bin", 10),
            ],
        );

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn assets_sorts_and_tags_uniform_tree() {
        // ARRANGE
        let ov = overlay();

        // ACT
        let result = assets_sim(
            &ov,
            vec![
                asset("board/blob/32768/u-boot.bin", 20),
                asset(
                    "board/partitions/C12A7328-F81F-11D2-BA4B-00A0C93EC93B/config.txt",
                    10,
                ),
            ],
        )
        .expect("assets must resolve");

        // ASSERT
        assert_eq!(result.len(), 2);
        assert!(matches!(
            result.first(),
            Some(OverlayAsset::EspFile { path, .. }) if path == "config.txt"
        ));
        assert!(matches!(
            result.get(1),
            Some(OverlayAsset::RawBlob { source, offset, .. }) if source == "u-boot.bin" && *offset == 32768
        ));
    }
}
