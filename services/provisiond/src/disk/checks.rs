//! Disk safety checks shared by the dry-run plan and the installer.

use std::path::Path;

use super::attrs::{MOUNTS_PATH, Mount, read_mounts};
use super::mount::get_disk_mounts;
use super::{
    Role, disk_is_non_empty, find_partition_device, has_state_partition, parent_disk,
    validate_block_device, validate_disk_size,
};
use crate::check::Check;

pub(crate) fn for_device(device: &str, force: bool) -> Vec<Check> {
    let exists = device_exists(device);
    if !exists.passed {
        return vec![exists];
    }

    vec![
        exists,
        block_device(device),
        disk_size(device),
        not_mounted(device, force),
        no_muak_install(device, force),
        empty_disk(device, force),
    ]
}

pub(crate) fn system_disk_in_use(device: &str, running: Option<&str>, allow: bool) -> Check {
    match running {
        Some(running) if running == device && allow => Check::pass(
            "system_disk_in_use",
            format!("'{device}' hosts the running system and --allow-system-disk was given"),
        ),
        Some(running) if running == device => Check::fail(
            "system_disk_in_use",
            format!(
                "Disk '{device}' hosts the running system. Use --allow-system-disk to destroy it."
            ),
        ),
        Some(running) => Check::pass(
            "system_disk_in_use",
            format!("'{device}' does not host the running system on '{running}'"),
        ),
        None => Check::pass(
            "system_disk_in_use",
            "The running system disk could not be determined".to_owned(),
        ),
    }
}

/// Returns the device path of the disk hosting the running system.
pub(crate) fn running_system_disk() -> Option<String> {
    if let Some(partition) = find_partition_device(Role::State)
        && let Some(disk) = parent_disk(&partition)
    {
        return Some(device_path(&disk));
    }

    let mounts = read_mounts(Path::new(MOUNTS_PATH));
    mount_disk(mounts.iter().find(|mount| mount.mount_point == "/"))
}

fn device_exists(device: &str) -> Check {
    if !Path::new(device).exists() {
        return Check::fail("device_exists", format!("Disk '{device}' does not exist"));
    }

    Check::pass("device_exists", format!("'{device}' exists"))
}

fn block_device(device: &str) -> Check {
    match validate_block_device(device) {
        Ok(()) => Check::pass("block_device", format!("'{device}' is a block device")),
        Err(e) => Check::fail("block_device", format!("{e:#}")),
    }
}

fn disk_size(device: &str) -> Check {
    match validate_disk_size(device) {
        Ok(()) => Check::pass("disk_size", format!("'{device}' meets the minimum size")),
        Err(e) => Check::fail("disk_size", format!("{e:#}")),
    }
}

fn not_mounted(device: &str, force: bool) -> Check {
    let mounted = get_disk_mounts(device);
    let first = mounted.first();

    match first {
        Some(mount) if !force => Check::fail(
            "not_mounted",
            format!(
                "Disk '{device}' is mounted at {}. Use --force to unmount automatically.",
                mount.mount_point
            ),
        ),
        Some(_) => Check::pass(
            "not_mounted",
            format!("'{device}' is mounted and will be unmounted"),
        ),
        None => Check::pass("not_mounted", format!("'{device}' is not mounted")),
    }
}

fn no_muak_install(device: &str, force: bool) -> Check {
    match has_state_partition(device) {
        Ok(true) if !force => Check::fail(
            "no_muak_install",
            format!(
                "Disk '{device}' already has a Muak installation (STATE partition found). \
                 Use --force to overwrite."
            ),
        ),
        Ok(true) => Check::pass(
            "no_muak_install",
            format!("'{device}' has a Muak installation that will be overwritten"),
        ),
        Ok(false) => Check::pass(
            "no_muak_install",
            format!("'{device}' has no Muak installation"),
        ),
        Err(e) => Check::fail("no_muak_install", format!("{e:#}")),
    }
}

fn empty_disk(device: &str, force: bool) -> Check {
    match disk_is_non_empty(device) {
        Ok(true) if !force => Check::fail(
            "empty_disk",
            format!(
                "Disk '{device}' is not empty and will be overwritten. Use --force to continue."
            ),
        ),
        Ok(true) => Check::pass("empty_disk", format!("'{device}' will be overwritten")),
        Ok(false) => Check::pass("empty_disk", format!("'{device}' is empty")),
        Err(e) => Check::fail("empty_disk", format!("{e:#}")),
    }
}

fn mount_disk(mount: Option<&Mount>) -> Option<String> {
    let disk = parent_disk(&mount?.device)?;
    let device = device_path(&disk);

    Path::new(&device).exists().then_some(device)
}

fn device_path(disk_name: &str) -> String {
    format!("/dev/{disk_name}")
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use tempfile::NamedTempFile;

    use super::*;

    #[test]
    fn device_checks_short_circuit_when_the_device_is_missing() {
        // ARRANGE
        let missing = "/dev/muak-missing";

        // ACT
        let checks = for_device(missing, false);

        // ASSERT
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].name, "device_exists");
        assert!(!checks[0].passed);
    }

    #[test]
    fn device_checks_refuse_a_regular_file() {
        // ARRANGE
        let mut file = NamedTempFile::new().expect("temp file");
        file.write_all(b"not a disk").expect("write");

        // ACT
        let checks = for_device(file.path().to_str().expect("utf8"), false);

        // ASSERT
        let block_device = checks
            .iter()
            .find(|check| check.name == "block_device")
            .expect("block device check");
        assert!(!block_device.passed, "{}", block_device.detail);
        assert!(block_device.detail.contains("not a block device"));
    }

    #[test]
    fn system_disk_in_use_refuses_the_running_disk() {
        // ARRANGE
        let running = Some("/dev/sda");

        // ACT
        let check = system_disk_in_use("/dev/sda", running, false);

        // ASSERT
        assert!(!check.passed);
        assert!(check.detail.contains("--allow-system-disk"));
    }

    #[test]
    fn system_disk_in_use_allows_the_running_disk_when_flagged() {
        // ARRANGE
        let running = Some("/dev/sda");

        // ACT
        let check = system_disk_in_use("/dev/sda", running, true);

        // ASSERT
        assert!(check.passed, "{}", check.detail);
        assert!(check.detail.contains("--allow-system-disk"));
    }

    #[test]
    fn system_disk_in_use_ignores_other_disks() {
        // ARRANGE
        let running = Some("/dev/nvme0n1");

        // ACT
        let check = system_disk_in_use("/dev/sda", running, false);

        // ASSERT
        assert!(check.passed, "{}", check.detail);
    }

    #[test]
    fn system_disk_in_use_passes_when_the_running_disk_is_unknown() {
        // ARRANGE
        let running: Option<&str> = None;

        // ACT
        let check = system_disk_in_use("/dev/sda", running, false);

        // ASSERT
        assert!(check.passed, "{}", check.detail);
        assert!(check.detail.contains("could not be determined"));
    }
}
