//! Mount pseudo filesystems.

use alloc::ffi::CString;
use std::path::Path;

use anyhow::{Context as _, Result};
use rustix::fs::{CWD, Mode, mkdirat};
use rustix::mount::{MountFlags, mount};

/// Mount pseudo filesystems required for early boot.
pub(crate) fn pseudo() -> Result<()> {
    create_and_mount(
        "/dev",
        "devtmpfs",
        "devtmpfs",
        MountFlags::NOSUID | MountFlags::NOEXEC,
        None,
    )?;
    create_and_mount(
        "/proc",
        "proc",
        "proc",
        MountFlags::NOSUID | MountFlags::NOEXEC | MountFlags::NODEV,
        None,
    )?;
    create_and_mount(
        "/sys",
        "sysfs",
        "sysfs",
        MountFlags::NOSUID | MountFlags::NOEXEC | MountFlags::NODEV,
        None,
    )?;
    create_and_mount(
        "/run",
        "tmpfs",
        "tmpfs",
        MountFlags::NOSUID | MountFlags::NODEV,
        Some("mode=0755"),
    )?;
    create_and_mount(
        "/sys/fs/selinux",
        "selinuxfs",
        "selinuxfs",
        MountFlags::NOSUID | MountFlags::NOEXEC | MountFlags::NODEV,
        None,
    )?;

    if Path::new("/sys/firmware/efi").exists() {
        create_and_mount(
            "/sys/firmware/efi/efivars",
            "efivarfs",
            "efivarfs",
            MountFlags::NOSUID | MountFlags::NOEXEC | MountFlags::NODEV,
            None,
        )?;
    }

    Ok(())
}

fn create_and_mount(
    target: &str,
    source: &str,
    fstype: &str,
    flags: MountFlags,
    data: Option<&str>,
) -> Result<()> {
    let path = Path::new(target);

    if !path.exists() {
        mkdirat(CWD, path, Mode::from_bits_truncate(0o755))
            .with_context(|| format!("Failed to create mount target: {target}"))?;
    }

    let data_cstring = data
        .map(|value| CString::new(value).context("CString conversion failed"))
        .transpose()?;

    mount(source, target, fstype, flags, data_cstring.as_deref())
        .with_context(|| format!("Failed to mount {source} to {target}"))?;

    Ok(())
}
