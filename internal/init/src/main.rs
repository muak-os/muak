//! Muak init - phase 1 initialization process.
//!
//! This is the first process started by the kernel. It mounts pseudo filesystems,
//! loads kernel modules, mounts the root filesystem, switches to the new root
//! and execute the PID 1 process.

mod modules;
mod mount;
mod switchroot;

use anyhow::Result;
use std::process;

/// Entry point that handles fatal errors.
fn main() {
    if let Err(e) = run() {
        kmsg::error!("FATAL ERROR: {:#}", e);
        process::exit(1);
    }
}

/// Run the initialization sequence.
fn run() -> Result<()> {
    mount::mount_pseudo()?;

    kmsg::init("init")?;

    kmsg::print(include_str!("../banner"));

    kmsg::info!("Pseudo filesystems mounted");

    match modules::load() {
        Ok(count) => kmsg::info!("Loaded {} kernel modules", count),
        Err(e) => kmsg::warn!("Module loading failed: {}", e),
    }

    kmsg::info!("Mounting rootfs");
    mount::mount_rootfs()?;
    kmsg::info!("Rootfs mounted successfully");

    match mount::mount_persistent() {
        Ok(true) => kmsg::info!("Persistent partitions mounted"),
        Ok(false) => kmsg::info!("No persistent partitions found (maintenance mode)"),
        Err(e) => kmsg::warn!("Failed to mount persistent partitions: {:#}", e),
    }

    kmsg::info!("Switching to new root");
    switchroot::switch("/newroot")?;

    unreachable!("switch_root should never return");
}
