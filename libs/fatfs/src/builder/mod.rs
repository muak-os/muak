//! FAT image building API.

pub(crate) mod clusters;
mod write;

use std::io::{Read, Write};

use clusters::assign_clusters;
use write::{CountWriter, stream_reader, write_boot_sector, write_dir_entries};

use crate::dir;
use crate::error::{FatError, Result};
use crate::layout;
use crate::name::short::{NAME_LEN, ShortNames};
use crate::table;
use crate::tree::DirIndex;
use crate::types::{FileMeta, FsInfo, MAX_IMAGE_SIZE, MIN_IMAGE_SIZE, Precomputed, SECTOR_SIZE};

/// Bytes reserved for directory clusters and FAT overhead when sizing a volume.
const DIRECTORY_OVERHEAD: u64 = 1 << 20;
/// Free space kept available on volumes sized by [`image_size_for`].
pub const FREE_SLACK: u64 = 4 << 20;

/// Returns the smallest FAT32 volume that fits `total_data` bytes of file data.
///
/// # Errors
///
/// Returns `Error::Fat` when the volume would exceed the FAT32 ceiling.
pub fn image_size_for(total_data: u64) -> Result<u64> {
    let target = total_data
        .saturating_add(DIRECTORY_OVERHEAD)
        .saturating_add(FREE_SLACK);
    let mut size = target.max(MIN_IMAGE_SIZE).next_multiple_of(SECTOR_SIZE);
    loop {
        let geometry = layout::compute(size)?;
        let capacity = geometry
            .data_cluster_count
            .saturating_mul(geometry.spc)
            .saturating_mul(SECTOR_SIZE);
        if capacity >= target {
            return Ok(size);
        }
        if size >= MAX_IMAGE_SIZE {
            return Err(FatError::Fat(
                "volume would exceed the FAT32 ceiling".to_owned(),
            ));
        }
        size = size
            .saturating_add(FREE_SLACK)
            .next_multiple_of(SECTOR_SIZE)
            .min(MAX_IMAGE_SIZE);
    }
}

/// Precomputes all FAT metadata from file paths and sizes.
///
/// # Errors
///
/// Returns `Error::Fat` when layout computation fails or files don't fit.
pub fn precompute<'a>(files: &'a [FileMeta<'a>], image_size: u64) -> Result<Precomputed<'a>> {
    let layout = layout::compute(image_size)?;
    let tree = DirIndex::collect(files);
    let aliases = assign_aliases(&tree, files)?;
    let dir_sizes = dir::sizes(&tree, files, &aliases);
    let cluster_map = assign_clusters(files, &layout, &dir_sizes)?;
    let fat_bytes = table::make_fat(&cluster_map, &layout);
    let dir_data = (0..tree.len())
        .map(|dir_index| dir::build_data(files, &tree, &cluster_map, &aliases, dir_index))
        .collect();
    let fs_info = FsInfo::from_map(&cluster_map, &layout);

    Ok(Precomputed {
        layout,
        tree,
        cluster_map,
        fat_bytes,
        dir_data,
        fs_info,
        image_size,
    })
}

/// Builds a FAT image by writing precomputed metadata and streaming file data.
///
/// # Errors
///
/// Returns `Error::Io` when writing fails, or `Error::Fat` when a reader
/// returns EOF before its declared size.
pub fn build<R: Read, W: Write>(
    precomputed: &Precomputed<'_>,
    readers: &mut [R],
    writer: &mut W,
) -> Result<()> {
    let mut counted = CountWriter {
        inner: writer,
        written: 0,
    };
    write_boot_sector(&mut counted, &precomputed.layout, precomputed.fs_info)?;
    counted.write_all(&precomputed.fat_bytes)?;
    counted.write_all(&precomputed.fat_bytes)?;
    write_dir_entries(&mut counted, precomputed)?;
    let cluster_bytes = precomputed.layout.spc.wrapping_mul(SECTOR_SIZE);
    for (index, reader) in readers.iter_mut().enumerate() {
        let size = precomputed
            .cluster_map
            .file_sizes
            .get(index)
            .copied()
            .unwrap_or(0);
        stream_reader(&mut counted, reader, size, cluster_bytes)?;
    }
    let written = counted.written;
    if written < precomputed.image_size {
        dir::write_zeros(&mut counted, precomputed.image_size.wrapping_sub(written))?;
    }

    Ok(())
}

/// Formats a writable target as an empty FAT volume.
///
/// # Errors
///
/// Returns `Error::Fat` when layout computation fails, or `Error::Io`
/// when writing the empty volume fails.
pub fn format<W: Write>(writer: &mut W, size: u64) -> Result<()> {
    let files: &[FileMeta<'_>] = &[];
    let precomputed = precompute(files, size)?;
    let readers: &mut [std::io::Empty] = &mut [];

    build(&precomputed, readers, writer)
}

fn assign_aliases<'a>(
    tree: &DirIndex<'a>,
    files: &'a [FileMeta<'a>],
) -> Result<Vec<Vec<[u8; NAME_LEN]>>> {
    (0..tree.len())
        .map(|dir_index| {
            let mut names = ShortNames::new();
            tree.entries(files, dir_index)
                .map(|(name, _)| names.assign(name))
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FAT32_MIN_CLUSTERS, MAX_IMAGE_SIZE};

    #[test]
    fn image_size_leaves_the_configured_slack() {
        // ARRANGE
        let total_data = 69 * 1024 * 1024;

        // ACT
        let size = image_size_for(total_data).expect("size must compute");
        let geometry = layout::compute(size).expect("geometry must compute");
        let capacity = geometry.data_cluster_count * geometry.spc * SECTOR_SIZE;

        // ASSERT
        assert!(
            capacity >= total_data + FREE_SLACK,
            "sized volumes must keep {FREE_SLACK} bytes free"
        );
        assert_eq!(size.rem_euclid(SECTOR_SIZE), 0, "size is sector aligned");
    }

    #[test]
    fn image_size_is_monotonic_in_the_payload() {
        // ARRANGE / ACT
        let small = image_size_for(1024).expect("small must compute");
        let large = image_size_for(64 * 1024 * 1024).expect("large must compute");

        // ASSERT
        assert!(large > small, "larger payloads need larger volumes");
    }

    #[test]
    fn image_size_uses_the_fat32_floor_for_tiny_payloads() {
        // ARRANGE / ACT
        let size = image_size_for(0).expect("empty payload must compute");
        let geometry = layout::compute(size).expect("geometry must compute");

        // ASSERT
        assert!(
            geometry.data_cluster_count >= FAT32_MIN_CLUSTERS,
            "tiny volumes must still be FAT32"
        );
        assert!(size >= MIN_IMAGE_SIZE, "minimum image size");
    }

    #[test]
    fn image_size_rejects_oversized_payloads() {
        // ARRANGE / ACT
        let result = image_size_for(MAX_IMAGE_SIZE);

        // ASSERT
        assert!(
            result.is_err(),
            "payloads above the ceiling must be rejected"
        );
    }

    #[test]
    fn precompute_rejects_files_larger_than_a_fat32_entry() {
        // ARRANGE
        let files = &[FileMeta::new(
            "big.bin",
            u64::from(u32::MAX).saturating_add(1),
        )];

        // ACT
        let result = precompute(files, 5_u64 * 1024 * 1024 * 1024);

        // ASSERT
        assert!(result.is_err(), "files over 4 GiB must not fit");
    }

    #[test]
    fn format_writes_a_bootable_empty_volume() {
        // ARRANGE
        let mut buf = Vec::new();

        // ACT
        format(&mut buf, 36 * 1024 * 1024).expect("format must succeed");

        // ASSERT
        assert_eq!(buf.get(510..512), Some(&[0x55, 0xAA][..]), "boot signature");
        assert_eq!(buf.get(3..11), Some(&b"MSWIN4.1"[..]), "OEM ID");
        assert_eq!(
            u64::try_from(buf.len()).unwrap_or(u64::MAX),
            36 * 1024 * 1024,
            "the image must be exactly the requested size"
        );
    }
}
