//! GPT partition placement policy and request types.

use super::partition::Partition;
use super::table::Table;
use crate::error::{ParttableError, Result};

/// The standard 1 MiB partition alignment in 512-byte sectors.
pub const ALIGN_1_MIB_SECTORS: u64 = 2048;

/// Selects how a partition slot should be chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// Automatically select the first available slot.
    Auto,
    /// Use the exact slot number given.
    Exact(u32),
}

/// Selects how a partition start LBA should be chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    /// Start at the first usable LBA.
    FirstUsable,
    /// Start after the last used partition.
    AfterLastUsed,
    /// Start at or after the given LBA.
    AtOrAfter(u64),
    /// Start after the partition with the given number.
    AfterPartition(u32),
}

/// Selects how a partition size should be chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    /// Size in bytes.
    Bytes(u64),
    /// Size in LBAs (512-byte sectors).
    Lbas(u64),
    /// Fill to the last usable LBA.
    FillToLastUsable,
}

/// Returns the resolved partition placement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    /// Partition number (1-based index).
    pub number: u32,
    /// The resolved partition entry.
    pub partition: Partition,
}

/// Describes one checked placement request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacementRequest {
    /// How the partition slot is chosen.
    pub slot: Slot,
    /// How the partition start LBA is chosen.
    pub start: Start,
    /// How the partition size is chosen.
    pub size: Size,
    /// Alignment boundary in LBAs.
    pub alignment_lba: u64,
    /// GPT partition type GUID.
    pub type_guid: [u8; 16],
    /// Unique partition GUID.
    pub unique_guid: [u8; 16],
    /// Partition attributes bitfield.
    pub attributes: u64,
    /// Partition name.
    pub name: String,
}

impl PlacementRequest {
    /// Places one partition into `table` using checked alignment and range rules.
    ///
    /// # Errors
    ///
    /// Returns an error when slot selection, sizing, alignment, or range validation fails.
    pub fn place(&self, table: &mut Table, sector_size: u64) -> Result<Placement> {
        let number = match self.slot {
            Slot::Auto => table.next_free_slot().ok_or_else(|| {
                ParttableError::InvalidPlacement("no free GPT partition slots".to_owned())
            })?,
            Slot::Exact(number) => resolve_exact_slot(table, number)?,
        };

        let anchor = resolve_start_anchor(table, self.start)?;
        let start = anchor.next_multiple_of(self.alignment_lba);
        let end = resolve_end_lba(table, start, self.size, sector_size)?;
        validate_partition_range(table, number, start, end)?;

        let partition = Partition {
            type_guid: self.type_guid,
            unique_guid: self.unique_guid,
            starting_lba: start,
            ending_lba: end,
            attributes: self.attributes,
            name: self.name.clone(),
        };
        table.set_partition(number, partition.clone());

        Ok(Placement { number, partition })
    }
}

fn resolve_start_anchor(table: &Table, start: Start) -> Result<u64> {
    match start {
        Start::FirstUsable => Ok(table.first_usable_lba()),
        Start::AfterLastUsed => table.last_used_ending_lba().map_or_else(
            || Ok(table.first_usable_lba()),
            |lba| checked_next_lba(lba, "after last used partition"),
        ),
        Start::AtOrAfter(lba) => Ok(lba),
        Start::AfterPartition(number) => table
            .partition(number)
            .map(|partition| {
                checked_next_lba(partition.ending_lba, &format!("after partition {number}"))
            })
            .transpose()?
            .ok_or_else(|| {
                ParttableError::InvalidPlacement(format!(
                    "cannot place after missing partition {number}"
                ))
            }),
    }
}

fn resolve_exact_slot(table: &Table, number: u32) -> Result<u32> {
    if !table.is_partition_used(number) {
        return Ok(number);
    }

    Err(ParttableError::InvalidPlacement(format!(
        "partition slot {number} is already in use"
    )))
}

fn resolve_end_lba(table: &Table, start: u64, size: Size, sector_size: u64) -> Result<u64> {
    match size {
        Size::Bytes(bytes) => {
            let lbas = nonzero_lbas(bytes.div_ceil(sector_size))?;
            checked_end_lba(start, lbas)
        }
        Size::Lbas(lbas) => {
            let lbas = nonzero_lbas(lbas)?;
            checked_end_lba(start, lbas)
        }
        Size::FillToLastUsable => Ok(table.last_usable_lba()),
    }
}

fn validate_partition_range(table: &Table, number: u32, start: u64, end: u64) -> Result<()> {
    if start > end {
        return Err(ParttableError::InvalidPlacement(format!(
            "partition {number} start LBA {start} is after end LBA {end}"
        )));
    }
    if start < table.first_usable_lba() {
        return Err(ParttableError::InvalidPlacement(format!(
            "partition {number} starts before first usable LBA {}",
            table.first_usable_lba()
        )));
    }
    if end > table.last_usable_lba() {
        return Err(ParttableError::InvalidPlacement(format!(
            "partition {number} ends after last usable LBA {}",
            table.last_usable_lba()
        )));
    }

    if let Some(existing_number) = table.partitions().find_map(|(n, entry)| {
        (n != number && start <= entry.ending_lba && end >= entry.starting_lba).then_some(n)
    }) {
        return Err(ParttableError::InvalidPlacement(format!(
            "partition {number} overlaps partition {existing_number}"
        )));
    }

    Ok(())
}

fn checked_next_lba(ending_lba: u64, context: &str) -> Result<u64> {
    ending_lba.checked_add(1).ok_or_else(|| {
        ParttableError::InvalidPlacement(format!("partition start LBA overflowed {context}"))
    })
}

fn checked_end_lba(start: u64, lbas: u64) -> Result<u64> {
    start
        .checked_add(lbas.saturating_sub(1))
        .ok_or_else(|| ParttableError::InvalidPlacement("partition end LBA overflowed".to_owned()))
}

fn nonzero_lbas(lbas: u64) -> Result<u64> {
    if lbas != 0 {
        return Ok(lbas);
    }

    Err(ParttableError::InvalidPlacement(
        "partition size must be greater than zero".to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use esp::EFI_GUID;

    use super::super::partition::{LINUX_FS_GUID, Partition};
    use super::super::table::ENTRIES_COUNT;
    use super::*;
    use crate::error::ParttableError;

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

    fn request(
        slot: Slot,
        start: Start,
        size: Size,
        type_guid: [u8; 16],
        name: &str,
    ) -> PlacementRequest {
        PlacementRequest {
            slot,
            start,
            size,
            alignment_lba: ALIGN_1_MIB_SECTORS,
            type_guid,
            unique_guid: [0xCD; 16],
            attributes: 0,
            name: name.to_owned(),
        }
    }

    #[test]
    fn place_partition_aligns_first_usable_request() {
        // ARRANGE
        let sector_count = 16 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");

        // ACT
        let placement = request(
            Slot::Exact(1),
            Start::FirstUsable,
            Size::Bytes(1024 * 1024),
            EFI_GUID,
            "EFI",
        )
        .place(&mut table, 512)
        .expect("placement must succeed");

        // ASSERT
        assert_eq!(placement.number, 1);
        assert!(
            placement
                .partition
                .starting_lba
                .is_multiple_of(ALIGN_1_MIB_SECTORS)
        );
    }

    #[test]
    fn place_partition_after_partition_uses_previous_end() {
        // ARRANGE
        let sector_count = 32 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        let first = request(
            Slot::Exact(1),
            Start::FirstUsable,
            Size::Bytes(1024 * 1024),
            EFI_GUID,
            "EFI",
        )
        .place(&mut table, 512)
        .expect("first placement must succeed");

        // ACT
        let second = request(
            Slot::Exact(2),
            Start::AfterPartition(first.number),
            Size::Bytes(1024 * 1024),
            LINUX_FS_GUID,
            "STATE",
        )
        .place(&mut table, 512)
        .expect("second placement must succeed");

        // ASSERT
        assert!(second.partition.starting_lba > first.partition.ending_lba);
        assert!(
            second
                .partition
                .starting_lba
                .is_multiple_of(ALIGN_1_MIB_SECTORS)
        );
    }

    #[test]
    fn place_partition_auto_slot_uses_next_free_slot() {
        // ARRANGE
        let sector_count = 16 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        table.set_partition(1, efi_partition(2048, 4095));

        // ACT
        let placement = request(
            Slot::Auto,
            Start::AfterLastUsed,
            Size::Bytes(1024 * 1024),
            LINUX_FS_GUID,
            "DATA",
        )
        .place(&mut table, 512)
        .expect("placement must succeed");

        // ASSERT
        assert_eq!(placement.number, 2);
    }

    #[test]
    fn place_partition_fill_to_last_usable_extends_to_table_end() {
        // ARRANGE
        let sector_count = 16 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");

        // ACT
        let placement = request(
            Slot::Exact(1),
            Start::FirstUsable,
            Size::FillToLastUsable,
            LINUX_FS_GUID,
            "DATA",
        )
        .place(&mut table, 512)
        .expect("placement must succeed");

        // ASSERT
        assert_eq!(placement.partition.ending_lba, table.last_usable_lba());
    }

    #[test]
    fn place_partition_rejects_overlap() {
        // ARRANGE
        let sector_count = 16 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        table.set_partition(1, efi_partition(2048, 4095));

        // ACT
        let result = request(
            Slot::Exact(2),
            Start::AtOrAfter(2048),
            Size::Lbas(10),
            LINUX_FS_GUID,
            "BAD",
        )
        .place(&mut table, 512);

        // ASSERT
        assert!(matches!(result, Err(ParttableError::InvalidPlacement(_))));
    }

    #[test]
    fn place_partition_rejects_used_exact_slot() {
        // ARRANGE
        let sector_count = 16 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        table.set_partition(1, efi_partition(2048, 4095));

        // ACT
        let result = request(
            Slot::Exact(1),
            Start::AfterLastUsed,
            Size::Lbas(10),
            LINUX_FS_GUID,
            "BAD",
        )
        .place(&mut table, 512);

        // ASSERT
        assert!(matches!(result, Err(ParttableError::InvalidPlacement(_))));
    }

    #[test]
    fn place_partition_auto_slot_rejects_fully_used_table() {
        // ARRANGE
        let sector_count = 16 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        let max_slots = u32::try_from(ENTRIES_COUNT).expect("slot count must fit in u32");
        for slot in 1..=max_slots {
            let start = 2048 + (u64::from(slot) - 1) * ALIGN_1_MIB_SECTORS;
            let end = start + 1023;
            table.set_partition(slot, efi_partition(start, end));
        }

        // ACT
        let result = request(
            Slot::Auto,
            Start::AfterLastUsed,
            Size::Lbas(1),
            LINUX_FS_GUID,
            "OVERFLOW",
        )
        .place(&mut table, 512);

        // ASSERT
        assert!(
            matches!(result, Err(ParttableError::InvalidPlacement(message)) if message == "no free GPT partition slots")
        );
    }

    #[test]
    fn place_partition_rejects_missing_anchor_partition() {
        // ARRANGE
        let sector_count = 16 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");

        // ACT
        let result = request(
            Slot::Exact(1),
            Start::AfterPartition(9),
            Size::Lbas(1),
            LINUX_FS_GUID,
            "DATA",
        )
        .place(&mut table, 512);

        // ASSERT
        assert!(
            matches!(result, Err(ParttableError::InvalidPlacement(message)) if message == "cannot place after missing partition 9")
        );
    }

    #[test]
    fn place_partition_rejects_zero_size() {
        // ARRANGE
        let sector_count = 16 * 2048;

        for size in [Size::Bytes(0), Size::Lbas(0)] {
            let mut table =
                Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");

            // ACT
            let result = request(
                Slot::Exact(1),
                Start::FirstUsable,
                size,
                LINUX_FS_GUID,
                "EMPTY",
            )
            .place(&mut table, 512);

            // ASSERT
            assert!(
                matches!(result, Err(ParttableError::InvalidPlacement(message)) if message == "partition size must be greater than zero")
            );
        }
    }

    #[test]
    fn place_partition_rejects_start_after_end() {
        // ARRANGE
        let sector_count = 16 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        let invalid_start = table.last_usable_lba() + 1;
        let mut req = request(
            Slot::Exact(1),
            Start::AtOrAfter(invalid_start),
            Size::FillToLastUsable,
            LINUX_FS_GUID,
            "PAST-END",
        );
        req.alignment_lba = 1;

        // ACT
        let result = req.place(&mut table, 512);

        // ASSERT
        assert!(
            matches!(result, Err(ParttableError::InvalidPlacement(message)) if message.contains("start LBA") && message.contains("is after end LBA"))
        );
    }

    #[test]
    fn place_partition_rejects_start_before_first_usable() {
        // ARRANGE
        let sector_count = 16 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        let invalid_start = table.first_usable_lba() - 1;
        let mut req = request(
            Slot::Exact(1),
            Start::AtOrAfter(invalid_start),
            Size::Lbas(1),
            LINUX_FS_GUID,
            "TOO-EARLY",
        );
        req.alignment_lba = 1;

        // ACT
        let result = req.place(&mut table, 512);

        // ASSERT
        assert!(
            matches!(result, Err(ParttableError::InvalidPlacement(message)) if message.contains("starts before first usable LBA"))
        );
    }

    #[test]
    fn place_partition_rejects_end_after_last_usable() {
        // ARRANGE
        let sector_count = 16 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        let invalid_start = table.last_usable_lba();

        // ACT
        let result = request(
            Slot::Exact(1),
            Start::AtOrAfter(invalid_start),
            Size::Lbas(2),
            LINUX_FS_GUID,
            "TOO-LATE",
        )
        .place(&mut table, 512);

        // ASSERT
        assert!(
            matches!(result, Err(ParttableError::InvalidPlacement(message)) if message.contains("ends after last usable LBA"))
        );
    }
}
