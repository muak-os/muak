//! Mount syscalls.
//!
//! The child path reaches this module between fork and exec, so nothing here
//! allocates, locks or panics. Rustix converts a path through a stack buffer
//! below 256 bytes and allocates above it, which is why resolution refuses a
//! longer path instead of letting the conversion reach the allocator here.

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
    // A bind of a directory carries whatever is mounted below it, so the flag
    // that repeats the bind on every submount is only meaningful here. It is
    // also meaningless on the remount below, so it stays on this call.
    if flags.recursive {
        bits |= MountFlags::REC;
    }
    // The type argument is ignored for a bind mount, so the empty string stands
    // in for the null pointer the raw call takes and rustix does not offer.
    mount::mount(source, target, c"", bits, None).map_err(Errno::from)?;

    // The `MS_BIND` path ignores every other flag passed in the same call, so a
    // single call would silently drop the read only, suid, device and exec
    // restrictions and leave the caller with a mount that looks restricted and
    // is not. `MS_REMOUNT` and `MS_BIND` together are what make this second
    // call change the per mount flags of the bind instead of the superblock.
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
    // The type name doubles as the source name, which is what a caller reads
    // back from `/proc/self/mountinfo` and what a kernel log names.
    mount::mount(kind, target, kind, carried(flags), data).map_err(Errno::from)
}

// The flags both kinds of mount carry. The recursive bit is deliberately absent:
// a filesystem mount has no submounts for it to reach, so only `bind` sets it.
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
