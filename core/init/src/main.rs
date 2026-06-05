//! Muak init - phase 1 initialization process.
//!
//! This is the first process started by the kernel. It mounts pseudo filesystems,
//! loads kernel modules, mounts the root filesystem, loads the `SELinux` policy,
//! switches to the new root and executes the PID 1 process.

extern crate alloc;

mod modules;
mod mount;
mod selinux;
mod switchroot;
mod sysctl;

use std::path::Path;
use std::process;

use anyhow::Result;

/// Mounted rootfs path used before `switch_root`.
const NEWROOT: &str = "/newroot";

/// Entry point that handles fatal errors.
fn main() {
    if let Err(e) = run() {
        kmsg::error!("FATAL ERROR: {:#}", e);
        process::exit(1);
    }
}

/// Run the initialization sequence.
fn run() -> Result<()> {
    mount::pseudo()?;

    kmsg::init("init")?;

    kmsg::info!("Pseudo filesystems mounted");

    let sysctls = sysctl::apply()?;
    kmsg::info!(
        "Sysctl hardening applied (updated {}, unchanged {}, skipped {})",
        sysctls.applied,
        sysctls.unchanged,
        sysctls.skipped
    );

    kmsg::info!("Mounting rootfs");
    mount::rootfs()?;
    kmsg::info!("Rootfs mounted successfully");

    match modules::load(Path::new(NEWROOT)) {
        Ok(count) => kmsg::info!("Loaded {} kernel modules", count),
        Err(e) => kmsg::warn!("Module loading failed: {}", e),
    }

    selinux::load()?;
    let mode = if selinux::is_enforcing().unwrap_or(false) {
        "enforcing"
    } else {
        "permissive"
    };
    kmsg::info!("SELinux policy loaded ({})", mode);

    if mount::persistent() {
        kmsg::info!("Persistent partitions mounted");
    } else {
        kmsg::info!("No valid persistent state found (maintenance mode)");
    }

    kmsg::info!("Switching to new root");
    switchroot::new_root(NEWROOT)?;

    Ok(())
}
