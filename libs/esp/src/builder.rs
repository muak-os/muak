//! Builder for ESP partitions.

use std::io::{Read, Write};

use fatfs::builder::{build, precompute};
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
pub fn compute_layout<'a>(files: &[FileMeta<'a>]) -> Result<Layout<'a>> {
    path::validate_spec_paths(files.iter().map(|file| file.path))?;

    let total_data: u64 = files.iter().map(|file| file.size).sum();
    let total_size = image_size_for(total_data)?;

    let files = files.to_vec();

    Ok(Layout { files, total_size })
}

/// Builder for ESP images with streaming file data.
pub struct Builder<'data, 'ctx, W: Write> {
    layout: &'ctx Layout<'data>,
    writer: &'ctx mut W,
    current_index: usize,
    readers: Vec<&'data mut (dyn Read + 'data)>,
}

impl<'data, 'ctx, W: Write> Builder<'data, 'ctx, W> {
    /// Creates a new ESP builder with the given layout and writer.
    #[must_use]
    pub fn new(layout: &'ctx Layout<'data>, writer: &'ctx mut W) -> Self {
        Self {
            layout,
            writer,
            current_index: 0,
            readers: Vec::new(),
        }
    }

    /// Adds a file to the ESP, validating path and size against the layout.
    ///
    /// Files must be added in the same order as specified in the layout.
    ///
    /// # Errors
    ///
    /// Returns an error when:
    /// - The path doesn't match the expected path in the layout
    /// - The size doesn't match the expected size in the layout
    /// - All files have already been added
    pub fn add_file(
        &mut self,
        path: &str,
        data: &'data mut (dyn Read + 'data),
        size: u64,
    ) -> Result<()> {
        let expected = self.layout.files.get(self.current_index).ok_or_else(|| {
            EspError::InvalidOrder(format!(
                "all {} files already added, cannot add more",
                self.layout.files.len()
            ))
        })?;

        if path != expected.path {
            return Err(EspError::InvalidOrder(format!(
                "expected path '{}', got '{path}'",
                expected.path
            )));
        }

        if size != expected.size {
            return Err(EspError::SizeMismatch {
                path: path.to_owned(),
                expected: expected.size,
                actual: size,
            });
        }

        self.readers.push(data);
        self.current_index = self.current_index.saturating_add(1);

        Ok(())
    }

    /// Finalizes the ESP image by writing the FAT filesystem.
    ///
    /// # Errors
    ///
    /// Returns an error when:
    /// - Not all files from the layout have been added
    /// - Writing the FAT image fails
    pub fn finish(self) -> Result<()> {
        if self.current_index != self.layout.files.len() {
            return Err(EspError::Incomplete {
                expected: self.layout.files.len(),
                actual: self.current_index,
            });
        }

        let precomputed = precompute(&self.layout.files, self.layout.total_size)?;

        let mut readers: Vec<&mut (dyn Read + 'data)> = self.readers;
        build(&precomputed, &mut readers, self.writer)?;

        Ok(())
    }
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
    use std::io::Cursor;

    use super::*;

    #[test]
    fn compute_layout_validates_paths() {
        // ARRANGE / ACT
        let result = compute_layout(&[FileMeta::new("../escape", 100)]);

        // ASSERT
        assert!(matches!(result, Err(EspError::InvalidPath(_))));
    }

    #[test]
    fn compute_layout_returns_minimum_size_for_empty() {
        // ARRANGE / ACT
        let layout = compute_layout(&[]).expect("layout must compute");

        // ASSERT
        assert!(layout.total_size >= MIN_IMAGE_BYTES);
        assert_eq!(layout.total_size.rem_euclid(SECTOR_SIZE), 0);
    }

    #[test]
    fn compute_layout_grows_with_data() {
        // ARRANGE / ACT
        let small = compute_layout(&[FileMeta::new("a.txt", 4096)]).expect("small must compute");
        let large =
            compute_layout(&[FileMeta::new("a.txt", 4 * 1024 * 1024)]).expect("large must compute");

        // ASSERT
        assert!(large.total_size > small.total_size);
    }

    #[test]
    fn builder_adds_files_in_order() {
        // ARRANGE
        let layout = compute_layout(&[
            FileMeta::new("EFI/BOOT/BOOTX64.EFI", 3),
            FileMeta::new("config.txt", 6),
        ])
        .expect("layout must compute");
        let mut output = Vec::new();
        let mut builder = Builder::new(&layout, &mut output);
        let mut reader1 = Cursor::new(b"uki".as_slice());
        let mut reader2 = Cursor::new(b"config".as_slice());

        // ACT
        builder
            .add_file("EFI/BOOT/BOOTX64.EFI", &mut reader1, 3)
            .expect("first file must be added");
        builder
            .add_file("config.txt", &mut reader2, 6)
            .expect("second file must be added");
        builder.finish().expect("build must succeed");

        // ASSERT
        assert!(!output.is_empty());
        assert_eq!(
            output.get(510..512),
            Some(&[0x55, 0xAA][..]),
            "boot signature must be valid"
        );
    }

    #[test]
    fn builder_rejects_wrong_path_order() {
        // ARRANGE
        let layout = compute_layout(&[
            FileMeta::new("EFI/BOOT/BOOTX64.EFI", 3),
            FileMeta::new("config.txt", 6),
        ])
        .expect("layout must compute");
        let mut output = Vec::new();
        let mut builder = Builder::new(&layout, &mut output);
        let mut reader = Cursor::new(b"config".as_slice());

        // ACT
        let result = builder.add_file("config.txt", &mut reader, 6);

        // ASSERT
        assert!(matches!(result, Err(EspError::InvalidOrder(_))));
    }

    #[test]
    fn builder_rejects_size_mismatch() {
        // ARRANGE
        let layout =
            compute_layout(&[FileMeta::new("config.txt", 6)]).expect("layout must compute");
        let mut output = Vec::new();
        let mut builder = Builder::new(&layout, &mut output);
        let mut reader = Cursor::new(b"config".as_slice());

        // ACT
        let result = builder.add_file("config.txt", &mut reader, 10);

        // ASSERT
        assert!(matches!(result, Err(EspError::SizeMismatch { .. })));
    }

    #[test]
    fn builder_rejects_incomplete_files() {
        // ARRANGE
        let layout = compute_layout(&[
            FileMeta::new("EFI/BOOT/BOOTX64.EFI", 3),
            FileMeta::new("config.txt", 6),
        ])
        .expect("layout must compute");
        let mut output = Vec::new();
        let mut builder = Builder::new(&layout, &mut output);
        let mut reader = Cursor::new(b"uki".as_slice());
        builder
            .add_file("EFI/BOOT/BOOTX64.EFI", &mut reader, 3)
            .expect("first file must be added");

        // ACT
        let result = builder.finish();

        // ASSERT
        assert!(matches!(result, Err(EspError::Incomplete { .. })));
    }
}
