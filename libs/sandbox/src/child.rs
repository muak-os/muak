//! The child path between fork and exec.
//!
//! Everything here runs in a process forked from a parent that may be
//! multithreaded. The parent may have owned an allocator lock, a panic lock or
//! a stdio lock at the instant of fork, and the thread that held it does not
//! exist in the child, so taking any of those locks would deadlock. The code
//! therefore allocates nothing, locks nothing, formats nothing, never panics
//! and only borrows what the parent already built.
//!
//! A failure cannot be formatted here. [`exec`] writes a fixed
//! `(stage code, errno)` pair to the report pipe and calls `_exit(127)`, and the
//! parent turns that pair into a [`crate::error::SandboxError`].

use core::ffi::c_char;
use std::os::fd::{AsRawFd as _, BorrowedFd, RawFd};

use crate::error::Errno;
use crate::policy::Policy;
use crate::stage::Stage;
#[cfg(feature = "limits")]
use crate::sys::linux::limits;
use crate::sys::linux::{credentials, spawn};

type Failure = (Stage, Errno);

/// # Safety
///
/// Must be called in the child of a successful fork whose parent built
/// `program`, `argv` and `envp` and whose descriptors are still open. The
/// function never returns.
pub(crate) unsafe fn exec(
    policy: &Policy<'_>,
    program: *const c_char,
    argv: *const *const c_char,
    envp: *const *const c_char,
    files: [RawFd; 3],
    pass: &[(BorrowedFd<'_>, RawFd)],
    report: RawFd,
) -> ! {
    if let Err((stage, errno)) = apply(policy) {
        fail(report, stage, errno);
    }

    // The descriptors belong to the exec step, not to the sandbox policy.
    let targets = [libc::STDIN_FILENO, libc::STDOUT_FILENO, libc::STDERR_FILENO];
    for (source, target) in files.into_iter().zip(targets) {
        if source >= 0_i32
            && let Err(errno) = spawn::redirect(source, target)
        {
            fail(report, Stage::Exec, errno);
        }
    }

    // Descriptors the caller wants the child to inherit are installed after
    // stdio, so stdio never clobbers a passed descriptor, and before the
    // exec, which is the only step that needs them. The duplication
    // drops the close-on-exec flag of the copy, but a descriptor already
    // sitting at its target was not copied, so the flag is cleared explicitly.
    for &(source, target) in pass {
        if let Err(errno) = spawn::redirect(source.as_raw_fd(), target) {
            fail(report, Stage::Exec, errno);
        }
        if let Err(errno) = spawn::clear_cloexec(target) {
            fail(report, Stage::Exec, errno);
        }
    }

    // The parent's handlers must not run in a process whose reactor is gone,
    // and the mask is cleared only after every sandbox step needed a signal.
    if let Err(errno) = spawn::restore_signals() {
        fail(report, Stage::Exec, errno);
    }

    // SAFETY: the pointers come from a `Cmd` the parent built and kept alive
    // across the fork, so they name NUL terminated strings and null terminated
    // pointer arrays in this copy of the address space. Execve only returns on
    // failure, and the returned number is reported below.
    let errno = unsafe { spawn::exec(program, argv, envp) };
    fail(report, Stage::Exec, errno);
}

fn apply(policy: &Policy<'_>) -> Result<(), Failure> {
    // 1. Signals first: the parent may be a multithreaded reactor whose handler
    //    would run in a process where the reactor no longer exists.
    trap(Stage::Signals, spawn::reset_signals())?;

    // 2. Limits before dropping privilege: Limit::Value sets the soft and the
    //    hard value, and raising a hard limit above the inherited maximum needs
    //    CAP_SYS_RESOURCE, which the uid drop removes.
    #[cfg(feature = "limits")]
    trap(Stage::Limits, limits::apply(&policy.limits))?;

    // 3. The bounding set before the uid drop: PR_CAPBSET_DROP needs
    //    CAP_SETPCAP, which a non-root identity no longer carries.
    #[cfg(feature = "caps")]
    if let Some(kept) = policy.caps {
        trap(Stage::Caps, credentials::bounding(kept))?;
    }

    // 4. Supplementary groups first, then gid, then uid: after setresuid the
    //    process can no longer call setgroups.
    if let Some(groups) = policy.id.groups {
        trap(Stage::Credentials, credentials::groups(groups))?;
    }
    trap(Stage::Credentials, credentials::gid(policy.id.gid))?;
    trap(Stage::Credentials, credentials::uid(policy.id.uid))?;

    // 5. The effective set after the uid drop, because the new identity is what
    //    the kernel checks when the permitted set changes.
    #[cfg(feature = "caps")]
    if let Some(kept) = policy.caps {
        trap(Stage::Caps, credentials::effective(kept))?;
    }

    // 6. No new privileges last: once set, later privileged operations fail, so
    //    every earlier step must already be done.
    if policy.no_new_privs {
        trap(Stage::NoNewPrivs, credentials::no_new_privs())?;
    }

    Ok(())
}

fn trap(stage: Stage, result: Result<(), Errno>) -> Result<(), Failure> {
    result.map_err(|errno| (stage, errno))
}

fn fail(report: RawFd, stage: Stage, errno: Errno) -> ! {
    let packed = u64::from(stage.code())
        | (u64::from(u32::from_le_bytes(errno.raw().to_le_bytes())) << 16_u32);
    let bytes = packed.to_le_bytes();
    let mut written = 0_usize;
    while written < bytes.len() {
        let remaining = bytes.len().saturating_sub(written);
        // SAFETY: the offset stays strictly below the eight byte array length,
        // so the pointer remains inside the array that outlives the call.
        let pointer = unsafe { bytes.as_ptr().add(written) };
        // SAFETY: `pointer` points to `remaining` initialized bytes of the
        // array above, which outlives the call.
        let result = unsafe { libc::write(report, pointer.cast(), remaining) };
        if result < 0 && !interrupted() {
            break;
        }
        if result > 0 {
            written = written.saturating_add(usize::try_from(result).unwrap_or(0));
        }
    }
    // SAFETY: `_exit` has no pointer preconditions, and the status fits the
    // integer argument the kernel expects.
    unsafe { libc::_exit(127) }
}

fn interrupted() -> bool {
    // SAFETY: `__errno_location` returns the current thread's errno slot, which
    // is always a valid readable pointer for the duration of the call.
    let location = unsafe { libc::__errno_location() };
    // SAFETY: `location` is the valid errno slot returned above and is read
    // before the call returns.
    unsafe { *location == libc::EINTR }
}

#[cfg(test)]
mod tests {
    use core::mem::size_of;
    use core::sync::atomic::{AtomicUsize, Ordering};
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::fs::File;
    use std::io::Read as _;

    use super::*;
    use crate::credentials::{Caps, Id};

    static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

    #[global_allocator]
    static GLOBAL: Counting = Counting;

    struct Counting;

    // SAFETY: every method forwards to the system allocator with the same
    // layout it received, and the counter update is an atomic that cannot
    // allocate or unwind.
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            // SAFETY: the layout is passed through unchanged to the system
            // allocator, which returns a pointer or null with the same
            // contract the caller expects.
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: the pointer and layout came from this allocator, which
            // forwards them unchanged to the system allocator.
            unsafe { System.dealloc(ptr, layout) }
        }
    }

    #[test]
    fn pre_exec_half_does_not_allocate() {
        // ARRANGE
        // SAFETY: `geteuid` and `getegid` read process state and dereference no
        // pointer, so the calls always succeed.
        let uid = unsafe { libc::geteuid() };
        // SAFETY: `getegid` reads process state and dereferences no pointer.
        let gid = unsafe { libc::getegid() };
        // As root a one entry group list is accepted, because the kernel
        // validates the count and not the gids, so only an unprivileged run
        // reaches the `setgroups` failure path. Either way the step must not
        // allocate.
        let groups = [u32::MAX];
        let policy = Policy::new()
            .id(Id::new(uid, gid).groups(&groups))
            .caps(Caps::all());
        let (read, write) = spawn::pipe().unwrap();
        let write_fd = write.as_raw_fd();

        // ACT
        // SAFETY: the test child runs only `apply`, which the crate keeps
        // allocation free and lock free, then writes a fixed integer and exits.
        let pid = unsafe { libc::fork() };
        if pid == 0_i32 {
            let before = ALLOCATIONS.load(Ordering::SeqCst);
            let outcome = apply(&policy);
            let after = ALLOCATIONS.load(Ordering::SeqCst);
            let delta = after.saturating_sub(before);
            let bytes = delta.to_le_bytes();
            // SAFETY: `bytes` is initialized and outlives the call, and the
            // write targets the pipe the parent still reads.
            unsafe {
                libc::write(write_fd, bytes.as_ptr().cast(), bytes.len());
            }
            // SAFETY: the child must not run destructors that belong to the
            // parent, so it leaves through `_exit` with a status the parent
            // does not inspect.
            unsafe {
                libc::_exit(i32::from(outcome.is_err()));
            };
        }
        drop(write);
        let mut bytes = [0_u8; size_of::<usize>()];
        let mut report = File::from(read);
        report.read_exact(&mut bytes).unwrap();
        let mut status = 0_i32;
        // SAFETY: `pid` is the child forked above and `status` is a valid
        // out pointer, so the kernel writes the wait status into it.
        unsafe {
            libc::waitpid(pid, &raw mut status, 0);
        };
        let delta = usize::from_le_bytes(bytes);

        // ASSERT
        assert_eq!(delta, 0, "the pre-exec half made {delta} allocations");
    }
}
