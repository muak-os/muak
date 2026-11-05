//! Detection of foreign device-mapper holders and active swap on a wipe target.

use std::io::ErrorKind;
use std::path::Path;

use ::disk::discover::parent_disk_in;
use ::disk::role::Role;
use anyhow::{Context as _, Result};

use super::sysfs::{SYSFS_BLOCK_ROOT, SYSFS_CLASS_BLOCK_ROOT};
use super::types::DiskInfo;
use crate::check::Check;

/// Path of the kernel swap table.
pub(crate) const SWAPS_PATH: &str = "/proc/swaps";

/// Filesystem locations consulted when detecting foreign users of a disk.
pub(crate) struct Paths<'a> {
    /// Root of the block device tree, normally `/sys/block`.
    pub sysfs_root: &'a Path,
    /// Flat block device class tree, normally `/sys/class/block`.
    pub class_block: &'a Path,
    /// Kernel swap table, normally `/proc/swaps`.
    pub swaps: &'a Path,
}

impl Paths<'_> {
    /// Returns the paths of the running system.
    pub(crate) fn system() -> Paths<'static> {
        Paths {
            sysfs_root: Path::new(SYSFS_BLOCK_ROOT),
            class_block: Path::new(SYSFS_CLASS_BLOCK_ROOT),
            swaps: Path::new(SWAPS_PATH),
        }
    }
}

/// Returns the checks refusing a disk used by foreign holders or active swap.
pub(crate) fn checks(disk: &DiskInfo, paths: &Paths<'_>) -> Vec<Check> {
    vec![holders(disk, paths.sysfs_root), swap(disk, paths)]
}

fn holders(disk: &DiskInfo, sysfs_root: &Path) -> Check {
    match holder_names(disk, sysfs_root) {
        Ok(names) if names.is_empty() => Check::pass(
            "foreign_holders",
            format!("'{}' has no foreign holders", disk.path),
        ),
        Ok(names) => Check::fail(
            "foreign_holders",
            format!("Disk '{}' is held by: {}", disk.path, names.join(", ")),
        ),
        Err(e) => Check::fail(
            "foreign_holders",
            format!("Cannot inspect the holders of '{}': {e:#}", disk.path),
        ),
    }
}

fn swap(disk: &DiskInfo, paths: &Paths<'_>) -> Check {
    match swap_device(disk, paths) {
        Ok(Some(device)) => Check::fail(
            "active_swap",
            format!("Disk '{}' has active swap on '{device}'", disk.path),
        ),
        Ok(None) => Check::pass("active_swap", format!("'{}' has no active swap", disk.path)),
        Err(e) => Check::fail(
            "active_swap",
            format!("Cannot inspect the swap table for '{}': {e:#}", disk.path),
        ),
    }
}

fn holder_names(disk: &DiskInfo, sysfs_root: &Path) -> Result<Vec<String>> {
    let disk_path = sysfs_root.join(&disk.name);
    let mut names = read_holders(&disk_path)?;

    for partition in &disk.partitions {
        names.extend(read_holders(&disk_path.join(&partition.name))?);
    }

    names.retain(|holder| !is_muak_mapping(holder, sysfs_root));
    names.sort();
    names.dedup();

    Ok(names)
}

fn is_muak_mapping(holder: &str, sysfs_root: &Path) -> bool {
    let Ok(name) = std::fs::read_to_string(sysfs_root.join(holder).join("dm/name")) else {
        return false;
    };
    let name = name.trim();

    name == Role::State.dm_name()
        || name == Role::Ephemeral.dm_name()
        || name == Role::Data.dm_name()
}

fn read_holders(device: &Path) -> Result<Vec<String>> {
    let holders = device.join("holders");
    let entries = match std::fs::read_dir(&holders) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("failed to read {}", holders.display())),
    };

    let mut names = Vec::new();
    for entry in entries {
        let entry =
            entry.with_context(|| format!("failed to read an entry in {}", holders.display()))?;
        names.push(entry.file_name().to_string_lossy().into_owned());
    }

    Ok(names)
}

fn swap_device(disk: &DiskInfo, paths: &Paths<'_>) -> Result<Option<String>> {
    let swaps = read_swaps(paths.swaps)?;

    Ok(swaps
        .into_iter()
        .find(|device| belongs_to(device, disk, paths.class_block)))
}

fn read_swaps(path: &Path) -> Result<Vec<String>> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("failed to read {}", path.display())),
    };

    Ok(content
        .lines()
        .skip(1)
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_owned)
        .collect())
}

fn belongs_to(device: &str, disk: &DiskInfo, class_block: &Path) -> bool {
    if device == disk.path || disk.partitions.iter().any(|part| part.path == device) {
        return true;
    }

    parent_disk_in(class_block, device).as_deref() == Some(disk.name.as_str())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::disk::types::PartitionInfo;

    fn disk(name: &str) -> DiskInfo {
        DiskInfo {
            name: name.to_owned(),
            path: format!("/dev/{name}"),
            ..DiskInfo::default()
        }
    }

    fn partition(number: u32) -> PartitionInfo {
        PartitionInfo {
            number,
            start_sector: 0,
            size_bytes: 0,
            name: format!("sda{number}"),
            path: format!("/dev/sda{number}"),
            fstype: String::new(),
            gpt_name: String::new(),
            mount_point: String::new(),
            in_use: false,
        }
    }

    fn sysfs() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn swaps(content: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("swaps");
        std::fs::write(&path, content).expect("write swaps");

        (dir, path)
    }

    fn make_holder(sysfs_root: &Path, device: &str, holder: &str) {
        let holders = sysfs_root.join(device).join("holders");
        std::fs::create_dir_all(&holders).expect("holders dir");
        std::fs::write(holders.join(holder), "").expect("holder");
    }

    fn make_mapping(sysfs_root: &Path, holder: &str, dm_name: &str) {
        let dm = sysfs_root.join(holder).join("dm");
        std::fs::create_dir_all(&dm).expect("dm dir");
        std::fs::write(dm.join("name"), dm_name).expect("dm name");
    }

    fn check<'a>(checks: &'a [Check], name: &str) -> &'a Check {
        checks
            .iter()
            .find(|check| check.name == name)
            .expect("check")
    }

    #[test]
    fn holders_on_the_disk_are_refused() {
        // ARRANGE
        let sysfs = sysfs();
        make_holder(sysfs.path(), "sda", "dm-0");
        let paths = Paths {
            sysfs_root: sysfs.path(),
            class_block: Path::new("/does-not-exist"),
            swaps: Path::new("/does-not-exist"),
        };

        // ACT
        let checks = checks(&disk("sda"), &paths);

        // ASSERT
        let holders = check(&checks, "foreign_holders");
        assert!(!holders.passed);
        assert!(holders.detail.contains("dm-0"), "{}", holders.detail);
    }

    #[test]
    fn holders_on_a_partition_are_refused() {
        // ARRANGE
        let sysfs = sysfs();
        make_holder(sysfs.path(), "sda/sda1", "luks-1234");
        let mut disk = disk("sda");
        disk.partitions.push(partition(1));
        let paths = Paths {
            sysfs_root: sysfs.path(),
            class_block: Path::new("/does-not-exist"),
            swaps: Path::new("/does-not-exist"),
        };

        // ACT
        let checks = checks(&disk, &paths);

        // ASSERT
        let holders = check(&checks, "foreign_holders");
        assert!(!holders.passed);
        assert!(holders.detail.contains("luks-1234"), "{}", holders.detail);
    }

    #[test]
    fn holders_on_an_unrelated_disk_are_ignored() {
        // ARRANGE
        let sysfs = sysfs();
        make_holder(sysfs.path(), "sdb/sdb1", "dm-0");
        let mut disk = disk("sda");
        disk.partitions.push(partition(1));
        let paths = Paths {
            sysfs_root: sysfs.path(),
            class_block: Path::new("/does-not-exist"),
            swaps: Path::new("/does-not-exist"),
        };

        // ACT
        let checks = checks(&disk, &paths);

        // ASSERT
        assert!(check(&checks, "foreign_holders").passed);
    }

    #[test]
    fn muak_mappings_are_not_foreign() {
        // ARRANGE
        let sysfs = sysfs();
        make_holder(sysfs.path(), "sda/sda1", "dm-0");
        make_holder(sysfs.path(), "sda/sda2", "dm-1");
        make_holder(sysfs.path(), "sda/sda3", "dm-2");
        make_mapping(sysfs.path(), "dm-0", Role::State.dm_name());
        make_mapping(sysfs.path(), "dm-1", Role::Ephemeral.dm_name());
        make_mapping(sysfs.path(), "dm-2", Role::Data.dm_name());
        let mut disk = disk("sda");
        disk.partitions.push(partition(1));
        disk.partitions.push(partition(2));
        disk.partitions.push(partition(3));
        let paths = Paths {
            sysfs_root: sysfs.path(),
            class_block: Path::new("/does-not-exist"),
            swaps: Path::new("/does-not-exist"),
        };

        // ACT
        let checks = checks(&disk, &paths);

        // ASSERT
        assert!(check(&checks, "foreign_holders").passed);
    }

    #[test]
    fn unreadable_holders_refuse_the_disk() {
        // ARRANGE
        let sysfs = sysfs();
        std::fs::write(sysfs.path().join("sda"), "").expect("blocking file");
        let paths = Paths {
            sysfs_root: sysfs.path(),
            class_block: Path::new("/does-not-exist"),
            swaps: Path::new("/does-not-exist"),
        };

        // ACT
        let checks = checks(&disk("sda"), &paths);

        // ASSERT
        let holders = check(&checks, "foreign_holders");
        assert!(!holders.passed);
        assert!(
            holders.detail.contains("Cannot inspect"),
            "{}",
            holders.detail
        );
    }

    #[test]
    fn swap_on_a_partition_is_refused() {
        // ARRANGE
        let (_dir, swaps) =
            swaps("Filename\tType\tSize\tUsed\tPriority\n/dev/sda1\tpartition\t1024\t0\t-2\n");
        let mut disk = disk("sda");
        disk.partitions.push(partition(1));
        let paths = Paths {
            sysfs_root: Path::new("/does-not-exist"),
            class_block: Path::new("/does-not-exist"),
            swaps: &swaps,
        };

        // ACT
        let checks = checks(&disk, &paths);

        // ASSERT
        let swap = check(&checks, "active_swap");
        assert!(!swap.passed);
        assert!(swap.detail.contains("/dev/sda1"), "{}", swap.detail);
    }

    #[test]
    fn swap_outside_the_inventory_is_resolved_through_sysfs() {
        // ARRANGE
        let class_block = tempfile::tempdir().expect("tempdir");
        let partition = class_block.path().join("sda").join("sda2");
        std::fs::create_dir_all(&partition).expect("partition");
        std::os::unix::fs::symlink(&partition, class_block.path().join("sda2")).expect("symlink");
        let (_dir, swaps) =
            swaps("Filename\tType\tSize\tUsed\tPriority\n/dev/sda2\tpartition\t1024\t0\t-2\n");
        let paths = Paths {
            sysfs_root: Path::new("/does-not-exist"),
            class_block: class_block.path(),
            swaps: &swaps,
        };

        // ACT
        let checks = checks(&disk("sda"), &paths);

        // ASSERT
        let swap = check(&checks, "active_swap");
        assert!(!swap.passed);
        assert!(swap.detail.contains("/dev/sda2"), "{}", swap.detail);
    }

    #[test]
    fn swap_on_an_unrelated_disk_is_ignored() {
        // ARRANGE
        let (_dir, swaps) =
            swaps("Filename\tType\tSize\tUsed\tPriority\n/dev/sdb1\tpartition\t1024\t0\t-2\n");
        let paths = Paths {
            sysfs_root: Path::new("/does-not-exist"),
            class_block: Path::new("/does-not-exist"),
            swaps: &swaps,
        };

        // ACT
        let checks = checks(&disk("sda"), &paths);

        // ASSERT
        assert!(check(&checks, "active_swap").passed);
    }

    #[test]
    fn a_clean_disk_passes_every_check() {
        // ARRANGE
        let sysfs = sysfs();
        let (_dir, swaps) = swaps("Filename\tType\tSize\tUsed\tPriority\n");
        let mut disk = disk("sda");
        disk.partitions.push(partition(1));
        let paths = Paths {
            sysfs_root: sysfs.path(),
            class_block: Path::new("/does-not-exist"),
            swaps: &swaps,
        };

        // ACT
        let checks = checks(&disk, &paths);

        // ASSERT
        assert!(checks.iter().all(|check| check.passed));
    }
}
