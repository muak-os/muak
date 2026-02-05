use anyhow::{Context, Result, bail};
use rustix::fs::{Mode, OFlags, open};
use rustix::ioctl::{Opcode, Updater, ioctl, opcode};
use rustix::mount::{MountFlags, mount};

use super::sysfs::find_partition_by_partname;

const BTRFS_IOCTL_MAGIC: u8 = 0x94;
const BTRFS_QUOTA_CTL_ENABLE: u64 = 1;
const BTRFS_IOC_QUOTA_CTL: Opcode = opcode::read_write::<QuotaCtlArgs>(BTRFS_IOCTL_MAGIC, 40);

/// Represents the btrfs_ioctl_quota_ctl_args structure from kernel
#[repr(C)]
struct QuotaCtlArgs {
    cmd: u64,
    status: u64,
}

fn enable_btrfs_quota(mount_point: &str) -> Result<()> {
    let file = open(
        mount_point,
        OFlags::RDONLY | OFlags::DIRECTORY,
        Mode::empty(),
    )
    .context("Failed to open mount point")?;

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

    kmsg::info!(@ "granola", "Enabled btrfs quota on {}", mount_point);
    Ok(())
}

pub fn mount_partitions() -> Result<()> {
    if let Some(state_dev) = find_partition_by_partname("STATE") {
        std::fs::create_dir_all("/run/state")?;

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
        std::fs::create_dir_all("/run/data")?;

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
