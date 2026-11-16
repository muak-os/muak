//! Root filesystem switch syscalls.
//!
//! The child path reaches this module between fork and exec, so nothing here
//! allocates, locks or panics and every path conversion stays below the stack
//! buffer limit that resolution enforces.

use std::path::Path;

use rustix::mount::{MountFlags, UnmountFlags};
use rustix::{mount, process};

use crate::error::Errno;

pub(crate) fn bind(root: &Path) -> Result<(), Errno> {
    // The type argument is ignored for a bind mount; the empty string stands in
    // for the null pointer the raw call takes.
    mount::mount(root, root, c"", MountFlags::BIND | MountFlags::REC, None).map_err(Errno::from)
}

pub(crate) fn chdir(path: &Path) -> Result<(), Errno> {
    process::chdir(path).map_err(Errno::from)
}

pub(crate) fn pivot_root(new_root: &Path, put_old: &Path) -> Result<(), Errno> {
    process::pivot_root(new_root, put_old).map_err(Errno::from)
}

pub(crate) fn detach(path: &Path) -> Result<(), Errno> {
    mount::unmount(path, UnmountFlags::DETACH).map_err(Errno::from)
}
