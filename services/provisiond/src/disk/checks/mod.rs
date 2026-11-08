//! Disk safety checks shared by the dry-run plan and the installer.

mod device;
pub(crate) mod partitions;

use std::path::Path;

use ::disk::role::Role;
use blockdev::kernel::Kernel;
use blockdev::mounts::Mount;
use blockdev::types::Disk;

use super::roles;
use crate::check::Check;

/// Runs the wipe-safe checks shared by the install dry-run and the installer.
pub(crate) fn for_disk(disk: &Disk, force: bool) -> Vec<Check> {
    vec![
        device::disk_size(disk),
        partitions::mounted(disk, force),
        partitions::muak(disk, force),
        device::empty(disk, force),
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
///
/// The kernel view is supplied by the caller.
pub(crate) fn running_system_disk<K: Kernel>(kernel: &K) -> Option<String> {
    if let Some(partition) = roles::device(Role::State)
        && let Some(disk) = kernel.parent_disk(&partition)
    {
        return Some(device_path(&disk));
    }

    let mounts = kernel.mounts();
    mount_disk(kernel, mounts.iter().find(|mount| mount.mount_point == "/"))
}

fn mount_disk<K: Kernel>(kernel: &K, mount: Option<&Mount>) -> Option<String> {
    let disk = kernel.parent_disk(&mount?.device)?;
    let path = device_path(&disk);

    Path::new(&path).exists().then_some(path)
}

fn device_path(disk_name: &str) -> String {
    format!("/dev/{disk_name}")
}

#[cfg(test)]
mod tests {
    use blockdev::types::Disk;
    use tempfile::NamedTempFile;

    use super::*;
    use crate::disk::constants::MIN_DISK_SIZE;

    fn disk(path: &str) -> Disk {
        Disk {
            name: path.rsplit('/').next().unwrap_or_default().to_owned(),
            path: path.to_owned(),
            size_bytes: MIN_DISK_SIZE,
            ..Disk::default()
        }
    }

    #[test]
    fn install_checks_cover_size_mounts_muak_and_content() {
        // ARRANGE
        let file = NamedTempFile::new().expect("temp disk");
        let disk = disk(file.path().to_str().expect("utf8"));

        // ACT
        let checks = for_disk(&disk, false);

        // ASSERT
        let names: Vec<&str> = checks.iter().map(|check| check.name).collect();
        assert_eq!(
            names,
            [
                "disk_size",
                "mounted_partitions",
                "muak_partitions",
                "empty_disk"
            ]
        );
        assert!(checks.iter().all(|check| check.passed));
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
