//! Resolution of configured disk entries to kernel device paths.

use anyhow::{Context as _, Result, bail};
use blockdev::types::Disk;
use config::system::disk::{self, Entry};

use super::{list_disks, select};

/// Device paths resolved from the configured disk entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Targets {
    /// Kernel device path of the system disk.
    pub system: String,
    /// Kernel device path of the data disk when it does not share the system disk.
    pub data: Option<String>,
}

impl Targets {
    /// Returns true when DATA lives on its own disk.
    pub fn is_split(&self) -> bool {
        self.data.is_some()
    }

    /// Returns the data disk path, falling back to the system disk when shared.
    pub fn data(&self) -> &str {
        self.data.as_deref().unwrap_or(&self.system)
    }
}

/// Resolves the configured entries into concrete device paths.
pub(crate) async fn resolve(entries: &[Entry]) -> Result<Targets> {
    let disks = list_disks().await?;

    resolve_with(entries, &disks)
}

/// Resolves the configured entries against one inventory snapshot.
pub(crate) fn resolve_with(entries: &[Entry], disks: &[Disk]) -> Result<Targets> {
    let system = system_device(entries, disks)?;

    let Some(data_entry) = disk::data(entries) else {
        return Ok(Targets { system, data: None });
    };

    let data = resolve_device(data_entry, disks)?;
    if data == system {
        bail!(
            "data disk resolves to the system disk '{system}', omit the data entry to share the system disk"
        );
    }

    Ok(Targets {
        system,
        data: Some(data),
    })
}

/// Resolves only the configured system disk to its kernel device path.
pub(crate) async fn resolve_system(entries: &[Entry]) -> Result<String> {
    let disks = list_disks().await?;

    system_device(entries, &disks)
}

fn system_device(entries: &[Entry], disks: &[Disk]) -> Result<String> {
    let entry = disk::system(entries).context("config has no system disk entry")?;

    resolve_device(entry, disks)
}

fn resolve_device(entry: &Entry, disks: &[Disk]) -> Result<String> {
    if let Some(path) = entry.device.as_deref() {
        return canonical_device(path);
    }

    resolve_selector(entry, disks)
}

fn resolve_selector(entry: &Entry, disks: &[Disk]) -> Result<String> {
    Ok(select::resolve(entry, disks)?.path.clone())
}

fn canonical_device(path: &str) -> Result<String> {
    let canonical =
        std::fs::canonicalize(path).with_context(|| format!("disk device '{path}' not found"))?;
    let name = canonical
        .file_name()
        .and_then(|name| name.to_str())
        .with_context(|| format!("disk device '{path}' has no file name"))?;

    Ok(format!("/dev/{name}"))
}

#[cfg(test)]
mod tests {
    use config::system::disk::Kind;

    use super::*;

    fn disk(name: &str, serial: &str) -> Disk {
        Disk {
            name: name.to_owned(),
            path: format!("/dev/{name}"),
            serial: serial.to_owned(),
            ..Disk::default()
        }
    }

    #[test]
    fn canonical_device_returns_dev_kernel_name() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("temp dir");
        std::fs::write(dir.path().join("sda"), b"").expect("device file");
        let link = dir.path().join("disk");
        std::os::unix::fs::symlink(dir.path().join("sda"), &link).expect("symlink");

        // ACT
        let device = canonical_device(link.to_str().expect("utf8 path")).expect("canonical");

        // ASSERT
        assert_eq!(device, "/dev/sda");
    }

    #[test]
    fn canonical_device_errors_on_missing_path() {
        // ARRANGE
        let path = "/nonexistent/muak-disk";

        // ACT
        let error = canonical_device(path).unwrap_err();

        // ASSERT
        assert!(error.to_string().contains("not found"), "{error}");
    }

    #[test]
    fn targets_without_data_share_system_disk() {
        // ARRANGE
        let targets = Targets {
            system: "/dev/sda".to_owned(),
            data: None,
        };

        // ACT & ASSERT
        assert!(!targets.is_split());
        assert_eq!(targets.data(), "/dev/sda");
    }

    #[test]
    fn targets_with_data_are_split() {
        // ARRANGE
        let targets = Targets {
            system: "/dev/sda".to_owned(),
            data: Some("/dev/sdb".to_owned()),
        };

        // ACT & ASSERT
        assert!(targets.is_split());
        assert_eq!(targets.data(), "/dev/sdb");
    }

    #[test]
    fn system_device_ignores_data_entry() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("temp dir");
        std::fs::write(dir.path().join("sda"), b"").expect("device file");
        let system_link = dir.path().join("system");
        std::os::unix::fs::symlink(dir.path().join("sda"), &system_link).expect("system link");
        let entries = vec![
            Entry::device(Kind::System, system_link.to_str().expect("utf8")),
            Entry::device(Kind::Data, "/nonexistent/muak-data"),
        ];

        // ACT
        let system = system_device(&entries, &[]).expect("system disk");

        // ASSERT
        assert_eq!(system, "/dev/sda");
    }

    #[test]
    fn duplicate_resolved_devices_error() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("temp dir");
        std::fs::write(dir.path().join("sda"), b"").expect("device file");
        let system_link = dir.path().join("system");
        let data_link = dir.path().join("data");
        std::os::unix::fs::symlink(dir.path().join("sda"), &system_link).expect("system link");
        std::os::unix::fs::symlink(dir.path().join("sda"), &data_link).expect("data link");
        let entries = vec![
            Entry::device(Kind::System, system_link.to_str().expect("utf8")),
            Entry::device(Kind::Data, data_link.to_str().expect("utf8")),
        ];

        // ACT
        let error = resolve_with(&entries, &[]).unwrap_err();

        // ASSERT
        assert!(
            error.to_string().contains("share the system disk"),
            "{error}"
        );
    }

    #[test]
    fn resolve_with_uses_the_given_snapshot() {
        // ARRANGE
        let disks = vec![disk("sda", "SN123"), disk("sdb", "SN456")];
        let mut system_entry = Entry::selector(Kind::System);
        system_entry.serial = Some("SN123".to_owned());
        let mut data_entry = Entry::selector(Kind::Data);
        data_entry.serial = Some("SN456".to_owned());
        let entries = vec![system_entry, data_entry];

        // ACT
        let targets = resolve_with(&entries, &disks).expect("targets");

        // ASSERT
        assert_eq!(targets.system, "/dev/sda");
        assert_eq!(targets.data.as_deref(), Some("/dev/sdb"));
    }

    #[test]
    fn resolve_with_shares_the_system_disk_without_a_data_entry() {
        // ARRANGE
        let disks = vec![disk("sda", "SN123")];
        let mut system_entry = Entry::selector(Kind::System);
        system_entry.serial = Some("SN123".to_owned());
        let entries = vec![system_entry];

        // ACT
        let targets = resolve_with(&entries, &disks).expect("targets");

        // ASSERT
        assert!(!targets.is_split());
        assert_eq!(targets.data(), "/dev/sda");
    }

    #[test]
    fn resolve_with_refuses_the_same_disk_for_system_and_data() {
        // ARRANGE
        let disks = vec![disk("sda", "SN123")];
        let mut system_entry = Entry::selector(Kind::System);
        system_entry.serial = Some("SN123".to_owned());
        let mut data_entry = Entry::selector(Kind::Data);
        data_entry.serial = Some("SN123".to_owned());
        let entries = vec![system_entry, data_entry];

        // ACT
        let error = resolve_with(&entries, &disks).unwrap_err();

        // ASSERT
        assert!(
            error.to_string().contains("share the system disk"),
            "{error}"
        );
    }

    #[test]
    fn selector_resolution_uses_fake_disks() {
        // ARRANGE
        let disks = vec![disk("sda", "SN123"), disk("sdb", "SN456")];
        let mut entry = Entry::selector(Kind::System);
        entry.serial = Some("SN456".to_owned());

        // ACT
        let path = resolve_selector(&entry, &disks).expect("selector path");

        // ASSERT
        assert_eq!(path, "/dev/sdb");
    }
}
