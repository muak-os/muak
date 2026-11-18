//! Spawning a command under a resolved policy.
//!
//! Creating a child from a multithreaded parent is safe here only because the child path is allocation free, panic free and lock free: a child inherits a lock whose owner thread no longer exists.
//!
//! The cgroup descriptor is handed to `clone3` with `CLONE_INTO_CGROUP`, because writing the pid to `cgroup.procs` afterwards leaves a window where the parent's limits apply.

use core::ffi::{CStr, c_char};
use core::mem::size_of;
use std::fs::File;
use std::io::Read as _;
use std::os::fd::{AsFd as _, AsRawFd as _, BorrowedFd, OwnedFd, RawFd};
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
/// The child must not allocate, so the program, the arguments and the environment are borrowed as C strings.
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
    pidfd: OwnedFd,
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

    /// Returns this command with every `(fd, target)` pair duplicated onto `target` in the child before exec.
    ///
    /// The child cannot inherit the parent descriptor number, so the pair names the source and the fixed number the child expects.
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

    /// Returns a descriptor that names this child.
    ///
    /// It is created by `clone3` together with the child, so it can never name a recycled pid.
    #[must_use = "the descriptor is the value the caller asked for"]
    pub fn pidfd(&self) -> BorrowedFd<'_> {
        self.pidfd.as_fd()
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
/// No report means the exec succeeded: a failing child writes a fixed stage and errno pair, which becomes a [`SandboxError::Denied`] owned by the mechanism of that stage.
///
/// A `cgroup` descriptor, from `Tree::fd` in the v2 module, makes the child be born inside that cgroup through `CLONE_INTO_CGROUP`.
///
/// # Errors
///
/// Returns [`SandboxError::Io`] when the pipe or the clone fails, and [`SandboxError::Denied`] when the child reports a failed stage.
#[must_use = "the child must be waited on, otherwise it becomes a zombie"]
pub fn run(
    resolved: &Resolved<'_>,
    cmd: &Cmd<'_>,
    cgroup: Option<BorrowedFd<'_>>,
) -> Result<Child> {
    let (read, write) = spawn::pipe().map_err(SandboxError::Io)?;
    // The write end must sit above the standard descriptors and every descriptor the child installs, otherwise a redirect lands on the report and the child can no longer report a failure.
    let write = spawn::at_least(write, report_floor(cmd.pass)).map_err(SandboxError::Io)?;
    let argv = pointers(cmd.argv);
    let envp = pointers(cmd.envp);
    let program = cmd.program.as_ptr();
    let files = [raw(cmd.stdin), raw(cmd.stdout), raw(cmd.stderr)];
    let report_fd = write.as_raw_fd();

    let cloned = spawn::clone3(cgroup).map_err(SandboxError::Io)?;
    let (raw_pid, pidfd) = match cloned {
        // SAFETY: this is the child created by the clone above, where the command pointers, passed descriptors and report descriptor are valid and `exec` never returns.
        spawn::Cloned::Child => unsafe {
            child::exec(
                resolved.policy,
                program,
                argv.as_ptr(),
                envp.as_ptr(),
                files,
                cmd.pass,
                report_fd,
            )
        },
        spawn::Cloned::Parent { pid, pidfd } => (pid, pidfd),
    };

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

// The lowest number the report pipe may occupy: one above the standard descriptors and above the highest descriptor the child installs.
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
