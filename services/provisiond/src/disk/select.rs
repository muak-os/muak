//! Selection of a physical disk from a config selector entry.

use anyhow::{Result, bail};
use config::system::disk::Entry;

use super::pattern;
use super::types::DiskInfo;

/// Returns true when `disk` satisfies every matcher present on a selector `entry`.
pub(crate) fn matches(entry: &Entry, disk: &DiskInfo) -> bool {
    entry.device.is_none()
        && !disk.read_only
        && string_matches(entry.serial.as_deref(), &disk.serial)
        && string_matches(entry.wwid.as_deref(), &disk.wwid)
        && string_matches(entry.model.as_deref(), &disk.model)
        && string_matches(entry.subsystem.as_deref(), &disk.subsystem)
        && string_matches(entry.bus_path.as_deref(), &disk.bus_path)
        && entry
            .rotational
            .is_none_or(|expected| expected == disk.rotational)
}

/// Resolves the disk selected by `entry`, warning about discarded candidates.
pub(crate) fn resolve<'a>(entry: &Entry, disks: &'a [DiskInfo]) -> Result<&'a DiskInfo> {
    let mut candidates = disks.iter().filter(|disk| matches(entry, disk));

    let Some(selected) = candidates.next() else {
        let available: Vec<&str> = disks.iter().map(|disk| disk.name.as_str()).collect();
        bail!(
            "no disk matches {} (available: {})",
            describe(entry),
            names(&available)
        );
    };

    let discarded: Vec<&str> = candidates.map(|disk| disk.name.as_str()).collect();
    if !discarded.is_empty() {
        kmsg::warn!(
            "multiple disks match {}: selected '{}', ignored {}",
            describe(entry),
            selected.name,
            names(&discarded)
        );
    }

    Ok(selected)
}

/// Renders a human readable description of a disk entry for messages.
pub(crate) fn describe(entry: &Entry) -> String {
    if let Some(device) = entry.device.as_deref() {
        return format!("device {device}");
    }

    let mut parts = Vec::new();
    push_matcher(&mut parts, "serial", entry.serial.as_deref());
    push_matcher(&mut parts, "wwid", entry.wwid.as_deref());
    push_matcher(&mut parts, "model", entry.model.as_deref());
    push_matcher(&mut parts, "subsystem", entry.subsystem.as_deref());
    push_matcher(&mut parts, "bus_path", entry.bus_path.as_deref());
    if let Some(rotational) = entry.rotational {
        parts.push(format!("rotational={rotational}"));
    }

    if parts.is_empty() {
        "selector".to_owned()
    } else {
        parts.join(" ")
    }
}

fn string_matches(pattern: Option<&str>, value: &str) -> bool {
    pattern.is_none_or(|pattern| pattern::matches(pattern, value))
}

fn push_matcher(parts: &mut Vec<String>, name: &str, value: Option<&str>) {
    if let Some(value) = value {
        parts.push(format!("{name}={value}"));
    }
}

fn names(disks: &[&str]) -> String {
    if disks.is_empty() {
        "none".to_owned()
    } else {
        disks.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use config::system::disk::Kind;

    use super::*;

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

    fn selector() -> Entry {
        Entry::selector(Kind::System)
    }

    #[test]
    fn matches_each_matcher_field() {
        // ARRANGE
        let mut disk = disk("sda");
        disk.serial = "SN123".to_owned();
        disk.wwid = "naa.5000".to_owned();
        disk.model = "Samsung SSD".to_owned();
        disk.subsystem = "scsi".to_owned();
        disk.bus_path = "/pci0000:00/0000:00:1f.2/ata1".to_owned();
        disk.rotational = true;

        let mut entry = selector();
        entry.serial = Some("SN123".to_owned());
        entry.wwid = Some("naa.5000".to_owned());
        entry.model = Some("Samsung SSD".to_owned());
        entry.subsystem = Some("scsi".to_owned());
        entry.bus_path = Some("/pci0000:00/0000:00:1f.2/ata1".to_owned());
        entry.rotational = Some(true);

        // ACT
        let matched = matches(&entry, &disk);

        // ASSERT
        assert!(matched);
    }

    #[test]
    fn matches_accepts_each_matcher_on_its_own() {
        // ARRANGE
        let mut disk = disk("sda");
        disk.serial = "SN123".to_owned();
        disk.wwid = "naa.5000".to_owned();
        disk.model = "Samsung SSD".to_owned();
        disk.subsystem = "scsi".to_owned();
        disk.bus_path = "/pci0000:00/ata1".to_owned();
        disk.rotational = true;

        // ACT & ASSERT
        let mut entry = selector();
        entry.serial = Some("SN123".to_owned());
        assert!(matches(&entry, &disk), "serial matcher");
        entry = selector();
        entry.wwid = Some("naa.5000".to_owned());
        assert!(matches(&entry, &disk), "wwid matcher");
        entry = selector();
        entry.model = Some("Samsung SSD".to_owned());
        assert!(matches(&entry, &disk), "model matcher");
        entry = selector();
        entry.subsystem = Some("scsi".to_owned());
        assert!(matches(&entry, &disk), "subsystem matcher");
        entry = selector();
        entry.bus_path = Some("/pci0000:00/ata1".to_owned());
        assert!(matches(&entry, &disk), "bus_path matcher");
        entry = selector();
        entry.rotational = Some(true);
        assert!(matches(&entry, &disk), "rotational matcher");
    }

    #[test]
    fn matches_supports_glob_patterns() {
        // ARRANGE
        let mut disk = disk("nvme0n1");
        disk.serial = "S3Z8NX0M123456".to_owned();
        disk.subsystem = "nvme".to_owned();

        let mut entry = selector();
        entry.serial = Some("S3Z8*456".to_owned());
        entry.subsystem = Some("nvm?".to_owned());

        // ACT
        let matched = matches(&entry, &disk);

        // ASSERT
        assert!(matched);
    }

    #[test]
    fn matches_requires_all_matchers() {
        // ARRANGE
        let mut disk = disk("sda");
        disk.serial = "SN123".to_owned();
        disk.model = "Other".to_owned();

        let mut entry = selector();
        entry.serial = Some("SN123".to_owned());
        entry.model = Some("Samsung SSD".to_owned());

        // ACT
        let matched = matches(&entry, &disk);

        // ASSERT
        assert!(!matched, "every matcher must hold");
    }

    #[test]
    fn matches_rejects_device_entries() {
        // ARRANGE
        let mut disk = disk("sda");
        disk.serial = "SN123".to_owned();
        let entry = Entry::device(Kind::System, "/dev/sda");

        // ACT
        let matched = matches(&entry, &disk);

        // ASSERT
        assert!(!matched, "device entries are not selectors");
    }

    #[test]
    fn matches_rejects_read_only_disks() {
        // ARRANGE
        let mut disk = disk("sda");
        disk.serial = "SN123".to_owned();
        disk.read_only = true;

        let mut entry = selector();
        entry.serial = Some("SN123".to_owned());

        // ACT
        let matched = matches(&entry, &disk);

        // ASSERT
        assert!(!matched);
    }

    #[test]
    fn resolve_errors_when_nothing_matches() {
        // ARRANGE
        let disks = vec![disk("sda"), disk("sdb")];
        let mut entry = selector();
        entry.serial = Some("missing".to_owned());

        // ACT
        let error = resolve(&entry, &disks).unwrap_err();

        // ASSERT
        let message = error.to_string();
        assert!(message.contains("no disk matches"), "{message}");
        assert!(message.contains("sda, sdb"), "{message}");
    }

    #[test]
    fn resolve_returns_first_of_several_matches() {
        // ARRANGE
        let disks = vec![disk("sda"), disk("sdb")];
        let mut entry = selector();
        entry.rotational = Some(false);

        // ACT
        let selected = resolve(&entry, &disks).unwrap();

        // ASSERT
        assert_eq!(selected.name, "sda");
    }
}
