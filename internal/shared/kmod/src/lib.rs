mod alias;
mod dep;
mod discovery;
mod loader;

pub use alias::AliasDb;
pub use dep::DepDb;
pub use discovery::for_each_modalias;
pub use loader::{ModuleLoader, load_module};

use std::path::Path;

pub fn find_kernel_release(modules_base: &Path) -> std::io::Result<String> {
    for entry in std::fs::read_dir(modules_base)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                if !name.starts_with('.') && name.chars().next().is_some_and(|c| c.is_ascii_digit())
                {
                    return Ok(name.to_string());
                }
            }
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "no kernel modules directory found",
    ))
}

pub fn load_all_hardware_modules(modules_base: &Path) -> Result<usize, Box<dyn std::error::Error>> {
    let krel = find_kernel_release(modules_base)?;
    let mod_dir = modules_base.join(&krel);

    let alias_db = AliasDb::load(&mod_dir.join("modules.alias"))?;
    let dep_db = DepDb::load(&mod_dir.join("modules.dep"))?;
    let mut loader = ModuleLoader::new(mod_dir);

    let mut loaded = 0;

    for_each_modalias(|modalias| {
        if let Some(module) = alias_db.find_module(modalias) {
            if load_module(module, &dep_db, &mut loader).is_ok() {
                loaded += 1;
            }
        }
    })?;

    Ok(loaded)
}
