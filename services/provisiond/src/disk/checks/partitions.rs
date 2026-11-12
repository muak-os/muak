//! Partition safety checks shared by the install and wipe paths.

use blockdev::types::Disk;

use crate::check::Check;
use crate::disk::roles;

pub(crate) fn read_only(disk: &Disk) -> Check {
    if disk.read_only {
        return Check::fail(
            "read_only",
            format!("Disk '{}' is read-only and cannot be wiped", disk.path),
        );
    }

    Check::pass("read_only", format!("'{}' is writable", disk.path))
}

pub(crate) fn mounted(disk: &Disk, allow: bool) -> Check {
    let mounted = disk
        .partitions
        .iter()
        .map(|partition| partition.mount_point.as_str())
        .find(|mount_point| !mount_point.is_empty());

    match mounted {
        Some(mount_point) if !allow => Check::fail(
            "mounted_partitions",
            format!(
                "Disk '{}' is mounted at '{mount_point}', unmount it or pass --force / --allow-system-disk to override",
                disk.path
            ),
        ),
        Some(_) => Check::pass(
            "mounted_partitions",
            format!("'{}' is mounted and will be unmounted", disk.path),
        ),
        None => Check::pass(
            "mounted_partitions",
            format!("'{}' has no mounted partitions", disk.path),
        ),
    }
}

pub(crate) fn muak(disk: &Disk, allow: bool) -> Check {
    if !roles::is_system(disk) {
        return Check::pass(
            "muak_partitions",
            format!("'{}' has no Muak STATE partition", disk.path),
        );
    }

    if allow {
        return Check::pass(
            "muak_partitions",
            format!(
                "'{}' carries the Muak STATE partition and the override was given",
                disk.path
            ),
        );
    }

    Check::fail(
        "muak_partitions",
        format!(
            "Disk '{}' carries the Muak STATE partition, pass --force / --allow-system-disk to overwrite it",
            disk.path
        ),
    )
}

#[cfg(test)]
mod tests {
    use ::disk::role::Role;
    use blockdev::types::Partition;
    use tempfile::NamedTempFile;

    use super::super::for_disk;
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

    fn partition(number: u32, mount_point: &str, gpt_name: &str) -> Partition {
        Partition {
            number,
            name: format!("sda{number}"),
            path: format!("/dev/sda{number}"),
            gpt_name: gpt_name.to_owned(),
            mount_point: mount_point.to_owned(),
            ..Partition::default()
        }
    }

    fn install(disk: &Disk, name: &str) -> bool {
        for_disk(disk, false)
            .into_iter()
            .find(|check| check.name == name)
            .expect("install check")
            .passed
    }

    #[test]
    fn read_only_refuses_a_read_only_disk() {
        // ARRANGE
        let mut disk = disk("/dev/sda");
        disk.read_only = true;

        // ACT
        let check = read_only(&disk);

        // ASSERT
        assert!(!check.passed);
    }

    #[test]
    fn a_mounted_partition_fails_the_install_list_and_the_wipe_check() {
        // ARRANGE
        let file = NamedTempFile::new().expect("temp disk");
        let mut disk = disk(file.path().to_str().expect("utf8"));
        disk.partitions.push(partition(1, "/mnt/data", ""));

        // ACT
        let install_passed = install(&disk, "mounted_partitions");
        let wipe = mounted(&disk, false);

        // ASSERT
        assert!(!install_passed, "the install list must refuse the mount");
        assert!(!wipe.passed, "the wipe check must refuse the mount");
        assert!(mounted(&disk, true).passed, "the override must pass");
    }

    #[test]
    fn state_partitions_fail_the_muak_check_on_both_paths() {
        // ARRANGE
        let state_file = NamedTempFile::new().expect("state disk");
        let mut state = disk(state_file.path().to_str().expect("utf8"));
        state
            .partitions
            .push(partition(1, "", Role::State.gpt_name()));

        // ACT
        let state_install = install(&state, "muak_partitions");
        let state_wipe = muak(&state, false);

        // ASSERT
        assert!(!state_install, "the install list must refuse STATE");
        assert!(!state_wipe.passed, "the wipe check must refuse STATE");
    }

    #[test]
    fn a_disk_without_muak_partitions_passes_both_paths() {
        // ARRANGE
        let file = NamedTempFile::new().expect("temp disk");
        let disk = disk(file.path().to_str().expect("utf8"));

        // ACT
        let install_passed = install(&disk, "muak_partitions");
        let wipe = muak(&disk, false);

        // ASSERT
        assert!(install_passed, "the install list must accept the disk");
        assert!(wipe.passed, "the wipe check must accept the disk");
        assert!(for_disk(&disk, false).iter().all(|check| check.passed));
    }
}
