//! FAT32 EFI System Partition image builder.

use std::io::{Cursor, Write as _};

use fatfs::{FatType, FileSystem, FormatVolumeOptions, FsOptions};

use crate::MisoError;

/// Minimum FAT32 image padding beyond the UKI content, in bytes.
const FAT_OVERHEAD_MIN_BYTES: usize = 1024 * 1024;

/// FAT32 metadata overhead as a fraction of the UKI size (5%).
const FAT_OVERHEAD_FRACTION: usize = 20;

/// Rounds `n` up to the nearest multiple of `align`.
fn align_up(n: usize, align: usize) -> usize {
    (n + align - 1) & !(align - 1)
}

/// Builds an in-memory FAT32 image containing the UKI at `/EFI/BOOT/{boot_filename}`.
pub fn build_efi_image(uki: &[u8], boot_filename: &str) -> Result<Vec<u8>, MisoError> {
    let overhead = (uki.len() / FAT_OVERHEAD_FRACTION).max(FAT_OVERHEAD_MIN_BYTES);
    let image_size = align_up(uki.len() + overhead, 512);
    let buf = vec![0u8; image_size];
    let mut cursor = Cursor::new(buf);

    fatfs::format_volume(
        &mut cursor,
        FormatVolumeOptions::new()
            .volume_label(*b"EFI        ")
            .fat_type(FatType::Fat32),
    )
    .map_err(|e| MisoError::Fat(e.to_string()))?;

    {
        let fs = FileSystem::new(&mut cursor, FsOptions::new())
            .map_err(|e| MisoError::Fat(e.to_string()))?;

        let root = fs.root_dir();
        let mut file = root
            .create_dir("EFI")
            .map_err(|e| MisoError::Fat(e.to_string()))?
            .create_dir("BOOT")
            .map_err(|e| MisoError::Fat(e.to_string()))?
            .create_file(boot_filename)
            .map_err(|e| MisoError::Fat(e.to_string()))?;
        file.write_all(uki).map_err(MisoError::Io)?;
    }

    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_efi_image_returns_fat32_volume() {
        // ARRANGE
        let uki = b"fake-uki-payload";
        let filename = "BOOTx64.EFI";

        // ACT
        let image = build_efi_image(uki, filename).expect("should build FAT32 image");

        // ASSERT
        assert!(!image.is_empty(), "image must not be empty");
        let mut cursor = Cursor::new(image);
        let fs = FileSystem::new(&mut cursor, FsOptions::new())
            .expect("image should be a valid FAT filesystem");
        let root = fs.root_dir();
        let efi = root.open_dir("EFI").expect("EFI directory must exist");
        let boot = efi.open_dir("BOOT").expect("BOOT directory must exist");
        let mut file = boot.open_file(filename).expect("UKI file must exist");
        let mut content = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut content).expect("should read UKI file");
        assert_eq!(content, uki);
    }

    #[test]
    fn build_efi_image_size_is_sector_aligned() {
        // ARRANGE
        let uki = vec![0xABu8; 512];

        // ACT
        let image = build_efi_image(&uki, "BOOTx64.EFI").expect("should succeed");

        // ASSERT
        assert_eq!(image.len() % 512, 0, "image size must be a multiple of 512");
    }

    #[test]
    fn build_efi_image_aarch64_boot_filename() {
        // ARRANGE
        let uki = b"arm-uki";
        let filename = "BOOTAA64.EFI";

        // ACT
        let image = build_efi_image(uki, filename).expect("should build FAT32 image");

        // ASSERT
        let mut cursor = Cursor::new(image);
        let fs = FileSystem::new(&mut cursor, FsOptions::new()).expect("valid FAT");
        let mut file = fs
            .root_dir()
            .open_dir("EFI")
            .unwrap()
            .open_dir("BOOT")
            .unwrap()
            .open_file(filename)
            .expect("BOOTAA64.EFI must exist");
        let mut content = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut content).unwrap();
        assert_eq!(content, uki);
    }

    #[test]
    fn build_efi_image_minimum_size_exceeds_uki() {
        // ARRANGE
        let uki = vec![0u8; 100];

        // ACT
        let image = build_efi_image(&uki, "BOOTx64.EFI").expect("should succeed");

        // ASSERT
        assert!(
            image.len() >= uki.len() + FAT_OVERHEAD_MIN_BYTES,
            "image must be at least UKI size + overhead"
        );
    }

    #[test]
    fn align_up_already_aligned() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(align_up(512, 512), 512);
        assert_eq!(align_up(1024, 512), 1024);
    }

    #[test]
    fn align_up_rounds_up() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(align_up(513, 512), 1024);
        assert_eq!(align_up(1, 512), 512);
    }
}
