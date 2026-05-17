//! MBR-specific constants and helpers.

use std::io::{Seek, SeekFrom, Write};

/// The byte offset of the first MBR partition entry.
pub const MBR_PARTITION_ENTRY_OFFSET: u64 = 446;

/// The canonical MBR boot signature.
pub const MBR_BOOT_SIGNATURE: [u8; 2] = [0x55, 0xAA];

/// The EFI System Partition MBR type.
pub const MBR_EFI_SYSTEM_TYPE: u8 = 0xEF;

/// The protective partition type for GPT disks.
pub const MBR_PROTECTIVE_GPT_TYPE: u8 = 0xEE;

const MBR_BYTES: usize = 512;
const MBR_ENTRY_BYTES: usize = 16;
const MBR_MAX_SLOTS: u8 = 4;
const MBR_CHS_LBA_PLACEHOLDER: [u8; 3] = [0xFE, 0xFF, 0xFF];
const MBR_STARTING_LBA_OFFSET: usize = 8;
const MBR_SIZE_LBA_OFFSET: usize = 12;
const MBR_PARTITION_TYPE_OFFSET: usize = 4;
const MBR_PARTITION_STARTING_LBA: u32 = 1;
const MBR_BOOT_SIGNATURE_OFFSET: u64 = 510;

/// A single MBR partition entry in LBA form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MbrPartitionEntry {
    pub bootable: bool,
    pub partition_type: u8,
    pub starting_lba: u32,
    pub size_lba: u32,
}

/// Returns the protective MBR partition size in LBAs.
pub fn protective_mbr_size_lba(disk_size: u64, sector_size: u64) -> u32 {
    (disk_size / sector_size)
        .saturating_sub(1)
        .min(u32::MAX as u64) as u32
}

/// Writes one MBR partition entry into `slot`.
pub fn write_mbr_partition_entry<W: Write + Seek>(
    writer: &mut W,
    slot: u8,
    entry: &MbrPartitionEntry,
) -> std::io::Result<()> {
    if slot >= MBR_MAX_SLOTS {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("invalid MBR slot {slot}; expected 0..=3"),
        ));
    }

    let offset = MBR_PARTITION_ENTRY_OFFSET + u64::from(slot) * MBR_ENTRY_BYTES as u64;
    let mut raw = [0u8; MBR_ENTRY_BYTES];
    raw[0] = if entry.bootable { 0x80 } else { 0x00 };
    raw[1..4].copy_from_slice(&MBR_CHS_LBA_PLACEHOLDER);
    raw[MBR_PARTITION_TYPE_OFFSET] = entry.partition_type;
    raw[5..8].copy_from_slice(&MBR_CHS_LBA_PLACEHOLDER);
    raw[MBR_STARTING_LBA_OFFSET..MBR_STARTING_LBA_OFFSET + 4]
        .copy_from_slice(&entry.starting_lba.to_le_bytes());
    raw[MBR_SIZE_LBA_OFFSET..MBR_SIZE_LBA_OFFSET + 4]
        .copy_from_slice(&entry.size_lba.to_le_bytes());

    writer.seek(SeekFrom::Start(offset))?;
    writer.write_all(&raw)
}

/// Writes the canonical MBR boot signature.
pub fn write_mbr_boot_signature<W: Write + Seek>(writer: &mut W) -> std::io::Result<()> {
    writer.seek(SeekFrom::Start(MBR_BOOT_SIGNATURE_OFFSET))?;
    writer.write_all(&MBR_BOOT_SIGNATURE)
}

/// Writes a GPT protective MBR covering the whole disk.
pub fn write_gpt_protective_mbr<W: Write + Seek>(
    writer: &mut W,
    disk_size: u64,
    sector_size: u64,
) -> std::io::Result<()> {
    let mut pmbr = [0u8; MBR_BYTES];
    let mut cursor = std::io::Cursor::new(pmbr.as_mut_slice());
    let entry = MbrPartitionEntry {
        bootable: false,
        partition_type: MBR_PROTECTIVE_GPT_TYPE,
        starting_lba: MBR_PARTITION_STARTING_LBA,
        size_lba: protective_mbr_size_lba(disk_size, sector_size),
    };

    write_mbr_partition_entry(&mut cursor, 0, &entry)?;
    write_mbr_boot_signature(&mut cursor)?;

    writer.seek(SeekFrom::Start(0))?;
    writer.write_all(&pmbr)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{
        MBR_BOOT_SIGNATURE, MBR_EFI_SYSTEM_TYPE, MBR_PARTITION_ENTRY_OFFSET,
        MBR_PROTECTIVE_GPT_TYPE, MbrPartitionEntry, protective_mbr_size_lba,
        write_gpt_protective_mbr, write_mbr_boot_signature, write_mbr_partition_entry,
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
    fn write_mbr_partition_entry_writes_lba_fields() {
        // ARRANGE
        let mut cursor = Cursor::new(vec![0u8; 512]);
        let entry = MbrPartitionEntry {
            bootable: false,
            partition_type: MBR_EFI_SYSTEM_TYPE,
            starting_lba: 123,
            size_lba: 456,
        };

        // ACT
        write_mbr_partition_entry(&mut cursor, 0, &entry).expect("MBR entry write must work");

        // ASSERT
        let data = cursor.into_inner();
        assert_eq!(data[450], MBR_EFI_SYSTEM_TYPE);
        assert_eq!(
            u32::from_le_bytes(data[454..458].try_into().expect("slice")),
            123
        );
        assert_eq!(
            u32::from_le_bytes(data[458..462].try_into().expect("slice")),
            456
        );
    }

    #[test]
    fn write_mbr_partition_entry_writes_bootable_flag() {
        // ARRANGE
        let mut cursor = Cursor::new(vec![0u8; 512]);
        let entry = MbrPartitionEntry {
            bootable: true,
            partition_type: MBR_EFI_SYSTEM_TYPE,
            starting_lba: 1,
            size_lba: 1,
        };

        // ACT
        write_mbr_partition_entry(&mut cursor, 0, &entry).expect("MBR entry write must work");

        // ASSERT
        let data = cursor.into_inner();
        assert_eq!(data[446], 0x80);
    }

    #[test]
    fn write_mbr_partition_entry_rejects_invalid_slot() {
        // ARRANGE
        let mut cursor = Cursor::new(vec![0u8; 512]);
        let entry = MbrPartitionEntry {
            bootable: false,
            partition_type: MBR_EFI_SYSTEM_TYPE,
            starting_lba: 1,
            size_lba: 1,
        };

        // ACT
        let result = write_mbr_partition_entry(&mut cursor, 4, &entry);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn write_mbr_boot_signature_writes_magic_bytes() {
        // ARRANGE
        let mut cursor = Cursor::new(vec![0u8; 512]);

        // ACT
        write_mbr_boot_signature(&mut cursor).expect("signature write must work");

        // ASSERT
        let data = cursor.into_inner();
        assert_eq!(data[510..512], MBR_BOOT_SIGNATURE);
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
