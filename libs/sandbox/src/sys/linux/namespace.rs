//! Namespace syscalls.
//!
//! Nothing here allocates, locks or panics, and `unshare` touches per process state only, which is why it can run before the mount steps.

use rustix::thread;

use crate::error::Errno;
use crate::namespace::Set;

pub(crate) fn unshare(set: Set) -> Result<(), Errno> {
    // SAFETY: no descriptor table flag is ever requested, the only hazard `unshare_unsafe` documents, and a single thread process unsharing its own namespaces has nothing to keep in step.
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
