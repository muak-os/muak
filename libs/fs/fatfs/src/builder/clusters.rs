//! Cluster allocation for the FAT image.

use crate::error::{FatError, Result};
use crate::types::{ClusterMap, FatLayout, FileMeta, ROOT_CLUSTER, SECTOR_SIZE};

/// Assigns a contiguous cluster range to every directory, then to every file.
///
/// # Errors
///
/// Returns `Error::Fat` when a file does not fit in a FAT32 directory entry,
/// when the cluster index leaves the FAT32 range, or when the payload needs
/// more clusters than the layout provides.
pub(crate) fn assign_clusters(
    files: &[FileMeta<'_>],
    layout: &FatLayout,
    dir_sizes: &[u64],
) -> Result<ClusterMap> {
    let cluster_bytes = layout.spc.wrapping_mul(SECTOR_SIZE);
    let mut next_cluster = u64::from(ROOT_CLUSTER);
    let mut dir_starts = Vec::with_capacity(dir_sizes.len());
    let mut dir_counts = Vec::with_capacity(dir_sizes.len());
    for &size in dir_sizes {
        let count = size.div_ceil(cluster_bytes);
        dir_starts.push(cluster_start(next_cluster)?);
        dir_counts
            .push(u32::try_from(count).map_err(|_conv| FatError::Fat("too many clusters".into()))?);
        next_cluster = advance(next_cluster, count)?;
    }

    let mut file_starts = Vec::with_capacity(files.len());
    let mut file_counts = Vec::with_capacity(files.len());
    let mut file_sizes = Vec::with_capacity(files.len());
    for file in files {
        if file.size > u64::from(u32::MAX) {
            return Err(FatError::Fat(format!(
                "file too large for FAT32 directory entry: {} bytes > {}",
                file.size,
                u32::MAX
            )));
        }
        let count = file.size.div_ceil(cluster_bytes);
        file_starts.push(cluster_start(next_cluster)?);
        file_counts.push(count);
        file_sizes.push(file.size);
        next_cluster = advance(next_cluster, count)?;
    }

    let used = next_cluster.wrapping_sub(u64::from(ROOT_CLUSTER));
    if used > layout.data_cluster_count {
        return Err(FatError::Fat(format!(
            "need {used} data clusters but only {} available",
            layout.data_cluster_count
        )));
    }

    Ok(ClusterMap {
        dir_starts,
        dir_counts,
        file_starts,
        file_counts,
        file_sizes,
    })
}

fn cluster_start(next_cluster: u64) -> Result<u32> {
    u32::try_from(next_cluster)
        .map_err(|_conv| FatError::Fat("cluster index exceeds FAT32 range".into()))
}

fn advance(next_cluster: u64, count: u64) -> Result<u64> {
    next_cluster
        .checked_add(count)
        .ok_or_else(|| FatError::Fat("cluster overflow".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(data_cluster_count: u64) -> FatLayout {
        FatLayout {
            total_sectors: 139_528,
            reserved_sectors: 8,
            fat_sectors: 546,
            spc: 2,
            data_cluster_count,
        }
    }

    #[test]
    fn directories_are_allocated_before_files() {
        // ARRANGE
        let files = &[FileMeta::new("a.bin", 1024), FileMeta::new("b.bin", 1)];

        // ACT
        let map = assign_clusters(files, &layout(1024), &[3072, 1024]).expect("map");

        // ASSERT
        assert_eq!(map.dir_starts, vec![2, 5], "root then the child directory");
        assert_eq!(map.dir_counts, vec![3, 1], "three and one cluster");
        assert_eq!(map.file_starts, vec![6, 7], "files follow the directories");
        assert_eq!(map.file_counts, vec![1, 1], "one cluster each");
    }

    #[test]
    fn partial_clusters_are_rounded_up() {
        // ARRANGE
        let files = &[FileMeta::new("a.bin", 1), FileMeta::new("b.bin", 2048)];

        // ACT
        let map = assign_clusters(files, &layout(1024), &[64]).expect("map");

        // ASSERT
        assert_eq!(map.file_counts, vec![1, 2], "1 byte and 2048 bytes");
    }

    #[test]
    fn oversized_payloads_are_rejected() {
        // ARRANGE
        let files = &[FileMeta::new("a.bin", 3 * 1024)];

        // ACT
        let result = assign_clusters(files, &layout(2), &[1024]);

        // ASSERT
        assert!(result.is_err(), "the layout must provide enough clusters");
    }

    #[test]
    fn files_above_four_gibibytes_are_rejected() {
        // ARRANGE
        let files = &[FileMeta::new(
            "big.bin",
            u64::from(u32::MAX).saturating_add(1),
        )];

        // ACT
        let result = assign_clusters(files, &layout(1024), &[]);

        // ASSERT
        assert!(result.is_err(), "FAT32 sizes are 32 bits wide");
    }
}
