//! Mount syscalls.

use core::ffi::CStr;
use std::path::Path;

use rustix::mount;
use rustix::mount::{MountFlags, MountPropagationFlags};

use crate::error::Errno;
use crate::mount::Flags;

pub(crate) fn private_root() -> Result<(), Errno> {
    mount::mount_change(
        "/",
        MountPropagationFlags::REC | MountPropagationFlags::PRIVATE,
    )
    .map_err(Errno::from)
}

pub(crate) fn bind(source: &Path, target: &Path, flags: Flags) -> Result<(), Errno> {
    let mut bits = MountFlags::BIND;
    if flags.recursive {
        bits |= MountFlags::REC;
    }
    mount::mount(source, target, c"", bits, None).map_err(Errno::from)?;

    let carried = carried(flags);
    if carried.is_empty() {
        return Ok(());
    }

    mount::mount_remount(target, MountFlags::BIND | carried, c"").map_err(Errno::from)
}

pub(crate) fn filesystem(
    kind: &CStr,
    target: &Path,
    data: Option<&CStr>,
    flags: Flags,
) -> Result<(), Errno> {
    mount::mount(kind, target, kind, carried(flags), data).map_err(Errno::from)
}

fn carried(flags: Flags) -> MountFlags {
    let mut bits = MountFlags::empty();
    if flags.read_only {
        bits |= MountFlags::RDONLY;
    }
    if flags.no_suid {
        bits |= MountFlags::NOSUID;
    }
    if flags.no_dev {
        bits |= MountFlags::NODEV;
    }
    if flags.no_exec {
        bits |= MountFlags::NOEXEC;
    }

    bits
}
