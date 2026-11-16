//! Root filesystem switch.
//!
//! The switch is the last of the three mount steps, because it is the one that
//! makes the mounts of the previous two reachable: after `pivot_root` the old
//! root is gone and only what was mounted below the new root is left.
//!
//! `apply` needs `rootfs` and `spawn`: the child path between fork and exec is
//! its only caller, and [`crate::sys::linux`] carries the calls it makes.
//!
//! # No `chroot` fallback
//!
//! `chroot` keeps the process in the host mount namespace, so a confined
//! process could still reach a mount it finds a path to, and it can be escaped
//! by a process that keeps a directory descriptor. A fallback that silently
//! weakens isolation is exactly the fail open behaviour this crate exists to
//! avoid, so a root filesystem without a mount namespace is refused during
//! resolution instead.
//!
//! Mount teardown is absent for the same reason it is absent from
//! [`crate::mount`]: the kernel destroys the mount namespace and everything in
//! it when the last process in it exits.

#[cfg(all(feature = "rootfs", feature = "spawn"))]
use std::path::Path;

#[cfg(all(feature = "rootfs", feature = "spawn"))]
use crate::error::Errno;
#[cfg(all(feature = "rootfs", feature = "spawn"))]
use crate::sys::linux;

/// Switches the child into `root` as its root directory.
#[cfg(all(feature = "rootfs", feature = "spawn"))]
pub(crate) fn apply(root: &Path) -> Result<(), Errno> {
    // The old root lands on the current directory inside the new root, so both
    // arguments of the switch are the same relative path.
    let current = Path::new(".");

    // 1. `pivot_root` refuses a new root that is not a mount point, so the new
    //    root is bound onto itself first. The mount is recursive so that every
    //    mount already below it moves with it.
    linux::rootfs::bind(root)?;

    // 2. The remaining steps name the new root relatively, so the working
    //    directory has to be inside it before the switch.
    linux::rootfs::chdir(root)?;

    // 3. `pivot_root(".", ".")` puts the new root in place and leaves the old
    //    root mounted on the current directory, which is now the new root.
    linux::rootfs::pivot_root(current, current)?;

    // 4. Detaching the old root drops it for good. A plain unmount would fail
    //    while anything still refers to it, and nothing can refer to a root
    //    that is already detached.
    linux::rootfs::detach(current)?;

    // 5. The working directory is the old root until this point, so the child
    //    has to leave it before the workload starts.
    linux::rootfs::chdir(Path::new("/"))
}
