//! Namespace syscalls.
//!
//! The child path reaches this module between fork and exec, so nothing here
//! allocates, locks or panics. `unshare` touches per process state only, which
//! is why it is the one call that can run before the mount steps.

use rustix::thread;

use crate::error::Errno;
use crate::namespace::Set;

pub(crate) fn unshare(set: Set) -> Result<(), Errno> {
    // SAFETY: no descriptor table flag is ever requested, so the call cannot
    // leave this thread looking at a table its siblings do not share, which is
    // the only hazard `unshare_unsafe` documents. A single thread process
    // unsharing its own namespaces has nothing to keep in step.
    unsafe { thread::unshare_unsafe(flags(set)) }.map_err(Errno::from)
}

fn flags(set: Set) -> thread::UnshareFlags {
    let mut flags = thread::UnshareFlags::empty();
    if set.contains(Set::MOUNT) {
        flags |= thread::UnshareFlags::NEWNS;
    }
    if set.contains(Set::NETWORK) {
        flags |= thread::UnshareFlags::NEWNET;
    }
    if set.contains(Set::IPC) {
        flags |= thread::UnshareFlags::NEWIPC;
    }
    if set.contains(Set::UTS) {
        flags |= thread::UnshareFlags::NEWUTS;
    }
    if set.contains(Set::CGROUP) {
        flags |= thread::UnshareFlags::NEWCGROUP;
    }
    flags
}
