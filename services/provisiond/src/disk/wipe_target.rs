//! Validation of a user supplied disk identifier before a wipe.

use anyhow::{Result, bail};

use super::foreign::{self, Paths};
use super::preflight::{self, Check};
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

/// The resolved wipe target, its inventory record and its safety checks.
pub(crate) struct Validation {
    pub target: Target,
    pub disk: DiskInfo,
    pub checks: Vec<Check>,
}

/// Resolves `id` to a whole disk and returns the wipe safety checks.
pub(crate) fn validate(
    disks: &[DiskInfo],
    id: &str,
    allow_system_disk: bool,
) -> Result<Validation> {
    validate_in(disks, id, allow_system_disk, &Paths::system())
}

/// Resolves `id` against an injectable sysfs root and swap table.
pub(crate) fn validate_in(
    disks: &[DiskInfo],
    id: &str,
    allow_system_disk: bool,
    paths: &Paths<'_>,
) -> Result<Validation> {
    let name = id.strip_prefix("/dev/").unwrap_or(id);
    let disk = find(disks, name)?;
    let checks = checks(
        disk,
        preflight::running_system_disk().as_deref(),
        allow_system_disk,
        paths,
    );

    Ok(Validation {
        target: Target {
            path: disk.path.clone(),
            is_system: disk.is_system,
            is_data: disk.is_data,
            partitions: disk
                .partitions
                .iter()
                .map(|partition| partition.number)
                .collect(),
        },
        disk: disk.clone(),
        checks,
    })
}

fn checks(disk: &DiskInfo, running: Option<&str>, allow: bool, paths: &Paths<'_>) -> Vec<Check> {
    let mut checks = vec![
        read_only(disk),
        preflight::system_disk_in_use(&disk.path, running, allow),
        mounted_partitions(disk, allow),
        muak_partitions(disk, allow),
    ];
    checks.extend(foreign::checks(disk, paths));

    checks
}

fn read_only(disk: &DiskInfo) -> Check {
    if disk.read_only {
        return Check::fail(
            "read_only",
            format!("Disk '{}' is read-only and cannot be wiped", disk.path),
        );
    }

    Check::pass("read_only", format!("'{}' is writable", disk.path))
}

fn mounted_partitions(disk: &DiskInfo, allow: bool) -> Check {
    let mounted = disk
        .partitions
        .iter()
        .map(|partition| partition.mount_point.as_str())
        .find(|mount_point| !mount_point.is_empty());

    match mounted {
        Some(mount_point) if !allow => Check::fail(
            "mounted_partitions",
            format!(
                "Disk '{}' is mounted at '{mount_point}', unmount it or pass --allow-system-disk",
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

fn muak_partitions(disk: &DiskInfo, allow: bool) -> Check {
    if !(disk.is_system || disk.is_data) {
        return Check::pass(
            "muak_partitions",
            format!("'{}' has no Muak STATE or DATA partition", disk.path),
        );
    }

    if allow {
        return Check::pass(
            "muak_partitions",
            format!(
                "'{}' carries the Muak {} partition and --allow-system-disk was given",
                disk.path,
                role(disk)
            ),
        );
    }

    Check::fail(
        "muak_partitions",
        format!(
            "Disk '{}' carries the Muak {} partition, pass --allow-system-disk to wipe it",
            disk.path,
            role(disk)
        ),
    )
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
            ..DiskInfo::default()
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

    fn check<'a>(validation: &'a Validation, name: &str) -> &'a Check {
        validation
            .checks
            .iter()
            .find(|check| check.name == name)
            .expect("check")
    }

    fn validate(disks: &[DiskInfo], id: &str, allow: bool) -> Result<Validation> {
        let dir = tempfile::tempdir()?;
        let swaps = dir.path().join("swaps");
        std::fs::write(&swaps, "")?;
        let paths = Paths {
            sysfs_root: dir.path(),
            class_block: dir.path(),
            swaps: &swaps,
        };

        validate_in(disks, id, allow, &paths)
    }

    #[test]
    fn validate_accepts_a_plain_whole_disk_by_name_or_path() {
        // ARRANGE
        let disks = vec![disk("sda")];

        // ACT
        let by_name = validate(&disks, "sda", false).expect("name");
        let by_path = validate(&disks, "/dev/sda", false).expect("path");

        // ASSERT
        assert_eq!(by_name.target.path, "/dev/sda");
        assert_eq!(by_path.disk.name, "sda");
    }

    #[test]
    fn validate_returns_the_disk_and_its_checks() {
        // ARRANGE
        let disks = vec![disk("sda")];

        // ACT
        let validation = validate(&disks, "sda", false).expect("target");

        // ASSERT
        assert_eq!(validation.disk.path, "/dev/sda");
        let names: Vec<&str> = validation.checks.iter().map(|check| check.name).collect();
        assert_eq!(
            names,
            [
                "read_only",
                "system_disk_in_use",
                "mounted_partitions",
                "muak_partitions",
                "foreign_holders",
                "active_swap",
            ]
        );
    }

    #[test]
    fn validate_rejects_partitions() {
        // ARRANGE
        let disks = vec![disk("sda"), disk("nvme0n1")];

        // ACT
        let sata = validate(&disks, "sda1", true);
        let nvme = validate(&disks, "/dev/nvme0n1p2", true);

        // ASSERT
        assert!(sata.is_err(), "sda1 is a partition");
        assert!(nvme.is_err(), "nvme0n1p2 is a partition");
    }

    #[test]
    fn validate_rejects_read_only_disks() {
        // ARRANGE
        let mut disks = vec![disk("sda")];
        disks[0].read_only = true;

        // ACT
        let validation = validate(&disks, "sda", true).expect("resolved");

        // ASSERT
        assert!(!check(&validation, "read_only").passed);
    }

    #[test]
    fn validate_rejects_mounted_disks_without_allow_system_disk() {
        // ARRANGE
        let mut disks = vec![disk("sda")];
        disks[0].partitions.push(partition(1, "/mnt/data"));

        // ACT
        let refused = validate(&disks, "sda", false).expect("resolved");
        let allowed = validate(&disks, "sda", true).expect("resolved");

        // ASSERT
        assert!(!check(&refused, "mounted_partitions").passed);
        assert!(check(&allowed, "mounted_partitions").passed);
    }

    #[test]
    fn validate_refuses_system_and_data_disks_without_allow_system_disk() {
        // ARRANGE
        let mut system = disk("sda");
        system.is_system = true;
        let mut data = disk("sdb");
        data.is_data = true;
        let disks = vec![system, data];

        // ACT
        let system_result = validate(&disks, "sda", false).expect("resolved");
        let data_result = validate(&disks, "sdb", false).expect("resolved");

        // ASSERT
        assert!(!check(&system_result, "muak_partitions").passed);
        assert!(!check(&data_result, "muak_partitions").passed);
    }

    #[test]
    fn validate_allows_system_disks_when_explicitly_allowed() {
        // ARRANGE
        let mut system = disk("sda");
        system.is_system = true;
        system.is_data = true;
        system.partitions.push(partition(1, ""));

        // ACT
        let validation = validate(&[system], "sda", true).expect("allowed");

        // ASSERT
        assert!(validation.target.is_system);
        assert!(validation.target.is_data);
        assert_eq!(validation.target.partitions, [1]);
        assert!(validation.checks.iter().all(|check| check.passed));
    }

    #[test]
    fn validate_unknown_disk_is_an_error() {
        // ARRANGE
        let disks = vec![disk("sda")];

        // ACT
        let result = validate(&disks, "sdz", false);

        // ASSERT
        assert!(result.is_err(), "unknown disks are refused");
    }
}
