//! MBR-specific constants and helpers.

use std::io::{Seek, SeekFrom, Write};

/// The byte offset of the first MBR partition entry.
pub const MBR_PARTITION_ENTRY_OFFSET: u64 = 446;

/// The canonical MBR boot signature.
pub const MBR_BOOT_SIGNATURE: [u8; 2] = [0x55, 0xAA];

/// The protective partition type for GPT disks.
pub const MBR_PROTECTIVE_GPT_TYPE: u8 = 0xEE;

const MBR_BYTES: usize = 512;
const MBR_STARTING_LBA_OFFSET: usize = 8;
const MBR_SIZE_LBA_OFFSET: usize = 12;
const MBR_PARTITION_TYPE_OFFSET: usize = 4;
const MBR_PARTITION_STARTING_LBA: u32 = 1;

/// Returns the protective MBR partition size in LBAs.
pub fn protective_mbr_size_lba(disk_size: u64, sector_size: u64) -> u32 {
    (disk_size / sector_size)
        .saturating_sub(1)
        .min(u32::MAX as u64) as u32
}

/// Writes a GPT protective MBR covering the whole disk.
pub fn write_gpt_protective_mbr<W: Write + Seek>(
    writer: &mut W,
    disk_size: u64,
    sector_size: u64,
) -> std::io::Result<()> {
    let mut pmbr = [0u8; MBR_BYTES];
    let entry_offset = MBR_PARTITION_ENTRY_OFFSET as usize;

    pmbr[entry_offset + MBR_PARTITION_TYPE_OFFSET] = MBR_PROTECTIVE_GPT_TYPE;
    pmbr[entry_offset + MBR_STARTING_LBA_OFFSET..entry_offset + MBR_STARTING_LBA_OFFSET + 4]
        .copy_from_slice(&MBR_PARTITION_STARTING_LBA.to_le_bytes());
    pmbr[entry_offset + MBR_SIZE_LBA_OFFSET..entry_offset + MBR_SIZE_LBA_OFFSET + 4]
        .copy_from_slice(&protective_mbr_size_lba(disk_size, sector_size).to_le_bytes());
    pmbr[510..512].copy_from_slice(&MBR_BOOT_SIGNATURE);

    writer.seek(SeekFrom::Start(0))?;
    writer.write_all(&pmbr)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{
        MBR_BOOT_SIGNATURE, MBR_PARTITION_ENTRY_OFFSET, MBR_PROTECTIVE_GPT_TYPE,
        protective_mbr_size_lba, write_gpt_protective_mbr,
    };

    #[test]
    fn protective_mbr_size_lba_uses_all_remaining_sectors() {
        // ARRANGE
        let disk_size = 4096;

        // ACT
        let size = protective_mbr_size_lba(disk_size, 512);

        // ASSERT
        assert_eq!(size, 7);
    }

    #[test]
    fn protective_mbr_size_lba_clamps_large_disks() {
        // ARRANGE
        let disk_size = (u32::MAX as u64 + 100) * 512;

        // ACT
        let size = protective_mbr_size_lba(disk_size, 512);

        // ASSERT
        assert_eq!(size, u32::MAX);
    }

    #[test]
    fn protective_mbr_size_lba_handles_empty_disk() {
        // ARRANGE
        let disk_size = 0;

        // ACT
        let size = protective_mbr_size_lba(disk_size, 512);

        // ASSERT
        assert_eq!(size, 0);
    }

    #[test]
    fn write_gpt_protective_mbr_writes_signature_and_type() {
        // ARRANGE
        let mut cursor = Cursor::new(vec![0u8; 512]);

        // ACT
        write_gpt_protective_mbr(&mut cursor, 4096, 512).expect("protective MBR write must work");

        // ASSERT
        let data = cursor.into_inner();
        assert_eq!(data[450], MBR_PROTECTIVE_GPT_TYPE);
        assert_eq!(data[510..512], MBR_BOOT_SIGNATURE);
    }

    #[test]
    fn write_gpt_protective_mbr_writes_starting_lba_one() {
        // ARRANGE
        let mut cursor = Cursor::new(vec![0u8; 512]);

        // ACT
        write_gpt_protective_mbr(&mut cursor, 4096, 512).expect("protective MBR write must work");

        // ASSERT
        let data = cursor.into_inner();
        let offset = MBR_PARTITION_ENTRY_OFFSET as usize + 8;
        assert_eq!(
            u32::from_le_bytes(data[offset..offset + 4].try_into().expect("slice")),
            1
        );
    }

    #[test]
    fn write_gpt_protective_mbr_writes_partition_size() {
        // ARRANGE
        let mut cursor = Cursor::new(vec![0u8; 512]);

        // ACT
        write_gpt_protective_mbr(&mut cursor, 4096, 512).expect("protective MBR write must work");

        // ASSERT
        let data = cursor.into_inner();
        let offset = MBR_PARTITION_ENTRY_OFFSET as usize + 12;
        assert_eq!(
            u32::from_le_bytes(data[offset..offset + 4].try_into().expect("slice")),
            7
        );
    }
}
