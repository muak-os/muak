//! Layout computation for ESP partitions.

use fatfs::error::FatError;

use crate::FileMeta;
use crate::error::{EspError, Result};
use crate::path;

const CLUSTER_SIZE: u64 = 4096;
const SECTOR_SIZE: u64 = 512;
const RESERVED_SECTORS: u64 = 32;
const MIN_IMAGE_BYTES: u64 = 1_u64 << 20;
const MAX_IMAGE_BYTES: u64 = 1_u64 << 29;
const DIRECTORY_OVERHEAD: u64 = 1_u64 << 20;

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
/// Returns an error when a path is invalid or the total data exceeds the supported image size.
pub fn compute<'a>(files: &[FileMeta<'a>]) -> Result<Layout<'a>> {
    path::validate_spec(files.iter().map(|file| file.path))?;

    let total_data: u64 = files.iter().map(|file| file.size).sum();
    let total_size = image_size_for(total_data)?;

    let files = files.to_vec();

    Ok(Layout { files, total_size })
}

fn image_size_for(total_data: u64) -> Result<u64> {
    let padded = total_data.saturating_add(DIRECTORY_OVERHEAD);
    let data_region = next_multiple_of(padded, CLUSTER_SIZE).max(MIN_IMAGE_BYTES);
    if data_region > MAX_IMAGE_BYTES {
        return Err(EspError::Fat(FatError::Fat(format!(
            "ESP image size {data_region} exceeds {MAX_IMAGE_BYTES} byte limit"
        ))));
    }
    let reserved = RESERVED_SECTORS.saturating_mul(SECTOR_SIZE);
    let fat_bytes = fat_bytes_for(data_region);
    let fats_total = fat_bytes.saturating_mul(2);
    let total = reserved
        .saturating_add(fats_total)
        .saturating_add(data_region);

    Ok(next_multiple_of(total, SECTOR_SIZE))
}

fn fat_bytes_for(image_size: u64) -> u64 {
    let clusters = image_size.saturating_div(CLUSTER_SIZE);
    let entries = clusters.saturating_mul(4);

    next_multiple_of(entries, SECTOR_SIZE)
}

fn next_multiple_of(value: u64, step: u64) -> u64 {
    let quotient = value.div_ceil(step);

    quotient.saturating_mul(step)
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};

    use super::*;
    use crate::image;

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
        assert!(layout.total_size >= MIN_IMAGE_BYTES);
        assert_eq!(layout.total_size.rem_euclid(SECTOR_SIZE), 0);
    }

    #[test]
    fn compute_layout_grows_with_data() {
        // ARRANGE / ACT
        let small = compute(&[FileMeta::new("a.txt", 4096)]).expect("small must compute");
        let large =
            compute(&[FileMeta::new("a.txt", 4 * 1024 * 1024)]).expect("large must compute");

        // ASSERT
        assert!(large.total_size > small.total_size);
    }

    #[test]
    fn build_produces_bootable_image() {
        // ARRANGE
        let layout = compute(&[
            FileMeta::new("EFI/BOOT/BOOTX64.EFI", 3),
            FileMeta::new("config.txt", 6),
        ])
        .expect("layout must compute");
        let mut output = Vec::new();
        let mut uki_reader = Cursor::new(b"uki".as_slice());
        let mut cfg_reader = Cursor::new(b"config".as_slice());
        let mut readers: Vec<&mut dyn Read> = vec![&mut uki_reader, &mut cfg_reader];

        // ACT
        image::build(&layout, &mut readers, &mut output).expect("build must succeed");

        // ASSERT
        assert!(!output.is_empty());
        assert_eq!(
            output.get(510..512),
            Some(&[0x55, 0xAA][..]),
            "boot signature must be valid"
        );
    }

    #[test]
    fn build_rejects_reader_count_mismatch() {
        // ARRANGE
        let layout = compute(&[
            FileMeta::new("EFI/BOOT/BOOTX64.EFI", 3),
            FileMeta::new("config.txt", 6),
        ])
        .expect("layout must compute");
        let mut output = Vec::new();
        let mut reader = Cursor::new(b"uki".as_slice());
        let mut readers: Vec<&mut dyn Read> = vec![&mut reader];

        // ACT
        let result = image::build(&layout, &mut readers, &mut output);

        // ASSERT
        assert!(matches!(result, Err(EspError::Incomplete { .. })));
    }

    #[test]
    fn build_rejects_empty_readers_for_nonempty_layout() {
        // ARRANGE
        let layout = compute(&[FileMeta::new("config.txt", 6)]).expect("layout must compute");
        let mut output = Vec::new();
        let mut readers: Vec<&mut dyn Read> = vec![];

        // ACT
        let result = image::build(&layout, &mut readers, &mut output);

        // ASSERT
        assert!(matches!(result, Err(EspError::Incomplete { .. })));
    }
}
