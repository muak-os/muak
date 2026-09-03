//! Disk state queries and partition deletion.

use std::fs::{File, OpenOptions};

use anyhow::Result;
use parttable::gpt;
use parttable::mbr::read;

use super::blkpg::delete_partition_blkpg;
use super::constants::SECTOR_SIZE;
use super::gpt::commit;

/// Returns `true` when `disk` contains GPT or MBR partition state.
pub fn disk_is_non_empty(disk: &str) -> Result<bool> {
    if gpt::io::read(&mut OpenOptions::new().read(true).open(disk)?).is_ok() {
        return Ok(true);
    }

    match read(&mut OpenOptions::new().read(true).open(disk)?) {
        Ok(entries) => Ok(entries.iter().any(Option::is_some)),
        Err(_) => Ok(false),
    }
}

/// Returns `true` when `disk` already has a Muak STATE partition installed.
pub fn has_state_partition(disk: &str) -> Result<bool> {
    let mut file = File::open(disk)?;
    match gpt::io::read(&mut file) {
        Ok(gpt) => Ok(gpt
            .used_partitions()
            .into_iter()
            .any(|(_, partition)| partition.name == "STATE")),
        Err(_) => Ok(false),
    }
}

/// Deletes the specified partitions from the GPT and removes their device nodes from the kernel.
pub fn delete_partitions(disk: &str, partitions: &[u32]) -> Result<()> {
    kmsg::info!("Deleting partitions {:?} from GPT on {}", partitions, disk);

    let mut file = OpenOptions::new().read(true).write(true).open(disk)?;
    let mut gpt = gpt::io::read(&mut file)?;

    for &partition_num in partitions {
        if !gpt.is_partition_used(partition_num) {
            kmsg::warn!("Partition {} is already unused, skipping", partition_num);
            continue;
        }

        gpt.remove_partition(partition_num)?;
        kmsg::info!("Removed partition {} from GPT", partition_num);
    }

    let sc = file
        .metadata()
        .map_or(0, |meta| meta.len())
        .checked_div(SECTOR_SIZE)
        .unwrap_or(0);
    commit(&mut file, &gpt, sc)?;
    drop(file);

    for &partition_num in partitions {
        delete_partition_blkpg(disk, partition_num)?;
    }

    kmsg::info!("Partitions deleted successfully");

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;
    use std::io::Write as _;

    use parttable::gpt::partition::{LINUX_FS_GUID, Partition};
    use parttable::gpt::table::Table;
    use parttable::mbr::{PartitionEntry, protective_size_lba, write};
    use tempfile::NamedTempFile;

    use super::*;

    /// Creates a blank disk image of the given size as a named temp file.
    fn blank_disk(size: u64) -> NamedTempFile {
        let mut file = NamedTempFile::new().expect("temp file");
        file.write_all(&vec![0_u8; usize::try_from(size).unwrap_or(0)])
            .expect("write");

        file
    }

    /// Writes a GPT with the given partition names to a temp disk file.
    fn disk_with_partitions(names: &[&str]) -> NamedTempFile {
        const DISK_SIZE: u64 = 64 * 1024 * 1024;
        let disk = blank_disk(DISK_SIZE);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(disk.path())
            .expect("open");
        let sector_count = file
            .metadata()
            .expect("metadata")
            .len()
            .checked_div(512)
            .unwrap_or(0);
        let mut gpt = Table::create(sector_count, 512, [0xff; 16]).expect("new gpt");
        for (i, &name) in names.iter().enumerate() {
            let mut guid = [0_u8; 16];
            let index = u64::try_from(i).unwrap_or(0);
            guid[0] = u8::try_from(index.saturating_add(1)).unwrap_or(0);
            let starting_lba = index.saturating_mul(4096).saturating_add(2048);
            gpt.set_partition(
                u32::try_from(index.saturating_add(1)).unwrap_or(0),
                Partition {
                    type_guid: LINUX_FS_GUID,
                    unique_guid: guid,
                    starting_lba,
                    ending_lba: starting_lba.saturating_add(4095),
                    attributes: 0,
                    name: name.into(),
                },
            );
        }
        let sc = file
            .metadata()
            .expect("metadata")
            .len()
            .checked_div(512)
            .unwrap_or(0);
        gpt::io::write_primary(&gpt, sc, &mut file).expect("write primary");
        gpt::io::write_backup(&gpt, sc, &mut file).expect("write backup");

        disk
    }

    fn disk_with_contents(bytes: &[u8]) -> NamedTempFile {
        let mut file = NamedTempFile::new().expect("temp file");
        file.write_all(bytes).expect("write");
        file
    }

    #[test]
    fn has_state_partition_returns_false_for_blank_disk() {
        // ARRANGE
        let disk = blank_disk(64 * 1024 * 1024);

        // ACT
        let result =
            has_state_partition(disk.path().to_str().expect("path")).expect("should succeed");

        // ASSERT
        assert!(!result);
    }

    #[test]
    fn has_state_partition_returns_false_for_efi_only_disk() {
        // ARRANGE
        let disk = disk_with_partitions(&["EFI"]);

        // ACT
        let result =
            has_state_partition(disk.path().to_str().expect("path")).expect("should succeed");

        // ASSERT
        assert!(!result, "EFI-only disk must not be treated as installed");
    }

    #[test]
    fn has_state_partition_returns_true_for_state_partition() {
        // ARRANGE
        let disk = disk_with_partitions(&["EFI", "STATE"]);

        // ACT
        let result =
            has_state_partition(disk.path().to_str().expect("path")).expect("should succeed");

        // ASSERT
        assert!(
            result,
            "disk with STATE partition must be detected as installed"
        );
    }

    #[test]
    fn has_state_partition_returns_true_for_state_only_disk() {
        // ARRANGE
        let disk = disk_with_partitions(&["STATE"]);

        // ACT
        let result =
            has_state_partition(disk.path().to_str().expect("path")).expect("should succeed");

        // ASSERT
        assert!(result);
    }

    #[test]
    fn has_state_partition_returns_false_for_unrelated_partitions() {
        // ARRANGE
        let disk = disk_with_partitions(&["BOOT", "ROOT", "SWAP"]);

        // ACT
        let result =
            has_state_partition(disk.path().to_str().expect("path")).expect("should succeed");

        // ASSERT
        assert!(!result, "non-Muak partitions must not block installation");
    }

    #[test]
    fn protective_mbr_size_for_large_disk_is_clamped() {
        // ARRANGE
        let disk_size = (u64::from(u32::MAX) + 100) * SECTOR_SIZE;

        // ACT
        let result = protective_size_lba(disk_size, SECTOR_SIZE);

        // ASSERT
        assert_eq!(result, u32::MAX);
    }

    #[test]
    fn disk_is_non_empty_returns_false_for_zeroed_disk() {
        // ARRANGE
        let disk = disk_with_contents(&[0; 4096]);

        // ACT
        let result = disk_is_non_empty(disk.path().to_str().expect("path"))
            .expect("disk emptiness check should succeed");

        // ASSERT
        assert!(!result);
    }

    #[test]
    fn disk_is_non_empty_returns_true_for_gpt_disk() {
        // ARRANGE
        let disk = NamedTempFile::new().expect("temp file");
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(disk.path())
            .expect("open");
        file.set_len(64 * 1024 * 1024).expect("resize");
        let sector_count = file
            .metadata()
            .expect("metadata")
            .len()
            .checked_div(512)
            .unwrap_or(0);
        let gpt = Table::create(sector_count, 512, [0xff; 16]).expect("new gpt");
        gpt::io::write_primary(&gpt, sector_count, &mut file).expect("write primary gpt");
        gpt::io::write_backup(&gpt, sector_count, &mut file).expect("write backup gpt");

        // ACT
        let result = disk_is_non_empty(disk.path().to_str().expect("path"))
            .expect("disk emptiness check should succeed");

        // ASSERT
        assert!(result);
    }

    #[test]
    fn disk_is_non_empty_returns_true_for_mbr_disk() {
        // ARRANGE
        let disk = NamedTempFile::new().expect("temp file");
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(disk.path())
            .expect("open");
        file.set_len(4096).expect("resize");
        write(
            &mut file,
            &[
                Some(PartitionEntry {
                    bootable: false,
                    partition_type: 0x83,
                    starting_lba: 1,
                    size_lba: 1,
                }),
                None,
                None,
                None,
            ],
        )
        .expect("write mbr");

        // ACT
        let result = disk_is_non_empty(disk.path().to_str().expect("path"))
            .expect("disk emptiness check should succeed");

        // ASSERT
        assert!(result);
    }
}
