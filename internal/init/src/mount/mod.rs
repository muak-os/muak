//! Mount operations for early boot.
//!
//! Provides functions to mount pseudo filesystems and the root filesystem
//! using squashfs and overlay mounts.

mod extensions;
mod partition;
mod squashfs;

use std::ffi::CString;
use std::path::Path;

use anyhow::{Context, Result};
use rustix::fs::{CWD, Mode, mkdirat};
use rustix::mount::{MountFlags, mount};

use partition::{enable_btrfs_quota, find_partition_by_partname};
use squashfs::attach_squashfs;

/// Mount pseudo filesystems required for early boot.
pub fn mount_pseudo() -> Result<()> {
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

/// Mount the root filesystem with extensions as overlays.
pub fn mount_rootfs() -> Result<()> {
    let newroot = Path::new("/newroot");
    if !newroot.exists() {
        mkdirat(CWD, newroot, Mode::from_bits_truncate(0o755))
            .context("Failed to create /newroot")?;
    }

    let work_dir = Path::new("/overlay");
    mkdirat(CWD, work_dir, Mode::from_bits_truncate(0o755)).context("Failed to create /overlay")?;

    let mut lower_dirs = Vec::new();

    let base_mount = work_dir.join("base");
    mkdirat(CWD, &base_mount, Mode::from_bits_truncate(0o755))
        .context("Failed to create /overlay/base")?;
    let base_mount_str = base_mount
        .to_str()
        .context("base mount path contains invalid UTF-8")?;
    attach_squashfs("/rootfs.sqsh", "/dev/loop0", base_mount_str)?;
    lower_dirs.push(base_mount_str.to_string());

    let extensions = extensions::discover_extensions();

    if !extensions.is_empty() {
        kmsg::info!("Loading {} extension(s)", extensions.len());
    }

    for (idx, ext_path) in extensions.iter().enumerate() {
        let ext_name = Path::new(ext_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("ext");

        let ext_mount = work_dir.join(ext_name);
        mkdirat(CWD, &ext_mount, Mode::from_bits_truncate(0o755))
            .context("Failed to create extension mount point")?;

        let loop_dev = format!("/dev/loop{}", idx + 1);
        let ext_mount_str = ext_mount
            .to_str()
            .context("extension mount path contains invalid UTF-8")?;
        attach_squashfs(ext_path, &loop_dev, ext_mount_str)?;
        lower_dirs.push(ext_mount_str.to_string());
    }

    if lower_dirs.len() == 1 {
        mount(
            lower_dirs[0].as_str(),
            "/newroot",
            "",
            MountFlags::BIND | MountFlags::RDONLY | MountFlags::NODEV,
            None,
        )
        .context("Failed to bind mount rootfs")?;
    } else {
        let options = format!("lowerdir={}", lower_dirs.join(":"));
        let options_cstr = CString::new(options.as_str()).expect("CString conversion failed");

        mount(
            "overlay",
            "/newroot",
            "overlay",
            MountFlags::RDONLY | MountFlags::NODEV,
            Some(options_cstr.as_c_str()),
        )
        .context("Failed to mount overlay rootfs")?;
    }

    Ok(())
}

/// Mount persistent STATE and DATA partitions if the system is installed.
pub fn mount_persistent() -> Result<bool> {
    let Some(state_dev) = find_partition_by_partname("STATE") else {
        return Ok(false);
    };

    let state_dir = Path::new("/run/state");
    if !state_dir.exists() {
        mkdirat(CWD, state_dir, Mode::from_bits_truncate(0o755))
            .context("Failed to create /run/state")?;
    }

    mount(
        state_dev.as_str(),
        "/run/state",
        "btrfs",
        MountFlags::empty(),
        None,
    )
    .context("Failed to mount STATE partition")?;

    kmsg::info!("Mounted STATE partition at /run/state");

    if let Some(data_dev) = find_partition_by_partname("DATA") {
        let data_dir = Path::new("/run/data");
        if !data_dir.exists() {
            mkdirat(CWD, data_dir, Mode::from_bits_truncate(0o755))
                .context("Failed to create /run/data")?;
        }

        mount(
            data_dev.as_str(),
            "/run/data",
            "btrfs",
            MountFlags::empty(),
            None,
        )
        .context("Failed to mount DATA partition")?;

        kmsg::info!("Mounted DATA partition at /run/data");
        enable_btrfs_quota("/run/data")?;
    }

    Ok(true)
}

/// Create a directory if it does not exist and mount a filesystem.
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
            .with_context(|| format!("Failed to create mount target: {}", target))?;
    }

    let data_cstring = data.map(|s| CString::new(s).expect("CString conversion failed"));

    mount(source, target, fstype, flags, data_cstring.as_deref())
        .with_context(|| format!("Failed to mount {} to {}", source, target))?;

    Ok(())
}
