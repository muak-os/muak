//! FAT32 EFI System Partition image builder.

use std::io::{Cursor, Write as _};

use fatfs::{FatType, FileSystem, FormatVolumeOptions, FsOptions};

use crate::MisoError;

/// Minimum FAT32 image padding beyond the content, in bytes.
const FAT_OVERHEAD_MIN_BYTES: usize = 1024 * 1024;

/// FAT32 metadata overhead as a fraction of the content size (5%).
const FAT_OVERHEAD_FRACTION: usize = 20;

/// Rounds `n` up to the nearest multiple of `align`.
fn align_up(n: usize, align: usize) -> usize {
    (n + align - 1) & !(align - 1)
}

/// Builds an in-memory FAT32 image containing the UKI at `/EFI/BOOT/{boot_filename}`.
pub fn build_efi_image(uki: &[u8], boot_filename: &str) -> Result<Vec<u8>, MisoError> {
    build_efi_image_with_blobs(uki, boot_filename, &[])
}

/// Builds an in-memory FAT32 image with the UKI and additional files in the FAT root.
pub fn build_efi_image_with_blobs(
    uki: &[u8],
    boot_filename: &str,
    blobs: &[(&str, &[u8])],
) -> Result<Vec<u8>, MisoError> {
    let total_content: usize = uki.len() + blobs.iter().map(|(_, d)| d.len()).sum::<usize>();
    let overhead = (total_content / FAT_OVERHEAD_FRACTION).max(FAT_OVERHEAD_MIN_BYTES);
    let image_size = align_up(total_content + overhead, 512);
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

        for &(name, data) in blobs {
            let mut blob_file = root
                .create_file(name)
                .map_err(|e| MisoError::Fat(e.to_string()))?;
            blob_file.write_all(data).map_err(MisoError::Io)?;
        }
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
            .expect("EFI dir must exist")
            .open_dir("BOOT")
            .expect("BOOT dir must exist")
            .open_file(filename)
            .expect("BOOTAA64.EFI must exist");
        let mut content = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut content).expect("read");
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
    fn build_efi_image_with_blobs_places_extra_files() {
        // ARRANGE
        let uki = b"uki-payload";
        let blob_a = b"firmware-blob-a";
        let blob_b = b"firmware-blob-b";
        let blobs: &[(&str, &[u8])] = &[("START4.ELF", blob_a), ("FIXUP4.DAT", blob_b)];

        // ACT
        let image = build_efi_image_with_blobs(uki, "BOOTAA64.EFI", blobs).expect("should succeed");

        // ASSERT
        let mut cursor = Cursor::new(image);
        let fs = FileSystem::new(&mut cursor, FsOptions::new()).expect("valid FAT");
        let root = fs.root_dir();

        let mut uki_file = root
            .open_dir("EFI")
            .expect("EFI")
            .open_dir("BOOT")
            .expect("BOOT")
            .open_file("BOOTAA64.EFI")
            .expect("UKI must exist");
        let mut uki_content = Vec::new();
        std::io::Read::read_to_end(&mut uki_file, &mut uki_content).expect("read uki");
        assert_eq!(uki_content, uki);

        for &(name, expected) in blobs {
            let mut f = root
                .open_file(name)
                .unwrap_or_else(|_| panic!("{name} must exist"));
            let mut content = Vec::new();
            std::io::Read::read_to_end(&mut f, &mut content).expect("read blob");
            assert_eq!(content, expected, "{name} content mismatch");
        }
    }

    #[test]
    fn build_efi_image_with_blobs_empty_blobs_same_as_without() {
        // ARRANGE
        let uki = b"uki";

        // ACT
        let with = build_efi_image_with_blobs(uki, "BOOTX64.EFI", &[]).expect("with blobs");
        let without = build_efi_image(uki, "BOOTX64.EFI").expect("without blobs");

        // ASSERT
        assert_eq!(with.len(), without.len());
    }

    #[test]
    fn build_efi_image_with_blobs_size_accounts_for_blob_data() {
        // ARRANGE
        let uki = vec![0u8; 100];
        let blob = vec![0xFFu8; 512 * 1024];
        let blobs: &[(&str, &[u8])] = &[("LARGE.BIN", &blob)];

        // ACT
        let image = build_efi_image_with_blobs(&uki, "BOOTX64.EFI", blobs).expect("should succeed");

        // ASSERT
        assert!(
            image.len() >= uki.len() + blob.len() + FAT_OVERHEAD_MIN_BYTES,
            "image must account for blob sizes"
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
