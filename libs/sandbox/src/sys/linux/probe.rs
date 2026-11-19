//! Availability probes for the syscalls detection cares about.

use std::io;

use rustix::io::Errno;
use rustix::mount::OpenTreeFlags;
use rustix::process::PidfdFlags;
use rustix::thread::CapabilitySet;
use rustix::{fs, mount, process, thread};

pub(crate) fn pivot_root() -> bool {
    present(&process::pivot_root("", ""))
}

pub(crate) fn open_tree() -> bool {
    present(&mount::open_tree(fs::CWD, "", OpenTreeFlags::empty()))
}

pub(crate) fn mount_setattr() -> bool {
    let flags: libc::c_long = 0;
    let size: libc::c_long = 0;
    // SAFETY: an empty path with an empty attribute and zero size names no mount, so the kernel rejects the call before touching any state.
    let result = unsafe {
        libc::syscall(
            libc::SYS_mount_setattr,
            libc::c_long::from(libc::AT_FDCWD),
            c"".as_ptr(),
            flags,
            core::ptr::null::<libc::c_void>(),
            size,
        )
    };

    present_raw(result)
}

pub(crate) fn clone3() -> bool {
    let size: libc::c_long = 0;
    // SAFETY: a null argument pointer with zero size passes `access_ok`, and the size check then rejects the call with EINVAL before anything is dereferenced.
    let result =
        unsafe { libc::syscall(libc::SYS_clone3, core::ptr::null::<libc::c_void>(), size) };

    present_raw(result)
}

pub(crate) fn pidfd() -> bool {
    present(&process::pidfd_open(process::getpid(), PidfdFlags::empty()))
}

pub(crate) fn capabilities() -> bool {
    present(&thread::capabilities(None))
}

pub(crate) fn cap_bounding() -> bool {
    present(&thread::capability_is_in_bounding_set(CapabilitySet::CHOWN))
}

fn present<T>(result: &rustix::io::Result<T>) -> bool {
    result.as_ref().err() != Some(&Errno::NOSYS)
}

fn present_raw(result: libc::c_long) -> bool {
    if result != -1 {
        return true;
    }

    io::Error::last_os_error().raw_os_error() != Some(libc::ENOSYS)
}
