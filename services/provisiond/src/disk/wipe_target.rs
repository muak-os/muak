//! Validation of a user supplied disk identifier before a wipe.

use anyhow::{Result, bail};

use super::preflight::running_system_disk;
use super::types::DiskInfo;

/// A disk cleared for wiping together with the partitions it carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Target {
    /// Kernel device path of the whole disk.
    pub path: String,
    /// Whether the disk carries the Muak STATE partition.
    pub is_system: bool,
    /// Whether the disk carries the Muak DATA partition.
    pub is_data: bool,
    /// Partition numbers currently present on the disk.
    pub partitions: Vec<u32>,
}

/// Resolves `id` to a whole disk and enforces the wipe safety rules.
pub(crate) fn resolve(disks: &[DiskInfo], id: &str, allow_system_disk: bool) -> Result<Target> {
    let name = id.strip_prefix("/dev/").unwrap_or(id);
    let disk = find(disks, name)?;

    if disk.read_only {
        bail!("disk '{}' is read-only", disk.path);
    }

    if let Some(running) = running_system_disk()
        && running == disk.path
        && !allow_system_disk
    {
        bail!(
            "disk '{}' hosts the running system, pass --allow-system-disk to wipe it",
            disk.path
        );
    }

    let mounted = mounted_partition(disk);
    if !mounted.is_empty() && !allow_system_disk {
        bail!(
            "disk '{}' is mounted at '{mounted}', unmount it or pass --allow-system-disk",
            disk.path
        );
    }

    if (disk.is_system || disk.is_data) && !allow_system_disk {
        bail!(
            "disk '{}' carries the Muak {} partition, pass --allow-system-disk to wipe it",
            disk.path,
            role(disk)
        );
    }

    Ok(Target {
        path: disk.path.clone(),
        is_system: disk.is_system,
        is_data: disk.is_data,
        partitions: disk
            .partitions
            .iter()
            .map(|partition| partition.number)
            .collect(),
    })
}

fn find<'a>(disks: &'a [DiskInfo], name: &str) -> Result<&'a DiskInfo> {
    if let Some(disk) = disks.iter().find(|disk| disk.name == name) {
        return Ok(disk);
    }

    if disks.iter().any(|disk| is_partition_of(name, &disk.name)) {
        bail!("'{name}' is a partition, wipe the whole disk instead");
    }

    bail!("disk '{name}' not found")
}

fn is_partition_of(name: &str, disk: &str) -> bool {
    let Some(suffix) = name.strip_prefix(disk) else {
        return false;
    };

    if suffix.chars().all(|character| character.is_ascii_digit()) {
        return !suffix.is_empty();
    }

    suffix.strip_prefix('p').is_some_and(|number| {
        !number.is_empty() && number.chars().all(|character| character.is_ascii_digit())
    })
}

fn mounted_partition(disk: &DiskInfo) -> &str {
    disk.partitions
        .iter()
        .map(|partition| partition.mount_point.as_str())
        .find(|mount_point| !mount_point.is_empty())
        .unwrap_or("")
}

fn role(disk: &DiskInfo) -> &'static str {
    match (disk.is_system, disk.is_data) {
        (true, true) => "STATE and DATA",
        (true, false) => "STATE",
        (false, _) => "DATA",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::PartitionInfo;

    fn disk(name: &str) -> DiskInfo {
        DiskInfo {
            name: name.to_owned(),
            path: format!("/dev/{name}"),
            size_bytes: 0,
            model: String::new(),
            serial: String::new(),
            wwid: String::new(),
            bus_path: String::new(),
            subsystem: String::new(),
            rotational: false,
            cdrom: false,
            removable: false,
            read_only: false,
            symlinks: Vec::new(),
            is_system: false,
            is_data: false,
            partitions: Vec::new(),
        }
    }

    fn partition(number: u32, mount_point: &str) -> PartitionInfo {
        PartitionInfo {
            number,
            start_sector: 0,
            size_bytes: 0,
            name: format!("sda{number}"),
            path: format!("/dev/sda{number}"),
            fstype: String::new(),
            gpt_name: String::new(),
            mount_point: mount_point.to_owned(),
            in_use: false,
        }
    }

    #[test]
    fn resolve_accepts_a_plain_whole_disk_by_name_or_path() {
        // ARRANGE
        let disks = vec![disk("sda")];

        // ACT
        let by_name = resolve(&disks, "sda", false).expect("name");
        let by_path = resolve(&disks, "/dev/sda", false).expect("path");

        // ASSERT
        assert_eq!(by_name.path, "/dev/sda");
        assert_eq!(by_path, by_name);
    }

    #[test]
    fn resolve_rejects_partitions() {
        // ARRANGE
        let disks = vec![disk("sda"), disk("nvme0n1")];

        // ACT
        let sata = resolve(&disks, "sda1", true);
        let nvme = resolve(&disks, "/dev/nvme0n1p2", true);

        // ASSERT
        sata.expect_err("sda1 is a partition");
        nvme.expect_err("nvme0n1p2 is a partition");
    }

    #[test]
    fn resolve_rejects_read_only_disks() {
        // ARRANGE
        let mut disks = vec![disk("sda")];
        disks[0].read_only = true;

        // ACT
        let result = resolve(&disks, "sda", true);

        // ASSERT
        result.expect_err("read-only disks cannot be wiped");
    }

    #[test]
    fn resolve_rejects_mounted_disks_without_allow_system_disk() {
        // ARRANGE
        let mut disks = vec![disk("sda")];
        disks[0].partitions.push(partition(1, "/mnt/data"));

        // ACT
        let refused = resolve(&disks, "sda", false);
        let allowed = resolve(&disks, "sda", true);

        // ASSERT
        refused.expect_err("mounted disks need explicit approval");
        allowed.expect("explicit approval allows mounted disks");
    }

    #[test]
    fn resolve_refuses_system_and_data_disks_without_allow_system_disk() {
        // ARRANGE
        let mut system = disk("sda");
        system.is_system = true;
        let mut data = disk("sdb");
        data.is_data = true;
        let disks = vec![system, data];

        // ACT
        let system_result = resolve(&disks, "sda", false);
        let data_result = resolve(&disks, "sdb", false);

        // ASSERT
        system_result.expect_err("STATE disks need explicit approval");
        data_result.expect_err("DATA disks need explicit approval");
    }

    #[test]
    fn resolve_allows_system_disks_when_explicitly_allowed() {
        // ARRANGE
        let mut system = disk("sda");
        system.is_system = true;
        system.is_data = true;
        system.partitions.push(partition(1, ""));

        // ACT
        let target = resolve(&[system], "sda", true).expect("allowed");

        // ASSERT
        assert!(target.is_system);
        assert!(target.is_data);
        assert_eq!(target.partitions, [1]);
    }

    #[test]
    fn resolve_unknown_disk_is_an_error() {
        // ARRANGE
        let disks = vec![disk("sda")];

        // ACT
        let result = resolve(&disks, "sdz", false);

        // ASSERT
        result.expect_err("unknown disks are refused");
    }
}
