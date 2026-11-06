//! Guard checks for a disk selected for wiping.

use anyhow::{Result, bail};
use blockdev::roots::Roots;
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
    validate_in(disks, id, allow_system_disk, &Roots::system())
}

/// Resolves `id` against injectable roots.
pub(crate) fn validate_in(
    disks: &[Disk],
    id: &str,
    allow_system_disk: bool,
    roots: &Roots,
) -> Result<Verdict> {
    let name = id.strip_prefix("/dev/").unwrap_or(id);
    let disk = find(disks, name)?;
    let checks = checks(
        disk,
        checks::running_system_disk().as_deref(),
        allow_system_disk,
        roots,
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

fn checks(disk: &Disk, running: Option<&str>, allow: bool, roots: &Roots) -> Vec<Check> {
    let mut checks = vec![
        partitions::read_only(disk),
        checks::system_disk_in_use(&disk.path, running, allow),
        partitions::mounted(disk, allow),
        partitions::muak(disk, allow),
    ];
    checks.extend(foreign::checks(disk, roots));

    checks
}

fn find<'a>(disks: &'a [Disk], name: &str) -> Result<&'a Disk> {
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

#[cfg(test)]
mod tests {
    use ::disk::role::Role;
    use blockdev::types::Partition;

    use super::*;

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

    fn roots() -> (tempfile::TempDir, Roots) {
        let dir = tempfile::tempdir().expect("test roots");
        let roots = Roots {
            block: dir.path().join("sys/block"),
            class_block: dir.path().join("sys/class/block"),
            dev: dir.path().join("dev"),
            mounts: dir.path().join("proc/mounts"),
            swaps: dir.path().join("proc/swaps"),
        };

        for relative in ["sys/block", "sys/class/block", "dev", "proc"] {
            std::fs::create_dir_all(dir.path().join(relative)).expect("test root");
        }
        std::fs::write(&roots.mounts, "").expect("mount table");
        std::fs::write(&roots.swaps, "").expect("swap table");

        (dir, roots)
    }

    fn validate(disks: &[Disk], id: &str, allow: bool) -> Result<Verdict> {
        let (_dir, roots) = roots();

        validate_in(disks, id, allow, &roots)
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
        assert!(sata.is_err(), "sda1 is a partition");
        assert!(nvme.is_err(), "nvme0n1p2 is a partition");
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
