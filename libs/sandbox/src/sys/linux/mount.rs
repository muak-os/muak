//! Mount syscalls.
//!
//! Nothing here allocates, locks or panics: rustix allocates for a path over 256 bytes, which is why resolution refuses a longer one.

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
    // The recursive flag only means something on the call that creates the bind, and nothing on the remount below.
    if flags.recursive {
        bits |= MountFlags::REC;
    }
    // The type argument is ignored for a bind mount, so the empty string stands in for the null pointer rustix does not offer.
    mount::mount(source, target, c"", bits, None).map_err(Errno::from)?;

    // `MS_BIND` ignores every other flag passed with it, so a single call would silently drop the restrictions, and `MS_REMOUNT` with `MS_BIND` is what changes the per mount flags and not the superblock.
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
    // The type name doubles as the source name, which is what a caller reads back from `/proc/self/mountinfo`.
    mount::mount(kind, target, kind, carried(flags), data).map_err(Errno::from)
}

// The flags both kinds of mount carry, without the recursive bit, which only `bind` sets.
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
