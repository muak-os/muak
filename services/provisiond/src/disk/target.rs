//! Resolution of configured disk entries to kernel device paths.

use anyhow::{Context as _, Result};
use blockdev::types::Disk;
use config::system::disk::{self, Entry};

use super::{list_disks, select};

/// Device path resolved from the configured system disk entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Targets {
    /// Kernel device path of the system disk.
    pub system: String,
}

/// Resolves the configured entries against one inventory snapshot.
pub(crate) fn resolve_with(entries: &[Entry], disks: &[Disk]) -> Result<Targets> {
    Ok(Targets {
        system: system_device(entries, disks)?,
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
    fn system_device_resolves_the_system_entry() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("temp dir");
        std::fs::write(dir.path().join("sda"), b"").expect("device file");
        let system_link = dir.path().join("system");
        std::os::unix::fs::symlink(dir.path().join("sda"), &system_link).expect("system link");
        let entries = vec![Entry::device(
            Kind::System,
            system_link.to_str().expect("utf8"),
        )];

        // ACT
        let system = system_device(&entries, &[]).expect("system disk");

        // ASSERT
        assert_eq!(system, "/dev/sda");
    }

    #[test]
    fn resolve_with_uses_the_given_snapshot() {
        // ARRANGE
        let disks = vec![disk("sda", "SN123")];
        let mut system_entry = Entry::selector(Kind::System);
        system_entry.serial = Some("SN123".to_owned());
        let entries = vec![system_entry];

        // ACT
        let targets = resolve_with(&entries, &disks).expect("targets");

        // ASSERT
        assert_eq!(targets.system, "/dev/sda");
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
