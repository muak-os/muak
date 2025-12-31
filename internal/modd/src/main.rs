mod uevent;

use kmod::{AliasDb, DepDb, ModuleLoader, load_module};
use notify::{Health, NotifyClient};
use std::path::Path;
use std::process;
use uevent::{UeventAction, UeventListener};

const SOCKET_PATH: &str = "/run/modd.sock";

fn main() {
    if let Err(e) = run() {
        kmsg::error!("Fatal error: {}", e);
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    kmsg::init("modd")?;
    kmsg::info!("Starting module daemon");

    let notifier = NotifyClient::new("modd")?;
    notifier.status("Initializing", Health::Healthy)?;

    let uname = nix::sys::utsname::uname()?;
    let krel = uname.release().to_string_lossy();
    let mod_dir = Path::new("/lib/modules").join(krel.as_ref());

    kmsg::info!("Module directory: {}", mod_dir.display());

    let alias_path = mod_dir.join("modules.alias");
    let dep_path = mod_dir.join("modules.dep");

    if !alias_path.exists() {
        return Err(format!("modules.alias not found: {}", alias_path.display()).into());
    }
    if !dep_path.exists() {
        return Err(format!("modules.dep not found: {}", dep_path.display()).into());
    }

    let alias_db = AliasDb::load(&alias_path)?;
    let dep_db = DepDb::load(&dep_path)?;
    let mut loader = ModuleLoader::new(mod_dir);

    kmsg::info!(
        "Loaded {} aliases, {} modules in dependency database",
        alias_db.len(),
        dep_db.len()
    );

    let listener = UeventListener::new()?;
    kmsg::info!("Listening for kernel uevents");

    notifier.ready(SOCKET_PATH)?;

    loop {
        let event = match listener.recv() {
            Ok(e) => e,
            Err(e) => {
                kmsg::warn!("Failed to receive uevent: {}", e);
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
            Ok(count) => {
                if count > 0 {
                    kmsg::info!(
                        "Loaded {} module(s) for {} ({})",
                        count,
                        module_name,
                        event.subsystem.as_deref().unwrap_or("unknown")
                    );
                }
            }
            Err(e) => {
                kmsg::warn!("Failed to load module {}: {}", module_name, e);
            }
        }
    }
}
