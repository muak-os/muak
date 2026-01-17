use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::dep;

#[derive(Error, Debug)]
pub enum LoadError {
    #[error("module not found: {0}")]
    NotFound(PathBuf),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("decompression error: {0}")]
    Decompress(String),

    #[error("syscall error: {0}")]
    Syscall(#[from] rustix::io::Errno),

    #[error("invalid module path: {0}")]
    InvalidPath(String),
}

pub struct ModuleLoader {
    mod_dir: PathBuf,
    loaded: HashSet<String>,
}

impl ModuleLoader {
    pub fn new(mod_dir: PathBuf) -> Self {
        Self {
            mod_dir,
            loaded: HashSet::new(),
        }
    }

    pub fn load_by_path(&mut self, relative_path: &str) -> Result<bool, LoadError> {
        let module_name = dep::get_module_name(relative_path)
            .ok_or_else(|| LoadError::InvalidPath(relative_path.to_string()))?;

        if self.loaded.contains(&module_name) {
            return Ok(false);
        }

        let full_path = self.mod_dir.join(relative_path);
        if !full_path.exists() {
            return Err(LoadError::NotFound(full_path));
        }

        let module_data = read_module(&full_path)?;
        init_module(&module_data)?;

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
    let mut file = File::open(path)?;

    if path.extension().is_some_and(|ext| ext == "zst") {
        zstd::decode_all(&mut file).map_err(|e| LoadError::Decompress(format!("zstd: {}", e)))
    } else {
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;
        Ok(data)
    }
}

fn init_module(module_data: &[u8]) -> Result<(), LoadError> {
    match rustix::system::init_module(module_data, c"") {
        Ok(()) => Ok(()),
        Err(rustix::io::Errno::EXIST) => Ok(()),
        Err(e) => Err(LoadError::Syscall(e)),
    }
}

pub fn load_module(
    module_name: &str,
    dep_db: &dep::DepDb,
    loader: &mut ModuleLoader,
) -> Result<usize, LoadError> {
    let load_order = dep_db
        .resolve_load_order(module_name)
        .ok_or_else(|| LoadError::NotFound(PathBuf::from(module_name)))?;

    let mut loaded_count = 0;
    for module_path in &load_order {
        if loader.load_by_path(module_path)? {
            loaded_count += 1;
        }
    }

    Ok(loaded_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_module_loader_new() {
        let loader = ModuleLoader::new(PathBuf::from("/lib/modules/test"));
        assert_eq!(loader.loaded_count(), 0);
    }

    #[test]
    fn test_module_loader_tracks_loaded() {
        let dir = TempDir::new().expect("Failed to create temp dir");
        let mut loader = ModuleLoader::new(dir.path().to_path_buf());

        loader.loaded.insert("test_module".to_string());

        assert!(loader.is_loaded("test_module"));
        assert!(!loader.is_loaded("other_module"));
        assert_eq!(loader.loaded_count(), 1);
    }

    #[test]
    fn test_module_loader_load_by_path_invalid_path() {
        let dir = TempDir::new().expect("Failed to create temp dir");
        let mut loader = ModuleLoader::new(dir.path().to_path_buf());

        let result = loader.load_by_path("invalid/path/noextension");
        assert!(matches!(result, Err(LoadError::InvalidPath(_))));
    }

    #[test]
    fn test_module_loader_load_by_path_not_found() {
        let dir = TempDir::new().expect("Failed to create temp dir");
        let mut loader = ModuleLoader::new(dir.path().to_path_buf());

        let result = loader.load_by_path("kernel/nonexistent.ko");
        assert!(matches!(result, Err(LoadError::NotFound(_))));
    }

    #[test]
    fn test_module_loader_skip_already_loaded() {
        let dir = TempDir::new().expect("Failed to create temp dir");
        let mut loader = ModuleLoader::new(dir.path().to_path_buf());

        loader.loaded.insert("already_loaded".to_string());

        let result = loader.load_by_path("kernel/already_loaded.ko");
        assert!(matches!(result, Ok(false)));
    }

    #[test]
    fn test_read_module_plain_ko() {
        let dir = TempDir::new().expect("Failed to create temp dir");
        let module_path = dir.path().join("test.ko");

        let expected_data = b"ELF module data here";
        std::fs::write(&module_path, expected_data).expect("write failed");

        let result = read_module(&module_path);
        assert!(result.is_ok());
        assert_eq!(result.expect("read failed"), expected_data);
    }

    #[test]
    fn test_read_module_zstd_compressed() {
        let dir = TempDir::new().expect("Failed to create temp dir");
        let module_path = dir.path().join("test.ko.zst");

        let original_data = b"This is the original module data";
        let compressed = zstd::encode_all(&original_data[..], 3).expect("compression failed");
        std::fs::write(&module_path, &compressed).expect("write failed");

        let result = read_module(&module_path);
        assert!(result.is_ok());
        assert_eq!(result.expect("read failed"), original_data);
    }

    #[test]
    fn test_read_module_invalid_zstd() {
        let dir = TempDir::new().expect("Failed to create temp dir");
        let module_path = dir.path().join("bad.ko.zst");

        std::fs::write(&module_path, b"not valid zstd data").expect("write failed");

        let result = read_module(&module_path);
        assert!(matches!(result, Err(LoadError::Decompress(_))));
    }

    #[test]
    fn test_read_module_not_found() {
        let result = read_module(Path::new("/nonexistent/module.ko"));
        assert!(matches!(result, Err(LoadError::Io(_))));
    }

    #[test]
    fn test_read_module_empty_file() {
        let dir = TempDir::new().expect("Failed to create temp dir");
        let module_path = dir.path().join("empty.ko");
        std::fs::write(&module_path, b"").expect("write failed");

        let result = read_module(&module_path);
        assert!(result.is_ok());
        assert!(result.expect("read failed").is_empty());
    }

    #[test]
    fn test_load_module_empty_dep_db() {
        let dir = TempDir::new().expect("Failed to create temp dir");

        let dep_path = dir.path().join("modules.dep");
        std::fs::write(&dep_path, "").expect("write failed");

        let dep_db = dep::DepDb::load(&dep_path).expect("load dep db");
        let mut loader = ModuleLoader::new(dir.path().to_path_buf());

        let result = load_module("nonexistent", &dep_db, &mut loader);

        assert!(result.is_ok());
        assert_eq!(result.expect("load failed"), 0);
    }

    #[test]
    fn test_load_module_respects_load_order() {
        let dir = TempDir::new().expect("Failed to create temp dir");

        let dep_path = dir.path().join("modules.dep");
        {
            let mut f = std::fs::File::create(&dep_path).expect("create failed");
            writeln!(f, "kernel/a.ko: kernel/b.ko").expect("write failed");
            writeln!(f, "kernel/b.ko:").expect("write failed");
        }

        let dep_db = dep::DepDb::load(&dep_path).expect("load dep db");
        let mut loader = ModuleLoader::new(dir.path().to_path_buf());

        let result = load_module("a", &dep_db, &mut loader);

        assert!(matches!(result, Err(LoadError::NotFound(_))));
    }
}
