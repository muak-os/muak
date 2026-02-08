use anyhow::{Result, bail};
use rustix::mount::{UnmountFlags, unmount};

pub fn unmount_partition(mount_point: &str) -> Result<()> {
    kmsg::info!(@ "reset", "Unmounting {}", mount_point);

    match unmount(mount_point, UnmountFlags::empty()) {
        Ok(()) => {
            kmsg::info!(@ "reset", "Unmounted {}", mount_point);
            Ok(())
        }
        Err(rustix::io::Errno::NOENT | rustix::io::Errno::INVAL) => {
            kmsg::warn!(@ "reset", "{} not mounted, skipping", mount_point);
            Ok(())
        }
        Err(e) => {
            bail!("Failed to unmount {}: {}", mount_point, e);
        }
    }
}
