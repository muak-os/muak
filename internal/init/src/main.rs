mod modules;
mod mount;
mod switchroot;

use anyhow::Result;
use std::process;

fn main() {
    if let Err(e) = run() {
        kmsg::error!("FATAL ERROR: {:#}", e);
        process::exit(1);
    }
}

fn run() -> Result<()> {
    kmsg::init("init")?;

    kmsg::print(include_str!("../banner"));

    mount::mount_pseudo()?;
    kmsg::info!("Pseudo filesystems mounted");

    match modules::load() {
        Ok(count) => kmsg::info!("Loaded {} kernel modules", count),
        Err(e) => kmsg::warn!("Module loading failed: {}", e),
    }

    kmsg::info!("Mounting rootfs");
    mount::mount_rootfs()?;
    kmsg::info!("Rootfs mounted successfully");

    kmsg::info!("Switching to new root");
    switchroot::switch("/newroot")?;

    unreachable!("switch_root should never return");
}
