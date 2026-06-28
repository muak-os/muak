//! Protective MBR serialization.

use std::io::{Seek, SeekFrom, Write};

use super::types::{
    MBR_BOOT_SIGNATURE, MBR_BYTES, MBR_ENTRY_SIZE, MBR_PARTITION_ENTRY_OFFSET,
    MBR_PROTECTIVE_GPT_TYPE, MbrPartitionEntry,
};
use crate::error::{ParttableError, Result};

/// Returns the protective MBR partition size in LBAs.
#[must_use]
pub fn protective_size_lba(disk_size: u64, sector_size: u64) -> u32 {
    let Some(sectors) = disk_size.checked_div(sector_size) else {
        return 0;
    };
    u32::try_from(sectors.saturating_sub(1).min(u64::from(u32::MAX))).unwrap_or(u32::MAX)
}

/// Returns a complete 512-byte protective MBR as a fixed-size array.
#[must_use]
pub fn protective_mbr_bytes(disk_size: u64, sector_size: u64) -> [u8; MBR_BYTES] {
    let mut mbr = [0_u8; MBR_BYTES];
    let entry = build_partition_entry(disk_size, sector_size);
    let Some(off) = usize::try_from(MBR_PARTITION_ENTRY_OFFSET).ok() else {
        return mbr;
    };
    let end = off.saturating_add(MBR_ENTRY_SIZE);
    if let Some(dst) = mbr.get_mut(off..end) {
        dst.copy_from_slice(&entry);
    }
    if let Some(dst) = mbr.get_mut(MBR_BYTES.saturating_sub(2)..) {
        dst.copy_from_slice(&MBR_BOOT_SIGNATURE);
    }

    mbr
}

/// Writes an MBR partition entry at the given slot index.
///
/// # Errors
///
/// Returns an error when seeking or writing fails.
pub fn write_entry<W: Write + Seek>(
    writer: &mut W,
    index: usize,
    entry: &MbrPartitionEntry,
) -> Result<()> {
    let entry_offset = usize::try_from(MBR_PARTITION_ENTRY_OFFSET)
        .map_err(|_err| ParttableError::Gpt("MBR entry offset overflow".to_owned()))?;
    let slot_offset = index
        .checked_mul(MBR_ENTRY_SIZE)
        .ok_or_else(|| ParttableError::Gpt("MBR entry index overflow".to_owned()))?;
    let offset = entry_offset
        .checked_add(slot_offset)
        .ok_or_else(|| ParttableError::Gpt("MBR entry offset overflow".to_owned()))?;
    writer.seek(SeekFrom::Start(u64::try_from(offset).unwrap_or(u64::MAX)))?;
    writer.write_all(&entry.to_bytes())?;
    Ok(())
}

/// Writes the MBR boot signature `0x55AA` at the end of the sector.
///
/// # Errors
///
/// Returns an error when seeking or writing fails.
pub fn write_signature<W: Write + Seek>(writer: &mut W) -> Result<()> {
    writer.seek(SeekFrom::Start(u64::try_from(MBR_BYTES - 2).unwrap_or(510)))?;
    writer.write_all(&MBR_BOOT_SIGNATURE)?;
    Ok(())
}

fn build_partition_entry(disk_size: u64, sector_size: u64) -> [u8; MBR_ENTRY_SIZE] {
    let entry = MbrPartitionEntry {
        bootable: false,
        partition_type: MBR_PROTECTIVE_GPT_TYPE,
        starting_lba: 1,
        size_lba: protective_size_lba(disk_size, sector_size),
    };
    entry.to_bytes()
}

#[cfg(test)]
mod tests {
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
}
