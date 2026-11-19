//! Mount pseudo filesystems.

use core::ffi::CStr;
use std::path::Path;

use anyhow::{Context as _, Result};
use rustix::fs::{CWD, Mode, mkdirat};
use rustix::mount::{MountFlags, mount};

const FS: &[Fs] = &[
    Fs {
        target: "/dev",
        source: "devtmpfs",
        fstype: "devtmpfs",
        flags: MountFlags::NOSUID.union(MountFlags::NOEXEC),
        data: None,
        required: Required::Fatal,
    },
    Fs {
        target: "/proc",
        source: "proc",
        fstype: "proc",
        flags: MountFlags::NOSUID
            .union(MountFlags::NOEXEC)
            .union(MountFlags::NODEV),
        data: None,
        required: Required::Fatal,
    },
    Fs {
        target: "/sys",
        source: "sysfs",
        fstype: "sysfs",
        flags: MountFlags::NOSUID
            .union(MountFlags::NOEXEC)
            .union(MountFlags::NODEV),
        data: None,
        required: Required::Fatal,
    },
    Fs {
        target: "/run",
        source: "tmpfs",
        fstype: "tmpfs",
        flags: MountFlags::NOSUID.union(MountFlags::NODEV),
        data: Some(c"mode=0755"),
        required: Required::Fatal,
    },
    #[cfg(feature = "selinux")]
    Fs {
        target: "/sys/fs/selinux",
        source: "selinuxfs",
        fstype: "selinuxfs",
        flags: MountFlags::NOSUID
            .union(MountFlags::NOEXEC)
            .union(MountFlags::NODEV),
        data: None,
        required: Required::Fatal,
    },
    Fs {
        target: "/sys/kernel/security",
        source: "securityfs",
        fstype: "securityfs",
        flags: MountFlags::NOSUID
            .union(MountFlags::NOEXEC)
            .union(MountFlags::NODEV),
        data: None,
        required: Required::Optional,
    },
    Fs {
        target: "/sys/firmware/efi/efivars",
        source: "efivarfs",
        fstype: "efivarfs",
        flags: MountFlags::NOSUID
            .union(MountFlags::NOEXEC)
            .union(MountFlags::NODEV),
        data: None,
        required: Required::Optional,
    },
];

/// Top-level pseudo mounts that follow the root switch and survive ramfs cleanup.
pub(crate) const CORE: [&str; 4] = ["/dev", "/proc", "/sys", "/run"];

/// Boot policy for a pseudo filesystem mount.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Required {
    /// Boot aborts when the mount fails.
    Fatal,
    /// Failure is logged and boot continues.
    Optional,
}

/// A pseudo filesystem mounted during early boot.
pub(crate) struct Fs {
    target: &'static str,
    source: &'static str,
    fstype: &'static str,
    flags: MountFlags,
    data: Option<&'static CStr>,
    required: Required,
}

/// Mount the pseudo filesystems listed in `FS`.
pub(crate) fn pseudo() -> Result<()> {
    for entry in FS {
        match entry.required {
            Required::Fatal => create_and_mount(entry)?,
            Required::Optional => mount_optional(entry),
        }
    }

    Ok(())
}

fn mount_optional(entry: &Fs) {
    if let Err(e) = create_and_mount(entry) {
        kmsg::warn!("Optional mount {} unavailable: {:#}", entry.target, e);
    }
}

fn create_and_mount(entry: &Fs) -> Result<()> {
    if !Path::new(entry.target).exists() {
        _ = mkdirat(CWD, entry.target, Mode::from_bits_truncate(0o755));
    }

    mount(
        entry.source,
        entry.target,
        entry.fstype,
        entry.flags,
        entry.data,
    )
    .with_context(|| format!("Failed to mount {} to {}", entry.source, entry.target))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{FS, Required};

    #[cfg(feature = "selinux")]
    const FATAL: [&str; 5] = ["/dev", "/proc", "/sys", "/run", "/sys/fs/selinux"];

    #[cfg(not(feature = "selinux"))]
    const FATAL: [&str; 4] = ["/dev", "/proc", "/sys", "/run"];

    #[test]
    fn core_mounts_lead_the_table() {
        // ARRANGE
        let leading: Vec<&str> = FS
            .iter()
            .take(super::CORE.len())
            .map(|entry| entry.target)
            .collect();

        // ACT
        let core_leads = leading == super::CORE;

        // ASSERT
        assert!(core_leads, "core mounts must open the table: {leading:?}");
    }

    #[test]
    fn targets_are_unique() {
        // ARRANGE
        let targets: Vec<&str> = FS.iter().map(|entry| entry.target).collect();

        // ACT
        let mut deduplicated = targets.clone();
        deduplicated.sort_unstable();
        deduplicated.dedup();

        // ASSERT
        assert_eq!(deduplicated.len(), targets.len());
    }

    #[test]
    fn fatal_entries_lead_the_table() {
        // ARRANGE
        let fatal: Vec<&str> = FS
            .iter()
            .filter(|entry| entry.required == Required::Fatal)
            .map(|entry| entry.target)
            .collect();
        let leading: Vec<&str> = FS
            .iter()
            .take(fatal.len())
            .map(|entry| entry.target)
            .collect();

        // ACT
        let fatal_leads = fatal == leading;

        // ASSERT
        assert_eq!(fatal, FATAL);
        assert!(fatal_leads);
    }

    #[test]
    fn entries_after_the_fatal_block_are_optional() {
        // ARRANGE
        let trailing = FS.iter().skip(FATAL.len());

        // ACT
        let all_optional = trailing
            .clone()
            .all(|entry| entry.required == Required::Optional);

        // ASSERT
        assert!(all_optional);
        assert_eq!(trailing.count(), FS.len() - FATAL.len());
    }
}
