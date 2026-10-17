//! Shared harness for wizard end-to-end tests.

pub mod fixtures;
pub mod pe;
pub mod server;

use std::io::Error as IoError;
use std::sync::OnceLock;

use server::{MockRegistry, Routes};
use tempfile::TempDir;
use wizard::config::init_cache;

/// The per-process harness: one registry and one cache.
pub struct Harness {
    pub registry: MockRegistry,
    pub _cache: TempDir,
}

impl Harness {
    pub fn start(routes: Routes) -> Result<Self, IoError> {
        static CACHE_INITIALIZED: OnceLock<()> = OnceLock::new();

        let registry = MockRegistry::start(routes)?;
        let cache = TempDir::new()?;

        CACHE_INITIALIZED.get_or_init(|| {
            init_cache(cache.path().to_path_buf());
        });

        Ok(Self {
            registry,
            _cache: cache,
        })
    }
}
