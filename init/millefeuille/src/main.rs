//! Phase 1 initialization process.

extern crate alloc;

mod modules;
mod mount;
#[cfg(feature = "selinux")]
mod selinux;
mod switchroot;

use std::path::Path;

use anyhow::Result;

/// Mounted rootfs path used before `switch_root`.
const NEWROOT: &str = "/newroot";

/// Entry point that handles fatal errors.
fn main() {
    if let Err(e) = run() {
        kmsg::error!("FATAL ERROR: {:#}", e);
        std::process::exit(1);
    }
}

/// Run the initialization sequence.
fn run() -> Result<()> {
    mount::pseudo()?;

    kmsg::init("millefeuille")?;

    kmsg::info!("Pseudo filesystems mounted");

    kmsg::info!("Mounting rootfs");
    mount::rootfs()?;
    kmsg::info!("Rootfs mounted successfully");

    #[cfg(feature = "selinux")]
    selinux::init()?;

    match modules::load(Path::new(NEWROOT)) {
        Ok(count) => kmsg::info!("Loaded {} kernel modules", count),
        Err(e) => kmsg::warn!("Module loading failed: {}", e),
    }

    let persistent = mount::persistent();
    if persistent {
        kmsg::info!("Persistent partitions mounted");
    } else {
        kmsg::info!("No valid persistent state found (maintenance mode)");
    }

    mount::bridge();
    kmsg::info!("Boot metadata bridged to {}", mount::BOOT_DIR);

    kmsg::info!("Switching to new root");
    switchroot::new_root(NEWROOT)?;
    switchroot::exec_init(&[("MAINTENANCE", if persistent { "false" } else { "true" })])?;

    Ok(())
}
