//! Selection of a physical disk from a config selector.

use anyhow::{Result, bail};
use blockdev::types::Disk;
use config::system::disk::Selector;

use super::pattern;

/// Returns true when `disk` satisfies every matcher present on a selector.
pub(crate) fn matches(selector: &Selector, disk: &Disk) -> bool {
    selector.device.is_none()
        && !disk.read_only
        && string_matches(selector.serial.as_deref(), &disk.serial)
        && string_matches(selector.wwid.as_deref(), &disk.wwid)
        && string_matches(selector.model.as_deref(), &disk.model)
        && string_matches(selector.subsystem.as_deref(), &disk.subsystem)
        && string_matches(selector.bus_path.as_deref(), &disk.bus_path)
        && selector
            .rotational
            .is_none_or(|expected| expected == disk.rotational)
}

/// Resolves the disk selected by `selector`, refusing ambiguous matches.
pub(crate) fn resolve<'a>(selector: &Selector, disks: &'a [Disk]) -> Result<&'a Disk> {
    let candidates: Vec<&Disk> = disks
        .iter()
        .filter(|disk| matches(selector, disk))
        .collect();

    if candidates.is_empty() {
        let available: Vec<&str> = disks.iter().map(|disk| disk.name.as_str()).collect();
        bail!(
            "no disk matches {} (available: {})",
            describe(selector),
            names(&available)
        );
    }

    if let [selected] = *candidates.as_slice() {
        return Ok(selected);
    }

    let identified: Vec<String> = candidates.iter().map(|disk| identify(disk)).collect();
    bail!(
        "{} matches multiple disks: {}; pin one with device, serial or wwid",
        describe(selector),
        identified.join(", ")
    );
}

/// Renders a human readable description of a disk selector for messages.
pub(crate) fn describe(selector: &Selector) -> String {
    if let Some(device) = selector.device.as_deref() {
        return format!("device {device}");
    }

    let mut parts = Vec::new();
    push_matcher(&mut parts, "serial", selector.serial.as_deref());
    push_matcher(&mut parts, "wwid", selector.wwid.as_deref());
    push_matcher(&mut parts, "model", selector.model.as_deref());
    push_matcher(&mut parts, "subsystem", selector.subsystem.as_deref());
    push_matcher(&mut parts, "bus_path", selector.bus_path.as_deref());
    if let Some(rotational) = selector.rotational {
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

fn identify(disk: &Disk) -> String {
    let mut facts = Vec::new();
    if !disk.serial.is_empty() {
        facts.push(format!("serial {}", disk.serial));
    }
    if !disk.wwid.is_empty() {
        facts.push(format!("wwid {}", disk.wwid));
    }

    if facts.is_empty() {
        return disk.name.clone();
    }

    format!("{} ({})", disk.name, facts.join(", "))
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
    use super::*;

    fn disk(name: &str) -> Disk {
        Disk {
            name: name.to_owned(),
            path: format!("/dev/{name}"),
            ..Disk::default()
        }
    }

    fn selector() -> Selector {
        Selector::default()
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
        let entry = Selector::device("/dev/sda");

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
    fn resolve_returns_the_only_match() {
        // ARRANGE
        let mut matched = disk("sda");
        matched.serial = "SN123".to_owned();
        let mut other = disk("sdb");
        other.serial = "SN456".to_owned();
        let disks = vec![matched, other];
        let mut entry = selector();
        entry.serial = Some("SN123".to_owned());

        // ACT
        let selected = resolve(&entry, &disks).unwrap();

        // ASSERT
        assert_eq!(selected.name, "sda");
    }

    #[test]
    fn resolve_errors_on_several_matches() {
        // ARRANGE
        let mut first = disk("sda");
        first.serial = "SN123".to_owned();
        first.wwid = "naa.5000".to_owned();
        let mut second = disk("sdb");
        second.serial = "SN456".to_owned();
        second.wwid = "naa.6000".to_owned();
        let disks = vec![first, second];
        let mut entry = selector();
        entry.rotational = Some(false);

        // ACT
        let error = resolve(&entry, &disks).unwrap_err();

        // ASSERT
        let message = error.to_string();
        assert!(message.contains("multiple disks"), "{message}");
        assert!(message.contains("sda"), "{message}");
        assert!(message.contains("sdb"), "{message}");
        assert!(message.contains("SN123"), "{message}");
        assert!(message.contains("SN456"), "{message}");
        assert!(message.contains("naa.5000"), "{message}");
        assert!(message.contains("naa.6000"), "{message}");
        assert!(message.contains("pin one"), "{message}");
    }
}
