//! Detection of foreign device-mapper holders and active swap on a wipe target.

use ::disk::role::Role;
use anyhow::{Context as _, Result};
use blockdev::roots::Roots;
use blockdev::types::Disk;
use blockdev::{discover, holders, mounts};

use crate::check::Check;

/// Returns the checks refusing a disk used by foreign holders or active swap.
pub(crate) fn checks(disk: &Disk, roots: &Roots) -> Vec<Check> {
    vec![foreign_holders(disk, roots), swap(disk, roots)]
}

fn foreign_holders(disk: &Disk, roots: &Roots) -> Check {
    match holder_names(disk, roots) {
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

fn swap(disk: &Disk, roots: &Roots) -> Check {
    match swap_device(disk, roots) {
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

fn holder_names(disk: &Disk, roots: &Roots) -> Result<Vec<String>> {
    let mut names = holders::try_names(roots, &disk.name)
        .with_context(|| format!("failed to read the holders of '{}'", disk.name))?;

    for partition in &disk.partitions {
        let path = format!("{}/{}", disk.name, partition.name);
        names.extend(
            holders::try_names(roots, &path)
                .with_context(|| format!("failed to read the holders of '{path}'"))?,
        );
    }

    names.retain(|holder| !is_muak_mapping(holder));
    names.sort();
    names.dedup();

    Ok(names)
}

fn is_muak_mapping(holder: &str) -> bool {
    holder == Role::State.dm_name()
        || holder == Role::Ephemeral.dm_name()
        || holder == Role::Data.dm_name()
}

fn swap_device(disk: &Disk, roots: &Roots) -> Result<Option<String>> {
    let swaps = mounts::try_read_swaps(&roots.swaps)
        .with_context(|| format!("failed to read {}", roots.swaps.display()))?;

    Ok(swaps
        .into_iter()
        .map(|swap| swap.device)
        .find(|device| belongs_to(device, disk, roots)))
}

fn belongs_to(device: &str, disk: &Disk, roots: &Roots) -> bool {
    if device == disk.path || disk.partitions.iter().any(|part| part.path == device) {
        return true;
    }

    discover::parent_disk_in(roots, device).as_deref() == Some(disk.name.as_str())
}

#[cfg(test)]
mod tests {
    use blockdev::types::Partition;

    use super::*;

    fn disk(name: &str) -> Disk {
        Disk {
            name: name.to_owned(),
            path: format!("/dev/{name}"),
            ..Disk::default()
        }
    }

    fn partition(number: u32) -> Partition {
        Partition {
            number,
            name: format!("sda{number}"),
            path: format!("/dev/sda{number}"),
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

    fn holder(roots: &Roots, device: &str, holder: &str) {
        let holders = roots.block.join(device).join("holders");
        std::fs::create_dir_all(&holders).expect("holders dir");
        std::fs::write(holders.join(holder), "").expect("holder");
    }

    fn mapping(roots: &Roots, holder: &str, dm_name: &str) {
        let dm = roots.block.join(holder).join("dm");
        std::fs::create_dir_all(&dm).expect("dm dir");
        std::fs::write(dm.join("name"), dm_name).expect("dm name");
    }

    fn swaps(roots: &Roots, content: &str) {
        std::fs::write(&roots.swaps, content).expect("swap table");
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
        let (_dir, roots) = roots();
        holder(&roots, "sda", "dm-0");

        // ACT
        let checks = checks(&disk("sda"), &roots);

        // ASSERT
        let holders = check(&checks, "foreign_holders");
        assert!(!holders.passed);
        assert!(holders.detail.contains("dm-0"), "{}", holders.detail);
    }

    #[test]
    fn holders_on_a_partition_are_refused() {
        // ARRANGE
        let (_dir, roots) = roots();
        holder(&roots, "sda/sda1", "luks-1234");
        let mut disk = disk("sda");
        disk.partitions.push(partition(1));

        // ACT
        let checks = checks(&disk, &roots);

        // ASSERT
        let holders = check(&checks, "foreign_holders");
        assert!(!holders.passed);
        assert!(holders.detail.contains("luks-1234"), "{}", holders.detail);
    }

    #[test]
    fn holders_on_an_unrelated_disk_are_ignored() {
        // ARRANGE
        let (_dir, roots) = roots();
        holder(&roots, "sdb/sdb1", "dm-0");
        let mut disk = disk("sda");
        disk.partitions.push(partition(1));

        // ACT
        let checks = checks(&disk, &roots);

        // ASSERT
        assert!(check(&checks, "foreign_holders").passed);
    }

    #[test]
    fn muak_mappings_are_not_foreign() {
        // ARRANGE
        let (_dir, roots) = roots();
        holder(&roots, "sda/sda1", "dm-0");
        holder(&roots, "sda/sda2", "dm-1");
        holder(&roots, "sda/sda3", "dm-2");
        mapping(&roots, "dm-0", Role::State.dm_name());
        mapping(&roots, "dm-1", Role::Ephemeral.dm_name());
        mapping(&roots, "dm-2", Role::Data.dm_name());
        let mut disk = disk("sda");
        disk.partitions.push(partition(1));
        disk.partitions.push(partition(2));
        disk.partitions.push(partition(3));

        // ACT
        let checks = checks(&disk, &roots);

        // ASSERT
        assert!(check(&checks, "foreign_holders").passed);
    }

    #[test]
    fn unreadable_holders_refuse_the_disk() {
        // ARRANGE
        let (_dir, roots) = roots();
        std::fs::write(roots.block.join("sda"), "").expect("blocking file");

        // ACT
        let checks = checks(&disk("sda"), &roots);

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
        let (_dir, roots) = roots();
        swaps(
            &roots,
            "Filename\tType\tSize\tUsed\tPriority\n/dev/sda1\tpartition\t1024\t0\t-2\n",
        );
        let mut disk = disk("sda");
        disk.partitions.push(partition(1));

        // ACT
        let checks = checks(&disk, &roots);

        // ASSERT
        let swap = check(&checks, "active_swap");
        assert!(!swap.passed);
        assert!(swap.detail.contains("/dev/sda1"), "{}", swap.detail);
    }

    #[test]
    fn swap_outside_the_inventory_is_resolved_through_sysfs() {
        // ARRANGE
        let (_dir, roots) = roots();
        let partition = roots.class_block.join("sda").join("sda2");
        std::fs::create_dir_all(&partition).expect("partition");
        std::os::unix::fs::symlink(&partition, roots.class_block.join("sda2")).expect("symlink");
        swaps(
            &roots,
            "Filename\tType\tSize\tUsed\tPriority\n/dev/sda2\tpartition\t1024\t0\t-2\n",
        );

        // ACT
        let checks = checks(&disk("sda"), &roots);

        // ASSERT
        let swap = check(&checks, "active_swap");
        assert!(!swap.passed);
        assert!(swap.detail.contains("/dev/sda2"), "{}", swap.detail);
    }

    #[test]
    fn swap_on_an_unrelated_disk_is_ignored() {
        // ARRANGE
        let (_dir, roots) = roots();
        swaps(
            &roots,
            "Filename\tType\tSize\tUsed\tPriority\n/dev/sdb1\tpartition\t1024\t0\t-2\n",
        );

        // ACT
        let checks = checks(&disk("sda"), &roots);

        // ASSERT
        assert!(check(&checks, "active_swap").passed);
    }

    #[test]
    fn a_clean_disk_passes_every_check() {
        // ARRANGE
        let (_dir, roots) = roots();
        let mut disk = disk("sda");
        disk.partitions.push(partition(1));

        // ACT
        let checks = checks(&disk, &roots);

        // ASSERT
        assert!(checks.iter().all(|check| check.passed));
    }
}
