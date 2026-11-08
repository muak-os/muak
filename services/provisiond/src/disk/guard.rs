//! Guard checks for a disk selected for wiping.

use anyhow::{Result, bail};
use blockdev::device;
use blockdev::kernel::Kernel;
use blockdev::types::Disk;

use super::checks::partitions;
use super::{checks, foreign, roles};
use crate::check::Check;

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
pub(crate) struct Verdict {
    pub target: Target,
    pub disk: Disk,
    pub checks: Vec<Check>,
}

/// Resolves `id` to a whole disk and returns the wipe safety checks.
pub(crate) fn validate(disks: &[Disk], id: &str, allow_system_disk: bool) -> Result<Verdict> {
    validate_in(disks, id, allow_system_disk, &blockdev::handle())
}

/// Resolves `id` against the supplied kernel view.
pub(crate) fn validate_in<K: Kernel>(
    disks: &[Disk],
    id: &str,
    allow_system_disk: bool,
    kernel: &K,
) -> Result<Verdict> {
    let name = id.strip_prefix("/dev/").unwrap_or(id);
    let disk = find(disks, name)?;
    let checks = checks(
        disk,
        checks::running_system_disk(kernel).as_deref(),
        allow_system_disk,
        kernel,
    );

    Ok(Verdict {
        target: Target {
            path: disk.path.clone(),
            is_system: roles::is_system(disk),
            is_data: roles::is_data(disk),
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

fn checks<K: Kernel>(disk: &Disk, running: Option<&str>, allow: bool, kernel: &K) -> Vec<Check> {
    let mut checks = vec![
        partitions::read_only(disk),
        checks::system_disk_in_use(&disk.path, running, allow),
        partitions::mounted(disk, allow),
        partitions::muak(disk, allow),
    ];
    checks.extend(foreign::checks(disk, kernel));

    checks
}

fn find<'a>(disks: &'a [Disk], name: &str) -> Result<&'a Disk> {
    if let Some(disk) = disks.iter().find(|disk| disk.name == name) {
        return Ok(disk);
    }

    if disks
        .iter()
        .any(|disk| device::belongs_to(name, &disk.name))
    {
        bail!("'{name}' is a partition, wipe the whole disk instead");
    }

    bail!("disk '{name}' not found")
}

#[cfg(test)]
mod tests {
    use std::os::fd::AsFd;

    use ::disk::role::Role;
    use blockdev::error::{BlockdevError, Result as BlockdevResult};
    use blockdev::mounts::{Mount, Swap};
    use blockdev::types::Partition;

    use super::*;

    struct Fake;

    impl Kernel for Fake {
        fn disks(&self) -> BlockdevResult<Vec<Disk>> {
            Err(BlockdevError::MissingRoot("unused".into()))
        }

        fn find_by_partname(&self, _partname: &str) -> Vec<String> {
            panic!("guard tests do not resolve partitions by name")
        }

        fn holders(&self, _device: &str) -> BlockdevResult<Vec<String>> {
            Ok(Vec::new())
        }

        fn mounts(&self) -> Vec<Mount> {
            Vec::new()
        }

        fn parent_disk(&self, _partition_device: &str) -> Option<String> {
            None
        }

        fn size<Fd: AsFd>(&self, _fd: &Fd) -> BlockdevResult<u64> {
            Err(BlockdevError::MissingRoot("unused".into()))
        }

        fn swaps(&self) -> BlockdevResult<Vec<Swap>> {
            Ok(Vec::new())
        }
    }

    fn disk(name: &str) -> Disk {
        Disk {
            name: name.to_owned(),
            path: format!("/dev/{name}"),
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

    fn validate(disks: &[Disk], id: &str, allow: bool) -> Result<Verdict> {
        validate_in(disks, id, allow, &Fake)
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
        let verdict = validate(&disks, "sda", false).expect("target");

        // ASSERT
        assert_eq!(verdict.disk.path, "/dev/sda");
        let names: Vec<&str> = verdict.checks.iter().map(|check| check.name).collect();
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
        let sata = sata.err().expect("sda1 is a partition").to_string();
        let nvme = nvme.err().expect("nvme0n1p2 is a partition").to_string();
        assert!(sata.contains("is a partition, wipe the whole disk instead"));
        assert!(nvme.contains("is a partition, wipe the whole disk instead"));
    }

    #[test]
    fn validate_treats_partition_lookalike_as_missing_disks() {
        // ARRANGE
        let disks = vec![disk("sda"), disk("nvme0n1")];

        // ACT
        let sdaa1 = validate(&disks, "sdaa1", true);
        let namespace = validate(&disks, "nvme0n11", true);

        // ASSERT
        assert_eq!(
            sdaa1.err().expect("sdaa1 is another disk").to_string(),
            "disk 'sdaa1' not found"
        );
        assert_eq!(
            namespace
                .err()
                .expect("nvme0n11 is namespace 11")
                .to_string(),
            "disk 'nvme0n11' not found"
        );
    }

    #[test]
    fn validate_allows_system_disks_when_explicitly_allowed() {
        // ARRANGE
        let mut system = disk("sda");
        system
            .partitions
            .push(partition(1, "", Role::State.gpt_name()));
        system
            .partitions
            .push(partition(2, "", Role::Data.gpt_name()));

        // ACT
        let verdict = validate(&[system], "sda", true).expect("allowed");

        // ASSERT
        assert!(verdict.target.is_system);
        assert!(verdict.target.is_data);
        assert_eq!(verdict.target.partitions, [1, 2]);
        assert!(verdict.checks.iter().all(|check| check.passed));
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
