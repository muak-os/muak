//! Partition discovery utilities.

use std::fs;
use std::path::Path;

/// Find a partition by its GPT partition name via sysfs.
pub fn find_partition_by_partname(partname: &str) -> Option<String> {
    let entries = fs::read_dir("/sys/class/block").ok()?;

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();

        if !entry.path().join("partition").exists() {
            continue;
        }

        let uevent = entry.path().join("uevent");
        let content = fs::read_to_string(&uevent).ok()?;
        if !matches_partname(&content, partname) {
            continue;
        }

        let dev_path = format!("/dev/{}", name);
        if Path::new(&dev_path).exists() {
            return Some(dev_path);
        }
    }

    None
}

/// Returns true if `uevent_content` contains a `PARTNAME=<partname>` line.
fn matches_partname(uevent_content: &str, partname: &str) -> bool {
    let target = format!("PARTNAME={}", partname);
    uevent_content.lines().any(|line| line.trim() == target)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn matches_partname_matches_data_partition() {
        // ARRANGE
        let uevent = "DEVNAME=nvme0n1p3\nPARTNAME=DATA\nDEVTYPE=partition\n";

        // ACT + ASSERT
        assert!(matches_partname(uevent, "DATA"));
    }
}
