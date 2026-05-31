//! MBR I/O utilities.

use std::io::{Seek, SeekFrom, Write};

use super::types::{
    MBR_BOOT_SIGNATURE, MBR_BOOT_SIGNATURE_OFFSET, MBR_BYTES, MBR_CHS_LBA_PLACEHOLDER,
    MBR_ENTRY_BYTES, MBR_MAX_SLOTS, MBR_PARTITION_ENTRY_OFFSET, MBR_PARTITION_STARTING_LBA,
    MBR_PARTITION_TYPE_OFFSET, MBR_PROTECTIVE_GPT_TYPE, MBR_SIZE_LBA_OFFSET,
    MBR_STARTING_LBA_OFFSET, MbrPartitionEntry,
};
use crate::error::{ParttableError, Result};

/// Returns the protective MBR partition size in LBAs.
#[must_use]
pub fn protective_size_lba(disk_size: u64, sector_size: u64) -> u32 {
    let sectors = disk_size.checked_div(sector_size).unwrap_or(0);
    let size_lba = sectors.saturating_sub(1).min(u64::from(u32::MAX));
    u32::try_from(size_lba).unwrap_or(u32::MAX)
}

/// Writes one MBR partition entry into `slot`.
///
/// # Errors
///
/// Returns an error when `slot` is invalid or the underlying writer fails.
pub fn write_entry<W: Write + Seek>(
    writer: &mut W,
    slot: u8,
    entry: &MbrPartitionEntry,
) -> Result<()> {
    if slot >= MBR_MAX_SLOTS {
        return Err(ParttableError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("invalid MBR slot {slot}; expected 0..=3"),
        )));
    }

    let entry_bytes = u64::try_from(MBR_ENTRY_BYTES).unwrap_or(0);
    let offset =
        MBR_PARTITION_ENTRY_OFFSET.saturating_add(u64::from(slot).saturating_mul(entry_bytes));
    let mut raw = [0_u8; MBR_ENTRY_BYTES];
    raw[0] = if entry.bootable { 0x80 } else { 0x00 };
    raw[1..4].copy_from_slice(&MBR_CHS_LBA_PLACEHOLDER);
    raw[MBR_PARTITION_TYPE_OFFSET] = entry.partition_type;
    raw[5..8].copy_from_slice(&MBR_CHS_LBA_PLACEHOLDER);
    raw[MBR_STARTING_LBA_OFFSET..MBR_STARTING_LBA_OFFSET + 4]
        .copy_from_slice(&entry.starting_lba.to_le_bytes());
    raw[MBR_SIZE_LBA_OFFSET..MBR_SIZE_LBA_OFFSET + 4]
        .copy_from_slice(&entry.size_lba.to_le_bytes());

    writer.seek(SeekFrom::Start(offset))?;
    writer.write_all(&raw)?;
    Ok(())
}

/// Writes the canonical MBR boot signature.
///
/// # Errors
///
/// Returns an error when seeking or writing the signature fails.
pub fn write_signature<W: Write + Seek>(writer: &mut W) -> Result<()> {
    writer.seek(SeekFrom::Start(MBR_BOOT_SIGNATURE_OFFSET))?;
    writer.write_all(&MBR_BOOT_SIGNATURE)?;
    Ok(())
}

/// Writes a GPT protective MBR covering the whole disk.
///
/// # Errors
///
/// Returns an error when any write to the output device fails.
pub fn write_protective<W: Write + Seek>(
    writer: &mut W,
    disk_size: u64,
    sector_size: u64,
) -> Result<()> {
    let mut pmbr = [0_u8; MBR_BYTES];
    let mut cursor = std::io::Cursor::new(pmbr.as_mut_slice());
    let entry = MbrPartitionEntry {
        bootable: false,
        partition_type: MBR_PROTECTIVE_GPT_TYPE,
        starting_lba: MBR_PARTITION_STARTING_LBA,
        size_lba: protective_size_lba(disk_size, sector_size),
    };

    write_entry(&mut cursor, 0, &entry)?;
    write_signature(&mut cursor)?;

    writer.seek(SeekFrom::Start(0))?;
    writer.write_all(&pmbr)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{
        super::types::{
            MBR_BOOT_SIGNATURE, MBR_EFI_SYSTEM_TYPE, MBR_PARTITION_ENTRY_OFFSET,
            MBR_PROTECTIVE_GPT_TYPE, MbrPartitionEntry,
        },
        protective_size_lba, write_entry, write_protective, write_signature,
    };

    fn le_u32_at(data: &[u8], offset: usize) -> u32 {
        let end = offset
            .checked_add(4)
            .expect("test field offset must not overflow");
        let bytes = data
            .get(offset..end)
            .expect("test data must contain field bytes");
        u32::from_le_bytes(bytes.try_into().expect("field must be four bytes"))
    }

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
    fn write_entry_writes_lba_fields() {
        // ARRANGE
        let mut cursor = Cursor::new(vec![0_u8; 512]);
        let entry = MbrPartitionEntry {
            bootable: false,
            partition_type: MBR_EFI_SYSTEM_TYPE,
            starting_lba: 123,
            size_lba: 456,
        };

        // ACT
        write_entry(&mut cursor, 0, &entry).expect("MBR entry write must work");

        // ASSERT
        let data = cursor.into_inner();
        assert_eq!(data.get(450), Some(&MBR_EFI_SYSTEM_TYPE));
        assert_eq!(le_u32_at(&data, 454), 123);
        assert_eq!(le_u32_at(&data, 458), 456);
    }

    #[test]
    fn write_entry_writes_bootable_flag() {
        // ARRANGE
        let mut cursor = Cursor::new(vec![0_u8; 512]);
        let entry = MbrPartitionEntry {
            bootable: true,
            partition_type: MBR_EFI_SYSTEM_TYPE,
            starting_lba: 1,
            size_lba: 1,
        };

        // ACT
        write_entry(&mut cursor, 0, &entry).expect("MBR entry write must work");

        // ASSERT
        let data = cursor.into_inner();
        assert_eq!(data.get(446), Some(&0x80));
    }

    #[test]
    fn write_entry_rejects_invalid_slot() {
        // ARRANGE
        let mut cursor = Cursor::new(vec![0_u8; 512]);
        let entry = MbrPartitionEntry {
            bootable: false,
            partition_type: MBR_EFI_SYSTEM_TYPE,
            starting_lba: 1,
            size_lba: 1,
        };

        // ACT
        let result = write_entry(&mut cursor, 4, &entry);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn write_signature_writes_magic_bytes() {
        // ARRANGE
        let mut cursor = Cursor::new(vec![0_u8; 512]);

        // ACT
        write_signature(&mut cursor).expect("signature write must work");

        // ASSERT
        let data = cursor.into_inner();
        assert_eq!(data.get(510..512), Some(MBR_BOOT_SIGNATURE.as_slice()));
    }

    #[test]
    fn write_protective_writes_signature_and_type() {
        // ARRANGE
        let mut cursor = Cursor::new(vec![0_u8; 512]);

        // ACT
        write_protective(&mut cursor, 4096, 512).expect("protective MBR write must work");

        // ASSERT
        let data = cursor.into_inner();
        assert_eq!(data.get(450), Some(&MBR_PROTECTIVE_GPT_TYPE));
        assert_eq!(data.get(510..512), Some(MBR_BOOT_SIGNATURE.as_slice()));
    }

    #[test]
    fn write_protective_writes_starting_lba_one() {
        // ARRANGE
        let mut cursor = Cursor::new(vec![0_u8; 512]);

        // ACT
        write_protective(&mut cursor, 4096, 512).expect("protective MBR write must work");

        // ASSERT
        let data = cursor.into_inner();
        let offset = usize::try_from(MBR_PARTITION_ENTRY_OFFSET)
            .expect("MBR partition entry offset must fit usize")
            + 8;
        assert_eq!(le_u32_at(&data, offset), 1);
    }

    #[test]
    fn write_protective_writes_partition_size() {
        // ARRANGE
        let mut cursor = Cursor::new(vec![0_u8; 512]);

        // ACT
        write_protective(&mut cursor, 4096, 512).expect("protective MBR write must work");

        // ASSERT
        let data = cursor.into_inner();
        let offset = usize::try_from(MBR_PARTITION_ENTRY_OFFSET)
            .expect("MBR partition entry offset must fit usize")
            + 12;
        assert_eq!(le_u32_at(&data, offset), 7);
    }
}
