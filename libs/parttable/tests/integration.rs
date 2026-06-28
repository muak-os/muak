//! Integration tests for the public API of the parttable library.

#[cfg(test)]
mod tests {
    use parttable::{
        error::ParttableError,
        gpt::{
            table::{Table, align_up_lba},
            types::{
                ALIGN_1_MIB_SECTORS, EFI_GUID, LINUX_FS_GUID, Partition, PlacementRequest, Size,
                Slot, Start,
            },
        },
    };

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
        let aligned = align_up_lba(ALIGN_1_MIB_SECTORS + 1, ALIGN_1_MIB_SECTORS);
        let placement = table
            .place_partition(request, 512)
            .expect("placement must succeed");
        let mut buf = Vec::new();
        table
            .write_primary_to(sc, &mut buf)
            .expect("primary write must succeed");
        table
            .write_backup_to(sc, &mut buf)
            .expect("backup write must succeed");

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
        let result = table.place_partition(request, 512);

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
        let result = table.place_partition(request, 512);

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
        let result = table.place_partition(request, 512);

        // ASSERT
        assert!(
            matches!(result, Err(ParttableError::InvalidPlacement(message)) if message == "partition end LBA overflowed")
        );
    }
}
