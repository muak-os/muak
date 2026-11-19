//! Namespace syscalls.

use crate::error::Errno;
use crate::namespace::Set;

pub(crate) fn unshare(set: Set) -> Result<(), Errno> {
    // SAFETY: no descriptor table flag is ever requested, the only hazard `unshare_unsafe` documents, and a single thread process unsharing its own namespaces has nothing to keep in step.
    unsafe { rustix::thread::unshare_unsafe(flags(set)) }.map_err(Errno::from)
}

fn flags(set: Set) -> rustix::thread::UnshareFlags {
    let mut flags = rustix::thread::UnshareFlags::empty();
    if set.contains(Set::MOUNT) {
        flags |= rustix::thread::UnshareFlags::NEWNS;
    }
    if set.contains(Set::NETWORK) {
        flags |= rustix::thread::UnshareFlags::NEWNET;
    }
    if set.contains(Set::IPC) {
        flags |= rustix::thread::UnshareFlags::NEWIPC;
    }
    if set.contains(Set::UTS) {
        flags |= rustix::thread::UnshareFlags::NEWUTS;
    }
    if set.contains(Set::CGROUP) {
        flags |= rustix::thread::UnshareFlags::NEWCGROUP;
    }

    flags
}
