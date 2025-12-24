use anyhow::{Context, Result, bail};
use nix::mount::{MsFlags, mount};
use std::fs;

use super::sysfs::find_partition_by_partname;

pub fn mount_partitions() -> Result<()> {
    if let Some(state_dev) = find_partition_by_partname("STATE") {
        fs::create_dir_all("/run/state")?;

        mount(
            Some(state_dev.as_str()),
            "/run/state",
            Some("btrfs"),
            MsFlags::empty(),
            None::<&str>,
        )
        .context("Failed to mount STATE partition")?;

        kmsg::info!(@ "granola", "Mounted STATE partition at /run/state");
    } else {
        bail!("STATE partition not found");
    }

    if let Some(data_dev) = find_partition_by_partname("DATA") {
        fs::create_dir_all("/run/data")?;

        mount(
            Some(data_dev.as_str()),
            "/run/data",
            Some("btrfs"),
            MsFlags::empty(),
            None::<&str>,
        )
        .context("Failed to mount DATA partition")?;

        kmsg::info!(@ "granola", "Mounted DATA partition at /run/data");
    } else {
        bail!("DATA partition not found");
    }

    Ok(())
}
