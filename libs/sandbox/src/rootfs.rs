//! Root filesystem switch.
//!
//! There is no `chroot` fallback: it keeps the process in the host mount namespace and can be escaped through a kept directory descriptor, so a root filesystem without a mount namespace is refused instead.

#[cfg(all(feature = "rootfs", feature = "spawn"))]
use std::path::Path;

#[cfg(all(feature = "rootfs", feature = "spawn"))]
use crate::error::Errno;
#[cfg(all(feature = "rootfs", feature = "spawn"))]
use crate::sys::linux;

/// Switches the child into `root` as its root directory.
#[cfg(all(feature = "rootfs", feature = "spawn"))]
pub(crate) fn apply(root: &Path) -> Result<(), Errno> {
    // The old root lands on the current directory inside the new root, so both arguments of the switch are the same relative path.
    let current = Path::new(".");

    // 1. `pivot_root` refuses a new root that is not a mount point, so the new root is bound onto itself first, recursively, so mounts already below it move with it.
    linux::rootfs::bind(root)?;

    // 2. The remaining steps name the new root relatively, so the working directory has to be inside it before the switch.
    linux::rootfs::chdir(root)?;

    // 3. `pivot_root(".", ".")` puts the new root in place and leaves the old root mounted on the current directory.
    linux::rootfs::pivot_root(current, current)?;

    // 4. A plain unmount would fail while anything still refers to the old root, and nothing can refer to a detached one.
    linux::rootfs::detach(current)?;

    // 5. The working directory is the old root until this point, so the child has to leave it before the workload starts.
    linux::rootfs::chdir(Path::new("/"))
}
