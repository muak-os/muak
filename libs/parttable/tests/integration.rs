//! Integration tests for the public API of the parttable library.

use std::io::Cursor;

use parttable::{
    error::ParttableError,
    gpt::{
        table::{Table, align_up_lba},
        types::{
            ALIGN_1_MIB_SECTORS, EFI_GUID, LINUX_FS_GUID, Partition, PlacementRequest, Size, Slot,
            Start,
        },
    },
    mbr::{
        io::{protective_size_lba, write_entry, write_protective, write_signature},
        types::{
            MBR_BOOT_SIGNATURE, MBR_EFI_SYSTEM_TYPE, MBR_PARTITION_ENTRY_OFFSET,
            MBR_PROTECTIVE_GPT_TYPE, MbrPartitionEntry,
        },
    },
};

fn blank_disk(size: usize) -> Cursor<Vec<u8>> {
    Cursor::new(vec![0_u8; size])
}

#[test]
fn public_gpt_api_wrappers_behave_as_expected() {
    // ARRANGE
    let mut disk = blank_disk(16 * 1024 * 1024);
    let mut table = Table::create(&mut disk, 512, [0xCD; 16]).expect("table must be created");
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
    table.write(&mut disk).expect("table must be written");
    disk.set_position(0);
    let reread = Table::read(&mut disk).expect("table must be read back");

    // ASSERT
    assert_eq!(aligned, ALIGN_1_MIB_SECTORS * 2);
    assert_eq!(placement.number, 1);
    assert_eq!(placement.partition.type_guid, EFI_GUID);
    assert_eq!(
        reread.partition(1).expect("partition must exist").name,
        "EFI"
    );
}

#[test]
fn public_mbr_api_wrappers_write_expected_bytes() {
    // ARRANGE
    let mut disk = blank_disk(512);
    let entry = MbrPartitionEntry {
        bootable: true,
        partition_type: MBR_EFI_SYSTEM_TYPE,
        starting_lba: 1,
        size_lba: 7,
    };

    // ACT
    let zero_sector_size = protective_size_lba(4096, 0);
    write_entry(&mut disk, 0, &entry).expect("entry write must succeed");
    write_signature(&mut disk).expect("signature write must succeed");
    let data = disk.into_inner();
    let mut protective = Cursor::new(vec![0_u8; 512]);
    write_protective(&mut protective, 4096, 512).expect("protective MBR write must succeed");
    let protective_data = protective.into_inner();

    // ASSERT
    assert_eq!(zero_sector_size, 0);
    assert_eq!(data[MBR_PARTITION_ENTRY_OFFSET as usize], 0x80);
    assert_eq!(data[450], MBR_EFI_SYSTEM_TYPE);
    assert_eq!(data[510..512], MBR_BOOT_SIGNATURE);
    assert_eq!(protective_data[450], MBR_PROTECTIVE_GPT_TYPE);
}

#[test]
fn public_gpt_api_reports_after_last_used_overflow() {
    // ARRANGE
    let mut disk = blank_disk(16 * 1024 * 1024);
    let mut table = Table::create(&mut disk, 512, [0xCD; 16]).expect("table must be created");
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
fn public_gpt_api_reports_after_partition_overflow() {
    // ARRANGE
    let mut disk = blank_disk(16 * 1024 * 1024);
    let mut table = Table::create(&mut disk, 512, [0xCD; 16]).expect("table must be created");
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
fn public_gpt_api_reports_end_lba_overflow() {
    // ARRANGE
    let mut disk = blank_disk(16 * 1024 * 1024);
    let mut table = Table::create(&mut disk, 512, [0xCD; 16]).expect("table must be created");
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
