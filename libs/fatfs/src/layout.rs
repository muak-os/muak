//! FAT32 layout selection from a target image size.

use crate::error::{FatError, Result};
use crate::types::{
    FAT_ENTRY_SIZE, FAT32_MIN_CLUSTERS, FatLayout, MAX_IMAGE_SIZE, RESERVED_SECTORS, SECTOR_SIZE,
};

pub(crate) fn compute(image_size: u64) -> Result<FatLayout> {
    if image_size > MAX_IMAGE_SIZE {
        return Err(FatError::Fat(format!(
            "image too large for FAT32: {image_size} bytes > {MAX_IMAGE_SIZE}"
        )));
    }
    let total_sectors = image_size.div_euclid(SECTOR_SIZE);
    if total_sectors < 2 {
        return Err(FatError::Fat("image too small for reserved area".into()));
    }
    let rsvd = RESERVED_SECTORS;
    let spc_values: &[u64] = &[64, 32, 16, 8, 4, 2, 1];
    for &spc in spc_values {
        let result = test_spc(spc, total_sectors, rsvd, 0);
        let (fat_sectors, final_clusters, _) = match result {
            Some(triple) if triple.1 >= FAT32_MIN_CLUSTERS => triple,
            _ => continue,
        };
        return Ok(FatLayout {
            total_sectors,
            reserved_sectors: rsvd,
            fat_sectors,
            spc,
            data_cluster_count: final_clusters,
        });
    }

    Err(FatError::Fat(
        "image size insufficient for any FAT type".into(),
    ))
}

fn test_spc(spc: u64, total_sectors: u64, rsvd: u64, root_secs: u64) -> Option<(u64, u64, u64)> {
    let data_sectors = total_sectors.wrapping_sub(rsvd);
    let total_clusters = data_sectors.div_euclid(spc);
    if total_clusters == 0 {
        return None;
    }
    let fat_entries = total_clusters.saturating_add(2);
    let fat_bytes = fat_entries.checked_mul(FAT_ENTRY_SIZE)?;
    let fat_sectors = fat_bytes
        .next_multiple_of(SECTOR_SIZE)
        .div_euclid(SECTOR_SIZE);
    let actual_data_sectors = data_sectors
        .wrapping_sub(fat_sectors.wrapping_mul(crate::types::FAT_COUNT))
        .wrapping_sub(root_secs);
    let final_clusters = actual_data_sectors.div_euclid(spc);
    if final_clusters == 0 {
        return None;
    }

    Some((fat_sectors, final_clusters, spc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MIN_IMAGE_SIZE, SECTOR_SIZE};

    #[test]
    fn compute_layout_accepts_the_largest_fat32_volume() {
        // ARRANGE / ACT
        let result = compute(MAX_IMAGE_SIZE);

        // ASSERT
        assert!(result.is_ok(), "the largest FAT32 volume must be accepted");
    }

    #[test]
    fn compute_layout_rejects_volumes_above_fat32_ceiling() {
        // ARRANGE
        let oversized = MAX_IMAGE_SIZE.saturating_add(SECTOR_SIZE);

        // ACT
        let result = compute(oversized);

        // ASSERT
        assert!(
            result.is_err(),
            "volumes above the FAT32 ceiling must be rejected"
        );
    }

    #[test]
    fn compute_layout_rejects_the_smallest_image_below_the_floor() {
        // ARRANGE
        let undersized = MIN_IMAGE_SIZE.saturating_sub(1);

        // ACT
        let result = compute(undersized);

        // ASSERT
        assert!(result.is_err(), "too-small volumes must be rejected");
    }
}
