//! GPT partition table persistence helpers shared by the plan executor.

use std::fs::{File, OpenOptions};
use std::io::Seek as _;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use parttable::gpt;
use parttable::gpt::table::Table;

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

/// Formats a partition device path based on disk naming convention.
pub(super) fn format_partition_name(disk: &str, partition: u32) -> String {
    if disk.contains("nvme") || disk.contains("mmcblk") {
        format!("{disk}p{partition}")
    } else {
        format!("{disk}{partition}")
    }
}

/// Logs a summary of the GPT written to a disk.
pub(super) fn verify(disk: &str) {
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

/// Returns a timestamp for deterministic-enough v7 partition GUIDs.
pub(super) fn uuid_now() -> uuid::Timestamp {
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
