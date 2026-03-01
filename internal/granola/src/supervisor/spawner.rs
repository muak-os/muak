use std::mem::ManuallyDrop;
use std::os::fd::{OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use rustix::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd};

use super::service::{ServiceState, ServiceStatus};

/// Result of spawning a service, including its PID and piped output fds.
pub struct SpawnResult {
    pub pid: i32,
    pub stdout: OwnedFd,
    pub stderr: OwnedFd,
}

/// Spawns the binary for a service and updates its state.
pub fn spawn(state: &mut ServiceState) -> Result<SpawnResult> {
    let (bin, args) = state
        .def
        .command
        .split_first()
        .context("command must not be empty")?;

    if !Path::new(bin).exists() {
        bail!("Binary not found: {}", bin);
    }

    let raw_listener: Option<RawFd> = state.listener_fd.as_ref().map(|fd| fd.as_fd().as_raw_fd());

    let mut cmd = Command::new(bin);
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());

    if let Some(src_raw) = raw_listener {
        // SAFETY: pre_exec runs in the child after fork, before exec.
        // src_raw is a valid & open fd inherited from the parent.
        // We dup2 it onto fd 3 and clear CLOEXEC so it survives exec.
        unsafe {
            cmd.pre_exec(move || {
                let src = BorrowedFd::borrow_raw(src_raw);
                // SAFETY: ManuallyDrop prevents the destructor from closing fd 3,
                // which we do not own — dup2 will close and reuse it.
                let mut dst = ManuallyDrop::new(OwnedFd::from_raw_fd(3));
                rustix::io::dup2(src, &mut dst)
                    .map_err(|e| std::io::Error::from_raw_os_error(e.raw_os_error()))?;
                rustix::io::fcntl_setfd(&BorrowedFd::borrow_raw(3), rustix::io::FdFlags::empty())
                    .map_err(|e| std::io::Error::from_raw_os_error(e.raw_os_error()))?;
                Ok(())
            });
        }
    }

    let mut child = cmd.spawn().context("Failed to spawn service")?;

    let pid = child.id() as i32;
    state.pid = Some(pid);
    state.status = ServiceStatus::Starting;

    kmsg::info!("Spawned {} with PID {}", state.def.name, pid);

    Ok(SpawnResult {
        pid,
        stdout: child.stdout.take().context("Failed to take stdout")?.into(),
        stderr: child.stderr.take().context("Failed to take stderr")?.into(),
    })
}
