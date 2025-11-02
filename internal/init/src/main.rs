mod logging;
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

    logging::log("Mounting rootfs");
    mount::mount_rootfs()?;
    logging::log("Rootfs mounted successfully");

    logging::log("Switching to new root");
    switchroot::switch("/newroot")?;

    unreachable!("switch_root should never return");
}
