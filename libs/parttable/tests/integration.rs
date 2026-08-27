//! Integration tests for the public API of the parttable library.

#[cfg(test)]
mod tests {
    use esp::EFI_GUID;
    use parttable::error::ParttableError;
    use parttable::gpt::io;
    use parttable::gpt::layout::{ALIGN_1_MIB_SECTORS, PlacementRequest, Size, Slot, Start};
    use parttable::gpt::partition::{LINUX_FS_GUID, Partition};
    use parttable::gpt::table::Table;

    fn sector_count(bytes: usize, sector_size: u64) -> u64 {
        u64::try_from(bytes).unwrap_or(0).div_ceil(sector_size)
    }

    #[test]
    fn gpt_write_and_reread_via_sequential_api() {
        // ARRANGE
        let sc = sector_count(16 * 1024 * 1024, 512);
        let mut table = Table::create(sc, 512, [0xCD; 16]).expect("table must be created");
        let request = PlacementRequest {
            slot: Slot::Exact(1),
            start: Start::AfterLastUsed,
            size: Size::Lbas(1),
            alignment_lba: 1,
            type_guid: EFI_GUID,
            unique_guid: [0xAB; 16],
            attributes: 0,
            name: "EFI".to_owned(),
        };

        // ACT
        let aligned = (ALIGN_1_MIB_SECTORS + 1).next_multiple_of(ALIGN_1_MIB_SECTORS);
        let placement = request
            .place(&mut table, 512)
            .expect("placement must succeed");
        let mut buf = Vec::new();
        io::write_primary(&table, sc, &mut buf).expect("primary write must succeed");
        io::write_backup(&table, sc, &mut buf).expect("backup write must succeed");

        // ASSERT
        assert_eq!(aligned, ALIGN_1_MIB_SECTORS * 2);
        assert_eq!(placement.number, 1);
        assert_eq!(placement.partition.type_guid, EFI_GUID);
    }

    #[test]
    fn gpt_api_reports_after_last_used_overflow() {
        // ARRANGE
        let sc = sector_count(16 * 1024 * 1024, 512);
        let mut table = Table::create(sc, 512, [0xCD; 16]).expect("table must be created");
        table.set_partition(
            1,
            Partition {
                type_guid: EFI_GUID,
                unique_guid: [0xAA; 16],
                starting_lba: u64::MAX - 1,
                ending_lba: u64::MAX,
                attributes: 0,
                name: "MAX".to_owned(),
            },
        );
        let request = PlacementRequest {
            slot: Slot::Exact(2),
            start: Start::AfterLastUsed,
            size: Size::Lbas(1),
            alignment_lba: 1,
            type_guid: LINUX_FS_GUID,
            unique_guid: [0xBB; 16],
            attributes: 0,
            name: "DATA".to_owned(),
        };

        // ACT
        let result = request.place(&mut table, 512);

        // ASSERT
        assert!(
            matches!(result, Err(ParttableError::InvalidPlacement(message)) if message == "partition start LBA overflowed after last used partition")
        );
    }

    #[test]
    fn gpt_api_reports_after_partition_overflow() {
        // ARRANGE
        let sc = sector_count(16 * 1024 * 1024, 512);
        let mut table = Table::create(sc, 512, [0xCD; 16]).expect("table must be created");
        table.set_partition(
            1,
            Partition {
                type_guid: EFI_GUID,
                unique_guid: [0xAA; 16],
                starting_lba: u64::MAX - 1,
                ending_lba: u64::MAX,
                attributes: 0,
                name: "MAX".to_owned(),
            },
        );
        let request = PlacementRequest {
            slot: Slot::Exact(2),
            start: Start::AfterPartition(1),
            size: Size::Lbas(1),
            alignment_lba: 1,
            type_guid: LINUX_FS_GUID,
            unique_guid: [0xBB; 16],
            attributes: 0,
            name: "DATA".to_owned(),
        };

        // ACT
        let result = request.place(&mut table, 512);

        // ASSERT
        assert!(
            matches!(result, Err(ParttableError::InvalidPlacement(message)) if message == "partition start LBA overflowed after partition 1")
        );
    }

    #[test]
    fn gpt_api_reports_end_lba_overflow() {
        // ARRANGE
        let sc = sector_count(16 * 1024 * 1024, 512);
        let mut table = Table::create(sc, 512, [0xCD; 16]).expect("table must be created");
        let request = PlacementRequest {
            slot: Slot::Exact(1),
            start: Start::AtOrAfter(u64::MAX),
            size: Size::Lbas(2),
            alignment_lba: 1,
            type_guid: LINUX_FS_GUID,
            unique_guid: [0xBB; 16],
            attributes: 0,
            name: "DATA".to_owned(),
        };

        // ACT
        let result = request.place(&mut table, 512);

        // ASSERT
        assert!(
            matches!(result, Err(ParttableError::InvalidPlacement(message)) if message == "partition end LBA overflowed")
        );
    }
}
