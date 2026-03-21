//! Muak init - phase 1 initialization process.
//!
//! This is the first process started by the kernel. It mounts pseudo filesystems,
//! loads kernel modules, mounts the root filesystem, loads the SELinux policy,
//! switches to the new root and executes the PID 1 process.

mod modules;
mod mount;
mod selinux;
mod switchroot;

use std::fs;
use std::process;

use anyhow::{Context, Result};

/// SELinux binary policy path inside the mounted rootfs.
const SELINUX_POLICY: &str = "/newroot/etc/selinux/policy.34";

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

    kmsg::info!("Pseudo filesystems mounted");

    match modules::load() {
        Ok(count) => kmsg::info!("Loaded {} kernel modules", count),
        Err(e) => kmsg::warn!("Module loading failed: {}", e),
    }

    kmsg::info!("Mounting rootfs");
    mount::mount_rootfs()?;
    kmsg::info!("Rootfs mounted successfully");

    let policy_bytes = fs::read(SELINUX_POLICY)
        .with_context(|| format!("Failed to read SELinux policy from {}", SELINUX_POLICY))?;
    selinux::load_policy(&policy_bytes).context("Failed to load SELinux policy")?;
    let mode = if selinux::is_enforcing().unwrap_or(false) {
        "enforcing"
    } else {
        "permissive"
    };
    kmsg::info!("SELinux policy loaded ({})", mode);

    match mount::mount_persistent() {
        Ok(true) => kmsg::info!("Persistent partitions mounted"),
        Ok(false) => kmsg::info!("No persistent partitions found (maintenance mode)"),
        Err(e) => return Err(e),
    }

    kmsg::info!("Switching to new root");
    switchroot::switch("/newroot")?;

    unreachable!("switch_root should never return");
}
