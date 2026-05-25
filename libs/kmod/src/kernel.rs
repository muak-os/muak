//! Kernel module loading.

use std::collections::HashSet;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use rustix::io::Errno;
use rustix::system::init_module as rustix_init_module;
use thiserror::Error;

use crate::deps;

/// Errors returned while loading kernel modules.
#[derive(Error, Debug)]
pub enum LoadError {
    #[error("module not found: {0}")]
    NotFound(PathBuf),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("decompression error: {0}")]
    Decompress(String),

    #[error("syscall error: {0}")]
    Syscall(#[from] Errno),

    #[error("invalid module path: {0}")]
    InvalidPath(String),
}

/// Loads modules from a specific module directory.
pub struct ModuleLoader {
    mod_dir: PathBuf,
    loaded: HashSet<String>,
}

impl ModuleLoader {
    /// Creates a new loader rooted at `mod_dir`.
    #[must_use]
    pub fn new(mod_dir: PathBuf) -> Self {
        Self {
            mod_dir,
            loaded: HashSet::new(),
        }
    }

    /// Loads a module from `relative_path` unless it was already loaded.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is invalid, the module file cannot be
    /// read, decompression fails, or the kernel rejects the module.
    pub fn load_by_path(&mut self, relative_path: &str) -> Result<bool, LoadError> {
        self.load_by_path_with(relative_path, init_module)
    }

    fn load_by_path_with(
        &mut self,
        relative_path: &str,
        init_module_fn: fn(&[u8]) -> Result<(), LoadError>,
    ) -> Result<bool, LoadError> {
        let module_name = deps::get_module_name(relative_path)
            .ok_or_else(|| LoadError::InvalidPath(relative_path.to_owned()))?;

        if self.loaded.contains(&module_name) {
            return Ok(false);
        }

        let full_path = self.mod_dir.join(relative_path);
        if !full_path.exists() {
            return Err(LoadError::NotFound(full_path));
        }

        let module_data = read_module(&full_path)?;
        init_module_fn(&module_data)?;

        self.loaded.insert(module_name);
        Ok(true)
    }

    #[cfg(test)]
    pub fn is_loaded(&self, module_name: &str) -> bool {
        self.loaded.contains(module_name)
    }

    #[cfg(test)]
    pub fn loaded_count(&self) -> usize {
        self.loaded.len()
    }
}

fn read_module(path: &Path) -> Result<Vec<u8>, LoadError> {
    if path.extension().is_some_and(|ext| ext == "zst") {
        let mut file = File::open(path)?;
        zstd::decode_all(&mut file).map_err(|error| LoadError::Decompress(format!("zstd: {error}")))
    } else {
        Ok(fs::read(path)?)
    }
}

/// Loads `module_name` and its dependencies.
///
/// # Errors
///
/// Returns an error when the module is absent from `dep_db`, its file cannot
/// be read, or the kernel rejects one of the modules in the load chain.
pub fn load_module(
    module_name: &str,
    dep_db: &deps::DepDb,
    loader: &mut ModuleLoader,
) -> Result<usize, LoadError> {
    load_module_with(module_name, dep_db, loader, init_module)
}

fn load_module_with(
    module_name: &str,
    dep_db: &deps::DepDb,
    loader: &mut ModuleLoader,
    init_module_fn: fn(&[u8]) -> Result<(), LoadError>,
) -> Result<usize, LoadError> {
    let load_order = dep_db
        .resolve_load_order(module_name)
        .ok_or_else(|| LoadError::NotFound(PathBuf::from(module_name)))?;

    let mut loaded_count = 0_usize;
    for module_path in &load_order {
        if loader.load_by_path_with(module_path, init_module_fn)? {
            loaded_count = loaded_count.saturating_add(1);
        }
    }

    Ok(loaded_count)
}

fn init_module(module_data: &[u8]) -> Result<(), LoadError> {
    map_init_module_result(rustix_init_module(module_data, c""))
}

fn map_init_module_result(result: Result<(), Errno>) -> Result<(), LoadError> {
    match result {
        Ok(()) | Err(Errno::EXIST) => Ok(()),
        Err(error) => Err(LoadError::Syscall(error)),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn module_loader_new() {
        // ACT
        let loader = ModuleLoader::new(PathBuf::from("/lib/modules/test"));

        // ASSERT
        assert_eq!(loader.loaded_count(), 0);
    }

    #[test]
    fn module_loader_tracks_loaded() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");
        let mut loader = ModuleLoader::new(dir.path().to_path_buf());

        // ACT
        loader.loaded.insert("test_module".to_string());

        // ASSERT
        assert!(loader.is_loaded("test_module"));
        assert!(!loader.is_loaded("other_module"));
        assert_eq!(loader.loaded_count(), 1);
    }

    #[test]
    fn module_loader_load_by_path_invalid_path() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");
        let mut loader = ModuleLoader::new(dir.path().to_path_buf());

        // ACT
        let result = loader.load_by_path("invalid/path/noextension");

        // ASSERT
        assert!(matches!(result, Err(LoadError::InvalidPath(_))));
    }

    #[test]
    fn module_loader_load_by_path_not_found() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");
        let mut loader = ModuleLoader::new(dir.path().to_path_buf());

        // ACT
        let result = loader.load_by_path("kernel/nonexistent.ko");

        // ASSERT
        assert!(matches!(result, Err(LoadError::NotFound(_))));
    }

    #[test]
    fn module_loader_load_by_path_success_records_module() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");
        let module_path = dir.path().join("kernel/success.ko");
        std::fs::create_dir_all(module_path.parent().expect("module parent must exist"))
            .expect("create module parent");
        std::fs::write(&module_path, b"module bytes").expect("write failed");
        let mut loader = ModuleLoader::new(dir.path().to_path_buf());

        // ACT
        let result = loader.load_by_path_with("kernel/success.ko", |_| Ok(()));

        // ASSERT
        assert!(matches!(result, Ok(true)));
        assert!(loader.is_loaded("success"));
    }

    #[test]
    fn module_loader_load_by_path_propagates_init_errors() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");
        let module_path = dir.path().join("kernel/broken.ko");
        std::fs::create_dir_all(module_path.parent().expect("module parent must exist"))
            .expect("create module parent");
        std::fs::write(&module_path, b"module bytes").expect("write failed");
        let mut loader = ModuleLoader::new(dir.path().to_path_buf());

        // ACT
        let result = loader.load_by_path_with("kernel/broken.ko", |_| {
            Err(LoadError::Decompress("simulated failure".to_owned()))
        });

        // ASSERT
        assert!(matches!(result, Err(LoadError::Decompress(_))));
        assert!(!loader.is_loaded("broken"));
    }

    #[test]
    fn module_loader_skip_already_loaded() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");
        let mut loader = ModuleLoader::new(dir.path().to_path_buf());

        loader.loaded.insert("already_loaded".to_string());

        // ACT
        let result = loader.load_by_path("kernel/already_loaded.ko");

        // ASSERT
        assert!(matches!(result, Ok(false)));
    }

    #[test]
    fn module_loader_load_by_path_calls_kernel_init_module() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");
        let module_path = dir.path().join("kernel/live.ko");
        std::fs::create_dir_all(module_path.parent().expect("module parent must exist"))
            .expect("create module parent");
        std::fs::write(&module_path, b"not a real kernel module").expect("write failed");
        let mut loader = ModuleLoader::new(dir.path().to_path_buf());

        // ACT
        let result = loader.load_by_path("kernel/live.ko");

        // ASSERT
        assert!(matches!(result, Err(LoadError::Syscall(_))));
    }

    #[test]
    fn read_module_plain_ko() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");
        let module_path = dir.path().join("test.ko");

        let expected_data = b"ELF module data here";
        std::fs::write(&module_path, expected_data).expect("write failed");

        // ACT
        let result = read_module(&module_path);

        // ASSERT
        assert!(result.is_ok());
        assert_eq!(result.expect("read failed"), expected_data);
    }

    #[test]
    fn read_module_zstd_compressed() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");
        let module_path = dir.path().join("test.ko.zst");

        let original_data = b"This is the original module data";
        let compressed = zstd::encode_all(&original_data[..], 3).expect("compression failed");
        std::fs::write(&module_path, &compressed).expect("write failed");

        // ACT
        let result = read_module(&module_path);

        // ASSERT
        assert!(result.is_ok());
        assert_eq!(result.expect("read failed"), original_data);
    }

    #[test]
    fn read_module_invalid_zstd() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");
        let module_path = dir.path().join("bad.ko.zst");

        std::fs::write(&module_path, b"not valid zstd data").expect("write failed");

        // ACT
        let result = read_module(&module_path);

        // ASSERT
        assert!(matches!(result, Err(LoadError::Decompress(_))));
    }

    #[test]
    fn read_module_not_found() {
        // ACT
        let result = read_module(Path::new("/nonexistent/module.ko"));

        // ASSERT
        assert!(matches!(result, Err(LoadError::Io(_))));
    }

    #[test]
    fn read_module_empty_file() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");
        let module_path = dir.path().join("empty.ko");
        std::fs::write(&module_path, b"").expect("write failed");

        // ACT
        let result = read_module(&module_path);

        // ASSERT
        assert!(result.is_ok());
        assert!(result.expect("read failed").is_empty());
    }

    #[test]
    fn map_init_module_result_accepts_existing_module() {
        // ACT
        let result = map_init_module_result(Err(Errno::EXIST));

        // ASSERT
        assert!(matches!(result, Ok(())));
    }

    #[test]
    fn map_init_module_result_propagates_other_syscalls() {
        // ACT
        let result = map_init_module_result(Err(Errno::PERM));

        // ASSERT
        assert!(matches!(result, Err(LoadError::Syscall(Errno::PERM))));
    }

    #[test]
    fn load_module_empty_dep_db() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");

        let dep_path = dir.path().join("modules.dep");
        std::fs::write(&dep_path, "").expect("write failed");

        let dep_db = deps::DepDb::load(&dep_path).expect("load dep db");
        let mut loader = ModuleLoader::new(dir.path().to_path_buf());

        // ACT
        let result = load_module("nonexistent", &dep_db, &mut loader);

        // ASSERT
        assert!(matches!(result, Err(LoadError::NotFound(_))));
    }

    #[test]
    fn load_module_respects_load_order() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");

        let dep_path = dir.path().join("modules.dep");
        {
            let mut f = std::fs::File::create(&dep_path).expect("create failed");
            writeln!(f, "kernel/a.ko: kernel/b.ko").expect("write failed");
            writeln!(f, "kernel/b.ko:").expect("write failed");
        }

        let dep_db = deps::DepDb::load(&dep_path).expect("load dep db");
        let mut loader = ModuleLoader::new(dir.path().to_path_buf());

        // ACT
        let result = load_module("a", &dep_db, &mut loader);

        // ASSERT
        assert!(matches!(result, Err(LoadError::NotFound(_))));
    }

    #[test]
    fn load_module_loads_dependencies_and_counts_only_new_modules() {
        // ARRANGE
        let dir = TempDir::new().expect("Failed to create temp dir");
        std::fs::create_dir_all(dir.path().join("kernel")).expect("create kernel dir");
        std::fs::write(dir.path().join("kernel/a.ko"), b"module a").expect("write module a");
        std::fs::write(dir.path().join("kernel/b.ko"), b"module b").expect("write module b");

        let dep_path = dir.path().join("modules.dep");
        {
            let mut file = std::fs::File::create(&dep_path).expect("create failed");
            writeln!(file, "kernel/a.ko: kernel/b.ko").expect("write failed");
            writeln!(file, "kernel/b.ko:").expect("write failed");
        }

        let dep_db = deps::DepDb::load(&dep_path).expect("load dep db");
        let mut loader = ModuleLoader::new(dir.path().to_path_buf());
        loader.loaded.insert("b".to_owned());

        // ACT
        let result = load_module_with("a", &dep_db, &mut loader, |_| Ok(()));

        // ASSERT
        assert!(matches!(result, Ok(1)));
        assert!(loader.is_loaded("a"));
        assert!(loader.is_loaded("b"));
    }
}
