//! Kernel module loading driven by device modalias matching.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context as _, Result};
use kmod::aliases::AliasDb;
use kmod::deps::DepDb;
use kmod::kernel::{ModuleLoader, load_module};
use kmod::sysfs::for_each_modalias;

pub(crate) struct Loader {
    alias_db: AliasDb,
    dep_db: DepDb,
    kernel: ModuleLoader,
}

impl Loader {
    /// Loads the alias and dependency databases for the running kernel.
    pub(crate) fn new(mod_dir: &Path) -> Result<Self> {
        let alias_path = mod_dir.join("modules.alias");
        let dep_path = mod_dir.join("modules.dep");

        let alias_db = AliasDb::load(&alias_path)
            .with_context(|| format!("Failed to load {}", alias_path.display()))?;
        let dep_db = DepDb::load(&dep_path)
            .with_context(|| format!("Failed to load {}", dep_path.display()))?;

        println!(
            "Loaded {} aliases, {} modules in dependency database",
            alias_db.len(),
            dep_db.len()
        );

        Ok(Self {
            alias_db,
            dep_db,
            kernel: ModuleLoader::new(mod_dir.to_path_buf()),
        })
    }

    /// Loads the module matching `modalias` and logs the outcome.
    pub(crate) fn load(&mut self, modalias: &str, subsystem: &str) -> usize {
        let Some(module_name) = self.alias_db.find_module(modalias) else {
            return 0;
        };

        match load_module(module_name, &self.dep_db, &mut self.kernel) {
            Ok(count) if count > 0 => {
                println!("Loaded {count} modules for {module_name} ({subsystem})");
                count
            }
            Ok(_) => {
                println!("Module {module_name} ({subsystem}) already loaded");
                0
            }
            Err(e) => {
                eprintln!("Failed to load module {module_name}: {e}");
                0
            }
        }
    }

    /// Loads modules for every device already present in sysfs.
    pub(crate) fn sweep(&mut self) -> usize {
        let mut seen = HashSet::new();
        let mut loaded = 0_usize;

        let scan = for_each_modalias(|modalias| {
            loaded = loaded.saturating_add(self.load_unseen(modalias, &mut seen));
        });

        if let Err(e) = scan {
            eprintln!("Failed to scan sysfs modalias: {e}");
        }

        loaded
    }

    fn load_unseen(&mut self, modalias: &str, seen: &mut HashSet<String>) -> usize {
        if !seen.insert(modalias.to_owned()) {
            return 0;
        }

        self.load(modalias, "sysfs")
    }
}
