//! GPT table wrapper with a stable workspace-local API.

use std::io::{Read, Seek, SeekFrom, Write};

use gptman::{GPT, GPTPartitionEntry, PartitionName};

use super::serialize;
use super::types::Partition;
use crate::error::{ParttableError, Result};

/// A zero-allocation reader that reports a fixed disk size.
///
/// Only [`SeekFrom::End(0)`] is supported — no actual I/O occurs.
/// This satisfies [`GPT::new_from`]'s [`Read`] + [`Seek`] bounds while
/// avoiding a full-disk `Vec` allocation.
struct SizedDisk(u64);

impl Read for SizedDisk {
    fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
        Ok(0)
    }
}

impl Seek for SizedDisk {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        match pos {
            SeekFrom::End(0) => Ok(self.0),
            SeekFrom::Start(_) | SeekFrom::End(_) | SeekFrom::Current(_) => {
                Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "only SeekFrom::End(0) is supported",
                ))
            }
        }
    }
}

/// A GPT table wrapper with a stable workspace-local API.
#[derive(Debug)]
pub struct Table {
    pub(crate) inner: GPT,
}

impl Table {
    /// Creates a new GPT from known disk geometry.
    ///
    /// # Errors
    ///
    /// Returns an error when GPT initialization or arithmetic overflows.
    pub fn create(sector_count: u64, sector_size: u64, disk_guid: [u8; 16]) -> Result<Self> {
        let disk_size = sector_count
            .checked_mul(sector_size)
            .ok_or_else(|| ParttableError::Gpt("disk size overflow".to_owned()))?;
        let inner = GPT::new_from(&mut SizedDisk(disk_size), sector_size, disk_guid)
            .map_err(|e| ParttableError::Gpt(e.to_string()))?;

        Ok(Self { inner })
    }

    /// Reads an existing GPT from `reader`.
    ///
    /// # Errors
    ///
    /// Returns an error when the GPT cannot be decoded from `reader`.
    pub fn read<R: Read + Seek>(reader: &mut R) -> Result<Self> {
        let inner = GPT::find_from(reader).map_err(|e| ParttableError::Gpt(e.to_string()))?;

        Ok(Self { inner })
    }

    /// Reads an existing GPT from `reader`.
    ///
    /// # Errors
    ///
    /// Returns an error when the GPT cannot be decoded from `reader`.
    pub fn from_reader<R: Read + Seek>(reader: &mut R) -> Result<Self> {
        Self::read(reader)
    }

    /// Returns the sector size of the disk.
    #[must_use]
    pub fn sector_size(&self) -> u64 {
        self.inner.sector_size
    }

    /// Returns the first usable LBA from the GPT header.
    #[must_use]
    pub fn first_usable_lba(&self) -> u64 {
        self.inner.header.first_usable_lba
    }

    /// Returns the last usable LBA from the GPT header.
    #[must_use]
    pub fn last_usable_lba(&self) -> u64 {
        self.inner.header.last_usable_lba
    }

    /// Returns the size in bytes of the primary GPT region
    /// (protective MBR + GPT header + partition entries).
    #[must_use]
    pub fn primary_gpt_size(&self) -> u64 {
        self.first_usable_lba().saturating_mul(self.sector_size())
    }

    /// Returns the byte offset on disk where the backup GPT
    /// (partition entries + header) should be placed.
    #[must_use]
    pub fn backup_data_offset(&self, sector_count: u64) -> u64 {
        let entries_size = u64::from(self.inner.header.number_of_partition_entries)
            .saturating_mul(u64::from(self.inner.header.size_of_partition_entry));
        let entries_sectors = entries_size.div_ceil(self.inner.sector_size);
        let backup_start_lba = sector_count
            .saturating_sub(1)
            .saturating_sub(entries_sectors);

        backup_start_lba.saturating_mul(self.inner.sector_size)
    }

    /// Returns all used partitions as `(number, partition)` pairs.
    #[must_use]
    pub fn used_partitions(&self) -> Vec<(u32, Partition)> {
        self.inner
            .iter()
            .filter(|&(_, entry)| entry.is_used())
            .map(|(number, entry)| (number, Partition::from(entry)))
            .collect()
    }

    /// Returns `true` when the table contains any used partition.
    #[must_use]
    pub fn has_used_partitions(&self) -> bool {
        self.inner.iter().any(|(_, entry)| entry.is_used())
    }

    /// Returns the used partition at `number`, if present.
    #[must_use]
    pub fn partition(&self, number: u32) -> Option<Partition> {
        self.inner
            .iter()
            .find(|&(entry_number, entry)| entry_number == number && entry.is_used())
            .map(|(_, entry)| Partition::from(entry))
    }

    /// Returns `true` when `number` refers to a used partition.
    #[must_use]
    pub fn is_partition_used(&self, number: u32) -> bool {
        self.partition(number).is_some()
    }

    /// Returns the highest used partition number, if any.
    #[must_use]
    pub fn highest_used_partition_number(&self) -> Option<u32> {
        self.inner
            .iter()
            .filter(|&(_, entry)| entry.is_used())
            .map(|(number, _)| number)
            .max()
    }

    /// Returns the last used ending LBA, if any.
    #[must_use]
    pub fn last_used_ending_lba(&self) -> Option<u64> {
        self.inner
            .iter()
            .filter(|&(_, entry)| entry.is_used())
            .map(|(_, entry)| entry.ending_lba)
            .max()
    }

    /// Returns the next free partition number, if any.
    #[must_use]
    pub fn next_free_slot(&self) -> Option<u32> {
        let max_slots = self.inner.iter().map(|(number, _)| number).max()?;
        (1..=max_slots).find(|&number| !self.is_partition_used(number))
    }

    /// Sets `number` to `partition`.
    pub fn set_partition(&mut self, number: u32, partition: Partition) {
        self.inner[number] = partition.into();
    }

    /// Removes the partition at `number`.
    ///
    /// # Errors
    ///
    /// Returns an error when `number` cannot be removed from the underlying GPT.
    pub fn remove_partition(&mut self, number: u32) -> Result<()> {
        match self.inner.remove(number) {
            Ok(()) => Ok(()),
            Err(err) => Err(ParttableError::Gpt(err.to_string())),
        }
    }

    /// Writes the protective MBR, GPT header, and partition entries sequentially.
    ///
    /// # Errors
    ///
    /// Returns an error when writing the data fails.
    pub fn write_primary_to<W: Write>(&self, sector_count: u64, writer: &mut W) -> Result<()> {
        use crate::mbr::io;

        let mbr = io::protective_mbr_bytes(
            sector_count.saturating_mul(self.inner.sector_size),
            self.inner.sector_size,
        );
        writer.write_all(&mbr)?;

        let (header, entries_crc) = serialize::gpt_header_bytes(&self.inner, false, sector_count);
        let header = serialize::finalize_gpt_header(header, entries_crc);
        let entries = serialize::partition_entries_bytes(&self.inner);

        serialize::write_gpt_header(&header, writer)?;
        writer.write_all(&entries)?;

        Ok(())
    }

    /// Writes the backup GPT (partition entries + header) at the end of the disk.
    ///
    /// # Errors
    ///
    /// Returns an error when writing the data fails.
    pub fn write_backup_to<W: Write>(&self, sector_count: u64, writer: &mut W) -> Result<()> {
        let entries = serialize::partition_entries_bytes(&self.inner);
        let (header, entries_crc) = serialize::gpt_header_bytes(&self.inner, true, sector_count);
        let header = serialize::finalize_gpt_header(header, entries_crc);

        writer.write_all(&entries)?;
        serialize::write_gpt_header(&header, writer)?;

        Ok(())
    }
}

/// Rounds `lba` up to the nearest multiple of `align`.
#[must_use]
pub fn align_up_lba(lba: u64, align: u64) -> u64 {
    lba.next_multiple_of(align)
}

impl From<&GPTPartitionEntry> for Partition {
    fn from(entry: &GPTPartitionEntry) -> Self {
        Self {
            type_guid: entry.partition_type_guid,
            unique_guid: entry.unique_partition_guid,
            starting_lba: entry.starting_lba,
            ending_lba: entry.ending_lba,
            attributes: entry.attribute_bits,
            name: entry.partition_name.to_string(),
        }
    }
}

impl From<Partition> for GPTPartitionEntry {
    fn from(partition: Partition) -> Self {
        Self {
            partition_type_guid: partition.type_guid,
            unique_partition_guid: partition.unique_guid,
            starting_lba: partition.starting_lba,
            ending_lba: partition.ending_lba,
            attribute_bits: partition.attributes,
            partition_name: PartitionName::from(partition.name.as_str()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::super::types::*;
    use super::*;

    fn efi_partition(starting_lba: u64, ending_lba: u64) -> Partition {
        Partition {
            type_guid: EFI_GUID,
            unique_guid: [0xAB; 16],
            starting_lba,
            ending_lba,
            attributes: 0,
            name: "EFI".to_owned(),
        }
    }

    #[test]
    fn align_up_lba_keeps_aligned_value() {
        // ARRANGE
        let lba = ALIGN_1_MIB_SECTORS;

        // ACT
        let result = align_up_lba(lba, ALIGN_1_MIB_SECTORS);

        // ASSERT
        assert_eq!(result, ALIGN_1_MIB_SECTORS);
    }

    #[test]
    fn align_up_lba_rounds_unaligned_value() {
        // ARRANGE
        let lba = ALIGN_1_MIB_SECTORS + 1;

        // ACT
        let result = align_up_lba(lba, ALIGN_1_MIB_SECTORS);

        // ASSERT
        assert_eq!(result, ALIGN_1_MIB_SECTORS * 2);
    }

    #[test]
    fn align_up_lba_keeps_zero() {
        // ARRANGE
        let lba = 0;

        // ACT
        let result = align_up_lba(lba, ALIGN_1_MIB_SECTORS);

        // ASSERT
        assert_eq!(result, 0);
    }

    #[test]
    fn align_up_lba_result_is_always_aligned() {
        // ARRANGE
        let cases = [1_u64, 100, 2047, 2048, 2049, 4095, 4096, 100_000];

        // ACT / ASSERT
        for lba in cases {
            let result = align_up_lba(lba, ALIGN_1_MIB_SECTORS);
            assert!(result.is_multiple_of(ALIGN_1_MIB_SECTORS));
            assert!(result >= lba);
        }
    }

    #[test]
    fn efi_guid_matches_uefi_spec_value() {
        assert_eq!(
            EFI_GUID,
            [
                0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e,
                0xc9, 0x3b,
            ]
        );
    }

    #[test]
    fn create_table_exposes_usable_lba_range() {
        // ARRANGE / ACT
        let table = Table::create(8 * 2048, 512, [0xCD; 16]).expect("table must be created");

        // ASSERT
        assert!(table.first_usable_lba() > 0);
        assert!(table.last_usable_lba() >= table.first_usable_lba());
    }

    #[test]
    fn sector_size_returns_configured_value() {
        // ARRANGE / ACT
        let table = Table::create(8 * 2048, 512, [0xCD; 16]).expect("table must be created");

        // ASSERT
        assert_eq!(table.sector_size(), 512);
    }

    #[test]
    fn partition_persists_through_sequential_write_and_read() {
        // ARRANGE
        let sector_count = 8 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        table.set_partition(1, efi_partition(2048, 4095));

        // ACT
        let mut buf = Vec::new();
        table
            .write_primary_to(sector_count, &mut buf)
            .expect("primary write must succeed");
        table
            .write_backup_to(sector_count, &mut buf)
            .expect("backup write must succeed");

        let mut cursor = Cursor::new(buf);
        let reread = GPT::find_from(&mut cursor).expect("GPT must be readable");

        // ASSERT
        let reread_table = Table { inner: reread };
        let partition = reread_table.partition(1).expect("partition must exist");
        assert_eq!(partition.type_guid, EFI_GUID);
        assert_eq!(partition.starting_lba, 2048);
        assert_eq!(partition.ending_lba, 4095);
        assert_eq!(partition.name, "EFI");
    }

    #[test]
    fn primary_entries_match_gptman_output() {
        // ARRANGE
        let sector_count = 8 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        table.set_partition(1, efi_partition(2048, 4095));

        // ACT
        let mut seq_buf = Vec::new();
        table.write_primary_to(sector_count, &mut seq_buf).unwrap();
        table.write_backup_to(sector_count, &mut seq_buf).unwrap();

        let mut disk = Cursor::new(vec![0_u8; 512 * usize::try_from(sector_count).unwrap_or(0)]);
        let mut ref_gpt = GPT::new_from(&mut disk, 512, [0xCD; 16]).expect("gptman create");
        ref_gpt[1] = efi_partition(2048, 4095).into();
        ref_gpt.write_into(&mut disk).expect("gptman write");
        let ref_data = disk.into_inner();

        // ASSERT
        let entries_start: usize = 512 + 512;
        let entries_end = entries_start.saturating_add(16384);
        assert_eq!(
            seq_buf.get(entries_start..entries_end).unwrap_or(&[]),
            ref_data.get(entries_start..entries_end).unwrap_or(&[]),
            "primary partition entries must match"
        );
    }

    #[test]
    fn backup_entries_match_gptman_output() {
        // ARRANGE
        let sector_count = 8 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        table.set_partition(1, efi_partition(2048, 4095));

        // ACT
        let mut seq_buf = Vec::new();
        table.write_primary_to(sector_count, &mut seq_buf).unwrap();
        table.write_backup_to(sector_count, &mut seq_buf).unwrap();

        let mut disk = Cursor::new(vec![0_u8; 512 * usize::try_from(sector_count).unwrap_or(0)]);
        let mut ref_gpt = GPT::new_from(&mut disk, 512, [0xCD; 16]).expect("gptman create");
        ref_gpt[1] = efi_partition(2048, 4095).into();
        ref_gpt.write_into(&mut disk).expect("gptman write");
        let ref_data = disk.into_inner();

        // ASSERT
        let backup_header_start = ref_data.len().saturating_sub(512);
        let backup_entries_start = backup_header_start.saturating_sub(16384);
        let seq_backup_header_start = seq_buf.len().saturating_sub(512);
        let seq_backup_entries_start = seq_backup_header_start.saturating_sub(16384);

        assert_eq!(
            seq_buf
                .get(seq_backup_entries_start..seq_backup_entries_start.saturating_add(16384))
                .unwrap_or(&[]),
            ref_data
                .get(backup_entries_start..backup_entries_start.saturating_add(16384))
                .unwrap_or(&[]),
            "backup partition entries must match"
        );
    }

    #[test]
    fn backup_header_matches_gptman_output() {
        // ARRANGE
        let sector_count = 8 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        table.set_partition(1, efi_partition(2048, 4095));

        // ACT
        let mut seq_buf = Vec::new();
        table.write_primary_to(sector_count, &mut seq_buf).unwrap();
        table.write_backup_to(sector_count, &mut seq_buf).unwrap();

        let mut disk = Cursor::new(vec![0_u8; 512 * usize::try_from(sector_count).unwrap_or(0)]);
        let mut ref_gpt = GPT::new_from(&mut disk, 512, [0xCD; 16]).expect("gptman create");
        ref_gpt[1] = efi_partition(2048, 4095).into();
        ref_gpt.write_into(&mut disk).expect("gptman write");
        let ref_data = disk.into_inner();

        // ASSERT
        let backup_header_start = ref_data.len().saturating_sub(512);
        let seq_backup_header_start = seq_buf.len().saturating_sub(512);

        assert_eq!(
            seq_buf
                .get(seq_backup_header_start..seq_backup_header_start.saturating_add(92))
                .unwrap_or(&[]),
            ref_data
                .get(backup_header_start..backup_header_start.saturating_add(92))
                .unwrap_or(&[]),
            "backup GPT headers must match"
        );
    }

    #[test]
    fn used_partitions_returns_only_used_entries() {
        // ARRANGE
        let sector_count = 8 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        table.set_partition(1, efi_partition(2048, 4095));
        table.set_partition(
            2,
            Partition {
                type_guid: LINUX_FS_GUID,
                unique_guid: [0xBC; 16],
                starting_lba: 4096,
                ending_lba: 8191,
                attributes: 0,
                name: "DATA".to_owned(),
            },
        );

        // ACT
        let used = table.used_partitions();

        // ASSERT
        assert_eq!(used.len(), 2);
        assert!(matches!(used.first(), Some(&(1, _))));
        assert!(matches!(used.get(1), Some(&(2, _))));
    }

    #[test]
    fn highest_used_partition_number_returns_maximum_used_slot() {
        // ARRANGE
        let sector_count = 8 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        table.set_partition(2, efi_partition(4096, 8191));
        table.set_partition(7, efi_partition(8192, 12287));

        // ACT
        let highest = table.highest_used_partition_number();

        // ASSERT
        assert_eq!(highest, Some(7));
    }

    #[test]
    fn last_used_ending_lba_returns_farthest_partition_end() {
        // ARRANGE
        let sector_count = 8 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        table.set_partition(1, efi_partition(2048, 4095));
        table.set_partition(2, efi_partition(4096, 12287));

        // ACT
        let last_end = table.last_used_ending_lba();

        // ASSERT
        assert_eq!(last_end, Some(12287));
    }

    #[test]
    fn next_free_slot_returns_first_unused_slot() {
        // ARRANGE
        let sector_count = 8 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        table.set_partition(1, efi_partition(2048, 4095));
        table.set_partition(3, efi_partition(4096, 8191));

        // ACT
        let next = table.next_free_slot();

        // ASSERT
        assert_eq!(next, Some(2));
    }

    #[test]
    fn remove_partition_clears_used_slot() {
        // ARRANGE
        let sector_count = 8 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        table.set_partition(1, efi_partition(2048, 4095));

        // ACT
        table
            .remove_partition(1)
            .expect("partition must be removed");

        // ASSERT
        assert!(!table.is_partition_used(1));
        assert!(!table.has_used_partitions());
    }
}
