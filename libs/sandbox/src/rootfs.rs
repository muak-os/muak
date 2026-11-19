//! Root filesystem switch.

#[cfg(all(feature = "rootfs", feature = "spawn"))]
use std::path::Path;

#[cfg(all(feature = "rootfs", feature = "spawn"))]
use crate::error::Errno;
#[cfg(all(feature = "rootfs", feature = "spawn"))]
use crate::sys::linux;

/// Switches the child into `root` as its root directory.
#[cfg(all(feature = "rootfs", feature = "spawn"))]
pub(crate) fn apply(root: &Path) -> Result<(), Errno> {
    let current = Path::new(".");
    linux::rootfs::bind(root)?;
    linux::rootfs::chdir(root)?;
    linux::rootfs::pivot_root(current, current)?;
    linux::rootfs::detach(current)?;

    linux::rootfs::chdir(Path::new("/"))
}
