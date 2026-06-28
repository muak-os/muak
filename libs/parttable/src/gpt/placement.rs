//! GPT partition placement algorithm.

#![expect(
    clippy::multiple_inherent_impl,
    reason = "split across table.rs and placement.rs by design"
)]

use super::table::{Table, align_up_lba};
use super::types::{Partition, Placement, PlacementRequest, Size, Slot, Start};
use crate::error::{ParttableError, Result};

impl Table {
    /// Places one partition using checked alignment and range rules.
    ///
    /// # Errors
    ///
    /// Returns an error when slot selection, sizing, alignment, or range validation fails.
    pub fn place_partition(
        &mut self,
        request: PlacementRequest,
        sector_size: u64,
    ) -> Result<Placement> {
        let number = match request.slot {
            Slot::Auto => self.next_free_slot().ok_or_else(|| {
                ParttableError::InvalidPlacement("no free GPT partition slots".to_owned())
            })?,
            Slot::Exact(number) => self.resolve_exact_slot(number)?,
        };

        let anchor = self.resolve_start_anchor(request.start)?;
        let start = align_up_lba(anchor, request.alignment_lba);
        let end = self.resolve_end_lba(start, request.size, sector_size)?;
        self.validate_partition_range(number, start, end)?;

        let partition = Partition {
            type_guid: request.type_guid,
            unique_guid: request.unique_guid,
            starting_lba: start,
            ending_lba: end,
            attributes: request.attributes,
            name: request.name,
        };
        self.set_partition(number, partition.clone());

        Ok(Placement { number, partition })
    }

    fn resolve_start_anchor(&self, start: Start) -> Result<u64> {
        match start {
            Start::FirstUsable => Ok(self.first_usable_lba()),
            Start::AfterLastUsed => self.last_used_ending_lba().map_or_else(
                || Ok(self.first_usable_lba()),
                |lba| Self::checked_next_lba(lba, "after last used partition"),
            ),
            Start::AtOrAfter(lba) => Ok(lba),
            Start::AfterPartition(number) => self
                .partition(number)
                .map(|partition| {
                    Self::checked_next_lba(
                        partition.ending_lba,
                        &format!("after partition {number}"),
                    )
                })
                .transpose()?
                .ok_or_else(|| {
                    ParttableError::InvalidPlacement(format!(
                        "cannot place after missing partition {number}"
                    ))
                }),
        }
    }

    fn resolve_exact_slot(&self, number: u32) -> Result<u32> {
        if !self.is_partition_used(number) {
            return Ok(number);
        }

        Err(ParttableError::InvalidPlacement(format!(
            "partition slot {number} is already in use"
        )))
    }

    fn resolve_end_lba(&self, start: u64, size: Size, sector_size: u64) -> Result<u64> {
        match size {
            Size::Bytes(bytes) => {
                let lbas = Self::nonzero_lbas(bytes.div_ceil(sector_size))?;
                Self::checked_end_lba(start, lbas)
            }
            Size::Lbas(lbas) => {
                let lbas = Self::nonzero_lbas(lbas)?;
                Self::checked_end_lba(start, lbas)
            }
            Size::FillToLastUsable => Ok(self.last_usable_lba()),
        }
    }

    fn checked_next_lba(ending_lba: u64, context: &str) -> Result<u64> {
        ending_lba.checked_add(1).ok_or_else(|| {
            ParttableError::InvalidPlacement(format!("partition start LBA overflowed {context}"))
        })
    }

    fn checked_end_lba(start: u64, lbas: u64) -> Result<u64> {
        start.checked_add(lbas.saturating_sub(1)).ok_or_else(|| {
            ParttableError::InvalidPlacement("partition end LBA overflowed".to_owned())
        })
    }

    fn nonzero_lbas(lbas: u64) -> Result<u64> {
        if lbas != 0 {
            return Ok(lbas);
        }

        Err(ParttableError::InvalidPlacement(
            "partition size must be greater than zero".to_owned(),
        ))
    }

    fn validate_partition_range(&self, number: u32, start: u64, end: u64) -> Result<()> {
        if start > end {
            return Err(ParttableError::InvalidPlacement(format!(
                "partition {number} start LBA {start} is after end LBA {end}"
            )));
        }
        if start < self.first_usable_lba() {
            return Err(ParttableError::InvalidPlacement(format!(
                "partition {number} starts before first usable LBA {}",
                self.first_usable_lba()
            )));
        }
        if end > self.last_usable_lba() {
            return Err(ParttableError::InvalidPlacement(format!(
                "partition {number} ends after last usable LBA {}",
                self.last_usable_lba()
            )));
        }

        if let Some(existing_number) = self.inner.iter().find_map(|(n, e)| {
            (n != number && e.is_used() && start <= e.ending_lba && end >= e.starting_lba)
                .then_some(n)
        }) {
            return Err(ParttableError::InvalidPlacement(format!(
                "partition {number} overlaps partition {existing_number}"
            )));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ParttableError;
    use crate::gpt::types::*;

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
        let placement = table
            .place_partition(
                request(
                    Slot::Exact(1),
                    Start::FirstUsable,
                    Size::Bytes(1024 * 1024),
                    EFI_GUID,
                    "EFI",
                ),
                512,
            )
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
        let first = table
            .place_partition(
                request(
                    Slot::Exact(1),
                    Start::FirstUsable,
                    Size::Bytes(1024 * 1024),
                    EFI_GUID,
                    "EFI",
                ),
                512,
            )
            .expect("first placement must succeed");

        // ACT
        let second = table
            .place_partition(
                request(
                    Slot::Exact(2),
                    Start::AfterPartition(first.number),
                    Size::Bytes(1024 * 1024),
                    LINUX_FS_GUID,
                    "STATE",
                ),
                512,
            )
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
        let placement = table
            .place_partition(
                request(
                    Slot::Auto,
                    Start::AfterLastUsed,
                    Size::Bytes(1024 * 1024),
                    LINUX_FS_GUID,
                    "DATA",
                ),
                512,
            )
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
        let placement = table
            .place_partition(
                request(
                    Slot::Exact(1),
                    Start::FirstUsable,
                    Size::FillToLastUsable,
                    LINUX_FS_GUID,
                    "DATA",
                ),
                512,
            )
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
        let result = table.place_partition(
            request(
                Slot::Exact(2),
                Start::AtOrAfter(2048),
                Size::Lbas(10),
                LINUX_FS_GUID,
                "BAD",
            ),
            512,
        );

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
        let result = table.place_partition(
            request(
                Slot::Exact(1),
                Start::AfterLastUsed,
                Size::Lbas(10),
                LINUX_FS_GUID,
                "BAD",
            ),
            512,
        );

        // ASSERT
        assert!(matches!(result, Err(ParttableError::InvalidPlacement(_))));
    }

    #[test]
    fn place_partition_auto_slot_rejects_fully_used_table() {
        // ARRANGE
        let sector_count = 16 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        let max_slots = table
            .inner
            .iter()
            .map(|(number, _)| number)
            .max()
            .expect("table must expose partition slots");
        for slot in 1..=max_slots {
            let start = 2048 + (u64::from(slot) - 1) * ALIGN_1_MIB_SECTORS;
            let end = start + 1023;
            table.set_partition(slot, efi_partition(start, end));
        }

        // ACT
        let result = table.place_partition(
            request(
                Slot::Auto,
                Start::AfterLastUsed,
                Size::Lbas(1),
                LINUX_FS_GUID,
                "OVERFLOW",
            ),
            512,
        );

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
        let result = table.place_partition(
            request(
                Slot::Exact(1),
                Start::AfterPartition(9),
                Size::Lbas(1),
                LINUX_FS_GUID,
                "DATA",
            ),
            512,
        );

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
            let result = table.place_partition(
                request(
                    Slot::Exact(1),
                    Start::FirstUsable,
                    size,
                    LINUX_FS_GUID,
                    "EMPTY",
                ),
                512,
            );

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
        let result = table.place_partition(req, 512);

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
        let result = table.place_partition(req, 512);

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
        let result = table.place_partition(
            request(
                Slot::Exact(1),
                Start::AtOrAfter(invalid_start),
                Size::Lbas(2),
                LINUX_FS_GUID,
                "TOO-LATE",
            ),
            512,
        );

        // ASSERT
        assert!(
            matches!(result, Err(ParttableError::InvalidPlacement(message)) if message.contains("ends after last usable LBA"))
        );
    }
}
