//! Muak module daemon (modd) - Hot-pluggable kernel module loader.
//!
//! Listens for kernel uevents and automatically loads appropriate kernel modules
//! based on modalias matching.

mod modules;
mod uevent;

use std::path::Path;

use granola::runtime::notify::Health;
use modules::Loader;
use rustix::system::uname;
use uevent::{UeventAction, UeventListener};

#[granola::service("modd")]
fn main(notifier: NotifyClient) -> Result<()> {
    notifier.status("Initializing", Health::Healthy)?;

    let uname = uname();
    let krel = uname.release().to_string_lossy();
    let mod_dir = Path::new("/lib/modules").join(krel.as_ref());

    println!("Module directory: {}", mod_dir.display());

    let mut loader = Loader::new(&mod_dir)?;
    let mut listener = UeventListener::new()?;

    let loaded = loader.sweep();
    println!("Initial sweep loaded {loaded} module(s) for devices already present");

    println!("Listening for kernel uevents");

    notifier.ready()?;

    loop {
        let event = match listener.recv() {
            Ok(event) => event,
            Err(e) => {
                eprintln!("Failed to receive uevent: {e}");
                break;
            }
        };

        if event.action != UeventAction::Add {
            continue;
        }

        let Some(modalias) = event.modalias else {
            continue;
        };

        let subsystem = event.subsystem.unwrap_or("unknown");
        loader.load(modalias, subsystem);
    }

    Ok(())
}
