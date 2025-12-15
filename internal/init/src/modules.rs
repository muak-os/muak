use std::path::Path;

use kmod::{AliasDb, DepDb, ModuleLoader, find_kernel_release, for_each_modalias, load_module};

use crate::logging;

pub fn load() -> Result<usize, Box<dyn std::error::Error>> {
    let modules_base = Path::new("/lib/modules");
    let krel = find_kernel_release(modules_base)?;
    let mod_dir = modules_base.join(&krel);

    logging::log(&format!(
        "Loading kernel modules from {}",
        mod_dir.display()
    ));

    let alias_db = AliasDb::load(&mod_dir.join("modules.alias"))?;
    let dep_db = DepDb::load(&mod_dir.join("modules.dep"))?;
    let mut loader = ModuleLoader::new(mod_dir);
    let mut total_loaded = 0;

    for_each_modalias(|modalias| {
        let Some(module_name) = alias_db.find_module(modalias) else {
            return;
        };
        match load_module(module_name, &dep_db, &mut loader) {
            Ok(count) => total_loaded += count,
            Err(e) => logging::log(&format!("Warning: failed to load {}: {}", module_name, e)),
        }
    })?;

    Ok(total_loaded)
}
