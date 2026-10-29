//! Layout computation for ESP partitions.

use fatfs::builder::image_size_for;

use crate::error::Result;
use crate::{FileMeta, path};

/// Precomputed layout for an ESP image.
#[derive(Clone, Debug)]
pub struct Layout<'a> {
    /// Files in the order they will be added to the ESP.
    pub files: Vec<FileMeta<'a>>,
    /// Total FAT image size in bytes.
    pub total_size: u64,
}

/// Computes the ESP layout from file metadata.
///
/// # Errors
///
/// Returns an error when a path is invalid or the payload does not fit in FAT32.
pub fn compute<'a>(files: &[FileMeta<'a>]) -> Result<Layout<'a>> {
    path::validate_spec(files.iter().map(|file| file.path))?;

    let total_data: u64 = files.iter().map(|file| file.size).sum();
    let total_size = image_size_for(total_data)?;
    let files = files.to_vec();

    Ok(Layout { files, total_size })
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};

    use super::*;
    use crate::error::EspError;
    use crate::image;

    const SECTOR_SIZE: u64 = 512;

    #[test]
    fn compute_layout_validates_paths() {
        // ARRANGE / ACT
        let result = compute(&[FileMeta::new("../escape", 100)]);

        // ASSERT
        assert!(matches!(result, Err(EspError::InvalidPath(_))));
    }

    #[test]
    fn compute_layout_returns_minimum_size_for_empty() {
        // ARRANGE / ACT
        let layout = compute(&[]).expect("layout must compute");

        // ASSERT
        assert_eq!(layout.total_size.rem_euclid(SECTOR_SIZE), 0);
        assert!(
            layout.total_size >= fatfs::types::MIN_IMAGE_SIZE,
            "ESP must be at least the smallest FAT32 image"
        );
    }

    #[test]
    fn compute_layout_keeps_free_space_for_the_boot_partition() {
        // ARRANGE
        let files = &[FileMeta::new("EFI/BOOT/BOOTAA64.EFI", 69 * 1024 * 1024)];

        // ACT
        let layout = compute(files).expect("layout must compute");

        // ASSERT
        assert!(
            layout.total_size >= 69 * 1024 * 1024 + fatfs::builder::FREE_SLACK,
            "the ESP must keep room for runtime writes, got {}",
            layout.total_size
        );
    }

    #[test]
    fn compute_layout_grows_with_data() {
        // ARRANGE / ACT
        let small =
            compute(&[FileMeta::new("a.txt", 36 * 1024 * 1024)]).expect("small must compute");
        let large =
            compute(&[FileMeta::new("a.txt", 72 * 1024 * 1024)]).expect("large must compute");

        // ASSERT
        assert!(large.total_size > small.total_size);
    }

    #[test]
    fn compute_rejects_payloads_larger_than_fat32() {
        // ARRANGE
        let files = &[FileMeta::new(
            "EFI/BOOT/BOOTX64.EFI",
            fatfs::types::MAX_IMAGE_SIZE,
        )];

        // ACT
        let result = compute(files);

        // ASSERT
        assert!(
            result.is_err(),
            "payloads pushing the ESP past the FAT32 ceiling must be rejected"
        );
    }

    #[test]
    fn build_produces_bootable_image() {
        // ARRANGE
        let data = vec![0xAB_u8; 33 * 1024 * 1024];
        let layout = compute(&[FileMeta::new(
            "EFI/BOOT/BOOTX64.EFI",
            u64::try_from(data.len()).unwrap_or(0),
        )])
        .expect("layout must compute");
        let mut output = Vec::new();
        let mut uki_reader = Cursor::new(data.as_slice());
        let mut readers: Vec<&mut dyn Read> = vec![&mut uki_reader];

        // ACT
        image::build(&layout, &mut readers, &mut output).expect("build must succeed");

        // ASSERT
        assert_ne!(output, b"");
        assert_eq!(
            output.get(510..512),
            Some(&[0x55, 0xAA][..]),
            "boot signature must be valid"
        );
    }

    #[test]
    fn build_rejects_reader_count_mismatch() {
        // ARRANGE
        let data = vec![0xAB_u8; 33 * 1024 * 1024];
        let layout = compute(&[
            FileMeta::new(
                "EFI/BOOT/BOOTX64.EFI",
                u64::try_from(data.len()).unwrap_or(0),
            ),
            FileMeta::new("config.txt", 0),
        ])
        .expect("layout must compute");
        let mut output = Vec::new();
        let mut reader = Cursor::new(data.as_slice());
        let mut readers: Vec<&mut dyn Read> = vec![&mut reader];

        // ACT
        let result = image::build(&layout, &mut readers, &mut output);

        // ASSERT
        assert!(matches!(result, Err(EspError::Incomplete { .. })));
    }

    #[test]
    fn build_rejects_empty_readers_for_nonempty_layout() {
        // ARRANGE
        let data = vec![0xAB_u8; 33 * 1024 * 1024];
        let layout = compute(&[FileMeta::new(
            "config.txt",
            u64::try_from(data.len()).unwrap_or(0),
        )])
        .expect("layout must compute");
        let mut output = Vec::new();
        let mut readers: Vec<&mut dyn Read> = vec![];

        // ACT
        let result = image::build(&layout, &mut readers, &mut output);

        // ASSERT
        assert!(matches!(result, Err(EspError::Incomplete { .. })));
    }
}
