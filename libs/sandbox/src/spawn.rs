//! Spawning a command under a resolved policy.
//!
//! The parent side of this module may allocate and format. The child side may
//! not, and it never does: the pointers and the report pipe are built before
//! fork, and the private child module only borrows them afterwards.
//!
//! # Fork safety
//!
//! Forking from a multithreaded parent is safe here only because the child path
//! is allocation free, panic free and lock free. A forked child of a threaded
//! process may inherit a lock whose owner thread does not exist in the child,
//! and taking it would deadlock. The child path is therefore reduced to raw
//! syscalls, and the integration test that runs eight threads with twenty
//! spawns each, plus the allocation counting unit test in the private child
//! module, enforce that discipline.

use core::ffi::{CStr, c_char};
use core::mem::size_of;
use std::fs::File;
use std::io::Read as _;
use std::os::fd::{AsFd, AsRawFd as _, BorrowedFd, OwnedFd, RawFd};
use std::os::unix::process::ExitStatusExt as _;
use std::process::ExitStatus;

use rustix::process;

use crate::child;
use crate::error::{Errno, Mechanism, Result, SandboxError};
use crate::resolve::Resolved;
use crate::stage::Stage;
use crate::sys::linux::spawn;

/// Process identifier of a spawned child.
pub type Pid = process::Pid;

/// Signal delivered by [`Child::signal`].
pub type Signal = process::Signal;

/// A command to execute, built from NUL terminated byte strings.
///
/// The child must not allocate, so the program, the arguments and the
/// environment are borrowed as C strings instead of being assembled from
/// `OsString`s between fork and exec.
#[derive(Clone, Copy, Debug)]
pub struct Cmd<'a> {
    program: &'a CStr,
    argv: &'a [&'a CStr],
    envp: &'a [&'a CStr],
    stdin: Option<BorrowedFd<'a>>,
    stdout: Option<BorrowedFd<'a>>,
    stderr: Option<BorrowedFd<'a>>,
    pass: &'a [(BorrowedFd<'a>, RawFd)],
}

/// A child process spawned under a policy.
#[derive(Debug)]
pub struct Child {
    pid: Pid,
    pidfd: Option<OwnedFd>,
}

impl<'a> Cmd<'a> {
    /// Returns a command running `program` with `argv` and `envp`.
    #[must_use = "the command is the value the caller asked for"]
    pub const fn new(program: &'a CStr, argv: &'a [&'a CStr], envp: &'a [&'a CStr]) -> Self {
        Self {
            program,
            argv,
            envp,
            stdin: None,
            stdout: None,
            stderr: None,
            pass: &[],
        }
    }

    /// Returns this command with `fd` installed as standard input.
    #[must_use = "the command is the value the caller asked for"]
    pub const fn stdin(mut self, fd: BorrowedFd<'a>) -> Self {
        self.stdin = Some(fd);
        self
    }

    /// Returns this command with `fd` installed as standard output.
    #[must_use = "the command is the value the caller asked for"]
    pub const fn stdout(mut self, fd: BorrowedFd<'a>) -> Self {
        self.stdout = Some(fd);
        self
    }

    /// Returns this command with `fd` installed as standard error.
    #[must_use = "the command is the value the caller asked for"]
    pub const fn stderr(mut self, fd: BorrowedFd<'a>) -> Self {
        self.stderr = Some(fd);
        self
    }

    /// Returns this command with every `(fd, target)` pair duplicated onto
    /// `target` in the child before exec.
    ///
    /// This is how a descriptor the child must inherit survives the exec, for
    /// example a socket the parent already bound: the child cannot inherit the
    /// parent descriptor number, so the pair names the source it should be
    /// reachable from and the fixed number the child expects.
    #[must_use = "the command is the value the caller asked for"]
    pub const fn pass(mut self, pass: &'a [(BorrowedFd<'a>, RawFd)]) -> Self {
        self.pass = pass;
        self
    }
}

impl Child {
    /// Returns the child process identifier.
    #[must_use = "the identifier is the value the caller asked for"]
    pub const fn pid(&self) -> Pid {
        self.pid
    }

    /// Returns the pidfd when the kernel and the caller asked for one.
    #[must_use = "the descriptor is the value the caller asked for"]
    pub fn pidfd(&self) -> Option<BorrowedFd<'_>> {
        self.pidfd.as_ref().map(AsFd::as_fd)
    }

    /// Delivers `signal` to the child.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxError::Io`] when the kernel refuses the signal.
    pub fn signal(&self, signal: Signal) -> Result<()> {
        spawn::kill(self.pid, signal).map_err(SandboxError::Io)
    }

    /// Delivers `SIGKILL` to the child.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxError::Io`] when the kernel refuses the signal.
    pub fn kill(&self) -> Result<()> {
        self.signal(Signal::KILL)
    }

    /// Waits for the child and returns its exit status.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxError::Io`] when the kernel cannot report the status.
    pub fn wait(&self) -> Result<ExitStatus> {
        let status = spawn::wait(self.pid).map_err(SandboxError::Io)?;
        Ok(ExitStatus::from_raw(status.as_raw()))
    }
}

/// Spawns `cmd` in a child configured by the resolved policy.
///
/// The parent creates a close-on-exec report pipe, moves the write end above
/// the standard descriptors and above every passed descriptor, then forks. A
/// child that cannot apply the policy writes a fixed stage and errno pair and
/// exits with 127; a child that execs closes the pipe on the way to the new
/// image. No report therefore means the exec succeeded, and a report becomes a
/// [`SandboxError::Denied`] owned by the mechanism of that stage.
///
/// # Errors
///
/// Returns [`SandboxError::Io`] when the pipe or fork fails, and
/// [`SandboxError::Denied`] when the child reports a failed stage.
#[must_use = "the child must be waited on, otherwise it becomes a zombie"]
pub fn run(resolved: &Resolved<'_>, cmd: &Cmd<'_>) -> Result<Child> {
    let (read, write) = spawn::pipe().map_err(SandboxError::Io)?;
    // The write end must sit above the standard descriptors and above every
    // descriptor the child installs, otherwise a caller that closed its stdio
    // gets the report on fd 0, 1 or 2, or a passed descriptor lands on the
    // report, and the child redirects over it before it can report a failure.
    let write = spawn::at_least(write, report_floor(cmd.pass)).map_err(SandboxError::Io)?;
    let argv = pointers(cmd.argv);
    let envp = pointers(cmd.envp);
    let program = cmd.program.as_ptr();
    let files = [raw(cmd.stdin), raw(cmd.stdout), raw(cmd.stderr)];
    let report_fd = write.as_raw_fd();

    let raw_pid = spawn::fork().map_err(SandboxError::Io)?;
    if raw_pid == 0_i32 {
        // SAFETY: this branch is the child of the fork above. The command
        // pointers, the passed descriptors and the report descriptor were
        // built by the parent and are valid in this copy of the address space,
        // and `exec` never returns.
        unsafe {
            child::exec(
                resolved.policy,
                program,
                argv.as_ptr(),
                envp.as_ptr(),
                files,
                cmd.pass,
                report_fd,
            )
        }
    }

    drop(write);
    let report = match reported(read) {
        Ok(report) => report,
        Err(error) => {
            spawn::reap(raw_pid);
            return Err(error);
        }
    };
    if let Some((stage, errno)) = report {
        spawn::reap(raw_pid);
        return Err(SandboxError::Denied {
            mechanism: mechanism(stage),
            source: errno,
        });
    }

    let pid = process::Pid::from_raw(raw_pid).ok_or(SandboxError::Io(Errno::new(libc::EINVAL)))?;
    let pidfd = if resolved.pidfd {
        spawn::pidfd_open(pid).ok()
    } else {
        None
    };
    Ok(Child { pid, pidfd })
}

fn mechanism(stage: Stage) -> Mechanism {
    match stage {
        Stage::Namespaces => Mechanism::Namespaces,
        Stage::Mounts => Mechanism::Mounts,
        Stage::Rootfs => Mechanism::Rootfs,
        Stage::Cgroup => Mechanism::Cgroup,
        Stage::Credentials | Stage::NoNewPrivs | Stage::Exec | Stage::Signals => Mechanism::Spawn,
        Stage::Caps => Mechanism::Caps,
        Stage::Seccomp => Mechanism::Seccomp,
        Stage::Limits => Mechanism::Limits,
    }
}

fn pointers(items: &[&CStr]) -> Vec<*const c_char> {
    let mut pointers: Vec<*const c_char> = items.iter().map(|item| item.as_ptr()).collect();
    pointers.push(core::ptr::null());
    pointers
}

fn raw(fd: Option<BorrowedFd<'_>>) -> RawFd {
    fd.map_or(-1, |fd| fd.as_raw_fd())
}

// The lowest number the report pipe may occupy: one above the standard
// descriptors and one above the highest descriptor the child installs.
fn report_floor(pass: &[(BorrowedFd<'_>, RawFd)]) -> RawFd {
    let highest = pass
        .iter()
        .map(|&(_, target)| target)
        .max()
        .unwrap_or(libc::STDERR_FILENO);
    highest.max(libc::STDERR_FILENO).saturating_add(1)
}

fn reported(read: OwnedFd) -> Result<Option<(Stage, Errno)>> {
    let mut file = File::from(read);
    let mut bytes = [0_u8; size_of::<u64>()];
    let mut filled = 0;
    while let Some(rest) = bytes.get_mut(filled..) {
        match file.read(rest) {
            Ok(0) => break,
            Ok(count) => filled = filled.saturating_add(count),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => {
                let raw = error.raw_os_error().unwrap_or(libc::EIO);
                return Err(SandboxError::Io(Errno::new(raw)));
            }
        }
    }
    if filled == 0 {
        return Ok(None);
    }
    if filled != bytes.len() {
        return Err(SandboxError::Io(Errno::new(libc::EIO)));
    }

    let packed = u64::from_le_bytes(bytes);
    let code = u16::try_from(packed & 0xffff).unwrap_or(0);
    let stage = Stage::from_code(code).ok_or(SandboxError::Io(Errno::new(libc::EINVAL)))?;
    let raw = u32::try_from((packed >> 16_u32) & 0xffff_ffff).unwrap_or(0);
    Ok(Some((
        stage,
        Errno::new(i32::from_le_bytes(raw.to_le_bytes())),
    )))
}
