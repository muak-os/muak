//! Whole-disk checks that do not need the partition list.

use blockdev::types::Disk;
use partdev::manage;

use crate::check::Check;
use crate::disk::constants::{GB, MB, MIN_DISK_SIZE};

pub(crate) fn disk_size(disk: &Disk) -> Check {
    if disk.size_bytes < MIN_DISK_SIZE {
        return Check::fail(
            "disk_size",
            format!(
                "Disk '{}' is too small ({} MB). Minimum required: {} MB",
                disk.path,
                disk.size_bytes.checked_div(MB).unwrap_or(0),
                MIN_DISK_SIZE.checked_div(MB).unwrap_or(0)
            ),
        );
    }

    kmsg::info!(
        "Disk size: {} GB ({} MB)",
        disk.size_bytes.checked_div(GB).unwrap_or(0),
        disk.size_bytes.checked_div(MB).unwrap_or(0)
    );

    Check::pass(
        "disk_size",
        format!("'{}' meets the minimum size", disk.path),
    )
}

pub(crate) fn empty(disk: &Disk, force: bool) -> Check {
    match manage::is_non_empty(&disk.path) {
        Ok(true) if !force => Check::fail(
            "empty_disk",
            format!(
                "Disk '{}' is not empty and will be overwritten. Use --force to continue.",
                disk.path
            ),
        ),
        Ok(true) => Check::pass("empty_disk", format!("'{}' will be overwritten", disk.path)),
        Ok(false) => Check::pass("empty_disk", format!("'{}' is empty", disk.path)),
        Err(e) => Check::fail("empty_disk", format!("{e:#}")),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use parttable::mbr;
    use tempfile::NamedTempFile;

    use super::*;

    fn disk(path: &str, size_bytes: u64) -> Disk {
        Disk {
            name: path.rsplit('/').next().unwrap_or_default().to_owned(),
            path: path.to_owned(),
            size_bytes,
            ..Disk::default()
        }
    }

    fn non_empty_file() -> NamedTempFile {
        let entry = mbr::PartitionEntry {
            bootable: false,
            partition_type: mbr::MBR_EFI_SYSTEM_TYPE,
            starting_lba: 1,
            size_lba: 1,
        };
        let mut file = NamedTempFile::new().expect("temp disk");
        file.write_all(&mbr::bytes(&entry)).expect("mbr");

        file
    }

    #[test]
    fn disk_size_refuses_a_disk_below_the_minimum() {
        // ARRANGE
        let disk = disk("/dev/sda", MIN_DISK_SIZE - 1);

        // ACT
        let check = disk_size(&disk);

        // ASSERT
        assert!(!check.passed);
        assert!(check.detail.contains("too small"));
    }

    #[test]
    fn disk_size_accepts_a_disk_at_the_minimum() {
        // ARRANGE
        let disk = disk("/dev/sda", MIN_DISK_SIZE);

        // ACT
        let check = disk_size(&disk);

        // ASSERT
        assert!(check.passed, "{}", check.detail);
    }

    #[test]
    fn empty_accepts_an_empty_disk() {
        // ARRANGE
        let file = NamedTempFile::new().expect("temp disk");
        let disk = disk(file.path().to_str().expect("utf8"), 0);

        // ACT
        let check = empty(&disk, false);

        // ASSERT
        assert!(check.passed, "{}", check.detail);
    }

    #[test]
    fn empty_refuses_a_non_empty_disk_without_force() {
        // ARRANGE
        let file = non_empty_file();
        let disk = disk(file.path().to_str().expect("utf8"), 0);

        // ACT
        let refused = empty(&disk, false);
        let forced = empty(&disk, true);

        // ASSERT
        assert!(!refused.passed, "an MBR must be refused");
        assert!(forced.passed, "{}", forced.detail);
    }

    #[test]
    fn empty_fails_when_the_device_cannot_be_probed() {
        // ARRANGE
        let disk = disk("/dev/muak-missing", 0);

        // ACT
        let check = empty(&disk, false);

        // ASSERT
        assert!(!check.passed);
    }
}
