//! Resolution of the configured disk selector to kernel device paths.

use anyhow::{Context as _, Result, bail};
use blockdev::types::Disk;
use config::system::disk::Selector;

use super::{list_disks, select};

/// Device path resolved from the configured system disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Targets {
    /// Kernel device path of the system disk.
    pub system: String,
}

/// Resolves the configured disk selector against one inventory snapshot.
pub(crate) fn resolve_with(selector: &Selector, disks: &[Disk]) -> Result<Targets> {
    Ok(Targets {
        system: resolve_device(selector, disks)?,
    })
}

/// Resolves only the configured system disk to its kernel device path.
pub(crate) async fn resolve_system(selector: &Selector) -> Result<String> {
    let disks = list_disks().await?;

    resolve_device(selector, &disks)
}

fn resolve_device(selector: &Selector, disks: &[Disk]) -> Result<String> {
    if selector.is_empty() {
        bail!("config has no disk selector");
    }

    if let Some(path) = selector.device.as_deref() {
        return canonical_device(path);
    }

    resolve_selector(selector, disks)
}

fn resolve_selector(selector: &Selector, disks: &[Disk]) -> Result<String> {
    Ok(select::resolve(selector, disks)?.path.clone())
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
    fn resolve_with_rejects_an_empty_selector() {
        // ARRANGE
        let selector = Selector::default();

        // ACT
        let error = resolve_with(&selector, &[]).unwrap_err();

        // ASSERT
        assert!(error.to_string().contains("no disk selector"), "{error}");
    }

    #[test]
    fn resolve_with_resolves_a_device_selector() {
        // ARRANGE
        let dir = tempfile::TempDir::new().expect("temp dir");
        std::fs::write(dir.path().join("sda"), b"").expect("device file");
        let system_link = dir.path().join("system");
        std::os::unix::fs::symlink(dir.path().join("sda"), &system_link).expect("system link");
        let selector = Selector::device(system_link.to_str().expect("utf8"));

        // ACT
        let targets = resolve_with(&selector, &[]).expect("system disk");

        // ASSERT
        assert_eq!(targets.system, "/dev/sda");
    }

    #[test]
    fn resolve_with_uses_the_given_snapshot() {
        // ARRANGE
        let disks = vec![disk("sda", "SN123")];
        let selector = Selector {
            serial: Some("SN123".to_owned()),
            ..Selector::default()
        };

        // ACT
        let targets = resolve_with(&selector, &disks).expect("targets");

        // ASSERT
        assert_eq!(targets.system, "/dev/sda");
    }

    #[test]
    fn selector_resolution_uses_fake_disks() {
        // ARRANGE
        let disks = vec![disk("sda", "SN123"), disk("sdb", "SN456")];
        let selector = Selector {
            serial: Some("SN456".to_owned()),
            ..Selector::default()
        };

        // ACT
        let path = resolve_selector(&selector, &disks).expect("selector path");

        // ASSERT
        assert_eq!(path, "/dev/sdb");
    }
}
