use anyhow::{Context, Result, bail};

use super::service::{ServiceState, ServiceStatus};

/// Spawns the binary for a service and updates its state.
pub fn spawn(state: &mut ServiceState) -> Result<i32> {
    if !std::path::Path::new(&state.def.binary).exists() {
        bail!("Binary not found: {}", state.def.binary);
    }

    let child = std::process::Command::new(state.def.binary)
        .args(&state.def.args)
        .spawn()
        .context("Failed to spawn service")?;

    let pid = child.id() as i32;
    state.pid = Some(pid);
    state.status = ServiceStatus::Starting;

    kmsg::info!("Spawned {} with PID {}", state.def.name, pid);

    Ok(pid)
}
