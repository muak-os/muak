//! Detection of foreign device-mapper holders and active swap on a wipe target.

use ::disk::role::Role;
use anyhow::{Context as _, Result};
use blockdev::kernel::Kernel;
use blockdev::types::Disk;

use crate::check::Check;

/// Returns the checks refusing a disk used by foreign holders or active swap.
pub(crate) fn checks<K: Kernel>(disk: &Disk, kernel: &K) -> Vec<Check> {
    vec![foreign_holders(disk, kernel), swap(disk, kernel)]
}

fn foreign_holders<K: Kernel>(disk: &Disk, kernel: &K) -> Check {
    match holder_names(disk, kernel) {
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

fn swap<K: Kernel>(disk: &Disk, kernel: &K) -> Check {
    match swap_device(disk, kernel) {
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

fn holder_names<K: Kernel>(disk: &Disk, kernel: &K) -> Result<Vec<String>> {
    let mut names = kernel
        .holders(&disk.name)
        .with_context(|| format!("failed to read the holders of '{}'", disk.name))?;

    for partition in &disk.partitions {
        let path = format!("{}/{}", disk.name, partition.name);
        names.extend(
            kernel
                .holders(&path)
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

fn swap_device<K: Kernel>(disk: &Disk, kernel: &K) -> Result<Option<String>> {
    let swaps = kernel
        .swaps()
        .context("failed to read the kernel swap table")?;

    Ok(swaps
        .into_iter()
        .map(|swap| swap.device)
        .find(|device| belongs_to(device, disk, kernel)))
}

fn belongs_to<K: Kernel>(device: &str, disk: &Disk, kernel: &K) -> bool {
    if device == disk.path || disk.partitions.iter().any(|part| part.path == device) {
        return true;
    }

    kernel.parent_disk(device).as_deref() == Some(disk.name.as_str())
}

#[cfg(test)]
mod tests {
    use std::os::fd::AsFd;

    use blockdev::error::{BlockdevError, Result as BlockdevResult};
    use blockdev::mounts::{Mount, Swap};
    use blockdev::types::Partition;

    use super::*;

    #[derive(Default)]
    struct Fake {
        holders: Vec<(String, Vec<String>)>,
        parents: Vec<(String, String)>,
        swaps: Vec<Swap>,
        unreadable: Vec<String>,
    }

    impl Kernel for Fake {
        fn disks(&self) -> BlockdevResult<Vec<Disk>> {
            Err(BlockdevError::MissingRoot("unused".into()))
        }

        fn find_by_partname(&self, _partname: &str) -> Vec<String> {
            panic!("foreign tests do not resolve partitions by name")
        }

        fn holders(&self, device: &str) -> BlockdevResult<Vec<String>> {
            let blocked = self.unreadable.contains(&device.to_owned());
            let names = self
                .holders
                .iter()
                .filter(|entry| entry.0.as_str() == device)
                .flat_map(|entry| entry.1.iter().cloned())
                .collect();

            (!blocked)
                .then_some(names)
                .ok_or_else(|| BlockdevError::MissingRoot(device.into()))
        }

        fn mounts(&self) -> Vec<Mount> {
            panic!("foreign tests do not read the mount table")
        }

        fn parent_disk(&self, partition_device: &str) -> Option<String> {
            self.parents
                .iter()
                .find(|entry| entry.0.as_str() == partition_device)
                .map(|entry| entry.1.clone())
        }

        fn size<Fd: AsFd>(&self, _fd: &Fd) -> BlockdevResult<u64> {
            Err(BlockdevError::MissingRoot("unused".into()))
        }

        fn swaps(&self) -> BlockdevResult<Vec<Swap>> {
            Ok(self.swaps.clone())
        }
    }

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

    fn holders(entries: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
        entries
            .iter()
            .map(|&(device, names)| {
                (
                    device.to_owned(),
                    names.iter().map(|&name| name.to_owned()).collect(),
                )
            })
            .collect()
    }

    fn swap_entry(device: &str) -> Swap {
        Swap {
            device: device.to_owned(),
        }
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
        let fake = Fake {
            holders: holders(&[("sda", &["dm-0"])]),
            ..Fake::default()
        };

        // ACT
        let checks = checks(&disk("sda"), &fake);

        // ASSERT
        let holders = check(&checks, "foreign_holders");
        assert!(!holders.passed);
        assert!(holders.detail.contains("dm-0"), "{}", holders.detail);
    }

    #[test]
    fn holders_on_a_partition_are_refused() {
        // ARRANGE
        let fake = Fake {
            holders: holders(&[("sda/sda1", &["luks-1234"])]),
            ..Fake::default()
        };
        let mut disk = disk("sda");
        disk.partitions.push(partition(1));

        // ACT
        let checks = checks(&disk, &fake);

        // ASSERT
        let holders = check(&checks, "foreign_holders");
        assert!(!holders.passed);
        assert!(holders.detail.contains("luks-1234"), "{}", holders.detail);
    }

    #[test]
    fn holders_on_an_unrelated_disk_are_ignored() {
        // ARRANGE
        let fake = Fake {
            holders: holders(&[("sdb/sdb1", &["dm-0"])]),
            ..Fake::default()
        };
        let mut disk = disk("sda");
        disk.partitions.push(partition(1));

        // ACT
        let checks = checks(&disk, &fake);

        // ASSERT
        assert!(check(&checks, "foreign_holders").passed);
    }

    #[test]
    fn muak_mappings_are_not_foreign() {
        // ARRANGE
        let fake = Fake {
            holders: holders(&[
                ("sda/sda1", &[Role::State.dm_name()]),
                ("sda/sda2", &[Role::Ephemeral.dm_name()]),
                ("sda/sda3", &[Role::Data.dm_name()]),
            ]),
            ..Fake::default()
        };
        let mut disk = disk("sda");
        disk.partitions.push(partition(1));
        disk.partitions.push(partition(2));
        disk.partitions.push(partition(3));

        // ACT
        let checks = checks(&disk, &fake);

        // ASSERT
        assert!(check(&checks, "foreign_holders").passed);
    }

    #[test]
    fn unreadable_holders_refuse_the_disk() {
        // ARRANGE
        let fake = Fake {
            unreadable: vec!["sda".to_owned()],
            ..Fake::default()
        };

        // ACT
        let checks = checks(&disk("sda"), &fake);

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
        let fake = Fake {
            swaps: vec![swap_entry("/dev/sda1")],
            ..Fake::default()
        };
        let mut disk = disk("sda");
        disk.partitions.push(partition(1));

        // ACT
        let checks = checks(&disk, &fake);

        // ASSERT
        let swap = check(&checks, "active_swap");
        assert!(!swap.passed);
        assert!(swap.detail.contains("/dev/sda1"), "{}", swap.detail);
    }

    #[test]
    fn swap_outside_the_inventory_is_resolved_through_the_kernel() {
        // ARRANGE
        let fake = Fake {
            parents: vec![("/dev/sda2".to_owned(), "sda".to_owned())],
            swaps: vec![swap_entry("/dev/sda2")],
            ..Fake::default()
        };

        // ACT
        let checks = checks(&disk("sda"), &fake);

        // ASSERT
        let swap = check(&checks, "active_swap");
        assert!(!swap.passed);
        assert!(swap.detail.contains("/dev/sda2"), "{}", swap.detail);
    }

    #[test]
    fn swap_on_an_unrelated_disk_is_ignored() {
        // ARRANGE
        let fake = Fake {
            swaps: vec![swap_entry("/dev/sdb1")],
            ..Fake::default()
        };

        // ACT
        let checks = checks(&disk("sda"), &fake);

        // ASSERT
        assert!(check(&checks, "active_swap").passed);
    }

    #[test]
    fn a_clean_disk_passes_every_check() {
        // ARRANGE
        let fake = Fake::default();
        let mut disk = disk("sda");
        disk.partitions.push(partition(1));

        // ACT
        let checks = checks(&disk, &fake);

        // ASSERT
        assert!(checks.iter().all(|check| check.passed));
    }
}
