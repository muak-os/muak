//! Protective MBR serialization.

use super::types::{
    MBR_BOOT_SIGNATURE, MBR_BYTES, MBR_PARTITION_ENTRY_OFFSET, MBR_PROTECTIVE_GPT_TYPE,
};

/// Returns the protective MBR partition size in LBAs.
#[must_use]
pub fn protective_size_lba(disk_size: u64, sector_size: u64) -> u32 {
    let sectors = disk_size.checked_div(sector_size).unwrap_or(0);
    let size_lba = sectors.saturating_sub(1).min(u64::from(u32::MAX));
    u32::try_from(size_lba).unwrap_or(u32::MAX)
}

/// Returns a complete 512-byte protective MBR as a fixed-size array.
#[must_use]
pub fn protective_mbr_bytes(disk_size: u64, sector_size: u64) -> [u8; MBR_BYTES] {
    let mut mbr = [0_u8; MBR_BYTES];
    let entry = build_partition_entry(disk_size, sector_size);
    let Some(off) = usize::try_from(MBR_PARTITION_ENTRY_OFFSET).ok() else {
        return mbr;
    };
    let end = off.saturating_add(16);
    if let Some(dst) = mbr.get_mut(off..end) {
        dst.copy_from_slice(&entry);
    }
    if let Some(dst) = mbr.get_mut(MBR_BYTES.saturating_sub(2)..) {
        dst.copy_from_slice(&MBR_BOOT_SIGNATURE);
    }

    mbr
}

fn build_partition_entry(disk_size: u64, sector_size: u64) -> [u8; 16] {
    let mut entry = [0_u8; 16];
    entry[0] = 0x00;
    entry[1] = 0x00;
    entry[2] = 0x02;
    entry[3] = 0x00;
    entry[4] = MBR_PROTECTIVE_GPT_TYPE;
    entry[5] = 0xFF;
    entry[6] = 0xFF;
    entry[7] = 0xFF;
    entry[8..12].copy_from_slice(&1_u32.to_le_bytes());
    entry[12..16].copy_from_slice(&protective_size_lba(disk_size, sector_size).to_le_bytes());

    entry
}

#[cfg(test)]
mod tests {
    use super::super::types::MBR_PROTECTIVE_GPT_TYPE;
    use super::*;

    #[test]
    fn protective_size_lba_uses_all_remaining_sectors() {
        // ARRANGE
        let disk_size = 4096;

        // ACT
        let size = protective_size_lba(disk_size, 512);

        // ASSERT
        assert_eq!(size, 7);
    }

    #[test]
    fn protective_size_lba_clamps_large_disks() {
        // ARRANGE
        let disk_size = (u64::from(u32::MAX) + 100) * 512;

        // ACT
        let size = protective_size_lba(disk_size, 512);

        // ASSERT
        assert_eq!(size, u32::MAX);
    }

    #[test]
    fn protective_size_lba_handles_empty_disk() {
        // ARRANGE
        let disk_size = 0;

        // ACT
        let size = protective_size_lba(disk_size, 512);

        // ASSERT
        assert_eq!(size, 0);
    }

    #[test]
    fn protective_mbr_bytes_has_correct_type() {
        // ARRANGE / ACT
        let mbr = protective_mbr_bytes(4096, 512);

        // ASSERT
        assert_eq!(mbr.get(450), Some(&MBR_PROTECTIVE_GPT_TYPE));
        assert_eq!(mbr.get(510), Some(&0x55));
        assert_eq!(mbr.get(511), Some(&0xAA));
    }

    #[test]
    fn protective_mbr_bytes_has_starting_lba_one() {
        // ARRANGE / ACT
        let mbr = protective_mbr_bytes(4096, 512);

        // ASSERT
        let offset = usize::try_from(MBR_PARTITION_ENTRY_OFFSET).unwrap_or(0) + 8;
        let start = u32::from_le_bytes(
            mbr.get(offset..offset + 4)
                .expect("start LBA bytes")
                .try_into()
                .unwrap(),
        );
        assert_eq!(start, 1);
    }

    #[test]
    fn protective_mbr_bytes_has_partition_size() {
        // ARRANGE / ACT
        let mbr = protective_mbr_bytes(4096, 512);

        // ASSERT
        let offset = usize::try_from(MBR_PARTITION_ENTRY_OFFSET).unwrap_or(0) + 12;
        let size = u32::from_le_bytes(
            mbr.get(offset..offset + 4)
                .expect("size LBA bytes")
                .try_into()
                .unwrap(),
        );
        assert_eq!(size, 7);
    }
}
