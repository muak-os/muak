use std::path::Path;

use anyhow::{Context, Result};
use kmod::{AliasDb, DepDb, ModuleLoader, for_each_modalias, load_module};

pub fn load() -> Result<usize> {
    let krel = rustix::system::uname()
        .release()
        .to_string_lossy()
        .into_owned();
    let mod_dir = Path::new("/lib/modules").join(&krel);

    let alias_db =
        AliasDb::load(&mod_dir.join("modules.alias")).context("Failed to load modules.alias")?;
    let dep_db = DepDb::load(&mod_dir.join("modules.dep")).context("Failed to load modules.dep")?;
    let mut loader = ModuleLoader::new(mod_dir);
    let mut total_loaded = 0;

    for_each_modalias(|modalias| {
        let Some(module_name) = alias_db.find_module(modalias) else {
            return;
        };
        match load_module(module_name, &dep_db, &mut loader) {
            Ok(count) => total_loaded += count,
            Err(e) => kmsg::warn!("Failed to load {}: {}", module_name, e),
        }
    })
    .context("Failed to iterate modaliases")?;

    Ok(total_loaded)
}
