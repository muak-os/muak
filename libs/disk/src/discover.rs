//! Partition discovery via sysfs.

use std::fs;
use std::path::Path;

use crate::role::Role;

const SYS_CLASS_BLOCK: &str = "/sys/class/block";
const DEV_DIR: &str = "/dev";

/// Finds partition device paths by their GPT partition name via sysfs.
#[must_use]
pub fn find_by_partname(partname: &str) -> Vec<String> {
    find_by_partname_in(Path::new(SYS_CLASS_BLOCK), Path::new(DEV_DIR), partname)
}

/// Finds the device path of the partition carrying `role`.
#[must_use]
pub fn find_partition_device(role: Role) -> Option<String> {
    find_partition_device_in(Path::new(SYS_CLASS_BLOCK), Path::new(DEV_DIR), role)
}

fn find_partition_device_in(sysfs_dir: &Path, dev_dir: &Path, role: Role) -> Option<String> {
    find_by_partname_in(sysfs_dir, dev_dir, role.gpt_name())
        .into_iter()
        .next()
}

fn find_by_partname_in(sysfs_dir: &Path, dev_dir: &Path, partname: &str) -> Vec<String> {
    let Ok(entries) = fs::read_dir(sysfs_dir) else {
        return Vec::new();
    };

    let mut devices = Vec::new();

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();

        if !entry.path().join("partition").exists() {
            continue;
        }

        let uevent = entry.path().join("uevent");
        let Ok(content) = fs::read_to_string(&uevent) else {
            continue;
        };
        if !matches_partname(&content, partname) {
            continue;
        }

        let dev_path = dev_dir.join(name.as_ref());
        if dev_path.exists() {
            devices.push(dev_path.to_string_lossy().into_owned());
        }
    }

    devices.sort_unstable();

    devices
}

fn matches_partname(uevent_content: &str, partname: &str) -> bool {
    let target = format!("PARTNAME={partname}");

    uevent_content.lines().any(|line| line.trim() == target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_partition(sysfs: &Path, dev: &Path, name: &str, partname: &str) {
        let entry = sysfs.join(name);
        std::fs::create_dir_all(&entry).expect("create sysfs partition");
        std::fs::write(entry.join("partition"), b"1").expect("write partition marker");
        let uevent = format!("DEVNAME={name}\nDEVTYPE=partition\nPARTNAME={partname}\n");
        std::fs::write(entry.join("uevent"), uevent).expect("write uevent");
        std::fs::write(dev.join(name), b"").expect("create dev node placeholder");
    }

    #[test]
    fn matches_partname_finds_exact_match() {
        // ARRANGE
        let uevent = "MAJOR=8\nMINOR=1\nDEVNAME=sda1\nDEVTYPE=partition\nPARTNAME=STATE\n";

        // ACT + ASSERT
        assert!(matches_partname(uevent, "STATE"));
    }

    #[test]
    fn matches_partname_returns_false_when_absent() {
        // ARRANGE
        let uevent = "MAJOR=8\nMINOR=1\nDEVNAME=sda1\nPARTNAME=DATA\n";

        // ACT + ASSERT
        assert!(!matches_partname(uevent, "STATE"));
    }

    #[test]
    fn matches_partname_does_not_match_prefix() {
        // ARRANGE
        let uevent = "PARTNAME=STATE2\n";

        // ACT + ASSERT
        assert!(!matches_partname(uevent, "STATE"));
    }

    #[test]
    fn matches_partname_does_not_match_suffix() {
        // ARRANGE
        let uevent = "PARTNAME=MYSTATE\n";

        // ACT + ASSERT
        assert!(!matches_partname(uevent, "STATE"));
    }

    #[test]
    fn matches_partname_handles_whitespace_trimming() {
        // ARRANGE
        let uevent = "  PARTNAME=STATE  \n";

        // ACT + ASSERT
        assert!(matches_partname(uevent, "STATE"));
    }

    #[test]
    fn matches_partname_handles_empty_uevent() {
        // ARRANGE
        let uevent = "";

        // ACT + ASSERT
        assert!(!matches_partname(uevent, "STATE"));
    }

    #[test]
    fn find_by_partname_returns_matching_devices() {
        // ARRANGE
        let temp = tempfile::tempdir().expect("create tempdir");
        let sysfs = temp.path().join("sys");
        let dev = temp.path().join("dev");
        std::fs::create_dir_all(&sysfs).expect("create sysfs");
        std::fs::create_dir_all(&dev).expect("create dev");
        create_partition(&sysfs, &dev, "nvme0n1p2", "STATE");
        create_partition(&sysfs, &dev, "nvme0n1p3", "DATA");

        // ACT
        let partitions = find_by_partname_in(&sysfs, &dev, "STATE");

        // ASSERT
        assert_eq!(
            partitions,
            vec![dev.join("nvme0n1p2").to_string_lossy().into_owned()]
        );
    }

    #[test]
    fn find_by_partname_skips_unreadable_uevent() {
        // ARRANGE
        let temp = tempfile::tempdir().expect("create tempdir");
        let sysfs = temp.path().join("sys");
        let dev = temp.path().join("dev");
        std::fs::create_dir_all(&sysfs).expect("create sysfs");
        std::fs::create_dir_all(&dev).expect("create dev");
        let bad = sysfs.join("bad0p1");
        std::fs::create_dir_all(&bad).expect("create sysfs partition");
        std::fs::write(bad.join("partition"), b"1").expect("write partition marker");
        create_partition(&sysfs, &dev, "nvme0n1p2", "STATE");

        // ACT
        let partitions = find_by_partname_in(&sysfs, &dev, "STATE");

        // ASSERT
        assert_eq!(
            partitions,
            vec![dev.join("nvme0n1p2").to_string_lossy().into_owned()]
        );
    }

    #[test]
    fn find_by_partname_returns_sorted_matches() {
        // ARRANGE
        let temp = tempfile::tempdir().expect("create tempdir");
        let sysfs = temp.path().join("sys");
        let dev = temp.path().join("dev");
        std::fs::create_dir_all(&sysfs).expect("create sysfs");
        std::fs::create_dir_all(&dev).expect("create dev");
        create_partition(&sysfs, &dev, "nvme0n1p3", "STATE");
        create_partition(&sysfs, &dev, "nvme0n1p2", "STATE");

        // ACT
        let partitions = find_by_partname_in(&sysfs, &dev, "STATE");

        // ASSERT
        assert_eq!(
            partitions,
            vec![
                dev.join("nvme0n1p2").to_string_lossy().into_owned(),
                dev.join("nvme0n1p3").to_string_lossy().into_owned(),
            ]
        );
    }

    #[test]
    fn find_partition_device_resolves_by_role() {
        // ARRANGE
        let temp = tempfile::tempdir().expect("create tempdir");
        let sysfs = temp.path().join("sys");
        let dev = temp.path().join("dev");
        std::fs::create_dir_all(&sysfs).expect("create sysfs");
        std::fs::create_dir_all(&dev).expect("create dev");
        create_partition(&sysfs, &dev, "nvme0n1p2", "STATE");

        // ACT
        let device = find_partition_device_in(&sysfs, &dev, Role::State);

        // ASSERT
        assert_eq!(
            device,
            Some(dev.join("nvme0n1p2").to_string_lossy().into_owned()),
            "roles must resolve through their canonical GPT name"
        );
    }
}
