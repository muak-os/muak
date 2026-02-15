//! Partition discovery utilities.

use std::fs;
use std::path::Path;

/// Find a partition by its GPT partition name via sysfs.
pub fn find_partition_by_partname(partname: &str) -> Option<String> {
    let entries = fs::read_dir("/sys/class/block").ok()?;
    let target = format!("PARTNAME={}", partname);

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();

        if !entry.path().join("partition").exists() {
            continue;
        }

        let uevent = entry.path().join("uevent");
        let content = fs::read_to_string(&uevent).ok()?;
        let found = content.lines().any(|line| line.trim() == target);
        if !found {
            continue;
        }

        let dev_path = format!("/dev/{}", name);
        if Path::new(&dev_path).exists() {
            return Some(dev_path);
        }
    }

    None
}
