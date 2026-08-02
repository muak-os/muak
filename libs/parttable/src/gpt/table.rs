//! In-memory GPT table model: creation, queries, and mutations.

use super::header::GptHeader;
use super::partition::Partition;
use crate::error::{ParttableError, Result};

/// Number of partition entry slots in a GPT table.
pub(crate) const ENTRIES_COUNT: usize = 128;
/// Size in bytes of a single partition entry.
pub(crate) const ENTRY_SIZE: u32 = 128;

/// An in-memory GPT partition table.
#[derive(Debug)]
pub struct Table {
    sector_size: u64,
    disk_guid: [u8; 16],
    first_usable_lba: u64,
    last_usable_lba: u64,
    entries: Vec<Option<Partition>>,
}

impl Table {
    /// Creates a new GPT from known disk geometry.
    ///
    /// # Errors
    ///
    /// Returns an error when the disk is too small for a GPT.
    pub fn create(sector_count: u64, sector_size: u64, disk_guid: [u8; 16]) -> Result<Self> {
        let entries_sectors = u64::from(ENTRY_SIZE)
            .saturating_mul(u64::try_from(ENTRIES_COUNT).unwrap_or(0))
            .div_ceil(sector_size);
        let first_usable = 2_u64.saturating_add(entries_sectors);
        let last_usable = sector_count
            .checked_sub(entries_sectors)
            .and_then(|count| count.checked_sub(2))
            .ok_or_else(|| ParttableError::Gpt("disk too small for GPT".to_owned()))?;

        Ok(Self {
            sector_size,
            disk_guid,
            first_usable_lba: first_usable,
            last_usable_lba: last_usable,
            entries: vec![None; ENTRIES_COUNT],
        })
    }

    /// Reconstructs a table from a parsed header and decoded entries.
    pub(crate) fn from_parts(
        first_usable_lba: u64,
        last_usable_lba: u64,
        disk_guid: [u8; 16],
        sector_size: u64,
        entries: Vec<Option<Partition>>,
    ) -> Result<Self> {
        if entries.len() != ENTRIES_COUNT {
            return Err(ParttableError::Gpt(
                "invalid partition entries length".to_owned(),
            ));
        }

        Ok(Self {
            sector_size,
            disk_guid,
            first_usable_lba,
            last_usable_lba,
            entries,
        })
    }

    /// Returns the header fields used when encoding this table.
    #[must_use]
    pub(crate) fn to_header(&self) -> GptHeader {
        GptHeader {
            entries_lba: 2,
            first_usable_lba: self.first_usable_lba,
            last_usable_lba: self.last_usable_lba,
            disk_guid: self.disk_guid,
            entries_count: u32::try_from(ENTRIES_COUNT).unwrap_or(0),
            entries_size: ENTRY_SIZE,
            entries_crc: 0,
        }
    }

    /// Returns the sector size of the disk.
    #[must_use]
    pub fn sector_size(&self) -> u64 {
        self.sector_size
    }

    /// Returns the first usable LBA from the GPT header.
    #[must_use]
    pub fn first_usable_lba(&self) -> u64 {
        self.first_usable_lba
    }

    /// Returns the last usable LBA from the GPT header.
    #[must_use]
    pub fn last_usable_lba(&self) -> u64 {
        self.last_usable_lba
    }

    /// Returns the size in bytes of the primary GPT region
    /// (protective MBR + GPT header + partition entries).
    #[must_use]
    pub fn primary_gpt_size(&self) -> u64 {
        self.first_usable_lba.saturating_mul(self.sector_size)
    }

    /// Returns the byte offset on disk where the backup GPT
    /// (partition entries + header) should be placed.
    #[must_use]
    pub fn backup_data_offset(&self, sector_count: u64) -> u64 {
        let entries_size =
            u64::from(ENTRY_SIZE).saturating_mul(u64::try_from(ENTRIES_COUNT).unwrap_or(0));
        let entries_sectors = entries_size.div_ceil(self.sector_size);
        sector_count
            .saturating_sub(1)
            .saturating_sub(entries_sectors)
            .saturating_mul(self.sector_size)
    }

    /// Returns all used partitions as `(number, partition)` pairs.
    #[must_use]
    pub fn used_partitions(&self) -> Vec<(u32, Partition)> {
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                entry
                    .as_ref()
                    .map(|partition| (slot_number(index), partition.clone()))
            })
            .collect()
    }

    /// Returns `true` when the table contains any used partition.
    #[must_use]
    pub fn has_used_partitions(&self) -> bool {
        self.entries.iter().any(Option::is_some)
    }

    /// Returns the used partition at `number`, if present.
    #[must_use]
    pub fn partition(&self, number: u32) -> Option<Partition> {
        self.entries
            .get(usize::try_from(number).ok()?.saturating_sub(1))?
            .clone()
    }

    /// Returns `true` when `number` refers to a used partition.
    #[must_use]
    pub fn is_partition_used(&self, number: u32) -> bool {
        self.partition(number).is_some()
    }

    /// Returns the highest used partition number, if any.
    #[must_use]
    pub fn highest_used_partition_number(&self) -> Option<u32> {
        self.entries
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, entry)| entry.as_ref().map(|_| slot_number(index)))
    }

    /// Returns the last used ending LBA, if any.
    #[must_use]
    pub fn last_used_ending_lba(&self) -> Option<u64> {
        self.entries
            .iter()
            .flatten()
            .map(|partition| partition.ending_lba)
            .max()
    }

    /// Returns the next free partition number, if any.
    #[must_use]
    pub fn next_free_slot(&self) -> Option<u32> {
        (1_u32..=u32::try_from(ENTRIES_COUNT).unwrap_or(0))
            .find(|&number| !self.is_partition_used(number))
    }

    /// Iterates over used partitions as `(number, partition)` pairs.
    pub fn partitions(&self) -> impl Iterator<Item = (u32, &Partition)> {
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                entry
                    .as_ref()
                    .map(|partition| (slot_number(index), partition))
            })
    }

    /// Sets `number` to `partition`.
    pub fn set_partition(&mut self, number: u32, partition: Partition) {
        if let Some(slot) = self
            .entries
            .get_mut(usize::try_from(number).unwrap_or(0).saturating_sub(1))
        {
            *slot = Some(partition);
        }
    }

    /// Removes the partition at `number`.
    ///
    /// # Errors
    ///
    /// Returns an error when `number` is out of range or the slot is already unused.
    pub fn remove_partition(&mut self, number: u32) -> Result<()> {
        let index = usize::try_from(number)
            .ok()
            .and_then(|number| number.checked_sub(1))
            .ok_or_else(|| ParttableError::Gpt("invalid partition number".to_owned()))?;
        let slot = self
            .entries
            .get_mut(index)
            .ok_or_else(|| ParttableError::Gpt("partition number out of range".to_owned()))?;
        if slot.is_none() {
            return Err(ParttableError::Gpt(format!(
                "partition {number} is already not set"
            )));
        }
        *slot = None;

        Ok(())
    }
}

fn slot_number(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(0).saturating_add(1)
}

#[cfg(test)]
mod tests {
    use super::super::partition::{EFI_GUID, LINUX_FS_GUID};
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
