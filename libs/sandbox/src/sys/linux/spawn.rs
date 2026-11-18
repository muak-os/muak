//! Process creation, execution, signalling and reaping syscalls.
//!
//! A child is always created by `clone3`, never by `fork`: `fork` carries no pidfd and cannot place the child in a cgroup, and the kernel this system ships has both.
//!
//! `clone_args.pidfd` is a user pointer the kernel writes the pidfd number through, while `clone_args.cgroup` is a plain descriptor value.

use core::ffi::{c_char, c_int};
use core::mem::size_of;
use std::os::fd::{AsRawFd as _, BorrowedFd, FromRawFd as _, OwnedFd};

use rustix::{io, process};

use crate::error::Errno;

const MAX_SIGNAL: c_int = 64;

// Asks the kernel to create a pidfd for the new child and store its number through the `pidfd` pointer field.
const CLONE_PIDFD: u64 = 0x0000_1000;

// Creates the child inside the cgroup the `cgroup` field names, instead of placing it there once it runs.
const CLONE_INTO_CGROUP: u64 = 0x0000_0002_0000_0000;

/// Outcome of a `clone3` call, told apart by whether the caller is the parent.
#[derive(Debug)]
pub(crate) enum Cloned {
    /// The caller is the new child, which continues from the call.
    Child,
    /// The caller is the parent, which now owns the child and its pidfd.
    Parent {
        /// Raw process identifier of the new child.
        pid: c_int,
        /// Descriptor that names the new child, always present.
        pidfd: OwnedFd,
    },
}

pub(crate) fn pipe() -> Result<(OwnedFd, OwnedFd), Errno> {
    let mut fds = [0_i32; 2];
    // SAFETY: `fds` holds room for the two descriptors the kernel writes, and the call only touches that array.
    let result = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    check(result)?;
    // SAFETY: a successful `pipe2` returns two newly owned descriptors that no other value owns yet.
    let read = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    // SAFETY: the second descriptor is distinct from the first and likewise owned by no other value.
    let write = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    Ok((read, write))
}

pub(crate) fn at_least(fd: OwnedFd, floor: c_int) -> Result<OwnedFd, Errno> {
    if fd.as_raw_fd() >= floor {
        return Ok(fd);
    }
    // SAFETY: `fcntl` reads the scalar descriptor and the scalar minimum, so no pointer is involved.
    let result = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, floor) };
    check(result)?;
    // SAFETY: a successful `F_DUPFD_CLOEXEC` returns a descriptor no other value owns, and dropping `fd` closes the low one the child would otherwise redirect over.
    Ok(unsafe { OwnedFd::from_raw_fd(result) })
}

pub(crate) fn clone3(cgroup: Option<BorrowedFd<'_>>) -> Result<Cloned, Errno> {
    // The slot the kernel writes the pidfd number into: the struct carries its address, never the number itself.
    let mut pidfd_number: c_int = -1;
    // SAFETY: every field of `clone_args` is a plain integer, so an all-zero value is valid.
    let mut args: libc::clone_args = unsafe { core::mem::zeroed() };
    // `CLONE_PIDFD` rejects an exit signal the process could not be waited on with.
    args.exit_signal = u64::from(libc::SIGCHLD.unsigned_abs());
    args.flags = CLONE_PIDFD;
    args.pidfd =
        u64::try_from((&raw mut pidfd_number).addr()).map_err(|_error| Errno::new(libc::EINVAL))?;
    if let Some(directory) = cgroup {
        args.flags |= CLONE_INTO_CGROUP;
        // Unlike `pidfd`, this field is the descriptor itself.
        args.cgroup = u64::from(directory.as_raw_fd().unsigned_abs());
    }
    // SAFETY: `args` and `pidfd_number` are initialized stack values that outlive the call and the kernel reads exactly the size given; without `CLONE_VM` the child continues in a copy of this address space where only allocation free and lock free code runs.
    let result = unsafe {
        libc::syscall(
            libc::SYS_clone3,
            &raw const args,
            size_of::<libc::clone_args>(),
        )
    };
    if result == -1 {
        return Err(last());
    }
    if result == 0 {
        return Ok(Cloned::Child);
    }
    let pid = c_int::try_from(result).map_err(|_error| Errno::new(libc::EINVAL))?;
    // A successful call always stores the descriptor it created, so the initial value means the flag was not honoured.
    if pidfd_number < 0_i32 {
        return Err(Errno::new(libc::EINVAL));
    }
    // SAFETY: the kernel stored a freshly created descriptor number that no other value owns.
    let pidfd = unsafe { OwnedFd::from_raw_fd(pidfd_number) };
    Ok(Cloned::Parent { pid, pidfd })
}

/// # Safety
///
/// `program` must point to a NUL terminated string, and `argv` and `envp` to null terminated arrays of NUL terminated strings that stay valid for the call.
pub(crate) unsafe fn exec(
    program: *const c_char,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> Errno {
    // SAFETY: the caller guarantees every pointer is valid, NUL terminated and alive, which is exactly what execve reads.
    unsafe {
        libc::execve(program, argv, envp);
    }
    last()
}

pub(crate) fn redirect(source: c_int, target: c_int) -> Result<(), Errno> {
    // SAFETY: dup2 reads only the two scalar descriptor numbers, so no pointer is involved.
    let result = unsafe { libc::dup2(source, target) };
    check(result)
}

pub(crate) fn clear_cloexec(fd: c_int) -> Result<(), Errno> {
    // SAFETY: `fcntl` reads the scalar descriptor and returns the flags as a number, so no pointer is involved.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags == -1_i32 {
        return Err(last());
    }
    // SAFETY: `fcntl` takes the descriptor and the scalar flag mask, which keeps every flag the descriptor already carried.
    let result = unsafe { libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) };
    check(result)
}

pub(crate) fn reset_signals() -> Result<(), Errno> {
    block_all()?;
    // SAFETY: an all-zero `sigaction` is a valid value, so no initialized field is lost.
    let mut action: libc::sigaction = unsafe { core::mem::zeroed() };
    action.sa_sigaction = libc::SIG_DFL;
    // SAFETY: `action.sa_mask` is a plain byte array inside the zeroed struct, and `sigemptyset` writes only to that initialized field.
    unsafe {
        libc::sigemptyset(&raw mut action.sa_mask);
    }
    for signal in 1_i32..=MAX_SIGNAL {
        // SAFETY: `action` is fully initialized and outlives the call, and the kernel only reads it; `SIGKILL` and `SIGSTOP` reject the change, so their error is deliberately ignored.
        unsafe {
            libc::sigaction(signal, &raw const action, core::ptr::null_mut());
        }
    }
    Ok(())
}

pub(crate) fn restore_signals() -> Result<(), Errno> {
    // SAFETY: an all-zero `sigset_t` is a valid empty set.
    let mut set: libc::sigset_t = unsafe { core::mem::zeroed() };
    // SAFETY: `set` is a plain byte array inside the zeroed struct, and `sigemptyset` writes only to that initialized value.
    unsafe {
        libc::sigemptyset(&raw mut set);
    }
    // SAFETY: `set` is initialized and outlives the call; a null old set asks the kernel not to write anything back.
    let result =
        unsafe { libc::sigprocmask(libc::SIG_SETMASK, &raw const set, core::ptr::null_mut()) };
    check(result)
}

pub(crate) fn wait(pid: process::Pid) -> Result<process::WaitStatus, Errno> {
    loop {
        let outcome = process::waitpid(Some(pid), process::WaitOptions::empty());
        let error = match outcome {
            Ok(Some((_, status))) => return Ok(status),
            Ok(None) => return Err(Errno::new(libc::ECHILD)),
            Err(error) => error,
        };
        if error != io::Errno::INTR {
            return Err(Errno::from(error));
        }
    }
}

pub(crate) fn kill(pid: process::Pid, signal: process::Signal) -> Result<(), Errno> {
    process::kill_process(pid, signal).map_err(Errno::from)
}

pub(crate) fn reap(raw_pid: c_int) {
    loop {
        // SAFETY: the pid names the child forked by this process, and a null status pointer asks the kernel to discard the value.
        let result = unsafe { libc::waitpid(raw_pid, core::ptr::null_mut(), 0) };
        if result >= 0_i32 || last() != Errno::new(libc::EINTR) {
            return;
        }
    }
}

fn block_all() -> Result<(), Errno> {
    // SAFETY: an all-zero `sigset_t` is a valid empty set.
    let mut set: libc::sigset_t = unsafe { core::mem::zeroed() };
    // SAFETY: `set` is a plain byte array inside the zeroed struct, and `sigfillset` writes only to that initialized value.
    unsafe {
        libc::sigfillset(&raw mut set);
    }
    // SAFETY: `set` is initialized and outlives the call; a null old set asks the kernel not to write anything back.
    let result =
        unsafe { libc::sigprocmask(libc::SIG_SETMASK, &raw const set, core::ptr::null_mut()) };
    check(result)
}

fn check(result: c_int) -> Result<(), Errno> {
    if result == -1_i32 {
        Err(last())
    } else {
        Ok(())
    }
}

fn last() -> Errno {
    // SAFETY: `__errno_location` returns the current thread's errno slot, always a valid readable pointer.
    let location = unsafe { libc::__errno_location() };
    // SAFETY: `location` is the valid errno slot returned above.
    Errno::new(unsafe { *location })
}
