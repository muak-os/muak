//! GPT-specific constants and helpers.

use std::io::{Read, Seek, Write};

use gptman::{GPT, GPTPartitionEntry, PartitionName};
use thiserror::Error;

/// The standard 1 MiB partition alignment in 512-byte sectors.
pub const ALIGN_1_MIB_SECTORS: u64 = 2048;

/// Linux filesystem partition type GUID (0FC63DAF-8483-4772-8E79-3D69D8477DE4).
pub const LINUX_FS_GUID: [u8; 16] = [
    0xaf, 0x3d, 0xc6, 0x0f, 0x83, 0x84, 0x72, 0x47, 0x8e, 0x79, 0x3d, 0x69, 0xd8, 0x47, 0x7d, 0xe4,
];

/// The EFI System Partition type GUID (C12A7328-F81F-11D2-BA4B-00A0C93EC93B).
pub const EFI_GUID: [u8; 16] = [
    0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9, 0x3b,
];

/// A GPT partition entry in a crate-local representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    pub type_guid: [u8; 16],
    pub unique_guid: [u8; 16],
    pub starting_lba: u64,
    pub ending_lba: u64,
    pub attributes: u64,
    pub name: String,
}

/// Errors returned by GPT table operations.
#[derive(Debug, Error)]
pub enum GptError {
    /// Wraps underlying I/O failures.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Wraps underlying GPT encoding and decoding failures.
    #[error("GPT error: {0}")]
    Gpt(String),
}

/// A GPT table wrapper with a stable workspace-local API.
#[derive(Debug)]
pub struct Table {
    inner: GPT,
}

impl Table {
    /// Creates a new GPT on `device` using the device size implied by the writer.
    pub fn create<W: Read + Write + Seek>(
        device: &mut W,
        sector_size: u64,
        disk_guid: [u8; 16],
    ) -> Result<Self, GptError> {
        let inner = GPT::new_from(device, sector_size, disk_guid)
            .map_err(|err| GptError::Gpt(err.to_string()))?;
        Ok(Self { inner })
    }

    /// Reads an existing GPT from `reader`.
    pub fn read<R: Read + Seek>(reader: &mut R) -> Result<Self, GptError> {
        let inner = GPT::find_from(reader).map_err(|err| GptError::Gpt(err.to_string()))?;
        Ok(Self { inner })
    }

    /// Returns the first usable LBA from the GPT header.
    pub fn first_usable_lba(&self) -> u64 {
        self.inner.header.first_usable_lba
    }

    /// Returns the last usable LBA from the GPT header.
    pub fn last_usable_lba(&self) -> u64 {
        self.inner.header.last_usable_lba
    }

    /// Returns all used partitions as `(number, partition)` pairs.
    pub fn used_partitions(&self) -> Vec<(u32, Partition)> {
        self.inner
            .iter()
            .filter(|(_, entry)| entry.is_used())
            .map(|(number, entry)| (number, Partition::from(entry)))
            .collect()
    }

    /// Returns `true` when the table contains any used partition.
    pub fn has_used_partitions(&self) -> bool {
        self.inner.iter().any(|(_, entry)| entry.is_used())
    }

    /// Returns the used partition at `number`, if present.
    pub fn partition(&self, number: u32) -> Option<Partition> {
        self.inner
            .iter()
            .find(|(entry_number, entry)| *entry_number == number && entry.is_used())
            .map(|(_, entry)| Partition::from(entry))
    }

    /// Returns `true` when `number` refers to a used partition.
    pub fn is_partition_used(&self, number: u32) -> bool {
        self.partition(number).is_some()
    }

    /// Returns the highest used partition number, if any.
    pub fn highest_used_partition_number(&self) -> Option<u32> {
        self.inner
            .iter()
            .filter(|(_, entry)| entry.is_used())
            .map(|(number, _)| number)
            .max()
    }

    /// Returns the last used ending LBA, if any.
    pub fn last_used_ending_lba(&self) -> Option<u64> {
        self.inner
            .iter()
            .filter(|(_, entry)| entry.is_used())
            .map(|(_, entry)| entry.ending_lba)
            .max()
    }

    /// Sets `number` to `partition`.
    pub fn set_partition(&mut self, number: u32, partition: Partition) {
        self.inner[number] = partition.into();
    }

    /// Removes the partition at `number`.
    pub fn remove_partition(&mut self, number: u32) -> Result<(), GptError> {
        self.inner
            .remove(number)
            .map_err(|err| GptError::Gpt(err.to_string()))
    }

    /// Writes the GPT back into `writer`.
    pub fn write<W: Write + Seek>(&mut self, writer: &mut W) -> Result<(), GptError> {
        self.inner
            .write_into(writer)
            .map(|_| ())
            .map_err(|err| GptError::Gpt(err.to_string()))
    }
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

/// Rounds `lba` up to the nearest multiple of `align`.
pub fn align_up_lba(lba: u64, align: u64) -> u64 {
    if lba.is_multiple_of(align) {
        lba
    } else {
        lba + (align - (lba % align))
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{ALIGN_1_MIB_SECTORS, EFI_GUID, Partition, Table, align_up_lba};

    fn blank_disk(size: usize) -> Cursor<Vec<u8>> {
        Cursor::new(vec![0u8; size])
    }

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
        let cases = [1u64, 100, 2047, 2048, 2049, 4095, 4096, 100_000];

        // ACT / ASSERT
        for lba in cases {
            let result = align_up_lba(lba, ALIGN_1_MIB_SECTORS);
            assert_eq!(result % ALIGN_1_MIB_SECTORS, 0);
            assert!(result >= lba);
        }
    }

    #[test]
    fn efi_guid_matches_uefi_spec_value() {
        // ARRANGE / ACT / ASSERT
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
        // ARRANGE
        let mut disk = blank_disk(8 * 1024 * 1024);

        // ACT
        let table = Table::create(&mut disk, 512, [0xCD; 16]).expect("table must be created");

        // ASSERT
        assert!(table.first_usable_lba() > 0);
        assert!(table.last_usable_lba() >= table.first_usable_lba());
    }

    #[test]
    fn set_partition_persists_through_write_and_read() {
        // ARRANGE
        let mut disk = blank_disk(8 * 1024 * 1024);
        let mut table = Table::create(&mut disk, 512, [0xCD; 16]).expect("table must be created");
        table.set_partition(1, efi_partition(2048, 4095));

        // ACT
        table.write(&mut disk).expect("table must be written");
        disk.set_position(0);
        let reread = Table::read(&mut disk).expect("table must be read back");

        // ASSERT
        let partition = reread.partition(1).expect("partition must exist");
        assert_eq!(partition.type_guid, EFI_GUID);
        assert_eq!(partition.starting_lba, 2048);
        assert_eq!(partition.ending_lba, 4095);
        assert_eq!(partition.name, "EFI");
    }

    #[test]
    fn used_partitions_returns_only_used_entries() {
        // ARRANGE
        let mut disk = blank_disk(8 * 1024 * 1024);
        let mut table = Table::create(&mut disk, 512, [0xCD; 16]).expect("table must be created");
        table.set_partition(1, efi_partition(2048, 4095));
        table.set_partition(
            2,
            Partition {
                type_guid: [
                    0xAF, 0x3D, 0xC6, 0x0F, 0x83, 0x84, 0x72, 0x47, 0x8E, 0x79, 0x3D, 0x69, 0xD8,
                    0x47, 0x7D, 0xE4,
                ],
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
        assert_eq!(used[0].0, 1);
        assert_eq!(used[1].0, 2);
    }

    #[test]
    fn highest_used_partition_number_returns_maximum_used_slot() {
        // ARRANGE
        let mut disk = blank_disk(8 * 1024 * 1024);
        let mut table = Table::create(&mut disk, 512, [0xCD; 16]).expect("table must be created");
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
        let mut disk = blank_disk(8 * 1024 * 1024);
        let mut table = Table::create(&mut disk, 512, [0xCD; 16]).expect("table must be created");
        table.set_partition(1, efi_partition(2048, 4095));
        table.set_partition(2, efi_partition(4096, 12287));

        // ACT
        let last_end = table.last_used_ending_lba();

        // ASSERT
        assert_eq!(last_end, Some(12287));
    }

    #[test]
    fn remove_partition_clears_used_slot() {
        // ARRANGE
        let mut disk = blank_disk(8 * 1024 * 1024);
        let mut table = Table::create(&mut disk, 512, [0xCD; 16]).expect("table must be created");
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
