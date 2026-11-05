//! GPT partition table persistence helpers shared by the plan executor.

use std::fs::{File, OpenOptions};
use std::io::Seek as _;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};
use parttable::gpt;
use parttable::gpt::table::Table;

/// Persists a GPT to an already-open disk.
pub(super) fn commit(file: &mut File, table: &Table, sector_count: u64) -> Result<()> {
    if sector_count == 0 {
        bail!("refusing to write a GPT for a disk with an unknown sector count");
    }

    file.seek(std::io::SeekFrom::Start(0))?;
    gpt::io::write_primary(table, sector_count, file)?;
    file.seek(std::io::SeekFrom::Start(
        table.backup_data_offset(sector_count),
    ))?;
    gpt::io::write_backup(table, sector_count, file)?;
    file.sync_all()?;

    Ok(())
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
    fn commit_rejects_unknown_sector_count() {
        // ARRANGE
        let mut file = tempfile::tempfile().expect("temp file");
        file.set_len(64 * 1024 * 1024).expect("resize");
        let table = Table::create(0x0002_0000, 512, [0xff; 16]).expect("table");

        // ACT
        let result = commit(&mut file, &table, 0);

        // ASSERT
        assert!(
            result.is_err(),
            "a zero sector count would place the backup GPT at byte 0"
        );
    }
}
