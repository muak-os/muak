//! Resolution of configured disk entries to kernel device paths.

use anyhow::{Context as _, Result, bail};
use config::system::disk::{self, Entry};

use super::select;
use super::sysfs::list_disks;
use super::types::DiskInfo;

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
    let system = resolve_system(entries).await?;

    let Some(data_entry) = disk::data(entries) else {
        return Ok(Targets { system, data: None });
    };

    let data = resolve_device(data_entry).await?;
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
    let entry = disk::system(entries).context("config has no system disk entry")?;

    resolve_device(entry).await
}

async fn resolve_device(entry: &Entry) -> Result<String> {
    if let Some(path) = entry.device.as_deref() {
        return canonical_device(path);
    }

    let disks = list_disks().await?;

    resolve_selector(entry, &disks)
}

fn resolve_selector(entry: &Entry, disks: &[DiskInfo]) -> Result<String> {
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

    fn disk(name: &str, serial: &str) -> DiskInfo {
        DiskInfo {
            name: name.to_owned(),
            path: format!("/dev/{name}"),
            size_bytes: 0,
            model: String::new(),
            serial: serial.to_owned(),
            wwid: String::new(),
            bus_path: String::new(),
            subsystem: String::new(),
            rotational: false,
            removable: false,
            read_only: false,
            partitions: Vec::new(),
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

    #[tokio::test]
    async fn resolve_system_ignores_data_entry() {
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
        let system = resolve_system(&entries).await.expect("system disk");

        // ASSERT
        assert_eq!(system, "/dev/sda");
    }

    #[tokio::test]
    async fn duplicate_resolved_devices_error() {
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
        let error = resolve(&entries).await.unwrap_err();

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
