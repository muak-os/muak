use std::os::fd::OwnedFd;
use std::process::Stdio;

use anyhow::{Context, Result, bail};

use super::service::{ServiceState, ServiceStatus};

/// Result of spawning a service, including its PID and piped output fds.
pub struct SpawnResult {
    pub pid: i32,
    pub stdout: OwnedFd,
    pub stderr: OwnedFd,
}

/// Spawns the binary for a service and updates its state.
pub fn spawn(state: &mut ServiceState) -> Result<SpawnResult> {
    if !std::path::Path::new(&state.def.binary).exists() {
        bail!("Binary not found: {}", state.def.binary);
    }

    let mut child = std::process::Command::new(state.def.binary)
        .args(&state.def.args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn service")?;

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
