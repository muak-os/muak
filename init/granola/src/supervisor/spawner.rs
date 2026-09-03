use std::mem::ManuallyDrop;
use std::os::fd::{OwnedFd, RawFd};
use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};
use rustix::fd::{AsFd as _, AsRawFd as _, BorrowedFd, FromRawFd as _};

use super::service::{ServiceState, ServiceStatus};

/// Output file descriptors produced by spawning a service.
pub struct SpawnResult {
    pub pid: i32,
    pub stdout: OwnedFd,
    pub stderr: OwnedFd,
}

/// Abstraction over spawning a supervised child process.
pub trait Spawn {
    fn spawn(&mut self, state: &mut ServiceState) -> Result<SpawnResult>;
}

/// Spawns real OS processes.
pub struct Spawner;

impl Spawn for Spawner {
    fn spawn(&mut self, state: &mut ServiceState) -> Result<SpawnResult> {
        let tokens = state.service.argv();
        let (program, args) = tokens.split_first().context("command must not be empty")?;

        if !Path::new(program).exists() {
            bail!("Binary not found: {program}");
        }

        let raw_listener: Option<RawFd> =
            state.listener_fd.as_ref().map(|fd| fd.as_fd().as_raw_fd());

        let mut cmd = Command::new(program);
        cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());

        if let Some(src_raw) = raw_listener {
            // SAFETY: src_raw is a valid, open descriptor inherited by the child.
            let prepare = move || unsafe { prepare_service_socket(src_raw) };
            // SAFETY: pre_exec runs in the child after fork, before exec;
            // src_raw is a valid, open descriptor inherited by the child.
            unsafe {
                cmd.pre_exec(prepare);
            }
        }

        let mut child = cmd.spawn().context("Failed to spawn service")?;

        let pid = child.id().cast_signed();
        state.pid = Some(pid);
        state.status = ServiceStatus::Starting;

        kmsg::info!("Spawned {} with PID {}", state.service.name, pid);

        Ok(SpawnResult {
            pid,
            stdout: child.stdout.take().context("Failed to take stdout")?.into(),
            stderr: child.stderr.take().context("Failed to take stderr")?.into(),
        })
    }
}

/// Duplicates the pre-bound listener onto fd 3 in the child and clears CLOEXEC
/// so it survives the upcoming exec.
///
/// # Errors
///
/// Returns an OS error when the descriptor cannot be duplicated or re-flagged.
///
/// # Safety
///
/// `src_raw` must be a valid, open file descriptor that the child is allowed
/// to use, and the caller must run before `exec` completes in the child.
unsafe fn prepare_service_socket(src_raw: RawFd) -> std::io::Result<()> {
    // SAFETY: src_raw is a valid, open descriptor in the child (see caller).
    let src = unsafe { BorrowedFd::borrow_raw(src_raw) };
    // SAFETY: fd 3 exists in the child during pre_exec. ManuallyDrop prevents
    // closing a descriptor we do not own — dup2 closes and reuses it itself.
    let mut dst = ManuallyDrop::new(unsafe { OwnedFd::from_raw_fd(3) });
    rustix::io::dup2(src, &mut dst).map_err(rustix_error)?;
    // SAFETY: fd 3 is open after the successful dup2 above.
    let listener = unsafe { BorrowedFd::borrow_raw(3) };
    rustix::io::fcntl_setfd(listener, rustix::io::FdFlags::empty()).map_err(rustix_error)?;

    Ok(())
}

/// Converts a rustix error into a standard library I/O error.
fn rustix_error(error: rustix::io::Errno) -> std::io::Error {
    std::io::Error::from_raw_os_error(error.raw_os_error())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::supervisor::service::{Service, ServiceState};

    fn make_state(command: &str) -> ServiceState {
        ServiceState::new(Service {
            name: "test-svc".to_owned(),
            command: command.to_owned(),
            depends_on: vec![],
        })
    }

    #[test]
    fn empty_command_returns_error() {
        // ARRANGE
        let mut state = make_state("");

        // ACT
        let result = Spawner.spawn(&mut state);

        // ASSERT
        let err = result.err().expect("expected an error");
        assert!(err.to_string().contains("command must not be empty"));
    }

    #[test]
    fn nonexistent_binary_returns_error() {
        // ARRANGE
        let mut state = make_state("/nonexistent/binary/xyz_abc");

        // ACT
        let result = Spawner.spawn(&mut state);

        // ASSERT
        let err = result.err().expect("expected an error");
        assert!(err.to_string().contains("Binary not found"));
    }
}
