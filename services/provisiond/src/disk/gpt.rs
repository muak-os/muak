//! GPT partition table creation on system disks.

use std::fs::{File, OpenOptions};
use std::io::Seek as _;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use esp::EFI_GUID;
use parttable::gpt;
use parttable::gpt::layout::{PlacementRequest, Size, Slot, Start};
use parttable::gpt::partition::LINUX_FS_GUID;
use parttable::gpt::table::Table;

use super::blkpg::add_partition_blkpg;
use super::constants::{EFI_SIZE, SECTOR_SIZE, STATE_SIZE};
use super::format::wait_for_device;

/// Deterministic disk GUID used so provisioned disks share a stable identifier.
const DISK_GUID: [u8; 16] = [0xff; 16];

/// Formats a partition device path based on disk naming convention.
pub fn format_partition_name(disk: &str, partition: u32) -> String {
    if disk.contains("nvme") || disk.contains("mmcblk") {
        format!("{disk}p{partition}")
    } else {
        format!("{disk}{partition}")
    }
}

/// Persists a GPT to an already-open disk.
pub(super) fn commit(file: &mut File, table: &Table, sector_count: u64) -> Result<()> {
    file.seek(std::io::SeekFrom::Start(0))?;
    gpt::io::write_primary(table, sector_count, file)?;
    file.seek(std::io::SeekFrom::Start(
        table.backup_data_offset(sector_count),
    ))?;
    gpt::io::write_backup(table, sector_count, file)?;
    file.sync_all()?;

    Ok(())
}

fn open_disk_rw(disk: &str) -> Result<(File, u64)> {
    let mut file = OpenOptions::new().read(true).write(true).open(disk)?;
    let size = file.seek(std::io::SeekFrom::End(0))?;

    Ok((file, size))
}

fn verify(disk: &str) {
    match OpenOptions::new().read(true).open(disk) {
        Ok(mut file) => match gpt::io::read(&mut file) {
            Ok(gpt) => {
                let count = gpt.used_partitions().len();
                kmsg::info!("Verified: GPT on {} has {} used partitions", disk, count);
            }
            Err(e) => kmsg::warn!("Could not verify GPT on {}: {}", disk, e),
        },
        Err(e) => kmsg::warn!("Could not open {} for GPT verification: {}", disk, e),
    }
}

/// Creates EFI and STATE on the system disk.
pub fn create_system_partitions(disk: &str) -> Result<(String, String)> {
    kmsg::info!("Creating GPT with EFI+STATE on system disk {}", disk);

    let (mut file, disk_size) = open_disk_rw(disk)?;
    kmsg::info!(
        "System disk size: {} GB",
        disk_size.checked_div(super::constants::GB).unwrap_or(0)
    );

    let sector_count = disk_size.checked_div(SECTOR_SIZE).unwrap_or(0);
    let mut gpt = Table::create(sector_count, SECTOR_SIZE, DISK_GUID)?;

    let efi = PlacementRequest::new(
        EFI_GUID,
        *uuid::Uuid::new_v7(uuid_now()).as_bytes(),
        "EFI",
        Size::Bytes(EFI_SIZE),
    )
    .slot(Slot::Exact(1))
    .place(&mut gpt, SECTOR_SIZE)?;

    let state = PlacementRequest::new(
        LINUX_FS_GUID,
        *uuid::Uuid::new_v7(uuid_now()).as_bytes(),
        "STATE",
        Size::Bytes(STATE_SIZE),
    )
    .slot(Slot::Exact(2))
    .start(Start::AfterPartition(efi.number))
    .place(&mut gpt, SECTOR_SIZE)?;

    commit(&mut file, &gpt, sector_count)?;
    drop(file);
    verify(disk);

    add_partition_blkpg(
        disk,
        efi.number,
        efi.partition.starting_lba,
        efi.partition.ending_lba,
    )?;
    add_partition_blkpg(
        disk,
        state.number,
        state.partition.starting_lba,
        state.partition.ending_lba,
    )?;

    kmsg::info!("System partitions registered on {}", disk);

    let efi_part = format_partition_name(disk, 1);
    let state_part = format_partition_name(disk, 2);

    wait_for_device(&efi_part)?;

    Ok((efi_part, state_part))
}

/// Creates a DATA partition filling the remaining space on `disk`.
pub fn create_data_partition(disk: &str) -> Result<String> {
    let (mut file, disk_size) = open_disk_rw(disk)?;
    kmsg::info!(
        "Data disk size: {} GB",
        disk_size.checked_div(super::constants::GB).unwrap_or(0)
    );

    file.seek(std::io::SeekFrom::Start(0))?;
    let (mut gpt, sector_count, start) = if let Ok(existing) = gpt::io::read(&mut file) {
        let sc = disk_size.checked_div(SECTOR_SIZE).unwrap_or(0);
        kmsg::info!("Appending DATA on existing GPT on {}", disk);
        (existing, sc, Start::AfterLastUsed)
    } else {
        file.seek(std::io::SeekFrom::Start(0))?;
        let sc = disk_size.checked_div(SECTOR_SIZE).unwrap_or(0);
        let gpt = Table::create(sc, SECTOR_SIZE, DISK_GUID)?;
        kmsg::info!("Creating new GPT with DATA as partition 1 on {}", disk);
        (gpt, sc, Start::FirstUsable)
    };

    let data = PlacementRequest::new(
        LINUX_FS_GUID,
        *uuid::Uuid::new_v7(uuid_now()).as_bytes(),
        "DATA",
        Size::FillToLastUsable,
    )
    .start(start)
    .place(&mut gpt, SECTOR_SIZE)?;

    commit(&mut file, &gpt, sector_count)?;
    drop(file);
    verify(disk);

    add_partition_blkpg(
        disk,
        data.number,
        data.partition.starting_lba,
        data.partition.ending_lba,
    )?;

    kmsg::info!("Data partition {} registered on {}", data.number, disk);

    let data_part = format_partition_name(disk, data.number);
    wait_for_device(&data_part)?;

    Ok(data_part)
}

fn uuid_now() -> uuid::Timestamp {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();

    uuid::Timestamp::from_unix(uuid::NoContext, dur.as_secs(), dur.subsec_nanos())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_partition_name_nvme_uses_p_separator() {
        // ARRANGE
        let disk = "/dev/nvme0n1";

        // ACT
        let name = format_partition_name(disk, 1);

        // ASSERT
        assert_eq!(name, "/dev/nvme0n1p1");
    }

    #[test]
    fn format_partition_name_mmcblk_uses_p_separator() {
        // ARRANGE
        let disk = "/dev/mmcblk0";

        // ACT
        let name = format_partition_name(disk, 2);

        // ASSERT
        assert_eq!(name, "/dev/mmcblk0p2");
    }

    #[test]
    fn format_partition_name_sda_uses_no_separator() {
        // ARRANGE
        let disk = "/dev/sda";

        // ACT
        let name = format_partition_name(disk, 3);

        // ASSERT
        assert_eq!(name, "/dev/sda3");
    }

    #[test]
    fn format_partition_name_vda_uses_no_separator() {
        // ARRANGE
        let disk = "/dev/vda";

        // ACT
        let name = format_partition_name(disk, 1);

        // ASSERT
        assert_eq!(name, "/dev/vda1");
    }
}
