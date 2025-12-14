mod logging;
mod modules;
mod mount;
mod switchroot;

use std::process;

fn main() {
    if let Err(e) = run() {
        logging::error(&format!("FATAL ERROR: {}", e));
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    mount::mount_pseudo()?;

    logging::init()?;
    logging::log("Pseudo filesystems mounted");

    match modules::load() {
        Ok(count) => logging::log(&format!("Loaded {} kernel modules", count)),
        Err(e) => logging::log(&format!("Warning: module loading failed: {}", e)),
    }

    logging::log("Mounting rootfs");
    mount::mount_rootfs()?;
    logging::log("Rootfs mounted successfully");

    logging::log("Switching to new root");
    switchroot::switch("/newroot")?;

    unreachable!("switch_root should never return");
}
