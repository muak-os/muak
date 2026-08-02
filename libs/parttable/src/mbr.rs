//! Master Boot Record sector codec and builders.

use core::ops::Range;
use std::io::{Read, Write};

use crate::error::{ParttableError, Result};

/// The byte offset of the first MBR partition entry.
pub const MBR_PARTITION_ENTRY_OFFSET: usize = 446;

/// The canonical MBR boot signature.
pub const MBR_BOOT_SIGNATURE: [u8; 2] = [0x55, 0xAA];

/// The EFI System Partition MBR type.
pub const MBR_EFI_SYSTEM_TYPE: u8 = 0xEF;

/// The protective partition type for GPT disks.
pub const MBR_PROTECTIVE_GPT_TYPE: u8 = 0xEE;

/// Size of a complete MBR sector in bytes.
pub const MBR_BYTES: usize = 512;

/// Size of a single MBR partition entry in bytes.
pub const MBR_ENTRY_SIZE: usize = 16;

/// Number of MBR partition entry slots.
const MBR_ENTRIES: usize = 4;

/// A single MBR partition entry in LBA form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartitionEntry {
    /// Whether the partition is bootable.
    pub bootable: bool,
    /// MBR partition type byte.
    pub partition_type: u8,
    /// Starting LBA of the partition.
    pub starting_lba: u32,
    /// Size of the partition in LBAs.
    pub size_lba: u32,
}

impl PartitionEntry {
    /// Serializes this entry into a 16-byte MBR partition entry.
    #[must_use]
    pub fn to_bytes(self) -> [u8; MBR_ENTRY_SIZE] {
        let mut entry = [0_u8; MBR_ENTRY_SIZE];
        put(&mut entry, 0..1, &[u8::from(self.bootable)]);
        put(&mut entry, 1..4, &[0x00, 0x02, 0x00]);
        put(&mut entry, 4..5, &[self.partition_type]);
        put(&mut entry, 5..8, &[0xFF, 0xFF, 0xFF]);
        put(&mut entry, 8..12, &self.starting_lba.to_le_bytes());
        put(&mut entry, 12..16, &self.size_lba.to_le_bytes());

        entry
    }

    /// Parses a 16-byte entry, returning `None` for unused (empty) slots.
    pub(crate) fn decode(bytes: &[u8; MBR_ENTRY_SIZE]) -> Option<Self> {
        let partition_type = *bytes.get(4)?;
        if partition_type == 0 {
            return None;
        }

        Some(Self {
            bootable: *bytes.first()? != 0,
            partition_type,
            starting_lba: le_u32(bytes, 8..12)?,
            size_lba: le_u32(bytes, 12..16)?,
        })
    }
}

/// Returns a complete 512-byte MBR sector embedding `entry` at the partition slot and the boot signature at the end.
#[must_use]
pub fn bytes(entry: &PartitionEntry) -> [u8; MBR_BYTES] {
    let mut mbr = [0_u8; MBR_BYTES];
    put(
        &mut mbr,
        MBR_PARTITION_ENTRY_OFFSET..MBR_PARTITION_ENTRY_OFFSET.saturating_add(MBR_ENTRY_SIZE),
        &entry.to_bytes(),
    );
    put(
        &mut mbr,
        MBR_BYTES.saturating_sub(2)..MBR_BYTES,
        &MBR_BOOT_SIGNATURE,
    );

    mbr
}

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
pub fn protective_bytes(disk_size: u64, sector_size: u64) -> [u8; MBR_BYTES] {
    let entry = PartitionEntry {
        bootable: false,
        partition_type: MBR_PROTECTIVE_GPT_TYPE,
        starting_lba: 1,
        size_lba: protective_size_lba(disk_size, sector_size),
    };

    bytes(&entry)
}

/// Writes a complete MBR sector with the given partition entries sequentially.
///
/// # Errors
///
/// Returns an error when writing the data fails.
pub fn write<W: Write>(
    writer: &mut W,
    entries: &[Option<PartitionEntry>; MBR_ENTRIES],
) -> Result<()> {
    let mut sector = [0_u8; MBR_BYTES];
    for (index, entry) in entries.iter().copied().enumerate() {
        if let Some(entry) = entry {
            let offset =
                MBR_PARTITION_ENTRY_OFFSET.saturating_add(index.saturating_mul(MBR_ENTRY_SIZE));
            put(
                &mut sector,
                offset..offset.saturating_add(MBR_ENTRY_SIZE),
                &entry.to_bytes(),
            );
        }
    }
    put(
        &mut sector,
        MBR_BYTES.saturating_sub(2)..MBR_BYTES,
        &MBR_BOOT_SIGNATURE,
    );
    writer.write_all(&sector)?;

    Ok(())
}

/// Reads and parses the four partition entries of an MBR sector.
///
/// # Errors
///
/// Returns an error when the sector cannot be read or the boot signature is missing.
pub fn read<R: Read>(reader: &mut R) -> Result<[Option<PartitionEntry>; MBR_ENTRIES]> {
    let mut sector = [0_u8; MBR_BYTES];
    reader.read_exact(&mut sector)?;
    let signature = sector.get(MBR_BYTES.saturating_sub(2)..MBR_BYTES);
    if signature != Some(MBR_BOOT_SIGNATURE.as_slice()) {
        return Err(ParttableError::Gpt("invalid MBR boot signature".to_owned()));
    }

    let (_boot, tail) = sector.split_at(MBR_PARTITION_ENTRY_OFFSET);
    let (entries_region, _signature_region) =
        tail.split_at(MBR_ENTRIES.saturating_mul(MBR_ENTRY_SIZE));

    let mut entries = [None; MBR_ENTRIES];
    for (slot, chunk) in entries
        .iter_mut()
        .zip(entries_region.chunks_exact(MBR_ENTRY_SIZE))
    {
        let mut bytes = [0_u8; MBR_ENTRY_SIZE];
        bytes.copy_from_slice(chunk);
        *slot = PartitionEntry::decode(&bytes);
    }

    Ok(entries)
}

fn put(bytes: &mut [u8], range: Range<usize>, data: &[u8]) {
    if let Some(dst) = bytes.get_mut(range) {
        dst.copy_from_slice(data);
    }
}

fn le_u32(bytes: &[u8; MBR_ENTRY_SIZE], range: Range<usize>) -> Option<u32> {
    let value: [u8; 4] = bytes.get(range)?.try_into().ok()?;

    Some(u32::from_le_bytes(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linux_entry() -> PartitionEntry {
        PartitionEntry {
            bootable: false,
            partition_type: 0x83,
            starting_lba: 1,
            size_lba: 1,
        }
    }

    #[test]
    fn to_bytes_embeds_entry_fields() {
        // ARRANGE
        let entry = PartitionEntry {
            bootable: false,
            partition_type: 0x83,
            starting_lba: 1,
            size_lba: 7,
        };

        // ACT
        let bytes = entry.to_bytes();

        // ASSERT
        assert_eq!(bytes.first(), Some(&0));
        assert_eq!(bytes.get(4), Some(&0x83));
        assert_eq!(bytes.get(8..12), Some(&1_u32.to_le_bytes()[..]));
        assert_eq!(bytes.get(12..16), Some(&7_u32.to_le_bytes()[..]));
    }

    #[test]
    fn to_bytes_then_decode_round_trips() {
        // ARRANGE
        let entry = PartitionEntry {
            bootable: true,
            partition_type: 0xEF,
            starting_lba: 2048,
            size_lba: 4096,
        };

        // ACT
        let decoded = PartitionEntry::decode(&entry.to_bytes()).expect("entry must decode");

        // ASSERT
        assert_eq!(decoded, entry);
    }

    #[test]
    fn decode_returns_none_for_zeroed_entry() {
        // ARRANGE
        let entry_bytes = [0_u8; MBR_ENTRY_SIZE];

        // ACT
        let decoded = PartitionEntry::decode(&entry_bytes);

        // ASSERT
        assert!(decoded.is_none());
    }

    #[test]
    fn entry_debug_output_is_readable() {
        // ARRANGE
        let entry = linux_entry();

        // ACT
        let debug = format!("{entry:?}");

        // ASSERT
        assert!(debug.contains("PartitionEntry"));
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
    fn protective_size_lba_handles_zero_sector_size() {
        // ARRANGE
        let disk_size = 4096;

        // ACT
        let size = protective_size_lba(disk_size, 0);

        // ASSERT
        assert_eq!(size, 0);
    }

    #[test]
    fn bytes_embeds_entry_and_signature() {
        // ARRANGE
        let entry = linux_entry();

        // ACT
        let mbr = bytes(&entry);

        // ASSERT
        assert_eq!(mbr.get(MBR_PARTITION_ENTRY_OFFSET + 4), Some(&0x83));
        assert_eq!(mbr.get(MBR_BYTES - 2), Some(&0x55));
        assert_eq!(mbr.get(MBR_BYTES - 1), Some(&0xAA));
    }

    #[test]
    fn write_then_read_round_trips() {
        // ARRANGE
        let entries = [Some(linux_entry()), None, None, None];

        // ACT
        let mut buf = Vec::new();
        write(&mut buf, &entries).expect("write must succeed");
        let decoded = read(&mut buf.as_slice()).expect("read must succeed");

        // ASSERT
        assert_eq!(decoded, entries);
    }

    #[test]
    fn read_rejects_missing_signature() {
        // ARRANGE
        let mut sector = [0_u8; MBR_BYTES];
        if let Some(byte) = sector.get_mut(MBR_PARTITION_ENTRY_OFFSET.saturating_add(4)) {
            *byte = 0x83;
        }

        // ACT
        let result = read(&mut sector.as_slice());

        // ASSERT
        assert!(matches!(
            result,
            Err(ParttableError::Gpt(message)) if message == "invalid MBR boot signature"
        ));
    }

    #[test]
    fn read_returns_none_for_empty_slots() {
        // ARRANGE
        let mut sector = [0_u8; MBR_BYTES];
        if let Some(byte) = sector.get_mut(MBR_PARTITION_ENTRY_OFFSET.saturating_add(4)) {
            *byte = 0x83;
        }
        if let Some(sig) = sector.get_mut(MBR_BYTES.saturating_sub(2)..) {
            sig.copy_from_slice(&MBR_BOOT_SIGNATURE);
        }

        // ACT
        let entries = read(&mut sector.as_slice()).expect("read must succeed");

        // ASSERT
        assert!(entries[0].is_some());
        assert!(entries[1].is_none());
        assert!(entries[2].is_none());
        assert!(entries[3].is_none());
    }
}
