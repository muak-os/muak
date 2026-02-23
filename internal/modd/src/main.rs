//! Muak module daemon (modd) - Hot-pluggable kernel module loader.
//!
//! Listens for kernel uevents and automatically loads appropriate kernel modules
//! based on modalias matching.

mod uevent;

use std::path::Path;
use std::process;

use anyhow::{Context, Result};
use kmod::{AliasDb, DepDb, ModuleLoader, load_module};
use notify::{Health, NotifyClient};
use uevent::{UeventAction, UeventListener};

const SOCKET_PATH: &str = "/run/modd.sock";

/// Entry point that handles fatal errors.
fn main() {
    if let Err(e) = run() {
        kmsg::error!("Fatal error: {:#}", e);
        process::exit(1);
    }
}

/// Run the module daemon main loop.
fn run() -> Result<()> {
    kmsg::init("modd")?;
    kmsg::info!("Starting module daemon");

    let notifier = NotifyClient::new("modd")?;
    notifier.status("Initializing", Health::Healthy)?;

    let uname = rustix::system::uname();
    let krel = uname.release().to_string_lossy();
    let mod_dir = Path::new("/lib/modules").join(krel.as_ref());

    println!("Module directory: {}", mod_dir.display());

    let alias_path = mod_dir.join("modules.alias");
    let dep_path = mod_dir.join("modules.dep");

    let alias_db = AliasDb::load(&alias_path)
        .with_context(|| format!("Failed to load {}", alias_path.display()))?;
    let dep_db =
        DepDb::load(&dep_path).with_context(|| format!("Failed to load {}", dep_path.display()))?;
    let mut loader = ModuleLoader::new(mod_dir);

    println!(
        "Loaded {} aliases, {} modules in dependency database",
        alias_db.len(),
        dep_db.len()
    );

    let listener = UeventListener::new()?;
    println!("Listening for kernel uevents");

    notifier.ready(SOCKET_PATH)?;

    loop {
        let event = match listener.recv() {
            Ok(e) => e,
            Err(e) => {
                eprintln!("Failed to receive uevent: {}", e);
                continue;
            }
        };

        if event.action != UeventAction::Add {
            continue;
        }

        let Some(modalias) = event.modalias else {
            continue;
        };

        let Some(module_name) = alias_db.find_module(&modalias) else {
            continue;
        };

        match load_module(module_name, &dep_db, &mut loader) {
            Ok(count) if count > 0 => {
                println!(
                    "Loaded {} modules for {} ({})",
                    count,
                    module_name,
                    event.subsystem.as_deref().unwrap_or("unknown")
                );
            }
            Ok(_) => {
                println!(
                    "Loaded module for {} ({})",
                    module_name,
                    event.subsystem.as_deref().unwrap_or("unknown")
                );
            }
            Err(e) => {
                eprintln!("Failed to load module {}: {}", module_name, e);
            }
        }
    }
}
