//! Partition discovery and Btrfs quota management.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use rustix::fs::{Mode, OFlags, open};
use rustix::ioctl::{Opcode, Updater, ioctl, opcode};

const BTRFS_IOCTL_MAGIC: u8 = 0x94;
const BTRFS_QUOTA_CTL_ENABLE: u64 = 1;
const BTRFS_IOC_QUOTA_CTL: Opcode = opcode::read_write::<QuotaCtlArgs>(BTRFS_IOCTL_MAGIC, 40);

#[repr(C)]
struct QuotaCtlArgs {
    cmd: u64,
    status: u64,
}

/// Find a partition by its GPT partition name via sysfs.
pub fn find_partition_by_partname(partname: &str) -> Option<String> {
    let entries = fs::read_dir("/sys/class/block").ok()?;
    let target = format!("PARTNAME={}", partname);

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();

        if !entry.path().join("partition").exists() {
            continue;
        }

        let uevent = entry.path().join("uevent");
        let content = fs::read_to_string(&uevent).ok()?;
        let found = content.lines().any(|line| line.trim() == target);
        if !found {
            continue;
        }

        let dev_path = format!("/dev/{}", name);
        if Path::new(&dev_path).exists() {
            return Some(dev_path);
        }
    }

    None
}

/// Enable Btrfs quota on a mounted partition.
pub fn enable_btrfs_quota(mount_point: &str) -> Result<()> {
    let file = open(
        mount_point,
        OFlags::RDONLY | OFlags::DIRECTORY,
        Mode::empty(),
    )
    .context("Failed to open mount point for btrfs quota")?;

    let mut args = QuotaCtlArgs {
        cmd: BTRFS_QUOTA_CTL_ENABLE,
        status: 0,
    };

    // SAFETY: ioctl is inherently unsafe, but Updater ensures proper argument passing
    unsafe {
        ioctl(
            &file,
            Updater::<BTRFS_IOC_QUOTA_CTL, QuotaCtlArgs>::new(&mut args),
        )
    }
    .map_err(|e| anyhow::anyhow!("Failed to enable btrfs quota: {}", e))?;

    kmsg::info!("Enabled btrfs quota on {}", mount_point);
    Ok(())
}
