//! Overlay asset placement: where each overlay entry belongs on the boot device.

use std::collections::HashSet;

use crate::domain::resolution::Overlay;
use crate::error::{Result, WizardError};

/// Where an overlay asset is placed on the boot device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Asset {
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

impl Asset {
    /// Returns the canonical output stream name for this asset.
    #[must_use]
    pub(crate) fn name(&self) -> &str {
        match *self {
            Asset::EspFile { ref path, .. } => path,
            Asset::RawBlob { ref source, .. } => source,
        }
    }

    /// Returns the payload size of this asset in bytes.
    #[must_use]
    pub(crate) fn size(&self) -> u64 {
        match *self {
            Asset::EspFile { size, .. } | Asset::RawBlob { size, .. } => size,
        }
    }
}

/// Where an overlay entry sits, once its OCI path is parsed and its partition
/// GUID or blob offset validated.
#[derive(Debug)]
enum Placement {
    /// An ESP file at its stripped path.
    Esp { file: String },
    /// A raw blob at a fixed byte offset.
    Blob { offset: u64, file: String },
}

/// Classifies every `(path, size)` entry into an overlay asset, rejecting
/// stray or malformed entries. The result is sorted canonically by name.
///
/// # Errors
///
/// Returns an error when an entry references a non-EFI partition type, a blob
/// directory holds more than one file, or a path sits outside `partitions/`
/// and `blob/`.
pub(crate) fn classify(overlay: &Overlay, entries: Vec<(String, u64)>) -> Result<Vec<Asset>> {
    let mut found: Vec<Asset> = Vec::new();
    let mut blob_dirs: HashSet<u64> = HashSet::new();
    let mut failure: Option<WizardError> = None;

    for (path, size) in entries {
        if failure.is_none()
            && let Err(error) = classify_one(overlay, &path, size, &mut found, &mut blob_dirs)
        {
            failure = Some(error);
        }
    }

    if let Some(error) = failure {
        return Err(error);
    }
    found.sort_by(|left, right| left.name().cmp(right.name()));

    Ok(found)
}

/// Returns the canonical stream name for an OCI entry path, or `None` when the
/// entry is not an overlay asset (e.g. outside the overlay name prefix).
#[must_use]
pub(crate) fn entry_name(overlay: &Overlay, path: &str) -> Option<String> {
    match placement(overlay, path) {
        Ok(Some(Placement::Esp { file } | Placement::Blob { file, .. })) => Some(file),
        _ => None,
    }
}

fn classify_one(
    overlay: &Overlay,
    path: &str,
    size: u64,
    found: &mut Vec<Asset>,
    blob_dirs: &mut HashSet<u64>,
) -> Result<()> {
    let Some(placement) = placement(overlay, path)? else {
        return Ok(());
    };
    match placement {
        Placement::Esp { file } => found.push(Asset::EspFile { path: file, size }),
        Placement::Blob { offset, file } => {
            if !blob_dirs.insert(offset) {
                return Err(WizardError::BuildError(format!(
                    "blob directory {offset} holds more than one file"
                )));
            }
            found.push(Asset::RawBlob {
                source: file,
                size,
                offset,
            });
        }
    }

    Ok(())
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
        if !esp::guid::is_esp(type_guid) {
            return Err(WizardError::BuildError(format!(
                "unsupported partition type GUID {type_guid}"
            )));
        }
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

#[cfg(test)]
mod tests {
    use koci::arch::Arch;

    use super::*;

    fn overlay() -> Overlay {
        Overlay::new(
            "board".to_owned(),
            "board".to_owned(),
            "ghcr.io/example/board:latest".to_owned(),
            Arch::Arm64,
        )
    }

    fn asset(path: &str, size: u64) -> (String, u64) {
        (path.to_owned(), size)
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
    fn classify_rejects_stray_files() {
        // ARRANGE
        let ov = overlay();

        // ACT
        let result = classify(&ov, vec![asset("board/stray.txt", 10)]);

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn classify_rejects_multi_file_blob() {
        // ARRANGE
        let ov = overlay();

        // ACT
        let result = classify(
            &ov,
            vec![
                asset("board/blob/8192/u-boot-sunxi-with-spl.bin", 10),
                asset("board/blob/8192/second.bin", 10),
            ],
        );

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn classify_sorts_and_tags_uniform_tree() {
        // ARRANGE
        let ov = overlay();

        // ACT
        let result = classify(
            &ov,
            vec![
                asset("board/blob/32768/u-boot.bin", 20),
                asset(
                    "board/partitions/C12A7328-F81F-11D2-BA4B-00A0C93EC93B/config.txt",
                    10,
                ),
            ],
        )
        .expect("classify must resolve");

        // ASSERT
        assert_eq!(result.len(), 2);
        assert!(matches!(
            result.first(),
            Some(Asset::EspFile { path, .. }) if path == "config.txt"
        ));
        assert!(matches!(
            result.get(1),
            Some(Asset::RawBlob { source, offset, .. }) if source == "u-boot.bin" && *offset == 32768
        ));
    }
}
