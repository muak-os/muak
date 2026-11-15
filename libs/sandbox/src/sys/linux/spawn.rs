//! Process creation, execution, signalling and reaping syscalls.
//!
//! The child path reaches this module between fork and exec, so no function
//! here allocates, locks or panics. Failures come back as raw kernel numbers
//! and the caller decides how to report them.

use core::ffi::{c_char, c_int};
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};

use rustix::{io, process};

use crate::error::Errno;

const MAX_SIGNAL: c_int = 64;

pub(crate) fn pipe() -> Result<(OwnedFd, OwnedFd), Errno> {
    let mut fds = [0_i32; 2];
    // SAFETY: `fds` holds room for the two descriptors the kernel writes, and
    // the call only touches that array.
    let result = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    check(result)?;
    // SAFETY: a successful `pipe2` returns two newly owned descriptors that no
    // other value owns yet, so each raw descriptor can be adopted exactly once.
    let read = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    // SAFETY: the second descriptor is distinct from the first and is likewise
    // owned by no other value.
    let write = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    Ok((read, write))
}

pub(crate) fn above_stdio(fd: OwnedFd) -> Result<OwnedFd, Errno> {
    if fd.as_raw_fd() > libc::STDERR_FILENO {
        return Ok(fd);
    }
    // SAFETY: `fcntl` reads the scalar descriptor and the scalar minimum, so
    // no pointer is involved and nothing can be invalidated.
    let result = unsafe {
        libc::fcntl(
            fd.as_raw_fd(),
            libc::F_DUPFD_CLOEXEC,
            libc::STDERR_FILENO + 1,
        )
    };
    check(result)?;
    // SAFETY: a successful `F_DUPFD_CLOEXEC` returns a descriptor that no other
    // value owns, so it can be adopted exactly once. Dropping `fd` then closes
    // the low descriptor the child would otherwise redirect over.
    Ok(unsafe { OwnedFd::from_raw_fd(result) })
}

pub(crate) fn fork() -> Result<c_int, Errno> {
    // SAFETY: fork takes no pointer arguments. The child continues in a copy of
    // this process, and the child path only calls allocation free and lock free
    // code before exec, so no inherited lock is ever taken there.
    let result = unsafe { libc::fork() };
    if result == -1_i32 {
        Err(last())
    } else {
        Ok(result)
    }
}

/// # Safety
///
/// `program` must point to a NUL terminated string, and `argv` and `envp` must
/// point to null terminated arrays of NUL terminated strings that stay valid
/// for the duration of the call.
pub(crate) unsafe fn exec(
    program: *const c_char,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> Errno {
    // SAFETY: the caller guarantees every pointer is valid, NUL terminated and
    // alive, which is exactly what execve reads; the call never returns on
    // success.
    unsafe {
        libc::execve(program, argv, envp);
    }
    last()
}

pub(crate) fn redirect(source: c_int, target: c_int) -> Result<(), Errno> {
    // SAFETY: dup2 reads only the two scalar descriptor numbers and installs
    // `source` as `target`; no pointer is involved.
    let result = unsafe { libc::dup2(source, target) };
    check(result)
}

pub(crate) fn reset_signals() -> Result<(), Errno> {
    block_all()?;
    // SAFETY: an all-zero `sigaction` is a valid value, so no initialized field
    // is lost.
    let mut action: libc::sigaction = unsafe { core::mem::zeroed() };
    action.sa_sigaction = libc::SIG_DFL;
    // SAFETY: `action.sa_mask` is a plain byte array inside the zeroed struct,
    // and `sigemptyset` writes only to that initialized field.
    unsafe {
        libc::sigemptyset(&raw mut action.sa_mask);
    }
    for signal in 1_i32..=MAX_SIGNAL {
        // SAFETY: `action` is fully initialized and outlives the call, and the
        // kernel only reads it. `SIGKILL` and `SIGSTOP` reject the change, so
        // their error is deliberately ignored.
        unsafe {
            libc::sigaction(signal, &raw const action, core::ptr::null_mut());
        }
    }
    Ok(())
}

pub(crate) fn restore_signals() -> Result<(), Errno> {
    // SAFETY: an all-zero `sigset_t` is a valid empty set.
    let mut set: libc::sigset_t = unsafe { core::mem::zeroed() };
    // SAFETY: `set` is a plain byte array inside the zeroed struct, and
    // `sigemptyset` writes only to that initialized value.
    unsafe {
        libc::sigemptyset(&raw mut set);
    }
    // SAFETY: `set` is initialized and outlives the call; a null old set asks
    // the kernel not to write anything back.
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

pub(crate) fn pidfd_open(pid: process::Pid) -> Result<OwnedFd, Errno> {
    process::pidfd_open(pid, process::PidfdFlags::empty()).map_err(Errno::from)
}

pub(crate) fn kill(pid: process::Pid, signal: process::Signal) -> Result<(), Errno> {
    process::kill_process(pid, signal).map_err(Errno::from)
}

pub(crate) fn reap(raw_pid: c_int) {
    loop {
        // SAFETY: the pid names the child forked by this process, and a null
        // status pointer asks the kernel to discard the value it would
        // otherwise write.
        let result = unsafe { libc::waitpid(raw_pid, core::ptr::null_mut(), 0) };
        if result >= 0_i32 || last() != Errno::new(libc::EINTR) {
            return;
        }
    }
}

fn block_all() -> Result<(), Errno> {
    // SAFETY: an all-zero `sigset_t` is a valid empty set.
    let mut set: libc::sigset_t = unsafe { core::mem::zeroed() };
    // SAFETY: `set` is a plain byte array inside the zeroed struct, and
    // `sigfillset` writes only to that initialized value.
    unsafe {
        libc::sigfillset(&raw mut set);
    }
    // SAFETY: `set` is initialized and outlives the call; a null old set asks
    // the kernel not to write anything back.
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
    // SAFETY: `__errno_location` returns the current thread's errno slot, which
    // is always a valid readable pointer for the duration of the call.
    let location = unsafe { libc::__errno_location() };
    // SAFETY: `location` is the valid errno slot returned above and is read
    // before the call returns.
    Errno::new(unsafe { *location })
}
