use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use nix::libc;

use crate::dep;

#[derive(Debug)]
pub enum LoadError {
    NotFound(PathBuf),
    Io(std::io::Error),
    Decompress(String),
    Syscall(nix::errno::Errno),
    InvalidPath(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(path) => write!(f, "module not found: {}", path.display()),
            Self::Io(e) => write!(f, "IO error: {}", e),
            Self::Decompress(s) => write!(f, "decompression error: {}", s),
            Self::Syscall(e) => write!(f, "syscall error: {}", e),
            Self::InvalidPath(s) => write!(f, "invalid module path: {}", s),
        }
    }
}

impl std::error::Error for LoadError {}

impl From<std::io::Error> for LoadError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<nix::errno::Errno> for LoadError {
    fn from(e: nix::errno::Errno) -> Self {
        Self::Syscall(e)
    }
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
    let ret = unsafe {
        libc::syscall(
            libc::SYS_init_module,
            module_data.as_ptr(),
            module_data.len(),
            c"".as_ptr(),
        )
    };

    if ret == 0 {
        return Ok(());
    }

    let errno = nix::errno::Errno::last();
    if errno == nix::errno::Errno::EEXIST {
        return Ok(());
    }

    Err(LoadError::Syscall(errno))
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
