use anyhow::{Context, Result, bail};
use rustix::mount::{MountFlags, mount};
use std::fs;
use std::process::Command;

use super::sysfs::find_partition_by_partname;

fn enable_btrfs_quota(mount_point: &str) -> Result<()> {
    let output = Command::new("/sbin/btrfs")
        .args(["quota", "enable", mount_point])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "Failed to enable btrfs quota on {}: {}",
            mount_point,
            stderr
        );
    }

    kmsg::info!(@ "granola", "Enabled btrfs quota on {}", mount_point);
    Ok(())
}

pub fn mount_partitions() -> Result<()> {
    if let Some(state_dev) = find_partition_by_partname("STATE") {
        fs::create_dir_all("/run/state")?;

        mount(
            state_dev.as_str(),
            "/run/state",
            "btrfs",
            MountFlags::empty(),
            None,
        )
        .context("Failed to mount STATE partition")?;

        kmsg::info!(@ "granola", "Mounted STATE partition at /run/state");
    } else {
        bail!("STATE partition not found");
    }

    if let Some(data_dev) = find_partition_by_partname("DATA") {
        fs::create_dir_all("/run/data")?;

        mount(
            data_dev.as_str(),
            "/run/data",
            "btrfs",
            MountFlags::empty(),
            None,
        )
        .context("Failed to mount DATA partition")?;

        kmsg::info!(@ "granola", "Mounted DATA partition at /run/data");

        enable_btrfs_quota("/run/data")?;
    } else {
        bail!("DATA partition not found");
    }

    Ok(())
}
