//! FAT32 ESP image builder.

use std::io::Write;

use fatfs::builder;
use fatfs::error::FatError;

use crate::error::EspError;
use crate::model::EspFile;
use crate::path;

const CLUSTER_SIZE: u64 = 4096;
const SECTOR_SIZE: u64 = 512;
const RESERVED_SECTORS: u64 = 32;
pub(crate) const MIN_IMAGE_BYTES: u64 = 1_u64 << 20;
const MAX_IMAGE_BYTES: u64 = 1_u64 << 29;
const DIRECTORY_OVERHEAD: u64 = 1_u64 << 20;

/// Builds a FAT32 ESP image, writing to any `Write` sink.
///
/// # Errors
///
/// Returns an error when a path is invalid or writing fails.
pub fn build<W: Write>(files: &mut [EspFile<'_>], writer: &mut W) -> Result<(), EspError> {
    let metas: Vec<(&str, u64)> = files
        .iter()
        .map(|file| (file.path.as_str(), file.size))
        .collect();
    let image_size = compute_fat_size(&metas)?;

    build_with_size(files, image_size, writer)
}

/// Precomputes the exact FAT32 image size from file metadata without writing.
///
/// # Errors
///
/// Returns an error when a path is invalid, contains parent traversal, or the total data exceeds
/// the supported image size.
pub fn compute_fat_size(files: &[(&str, u64)]) -> Result<u64, EspError> {
    path::validate_spec_paths(files.iter().map(|&(path, _size)| path))?;
    let total_data: u64 = files.iter().map(|&(_path, size)| size).sum();

    image_size_for(total_data)
}

fn build_with_size<W: Write>(
    files: &mut [EspFile<'_>],
    image_size: u64,
    writer: &mut W,
) -> Result<(), EspError> {
    builder::build(files, image_size, writer)?;

    Ok(())
}

fn image_size_for(total_data: u64) -> Result<u64, EspError> {
    let padded = total_data.saturating_add(DIRECTORY_OVERHEAD);
    let data_region = next_multiple_of(padded, CLUSTER_SIZE).max(MIN_IMAGE_BYTES);
    if data_region > MAX_IMAGE_BYTES {
        return Err(FatError::Fat(format!(
            "ESP image size {data_region} exceeds {MAX_IMAGE_BYTES} byte limit"
        ))
        .into());
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

    use fatfs::error::FatError;

    use super::{build, build_with_size, compute_fat_size};
    use crate::error::EspError;
    use crate::model::{Arch, EspFile};

    fn fat_boot(data: &mut Cursor<Vec<u8>>) -> EspFile<'_> {
        let size = u64::try_from(data.get_ref().len()).unwrap_or(u64::MAX);

        EspFile::boot(Arch::X86_64, data, size)
    }

    #[test]
    fn compute_fat_size_returns_minimum_for_empty_spec() {
        // ARRANGE / ACT
        let size = compute_fat_size(&[]).expect("size must compute");

        // ASSERT
        assert!(size >= 1024 * 1024);
        assert_eq!(size.rem_euclid(512), 0);
    }

    #[test]
    fn compute_fat_size_grows_with_total_data() {
        // ARRANGE / ACT
        let small = compute_fat_size(&[("a", 4096)]).expect("small must compute");
        let large = compute_fat_size(&[("a", 4 * 1024 * 1024)]).expect("large must compute");

        // ASSERT
        assert!(large > small);
        assert_eq!(large.rem_euclid(512), 0);
    }

    #[test]
    fn compute_fat_size_rejects_invalid_paths() {
        // ARRANGE / ACT
        let result = compute_fat_size(&[("../escape", 0)]);

        // ASSERT
        assert!(matches!(result, Err(EspError::InvalidPath(_))));
    }

    #[test]
    fn compute_fat_size_rejects_oversized_data() {
        // ARRANGE
        let too_big = 600 * 1024 * 1024;

        // ACT
        let result = compute_fat_size(&[("big", too_big)]);

        // ASSERT
        assert!(matches!(result, Err(EspError::Fat(_))));
    }

    #[test]
    fn build_with_size_emits_readable_fat_image() {
        // ARRANGE
        let mut data = Cursor::new(b"uki-payload".to_vec());
        let mut files = vec![fat_boot(&mut data)];

        // ACT
        let mut out = Vec::new();
        build(&mut files, &mut out).expect("build must succeed");

        // ASSERT
        assert!(!out.is_empty());
        assert_eq!(
            out.get(510..512),
            Some(&[0x55, 0xAA][..]),
            "boot signature must be valid"
        );
        let fs_type_54 = out.get(54..62).unwrap_or(&[]);
        let fs_type_82 = out.get(82..90).unwrap_or(&[]);
        let valid =
            fs_type_54 == b"FAT12   " || fs_type_54 == b"FAT16   " || fs_type_82 == b"FAT32   ";
        assert!(
            valid,
            "filesystem type must be FAT12/FAT16 at offset 54 or FAT32 at offset 82"
        );
    }

    #[test]
    fn build_with_size_uses_explicit_size() {
        // ARRANGE
        let mut data = Cursor::new(b"uki".to_vec());
        let mut files = vec![fat_boot(&mut data)];
        let image_size =
            compute_fat_size(&[("EFI/BOOT/BOOTX64.EFI", 3)]).expect("size must compute");

        // ACT
        let mut out = Vec::new();
        build_with_size(&mut files, image_size, &mut out).expect("build must succeed");

        // ASSERT
        assert_eq!(out.len(), usize::try_from(image_size).unwrap_or(0));
    }

    struct FailingReader;

    impl std::io::Read for FailingReader {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("disk on fire"))
        }
    }

    #[test]
    fn build_propagates_io_errors_from_reader() {
        // ARRANGE
        let mut failing = FailingReader;
        let mut file = EspFile::boot(Arch::X86_64, &mut failing, 1024);

        // ACT
        let mut out = Vec::new();
        let result = build(core::slice::from_mut(&mut file), &mut out);

        // ASSERT
        assert!(matches!(result, Err(EspError::Fat(FatError::Io(_)))));
    }

    #[test]
    fn build_detects_short_reader() {
        // ARRANGE
        let mut cursor = Cursor::new(Vec::<u8>::new());
        let mut file = EspFile::boot(Arch::X86_64, &mut cursor, 16);

        // ACT
        let mut out = Vec::new();
        let result = build(core::slice::from_mut(&mut file), &mut out);

        // ASSERT
        assert!(matches!(result, Err(EspError::Fat(_))));
    }
}
